//! Snapshot-based saves: all filesystem/HTTP I/O stays off the foreground thread.
use super::*;
use dunlin_workspace::fs::FileStat;

struct Save {
    id: String,
    project: String,
    root: String,
    path: String,
    text: String,
    expected: Option<u64>,
    backend: WorkspaceBackend,
    generation: u64,
}

impl Save {
    fn write(self) -> (Self, Result<FileStat, String>) {
        let result = self
            .backend
            .write_file(&self.path, &self.text, self.expected);
        (self, result)
    }
}

impl DunlinApp {
    fn prepare_file_save(
        &mut self,
        id: &str,
        expected: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<Save, String> {
        let tab = self
            .workbench
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .ok_or("File tab is closed")?;
        if tab.saving || tab.loading {
            return Err("Wait for the file operation to finish".into());
        }
        if tab.untitled || tab.readonly {
            return Err("Choose a writable, named file before saving".into());
        }
        if tab.git_changed && expected.is_some() {
            return Err(
                "Resolve the Git-changed file's reload/overwrite conflict before saving".into(),
            );
        }
        if self.ai_commit_locks_project_id(&tab.project)
            || self.git_history.running
            || self.explorer.busy
            || self.group_cleanup.busy
        {
            return Err("Wait for the repository operation to finish before saving".into());
        }
        if !self
            .projects
            .iter()
            .any(|project| project.id == tab.project && project.path == tab.root)
        {
            return Err(
                "A dirty tab's project was relocated. Restore its path before saving.".into(),
            );
        }
        let tab = self
            .workbench
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .unwrap();
        tab.saving = true;
        // Invalidate in-flight reload/stat requests before starting a write.
        tab.reload_generation = tab.reload_generation.wrapping_add(1);
        let save = Save {
            id: tab.id.clone(),
            project: tab.project.clone(),
            root: tab.root.clone(),
            path: tab.inspector.path.clone(),
            text: tab.editor.read(cx).value().to_string(),
            expected,
            backend: tab.backend.clone(),
            generation: tab.reload_generation,
        };
        cx.notify();
        Ok(save)
    }

    fn complete_file_save(
        &mut self,
        save: Save,
        result: Result<FileStat, String>,
        announce: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.workbench.tabs.iter_mut().find(|tab| tab.id == save.id) else {
            return false;
        };
        tab.saving = false;
        if tab.project != save.project
            || tab.root != save.root
            || tab.inspector.path != save.path
            || tab.reload_generation != save.generation
        {
            self.file_notice(
                "The file changed location during saving. Review the buffer before closing it.",
                cx,
            );
            return false;
        }
        let active =
            self.workbench.active.as_deref() == Some(&save.id) && self.project().id == save.project;
        // Reborrow after checking the selected project.
        let tab = self
            .workbench
            .tabs
            .iter_mut()
            .find(|tab| tab.id == save.id)
            .unwrap();
        let name = tab.inspector.name.clone();
        let success = match result {
            Ok(stat) => {
                let dirty = buffer_changed(Some(&save.text), tab.editor.read(cx).value().as_ref());
                tab.saved_text = Some(save.text);
                tab.dirty = dirty;
                tab.mtime = Some(stat.mtime_ms);
                tab.inspector.bytes = stat.bytes as usize;
                tab.conflict = None;
                tab.git_changed = false;
                tab.minimap.invalidate();
                if active {
                    self.editor_dirty = dirty;
                    self.editor_mtime = tab.mtime;
                    self.editor_conflict = None;
                    self.editor_stale = false;
                    self.inspector = Some(tab.inspector.clone());
                }
                if announce {
                    self.notice = Some(format!("Saved {name}"));
                }
                if self.project().id == save.project {
                    self.refresh_git_status(cx);
                    self.remember_dashboard_project(cx);
                }
                if active && dirty && self.autosave {
                    self.schedule_autosave(cx);
                }
                !dirty
            }
            Err(error) => {
                if error.starts_with("conflict:") {
                    let conflict = EditorConflict {
                        path: save.path,
                        message: error,
                    };
                    tab.conflict = Some(conflict.clone());
                    if active {
                        self.editor_conflict = Some(conflict);
                    }
                    self.file_notice(
                        format!("{name} changed elsewhere · reload or overwrite"),
                        cx,
                    );
                } else {
                    self.file_notice(format!("Could not save {name}: {error}"), cx);
                }
                false
            }
        };
        if active && success {
            self.finish_pending_editor(cx);
        }
        cx.notify();
        success
    }

    pub(crate) fn save_file_tab(
        &mut self,
        id: &str,
        expected: Option<u64>,
        announce: bool,
        cx: &mut Context<Self>,
    ) {
        let save = match self.prepare_file_save(id, expected, cx) {
            Ok(save) => save,
            Err(error) => {
                if announce {
                    self.file_notice(error, cx);
                }
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let (save, result) = cx
                .background_executor()
                .spawn(async move { save.write() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.complete_file_save(save, result, announce, cx);
            });
        })
        .detach();
    }

    pub(crate) fn save_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.save_tabs_then(None, cx, |_, _| {});
    }

