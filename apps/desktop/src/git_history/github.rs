//! Explicit GitHub capability in the repository tab. No local fallback for relay.
use super::*;
use dunlin_ui::component::input::{Textarea, TextareaState};
use dunlin_workspace::github::{self as api, Action, Job, Overview, Plan, PullDetails, Run};
use std::time::{Duration, Instant};

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum PendingGithub {
    Local(Plan),
    Remote(crate::remote::RemoteGithubPlan),
}

impl PendingGithub {
    fn summary(&self) -> &str {
        match self {
            Self::Local(plan) => &plan.summary,
            Self::Remote(plan) => &plan.summary,
        }
    }
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum GithubBackend {
    Local {
        root: String,
        remote: String,
    },
    Remote {
        workspace: crate::remote::RemoteWorkspace,
        remote: String,
    },
}

impl GithubBackend {
    fn load(&self) -> Result<Overview, String> {
        match self {
            Self::Local { root, remote } => api::load(root, remote),
            Self::Remote { workspace, remote } => workspace.github_load(remote),
        }
    }

    fn runs(&self, repo: &api::Repository, head: &str) -> Result<Vec<Run>, String> {
        match self {
            Self::Local { .. } => api::runs(repo, head),
            Self::Remote { workspace, remote } => workspace.github_runs(remote, head),
        }
    }

    fn jobs(&self, repo: &api::Repository, run: &Run) -> Result<(Run, Vec<Job>), String> {
        match self {
            Self::Local { .. } => api::refresh_run(repo, run),
            Self::Remote { workspace, remote } => workspace.github_jobs(remote, run),
        }
    }

    fn pull(&self, repo: &api::Repository, number: u64) -> Result<PullDetails, String> {
        match self {
            Self::Local { .. } => api::details(repo, number),
            Self::Remote { workspace, remote } => workspace.github_pull(remote, number),
        }
    }

    fn log(&self, repo: &api::Repository, run: &Run, job: &Job) -> Result<String, String> {
        match self {
            Self::Local { .. } => api::logs(repo, run, job),
            Self::Remote { workspace, remote } => workspace.github_log(remote, run, job),
        }
    }

    fn prepare(&self, repo: &api::Repository, action: Action) -> Result<PendingGithub, String> {
        match self {
            Self::Local { .. } => api::prepare(repo, action).map(PendingGithub::Local),
            Self::Remote { workspace, remote } => workspace
                .github_prepare(remote, action)
                .map(PendingGithub::Remote),
        }
    }

