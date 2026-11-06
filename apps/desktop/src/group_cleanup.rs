//! Explicit group-generated-data cleanup with a fresh, recoverable Trash plan.
use crate::app::DunlinApp;
use dunlin_ui::*;
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct State {
    pub busy: bool,
}
impl DunlinApp {
    fn cleanup_roots(&self, group: &str) -> Result<Vec<PathBuf>, String> {
        if !self.project_groups.iter().any(|g| g.id == group) {
            return Err("Group no longer exists".into());
        }
        let ids = crate::project_tree::descendants(&self.project_groups, group);
        let members: Vec<_> = self
            .projects
            .iter()
            .filter(|p| p.group_id.as_ref().is_some_and(|id| ids.contains(id)))
            .collect();
        if members.is_empty() {
            return Err("This group has no repositories".into());
        }
        if members
            .iter()
            .any(|p| !p.trusted || p.is_remote() || p.is_chat_space())
        {
            return Err("Clear requires trusted local repositories only".into());
        }
        Ok(members
            .into_iter()
            .map(|p| PathBuf::from(&p.path))
            .collect())
    }
    fn cleanup_guard(&self) -> Result<(), String> {
        if self.has_running_agents()
            || self.harness.status.is_busy()
            || self.git_history.running
            || self.branch_ui.running()
            || self.explorer.busy
            || self.local_import_running
            || self.ai_commit_preparing()
            || self
                .projects
                .iter()
                .any(|p| self.git_mutation_in_flight(&p.id))
            || self
                .terminal
                .tabs
                .iter()
                .any(|t| matches!(t.status, crate::terminal::Status::Running))
        {
            return Err(
                "Stop agents, terminal sessions and repository operations before Clear".into(),
            );
        }
        if self.has_dirty_tabs() {
            return Err("Save or close edited files before Clear".into());
        }
        Ok(())
    }
    pub(crate) fn preview_group_cleanup(
        &mut self,
        group: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.group_cleanup.busy {
            return;
        }
        let roots = match self
            .cleanup_guard()
            .and_then(|_| self.cleanup_roots(&group))
        {
            Ok(roots) => roots,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        self.group_cleanup.busy = true;
        let requested = roots.clone();
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_executor().spawn(async move { dunlin_workspace::cleanup::preview(&requested) }).await;
            let plan = match plan {
                Ok(plan) => plan, Err(error) => { let _ = this.update(cx, |this, cx| { this.group_cleanup.busy = false; this.file_notice(error, cx); }); return; }
            };
            let paths = plan.paths();
            let answer = this.update_in(cx, |this, window, cx| {
                if this.cleanup_roots(&group).ok().as_ref() != Some(&roots) || this.group_settings_draft.as_ref().map(|d| d.group_id.as_str()) != Some(&group) {
                    this.group_cleanup.busy = false; cx.notify(); return None;
                }
                if paths.is_empty() || paths.len() > 100 {
                    this.group_cleanup.busy = false;
                    this.file_notice(if paths.is_empty() { "No ignored dependency/cache folders can be cleared" } else { "More than 100 folders found. Choose a smaller group to review every path." }, cx);
                    return None;
                }
                let details = format!("Move these {} ignored, untracked generated folders to system Trash?\n\n{}\n\nSource, tracked files and Git metadata are not included. Stop external build/install tools too. Restore from Trash if needed; disk space is reclaimed when you empty it.", paths.len(), paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                let answer = window.prompt(PromptLevel::Warning, "Clear generated folders?", Some(&details), &["Cancel", "Move to Trash"], cx);
                Some(this.overlay_backdrop.track_native(answer, cx))
            }).ok().flatten();
            let approved = if let Some(answer) = answer { answer.await.ok() == Some(1) } else { false };
            if !approved { let _ = this.update(cx, |this, cx| { this.group_cleanup.busy = false; cx.notify(); }); return; }
            let valid = this.update(cx, |this, _| this.cleanup_roots(&group).ok().as_ref() == Some(&roots) && this.cleanup_guard().is_ok()
                && !this.workbench.tabs.iter().any(|tab| paths.iter().any(|path| {
                    let file = std::path::Path::new(&tab.inspector.path);
                    let file = if file.is_absolute() { file.to_path_buf() } else { PathBuf::from(&tab.root).join(file) };
                    file.starts_with(path)
                }))).unwrap_or(false);
            let result = if valid {
                cx.background_executor().spawn(async move { plan.apply(crate::platform::trash) }).await
            } else { Err("Group, activity or open files changed. Close affected files and preview Clear again.".into()) };
            let _ = this.update(cx, |this, cx| {
                this.group_cleanup.busy = false;
                this.file_notice(match result { Ok(count) => format!("Moved {count} generated folders to Trash"), Err(error) => error }, cx);
                this.refresh_file_tree(cx);
            });
        }).detach();
        cx.notify();
    }
}
