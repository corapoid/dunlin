//! Project-owned Git history tab, independent of chat, files and the agent graph.
use crate::app::DunlinApp;
use crate::remote::{RemoteHost, WorkspaceBackend};
use dunlin_domain::Project;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Disableable;
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        diff::{Diff, DiffFile, DiffState},
        input::InputState,
    },
    prelude::FluentBuilder as _,
    *,
};
use dunlin_workspace::git::history::{self, Commit};
use std::path::Path;

mod actions;
mod budgets;
mod checks;
mod compare;
mod context;
mod details;
mod graph;
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
    pub(crate) mutation_project: Option<String>,
    pub(crate) mutation_global: bool,
    pub(crate) cancel_check: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl State {
    pub(crate) fn tab_label(&self) -> &'static str {
        if let Some(v) = &self.view {
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
    loaded_revision: String,
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
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    mutations: mutations::Form,
    context: context::State,
    review: review::State,
    checks: checks::State,
    budgets: budgets::State,
    subtasks: subtasks::State,
    compare: compare::State,
    impact: impact::State,
    hovered: Option<usize>,
    inspector: details::State,
}
impl View {
    fn hide_tools(&mut self) {
        self.impact.visible = false;
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
#[derive(Clone, Debug, PartialEq, Eq)]
struct LaneRow {
    lane: usize,
    edges: Vec<(usize, usize)>,
    width: usize,
    incoming: usize,
    color: usize,
    incoming_colors: Vec<usize>,
    edge_colors: Vec<usize>,
}
fn layout(commits: &[Commit]) -> Vec<LaneRow> {
    let mut lanes: Vec<(String, usize)> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let incoming = lanes.len();
            let lane = lanes
                .iter()
                .position(|(id, _)| id == &commit.oid)
                .unwrap_or_else(|| {
                    let color = graph::distinct(
                        graph::identity(commit),
                        lanes.iter().map(|(_, color)| *color),
                    );
                    lanes.push((commit.oid.clone(), color));
                    lanes.len() - 1
                });
            let before = lanes.clone();
            let color = before[lane].1;
            lanes.remove(lane);
            for (offset, parent) in commit.parents.iter().enumerate() {
                if !lanes.iter().any(|(id, _)| id == parent) {
                    let parent_color = if offset == 0 {
                        color
                    } else {
                        graph::distinct(graph::hash(parent), lanes.iter().map(|(_, color)| *color))
                    };
                    lanes.insert(
                        (lane + offset).min(lanes.len()),
                        (parent.clone(), parent_color),
                    );
                }
            }
            let mut edges = Vec::new();
            let mut edge_colors = Vec::new();
            for (column, (id, lane_color)) in before.iter().enumerate() {
                if id == &commit.oid {
                    for parent in &commit.parents {
                        if let Some(to) = lanes.iter().position(|(id, _)| id == parent) {
                            edges.push((column, to));
                            edge_colors.push(lanes[to].1);
                        }
                    }
                } else if let Some(to) = lanes.iter().position(|(next, _)| next == id) {
                    edges.push((column, to));
                    edge_colors.push(*lane_color);
                }
            }
            LaneRow {
                lane,
                width: before.len().max(lanes.len()),
                edges,
                incoming,
                color,
                incoming_colors: before
                    .iter()
                    .take(incoming)
                    .map(|(_, color)| *color)
                    .collect(),
                edge_colors,
            }
        })
        .collect()
}

/// Anchor the first visible commit, including its fractional row offset, when
/// new commits are inserted above it. Falling out of a filter resets separately.
fn scroll_anchor(before: &[Commit], after: &[Commit], offset: f32) -> Option<f32> {
    let distance = (-offset).max(0.);
    let index = (distance / graph::ROW_HEIGHT).floor() as usize;
    let oid = &before.get(index)?.oid;
    let next = after.iter().position(|commit| &commit.oid == oid)?;
    Some(-(next as f32 * graph::ROW_HEIGHT + distance % graph::ROW_HEIGHT))
}

impl DunlinApp {
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
        self.remember_workspace_tab();
        self.workspace_tools.current = None;
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
            self.remember_workspace_tab();
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
            loaded_revision: String::new(),
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
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            mutations: mutations::Form::new(window, cx),
            context: context::State::new(window, cx),
            review: review::State::new(window, cx),
            checks: checks::State::new(window, cx),
            budgets: budgets::State::new(window, cx),
            subtasks: subtasks::State::new(window, cx),
            compare: compare::State::new(window, cx),
            impact: impact::State::new(window, cx),
            hovered: None,
            inspector: details::State::new(cx),
        });
        self.git_history.active = true;
        self.remember_workspace_tab();
        self.workbench
            .panes
            .opened(crate::workspace_panes::TabKey::Git(
                self.project().id.clone(),
            ));
        self.git_history
            .view
            .as_ref()
            .unwrap()
            .focus
            .focus(window, cx);
        self.load_git_history(cx);
        self.load_git_overview(cx);
    }
    pub(crate) fn show_git_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(window, cx);
        if let Some(view) = &mut self.git_history.view {
            view.hide_tools();
            view.overview.visible = false;
        }
        cx.notify();
    }
    pub(crate) fn close_git_history(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = self
            .git_history
            .view
            .as_ref()
            .map(|view| view.project.id.clone())
        {
            if !self.guard_workspace_tab_close(&crate::workspace_panes::TabKey::Git(project), cx) {
                return;
            }
        }
        if let Some(view) = &self.git_history.view {
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::Git(
                    view.project.id.clone(),
                ));
        }
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
        // Keep the displayed snapshot and its immutable comparison while a new
        // graph is read. Replacing it atomically avoids empty flashes and jumps.
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
                    Ok::<_, String>((response.scope, rows, lanes, revision))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.prune(&this.projects, &this.remote_hosts);
                if !this.projects.iter().any(|p| owns(&project, p)) {
                    return;
                }
                let mut select = None;
                if let Some(view) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|v| v.id == id && v.load_request == request)
                {
                    view.loading = false;
                    match result {
                        Ok((root, rows, lanes, revision)) => {
                            if let Err(error) = pin_scope(&mut view.scope, root) {
                                view.error = Some(error);
                                cx.notify();
                                return;
                            }
                            let previous = view
                                .selected
                                .and_then(|i| view.rows.get(i))
                                .map(|c| c.oid.clone());
                            let retained = previous
                                .as_ref()
                                .and_then(|oid| rows.iter().position(|c| &c.oid == oid));
                            let old_offset = view.scroll.0.borrow().base_handle.offset();
                            let anchored_y =
                                scroll_anchor(&view.rows, &rows, f32::from(old_offset.y));
                            view.loaded_revision = revision;
                            view.lanes = lanes;
                            view.rows = rows;
                            view.hovered = None;
                            if let Some(index) = retained {
                                view.selected = Some(index);
                                if let Some(y) = anchored_y {
                                    view.scroll
                                        .0
                                        .borrow()
                                        .base_handle
                                        .set_offset(point(old_offset.x, px(y)));
                                }
                            } else {
                                view.selected = None;
                                view.detail = None;
                                view.detail_loading = false;
                                view.patch_loading = false;
                                view.detail_request = uuid::Uuid::now_v7();
                                view.patch_request = uuid::Uuid::now_v7();
                                view.selected_file = None;
                                view.patch_note = None;
                                view.detail_error = None;
                                view.inspector.clear();
                                view.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
                                select = (!view.rows.is_empty()).then_some(0);
                            }
                            view.error = None;
                        }
                        Err(error) => {
                            view.error = Some(format!(
                                "{error}. {}No fetch was performed.",
                                if view.rows.is_empty() {
                                    "Check the revision or whether the repository has commits. "
                                } else {
                                    "Showing the previous cached snapshot. "
                                }
                            ))
                        }
                    }
                }
                if let Some(index) = select {
                    this.select_history_commit(index, 0, cx);
                }
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
        if view.selected == Some(index)
            && view.parent == parent
            && (view.detail.is_some() || view.detail_loading)
        {
            view.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
            return;
        }
        let oid = commit.oid.clone();
        let project = view.project.clone();
        let backend = view.backend.clone();
        let id = view.id;
        view.selected = Some(index);
        view.inspector.clear();
        view.branch_form_open = false;
        view.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
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
                let mut open_patch = false;
                if let Some(view) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|view| view.id == id && view.detail_request == request)
                {
                    view.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            open_patch = view.inspector.tab == details::Tab::Patch
                                && !detail.files.is_empty();
                            view.inspector.install(&detail.files);
                            view.detail = Some(detail);
                        }
                        Err(error) => view.detail_error = Some(error),
                    }
                }
                if open_patch {
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
    let Some(view) = this
        .git_history
        .view
        .as_ref()
        .filter(|view| owns(&view.project, this.project()))
    else {
        return div().into_any_element();
    };
    if view.overview.visible {
        return overview::render(this, cx);
    }
    browser(this, cx)
}

