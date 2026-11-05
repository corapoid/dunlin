//! Bottom-panel quick credentials. Optimized for short-lived keys.

use dunlin_domain::vault::now_secs;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::input::{Input, InputState};
use dunlin_ui::*;

use crate::app::DunlinApp;
use crate::vault::{NewSecret, TTL_OPTIONS};

#[derive(Default)]
pub(crate) struct State {
    pub label: Option<Entity<InputState>>,
    pub value: Option<Entity<InputState>>,
    pub host: Option<Entity<InputState>>,
    pub ttl_secs: i64,
    pub allow_writes: bool,
    pub message: Option<String>,
}

impl State {
    fn ensure(&mut self, window: &mut Window, cx: &mut Context<DunlinApp>) {
        if self.ttl_secs == 0 {
            self.ttl_secs = 3600;
        }
        if self.label.is_none() {
            self.label =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. GitHub")));
        }
        if self.value.is_none() {
            self.value = Some(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Paste key")
                    .masked(true)
            }));
        }
        if self.host.is_none() {
            self.host =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("api.github.com")));
        }
    }
}

fn pill(id: String, label: impl Into<SharedString>, active: bool, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px(px(7.))
        .rounded(px(5.))
        .border_1()
        .border_color(if active { p.border_strong } else { p.border })
        .bg(if active { p.card } else { p.bg })
        .text_size(px(10.))
        .text_color(if active { p.fg } else { p.muted_fg })
        .flex()
        .items_center()
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .child(label.into())
}

