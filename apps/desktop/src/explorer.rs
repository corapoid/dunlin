//! Project-scoped Explorer operations. Filesystem and relay work stay off the UI thread.
use crate::{app::DunlinApp, remote::WorkspaceBackend};
use dunlin_ui::*;
use dunlin_workspace::{
    fs as workspace_fs,
    transfer::{self, TransferReport},
};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct State {
    pub selected: Option<(String, String, bool)>,
    pub creation: Option<Creation>,
    pub busy: bool,
    pub expand_after_scan: Vec<String>,
    pub tree_generation: u64,
    pub git_generation: u64,
}

#[derive(Clone)]
pub(crate) struct Creation {
    pub project: String,
    pub root: String,
    pub parent: String,
    pub directory: bool,
    pub rename: Option<String>,
}

#[derive(Clone)]
pub(crate) struct EntryDrag {
    pub project: String,
    pub root: String,
    pub path: String,
    pub directory: bool,
}

impl Render for EntryDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .p(px(8.))
            .rounded(px(6.))
            .bg(p.card)
            .border_1()
            .border_color(p.border)
            .text_size(px(12.))
            .text_color(p.fg)
            .child(format!(
                "{} {}",
                if self.directory { "Folder:" } else { "File:" },
                Path::new(&self.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ))
    }
}

pub(crate) fn drop_target(
    view: Stateful<Div>,
    target: String,
    cx: &mut Context<DunlinApp>,
) -> Stateful<Div> {
    let external_target = target.clone();
    view.drag_over::<EntryDrag>(|style, _, _, cx| style.bg(palette(cx).blue_soft))
        .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(palette(cx).blue_soft))
        .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
            cx.stop_propagation();
            this.drop_explorer_paths(paths.paths().to_vec(), external_target.clone(), false, cx);
        }))
        .on_drop(cx.listener(move |this, entry: &EntryDrag, _, cx| {
            cx.stop_propagation();
            if entry.project != this.project().id || entry.root != this.project().path {
                this.set_notice(
                    "The dragged item belongs to another project. Use Finder to copy it instead.",
                    cx,
                );
                return;
            }
            this.drop_explorer_paths(vec![entry.path.clone().into()], target.clone(), true, cx);
        }))
}

fn remap(path: &Path, report: &TransferReport) -> Option<PathBuf> {
    report
        .completed
        .iter()
        .filter(|entry| entry.moved)
        .find_map(|entry| {
            path.strip_prefix(&entry.source)
                .ok()
                .map(|tail| Path::new(&entry.destination).join(tail))
        })
}

impl DunlinApp {
    pub(crate) fn select_explorer_entry(&mut self, path: &str, directory: bool) {
        self.explorer.selected = Some((self.project().id.clone(), path.into(), directory));
    }