fn browser(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let view = this.git_history.view.as_ref().unwrap();
    let width = graph::width(view);
    let list = graph::render(this, cx);
    let detail_panel = details::render(this, cx);
    let status = actions::status(this, cx);
    div()
        .id("git-history-panel")
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("git-history-panel")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .track_focus(&view.focus)
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .text_color(p.fg)
        .flex()
        .flex_col()
        .overflow_hidden()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            let Some(view) = this.git_history.view.as_ref() else {
                return;
            };
            if !view.focus.is_focused(window) || event.keystroke.modifiers.platform {
                return;
            }
            let index = match event.keystroke.key.as_str() {
                "up" => view.selected.unwrap_or(0).saturating_sub(1),
                "down" => (view.selected.unwrap_or(0) + 1).min(view.rows.len().saturating_sub(1)),
                "home" => 0,
                "end" => view.rows.len().saturating_sub(1),
                "enter" => {
                    if let Some(view) = &mut this.git_history.view {
                        if let Some(commit) = view.selected.and_then(|i| view.rows.get(i)) {
                            view.mutations.initialize_target(&commit.oid, window, cx);
                        }
                        view.inspector.tab = details::Tab::Operations;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                _ => return,
            };
            this.select_history_commit(index, 0, cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .px(px(10.))
                .py(px(6.))
                .flex_shrink_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(
                    dunlin_ui::input(&view.revision)
                        .aria_label("Commit or revision")
                        .min_w(px(120.))
                        .flex_1(),
                )
                .child(
                    dunlin_ui::toolbar_button("history-revision")
                        .label("Show history")
                        .disabled(view.loading || this.git_history.running)
                        .on_click(cx.listener(|this, _, _, cx| this.load_git_history(cx))),
                )
                .child(
                    dunlin_ui::toolbar_button("history-all")
                        .label("All refs")
                        .disabled(view.loading || this.git_history.running)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(v) = &this.git_history.view {
                                v.revision.update(cx, |s, cx| s.set_value("", window, cx));
                            }
                            this.load_git_history(cx);
                        })),
                )
                .child(
                    dunlin_ui::toolbar_button("history-state")
                        .label("Repository state")
                        .on_click(cx.listener(|this, _, _, cx| this.open_git_overview(cx))),
                )
                .child(div().flex_1())
                .child(
                    dunlin_ui::toolbar_button("history-refresh")
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .disabled(view.loading || this.git_history.running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.load_git_history(cx);
                            this.load_git_overview(cx);
                        })),
                ),
        )
        .child(div().px(px(10.)).flex_shrink_0().child(status))
        .child(
            div().flex_1().min_h(px(0.)).min_w(px(0.)).child(
                dunlin_ui::component::resizable::h_resizable("history-columns")
                    .child(
                        dunlin_ui::component::resizable::resizable_panel()
                            .size(px(620.))
                            .size_range(px(220.)..px(4000.))
                            .child(
                                div()
                                    .size_full()
                                    .min_w(px(0.))
                                    .min_h(px(0.))
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .h(px(34.))
                                            .px(px(12.))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(p.muted_fg)
                                                    .child("REPOSITORY HISTORY"),
                                            )
                                            .child(div().flex_1())
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(p.muted_fg)
                                                    .child(if view.loading {
                                                        "Refreshing…".into()
                                                    } else {
                                                        format!(
                                                            "{} commits · {} · {}",
                                                            view.rows.len(),
                                                            if view.project.is_remote() {
                                                                "relay"
                                                            } else {
                                                                "local"
                                                            },
                                                            if view.loaded_revision.is_empty() {
                                                                "all refs".into()
                                                            } else {
                                                                view.loaded_revision
                                                                    .chars()
                                                                    .take(12)
                                                                    .collect::<String>()
                                                            }
                                                        )
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .h(px(24.))
                                            .pr(px(10.))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .text_size(px(10.))
                                            .text_color(p.muted_fg)
                                            .border_b_1()
                                            .border_color(p.border)
                                            .child(
                                                div()
                                                    .w(px(width))
                                                    .flex_shrink_0()
                                                    .pl(px(10.))
                                                    .child("Graph"),
                                            )
                                            .child(div().flex_1().child("Commit"))
                                            .child(div().w(px(112.)).min_w(px(70.)).child("Date")),
                                    )
                                    .when(view.rows.is_empty(), |panel| {
                                        panel.child(
                                            div()
                                                .p(px(16.))
                                                .text_size(px(12.))
                                                .text_color(p.muted_fg)
                                                .child(if view.loading {
                                                    "Loading repository history…"
                                                } else {
                                                    "No commits in this snapshot."
                                                }),
                                        )
                                    })
                                    .child(list)
                                    .when(view.limit < 2000 && view.rows.len() == view.limit, |panel| {
                                        panel.child(
                                            dunlin_ui::action_button("history-more")
                                                .label("Load more commits")
                                                .disabled(view.loading || this.git_history.running)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if let Some(v) = &mut this.git_history.view {
                                                        v.limit = (v.limit + 500).min(2000);
                                                    }
                                                    this.load_git_history(cx);
                                                })),
                                        )
                                    })
                                    .child(div().h(px(26.)).px(px(10.)).flex_shrink_0().text_size(px(10.)).text_color(p.muted_fg).child("↑ ↓ Select · Enter Operations · Right-click for commit actions")),
                            ),
                    )
                    .child(
                        dunlin_ui::component::resizable::resizable_panel()
                            .size(px(480.))
                            .size_range(px(220.)..px(4000.))
                            .child(detail_panel),
                    ),
            ),
        )
        .when_some(view.error.as_ref(), |panel, error| {
            panel.child(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(p.red)
                    .child(error.clone()),
            )
        })
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
