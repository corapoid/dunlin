//! Server-owned Linear integration. UI drafts are never persisted or exported;
//! only the selected relay receives the explicitly submitted token.
use crate::{app::DunlinApp, server_panel};
use dunlin_mcp_client::linear::{Filter, Inbox, is_issue_url};
use dunlin_ui::{
    component::{Disableable, button::ButtonVariants as _, input::InputState},
    prelude::FluentBuilder as _,
    *,
};
use serde::Serialize;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;
#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Default)]
pub(crate) struct State {
    pub inbox: Option<Inbox>,
    error: Option<String>,
    request: Option<String>,
    fetched: Option<Instant>,
    form_open: bool,
    token: Option<Entity<InputState>>,
    label: Option<Entity<InputState>>,
    team: Option<Entity<InputState>>,
}

enum Action {
    Load,
    Sync,
    Disconnect,
    Connect {
        filter: Filter,
        token: Zeroizing<String>,
    },
}

impl DunlinApp {
    pub(crate) fn refresh_linear_inbox(&mut self, cx: &mut Context<Self>) {
        if self
            .server_panel
            .linear
            .fetched
            .is_some_and(|checked| checked.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        self.linear_action(Action::Load, cx);
    }

    fn edit_linear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &mut self.server_panel.linear;
        if state.form_open {
            if let Some(token) = &state.token {
                token.update(cx, |input, cx| input.set_value("", window, cx));
            }
            state.form_open = false;
        } else {
            let filter = state
                .inbox
                .as_ref()
                .and_then(|inbox| inbox.filter.clone())
                .unwrap_or(Filter {
                    label: "dunlin".into(),
                    team: None,
                });
            state.token = Some(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Linear API key or access token")
                    .masked(true)
            }));
            state.label = Some(cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("Issue label");
                input.set_value(filter.label, window, cx);
                input
            }));
            state.team = Some(cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("Optional Linear team");
                input.set_value(filter.team.unwrap_or_default(), window, cx);
                input
            }));
            state.form_open = true;
        }
        cx.notify();
    }

    fn save_linear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &mut self.server_panel.linear;
        if state.request.is_some() {
            return;
        }
        let Some(token_input) = state.token.clone() else {
            return;
        };
        let token = Zeroizing::new(token_input.read(cx).value().to_string());
        let label = state
            .label
            .as_ref()
            .unwrap()
            .read(cx)
            .value()
            .trim()
            .to_string();
        let team = state
            .team
            .as_ref()
            .unwrap()
            .read(cx)
            .value()
            .trim()
            .to_string();
        let filter = Filter {
            label,
            team: (!team.is_empty()).then_some(team),
        };
        if filter.validate().is_err()
            || token.is_empty()
            || token.len() > 8192
            || !token.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            state.error = Some(
                "Enter a valid token, a label and optionally a team (128 characters maximum)."
                    .into(),
            );
            cx.notify();
            return;
        }
        token_input.update(cx, |input, cx| input.set_value("", window, cx));
        self.linear_action(Action::Connect { filter, token }, cx);
    }

    fn linear_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.server_panel.linear.request.is_some() || self.server_panel.host.is_none() {
            return;
        }
        let (host, token) = match self.server_credentials() {
            Ok(credentials) => credentials,
            Err(error) => {
                self.server_panel.linear.error = Some(error);
                return;
            }
        };
        let ticket = uuid::Uuid::now_v7().to_string();
        self.server_panel.linear.request = Some(ticket.clone());
        self.server_panel.linear.fetched = Some(Instant::now());
        let connecting = matches!(action, Action::Connect { .. });
        cx.spawn(async move |this, cx| {
            let task_host = host.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let path = "/v1/integrations/linear";
                    let result = match action {
                        Action::Load => server_panel::request::<Inbox>(
                            &task_host,
                            &token,
                            "GET",
                            path,
                            None::<&()>,
                        ),
                        Action::Sync => server_panel::request::<Inbox>(
                            &task_host,
                            &token,
                            "POST",
                            "/v1/integrations/linear/sync",
                            None::<&()>,
                        ),
                        Action::Disconnect => server_panel::request::<Inbox>(
                            &task_host,
                            &token,
                            "DELETE",
                            path,
                            None::<&()>,
                        ),
                        Action::Connect {
                            filter,
                            token: linear_token,
                        } => {
                            #[derive(Serialize)]
                            struct Connect<'a> {
                                filter: Filter,
                                token: &'a str,
                            }
                            server_panel::request::<Inbox>(
                                &task_host,
                                &token,
                                "PUT",
                                path,
                                Some(&Connect {
                                    filter,
                                    token: &linear_token,
                                }),
                            )
                        }
                    };
                    result.and_then(|inbox| {
                        inbox.validate().map_err(|_| {
                            "Invalid Linear inbox from relay; update the relay".to_string()
                        })?;
                        Ok(inbox)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&host.id)
                    || this.server_panel.linear.request.as_deref() != Some(&ticket)
                    || !this.remote_hosts.iter().any(|current| current == &host)
                {
                    return;
                }
                let state = &mut this.server_panel.linear;
                state.request = None;
                match result {
                    Ok(inbox) => {
                        state.inbox = Some(inbox);
                        state.error = None;
                        if connecting {
                            state.form_open = false;
                            state.token = None;
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let state = &this.server_panel.linear;
    let busy = state.request.is_some();
    let configured = state
        .inbox
        .as_ref()
        .is_some_and(|inbox| inbox.filter.is_some());
    let mut body=div().flex().flex_col().gap(px(10.)).py(px(12.))
        .border_t_1().border_b_1().border_color(p.border)
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Linear · server MCP integration"))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Your token stays in this relay's keyring, not in agent context. The relay refreshes tagged tasks every minute, even with this app closed."))
        .child(div().flex().flex_wrap().items_center().gap(px(8.))
            .child(dunlin_ui::settings::button("linear-configure").relative().children(crate::gui_probe::marker("linear-configure")).label(if state.form_open {"Cancel configuration"} else if configured {"Reconnect / change filter…"} else {"Connect Linear…"})
                .disabled(busy).on_click(cx.listener(|this,_,window,cx|this.edit_linear(window,cx))))
            .when(configured,|row|row
                .child(dunlin_ui::settings::button("linear-sync").label(if busy {"Working…"} else {"Sync now"}).disabled(busy)
                    .on_click(cx.listener(|this,_,_,cx|this.linear_action(Action::Sync,cx))))
                .child(dunlin_ui::settings::button("linear-disconnect").label("Disconnect").disabled(busy)
                    .on_click(cx.listener(|this,_,_,cx|this.linear_action(Action::Disconnect,cx))))));
    if state.form_open {
        if let (Some(token), Some(label), Some(team)) = (&state.token, &state.label, &state.team) {
            body = body
                .child(
                    dunlin_ui::input(token)
                        .aria_label("Linear API key or access token")
                        .disabled(busy),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            dunlin_ui::input(label)
                                .aria_label("Linear issue label")
                                .disabled(busy),
                        )
                        .child(
                            dunlin_ui::input(team)
                                .aria_label("Optional Linear team")
                                .disabled(busy),
                        ),
                )
                .child(
                    dunlin_ui::settings::button("linear-save")
                        .relative()
                        .children(crate::gui_probe::marker("linear-save"))
                        .primary()
                        .label("Connect and save token on server")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.save_linear(window, cx))),
                );
        }
    }
    if let Some(error) = state
        .error
        .as_ref()
        .or_else(|| state.inbox.as_ref().and_then(|inbox| inbox.error.as_ref()))
    {
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(p.amber)
                .child(error.clone()),
        );
    }
    if let Some(inbox) = &state.inbox {
        if let Some(filter) = &inbox.filter {
            let time = inbox
                .synced_at
                .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "not synchronized".into());
            let stale = inbox.error.is_some()
                || inbox
                    .synced_at
                    .is_none_or(|time| time < chrono::Utc::now().timestamp() - 120);
            body = body.child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "Label: {} · {} tasks · Last successful sync: {time}{}",
                        filter.label,
                        inbox.issues.len(),
                        if stale { " · stale snapshot" } else { "" }
                    )),
            );
            if inbox.issues.is_empty() {
                body = body.child(div().text_size(px(12.)).child("No tasks match this label."));
            } else {
                let mut list = div()
                    .id("linear-inbox")
                    .h(px(260.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.));
                for issue in &inbox.issues {
                    let assignment = issue.clone();
                    let url = issue.url.clone();
                    list = list.child(
                        div()
                            .relative()
                            .children(crate::gui_probe::marker_named(format!(
                                "linear-issue:{}",
                                issue.id
                            )))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .py(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div().text_size(px(11.)).text_color(p.muted_fg).child(
                                            format!("{} · {}", issue.identifier, issue.status),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .truncate()
                                            .child(issue.title.clone()),
                                    ),
                            )
                            .child(
                                dunlin_ui::settings::button(SharedString::from(format!(
                                    "linear-assign:{}",
                                    issue.id
                                )))
                                .relative()
                                .children(crate::gui_probe::marker_named(format!(
                                    "linear-assign:{}",
                                    issue.id
                                )))
                                .label("Assign to host…")
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.assign_linear_job(assignment.clone(), window, cx)
                                    },
                                )),
                            )
                            .child(
                                dunlin_ui::settings::button(SharedString::from(format!(
                                    "linear-open:{}",
                                    issue.id
                                )))
                                .label("Open in Linear")
                                .disabled(!is_issue_url(&url))
                                .on_click(move |_, _, cx| {
                                    if is_issue_url(&url) {
                                        cx.open_url(&url);
                                    }
                                }),
                            ),
                    );
                }
                body = body.child(list);
            }
        }
    }
    body.into_any_element()
}
