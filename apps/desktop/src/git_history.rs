//! Project-owned Git history tab, independent of chat, files and the agent graph.
use crate::app::DunlinApp;
use crate::remote::{RemoteHost, WorkspaceBackend};
use dunlin_domain::Project;
use dunlin_ui::component::Disableable;
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        diff::{Diff, DiffFile, DiffState},
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};
use dunlin_workspace::git::history::{self, Commit};
use std::path::Path;

mod budgets;
mod checks;
mod compare;
mod context;
mod details;
mod github;
mod impact;
mod mutations;
mod overview;
mod review;
#[cfg(debug_assertions)]
pub(crate) mod smoke;
mod subtasks;
#[cfg(debug_assertions)]
mod workbench_smoke;

#[derive(Default)]
pub(crate) struct State {
    pub view: Option<View>,
    pub running: bool,
    pub active: bool,
    mutation_project: Option<String>,
    mutation_global: bool,
    pub(crate) cancel_check: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl State {
    pub(crate) fn tab_label(&self) -> &'static str {
        if let Some(v) = &self.view {
            if v.github.visible {
                return "GitHub · CI / PR";
            }
            if v.context.visible {
                return "Context & decisions";
            }
            if v.review.visible {
                return "Task review";
            }
            if v.checks.visible {
                return "Checks & problems";
            }
            if v.budgets.visible {
                return "Task budgets";
            }
            if v.subtasks.visible {
                return "Subtask worktrees";
            }
            if v.compare.visible {
                return "Compare solutions";
            }
        }
        "Git history"
    }
    pub(crate) fn locks(&self, project: &str) -> bool {
        self.running && (self.mutation_global || self.mutation_project.as_deref() == Some(project))
    }
    pub(crate) fn is_open_for(&self, project: &Project) -> bool {
        self.view
            .as_ref()
            .is_some_and(|view| owns(&view.project, project))
    }
    pub(crate) fn is_active_for(&self, project: &Project) -> bool {
        self.active && self.is_open_for(project)
    }
    pub(crate) fn prune(&mut self, projects: &[Project], hosts: &[RemoteHost]) {
        if self.view.as_ref().is_some_and(|view| {
            !projects.iter().any(|project| owns(&view.project, project))
                || !backend_current(&view.backend, hosts)
        }) {
            self.view = None;
            self.active = false;
        }
    }
}
pub(crate) struct View {
    id: uuid::Uuid,
    project: Project,
    backend: WorkspaceBackend,
    overview: overview::State,
    rows: Vec<Commit>,
    selected: Option<usize>,
    name: Entity<InputState>,
    pub(crate) revision: Entity<InputState>,
    branch_form_open: bool,
    error: Option<String>,
    loading: bool,
    limit: usize,
    scope: Option<String>,
    load_request: uuid::Uuid,
    detail_request: uuid::Uuid,
    patch_request: uuid::Uuid,
    detail: Option<history::CommitDetails>,
    detail_error: Option<String>,
    patch_note: Option<String>,
    parent: usize,
    selected_file: Option<usize>,
    detail_loading: bool,
    patch_loading: bool,
    diff: Entity<DiffState>,
    lanes: Vec<LaneRow>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    mutations: mutations::Form,
    github: github::State,
    context: context::State,
    review: review::State,
    checks: checks::State,
    budgets: budgets::State,
    subtasks: subtasks::State,
    compare: compare::State,
    impact: impact::State,
}
impl View {
    fn hide_tools(&mut self) {
        self.impact.visible = false;
        self.github.visible = false;
        self.context.visible = false;
        self.review.visible = false;
        self.checks.visible = false;
        self.budgets.visible = false;
        self.subtasks.visible = false;
        self.compare.visible = false;
    }
}

fn owns(owner: &Project, project: &Project) -> bool {
    let same_environment = match (&owner.remote, &project.remote) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.host_id == b.host_id && a.project_id == b.project_id && a.path == b.path
        }
        _ => false,
    };
    owner.id == project.id && owner.path == project.path && project.trusted && same_environment
}

