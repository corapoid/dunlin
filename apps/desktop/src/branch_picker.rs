//! Explicit, scope-owned branch checkout. No stash/reset/force or automatic AI.
use crate::{
    app::{DunlinApp, EditorConflict},
    remote::WorkspaceBackend,
};
use dunlin_domain::Project;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    assets::IconName,
    component::{
        Icon, Sizable,
        input::{Input, InputEvent, InputState},
    },
    *,
};
use dunlin_workspace::git::BranchContext;

pub(crate) const STATUS_BAR_HEIGHT: f32 = 27.;
pub(crate) const STATUS_BAR_INSET: f32 = 11.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presentation {
    Modal,
    StatusBar,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    width: f32,
    height: f32,
    header: f32,
    summary: f32,
    search: f32,
    footer: f32,
    row: f32,
}
impl Presentation {
    fn radius(self) -> f32 {
        match self {
            Self::Modal => 12.,
            Self::StatusBar => 8.,
        }
    }
    // Geometry never depends on loading state, errors, filtering or row count.
    fn layout(self) -> Layout {
        match self {
            Self::Modal => Layout {
                width: 530.,
                height: 460.,
                header: 50.,
                summary: 30.,
                search: 44.,
                footer: 42.,
                row: 34.,
            },
            Self::StatusBar => Layout {
                width: 300.,
                height: 170.,
                header: 0.,
                summary: 0.,
                search: 42.,
                footer: 26.,
                row: 28.,
            },
        }
    }
}
impl Layout {
    fn list_height(self) -> f32 {
        self.height - 2. - self.header - self.summary - self.search - self.footer
    }
    fn selected_offset(self, index: usize) -> f32 {
        (6. + index as f32 * (self.row + 2.) + self.row + 6. - self.list_height()).max(0.)
    }
}