    fn execute(&self, plan: PendingGithub) -> Result<String, String> {
        match (self, plan) {
            (Self::Local { .. }, PendingGithub::Local(plan)) => api::execute(plan),
            (Self::Remote { workspace, .. }, PendingGithub::Remote(plan)) => {
                workspace.github_execute(&plan.id)
            }
            _ => Err("GitHub plan belongs to another backend; prepare a fresh request".into()),
        }
    }
}

pub(super) struct State {
    pub visible: bool,
    remote: Entity<InputState>,
    data: Option<Overview>,
    loading: bool,
    error: Option<String>,
    request: uuid::Uuid,
    selected_run: Option<Run>,
    jobs: Vec<Job>,
    log: Option<String>,
    detail: Option<PullDetails>,
    follow: bool,
    checked: Option<Instant>,
    pending: Option<(uuid::Uuid, PendingGithub)>,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    base: Entity<InputState>,
    head: Entity<InputState>,
    file: Entity<InputState>,
    line: Entity<InputState>,
    create_form: bool,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            remote: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value("origin")
                    .placeholder("Git remote")
            }),
            data: None,
            loading: false,
            error: None,
            request: uuid::Uuid::now_v7(),
            selected_run: None,
            jobs: Vec::new(),
            log: None,
            detail: None,
            follow: false,
            checked: None,
            pending: None,
            title: cx.new(|cx| InputState::new(window, cx).placeholder("PR title")),
            body: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Description / comment / review body")
            }),
            base: cx.new(|cx| InputState::new(window, cx).placeholder("Existing base branch")),
            head: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Existing head branch (already pushed)")
            }),
            file: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Line review: repository-relative file")
            }),
            line: cx
                .new(|cx| InputState::new(window, cx).placeholder("RIGHT-side diff line number")),
            create_form: false,
            scroll: ScrollHandle::new(),
        }
    }
}
#[derive(Clone)]
enum Read {
    Overview,
    Jobs(Run),
    Pull(u64),
    Log(Run, Job),
    Runs(String),
}
enum Response {
    Overview(Overview),
    Jobs(Run, Vec<Job>),
    Pull(Box<PullDetails>),
    Log(String),
    Runs(Vec<Run>),
}
impl DunlinApp {
    fn github_current(&self, id: uuid::Uuid) -> bool {
        self.git_history
            .view
            .as_ref()
            .is_some_and(|v| v.id == id && owns(&v.project, self.project()))
    }
    pub(super) fn open_github(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = &mut self.git_history.view {
            v.hide_tools();
            v.github.visible = true;
            v.context.visible = false;
            v.review.visible = false;
            v.checks.visible = false;
            v.overview.visible = false;
        }
        cx.notify();
    }
    fn github_read(&mut self, read: Read, cx: &mut Context<Self>) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = &mut self.git_history.view else {
            return;
        };
        if self.git_history.running || view.github.loading {
            return;
        }
        let id = view.id;
        let project = view.project.clone();
        let remote = view.github.remote.read(cx).value().to_string();
        let repo = view.github.data.as_ref().map(|d| d.repository.clone());
        let request = uuid::Uuid::now_v7();
        view.github.request = request;
        view.github.loading = true;
        view.github.error = None;
        if matches!(read, Read::Pull(_)) {
            view.github.detail = None;
            view.github.selected_run = None;
            view.github.jobs.clear();
            view.github.log = None;
            view.github.create_form = false;
            view.github.pending = None;
        }
        if matches!(read, Read::Log(..)) {
            view.github.log = None;
        }
        if let Read::Jobs(run) = &read
            && view
                .github
                .selected_run
                .as_ref()
                .is_none_or(|old| old.id != run.id || old.run_attempt != run.run_attempt)
        {
            view.github.jobs.clear();
            view.github.log = None;
        }
        let backend = if project.is_remote() {
            match self.remote_workspace_for_project(&project) {
                Ok(workspace) => GithubBackend::Remote {
                    workspace,
                    remote: remote.clone(),
                },
                Err(error) => {
                    if let Some(view) = &mut self.git_history.view
                        && view.id == id
                    {
                        view.github.loading = false;
                        view.github.error = Some(error);
                    }
                    cx.notify();
                    return;
                }
            }
        } else {
            GithubBackend::Local {
                root: project.path.clone(),
                remote: remote.clone(),
            }
        };
        cx.spawn(async move |this, cx| {
            let overview = matches!(read, Read::Overview);
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Read::Overview = read {
                        return backend.load().map(Response::Overview);
                    }
                    let repo = repo.ok_or("Load GitHub first")?;
                    match read {
                        Read::Jobs(run) => backend
                            .jobs(&repo, &run)
                            .map(|(run, jobs)| Response::Jobs(run, jobs)),
                        Read::Pull(number) => backend
                            .pull(&repo, number)
                            .map(|d| Response::Pull(Box::new(d))),
                        Read::Log(run, job) => backend.log(&repo, &run, &job).map(Response::Log),
                        Read::Runs(head) => backend.runs(&repo, &head).map(Response::Runs),
                        Read::Overview => unreachable!(),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.github_current(id) {
                    return;
                }
                let s = &mut this.git_history.view.as_mut().unwrap().github;
                if s.request != request {
                    return;
                }
                s.loading = false;
                s.checked = Some(Instant::now());
                match result {
                    Ok(Response::Overview(data)) => {
                        if s.data.as_ref().is_some_and(|old| {
                            old.repository != data.repository || old.account != data.account
                        }) {
                            s.selected_run = None;
                            s.jobs.clear();
                            s.detail = None;
                            s.log = None;
                            s.pending = None;
                        }
                        s.data = Some(data);
                    }
                    Ok(Response::Jobs(run, jobs)) => {
                        if let Some(data) = &mut s.data
                            && let Some(old) = data.runs.iter_mut().find(|old| old.id == run.id)
                        {
                            *old = run.clone();
                        }
                        s.selected_run = Some(run);
                        s.jobs = jobs;
                    }
                    Ok(Response::Pull(detail)) => s.detail = Some(*detail),
                    Ok(Response::Log(log)) => s.log = Some(log),
                    Ok(Response::Runs(runs)) => {
                        if let Some(d) = &mut s.data {
                            d.runs = runs;
                        }
                        s.selected_run = None;
                        s.jobs.clear();
                        s.log = None;
                        s.follow = false;
                    }
                    Err(e) => {
                        s.error = Some(e);
                        s.follow = false;
                        s.pending = None;
                        s.jobs.clear();
                        s.log = None;
                        s.detail = None;
                        if overview {
                            s.data = None;
                        }
                    }
                }
                let follow = s.follow;
                cx.notify();
                if follow {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_secs(15))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if !this.github_current(id) || !this.git_history.active {
                                return;
                            }
                            let s = &this.git_history.view.as_ref().unwrap().github;
                            if !s.visible || !s.follow || s.request != request {
                                return;
                            }
                            let next = s
                                .selected_run
                                .clone()
                                .map(Read::Jobs)
                                .unwrap_or(Read::Overview);
                            this.github_read(next, cx);
                        });
                    })
                    .detach();
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn github_evidence(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        if !self.github_current(view.id) {
            return;
        }
        let Some(data) = &view.github.data else {
            return;
        };
        let evidence = api::agent_context(&data.repository, "Selected CI/PR evidence", &text);
        let old = self.composer.read(cx).value();
        if old.len() + evidence.len() > 100_000 {
            self.file_notice("Composer evidence limit reached; trim the draft first", cx);
            return;
        }
        let draft = if old.is_empty() {
            evidence
        } else {
            format!("{old}\n\n{evidence}")
        };
        self.show_chat_tab(window, cx);
        self.composer
            .update(cx, |s, cx| s.set_value(draft, window, cx));
        self.file_notice(
            "Evidence appended to the draft. Review before sending; no agent was started.",
            cx,
        );
    }
    fn github_lock(&mut self, project: &Project, lock: bool) {
        self.git_history.running = lock;
        self.git_history.mutation_global = lock;
        self.git_history.mutation_project = lock.then(|| project.id.clone());
        if lock {
            self.begin_git_mutation(&project.id);
        } else {
            self.finish_git_mutation(&project.id);
        }
    }
    fn prepare_github_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        if !self.github_current(view.id) || self.git_history.running || view.github.loading {
            return;
        }
        let Some(data) = &view.github.data else {
            return;
        };
        let repo = data.repository.clone();
        let id = view.id;
        let project = view.project.clone();
        let remote = view.github.remote.read(cx).value().to_string();
        self.git_history.view.as_mut().unwrap().github.pending = None;
        let backend = if project.is_remote() {
            match self.remote_workspace_for_project(&project) {
                Ok(workspace) => GithubBackend::Remote { workspace, remote },
                Err(error) => {
                    self.file_notice(error, cx);
                    return;
                }
            }
        } else {
            GithubBackend::Local {
                root: project.path.clone(),
                remote,
            }
        };
        self.github_lock(&project, true);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.prepare(&repo, action) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.github_lock(&project, false);
                if this.github_current(id) {
                    let s = &mut this.git_history.view.as_mut().unwrap().github;
                    match result {
                        Ok(plan) => s.pending = Some((uuid::Uuid::now_v7(), plan)),
                        Err(e) => s.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn confirm_github_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        let Some((token, plan)) = &view.github.pending else {
            return;
        };
        if !self.github_current(view.id) || self.git_history.running {
            return;
        }
        let token = *token;
        let id = view.id;
        let summary = api::display_text(plan.summary(), 70_000);
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Publish this GitHub action?",
            Some(&summary),
            &["Cancel", "Submit once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if !this.github_current(id) || this.git_history.running {
                    return;
                }
                let view = this.git_history.view.as_mut().unwrap();
                if view
                    .github
                    .pending
                    .as_ref()
                    .is_none_or(|(t, _)| *t != token)
                {
                    return;
                }
                let (_, plan) = view.github.pending.take().unwrap();
                let project = view.project.clone();
                let remote = view.github.remote.read(cx).value().to_string();
                let backend = if project.is_remote() {
                    match this.remote_workspace_for_project(&project) {
                        Ok(workspace) => GithubBackend::Remote { workspace, remote },
                        Err(error) => {
                            this.file_notice(error, cx);
                            return;
                        }
                    }
                } else {
                    GithubBackend::Local {
                        root: project.path.clone(),
                        remote,
                    }
                };
                this.github_lock(&project, true);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { backend.execute(plan) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.github_lock(&project, false);
                        match result {
                            Ok(message) => this.file_notice(message, cx),
                            Err(e) => this.file_notice(e, cx),
                        }
                        if this.github_current(id) {
                            let s = &mut this.git_history.view.as_mut().unwrap().github;
                            s.follow = false;
                            s.detail = None;
                            s.jobs.clear();
                            s.log = None;
                            this.github_read(Read::Overview, cx);
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
    }
}
fn plain(text: &str) -> String {
    api::display_text(text, 80_000)
}
pub(super) fn refresh_visible(app: &mut DunlinApp, cx: &mut Context<DunlinApp>) {
    let Some(v) = &app.git_history.view else {
        return;
    };
    let s = &v.github;
    if app.git_history.active
        && s.follow
        && !s.loading
        && s.checked
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(15))
    {
        let next = s
            .selected_run
            .clone()
            .map(Read::Jobs)
            .unwrap_or(Read::Overview);
        app.github_read(next, cx);
    }
}

#[cfg(debug_assertions)]
pub(super) fn smoke_step(
    app: &mut DunlinApp,
    stage: &mut i32,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> bool {
    use serde_json::json;
    match *stage {
        0 if app.active_file_tab().is_some_and(|t| !t.loading) => {
            app.open_git_history(window, cx);
            app.open_github(cx);
            let repository = api::repository(&app.project().path, "origin").unwrap();
            let head = repository.local_head.clone();
            let run:Run=serde_json::from_value(json!({"id":17,"run_attempt":2,"name":"Build / Test","head_sha":head,"head_branch":"topic","status":"completed","conclusion":"failure","event":"pull_request"})).unwrap();
            let job:Job=serde_json::from_value(json!({"id":19,"run_id":17,"run_attempt":2,"head_sha":head,"name":"Tests","status":"completed","conclusion":"failure","steps":[{"number":1,"name":"Build","status":"completed","conclusion":"success"},{"number":2,"name":"Test","status":"completed","conclusion":"failure"}]})).unwrap();
            let pr=serde_json::from_value(json!({"node_id":"PR_fixture","number":42,"title":"Fix regression","body":"CI requires review","state":"open","draft":false,"head":{"ref":"topic","sha":head},"base":{"ref":"main","sha":"b".repeat(40)},"updated_at":"2026-11-01T12:00:00Z","mergeable":true,"merged":false})).unwrap();
            let s = &mut app.git_history.view.as_mut().unwrap().github;
            s.data = Some(Overview {
                repository,
                account: "fixture-user".into(),
                runs: vec![run.clone()],
                pulls: vec![pr],
            });
            s.selected_run = Some(run);
            s.jobs = vec![job];
            s.log =
                Some("Test failed at src/example.rs:12\nNo provider calls in this fixture".into());
            app.composer
                .update(cx, |s, cx| s.set_value("Existing draft", window, cx));
            *stage = 1;
            cx.notify();
        }
        1 => {
            assert!(app.git_history.view.as_ref().unwrap().github.visible);
            app.github_evidence("Workflow 17 attempt 2 · Test failure".into(), window, cx);
            let draft = app.composer.read(cx).value();
            assert!(draft.starts_with("Existing draft\n\n"));
            assert!(draft.contains("UNTRUSTED"));
            assert!(draft.contains("Test failure"));
            assert!(!app.harness.running);
            app.open_git_history(window, cx);
            app.open_github(cx);
            let s = &mut app.git_history.view.as_mut().unwrap().github;
            let pull = s.data.as_ref().unwrap().pulls[0].clone();
            s.detail = Some(PullDetails {
                pull,
                files: vec![
                    json!({"filename":"src/example.rs","additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+fixed"}),
                ],
                comments: vec![json!({"user":{"login":"reviewer"},"body":"Please test this"})],
                reviews: vec![],
                review_comments: vec![],
                checks: vec![json!({"name":"Tests","status":"completed","conclusion":"failure"})],
                statuses: vec![],
                bounded_lists: false,
            });
            *stage = 2;
            cx.notify();
        }
        2 => {
            let s = &mut app.git_history.view.as_mut().unwrap().github;
            s.create_form = true;
            s.title
                .update(cx, |s, cx| s.set_value("Draft PR", window, cx));
            s.body.update(cx, |s, cx| s.set_value("Body", window, cx));
            *stage = 3;
            cx.notify();
        }
        3 => {
            assert!(!app.harness.running);
            assert!(!app.git_history.running);
            super::context::smoke_begin(app, window, cx);
            *stage = 4;
        }
        4 => {
            super::context::smoke_finish(app, window, cx);
            super::review::smoke_begin(app, window, cx);
            *stage = 5;
        }
        5 if !app.git_history.running => {
            if super::review::smoke_edit(app, cx) {
                *stage = 6;
            }
        }
        6 => {
            if super::review::smoke_diff(app, cx) {
                *stage = 7;
            }
        }
        7 => {
            if super::review::smoke_finish(app, cx) {
                super::checks::smoke_begin(app, window, cx);
                *stage = 8;
            }
        }
        8 => {
            println!(
                "PASS: native GitHub CI/PR views and publication forms, context envelope, approved decision persistence/draft, real isolated checkpoint capture and review diff, check report/diagnostic UI; draft text retained, no agent sends or real GitHub calls. No manual publication/restore dialog acceptance QA."
            );
            cx.quit();
            return true;
        }
        _ => {}
    }
    false
}
fn action(
    id: impl Into<ElementId>,
    label: &'static str,
    value: Action,
    busy: bool,
    cx: &mut Context<DunlinApp>,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .rounded(px(4.))
        .label(label)
        .disabled(busy)
        .on_click(cx.listener(move |this, _, _, cx| this.prepare_github_action(value.clone(), cx)))
}
fn read_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    read: Read,
    busy: bool,
    cx: &mut Context<DunlinApp>,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .rounded(px(4.))
        .label(label)
        .disabled(busy)
        .on_click(cx.listener(move |this, _, _, cx| this.github_read(read.clone(), cx)))
}
fn badge(status: &str, conclusion: Option<&str>, cx: &App) -> AnyElement {
    let p = palette(cx);
    let value = conclusion.unwrap_or(status);
    let (symbol, color) = match value {
        "success" => ("✓", p.green),
        "failure" | "timed_out" | "action_required" => ("×", p.red),
        "in_progress" | "queued" | "waiting" | "pending" | "requested" => ("●", p.amber),
        _ => ("—", p.muted_fg),
    };
    div()
        .flex_shrink_0()
        .text_color(color)
        .text_size(px(11.))
        .child(format!("{symbol} {}", plain(value)))
        .into_any_element()
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let s = &view.github;
    let p = palette(cx);
    let busy = this.git_history.running || s.loading;
    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .text_size(px(12.));
    if let Some(data) = &s.data {
        content = content.child(format!("github.com/{} · remote {} · account {} · local HEAD {}", data.repository.name,data.repository.remote, data.account, data.repository.local_head))
            .child("Up to 20 CI runs for an exact commit; 50 most recently updated PRs, all states. No push/checkout. Logs and PR text can contain secrets: inspect before adding to an agent draft.")
            .child(Button::new("github-follow").ghost().small().label(if s.follow { "Pause 15s refresh" } else { "Follow CI every 15s" }).disabled(busy).on_click(cx.listener(|this, _, _, cx| {
                let Some(v) = &mut this.git_history.view else { return; }; v.github.follow = !v.github.follow;
                let read = v.github.selected_run.clone().map(Read::Jobs).unwrap_or(Read::Overview); this.github_read(read, cx);
            })))
            .child(Button::new("github-create-form").ghost().small().label("New PR…").on_click(cx.listener(|this, _, _, cx| { if let Some(v) = &mut this.git_history.view { v.github.create_form = !v.github.create_form; } cx.notify(); })))
            .child(div().text_color(p.muted_fg).child(s.checked.map_or("Not refreshed".into(), |t| format!("Last response {}s ago · polled status, not live logs", t.elapsed().as_secs()))));
        for run in &data.runs {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(badge(&run.status, run.conclusion.as_deref(), cx))
                    .child(read_button(
                        format!("ci-run-{}", run.id),
                        format!(
                            "{} · #{} attempt {} · {}",
                            plain(run.name.as_deref().unwrap_or("Workflow")),
                            run.id,
                            run.run_attempt,
                            plain(&run.event)
                        ),
                        Read::Jobs(run.clone()),
                        busy,
                        cx,
                    )),
            );
        }
        if data.runs.is_empty() {
            content = content
                .child("No Actions runs for this commit (not proof that external checks passed).");
        }
        if let Some(run) = &s.selected_run {
            content = content
                .child(format!(
                    "Workflow #{} · attempt {} · commit {}",
                    run.id, run.run_attempt, run.head_sha
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .child(action(
                            "ci-cancel",
                            "Cancel run…",
                            Action::Cancel { run: run.id },
                            busy,
                            cx,
                        ))
                        .child(action(
                            "ci-rerun-failed",
                            "Rerun failed…",
                            Action::Rerun {
                                run: run.id,
                                failed_only: true,
                            },
                            busy,
                            cx,
                        ))
                        .child(action(
                            "ci-rerun",
                            "Rerun all…",
                            Action::Rerun {
                                run: run.id,
                                failed_only: false,
                            },
                            busy,
                            cx,
                        )),
                );
            content=content.child("CI preview shows up to 20 jobs and 100 steps per job; inspect the workflow on GitHub for larger runs.");
            let url = format!(
                "https://github.com/{}/actions/runs/{}",
                data.repository.name, run.id
            );
            content = content.child(
                Button::new("github-run-web")
                    .ghost()
                    .small()
                    .label("Open workflow on GitHub")
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
            for job in s.jobs.iter().take(20) {
                let evidence = format!(
                    "Run {} attempt {} commit {}\n{}",
                    run.id,
                    run.run_attempt,
                    run.head_sha,
                    serde_json::to_string_pretty(job).unwrap_or_default()
                );
                let mut block = div()
                    .border_l_2()
                    .border_color(p.border)
                    .pl(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .child(badge(&job.status, job.conclusion.as_deref(), cx))
                            .child(plain(&job.name))
                            .child(read_button(
                                format!("ci-log-{}", job.id),
                                "Load completed-job log",
                                Read::Log(run.clone(), job.clone()),
                                busy,
                                cx,
                            ))
                            .child(
                                Button::new(format!("ci-evidence-{}", job.id))
                                    .ghost()
                                    .small()
                                    .label("Add status/error to agent draft")
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.github_evidence(evidence.clone(), w, cx)
                                    })),
                            ),
                    );
                for step in job.steps.iter().take(100) {
                    block = block.child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .pl(px(10.))
                            .child(badge(&step.status, step.conclusion.as_deref(), cx))
                            .child(format!("{} · {}", step.number, plain(&step.name))),
                    );
                }
                content = content.child(block);
            }
        }
        if let Some(log) = &s.log {
            let text = log.clone();
            content = content
                .child(
                    Button::new("ci-log-to-agent")
                        .ghost()
                        .small()
                        .label("Add displayed log to agent draft")
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.github_evidence(text.clone(), w, cx)
                        })),
                )
                .child(
                    div()
                        .font_family("Menlo")
                        .text_size(px(11.))
                        .child(log.clone()),
                );
        }
        content = content.child(
            div()
                .pt(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Pull requests"),
        );
        for pr in &data.pulls {
            content = content.child(read_button(
                format!("github-pr-{}", pr.number),
                format!(
                    "#{} · {} · {}{}",
                    pr.number,
                    plain(&pr.state),
                    plain(&pr.title),
                    if pr.draft { " [draft]" } else { "" }
                ),
                Read::Pull(pr.number),
                busy,
                cx,
            ));
        }
        if let Some(detail) = &s.detail {
            let pr = &detail.pull;
            let number = pr.number;
            let url = format!("https://github.com/{}/pull/{number}", data.repository.name);
            content = content
                .child(format!(
                    "#{} {} · {} → {} · head {}",
                    number,
                    plain(&pr.title),
                    plain(&pr.head.name),
                    plain(&pr.base.name),
                    pr.head.sha
                ))
                .child(plain(pr.body.as_deref().unwrap_or("")))
                .child(
                    Button::new("github-pr-web")
                        .ghost()
                        .small()
                        .label("Open PR on GitHub")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            if detail.bounded_lists {
                content = content.child(div().text_color(p.amber).child("100-item cap reached: files/discussions/checks may be incomplete. Inspect the full PR on GitHub."));
            }
            let evidence = format!(
                "PR #{} {}\nHead {}\n{}\nChecks:\n{}\nReviews:\n{}",
                number,
                pr.title,
                pr.head.sha,
                pr.body.as_deref().unwrap_or(""),
                serde_json::to_string_pretty(&detail.checks).unwrap_or_default(),
                serde_json::to_string_pretty(&detail.reviews).unwrap_or_default()
            );
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(
                        Button::new("github-pr-evidence")
                            .ghost()
                            .small()
                            .label("Add PR/checks to agent draft")
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.github_evidence(evidence.clone(), w, cx)
                            })),
                    )
                    .child(read_button(
                        "github-pr-refresh",
                        "Refresh PR",
                        Read::Pull(number),
                        busy,
                        cx,
                    ))
                    .child(read_button(
                        "github-pr-ci",
                        "Load CI for PR head",
                        Read::Runs(pr.head.sha.clone()),
                        busy,
                        cx,
                    ))
                    .child(action(
                        "github-pr-close",
                        "Close…",
                        Action::State {
                            number,
                            open: false,
                        },
                        busy,
                        cx,
                    ))
                    .child(action(
                        "github-pr-reopen",
                        "Reopen…",
                        Action::State { number, open: true },
                        busy,
                        cx,
                    )),
            );
            content = content.child(action(
                "github-pr-draft",
                if pr.draft {
                    "Mark ready for review…"
                } else {
                    "Convert to draft…"
                },
                Action::Draft {
                    number,
                    draft: !pr.draft,
                },
                busy,
                cx,
            ));
            for method in ["merge", "squash", "rebase"] {
                content = content.child(action(
                    format!("github-merge-{method}"),
                    match method {
                        "merge" => "Merge…",
                        "squash" => "Squash merge…",
                        _ => "Rebase merge…",
                    },
                    Action::Merge {
                        number,
                        method: method.into(),
                    },
                    busy,
                    cx,
                ));
            }
            for check in detail.checks.iter().chain(&detail.statuses) {
                content = content.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(badge(
                            check["status"]
                                .as_str()
                                .or_else(|| check["state"].as_str())
                                .unwrap_or("unknown"),
                            check["conclusion"].as_str(),
                            cx,
                        ))
                        .child(plain(
                            check["name"]
                                .as_str()
                                .or_else(|| check["context"].as_str())
                                .unwrap_or("Check"),
                        )),
                );
            }
            content=content.child("UI preview: up to 20 entries per discussion section and 20 files; long bodies/patches are truncated. Use GitHub for the complete review.");
            for (kind, values) in [
                ("Comment", &detail.comments),
                ("Review", &detail.reviews),
                ("Line review", &detail.review_comments),
            ] {
                for value in values.iter().take(20) {
                    content = content.child(format!(
                        "{} · {} · {} · {}",
                        kind,
                        plain(value["user"]["login"].as_str().unwrap_or("unknown")),
                        plain(value["state"].as_str().unwrap_or("")),
                        api::display_text(value["body"].as_str().unwrap_or(""), 4000)
                    ));
                }
            }
            for file in detail.files.iter().take(20) {
                content = content
                    .child(
                        div()
                            .pt(px(8.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} · +{} -{}",
                                plain(file["filename"].as_str().unwrap_or("unknown")),
                                file["additions"],
                                file["deletions"]
                            )),
                    )
                    .child(
                        div().font_family("Menlo").text_size(px(11.)).child(
                            file["patch"]
                                .as_str()
                                .map(|t| api::display_text(t, 12000))
                                .unwrap_or(
                                    "Patch unavailable (binary/large file); inspect on GitHub"
                                        .into(),
                                ),
                        ),
                    );
            }
        }
        if s.create_form || s.detail.is_some() {
            content = content
                .child(
                    div()
                        .pt(px(12.))
                        .child("Publication draft · never auto-pushes or sends"),
                )
                .child(Input::new(&s.title).small())
                .child(Textarea::new(&s.body).h(px(110.)));
            let title = s.title.read(cx).value().to_string();
            let body = s.body.read(cx).value().to_string();
            if s.create_form {
                content = content
                    .child(Input::new(&s.base).small())
                    .child(Input::new(&s.head).small());
                for draft in [true, false] {
                    content = content.child(action(
                        format!("github-create-{draft}"),
                        if draft {
                            "Create draft PR…"
                        } else {
                            "Create ready PR…"
                        },
                        Action::Create {
                            base: s.base.read(cx).value().to_string(),
                            head: s.head.read(cx).value().to_string(),
                            title: title.clone(),
                            body: body.clone(),
                            draft,
                        },
                        busy,
                        cx,
                    ));
                }
            } else if let Some(detail) = &s.detail {
                let number = detail.pull.number;
                let copy_title = detail.pull.title.clone();
                let copy_body = detail.pull.body.clone().unwrap_or_default();
                content = content.child(Button::new("github-copy-pr-form").ghost().small().label("Load current title/body into publication draft").on_click(cx.listener(move |this, _, w, cx| {
                    if let Some(v) = &this.git_history.view {
                        if !v.github.title.read(cx).value().is_empty() || !v.github.body.read(cx).value().is_empty() { this.file_notice("Clear the publication draft first; existing text was preserved", cx); return; }
                        v.github.title.update(cx, |s, cx| s.set_value(copy_title.clone(), w, cx)); v.github.body.update(cx, |s, cx| s.set_value(copy_body.clone(), w, cx));
                    }
                })))
                    .child(action("github-edit", "Publish title/description…", Action::Edit { number, title, body: body.clone() }, busy, cx))
                    .child(action("github-comment", "Publish comment…", Action::Comment { number, body: body.clone() }, busy, cx));
                for event in ["COMMENT", "APPROVE", "REQUEST_CHANGES"] {
                    content = content.child(action(
                        format!("github-review-{event}"),
                        match event {
                            "APPROVE" => "Approve…",
                            "REQUEST_CHANGES" => "Request changes…",
                            _ => "Submit review comment…",
                        },
                        Action::Review {
                            number,
                            body: body.clone(),
                            event: event.into(),
                        },
                        busy,
                        cx,
                    ));
                }
                content = content
                    .child(Input::new(&s.file).small())
                    .child(Input::new(&s.line).small())
                    .child(action(
                        "github-line-comment",
                        "Publish RIGHT-side line comment…",
                        Action::LineComment {
                            number,
                            path: s.file.read(cx).value().to_string(),
                            line: s.line.read(cx).value().parse().unwrap_or(0),
                            body,
                        },
                        busy,
                        cx,
                    ));
            }
        }
    } else {
        content = content.child("Uses installed gh and its existing authentication. Choose a remote, then Load GitHub. Opening this view does not log in or write to GitHub.");
    }
    if let Some((_, plan)) = &s.pending {
        content = content
            .child(
                div()
                    .border_t_1()
                    .border_color(p.border)
                    .pt(px(8.))
                    .child(plain(plan.summary())),
            )
            .child(
                Button::new("github-confirm")
                    .small()
                    .label("Review and confirm publication…")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| this.confirm_github_action(w, cx))),
            )
            .child(
                Button::new("github-cancel-plan")
                    .ghost()
                    .small()
                    .label("Discard prepared request")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(v) = &mut this.git_history.view {
                            v.github.pending = None;
                        }
                        cx.notify();
                    })),
            );
    }
    if let Some(error) = &s.error {
        content = content.child(div().text_color(p.red).child(error.clone()));
    }
    div()
        .id("github-panel")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.bg)
        .text_color(p.fg)
        .child(
            div()
                .flex_shrink_0()
                .p(px(10.))
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child("GitHub · CI / PR")
                .child(
                    Button::new("github-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.github.visible = false;
                                v.github.follow = false;
                            }
                            cx.notify();
                        })),
                )
                .child(Input::new(&s.remote).small().w(px(150.)))
                .child(read_button(
                    "github-load",
                    if s.loading {
                        "Loading…"
                    } else {
                        "Load / refresh GitHub"
                    },
                    Read::Overview,
                    busy,
                    cx,
                )),
        )
        .child(
            div()
                .id("github-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(content),
        )
        .into_any_element()
}
