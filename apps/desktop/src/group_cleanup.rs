//! Project/group generated-data cleanup, always through recoverable system Trash.
use crate::app::DunlinApp;
use dunlin_domain::{Project, ProjectGroup};
use dunlin_ui::*;
use dunlin_workspace::cleanup::{Folder, Plan, Skipped};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod view;
pub(crate) use view::render;
#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Default)]
pub(crate) struct State {
    /// Kept across inspection, review and confirmation to exclude conflicting work.
    pub busy: bool,
    pub dialog: Option<Dialog>,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone)]
enum Scope {
    Project(String),
    Group(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    id: String,
    name: String,
    path: PathBuf,
    trusted: bool,
    remote: bool,
    chat: bool,
    pending_clone: bool,
}

fn members(
    scope: &Scope,
    projects: &[Project],
    groups: &[ProjectGroup],
) -> Result<Vec<Member>, String> {
    let ids = match scope {
        Scope::Project(id) => {
            if !projects.iter().any(|p| p.id == *id) {
                return Err("Project no longer exists".into());
            }
            None
        }
        Scope::Group(id) => {
            if !groups.iter().any(|g| g.id == *id) {
                return Err("Group no longer exists".into());
            }
            Some(crate::project_tree::descendants(groups, id))
        }
    };
    let mut members: Vec<_> = projects
        .iter()
        .filter(|p| match scope {
            Scope::Project(id) => p.id == *id,
            Scope::Group(_) => p
                .group_id
                .as_ref()
                .is_some_and(|id| ids.as_ref().unwrap().contains(id)),
        })
        .map(|p| Member {
            id: p.id.clone(),
            name: p.name.clone(),
            path: PathBuf::from(&p.path),
            trusted: p.trusted,
            remote: p.is_remote(),
            chat: p.is_chat_space(),
            pending_clone: p.is_not_downloaded(),
        })
        .collect();
    members.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(members)
}

fn roots(members: &[Member]) -> (Vec<PathBuf>, Vec<Skipped>) {
    let mut roots = Vec::new();
    let mut skipped = Vec::new();
    for member in members {
        let reason = if member.chat {
            Some("Standalone chat, not a repository")
        } else if member.remote {
            Some("Remote project; cleanup is local-only")
        } else if !member.trusted {
            Some("Project is not trusted")
        } else if member.pending_clone {
            Some("Project has not been downloaded")
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped.push(Skipped {
                path: member.path.clone(),
                reason: format!("{} · {reason}", member.name),
            });
        } else {
            roots.push(member.path.clone());
        }
    }
    (roots, skipped)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Inspecting,
    Reviewing,
    Confirming,
    Moving,
    Finished,
}

pub(crate) struct Dialog {
    scope: Scope,
    members: Vec<Member>,
    title: String,
    phase: Phase,
    plan: Plan,
    folders: Vec<Folder>,
    selected: Vec<bool>,
    skipped: Vec<Skipped>,
    moved: Vec<PathBuf>,
    error: Option<String>,
    pub focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl Dialog {
    fn total(&self) -> (usize, u64) {
        selection_total(&self.folders, &self.selected)
    }

    fn locked(&self) -> bool {
        matches!(self.phase, Phase::Confirming | Phase::Moving)
    }

    fn row_count(&self) -> usize {
        (if self.phase == Phase::Finished {
            self.moved.len()
        } else {
            self.folders.len()
        }) + self.skipped.len()
    }
}

fn selection_total(folders: &[Folder], selected: &[bool]) -> (usize, u64) {
    folders
        .iter()
        .zip(selected)
        .filter(|(_, selected)| **selected)
        .fold((0, 0u64), |(count, bytes), (folder, _)| {
            (count + 1, bytes.saturating_add(folder.bytes))
        })
}

impl DunlinApp {
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
                "Stop agents, terminal sessions and repository operations before cleanup".into(),
            );
        }
        if self.has_dirty_tabs() {
            return Err("Save or close edited files before cleanup".into());
        }
        Ok(())
    }

    pub(crate) fn preview_group_cleanup(
        &mut self,
        group: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview_cleanup(Scope::Group(group), window, cx);
    }

    pub(crate) fn preview_project_cleanup(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview_cleanup(Scope::Project(project), window, cx);
    }

    fn preview_cleanup(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.group_cleanup.busy || self.group_cleanup.dialog.is_some() {
            return;
        }
        let snapshot = match self
            .cleanup_guard()
            .and_then(|()| members(&scope, &self.projects, &self.project_groups))
        {
            Ok(members) => members,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let title = match &scope {
            Scope::Project(_) => format!("Clean project · {}", snapshot[0].name),
            Scope::Group(id) => format!(
                "Clean group · {}",
                crate::project_tree::group_path(&self.project_groups, id)
            ),
        };
        let (roots, skipped) = roots(&snapshot);
        let cancel = Arc::new(AtomicBool::new(false));
        self.group_cleanup.generation += 1;
        let generation = self.group_cleanup.generation;
        self.group_cleanup.cancel = Some(cancel.clone());
        self.group_cleanup.busy = true;
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.group_cleanup.dialog = Some(Dialog {
            scope,
            members: snapshot,
            title,
            phase: Phase::Inspecting,
            plan: Plan::default(),
            folders: Vec::new(),
            selected: Vec::new(),
            skipped,
            moved: Vec::new(),
            error: None,
            focus,
            scroll: UniformListScrollHandle::new(),
        });
        self.context_menu = None;
        self.project_menu_open = false;
        cx.spawn(async move |this, cx| {
            let preview = cx
                .background_executor()
                .spawn(async move { dunlin_workspace::cleanup::inspect(&roots, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.group_cleanup.generation != generation {
                    return;
                }
                let Some(dialog) = &mut this.group_cleanup.dialog else {
                    return;
                };
                if members(&dialog.scope, &this.projects, &this.project_groups)
                    .ok()
                    .as_ref()
                    != Some(&dialog.members)
                {
                    dialog.phase = Phase::Finished;
                    dialog.error = Some(
                        "Project membership or trust changed. Close and inspect again.".into(),
                    );
                    this.group_cleanup.busy = false;
                } else {
                    dialog.folders = preview.plan.folders();
                    dialog.selected = vec![true; dialog.folders.len()];
                    dialog.plan = preview.plan;
                    dialog.skipped.extend(preview.skipped);
                    dialog.phase = Phase::Reviewing;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn close_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .group_cleanup
            .dialog
            .as_ref()
            .is_some_and(Dialog::locked)
        {
            return;
        }
        if let Some(cancel) = self.group_cleanup.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.group_cleanup.generation += 1;
        self.group_cleanup.dialog = None;
        self.group_cleanup.busy = false;
        self.overlay_focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn validate_cleanup_context(&self, paths: &[PathBuf]) -> Result<(), String> {
        let dialog = self.group_cleanup.dialog.as_ref().ok_or("Cleanup closed")?;
        if members(&dialog.scope, &self.projects, &self.project_groups)? != dialog.members {
            return Err(
                "Project membership, paths or trust changed. Close and inspect again.".into(),
            );
        }
        self.cleanup_guard()?;
        if self.workbench.tabs.iter().any(|tab| {
            let file = std::path::Path::new(&tab.inspector.path);
            let file = if file.is_absolute() {
                file.to_path_buf()
            } else {
                PathBuf::from(&tab.root).join(file)
            };
            let canonical = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            paths
                .iter()
                .any(|path| file.starts_with(path) || canonical.starts_with(path))
        }) {
            return Err("Close files inside the selected folders before cleanup.".into());
        }
        Ok(())
    }

    fn confirm_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.group_cleanup.dialog else {
            return;
        };
        if dialog.phase != Phase::Reviewing || dialog.total().0 == 0 {
            return;
        }
        let plan = match dialog.plan.selected(&dialog.selected) {
            Ok(plan) => plan,
            Err(error) => {
                self.group_cleanup.dialog.as_mut().unwrap().error = Some(error);
                cx.notify();
                return;
            }
        };
        let paths = plan.paths();
        if let Err(error) = self.validate_cleanup_context(&paths) {
            self.group_cleanup.dialog.as_mut().unwrap().error = Some(error);
            cx.notify();
            return;
        }
        let dialog = self.group_cleanup.dialog.as_mut().unwrap();
        let (count, bytes) = dialog.total();
        dialog.phase = Phase::Confirming;
        dialog.error = None;
        let details = format!(
            "Move {count} selected folders (approximately {}) to system Trash?\n\nAll contents of the selected folders will move, including any custom files you placed inside them. Dependencies/builds will need to be recreated. Stop external build and install tools first. Tracked files and Git metadata are excluded.\n\nYou can restore folders from Trash. Disk space is reclaimed only when you empty Trash; Dunlin never empties it.",
            crate::app::format_size(bytes)
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Move generated folders to Trash?",
            Some(&details),
            &["Cancel", "Move to Trash"],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        let generation = self.group_cleanup.generation;
        cx.spawn(async move |this, cx| {
            let approved = answer.await.ok() == Some(1);
            let start = this
                .update(cx, |this, cx| {
                    if this.group_cleanup.generation != generation {
                        return false;
                    }
                    if !approved {
                        if let Some(dialog) = &mut this.group_cleanup.dialog {
                            dialog.phase = Phase::Reviewing;
                        }
                        cx.notify();
                        return false;
                    }
                    if let Err(error) = this.validate_cleanup_context(&paths) {
                        if let Some(dialog) = &mut this.group_cleanup.dialog {
                            dialog.phase = Phase::Reviewing;
                            dialog.error = Some(error);
                        }
                        cx.notify();
                        return false;
                    }
                    let dialog = this.group_cleanup.dialog.as_mut().unwrap();
                    dialog.phase = Phase::Moving;
                    dialog.skipped.extend(
                        dialog
                            .folders
                            .iter()
                            .zip(&dialog.selected)
                            .filter(|(_, selected)| !**selected)
                            .map(|(folder, _)| Skipped {
                                path: folder.path.clone(),
                                reason: "Not selected".into(),
                            }),
                    );
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !start {
                return;
            }
            let outcome = cx
                .background_executor()
                .spawn(async move { plan.apply_available(crate::platform::trash) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.group_cleanup.generation != generation {
                    return;
                }
                this.group_cleanup.busy = false;
                if let Some(dialog) = &mut this.group_cleanup.dialog {
                    dialog.phase = Phase::Finished;
                    dialog.moved = outcome.moved;
                    dialog.skipped.extend(outcome.skipped);
                    dialog.scroll.scroll_to_item(0, ScrollStrategy::Top);
                }
                this.refresh_file_tree(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, group: Option<&str>) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: format!("/projects/{id}"),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: group.map(str::to_owned),
            settings: Default::default(),
        }
    }

    fn group(id: &str, parent: Option<&str>) -> ProjectGroup {
        ProjectGroup {
            id: id.into(),
            name: id.into(),
            parent_id: parent.map(str::to_owned),
            tone: dunlin_domain::ProjectTone::Blue,
            settings: Default::default(),
        }
    }

    #[test]
    fn group_scope_includes_descendants_but_not_peers_or_ungrouped_projects() {
        let groups = vec![
            group("root", None),
            group("child", Some("root")),
            group("peer", None),
        ];
        let projects = vec![
            project("direct", Some("root")),
            project("nested", Some("child")),
            project("outside", Some("peer")),
            project("ungrouped", None),
        ];
        let snapshot = members(&Scope::Group("root".into()), &projects, &groups).unwrap();
        assert_eq!(
            snapshot.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["direct", "nested"]
        );
        let single = members(&Scope::Project("nested".into()), &projects, &groups).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].id, "nested");
        assert!(members(&Scope::Group("missing".into()), &projects, &groups).is_err());
        assert!(members(&Scope::Project("missing".into()), &projects, &groups).is_err());
        assert!(
            members(
                &Scope::Group("empty".into()),
                &projects,
                &[group("empty", None)]
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn unsupported_members_are_reported_without_blocking_trusted_local_projects() {
        let mut remote = project("remote", Some("root"));
        remote.remote = Some(dunlin_domain::RemoteProject::default());
        let mut untrusted = project("untrusted", Some("root"));
        untrusted.trusted = false;
        let mut chat = project("chat", Some("root"));
        chat.kind = dunlin_domain::ProjectKind::ChatSpace;
        let projects = vec![remote, untrusted, chat, project("local", Some("root"))];
        let snapshot = members(
            &Scope::Group("root".into()),
            &projects,
            &[group("root", None)],
        )
        .unwrap();
        let (roots, skipped) = roots(&snapshot);
        assert_eq!(roots, vec![PathBuf::from("/projects/local")]);
        assert_eq!(skipped.len(), 3);
        assert!(skipped.iter().any(|s| s.reason.contains("local-only")));
        assert!(skipped.iter().any(|s| s.reason.contains("not trusted")));
        assert!(skipped.iter().any(|s| s.reason.contains("Standalone chat")));
    }

    #[test]
    fn snapshot_is_stable_under_sorting_but_detects_path_trust_and_membership_changes() {
        let groups = vec![group("root", None)];
        let scope = Scope::Group("root".into());
        let mut projects = vec![project("a", Some("root")), project("b", Some("root"))];
        let original = members(&scope, &projects, &groups).unwrap();
        projects.reverse();
        assert_eq!(members(&scope, &projects, &groups).unwrap(), original);
        projects[0].trusted = false;
        assert_ne!(members(&scope, &projects, &groups).unwrap(), original);
        projects[0].trusted = true;
        projects[0].path = "/other".into();
        assert_ne!(members(&scope, &projects, &groups).unwrap(), original);
        projects[0].path = "/projects/b".into();
        projects[0].group_id = None;
        assert_ne!(members(&scope, &projects, &groups).unwrap(), original);
    }

    #[test]
    fn totals_follow_selection_without_overflow() {
        let folders = vec![
            Folder {
                path: "/a".into(),
                kind: dunlin_workspace::cleanup::Kind::Build,
                bytes: 100,
            },
            Folder {
                path: "/b".into(),
                kind: dunlin_workspace::cleanup::Kind::Cache,
                bytes: 200,
            },
        ];
        assert_eq!(selection_total(&folders, &[true, false]), (1, 100));
        assert_eq!(selection_total(&folders, &[true, true]), (2, 300));
        assert_eq!(selection_total(&folders, &[false, false]), (0, 0));
        let large = vec![
            Folder {
                bytes: u64::MAX,
                ..folders[0].clone()
            },
            folders[1].clone(),
        ];
        assert_eq!(selection_total(&large, &[true, true]), (2, u64::MAX));
    }
}