#[derive(Default)]
pub(crate) struct State {
    picker: Option<Picker>,
    job: Option<Job>,
}
struct Job {
    id: uuid::Uuid,
    project: String,
}
struct Picker {
    id: uuid::Uuid,
    project: Project,
    backend: WorkspaceBackend,
    navigation: u64,
    presentation: Presentation,
    filter: Entity<InputState>,
    context: Option<BranchContext>,
    busy: bool,
    error: Option<String>,
    selected: usize,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
#[derive(Clone)]
struct Request {
    id: uuid::Uuid,
    project: Project,
    backend: WorkspaceBackend,
    navigation: u64,
    branch: String,
}
impl State {
    pub(crate) fn input_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.picker
            .as_ref()
            .map(|p| p.filter.read(cx).focus_handle(cx))
    }
    pub fn running(&self) -> bool {
        self.job.is_some()
    }
    pub fn dismiss(&mut self) {
        self.picker = None;
    }
    pub fn is_open(&self) -> bool {
        self.picker.is_some()
    }
    pub fn locks(&self, project: &str) -> bool {
        self.job.as_ref().is_some_and(|job| job.project == project)
    }
}
fn same_project(a: &Project, b: &Project) -> bool {
    a.id == b.id
        && a.path == b.path
        && b.trusted
        && a.remote
            .as_ref()
            .map(|r| (&r.host_id, &r.project_id, &r.path))
            == b.remote
                .as_ref()
                .map(|r| (&r.host_id, &r.project_id, &r.path))
}
fn filtered(context: &BranchContext, query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    let mut branches: Vec<_> = context
        .branches
        .iter()
        .filter(|branch| branch.to_lowercase().contains(&query))
        .cloned()
        .collect();
    branches.sort_by_key(|branch| (branch != &context.current, branch.to_lowercase()));
    branches
}
impl DunlinApp {
    pub(crate) fn open_branch_picker_filtered(
        &mut self,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker(&self.project().id.clone(), window, cx);
        if let Some(picker) = &self.branch_ui.picker {
            picker.filter.update(cx, |state, cx| {
                state.set_value(branch.to_string(), window, cx)
            });
        }
    }
    pub(crate) fn open_branch_picker(
        &mut self,
        project: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker_with(project, Presentation::Modal, window, cx);
    }
    fn open_branch_picker_with(
        &mut self,
        project: &str,
        presentation: Presentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.branch_ui.job.is_some() {
            self.file_notice("Wait for the current branch switch to finish", cx);
            return;
        }
        let Some(project) = self.projects.iter().find(|p| p.id == project).cloned() else {
            return;
        };
        if !project.trusted {
            self.file_notice("Trust the repository before switching its branch", cx);
            return;
        }
        let backend = if let Some(remote) = &project.remote {
            let Some(host) = self.remote_host(&remote.host_id) else {
                self.file_notice("Remote server is unavailable", cx);
                return;
            };
            let token = match self.remote_token(&host.id) {
                Ok(token) => token,
                Err(e) => {
                    self.file_notice(e, cx);
                    return;
                }
            };
            WorkspaceBackend::Remote(crate::remote::RemoteWorkspace {
                host,
                project: remote.clone(),
                token,
            })
        } else {
            WorkspaceBackend::Local {
                root: project.path.clone(),
            }
        };
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search local branches…"));
        let subscription = cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(picker) = this.branch_ui.picker.as_mut() {
                    picker.selected = 0;
                    picker.scroll.set_offset(point(px(0.), px(0.)));
                }
                cx.notify();
            }
        });
        let id = uuid::Uuid::now_v7();
        let navigation = self.navigation_epoch();
        self.branch_ui.picker = Some(Picker {
            id,
            project: project.clone(),
            backend: backend.clone(),
            navigation,
            presentation,
            filter: filter.clone(),
            context: None,
            busy: true,
            error: None,
            selected: 0,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        });
        self.command_open = false;
        self.settings_open = false;
        self.project_menu_open = false;
        self.session_menu_open = false;
        filter.update(cx, |state, cx| state.focus(window, cx));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.branch_context() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.navigation_epoch() != navigation
                    || !this.projects.iter().any(|p| same_project(&project, p))
                {
                    if this.branch_ui.picker.as_ref().is_some_and(|p| p.id == id) {
                        this.branch_error(
                            "The project changed; reopen the branch picker".into(),
                            cx,
                        );
                    }
                    return;
                }
                if let Some(picker) = this.branch_ui.picker.as_mut().filter(|p| p.id == id) {
                    picker.busy = false;
                    match result {
                        Ok(context) => picker.context = Some(context),
                        Err(e) => picker.error = Some(e),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn close_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_ui.picker = None; // Running Git jobs retain their own lock.
        self.focus_handle.focus(window, cx);
        cx.notify();
    }
    pub(crate) fn branch_picker_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if key == "escape" {
            self.close_branch_picker(window, cx);
            cx.stop_propagation();
            return;
        }
        let Some(picker) = self.branch_ui.picker.as_mut() else {
            return;
        };
        if picker.busy {
            return;
        }
        let rows = picker
            .context
            .as_ref()
            .map(|context| filtered(context, &picker.filter.read(cx).value()))
            .unwrap_or_default();
        match key {
            "up" | "down" if !rows.is_empty() => {
                picker.selected = (picker.selected as isize + if key == "up" { -1 } else { 1 })
                    .rem_euclid(rows.len() as isize) as usize;
                picker.scroll.set_offset(point(
                    px(0.),
                    -px(picker
                        .presentation
                        .layout()
                        .selected_offset(picker.selected)),
                ));
                cx.stop_propagation();
                cx.notify();
            }
            "enter" if !rows.is_empty() => {
                let branch = rows[picker.selected.min(rows.len() - 1)].clone();
                self.choose_branch(&branch, window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
    fn branch_request_valid(&self, request: &Request) -> bool {
        self.navigation_epoch() == request.navigation
            && match &request.backend {
                WorkspaceBackend::Local { .. } => true,
                WorkspaceBackend::Remote(remote) => self
                    .remote_host(&remote.host.id)
                    .is_some_and(|host| host.url == remote.host.url),
            }
            && self
                .projects
                .iter()
                .any(|p| same_project(&request.project, p))
            && self
                .branch_ui
                .picker
                .as_ref()
                .is_some_and(|p| p.id == request.id)
    }
    fn branch_checkout_guard(
        &self,
        project: &Project,
        allow_dirty_tabs: bool,
    ) -> Result<(), String> {
        if !self.projects.iter().any(|p| same_project(project, p)) {
            return Err("The project changed; reopen the branch picker".into());
        }
        if self.branch_ui.job.is_some()
            || self.ai_commit_locks_project_id(&project.id)
            || self.git_mutation_in_flight(&project.id)
            || self.explorer.busy
        {
            return Err("Finish the current repository operation before switching branches".into());
        }
        if self.live_agents().iter().any(|(owner, session, _)| {
            owner == &project.id
                || self
                    .group_contexts
                    .get(session)
                    .is_some_and(|group| group.roots.iter().any(|root| root.id == project.id))
        }) {
            return Err(
                "Finish or cancel agents working in this repository before switching branches"
                    .into(),
            );
        }
        if self.workbench.tabs.iter().any(|tab| {
            tab.project == project.id && (tab.loading || (!allow_dirty_tabs && tab.dirty))
        }) || (!allow_dirty_tabs && self.project().id == project.id && self.editor_dirty)
        {
            return Err(
                "Save affected buffers and wait for file loading before switching branches".into(),
            );
        }
        Ok(())
    }
    fn branch_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.branch_ui.picker {
            picker.busy = false;
            picker.error = Some(error);
        }
        cx.notify();
    }
    pub(crate) fn choose_branch(
        &mut self,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = &self.branch_ui.picker else {
            return;
        };
        if picker.busy {
            return;
        }
        let Some(context) = &picker.context else {
            return;
        };
        if !context.branches.iter().any(|b| b == branch) {
            return;
        }
        if context.current == branch {
            self.close_branch_picker(window, cx);
            return;
        }
        let request = Request {
            id: picker.id,
            project: picker.project.clone(),
            backend: picker.backend.clone(),
            navigation: picker.navigation,
            branch: branch.into(),
        };
        self.stash_editor_tab();
        if let Err(e) = self.branch_checkout_guard(&request.project, true) {
            self.branch_error(e, cx);
            return;
        }
        let dirty = self
            .workbench
            .tabs
            .iter()
            .any(|tab| tab.project == request.project.id && tab.dirty);
        if let Some(picker) = &mut self.branch_ui.picker {
            picker.busy = true;
            picker.error = None;
        }
        let answer = dirty.then(|| {
            let answer = window.prompt(PromptLevel::Warning, "Save buffers before switching branch?", Some("Save all unsaved files in this project, then review the Git changes. Nothing will be discarded or stashed automatically."), &["Cancel", "Save all and continue"], cx);
            self.overlay_backdrop.track_native(answer, cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Some(answer) = answer {
                let approved = answer.await == Ok(1);
                if !approved {
                    let _ = this.update(cx, |this, cx| { if this.branch_request_valid(&request) { this.branch_error("Branch switch canceled; buffers are unchanged".into(), cx); } });
                    return;
                }
                let saved = this.update(cx, |this, cx| {
                    if !this.branch_request_valid(&request) { return false; }
                    if let Err(e) = this.branch_checkout_guard(&request.project, true) { this.branch_error(e, cx); return false; }
                    this.save_tabs_for_project(Some(&request.project.id), cx)
                }).unwrap_or(false);
                if !saved {
                    let _ = this.update(cx, |this, cx| { if this.branch_request_valid(&request) { this.branch_error("Branch switch stopped. Resolve unsaved files or save conflicts first.".into(), cx); } });
                    return;
                }
            }
            if !this.read_with(cx, |this, _| this.branch_request_valid(&request)).unwrap_or(false) { return; }
            let backend = request.backend.clone();
            let context = cx.background_executor().spawn(async move { backend.branch_context() }).await;
            let context = match context {
                Ok(context) => context,
                Err(error) => { let _ = this.update(cx, |this, cx| { if this.branch_request_valid(&request) { this.branch_error(error, cx); } }); return; }
            };
            let confirmation = this.update_in(cx, |this, window, cx| {
                if !this.branch_request_valid(&request) { return None; }
                if let Err(e) = this.branch_checkout_guard(&request.project, false) { this.branch_error(e, cx); return None; }
                if let Some(picker) = &mut this.branch_ui.picker { picker.context = Some(context.clone()); }
                let description = format!("{} → {}. Changes: {} staged, {} unstaged, {} untracked. Git will carry compatible changes to the destination branch and refuse conflicts or overwrites, including ignored files. No stash, reset, merge or commit will be created. To keep this work separate, cancel and commit or stash it yourself first.", context.current, request.branch, context.staged, context.unstaged, context.untracked);
                Some(context.dirty().then(|| {
                    let answer = window.prompt(PromptLevel::Warning, "Keep uncommitted changes while switching?", Some(&description), &["Cancel", "Keep changes and switch"], cx);
                    this.overlay_backdrop.track_native(answer, cx)
                }))
            }).ok().flatten();
            let Some(confirmation) = confirmation else { return; };
            if let Some(answer) = confirmation && answer.await != Ok(1) {
                let _ = this.update(cx, |this, cx| { if this.branch_request_valid(&request) { this.branch_error("Branch switch canceled; Git changes are unchanged".into(), cx); } });
                return;
            }
            let started = this.update(cx, |this, cx| {
                if !this.branch_request_valid(&request) { return false; }
                this.stash_editor_tab();
                if let Err(e) = this.branch_checkout_guard(&request.project, false) { this.branch_error(e, cx); return false; }
                this.begin_git_mutation(&request.project.id);
                this.repo_summaries.invalidate();
                this.branch_ui.job = Some(Job { id: request.id, project: request.project.id.clone() });
                cx.notify(); true
            }).unwrap_or(false);
            if !started { return; }
            let backend = request.backend.clone(); let branch = request.branch.clone();
            let keep_changes = context.dirty();
            let result = cx.background_executor().spawn(async move {
                backend.switch_branch_checked(&branch, &context, keep_changes)?;
                Ok::<_, String>(backend.git_status().ok())
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_git_mutation(&request.project.id);
                this.repo_summaries.invalidate();
                if this.branch_ui.job.as_ref().is_some_and(|job| job.id == request.id) { this.branch_ui.job = None; }
                if !this.projects.iter().any(|p| same_project(&request.project, p)) { cx.notify(); return; }
                match result {
                    Ok(status) => {
                        if this.branch_ui.picker.as_ref().is_some_and(|p| p.id == request.id) { this.branch_ui.picker = None; this.focus_handle.focus(window, cx); }
                        let branch = status.as_ref().filter(|s| s.error.is_none() && !s.branch.is_empty()).map(|s| s.branch.clone()).unwrap_or(request.branch);
                        if let Some(project) = this.projects.iter_mut().find(|p| p.id == request.project.id) {
                            project.branch = Some(branch.clone());
                            let project = project.clone();
                            if let Some(store) = &this.store && let Err(error) = store.upsert_project(&project) { this.report_storage_error("save branch selection", error); }
                        }
                        this.repo_summaries.values.insert(request.project.id.clone(), crate::project_groups::RepoSummary { path: request.project.path.clone(), branch: Some(branch.clone()), changes: status.as_ref().map(|s| s.changes.len()), activity: Default::default(), monorepo: None });
                        if this.project().id == request.project.id && this.project().path == request.project.path {
                            this.diff_view = None;
                            if this.editor_dirty {
                                this.editor_conflict = Some(EditorConflict { path: this.inspector.as_ref().map(|i| i.path.clone()).unwrap_or_default(), message: "The branch changed while this buffer was edited. Its text is preserved; reload or save explicitly.".into() });
                            } else { this.close_inspector(); }
                            this.refresh_file_tree(cx); this.refresh_git_status(cx); this.invalidate_project_icon();
                        }
                        this.workbench.tabs.retain(|tab| tab.project != request.project.id || tab.dirty);
                        this.file_notice(format!("Switched {} to {branch} · local changes preserved", request.project.name), cx);
                    }
                    Err(error) => {
                        if this.branch_ui.picker.as_ref().is_some_and(|p| p.id == request.id) { this.branch_error(error, cx); }
                        else { this.file_notice(error, cx); }
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
}

pub(crate) fn badge(
    this: &DunlinApp,
    project: &str,
    label: &str,
    id: impl Into<ElementId>,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    branch_badge(this, project, label, id.into(), Presentation::Modal, cx)
}
pub(crate) fn status_badge(
    this: &DunlinApp,
    project: &str,
    label: &str,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    branch_badge(
        this,
        project,
        label,
        "status-branch-picker".into(),
        Presentation::StatusBar,
        cx,
    )
}
fn branch_badge(
    this: &DunlinApp,
    project: &str,
    label: &str,
    id: ElementId,
    presentation: Presentation,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let project = project.to_string();
    let label = if this.branch_ui.locks(&project) {
        "Switching…"
    } else {
        label
    };
    div()
        .id(id)
        .aria_label(format!("Switch Git branch · {label}"))
        .px(px(6.))
        .py(px(3.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(11.))
        .text_color(p.muted_fg)
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover).text_color(p.fg))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_branch_picker_with(&project, presentation, window, cx);
        }))
        .child(Icon::new(IconName::GitBranch).size(px(12.)))
        .child(div().max_w(px(144.)).truncate().child(label.to_string()))
        .child(Icon::new(IconName::ChevronDown).size(px(10.)))
        .into_any_element()
}
pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let Some(picker) = &this.branch_ui.picker else {
        return div().into_any_element();
    };
    let p = palette(cx);
    let compact = picker.presentation == Presentation::StatusBar;
    let layout = picker.presentation.layout();
    let mut rows = div().flex().flex_col().gap(px(2.)).p(px(6.));
    if let Some(error) = &picker.error {
        rows = rows.child(
            div()
                .p(px(9.))
                .text_size(px(11.))
                .text_color(p.red)
                .child(error.clone()),
        );
    }
    if picker.busy {
        rows = rows.child(
            div()
                .p(px(10.))
                .text_size(px(12.))
                .text_color(p.muted_fg)
                .child(if picker.context.is_some() {
                    "Preparing branch switch…"
                } else {
                    "Loading branches…"
                }),
        );
    } else if let Some(context) = &picker.context {
        let branches = filtered(context, &picker.filter.read(cx).value());
        for (index, branch) in branches.iter().enumerate() {
            let branch = branch.clone();
            let current = branch == context.current;
            rows = rows.child(
                div()
                    .id(format!("branch-choice:{index}"))
                    .px(px(9.))
                    .h(px(layout.row))
                    .flex_shrink_0()
                    .text_size(px(if compact { 12. } else { 13. }))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(index == picker.selected, |row| row.bg(p.accent))
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_branch(&branch, window, cx)
                    }))
                    .when(!compact, |row| {
                        row.child(Icon::new(IconName::GitBranch).size(px(13.)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(branches[index].clone()),
                    )
                    .when(current, |row| {
                        if compact {
                            row.child(
                                Icon::new(IconName::Check)
                                    .size(px(12.))
                                    .text_color(p.muted_fg),
                            )
                        } else {
                            row.child(div().text_size(px(10.)).text_color(p.blue).child("Current"))
                        }
                    }),
            );
        }
        if branches.is_empty() {
            rows = rows.child(div().p(px(12.)).text_color(p.muted_fg).child(
                "No matching local branches. Create or fetch branches in your Git client first.",
            ));
        }
    }
    let panel = div()
        .id("branch-picker-panel")
        .child(crate::effects::rounded_probe(
            &this.overlay_backdrop,
            picker.presentation.radius(),
        ))
        .aria_label(if compact {
            "Local branches"
        } else {
            "Switch Git branch"
        })
        .w(px(layout.width))
        .h(px(layout.height))
        .flex_shrink_0()
        .max_w_full()
        .flex()
        .flex_col()
        .rounded(px(picker.presentation.radius()))
        .bg(crate::effects::panel_fill(&this.overlay_backdrop, p.card))
        .shadow(crate::effects::panel_shadow())
        .border_1()
        .border_color(crate::effects::panel_border(&this.overlay_backdrop, &p))
        .overflow_hidden()
        .on_click(|_, _, cx| cx.stop_propagation())
        .when(!compact, |panel| {
            panel.child(
                div()
                    .h(px(layout.header))
                    .flex_shrink_0()
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Switch branch · {}", picker.project.name)),
                    )
                    .child(
                        div()
                            .id("branch-picker-close")
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .text_size(px(11.))
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_branch_picker(window, cx)
                            }))
                            .child("Close"),
                    ),
            )
        })
        .when(!compact, |panel| {
            panel.child(
                div()
                    .h(px(layout.summary))
                    .flex_shrink_0()
                    .px(px(14.))
                    .pb(px(10.))
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .truncate()
                    .child(
                        picker
                            .context
                            .as_ref()
                            .map(|c| {
                                format!(
                                    "Current: {} · {} staged · {} unstaged · {} untracked",
                                    c.current, c.staged, c.unstaged, c.untracked
                                )
                            })
                            .unwrap_or_else(|| "Reading repository…".into()),
                    ),
            )
        })
        .child(
            div()
                .h(px(layout.search))
                .flex_shrink_0()
                .px(px(12.))
                .flex()
                .items_center()
                .child(Input::new(&picker.filter).small()),
        )
        .child(
            div()
                .id("branch-picker-list")
                .track_scroll(&picker.scroll)
                .h(px(layout.list_height()))
                .flex_shrink_0()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(rows),
        )
        .child(
            div()
                .h(px(layout.footer))
                .flex_shrink_0()
                .px(px(12.))
                .flex()
                .items_center()
                .border_t_1()
                .border_color(p.border)
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child(div().flex_1().min_w(px(0.)).truncate().child(if compact {
                    "Local branches · Esc closes"
                } else {
                    "Local branches only · no automatic stash or discarded changes · Esc closes"
                })),
        );
    // Render outside the status bar's clipped layout, aligned to the branch
    // trigger at its left edge. Both presentations use the shared backdrop;
    // the dismiss layer itself must not add a second dimming pass.
    let panel = if compact {
        panel
            .absolute()
            .left(px(STATUS_BAR_INSET))
            .bottom(px(STATUS_BAR_HEIGHT + 4.))
            .shadow(vec![
                BoxShadow::new(px(0.), px(4.), dunlin_ui::black().opacity(0.16))
                    .blur_radius(px(16.)),
            ])
    } else {
        panel
    };
    div()
        .id("branch-picker-overlay")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .when(!compact, |overlay| {
            overlay.items_center().justify_center().p(px(20.))
        })
        .on_click(cx.listener(|this, _, window, cx| this.close_branch_picker(window, cx)))
        .child(panel)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Presentation, filtered};
    #[test]
    fn fixed_panel_geometry_reserves_the_same_scroll_area_before_and_after_loading() {
        for presentation in [Presentation::Modal, Presentation::StatusBar] {
            let layout = presentation.layout();
            assert!(layout.list_height() >= 100.);
            assert_eq!(
                layout.height,
                layout.header
                    + layout.summary
                    + layout.search
                    + layout.list_height()
                    + layout.footer
                    + 2.
            );
            // Branch count/search/empty/error content is confined to this area;
            // it cannot change the panel or its anchor while data arrives.
            for count in [0, 1, 2, 50, 500] {
                let content_height = count as f32 * (layout.row + 2.) + 12.;
                assert_eq!(presentation.layout(), layout);
                if count > 50 {
                    assert!(content_height > layout.list_height());
                }
            }
        }
    }
    #[test]
    fn status_bar_uses_a_compact_popover_instead_of_the_centered_modal() {
        let modal = Presentation::Modal.layout();
        let compact = Presentation::StatusBar.layout();
        assert!(compact.width < modal.width);
        assert!(compact.height < modal.height / 2.);
        assert_eq!((compact.header, compact.summary), (0., 0.));
        assert!(compact.row < modal.row);
        // One radius value is used by both the panel and backdrop measurement;
        // never leave a square unblurred/undimmed hole behind rounded corners.
        assert_eq!(Presentation::Modal.radius(), 12.);
        assert_eq!(Presentation::StatusBar.radius(), 8.);
    }
    #[test]
    fn keyboard_selection_remains_inside_each_fixed_scroll_viewport() {
        for presentation in [Presentation::Modal, Presentation::StatusBar] {
            let layout = presentation.layout();
            assert_eq!(layout.selected_offset(0), 0.);
            for index in 0..512 {
                let top = 6. + index as f32 * (layout.row + 2.) - layout.selected_offset(index);
                assert!(top >= 6.);
                assert!(top + layout.row <= layout.list_height() - 6.);
            }
            assert!(layout.selected_offset(511) > 0.);
        }
    }
    #[test]
    fn filter_keeps_current_first_and_supports_case_insensitive_branch_names() {
        let context = dunlin_workspace::git::BranchContext {
            root: "/project".into(),
            current: "main".into(),
            head: None,
            branches: vec!["feature/Web".into(), "main".into(), "fix/other".into()],
            status: String::new(),
            staged: 0,
            unstaged: 0,
            untracked: 0,
        };
        assert_eq!(filtered(&context, ""), ["main", "feature/Web", "fix/other"]);
        assert_eq!(filtered(&context, "WEB"), ["feature/Web"]);
    }
    #[test]
    fn hiding_the_picker_does_not_release_a_running_repository_lock() {
        let mut state = super::State {
            job: Some(super::Job {
                id: uuid::Uuid::now_v7(),
                project: "one".into(),
            }),
            ..Default::default()
        };
        state.dismiss();
        assert!(state.running() && state.locks("one"));
        assert!(!state.locks("another"));
    }
}
