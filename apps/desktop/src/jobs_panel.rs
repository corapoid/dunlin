//! Native controls for relay-owned queues. All enqueue operations require a
//! separate human approval; opening this view never starts a paid turn.
use crate::{
    app::DunlinApp,
    remote::{self, RemoteHost, RemoteProjectInfo},
    server_panel,
};
use dunlin_domain::jobs::{Enqueue, Profile, Queue, Status};
use dunlin_mcp_client::linear::{Issue, issue_digest};
use dunlin_ui::{
    component::{
        Disableable,
        button::ButtonVariants as _,
        input::InputState,
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
    *,
};
use std::time::{Duration, Instant};
#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Default)]
pub(crate) struct State {
    pub queue: Option<Queue>,
    pub error: Option<String>,
    request: Option<String>,
    submission: Option<String>,
    fetched: Option<Instant>,
    pub draft: Option<Draft>,
}
pub(crate) struct Draft {
    issue: Issue,
    host: String,
    projects: Vec<RemoteProjectInfo>,
    project: Option<String>,
    harness: String,
    model: Entity<InputState>,
    mode: Entity<InputState>,
    seconds: Entity<InputState>,
    tools: Entity<InputState>,
    review: Entity<InputState>,
    allow_tools: bool,
    loading: Option<String>,
    approval: bool,
}
impl DunlinApp {
    pub(crate) fn refresh_job_queue(&mut self, cx: &mut Context<Self>) {
        if self.server_panel.jobs.request.is_some()
            || self
                .server_panel
                .jobs
                .fetched
                .is_some_and(|time| time.elapsed() < Duration::from_secs(4))
        {
            return;
        }
        let Ok((host, token)) = self.server_credentials() else {
            return;
        };
        let ticket = uuid::Uuid::now_v7().to_string();
        self.server_panel.jobs.request = Some(ticket.clone());
        self.server_panel.jobs.fetched = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let worker = host.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    server_panel::request::<Queue>(&worker, &token, "GET", "/v1/jobs", None::<&()>)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&host.id)
                    || this.server_panel.jobs.request.as_deref() != Some(&ticket)
                    || !this.remote_hosts.iter().any(|current| current == &host)
                {
                    return;
                }
                this.server_panel.jobs.request = None;
                match result {
                    Ok(queue) if queue.validate().is_ok() => {
                        this.server_panel.jobs.queue = Some(queue);
                        if this.server_panel.jobs.submission.is_none() {
                            this.server_panel.jobs.error = None;
                        }
                    }
                    Ok(_) => this.server_panel.jobs.error = Some("Invalid queue snapshot".into()),
                    Err(error) => this.server_panel.jobs.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn assign_linear_job(
        &mut self,
        issue: Issue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.server_panel.jobs.submission.is_some()
            || self
                .server_panel
                .jobs
                .draft
                .as_ref()
                .is_some_and(|draft| draft.approval)
        {
            return;
        }
        let Some(host) = self.server_panel.host.clone() else {
            return;
        };
        let input = |placeholder: &'static str,
                     value: &str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        self.server_panel.jobs.draft = Some(Draft {
            issue,
            host: host.clone(),
            projects: Vec::new(),
            project: None,
            harness: "opencode".into(),
            model: input("Exact model id", "", window, cx),
            mode: input("Exact mode id", "build", window, cx),
            seconds: input("Seconds (30–3600)", "900", window, cx),
            tools: input("Tool requests (1–256)", "32", window, cx),
            review: input("Linear review status", "In Review", window, cx),
            allow_tools: false,
            loading: None,
            approval: false,
        });
        self.job_target(host, cx);
        cx.notify();
    }
    fn job_target(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(host) = self.remote_host(&id) else {
            return;
        };
        let token = match self.remote_token(&id) {
            Ok(token) => token,
            Err(error) => {
                self.server_panel.jobs.error = Some(error);
                return;
            }
        };
        let Some(draft) = self.server_panel.jobs.draft.as_mut() else {
            return;
        };
        if draft.approval {
            return;
        }
        let ticket = uuid::Uuid::now_v7().to_string();
        draft.host = id.clone();
        draft.project = None;
        draft.projects.clear();
        draft.loading = Some(ticket.clone());
        cx.spawn(async move |this, cx| {
            let url = host.url.clone();
            let result = cx
                .background_executor()
                .spawn(async move { remote::projects(&url, &token) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(draft) = this.server_panel.jobs.draft.as_mut() else {
                    return;
                };
                if draft.host != id
                    || draft.loading.as_deref() != Some(&ticket)
                    || !this.remote_hosts.iter().any(|current| current == &host)
                {
                    return;
                }
                draft.loading = None;
                match result {
                    Ok(projects) => {
                        draft.project = projects.first().map(|project| project.id.clone());
                        draft.projects = projects;
                        this.server_panel.jobs.error = None;
                    }
                    Err(error) => this.server_panel.jobs.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn approve_job(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.server_panel.jobs.draft else {
            return;
        };
        if draft.approval || draft.loading.is_some() {
            return;
        }
        let Some(project) = draft.project.clone() else {
            self.server_panel.jobs.error = Some("Choose a project on the target host".into());
            cx.notify();
            return;
        };
        let Some(host) = self.remote_host(&draft.host) else {
            return;
        };
        let profile = Profile {
            harness: draft.harness.clone(),
            model: draft.model.read(cx).value().trim().into(),
            mode: draft.mode.read(cx).value().trim().into(),
            timeout_secs: draft.seconds.read(cx).value().trim().parse().unwrap_or(0),
            max_tool_requests: draft.tools.read(cx).value().trim().parse().unwrap_or(0),
            allow_tools: draft.allow_tools,
            review_state: draft.review.read(cx).value().trim().into(),
        };
        if let Err(error) = profile.validate() {
            self.server_panel.jobs.error = Some(error);
            cx.notify();
            return;
        }
        let request = Enqueue {
            issue_id: draft.issue.id.clone(),
            expected_issue_digest: issue_digest(&draft.issue),
            project_id: project.clone(),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            profile,
            approved: true,
        };
        let explanation = format!(
            "Task: {} — {}\nHost: {}\nProject: {}\nHarness: {}\nModel: {}\nMode: {}\nBudget: one ACP turn, {} seconds, {} client tool requests (not a monetary cap).\nPermission requests: {}\nLinear review status: {}\n\nThe relay creates an isolated checkout from committed HEAD. It does not automatically merge or push. A normally finished agent turn means review required, not verified task success. The target host must have its own Linear authorization and an identical fresh issue snapshot. Do not assign this issue to another host while this run is active.",
            draft.issue.identifier,
            draft.issue.title,
            host.name,
            project,
            request.profile.harness,
            request.profile.model,
            request.profile.mode,
            request.profile.timeout_secs,
            request.profile.max_tool_requests,
            if request.profile.allow_tools {
                "allow once inside the relay sandbox"
            } else {
                "deny"
            },
            request.profile.review_state
        );
        self.server_panel.jobs.draft.as_mut().unwrap().approval = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Approve unattended execution on this host?",
            Some(&explanation),
            &[
                PromptButton::cancel("Cancel"),
                PromptButton::new("Approve and enqueue"),
            ],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn(async move |this, cx| {
            let approved = answer.await.ok() == Some(1);
            let _ = this.update(cx, |this, cx| {
                let same = this.server_panel.jobs.draft.as_ref().is_some_and(|draft| {
                    draft.host == host.id && draft.issue.id == request.issue_id && draft.approval
                });
                if !same {
                    return;
                }
                this.server_panel.jobs.draft.as_mut().unwrap().approval = false;
                if approved {
                    this.submit_job(host, request, cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn submit_job(&mut self, host: RemoteHost, request: Enqueue, cx: &mut Context<Self>) {
        let token = match self.remote_token(&host.id) {
            Ok(token) => token,
            Err(error) => {
                self.server_panel.jobs.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.server_panel.jobs.draft = None;
        let source = self.server_panel.host.clone();
        let ticket = uuid::Uuid::now_v7().to_string();
        self.server_panel.jobs.submission = Some(ticket.clone());
        cx.spawn(async move |this, cx| {
            let worker = host.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    server_panel::request::<dunlin_domain::jobs::Job>(
                        &worker,
                        &token,
                        "POST",
                        "/v1/jobs",
                        Some(&request),
                    )
                })
                .await;
            let _ =
                this.update(cx, |this, cx| {
                    if this.server_panel.host != source
                        || this.server_panel.jobs.submission.as_deref() != Some(&ticket)
                    {
                        return;
                    }
                    this.server_panel.jobs.submission = None;
                    match result {
                        Ok(job) if job.validate().is_ok() => {
                            this.server_panel.jobs.error = None;
                            // Follow the actual target after cross-host assignment, not the source inbox.
                            if this.server_panel.host.as_deref() != Some(&host.id) {
                                this.server_panel = server_panel::State {
                                    host: Some(host.id.clone()),
                                    ..Default::default()
                                };
                            }
                            this.server_panel.jobs.fetched = None;
                            this.refresh_server(cx);
                            this.notice = Some(format!(
                                "Task queued on {}. The relay owns execution.",
                                host.name
                            ));
                        }
                        Ok(_) => this.server_panel.jobs.error = Some(
                            "Invalid enqueue response; inspect the target queue before retrying"
                                .into(),
                        ),
                        Err(error) => this.server_panel.jobs.error = Some(error),
                    }
                    cx.notify();
                });
        })
        .detach();
        cx.notify();
    }
    fn job_action(
        &mut self,
        id: Option<String>,
        action: &'static str,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        if self.server_panel.jobs.request.is_some() {
            return;
        }
        let Ok((host, token)) = self.server_credentials() else {
            return;
        };
        let path = id
            .map(|id| format!("/v1/jobs/{id}"))
            .unwrap_or_else(|| "/v1/jobs/queue".into());
        let ticket = uuid::Uuid::now_v7().to_string();
        self.server_panel.jobs.request = Some(ticket.clone());
        cx.spawn(async move |this, cx| {
            let worker = host.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    server_panel::request::<serde_json::Value>(
                        &worker,
                        &token,
                        "POST",
                        &path,
                        Some(&dunlin_domain::jobs::Action {
                            action: action.into(),
                            approved,
                        }),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&host.id)
                    || this.server_panel.jobs.request.as_deref() != Some(&ticket)
                {
                    return;
                }
                this.server_panel.jobs.request = None;
                match result {
                    Ok(_) => {
                        this.server_panel.jobs.error = None;
                        this.server_panel.jobs.fetched = None;
                        this.refresh_job_queue(cx);
                    }
                    Err(error) => this.server_panel.jobs.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn open_job_checkout(&mut self, host_id: &str, project: String, cx: &mut Context<Self>) {
        let Some(host) = self.remote_host(host_id) else {
            return;
        };
        let token = match self.remote_token(host_id) {
            Ok(token) => token,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let url = host.url.clone();
            let result = cx
                .background_executor()
                .spawn(async move { remote::projects(&url, &token) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.remote_hosts.iter().any(|current| current == &host) {
                    return;
                }
                match result {
                    Ok(projects) => {
                        if let Some(info) = projects.into_iter().find(|info| info.id == project) {
                            this.add_remote_project(&host.id, &info, cx);
                        } else {
                            this.set_notice(
                                "Review checkout is no longer granted on the server",
                                cx,
                            );
                        }
                    }
                    Err(error) => this.set_notice(error, cx),
                }
            });
        })
        .detach();
    }
    fn confirm_job_action(
        &mut self,
        id: Option<String>,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = if action == "reconcile" {
            "Confirm that you have inspected the server and stopped all orphaned job processes. Interrupted runs will be marked cancelled; their prompts are never replayed automatically. The queue remains paused until you resume it."
        } else if action == "resolve_status" {
            "Confirm that you inspected Linear and handled the issue status manually. This resolves the delivery record without sending any mutation to Linear. It does not mark the task verified or complete."
        } else {
            "Inspect the issue in Linear first. A previous status request may already have completed. Retry only if the approved review transition is still appropriate."
        };
        let host = self.server_panel.host.clone();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Reconcile server state?",
            Some(message),
            &[PromptButton::cancel("Cancel"), PromptButton::new("Confirm")],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn(async move |this, cx| {
            if answer.await.ok() == Some(1) {
                let _ = this.update(cx, |this, cx| {
                    if this.server_panel.host == host {
                        this.job_action(id, action, true, cx);
                    }
                });
            }
        })
        .detach();
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let state = &this.server_panel.jobs;
    let busy = state.request.is_some() || state.submission.is_some();
    let admin = this
        .server_panel
        .info
        .as_ref()
        .is_some_and(|info| info.role == "admin");
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .py(px(12.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Host task queue · one job at a time"),
        );
    if let Some(queue) = &state.queue {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(if queue.paused {
                    "Queue paused"
                } else {
                    "Queue enabled"
                }))
                .when(admin, |row| {
                    row.child(
                        dunlin_ui::settings::button("jobs-pause")
                            .label(if queue.paused { "Resume" } else { "Pause" })
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let paused = this
                                    .server_panel
                                    .jobs
                                    .queue
                                    .as_ref()
                                    .is_some_and(|queue| queue.paused);
                                this.job_action(
                                    None,
                                    if paused { "resume" } else { "pause" },
                                    false,
                                    cx,
                                );
                            })),
                    )
                }),
        );
        if queue.jobs.iter().any(|job| job.status == Status::Unknown) {
            body=body.child(div().text_color(p.amber).child("Interrupted execution blocks new jobs until an administrator verifies that old processes stopped."))
            .when(admin,|body|body.child(dunlin_ui::settings::button("jobs-reconcile").label("Reconcile interrupted runs…").disabled(busy).on_click(cx.listener(|this,_,window,cx|this.confirm_job_action(None,"reconcile",window,cx)))));
        }
        for job in &queue.jobs {
            let can_act = admin || queue.self_user_id.as_deref() == Some(&job.owner);
            let id = job.id.clone();
            let front = id.clone();
            let sync = id.clone();
            let resolve = id.clone();
            let archive = id.clone();
            let checkout = job.checkout_project.clone();
            let project_host = this.server_panel.host.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(p.border)
                    .child(div().text_size(px(12.)).child(format!(
                        "{} · {} · {}",
                        job.identifier,
                        if job.cancel_requested && job.status.active() {
                            "Cancelling"
                        } else {
                            job.status.label()
                        },
                        job.owner_name
                    )))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "{} · {} / {} · Linear: {}",
                                job.title, job.profile.harness, job.profile.model, job.sync_state
                            )),
                    )
                    .when(!job.report.is_empty(), |body| {
                        body.child(div().text_size(px(11.)).child(job.report.clone()))
                    })
                    .when_some(job.sync_error.clone(), |body, error| {
                        body.child(div().text_size(px(11.)).text_color(p.amber).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .when(
                                can_act
                                    && matches!(
                                        job.status,
                                        Status::Queued | Status::Preparing | Status::Running
                                    ),
                                |row| {
                                    row.child(
                                        dunlin_ui::settings::button(SharedString::from(format!(
                                            "job-cancel:{id}"
                                        )))
                                        .label("Cancel")
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.job_action(
                                                    Some(id.clone()),
                                                    "cancel",
                                                    false,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    )
                                },
                            )
                            .when(admin && job.status == Status::Queued, |row| {
                                row.child(
                                    dunlin_ui::settings::button(SharedString::from(format!(
                                        "job-front:{front}"
                                    )))
                                    .label("Move first")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.job_action(Some(front.clone()), "front", false, cx)
                                    })),
                                )
                            })
                            .when(can_act && job.sync_state == "unknown", |row| {
                                row.child(
                                    dunlin_ui::settings::button(SharedString::from(format!(
                                        "job-sync:{sync}"
                                    )))
                                    .label("Reconcile Linear status…")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_job_action(
                                            Some(sync.clone()),
                                            "retry_status",
                                            window,
                                            cx,
                                        )
                                    })),
                                )
                            })
                            .when(
                                can_act && matches!(job.sync_state.as_str(), "unknown" | "pending"),
                                |row| {
                                    row.child(
                                        dunlin_ui::settings::button(SharedString::from(format!(
                                            "job-resolve:{resolve}"
                                        )))
                                        .label("Mark status manually resolved…")
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.confirm_job_action(
                                                    Some(resolve.clone()),
                                                    "resolve_status",
                                                    window,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    )
                                },
                            )
                            .when(
                                can_act
                                    && matches!(
                                        job.status,
                                        Status::Review
                                            | Status::Failed
                                            | Status::Blocked
                                            | Status::Cancelled
                                    )
                                    && matches!(
                                        job.sync_state.as_str(),
                                        "none" | "sent" | "resolved"
                                    ),
                                |row| {
                                    row.child(
                                        dunlin_ui::settings::button(SharedString::from(format!(
                                            "job-archive:{archive}"
                                        )))
                                        .label("Archive record")
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.job_action(
                                                    Some(archive.clone()),
                                                    "archive",
                                                    false,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    )
                                },
                            )
                            .when_some(checkout, |row, project| {
                                row.child(
                                    dunlin_ui::settings::button(SharedString::from(format!(
                                        "job-checkout:{project}"
                                    )))
                                    .label("Open review checkout")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(host) = &project_host {
                                            this.open_job_checkout(host, project.clone(), cx);
                                        }
                                    })),
                                )
                            }),
                    ),
            );
        }
    }
    if let Some(error) = &state.error {
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(p.amber)
                .child(error.clone()),
        );
    }
    if let Some(draft) = &state.draft {
        body = body.child(draft_view(this, draft, cx));
    }
    body.into_any_element()
}

fn draft_view(this: &DunlinApp, draft: &Draft, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let app = cx.entity().downgrade();
    let project_app = app.clone();
    let harness_app = app.clone();
    let hosts = this.remote_hosts.clone();
    let projects = draft.projects.clone();
    let selected = draft.host.clone();
    let frozen = draft.approval || draft.loading.is_some();
    let host_name = hosts
        .iter()
        .find(|host| host.id == selected)
        .map(|host| host.name.clone())
        .unwrap_or_else(|| "Choose host".into());
    let project_name = projects
        .iter()
        .find(|project| Some(&project.id) == draft.project.as_ref())
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "Choose project".into());
    div().flex().flex_col().gap(px(8.)).p(px(12.)).border_1().border_color(p.blue).rounded(px(dunlin_ui::radii::PANEL))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Assign {} · {}",draft.issue.identifier,draft.issue.title)))
        .child(div().id("job-description").max_h(px(140.)).overflow_y_scroll().text_size(px(11.)).child(draft.issue.description.clone()))
        .child(div().flex().flex_wrap().gap(px(8.))
            .child(dunlin_ui::settings::button("job-host").label(host_name).disabled(frozen).dropdown_menu_with_anchor(Anchor::TopLeft,move|mut menu,_,_|{
                for host in &hosts{let app=app.clone();let id=host.id.clone();menu=menu.item(PopupMenuItem::new(host.name.clone()).checked(id==selected).on_click(move|_,_,cx|{let _=app.update(cx,|this,cx|this.job_target(id.clone(),cx));}));}menu}))
            .child(dunlin_ui::settings::button("job-project").label(project_name).disabled(frozen).dropdown_menu_with_anchor(Anchor::TopLeft,move|mut menu,_,_|{
                for project in &projects{let app=project_app.clone();let id=project.id.clone();menu=menu.item(PopupMenuItem::new(project.name.clone()).on_click(move|_,_,cx|{let _=app.update(cx,|this,cx|{if let Some(draft)=&mut this.server_panel.jobs.draft{draft.project=Some(id.clone());}cx.notify();});}));}menu}))
            .child(dunlin_ui::settings::button("job-harness").label(draft.harness.clone()).disabled(frozen).dropdown_menu_with_anchor(Anchor::TopLeft,move|mut menu,_,_|{
                for (id,label,_) in dunlin_workspace::server::HARNESSES{let app=harness_app.clone();menu=menu.item(PopupMenuItem::new(*label).on_click(move|_,_,cx|{let _=app.update(cx,|this,cx|{if let Some(draft)=&mut this.server_panel.jobs.draft{draft.harness=(*id).into();}cx.notify();});}));}menu})))
        .child(dunlin_ui::settings::field("Exact model id",dunlin_ui::input(&draft.model).aria_label("Exact model id").disabled(frozen),p))
        .child(dunlin_ui::settings::field("Exact mode id",dunlin_ui::input(&draft.mode).aria_label("Exact mode id").disabled(frozen),p))
        .child(div().flex().gap(px(8.))
            .child(div().flex_1().child(dunlin_ui::settings::field("Wall-clock seconds",dunlin_ui::input(&draft.seconds).aria_label("Wall-clock seconds").disabled(frozen),p)))
            .child(div().flex_1().child(dunlin_ui::settings::field("Client tool request limit",dunlin_ui::input(&draft.tools).aria_label("Client tool request limit").disabled(frozen),p))))
        .child(dunlin_ui::settings::field("Linear review status",dunlin_ui::input(&draft.review).aria_label("Linear review status").disabled(frozen),p))
        .child(dunlin_ui::switch("job-allow-tools","Allow once permission requests (not read-only isolation)",draft.allow_tools).disabled(frozen).on_click(cx.listener(|this,checked:&bool,_,cx|{if let Some(draft)=&mut this.server_panel.jobs.draft{draft.allow_tools=*checked;}cx.notify();})))
        .child(div().text_size(px(10.5)).text_color(p.muted_fg).child("Each target needs its own Linear authorization. Budgets cover one ACP turn, wall-clock time and client requests—not unseen model calls or provider billing. Changes stay in an isolated checkout."))
        .child(div().flex().gap(px(8.))
            .child(dunlin_ui::settings::button("job-close").relative().children(crate::gui_probe::marker("job-close")).label("Cancel assignment").disabled(frozen).on_click(cx.listener(|this,_,_,cx|{this.server_panel.jobs.draft=None;cx.notify();})))
            .child(dunlin_ui::settings::button("job-approve").relative().children(crate::gui_probe::marker("job-approve")).primary().label("Review and approve…").disabled(frozen).on_click(cx.listener(|this,_,window,cx|this.approve_job(window,cx)))))
        .into_any_element()
}
