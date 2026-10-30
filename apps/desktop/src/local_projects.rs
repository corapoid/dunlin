//! Bulk import keeps existing identities/settings and never activates every
//! discovered repository. Only explicit top-level selections may be symlinks.
use crate::app::DunlinApp;
use dunlin_domain::{Project, ProjectTone};
use dunlin_ui::*;
use std::{collections::HashSet, path::PathBuf};

pub(crate) fn canonical_local_paths(projects: &[Project]) -> HashSet<PathBuf> {
    projects
        .iter()
        .filter(|p| !p.is_remote())
        .filter_map(|p| std::fs::canonicalize(&p.path).ok())
        .collect()
}
fn activate_import(was_empty: bool, single: bool, count: usize, same_navigation: bool) -> bool {
    same_navigation && (was_empty || (single && count == 1))
}
impl DunlinApp {
    pub(crate) fn import_local_folders(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.local_import_running {
            self.file_notice("Repository discovery is already running", cx);
            return;
        }
        self.local_import_running = true;
        let navigation = self.navigation_epoch();
        let single = paths.len() == 1;
        self.file_notice("Looking for local repositories…", cx);
        cx.spawn(async move |this, cx| {
            let discovery = cx.background_executor().spawn(async move { dunlin_workspace::repositories::discover(&paths) }).await;
            let _ = this.update(cx, |this, cx| {
                this.local_import_running = false;
                let mut existing = canonical_local_paths(&this.projects);
                let was_empty = this.projects.is_empty(); let mut first = None; let mut added = 0; let mut duplicate = 0; let mut failures = discovery.warnings;
                for candidate in discovery.projects {
                    if existing.contains(&candidate.path) {
                        duplicate += 1;
                        if first.is_none() { first = this.projects.iter().position(|p| !p.is_remote() && std::fs::canonicalize(&p.path).ok().as_ref() == Some(&candidate.path)); }
                        continue;
                    }
                    let path = candidate.path.to_string_lossy().into_owned();
                    let name = candidate.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
                    let tones = [ProjectTone::Blue, ProjectTone::Green, ProjectTone::Violet, ProjectTone::Amber, ProjectTone::Rose, ProjectTone::Teal];
                    let project = Project { id: format!("project-{}", uuid::Uuid::now_v7()), name, path, branch: candidate.branch, tone: tones[this.projects.len() % tones.len()], pinned: false, trusted: true, running_sessions: 0, remote: None, group_id: None, settings: Default::default() };
                    if let Some(store) = &this.store && let Err(error) = store.upsert_project(&project) { failures.push(error.to_string()); continue; }
                    existing.insert(candidate.path);
                    this.repo_summaries.values.insert(project.id.clone(), crate::project_groups::RepoSummary { path: project.path.clone(), branch: project.branch.clone(), changes: None, activity: Default::default(), monorepo: candidate.monorepo });
                    this.projects.push(project); first.get_or_insert(this.projects.len() - 1); added += 1;
                }
                this.repo_summaries.invalidate();
                // Do not interrupt the current project/buffers while adding a
                // collection. The normal single-project selection guard stays.
                if activate_import(was_empty, single, added + duplicate, this.navigation_epoch() == navigation) && let Some(index) = first { this.select_project(index, cx); }
                this.persist_settings(cx);
                let mut message = format!("Added {added} projects · {duplicate} already open");
                if discovery.limited { message.push_str(" · discovery limit reached; select smaller folders for the remainder (8 levels / 20,000 entries / 256 projects / 8 seconds)"); }
                if !failures.is_empty() { message.push_str(&format!(" · {} folders could not be imported: {}", failures.len(), failures[0])); }
                this.file_notice(message.clone(), cx);
                if discovery.limited || !failures.is_empty() {
                    // Partial import reports must remain available until the
                    // user dismisses them, not expire like a routine file toast.
                    this.toast = Some(crate::app::Toast { id: uuid::Uuid::now_v7(), message, action_label: "OK".into(), action: crate::app::ToastAction::Info });
                }
            });
        }).detach();
    }
}

pub(crate) fn monorepo_badge(
    this: &DunlinApp,
    project: &Project,
    p: &Palette,
) -> Option<AnyElement> {
    let kind = this
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|s| s.path == project.path)?
        .monorepo
        .clone()?;
    Some(div().flex().flex_shrink_0().child(div().id(format!("monorepo:{}", project.id)).px(px(4.)).py(px(1.)).rounded(px(4.)).border_1().border_color(p.border_strong).bg(p.muted).text_size(px(9.)).text_color(p.muted_fg).child("Monorepo")
        .tooltip(move |window, cx| component::tooltip::Tooltip::new(format!("Detected: {kind}. Packages stay inside this project; no scripts are executed.")).build(window, cx))).into_any_element())
}

#[cfg(test)]
mod tests {
    #[test]
    fn collection_imports_do_not_switch_every_project_or_interrupt_later_navigation() {
        assert!(!super::activate_import(false, true, 12, true));
        assert!(!super::activate_import(false, false, 2, true));
        assert!(super::activate_import(true, false, 12, true));
        assert!(super::activate_import(false, true, 1, true));
        assert!(!super::activate_import(true, true, 1, false));
    }
    #[cfg(unix)]
    #[test]
    fn canonical_duplicates_do_not_change_saved_project_settings_or_remote_identities() {
        let root = std::env::temp_dir().join(format!("dunlin-import-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&root).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let mut project = dunlin_domain::Project {
            id: "old".into(),
            name: "Custom name".into(),
            path: root.to_string_lossy().into(),
            branch: Some("main".into()),
            tone: dunlin_domain::ProjectTone::Green,
            pinned: true,
            trusted: false,
            running_sessions: 0,
            remote: None,
            group_id: Some("group".into()),
            settings: Default::default(),
        };
        project.settings.harness = Some("Custom harness".into());
        let mut duplicate = project.clone();
        duplicate.path = alias.to_string_lossy().into();
        let paths = super::canonical_local_paths(&[project.clone(), duplicate]);
        assert_eq!(paths.len(), 1);
        assert!(project.pinned);
        assert!(!project.trusted);
        assert_eq!(project.group_id.as_deref(), Some("group"));
        assert_eq!(project.settings.harness.as_deref(), Some("Custom harness"));
        project.remote = Some(dunlin_domain::RemoteProject::default());
        assert!(super::canonical_local_paths(&[project]).is_empty());
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