fn backend_current(backend: &WorkspaceBackend, hosts: &[RemoteHost]) -> bool {
    match backend {
        WorkspaceBackend::Local { .. } => true,
        WorkspaceBackend::Remote(remote) => hosts.iter().any(|host| {
            host.id == remote.host.id
                && host.url == remote.host.url
                && host.device_id == remote.host.device_id
        }),
    }
}

// Graph and inventory may finish in either order after an initial read. Neither
// may replace a scope pinned by the other while its request was in flight.
fn pin_scope(scope: &mut Option<String>, incoming: String) -> Result<(), String> {
    if scope.as_ref().is_some_and(|scope| scope != &incoming) {
        return Err("The repository scope changed. Close and reopen Git history.".into());
    }
    *scope = Some(incoming);
    Ok(())
}

/// Each active lane owns an expected ancestor. Edges connect every visible
/// parent, including merge parents; unshown ancestors continue below the limit.
#[derive(Clone, Debug)]
struct LaneRow {
    lane: usize,
    edges: Vec<(usize, usize)>,
    width: usize,
    incoming: usize,
}
fn layout(commits: &[Commit]) -> Vec<LaneRow> {
    let mut lanes: Vec<String> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let incoming = lanes.len();
            let lane = lanes
                .iter()
                .position(|id| id == &commit.oid)
                .unwrap_or_else(|| {
                    lanes.push(commit.oid.clone());
                    lanes.len() - 1
                });
            let before = lanes.clone();
            lanes.remove(lane);
            for (offset, parent) in commit.parents.iter().enumerate() {
                if !lanes.contains(parent) {
                    lanes.insert((lane + offset).min(lanes.len()), parent.clone());
                }
            }
            let mut edges = Vec::new();
            for (column, id) in before.iter().enumerate() {
                if id == &commit.oid {
                    for parent in &commit.parents {
                        if let Some(to) = lanes.iter().position(|id| id == parent) {
                            edges.push((column, to));
                        }
                    }
                } else if let Some(to) = lanes.iter().position(|next| next == id) {
                    edges.push((column, to));
                }
            }
            LaneRow {
                lane,
                width: before.len().max(lanes.len()),
                edges,
                incoming,
            }
        })
        .collect()
}

impl DunlinApp {
    pub(crate) fn selected_github_evidence(&self) -> Vec<(String, String, String)> {
        self.git_history
            .view
            .as_ref()
            .map(|view| view.github.attachable_evidence())
            .unwrap_or_default()
    }