    /// Closing/quitting is a continuation, never a guess made before HTTP completes.
    pub(crate) fn save_tabs_then(
        &mut self,
        project: Option<&str>,
        cx: &mut Context<Self>,
        on_saved: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        self.stash_editor_tab();
        let project = project.map(str::to_owned);
        let ids: Vec<_> = self
            .workbench
            .tabs
            .iter()
            .filter(|tab| {
                tab.dirty
                    && project
                        .as_ref()
                        .is_none_or(|project| &tab.project == project)
            })
            .map(|tab| (tab.id.clone(), tab.mtime, tab.untitled))
            .collect();
        if let Some((id, _, _)) = ids.iter().find(|(_, _, untitled)| *untitled) {
            self.save_untitled_file(id, cx);
            return;
        }
        let mut saves = Vec::new();
        for (id, expected, _) in ids {
            match self.prepare_file_save(&id, expected, cx) {
                Ok(save) => saves.push(save),
                Err(error) => {
                    for save in saves {
                        if let Some(tab) =
                            self.workbench.tabs.iter_mut().find(|tab| tab.id == save.id)
                        {
                            tab.saving = false;
                        }
                    }
                    self.file_notice(error, cx);
                    return;
                }
            }
        }
        // Do not close a clean tab while its Save As or an earlier save is in flight.
        if self.workbench.tabs.iter().any(|tab| {
            tab.saving
                && !saves.iter().any(|save| save.id == tab.id)
                && project
                    .as_ref()
                    .is_none_or(|project| &tab.project == project)
        }) {
            for save in saves {
                if let Some(tab) = self.workbench.tabs.iter_mut().find(|tab| tab.id == save.id) {
                    tab.saving = false;
                }
            }
            self.file_notice("Wait for pending saves to finish", cx);
            return;
        }
        cx.spawn(async move |this, cx| {
            let mut saves = saves.into_iter();
            while let Some(save) = saves.next() {
                let (save, result) = cx
                    .background_executor()
                    .spawn(async move { save.write() })
                    .await;
                if !matches!(
                    this.update(cx, |this, cx| this
                        .complete_file_save(save, result, false, cx)),
                    Ok(true)
                ) {
                    let _ = this.update(cx, |this, cx| {
                        for save in saves {
                            if let Some(tab) =
                                this.workbench.tabs.iter_mut().find(|tab| tab.id == save.id)
                            {
                                tab.saving = false;
                            }
                        }
                        cx.notify();
                    });
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.stash_editor_tab();
                if !this.workbench.tabs.iter().any(|tab| {
                    (tab.dirty || tab.saving)
                        && project
                            .as_ref()
                            .is_none_or(|project| &tab.project == project)
                }) {
                    on_saved(this, cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn saving_a_snapshot_does_not_mark_later_edits_clean() {
        assert!(!super::buffer_changed(
            Some("saved snapshot"),
            "saved snapshot"
        ));
        assert!(super::buffer_changed(
            Some("saved snapshot"),
            "edits typed during HTTP"
        ));
    }
}