fn remaining(expires_at: Option<i64>) -> String {
    let Some(expires_at) = expires_at else {
        return "no expiry".into();
    };
    let remaining = expires_at - now_secs();
    if remaining <= 0 {
        return "expired".into();
    }
    if remaining >= 3600 {
        format!("{} h", (remaining / 3600).max(1))
    } else {
        format!("{} min", (remaining / 60).max(1))
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let project = this.project().clone();
    let mut body = div()
        .id("vault-panel")
        .size_full()
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Quick credentials"),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child("Use-only: the agent sees the name, never the key."),
                )
                .child(div().flex_1())
                .child(
                    dunlin_ui::button("vault-open-settings")
                        .ghost()
                        .small()
                        .label("Manage in Settings…")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_tab = crate::app::SettingsTab::Secrets;
                            cx.notify();
                        })),
                ),
        );
    if !this.vault.requested_enabled {
        return body
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child("The credential broker is disabled in Settings → Secrets."),
            )
            .into_any_element();
    }
    if this.vault.service.is_none() {
        return body
            .child(
                div().text_size(px(11.)).text_color(p.red).child(
                    this.vault
                        .error
                        .clone()
                        .unwrap_or_else(|| "Credential broker unavailable".into()),
                ),
            )
            .into_any_element();
    }
    if project.is_remote() {
        return body
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child("Quick credentials are local-only in this build. Remote harnesses run on the relay and cannot reach this Mac's Keychain."),
            )
            .into_any_element();
    }
    if !project.trusted {
        return body
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child("Trust this project before creating credentials."),
            )
            .into_any_element();
    }
    if this.vault_needs_harness_restart() {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child("This harness started without vault access. Restart it to attach the credential MCP server."),
                )
                .child(
                    dunlin_ui::button("vault-panel-restart-harness")
                        .small()
                        .label("Restart harness")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let answer = window.prompt(
                                PromptLevel::Warning,
                                "Restart the harness?",
                                Some("This cancels the current turn and reconnects the ACP harness so it can receive the vault MCP server."),
                                &["Cancel", "Restart"],
                                cx,
                            );
                            let answer = this.overlay_backdrop.track_native(answer, cx);
                            cx.spawn(async move |this, cx| {
                                if answer.await == Ok(1) {
                                    let _ = this.update(cx, |this, cx| this.restart_harness(cx));
                                }
                            }).detach();
                        })),
                ),
        );
    }
    this.vault.ui.panel.ensure(window, cx);
    let panel = &this.vault.ui.panel;
    let label = panel.label.as_ref().unwrap();
    let value = panel.value.as_ref().unwrap();
    let host = panel.host.as_ref().unwrap();
    body = body
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().flex_1().min_w(px(0.)).child(Input::new(label)))
                .child(div().flex_1().min_w(px(0.)).child(Input::new(value)))
                .child(div().flex_1().min_w(px(0.)).child(Input::new(host))),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.))
                .child(div().text_size(px(9.5)).text_color(p.muted_fg).child("TTL"))
                .children(TTL_OPTIONS.into_iter().map(|(ttl, label)| {
                    pill(
                        format!("vault-panel-ttl-{ttl}"),
                        label,
                        this.vault.ui.panel.ttl_secs == ttl,
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.vault.ui.panel.ttl_secs = ttl;
                        cx.notify();
                    }))
                    .into_any_element()
                }))
                .child(
                    pill(
                        "vault-panel-writes".into(),
                        "allow writes",
                        this.vault.ui.panel.allow_writes,
                        p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.vault.ui.panel.allow_writes = !this.vault.ui.panel.allow_writes;
                        cx.notify();
                    })),
                )
                .child(
                    dunlin_ui::button("vault-panel-create")
                        .primary()
                        .small()
                        .label("Create quick key")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let panel = &this.vault.ui.panel;
                            let Some(label) = &panel.label else {
                                return;
                            };
                            let Some(value) = &panel.value else {
                                return;
                            };
                            let Some(host) = &panel.host else {
                                return;
                            };
                            let input = NewSecret {
                                label: label.read(cx).value().trim().to_string(),
                                description: "Quick credential created from the Secrets panel."
                                    .into(),
                                value: value.read(cx).value().to_string(),
                                hosts: host
                                    .read(cx)
                                    .value()
                                    .split(|character: char| {
                                        character == ',' || character.is_whitespace()
                                    })
                                    .filter(|host| !host.is_empty())
                                    .map(str::to_string)
                                    .collect(),
                                path_prefix: "/".into(),
                                injection: dunlin_domain::vault::Injection::Header {
                                    name: "Authorization".into(),
                                    prefix: "Bearer ".into(),
                                },
                                allowed_headers: vec!["Accept".into(), "Content-Type".into()],
                                ttl_secs: panel.ttl_secs,
                                allow_writes: panel.allow_writes,
                                auto_approve: true,
                                project_ids: vec![this.project().id.clone()],
                            };
                            match this.create_vault_secret(input, cx) {
                                Ok(alias) => {
                                    this.vault.ui.panel.message = Some(format!("Created {alias}."));
                                    if let Some(label) = &this.vault.ui.panel.label {
                                        label.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                    }
                                    if let Some(host) = &this.vault.ui.panel.host {
                                        host.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                    }
                                    // Drop the masked entity so the value is not kept in undo history.
                                    this.vault.ui.panel.value = None;
                                }
                                Err(error) => this.vault.ui.panel.message = Some(error),
                            }
                            cx.notify();
                        })),
                ),
        );
    body = body.child(
        div()
            .text_size(px(9.5))
            .text_color(p.muted_fg)
            .child("Quick keys use Authorization: Bearer by default. Use Settings → Secrets for x-api-key, Basic auth, path prefixes or custom headers."),
    );
    if let Some(message) = &this.vault.ui.panel.message {
        body = body.child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child(message.clone()),
        );
    }
    let secrets = this
        .vault_secrets_for_project(&project.id)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if secrets.is_empty() {
        body = body.child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("No active quick keys for this project."),
        );
    }
    for secret in secrets {
        let id = secret.id.clone();
        let label = secret.label.clone();
        let alias = secret.alias.clone();
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .p(px(6.))
                .rounded(px(6.))
                .bg(p.card)
                .border_1()
                .border_color(p.border)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(div().text_size(px(10.5)).font_weight(FontWeight::MEDIUM).child(label))
                        .child(
                            div()
                                .text_size(px(9.5))
                                .text_color(p.muted_fg)
                                .child(format!(
                                    "{} · {} · {}{}",
                                    alias,
                                    secret.allowed_hosts.join(", "),
                                    secret
                                        .allowed_methods
                                        .iter()
                                        .map(|method| method.as_str())
                                        .collect::<Vec<_>>()
                                        .join(" "),
                                    if secret.auto_approve { " · auto" } else { "" }
                                )),
                        ),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(if secret.is_expired(now_secs()) { p.red } else { p.muted_fg })
                        .child(remaining(secret.expires_at)),
                )
                .child(dunlin_ui::button(format!("vault-panel-revoke-{id}")).ghost().label("Revoke").on_click(cx.listener(move |this, _, window, cx| {
                    let answer = window.prompt(PromptLevel::Warning, "Revoke this quick key?", Some("The value is removed from the macOS Keychain immediately. The agent can no longer use it."), &["Cancel", "Revoke"], cx);
                    let answer = this.overlay_backdrop.track_native(answer, cx);
                    let id = id.clone();
                    cx.spawn(async move |this, cx| {
                        if answer.await == Ok(1) {
                            let _ = this.update(cx, |this, cx| {
                                if let Err(error) = this.delete_vault_secret(&id, cx) {
                                    this.vault.ui.panel.message = Some(error);
                                    cx.notify();
                                }
                            });
                        }
                    }).detach();
                }))),
        );
    }
    body.into_any_element()
}