    pub(crate) fn open_git_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        let project = self.project().clone();
        self.git_history.prune(&self.projects, &self.remote_hosts);
        if !project.trusted {
            self.file_notice("Trust the project before reading Git history", cx);
            return;
        }
        if project.is_chat_space()
            && !project.is_remote()
            && !Path::new(&project.path).join(".git").exists()
        {
            self.file_notice(
                "This scratch chat has no Git repository. Use New Chat in Repository… for an explicit repository context.",
                cx,
            );
            return;
        }
        if self.git_history.running && !self.git_history.is_open_for(&project) {
            self.file_notice(
                "Wait for the repository operation to finish before opening another history tab",
                cx,
            );
            return;
        }
        self.branch_ui.dismiss();
        self.project_settings_target = None;
        self.project_runtime_draft = None;
        self.group_settings_draft = None;
        self.context_menu = None;
        self.project_menu_open = false;
        self.settings_open = false;
        self.command_open = false;
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.spec.active = false;
        self.section = dunlin_domain::SidebarSection::SourceControl;
        if self.git_history.is_open_for(&project) {
            self.git_history.active = true;
            if let Some(view) = &self.git_history.view {
                view.focus.focus(window, cx);
            }
            cx.notify();
            return;
        }
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        self.git_history.view = Some(View {
            id: uuid::Uuid::now_v7(),
            project,
            backend,
            overview: overview::State::default(),
            rows: Vec::new(),
            selected: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("feature/new-branch")),
            revision: cx.new(|cx| {
                InputState::new(window, cx).placeholder("All refs, or enter commit / branch / tag")
            }),
            branch_form_open: false,
            error: None,
            loading: false,
            limit: 500,
            scope: None,
            load_request: uuid::Uuid::now_v7(),
            detail_request: uuid::Uuid::now_v7(),
            patch_request: uuid::Uuid::now_v7(),
            detail: None,
            detail_error: None,
            patch_note: None,
            parent: 0,
            selected_file: None,
            detail_loading: false,
            patch_loading: false,
            diff: cx.new(|cx| DiffState::new(Vec::new(), cx)),
            lanes: Vec::new(),
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            mutations: mutations::Form::new(window, cx),
            github: github::State::new(window, cx),
            context: context::State::new(window, cx),
            review: review::State::new(window, cx),
            checks: checks::State::new(window, cx),
            budgets: budgets::State::new(window, cx),
            subtasks: subtasks::State::new(window, cx),
            compare: compare::State::new(window, cx),
            impact: impact::State::new(window, cx),
        });
        self.git_history.active = true;
        self.git_history
            .view
            .as_ref()
            .unwrap()
            .focus
            .focus(window, cx);
        self.load_git_history(cx);
    }
    pub(crate) fn close_git_history(&mut self, cx: &mut Context<Self>) {
        // Mutation ownership lives outside the view, so closing a read tab must
        // never release the in-flight ref lock or the quit guard.
        self.git_history.active = false;
        self.git_history.view = None;
        cx.notify();
    }
    pub(crate) fn load_git_history(&mut self, cx: &mut Context<Self>) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        view.loading = true;
        view.rows.clear();
        view.lanes.clear();
        view.selected = None;
        view.detail = None;
        view.detail_loading = false;
        view.patch_loading = false;
        view.detail_request = uuid::Uuid::now_v7();
        view.patch_request = uuid::Uuid::now_v7();
        view.patch_note = None;
        view.detail_error = None;
        view.error = None;
        view.load_request = uuid::Uuid::now_v7();
        let request = view.load_request;
        let expected = view.scope.clone();
        let backend = view.backend.clone();
        let project = view.project.clone();
        let (id, limit) = (view.id, view.limit);
        let revision = view.revision.read(cx).value().to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let response = backend.history_load(&revision, limit, expected.as_deref())?;
                    let rows = response.value;
                    let lanes = layout(&rows);
                    Ok::<_, String>((response.scope, rows, lanes))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.prune(&this.projects, &this.remote_hosts);
                if !this.projects.iter().any(|p| owns(&project, p)) { return; }
                let mut select = false;
                if let Some(view) = this.git_history.view.as_mut().filter(|v| v.id == id && v.load_request == request) {
                    view.loading = false;
                    match result {
                        Ok((root, rows, lanes)) => {
                            if let Err(error) = pin_scope(&mut view.scope, root) {
                                view.error = Some(error);
                                cx.notify();
                                return;
                            }
                            view.lanes = lanes;
                            view.rows = rows;
                            view.scroll.set_offset(point(px(0.), px(0.)));
                            select = !view.rows.is_empty();
                            view.error = None;
                        }
                        Err(error) => {
                            view.error = Some(format!("{error}. Check the commit/ref or whether the repository has commits. No fetch was performed."))
                        }
                    }
                }
                if select { this.select_history_commit(0, 0, cx); }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_history_commit(&mut self, index: usize, parent: usize, cx: &mut Context<Self>) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        let Some(commit) = view.rows.get(index) else {
            return;
        };
        let Some(expected) = view.scope.clone() else {
            return;
        };
        let oid = commit.oid.clone();
        let project = view.project.clone();
        let backend = view.backend.clone();
        let id = view.id;
        view.selected = Some(index);
        view.scroll.scroll_to_item(index);
        view.parent = parent;
        view.detail = None;
        view.detail_error = None;
        view.detail_loading = true;
        view.patch_loading = false;
        view.selected_file = None;
        view.patch_note = None;
        view.diff
            .update(cx, |state, cx| state.set_files(Vec::new(), cx));
        view.detail_request = uuid::Uuid::now_v7();
        view.patch_request = uuid::Uuid::now_v7();
        let request = view.detail_request;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(80))
                .await;
            let current = this
                .update(cx, |this, _| {
                    this.git_history.prune(&this.projects, &this.remote_hosts);
                    this.git_history.view.as_ref().is_some_and(|view| {
                        view.id == id
                            && view.detail_request == request
                            && this
                                .projects
                                .iter()
                                .any(|project| owns(&view.project, project))
                    })
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { backend.history_details(&expected, &oid, parent) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.prune(&this.projects, &this.remote_hosts);
                if !this.projects.iter().any(|p| owns(&project, p)) {
                    return;
                }
                let mut select_file = false;
                if let Some(view) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|view| view.id == id && view.detail_request == request)
                {
                    view.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            select_file = !detail.files.is_empty();
                            view.detail = Some(detail);
                        }
                        Err(error) => view.detail_error = Some(error),
                    }
                }
                if select_file {
                    this.select_history_file(0, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_history_file(&mut self, index: usize, cx: &mut Context<Self>) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        let Some(detail) = &view.detail else {
            return;
        };
        let Some(file) = detail.files.get(index).cloned() else {
            return;
        };
        let Some(expected) = view.scope.clone() else {
            return;
        };
        let (id, oid, parent, project) = (
            view.id,
            detail.commit.oid.clone(),
            view.parent,
            view.project.clone(),
        );
        let backend = view.backend.clone();
        view.selected_file = Some(index);
        view.patch_loading = true;
        view.patch_note = None;
        view.diff
            .update(cx, |state, cx| state.set_files(Vec::new(), cx));
        view.patch_request = uuid::Uuid::now_v7();
        let request = view.patch_request;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(80))
                .await;
            let current = this
                .update(cx, |this, _| {
                    this.git_history.prune(&this.projects, &this.remote_hosts);
                    this.git_history.view.as_ref().is_some_and(|view| {
                        view.id == id
                            && view.patch_request == request
                            && this
                                .projects
                                .iter()
                                .any(|project| owns(&view.project, project))
                    })
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    let patch = backend.history_patch(&expected, &oid, parent, index, &file)?;
                    if !patch.contains("\n@@ ") {
                        return Ok((
                            Vec::new(),
                            Some(if patch.is_empty() {
                                "No patch for this comparison".into()
                            } else {
                                patch
                            }),
                        ));
                    }
                    let files = DiffFile::parse(&patch)
                        .map_err(|e| format!("Could not display this commit patch: {e}"))?;
                    if files.is_empty() {
                        return Ok((files, Some(patch)));
                    }
                    Ok::<_, String>((files, None))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.prune(&this.projects, &this.remote_hosts);
                if !this.projects.iter().any(|p| owns(&project, p)) {
                    return;
                }
                if let Some(view) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|view| view.id == id && view.patch_request == request)
                {
                    view.patch_loading = false;
                    match result {
                        Ok((files, note)) => {
                            view.diff.update(cx, |state, cx| state.set_files(files, cx));
                            view.patch_note = note;
                        }
                        Err(error) => view.patch_note = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn checkout_history_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.git_history.view.as_ref() else {
            return;
        };
        if view.project.is_remote() {
            self.file_notice(
                "Commit checkout is local-only; remote history remains read-only",
                cx,
            );
            return;
        }
        let Some(commit) = view.selected.and_then(|i| view.rows.get(i)) else {
            return;
        };
        let project = view.project.clone();
        let id = view.id;
        let oid = commit.oid.clone();
        let short = oid.chars().take(10).collect::<String>();
        let Some(scope) = view.scope.clone() else {
            return;
        };
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let blocked = self.git_history.running
            || self.branch_ui.running()
            || self.git_mutation_in_flight(&project.id)
            || self.ai_commit_locks_project_id(&project.id)
            || self.explorer.busy
            || self.editor_dirty
            || self
                .workbench
                .tabs
                .iter()
                .any(|tab| tab.project == project.id && (tab.loading || tab.dirty))
            || !self.projects.iter().any(|p| {
                p.id == project.id && p.path == project.path && p.trusted && !p.is_remote()
            })
            || self.live_agents().iter().any(|(owner, session, _)| {
                owner == &project.id
                    || self
                        .group_contexts
                        .get(session)
                        .is_some_and(|g| g.roots.iter().any(|r| r.id == project.id))
            });
        if blocked {
            if let Some(v) = &mut self.git_history.view {
                v.error = Some("Save buffers and finish repository operations/agents first; commit checkout requires a clean worktree".into());
            }
            cx.notify();
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Checkout this commit?",
            Some(&format!(
                "Detach HEAD at {short} in {}? The worktree must be clean; no files are discarded or stashed. You can return to a branch with the branch picker or create a branch from this commit.",
                project.name
            )),
            &["Cancel", "Checkout commit"],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(1) {
                return;
            }
            let started = this
                .update(cx, |this, cx| {
                    if !this
                        .projects
                        .iter()
                        .any(|p| p.id == project.id && p.path == project.path && p.trusted)
                        || this.git_history.running
                        || this.branch_ui.running()
                        || this.editor_dirty
                        || this
                            .workbench
                            .tabs
                            .iter()
                            .any(|tab| tab.project == project.id && (tab.loading || tab.dirty))
                    {
                        return false;
                    }
                    this.git_history.running = true;
                    this.git_history.mutation_project = Some(project.id.clone());
                    this.begin_git_mutation(&project.id);
                    this.stash_editor_tab();
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !started {
                return;
            }
            let backend = backend.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend.checkout_commit_at(&scope, &oid) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.git_history.running = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                match result {
                    Ok(()) => {
                        this.repo_summaries.invalidate();
                        this.close_inspector();
                        this.workbench
                            .tabs
                            .retain(|tab| tab.project != project.id || tab.dirty);
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                        this.editor_stale = true;
                        this.file_notice(
                            format!(
                                "Checked out {short} in {} · detached HEAD; branches unchanged",
                                project.name
                            ),
                            cx,
                        );
                        if this.git_history.view.as_ref().is_some_and(|v| v.id == id) {
                            this.load_git_history(cx);
                        }
                        window.request_animation_frame();
                    }
                    Err(error) => {
                        if let Some(v) = this.git_history.view.as_mut().filter(|v| v.id == id) {
                            v.error = Some(error.clone());
                        }
                        this.file_notice(
                            format!("Could not checkout commit in {}: {error}", project.name),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_history_branch(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.git_history.view.as_ref() else {
            return;
        };
        let Some(commit) = view.selected.and_then(|i| view.rows.get(i)) else {
            return;
        };
        let project = view.project.clone();
        let id = view.id;
        let oid = commit.oid.clone();
        let name = view.name.read(cx).value().to_string();
        let Some(expected) = view.scope.clone() else {
            return;
        };
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let blocked = self.git_history.running
            || self.branch_ui.running()
            || self.git_mutation_in_flight(&project.id)
            || self.ai_commit_locks_project_id(&project.id)
            || self.explorer.busy
            || !self
                .projects
                .iter()
                .any(|p| p.id == project.id && p.path == project.path && p.trusted)
            || self.live_agents().iter().any(|(owner, session, _)| {
                owner == &project.id
                    || self
                        .group_contexts
                        .get(session)
                        .is_some_and(|g| g.roots.iter().any(|r| r.id == project.id))
            });
        if blocked {
            if let Some(v) = &mut self.git_history.view {
                v.error = Some("Finish repository operations and agents first; reopen history if the project changed".into());
            }
            cx.notify();
            return;
        }
        self.git_history.running = true;
        self.git_history.mutation_project = Some(project.id.clone());
        self.begin_git_mutation(&project.id);
        cx.spawn(async move |this, cx| {
            let branch = name.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend.create_branch_at(&expected, &oid, &branch) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                match result {
                    Ok(()) => {
                        this.repo_summaries.invalidate();
                        this.file_notice(
                            format!(
                                "Created {name} in {}; working branch unchanged",
                                project.name
                            ),
                            cx,
                        );
                        if this.git_history.view.as_ref().is_some_and(|v| v.id == id) {
                            this.load_git_history(cx);
                        }
                    }
                    Err(error) => {
                        if let Some(v) = this.git_history.view.as_mut().filter(|v| v.id == id) {
                            v.error = Some(error.clone());
                        }
                        this.file_notice(
                            format!("Could not create branch in {}: {error}", project.name),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn graph_cell(row: LaneRow, width: f32, color: Hsla) -> AnyElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let paint = |window: &mut Window, x, y, w, h, radius| {
                window.paint_quad(quad(
                    Bounds::new(bounds.origin + point(px(x), px(y)), size(px(w), px(h))),
                    Corners::all(px(radius)),
                    color,
                    Edges::default(),
                    color,
                    BorderStyle::default(),
                ))
            };
            for (from, to) in &row.edges {
                let a = 12. + *from as f32 * 16.;
                let b = 12. + *to as f32 * 16.;
                paint(window, a, 16., 1.5, 8., 0.);
                paint(window, a.min(b), 24., (a - b).abs().max(1.5), 1.5, 0.);
                paint(window, b, 24., 1.5, 8., 0.);
            }
            // Incoming lanes from the preceding row end at this row's center.
            for lane in 0..row.incoming {
                paint(window, 12. + lane as f32 * 16., 0., 1.5, 16., 0.);
            }
            paint(window, 8.5 + row.lane as f32 * 16., 12.5, 8., 8., 4.);
        },
    )
    .w(px(width))
    .h(px(32.))
    .flex_shrink_0()
    .into_any_element()
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.impact.visible)
    {
        impact::refresh_visible(this, cx);
        return impact::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.compare.visible)
    {
        return compare::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.subtasks.visible)
    {
        return subtasks::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.budgets.visible)
    {
        return budgets::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.checks.visible)
    {
        return checks::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.review.visible)
    {
        return review::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.context.visible)
    {
        return context::render(this, cx);
    }
    if this
        .git_history
        .view
        .as_ref()
        .is_some_and(|v| v.github.visible)
    {
        github::refresh_visible(this, cx);
        return github::render(this, cx);
    }
    let p = palette(cx);
    let Some(view) = this
        .git_history
        .view
        .as_ref()
        .filter(|view| owns(&view.project, this.project()))
    else {
        return div().into_any_element();
    };
    let width = (view.lanes.iter().map(|r| r.width).max().unwrap_or(1) as f32 * 16. + 24.).max(70.);
    if view.overview.visible {
        return overview::render(this, cx);
    }
    let mut list = div()
        .id("git-history-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .track_scroll(&view.scroll)
        .flex()
        .flex_col();
    for (i, (commit, lane)) in view.rows.iter().zip(&view.lanes).enumerate() {
        let selected = view.selected == Some(i);
        let date = chrono::DateTime::from_timestamp(commit.timestamp, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        list = list.child(
            div()
                .id(("history-commit", i))
                .h(px(32.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .cursor_pointer()
                .when(selected, |s| s.bg(p.blue_soft))
                .hover(move |s| s.bg(p.hover))
                .aria_label(format!("Commit {}: {}", commit.oid, commit.subject))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(view) = &this.git_history.view {
                        view.focus.focus(window, cx);
                    }
                    this.select_history_commit(i, 0, cx);
                }))
                .child(graph_cell(lane.clone(), width, p.blue))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(commit.oid.chars().take(8).collect::<String>()),
                )
                .child(
                    div()
                        .min_w(px(160.))
                        .flex_1()
                        .text_size(px(12.))
                        .truncate()
                        .child(commit.subject.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.blue)
                        .child(commit.refs.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("{} · {date}", commit.author)),
                ),
        );
    }
    let selected = view.selected.and_then(|i| view.rows.get(i));
    let detail_panel = details::render(this, cx);
    div().id("git-history-panel").track_focus(&view.focus).flex_1().min_w(px(0.)).min_h(px(0.)).bg(p.bg).text_color(p.fg).flex().flex_col().overflow_hidden()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let Some(view) = this.git_history.view.as_ref() else { return; };
                if !view.focus.is_focused(window) || event.keystroke.modifiers.platform { return; }
                let index = match event.keystroke.key.as_str() {
                    "up" => view.selected.unwrap_or(0).saturating_sub(1),
                    "down" => (view.selected.unwrap_or(0) + 1).min(view.rows.len().saturating_sub(1)),
                    "home" => 0,
                    "end" => view.rows.len().saturating_sub(1),
                    _ => return,
                };
                this.select_history_commit(index, 0, cx);
                cx.stop_propagation();
            }))
              .child(div().px(px(12.)).py(px(6.)).flex_shrink_0().flex().flex_wrap().items_center().gap(px(6.))
                  .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("History"))
                  .child(div().text_size(px(11.)).text_color(p.muted_fg).child(view.project.name.clone()))
                  .child(div().flex_1())
                    .child(dunlin_ui::button("history-inventory").ghost().label("Repository state").tooltip("Stashes, worktrees and operations").on_click(cx.listener(|this, _, _, cx| this.open_git_overview(cx))))
                    .child(dunlin_ui::button("history-github").ghost().label("GitHub · CI / PR").on_click(cx.listener(|this,_,_,cx|this.open_github(cx))))
                   .child(dunlin_ui::button("history-selected-operation").ghost().label("Merge / rebase / pick…").tooltip("Review an operation using the selected commit").disabled(selected.is_none()||view.loading||this.git_history.running).on_click(cx.listener(|this,_,window,cx|this.operate_on_history_commit(window,cx))))
                  .when(!view.project.is_remote(), |toolbar| toolbar.child(dunlin_ui::button("history-checkout").ghost().label("Checkout commit…").tooltip("Detach HEAD at the selected commit; requires a clean worktree").disabled(selected.is_none() || view.loading || this.git_history.running).on_click(cx.listener(|this, _, window, cx| this.checkout_history_commit(window, cx)))))
                  .child(dunlin_ui::button("history-branch-form").ghost().label("Create branch…").disabled(selected.is_none() || view.loading || this.git_history.running).on_click(cx.listener(|this, _, _, cx| {
                      if let Some(view) = &mut this.git_history.view { view.branch_form_open = !view.branch_form_open; }
                      cx.notify();
                  })))
                  .child(dunlin_ui::button("history-refresh").ghost().label("Refresh").disabled(view.loading || this.git_history.running).on_click(cx.listener(|this, _, _, cx| this.load_git_history(cx))))
                  .child(dunlin_ui::button("history-more").ghost().label("Load more").tooltip("Load up to 2,000 commits").disabled(view.loading || this.git_history.running || view.limit >= 2000).on_click(cx.listener(|this, _, _, cx| { if let Some(v) = &mut this.git_history.view { v.limit = (v.limit + 500).min(2000); } this.load_git_history(cx); }))))
              .child(div().px(px(12.)).pb(px(6.)).flex_shrink_0().text_size(px(11.)).text_color(p.muted_fg).child(if view.loading { "Loading history…".to_string() } else { format!("{} commits · {} snapshot · cached refs · no fetch", view.rows.len(), if view.project.is_remote() { "relay" } else { "local" }) }))
            .child(div().px(px(12.)).pb(px(8.)).flex().gap(px(8.))
                .child(Input::new(&view.revision).flex_1())
                  .child(dunlin_ui::button("history-revision").ghost().label("Show history").disabled(view.loading || this.git_history.running).on_click(cx.listener(|this, _, _, cx| this.load_git_history(cx)))))
             .child(div().flex_1().min_h(px(80.)).flex().flex_col().overflow_hidden().child(list))
             .child(detail_panel)
              .when(view.branch_form_open, |panel| panel.child(
                  div().px(px(12.)).py(px(8.)).flex_shrink_0().flex().flex_col().gap(px(6.)).border_t_1().border_color(p.border)
                      .child(div().flex().items_center().gap(px(8.))
                          .child(Input::new(&view.name).flex_1())
                          .child(dunlin_ui::button("history-create").primary().label(if this.git_history.running { "Creating…" } else { "Create branch" }).disabled(selected.is_none() || this.git_history.running || view.loading).on_click(cx.listener(|this, _, _, cx| this.create_history_branch(cx)))))
                      .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Creates a branch on the local or relay repository from the selected commit only — no checkout or changes to files/staging."))))
              .when_some(view.error.as_ref(), |panel, error| panel.child(div().px(px(12.)).py(px(6.)).flex_shrink_0().text_size(px(12.)).text_color(p.red).child(error.clone())))
          .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn structural_mutation_lock_protects_aliases_and_survives_view_close() {
        let mut state = super::State {
            running: true,
            mutation_global: true,
            mutation_project: Some("repo".into()),
            ..Default::default()
        };
        state.view = None;
        state.prune(&[], &[]);
        assert!(state.locks("alias"));
        assert!(state.locks("repo"));
        state.running = false;
        state.mutation_global = false;
        assert!(!state.locks("alias"));
    }
    #[test]
    fn concurrent_graph_and_inventory_reads_cannot_replace_each_others_pinned_scope() {
        let mut scope = None;
        assert!(super::pin_scope(&mut scope, "first".into()).is_ok());
        assert!(super::pin_scope(&mut scope, "second".into()).is_err());
        assert_eq!(scope.as_deref(), Some("first"));
        assert!(super::pin_scope(&mut scope, "first".into()).is_ok());
    }

    #[test]
    fn remote_snapshots_are_bound_to_host_device_project_and_server_path() {
        let mut original = project();
        original.remote = Some(dunlin_domain::RemoteProject {
            host_id: "host".into(),
            project_id: "server-project".into(),
            path: "/repo".into(),
        });
        assert!(super::owns(&original, &original));
        for field in ["host", "project", "path"] {
            let mut changed = original.clone();
            let remote = changed.remote.as_mut().unwrap();
            match field {
                "host" => remote.host_id = "another".into(),
                "project" => remote.project_id = "another".into(),
                _ => remote.path = "/another".into(),
            }
            assert!(!super::owns(&original, &changed));
        }
        let host = crate::remote::RemoteHost {
            id: "host".into(),
            name: "Host".into(),
            url: "http://relay.invalid".into(),
            user_name: "member".into(),
            user_role: "member".into(),
            device_id: "device".into(),
        };
        let backend =
            crate::remote::WorkspaceBackend::Remote(Box::new(crate::remote::RemoteWorkspace {
                host: host.clone(),
                project: original.remote.unwrap(),
                token: "not-a-real-token".into(),
            }));
        assert!(super::backend_current(
            &backend,
            std::slice::from_ref(&host)
        ));
        assert!(!super::backend_current(&backend, &[]));
        let mut changed = host.clone();
        changed.url = "http://another.invalid".into();
        assert!(!super::backend_current(&backend, &[changed]));
        let mut changed = host;
        changed.device_id = "new-pairing".into();
        assert!(!super::backend_current(&backend, &[changed]));
    }

    fn project() -> dunlin_domain::Project {
        dunlin_domain::Project {
            id: "repo".into(),
            name: "Repo".into(),
            path: "/repo".into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    #[test]
    fn history_cannot_be_rendered_for_another_root_remote_or_revoked_project() {
        let original = project();
        assert!(super::owns(&original, &original));
        let mut changed = original.clone();
        changed.id = "another".into();
        assert!(!super::owns(&original, &changed));
        changed = original.clone();
        changed.path = "/other".into();
        assert!(!super::owns(&original, &changed));
        changed = original.clone();
        changed.trusted = false;
        assert!(!super::owns(&original, &changed));
        changed = original.clone();
        changed.remote = Some(Default::default());
        assert!(!super::owns(&original, &changed));
    }

    #[test]
    fn closing_or_pruning_a_read_tab_does_not_release_mutation_ownership() {
        let mut state = super::State {
            running: true,
            mutation_project: Some("repo".into()),
            ..Default::default()
        };
        assert!(state.view.is_none());
        assert!(!state.is_active_for(&project()));
        assert!(state.locks("repo"));
        assert!(!state.locks("another"));
        state.prune(&[], &[]);
        assert!(state.locks("repo"));
        state.running = false;
        state.mutation_project = None;
        assert!(!state.locks("repo"));
    }

    #[test]
    fn layout_connects_both_merge_parents_and_collapses_shared_ancestors() {
        let commit = |id: &str, parents: &[&str]| dunlin_workspace::git::history::Commit {
            oid: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: String::new(),
            timestamp: 0,
            subject: String::new(),
            refs: String::new(),
        };
        let rows = super::layout(&[
            commit("merge", &["a", "b"]),
            commit("a", &["root"]),
            commit("b", &["root"]),
            commit("root", &[]),
        ]);
        assert_eq!(rows[0].edges, vec![(0, 0), (0, 1)]);
        assert_eq!(rows[2].edges.len(), 2);
        assert!(rows[3].edges.is_empty());
    }
}