    pub(crate) fn explorer_parent(&self) -> String {
        self.explorer
            .selected
            .as_ref()
            .filter(|(project, _, _)| project == &self.project().id)
            .map(|(_, path, directory)| {
                if *directory {
                    path.clone()
                } else {
                    Path::new(path)
                        .parent()
                        .unwrap_or(Path::new(&self.project().path))
                        .to_string_lossy()
                        .into()
                }
            })
            .unwrap_or_else(|| self.project().path.clone())
    }

    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_explorer_entry(self.explorer_parent(), false, window, cx);
    }

    pub(crate) fn new_explorer_entry(
        &mut self,
        parent: String,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.projects.is_empty() || self.explorer.busy {
            return;
        }
        self.explorer.creation = Some(Creation {
            project: self.project().id.clone(),
            root: self.project().path.clone(),
            parent,
            directory,
            rename: None,
        });
        self.new_file_prompt = true;
        self.new_file_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn rename_explorer_entry(
        &mut self,
        path: String,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.explorer.busy
            || self
                .dirty_tab_paths()
                .iter()
                .any(|p| Path::new(p).starts_with(&path))
        {
            self.file_notice(
                "Save affected tabs and wait for loading to finish before renaming",
                cx,
            );
            return;
        }
        let parent = Path::new(&path)
            .parent()
            .unwrap_or(Path::new(&self.project().path))
            .to_string_lossy()
            .into_owned();
        self.new_explorer_entry(parent, directory, window, cx);
        if let Some(creation) = self.explorer.creation.as_mut() {
            creation.rename = Some(path.clone());
        }
        let name = Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.new_file_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
    }

    pub(crate) fn delete_explorer_entry(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.explorer.busy
            || self
                .dirty_tab_paths()
                .iter()
                .any(|p| Path::new(p).starts_with(&path))
        {
            self.file_notice(
                "Save affected tabs and wait for loading to finish before deleting",
                cx,
            );
            return;
        }
        let project = self.project().id.clone();
        let root = self.project().path.clone();
        let remote = self.project().is_remote();
        let question = if remote {
            "Delete this remote file or empty folder?"
        } else {
            "Move this item to the system Trash?"
        };
        let receiver = window.prompt(
            PromptLevel::Warning,
            question,
            Some(&path),
            &["Cancel", if remote { "Delete" } else { "Move to Trash" }],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let operation = this
                .update(cx, |this, cx| {
                    if this.project().id != project
                        || this.project().path != root
                        || !this.guard_ai_commit_mutation(cx)
                        || this.explorer.busy
                        || this
                            .dirty_tab_paths()
                            .iter()
                            .any(|p| Path::new(p).starts_with(&path))
                    {
                        return None;
                    }
                    match this.backend() {
                        Ok(backend) => {
                            this.explorer.busy = true;
                            cx.notify();
                            Some(backend)
                        }
                        Err(e) => {
                            this.file_notice(e, cx);
                            None
                        }
                    }
                })
                .ok()
                .flatten();
            let Some(backend) = operation else {
                return;
            };
            let target = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend.delete_entry(&target) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.explorer.busy = false;
                if this.project().id != project || this.project().path != root {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.workbench.tabs.retain(|t| {
                            t.project != project || !Path::new(&t.inspector.path).starts_with(&path)
                        });
                        if this
                            .inspector
                            .as_ref()
                            .is_some_and(|i| Path::new(&i.path).starts_with(&path))
                        {
                            this.workbench.active = None;
                            this.workbench.show_file = false;
                            this.close_inspector();
                        }
                        this.explorer.selected = None;
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                        this.file_notice(
                            if remote {
                                "Deleted item on the relay"
                            } else {
                                "Moved item to the system Trash"
                            },
                            cx,
                        );
                    }
                    Err(e) => this.file_notice(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn reveal_explorer_entry(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.project().is_remote() {
            self.file_notice("Remote files cannot be revealed in this Mac's Finder", cx);
            return;
        }
        let root = self.project().path.clone();
        let relative = workspace_fs::relative_to(&root, Path::new(path));
        match workspace_fs::resolve(&root, &relative) {
            Ok(path) => {
                if let Err(e) = std::process::Command::new("/usr/bin/open")
                    .arg("-R")
                    .arg(path)
                    .spawn()
                {
                    self.file_notice(e.to_string(), cx);
                }
            }
            Err(e) => self.file_notice(e, cx),
        }
    }

    pub(crate) fn create_remote_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.explorer.busy || !self.new_file_prompt {
            return;
        }
        let Some(creation) = self.explorer.creation.clone() else {
            return;
        };
        if creation.project != self.project().id || creation.root != self.project().path {
            self.new_file_prompt = false;
            self.set_notice(
                "The project changed. Choose where to create the item again.",
                cx,
            );
            return;
        }
        let name = self.new_file_input.read(cx).value().trim().to_owned();
        if name.is_empty()
            || Path::new(&name).is_absolute()
            || name
                .split(['/', '\\'])
                .any(|part| part == ".." || part.eq_ignore_ascii_case(".git"))
        {
            self.set_notice(
                "Enter a relative name inside the selected folder, outside Git metadata",
                cx,
            );
            return;
        }
        if creation.rename.is_some()
            && (name.contains(['/', '\\'])
                || self
                    .dirty_tab_paths()
                    .iter()
                    .any(|p| Path::new(p).starts_with(creation.rename.as_ref().unwrap())))
        {
            self.file_notice(
                "Use a single name and save affected tabs before renaming",
                cx,
            );
            return;
        }
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let full = Path::new(&creation.parent)
            .join(&name)
            .to_string_lossy()
            .to_string();
        if !self.project().is_remote()
            && !crate::app::path_inside(Path::new(&creation.root), Path::new(&full))
        {
            self.set_notice("Choose a location inside the current project", cx);
            return;
        }
        let path = full.clone();
        self.explorer.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let directory = creation.directory;
            let rename = creation.rename.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(source) = rename {
                        backend
                            .rename_entry(
                                &source,
                                Path::new(&path)
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_str()
                                    .unwrap_or(""),
                            )
                            .map(Some)
                    } else if directory {
                        backend.create_directory(&path).map(|_| None)
                    } else {
                        backend.create_file(&path).map(|_| None)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.explorer.busy = false;
                if this.project().id != creation.project || this.project().path != creation.root {
                    return;
                }
                match result {
                    Ok(renamed) => {
                        this.new_file_prompt = false;
                        if let Some(entry) = renamed {
                            this.retarget_file_tabs(
                                &TransferReport {
                                    completed: vec![entry],
                                    error: None,
                                },
                                cx,
                            );
                        }
                        this.explorer.expand_after_scan.push(creation.parent);
                        if directory {
                            this.explorer.expand_after_scan.push(full.clone());
                        }
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                        this.select_explorer_entry(&full, directory);
                        if !directory && creation.rename.is_none() {
                            this.open_file(&full, window, cx);
                        }
                        this.set_notice(
                            format!(
                                "{} {name}",
                                if creation.rename.is_some() {
                                    "Renamed"
                                } else {
                                    "Created"
                                }
                            ),
                            cx,
                        );
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn drop_explorer_paths(
        &mut self,
        sources: Vec<PathBuf>,
        target: String,
        internal: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.explorer.busy {
            self.set_notice("Wait for the current file operation to finish", cx);
            return;
        }
        if self.new_file_prompt || self.settings_open {
            self.set_notice("Close the open dialog before dropping files", cx);
            return;
        }
        if sources.is_empty() || self.projects.is_empty() {
            return;
        }
        if self.project().is_remote() && !internal {
            self.set_notice("Finder imports currently work for local projects only. Remote Explorer supports creating and moving items on the relay.", cx);
            return;
        }
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let project = self.project().clone();
        let root = project.path.clone();
        let local = !project.is_remote();
        let destination = target.clone();
        let dirty_paths = self.dirty_tab_paths();
        let open_path = self.inspector.as_ref().map(|file| file.path.clone());
        self.explorer.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                for path in &dirty_paths {
                    let path = if local { std::fs::canonicalize(path).unwrap_or_else(|_| path.into()) } else { path.into() };
                    if sources.iter().any(|source| path.starts_with(if local { std::fs::canonicalize(source).unwrap_or_else(|_| source.clone()) } else { source.clone() })) {
                        return Err("Save the open file before moving it or its folder".to_string());
                    }
                }
                let canonical_open = if local { open_path.as_ref().and_then(|path| std::fs::canonicalize(path).ok()) } else { None };
                let report = if internal {
                    backend.move_entries(&sources.iter().map(|path| path.to_string_lossy().to_string()).collect::<Vec<_>>(), &destination)?
                } else {
                    match backend {
                        WorkspaceBackend::Local { root } => transfer::transfer(&root, &sources, &workspace_fs::relative_to(&root, Path::new(&destination))),
                        WorkspaceBackend::Remote(_) => return Err("Finder import is not supported on a relay".into()),
                    }
                };
                Ok((report, open_path, canonical_open))
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.explorer.busy = false;
                if this.project().id != project.id || this.project().path != root { return; }
                match result {
                    Ok((report, old_open, canonical_open)) => {
                        this.retarget_file_tabs(&report, cx);
                        if let Some(inspector) = this.inspector.as_mut()
                            && let Some(path) = remap(Path::new(&inspector.path), &report).or_else(|| {
                                (old_open.as_deref() == Some(inspector.path.as_str())).then(|| canonical_open.as_ref().and_then(|path| remap(path, &report))).flatten()
                            }) {
                            inspector.path = path.to_string_lossy().into();
                            inspector.name = path.file_name().unwrap_or_default().to_string_lossy().into();
                            inspector.directory = path.parent().map(|path| workspace_fs::relative_to(&root, path)).filter(|path| !path.is_empty()).unwrap_or_else(|| ".".into());
                        }
                        if this.diff_view.as_ref().is_some_and(|view| remap(Path::new(&view.path), &report).is_some()) { this.diff_view = None; }
                        if let Some((_, selected, _)) = this.explorer.selected.as_mut()
                            && let Some(path) = remap(Path::new(selected), &report) { *selected = path.to_string_lossy().into(); }
                        let expanded = this.file_tree.expanded_paths().into_iter().map(|path| remap(Path::new(&path), &report).map(|path| path.to_string_lossy().to_string()).unwrap_or(path));
                        this.explorer.expand_after_scan.extend(expanded);
                        this.explorer.expand_after_scan.push(target);
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                        let count = report.completed.len();
                        this.set_notice(report.error.map(|error| format!("Transferred {count} item(s); stopped: {error}")).unwrap_or_else(|| if count == 0 { "Nothing to move".into() } else { format!("Transferred {count} item(s); external originals were kept") }), cx);
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn initialize_repository(&mut self, cx: &mut Context<Self>) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        if self.git_loading {
            return;
        }
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let project = self.project().id.clone();
        self.git_loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.git_initialize() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.project().id != project {
                    return;
                }
                this.git_loading = false;
                match result {
                    Ok(()) => {
                        this.refresh_git_status(cx);
                        this.set_notice(
                            "Initialized a Git repository. No files were staged or committed.",
                            cx,
                        );
                    }
                    Err(error) => this.set_notice(error, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::remap;
    use dunlin_workspace::transfer::{TransferReport, Transferred};
    use std::path::Path;
    #[test]
    fn moved_editor_paths_are_retargeted_by_components_not_string_prefixes() {
        let report = TransferReport {
            completed: vec![Transferred {
                source: "/project/src".into(),
                destination: "/project/lib/src".into(),
                moved: true,
            }],
            error: None,
        };
        assert_eq!(
            remap(Path::new("/project/src/main.rs"), &report),
            Some("/project/lib/src/main.rs".into())
        );
        assert!(remap(Path::new("/project/src-other/main.rs"), &report).is_none());
        let copy = TransferReport {
            completed: vec![Transferred {
                source: "/external".into(),
                destination: "/project/external".into(),
                moved: false,
            }],
            error: None,
        };
        assert!(remap(Path::new("/external/main.rs"), &copy).is_none());
    }
}
