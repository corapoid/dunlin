use super::*;
use dunlin_domain::task_budget::{Budget, Member};
pub(super) struct State {
    pub visible: bool,
    label: Entity<InputState>,
    requests: Entity<InputState>,
    seconds: Entity<InputState>,
    usd: Entity<InputState>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(w: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            label: cx.new(|cx| InputState::new(w, cx).placeholder("Task label")),
            requests: cx.new(|cx| {
                InputState::new(w, cx)
                    .default_value("4")
                    .placeholder("Client request cap")
            }),
            seconds: cx.new(|cx| {
                InputState::new(w, cx)
                    .default_value("600")
                    .placeholder("Wall time seconds")
            }),
            usd: cx.new(|cx| InputState::new(w, cx).placeholder("Optional reported USD ceiling")),
            scroll: ScrollHandle::new(),
        }
    }
}
impl DunlinApp {
    fn forget_task_budget(&mut self, id: String, w: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.task_budgets.registry.clone();
        if let Err(e) = next.forget_released(&id) {
            self.file_notice(e, cx);
            return;
        }
        let receiver=w.prompt(PromptLevel::Warning,"Forget released budget history?",Some("Removes only this released ledger record, including its cost/request evidence. Older overlapping records must be released and forgotten first so no prior assignment can be revived. Sessions, queued text, comparison results and files are retained. No prompt is sent."),&["Cancel","Forget record"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let mut next = this.task_budgets.registry.clone();
                if let Err(e) = next
                    .forget_released(&id)
                    .and_then(|_| this.persist_task_budgets(next))
                {
                    this.file_notice(e, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn open_task_budgets(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(w, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let v = self.git_history.view.as_mut().unwrap();
        v.hide_tools();
        v.budgets.visible = true;
        cx.notify();
    }
    fn arm_session_budget(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.harness.running || !self.harness.queued_prompts().is_empty() {
            self.file_notice("Finish/stop this turn and explicitly clear or send queued prompts before approving a new budget",cx);
            return;
        }
        let Some(v) = &self.git_history.view else {
            return;
        };
        if !owns(&v.project, self.project()) {
            return;
        }
        let s = &v.budgets;
        let label = s.label.read(cx).value().to_string();
        let requests = s.requests.read(cx).value().parse::<u32>();
        let seconds = s.seconds.read(cx).value().parse::<u64>();
        let usd = s.usd.read(cx).value();
        let limit = if usd.trim().is_empty() {
            Ok(None)
        } else {
            usd.parse::<f64>()
                .ok()
                .and_then(dunlin_domain::task_budget::micro_usd)
                .filter(|n| *n > 0)
                .map(Some)
                .ok_or("USD ceiling must be finite and positive")
        };
        let (Ok(requests), Ok(seconds), Ok(limit)) = (requests, seconds, limit) else {
            self.file_notice(
                "Use integer request/time limits and an optional finite positive USD ceiling",
                cx,
            );
            return;
        };
        if !(1..=1000).contains(&requests)
            || !(1..=86400).contains(&seconds)
            || label.trim().is_empty()
            || label.len() > 256
        {
            self.file_notice(
                "Use a label, 1–1,000 client requests and 1–86,400 seconds",
                cx,
            );
            return;
        }
        let project = v.project.clone();
        let session = self.session().id.clone();
        let view_id = v.id;
        let description = format!(
            "Task: {label}\nSession: {session}\nRoot: {}\nAt most {requests} client wire requests (failed attempts/retries count); {seconds}s wall time from approval.\nReported USD ceiling: {:?}\n\nUSD limits need fresh verified cumulative USD; unknown is not zero. In-flight/late costs can exceed this ceiling: this is NOT a provider billing cap. Internal harness retries/child turns are not client requests; unknown child cost coverage stops USD-limited work. Cancellation is requested, not a sandbox kill. The ledger survives view closure/restart. A new approval explicitly replaces this session's prior budget.",
            project.path,
            limit.map(|v| v as f64 / 1_000_000.)
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Approve this task budget?",
            Some(&description),
            &["Cancel", "Approve limits"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if !this
                    .git_history
                    .view
                    .as_ref()
                    .is_some_and(|v| v.id == view_id && owns(&project, this.project()))
                    || this.session().id != session
                    || this.harness.running
                    || !this.harness.queued_prompts().is_empty()
                {
                    return;
                }
                let now = crate::harness::now_ms();
                let cost = this.harness.reported_cost.clone();
                let budget = Budget {
                    id: uuid::Uuid::now_v7().to_string(),
                    label,
                    started_ms: now,
                    deadline_ms: now + seconds * 1000,
                    max_attempts: requests,
                    reported_usd_limit: limit,
                    stopped: None,
                    members: vec![Member {
                        session,
                        project: project.id,
                        root: project.path,
                        attempts: 0,
                        baseline: cost.clone(),
                        latest: cost,
                        opaque_children: false,
                    }],
                };
                match this.approve_task_budget(budget) {
                    Ok(()) => this.file_notice("Task limits approved; no prompt was sent", cx),
                    Err(e) => this.file_notice(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn change_task_budget(
        &mut self,
        id: String,
        release: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.task_budgets.registry.tasks.iter().find(|t| t.id == id) else {
            return;
        };
        if release
            && task.members.iter().any(|m| {
                if m.session == self.session().id {
                    self.harness.running || !self.harness.queued_prompts().is_empty()
                } else {
                    self.parked_runtimes
                        .get(&m.session)
                        .is_some_and(|r| r.state.running || !r.state.queued_prompts().is_empty())
                }
            })
        {
            self.file_notice("Stop all assigned sessions and explicitly clear/send their queued prompts before releasing limits",cx);
            return;
        }
        let summary = format!(
            "{}\n{}",
            task.label,
            if release {
                "This releases limits for ALL sessions assigned to this task. Future prompts may be unlimited unless a newer budget applies. No prompt is sent by this action."
            } else {
                "This pauses ALL assigned sessions and requests cancellation. Queued text remains available; the budget continues blocking new sends."
            }
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            if release {
                "Release this task's limits?"
            } else {
                "Stop this budgeted task?"
            },
            Some(&summary),
            &["Cancel", "Confirm"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let mut next = this.task_budgets.registry.clone();
                let Some(task) = next.tasks.iter_mut().find(|t| t.id == id) else {
                    return;
                };
                if release
                    && task.members.iter().any(|m| {
                        if m.session == this.session().id {
                            this.harness.running || !this.harness.queued_prompts().is_empty()
                        } else {
                            this.parked_runtimes.get(&m.session).is_some_and(|r| {
                                r.state.running || !r.state.queued_prompts().is_empty()
                            })
                        }
                    })
                {
                    return;
                }
                task.stopped = Some(
                    if release {
                        "Released by user"
                    } else {
                        "Stopped by user"
                    }
                    .into(),
                );
                if let Err(e) = this.persist_task_budgets(next) {
                    this.file_notice(e, cx);
                }
                this.poll_task_budgets(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let s = &this.git_history.view.as_ref().unwrap().budgets;
    let p = palette(cx);
    let mut body=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.))
        .child("Durable client-request/time budgets. Reported USD is optional and can be unknown; it is not a guaranteed provider billing cap. Queued/retried client sends share the ledger; opaque harness child work is not falsely priced at zero.")
        .child(Input::new(&s.label).small()).child(Input::new(&s.requests).small()).child(Input::new(&s.seconds).small()).child(Input::new(&s.usd).small())
        .child(dunlin_ui::button("budget-arm").label("Review and approve session limits…").on_click(cx.listener(|this,_,w,cx|this.arm_session_budget(w,cx))));
    if let Some(e) = &this.task_budgets.error {
        body = body.child(div().text_color(p.red).child(e.clone()));
    }
    for task in this
        .task_budgets
        .registry
        .tasks
        .iter()
        .rev()
        .filter(|t| t.members.iter().any(|m| m.project == this.project().id))
    {
        let id = task.id.clone();
        let release = id.clone();
        let forget = id.clone();
        body = body
            .child(
                div()
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(p.border)
                    .child(format!(
                        "{} · requests {}/{} · remaining {}s · reported USD {} · {}",
                        task.label,
                        task.attempts(),
                        task.max_attempts,
                        task.deadline_ms.saturating_sub(crate::harness::now_ms()) / 1000,
                        task.spent()
                            .map(|v| format!("{:.6}", v as f64 / 1_000_000.))
                            .unwrap_or("unknown".into()),
                        task.stopped.as_deref().unwrap_or("approved")
                    )),
            )
            .child(
                dunlin_ui::button(format!("budget-stop-{id}"))
                    .ghost()
                    .small()
                    .label("Stop assigned work…")
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.change_task_budget(id.clone(), false, w, cx)
                    })),
            )
            .child(
                dunlin_ui::button(format!("budget-release-{release}"))
                    .ghost()
                    .small()
                    .label("Release limits…")
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.change_task_budget(release.clone(), true, w, cx)
                    })),
            );
        body = body.child(
            dunlin_ui::button(format!("budget-forget-{forget}"))
                .ghost()
                .small()
                .label("Forget released history…")
                .disabled(task.stopped.as_deref() != Some("Released by user"))
                .on_click(cx.listener(move |this, _, w, cx| {
                    this.forget_task_budget(forget.clone(), w, cx)
                })),
        );
        for member in &task.members {
            body = body.child(format!(
                "{} · {} · attempts {} · child cost coverage {}",
                member.session,
                member.root,
                member.attempts,
                if member.opaque_children {
                    "unknown"
                } else {
                    "no child work observed"
                }
            ));
        }
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.bg)
        .text_color(p.fg)
        .child(
            div()
                .p(px(10.))
                .flex_shrink_0()
                .flex()
                .gap(px(8.))
                .child("Task budgets")
                .child(
                    dunlin_ui::button("budgets-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.budgets.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("budget-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}
