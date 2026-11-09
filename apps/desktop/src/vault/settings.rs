//! Settings → Credentials. Values are accepted once and moved to the Keychain.

use dunlin_domain::vault::now_secs;
use dunlin_domain::vault::{HttpMethod, Injection, Operation, Recipe, RecipeParameter, Secret};
use dunlin_ui::component::Disableable;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::{Editor, EditorState, InputState};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::DunlinApp;
use crate::vault::{NewSecret, TTL_OPTIONS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InjectionKind {
    Header,
    OAuth2,
    AwsSigV4,
}

pub(crate) struct Draft {
    pub advanced: bool,
    pub secret_id: Option<String>,
    pub label: Entity<InputState>,
    pub description: Entity<InputState>,
    pub value: Entity<InputState>,
    pub hosts: Entity<InputState>,
    pub path_prefix: Entity<InputState>,
    pub header_name: Entity<InputState>,
    pub header_prefix: Entity<InputState>,
    pub injection_kind: InjectionKind,
    pub oauth_token_url: Entity<InputState>,
    pub oauth_client_id: Entity<InputState>,
    pub aws_access_key_id: Entity<InputState>,
    pub aws_region: Entity<InputState>,
    pub aws_service: Entity<InputState>,
    pub headers: Entity<InputState>,
    pub ttl_secs: i64,
    pub allow_writes: bool,
    pub require_approval: bool,
    pub project_ids: Vec<String>,
}

impl Draft {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<DunlinApp>,
        secret: Option<&Secret>,
        project_ids: Vec<String>,
    ) -> Self {
        let label =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. GitHub (work)"));
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("When should the agent use this? This text is visible to the agent.")
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if secret.is_some() {
                    "Leave blank to keep the current value"
                } else {
                    "Paste the credential value"
                })
                .masked(true)
        });
        let hosts = cx.new(|cx| {
            InputState::new(window, cx).placeholder("api.github.com (one or more, comma-separated)")
        });
        let path_prefix = cx.new(|cx| InputState::new(window, cx).placeholder("/"));
        let header_name = cx.new(|cx| InputState::new(window, cx).placeholder("Authorization"));
        let header_prefix = cx.new(|cx| InputState::new(window, cx).placeholder("Bearer "));
        header_name.update(cx, |input, cx| input.set_value("Authorization", window, cx));
        header_prefix.update(cx, |input, cx| input.set_value("Bearer ", window, cx));
        let oauth_token_url = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://auth.example.com/oauth/token")
        });
        let oauth_client_id =
            cx.new(|cx| InputState::new(window, cx).placeholder("OAuth client ID"));
        let aws_access_key_id = cx.new(|cx| InputState::new(window, cx).placeholder("AKIA…"));
        let aws_region = cx.new(|cx| InputState::new(window, cx).placeholder("us-east-1"));
        let aws_service = cx.new(|cx| InputState::new(window, cx).placeholder("execute-api or s3"));
        let mut injection_kind = InjectionKind::Header;
        let headers = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Accept, Content-Type (extra allowed request headers)")
        });
        headers.update(cx, |input, cx| {
            input.set_value(
                secret
                    .map(|secret| secret.allowed_headers.join(", "))
                    .unwrap_or_else(|| "Accept, Content-Type".into()),
                window,
                cx,
            )
        });
        if let Some(secret) = secret {
            label.update(cx, |input, cx| {
                input.set_value(secret.label.clone(), window, cx)
            });
            description.update(cx, |input, cx| {
                input.set_value(secret.description.clone(), window, cx)
            });
            hosts.update(cx, |input, cx| {
                input.set_value(secret.allowed_hosts.join(", "), window, cx)
            });
            path_prefix.update(cx, |input, cx| {
                input.set_value(secret.path_prefix.clone(), window, cx)
            });
            match &secret.injection {
                Injection::Header { name, prefix } => {
                    header_name.update(cx, |input, cx| input.set_value(name.clone(), window, cx));
                    header_prefix
                        .update(cx, |input, cx| input.set_value(prefix.clone(), window, cx));
                }
                Injection::OAuth2 {
                    token_url,
                    client_id,
                } => {
                    injection_kind = InjectionKind::OAuth2;
                    oauth_token_url.update(cx, |input, cx| {
                        input.set_value(token_url.clone(), window, cx)
                    });
                    oauth_client_id.update(cx, |input, cx| {
                        input.set_value(client_id.clone(), window, cx)
                    });
                }
                Injection::AwsSigV4 {
                    access_key_id,
                    region,
                    service,
                } => {
                    injection_kind = InjectionKind::AwsSigV4;
                    aws_access_key_id.update(cx, |input, cx| {
                        input.set_value(access_key_id.clone(), window, cx)
                    });
                    aws_region.update(cx, |input, cx| input.set_value(region.clone(), window, cx));
                    aws_service
                        .update(cx, |input, cx| input.set_value(service.clone(), window, cx));
                }
                _ => {}
            }
        }
        Self {
            advanced: secret.is_some_and(|secret| !secret.allowed_hosts.is_empty()),
            secret_id: secret.map(|secret| secret.id.clone()),
            label,
            description,
            value,
            hosts,
            path_prefix,
            header_name,
            header_prefix,
            injection_kind,
            oauth_token_url,
            oauth_client_id,
            aws_access_key_id,
            aws_region,
            aws_service,
            headers,
            ttl_secs: secret
                .and_then(|secret| secret.expires_at)
                .map(|expires_at| (expires_at - now_secs()).clamp(15 * 60, 24 * 60 * 60))
                .unwrap_or(3600),
            allow_writes: secret.is_some_and(|secret| {
                secret
                    .allowed_methods
                    .iter()
                    .any(|method| method.is_write())
            }),
            require_approval: secret.is_some_and(|secret| !secret.auto_approve),
            project_ids,
        }
    }

    fn read(&self, cx: &App) -> NewSecret {
        let hosts = self
            .hosts
            .read(cx)
            .value()
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        NewSecret {
            storage_only: !self.advanced,
            label: self.label.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
            value: self.value.read(cx).value().to_string(),
            hosts,
            path_prefix: if self.advanced {
                self.path_prefix.read(cx).value().to_string()
            } else {
                "/".into()
            },
            injection: match self.injection_kind {
                InjectionKind::Header => Injection::Header {
                    name: self.header_name.read(cx).value().to_string(),
                    prefix: self.header_prefix.read(cx).value().to_string(),
                },
                InjectionKind::OAuth2 => Injection::OAuth2 {
                    token_url: self.oauth_token_url.read(cx).value().trim().to_string(),
                    client_id: self.oauth_client_id.read(cx).value().trim().to_string(),
                },
                InjectionKind::AwsSigV4 => Injection::AwsSigV4 {
                    access_key_id: self.aws_access_key_id.read(cx).value().trim().to_string(),
                    region: self.aws_region.read(cx).value().trim().to_string(),
                    service: self.aws_service.read(cx).value().trim().to_string(),
                },
            },
            allowed_headers: self
                .headers
                .read(cx)
                .value()
                .split(|character: char| character == ',' || character.is_whitespace())
                .filter(|header| !header.is_empty())
                .map(str::to_string)
                .collect(),
            ttl_secs: self.ttl_secs,
            allow_writes: self.allow_writes,
            auto_approve: !self.require_approval,
            project_ids: self.project_ids.clone(),
        }
    }
}

pub(crate) struct OperationDraft {
    pub secret_id: String,
    pub secret_label: String,
    pub name: Entity<InputState>,
    pub description: Entity<InputState>,
    pub host: Entity<InputState>,
    pub path: Entity<InputState>,
    pub parameters: Entity<InputState>,
    pub method: HttpMethod,
}

impl OperationDraft {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<DunlinApp>, secret: &Secret) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("issues-list"));
        let description = cx
            .new(|cx| InputState::new(window, cx).placeholder("What does this operation return?"));
        let host = cx.new(|cx| InputState::new(window, cx).placeholder("api.github.com"));
        let path =
            cx.new(|cx| InputState::new(window, cx).placeholder("/repos/{owner}/{repo}/issues"));
        let parameters =
            cx.new(|cx| InputState::new(window, cx).placeholder("owner, repo (comma-separated)"));
        host.update(cx, |input, cx| {
            input.set_value(
                secret.allowed_hosts.first().cloned().unwrap_or_default(),
                window,
                cx,
            )
        });
        Self {
            secret_id: secret.id.clone(),
            secret_label: secret.label.clone(),
            name,
            description,
            host,
            path,
            parameters,
            method: HttpMethod::Get,
        }
    }

    fn read(&self, cx: &App) -> Operation {
        Operation {
            name: self.name.read(cx).value().trim().to_string(),
            description: self.description.read(cx).value().trim().to_string(),
            method: self.method,
            host: self.host.read(cx).value().trim().to_string(),
            path: self.path.read(cx).value().trim().to_string(),
            parameters: self
                .parameters
                .read(cx)
                .value()
                .split(|character: char| character == ',' || character.is_whitespace())
                .filter(|parameter| !parameter.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

pub(crate) struct RecipeDraft {
    pub secret_id: String,
    pub secret_label: String,
    pub name: Entity<InputState>,
    pub description: Entity<InputState>,
    pub program: Entity<InputState>,
    pub args: Entity<EditorState>,
    pub env_var: Entity<InputState>,
    pub parameters: Entity<EditorState>,
    pub timeout: Entity<InputState>,
}

impl RecipeDraft {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<DunlinApp>, secret: &Secret) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("gh-api"));
        let description =
            cx.new(|cx| InputState::new(window, cx).placeholder("What does this recipe do?"));
        let program = cx.new(|cx| {
            InputState::new(window, cx).placeholder("/opt/homebrew/bin/gh (absolute path)")
        });
        let args = cx.new(|cx| EditorState::new(window, cx).language("text"));
        let env_var = cx.new(|cx| InputState::new(window, cx).placeholder("GH_TOKEN"));
        let parameters = cx.new(|cx| EditorState::new(window, cx).language("text"));
        let timeout = cx.new(|cx| InputState::new(window, cx).placeholder("30"));
        env_var.update(cx, |input, cx| input.set_value("DUNLIN_SECRET", window, cx));
        timeout.update(cx, |input, cx| input.set_value("30", window, cx));
        Self {
            secret_id: secret.id.clone(),
            secret_label: secret.label.clone(),
            name,
            description,
            program,
            args,
            env_var,
            parameters,
            timeout,
        }
    }

    fn read(&self, cx: &App) -> Recipe {
        let args = self
            .args
            .read(cx)
            .value()
            .lines()
            .map(str::to_string)
            .filter(|line| !line.is_empty())
            .collect();
        let parameters = self
            .parameters
            .read(cx)
            .value()
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let (name, max_len) = line
                    .split_once(':')
                    .map(|(name, max)| (name.trim(), max.trim().parse().unwrap_or(512)))
                    .unwrap_or((line, 512));
                Some(RecipeParameter {
                    name: name.to_string(),
                    required: true,
                    max_len: max_len.clamp(1, 4096),
                    allow_leading_dash: false,
                })
            })
            .collect();
        Recipe {
            id: format!("recipe-{}", uuid::Uuid::now_v7().simple()),
            secret_id: self.secret_id.clone(),
            name: self.name.read(cx).value().trim().to_string(),
            description: self.description.read(cx).value().trim().to_string(),
            program: self.program.read(cx).value().trim().to_string(),
            args,
            env_var: self.env_var.read(cx).value().trim().to_string(),
            parameters,
            timeout_secs: self.timeout.read(cx).value().trim().parse().unwrap_or(30),
            program_hash: None,
            created_at: now_secs(),
        }
    }
}

fn chip(id: String, label: impl Into<SharedString>, active: bool, _p: Palette) -> Button {
    segment_button(id, label, active)
}

fn section_label(text: &'static str, p: Palette) -> Div {
    dunlin_ui::settings::section(text, p)
}

pub(super) fn secret_field(label: &'static str, state: &Entity<InputState>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .min_w(px(0.))
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe(label)
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(section_label(label, p))
        .child(
            div()
                .relative()
                .w_full()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe(match label {
                            "Name" => "vault-name-input",
                            "Key" => "vault-key-input",
                            _ => "vault-other-input",
                        })
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .child(
                    dunlin_ui::settings::input(state, p)
                        .aria_label(label)
                        .w_full(),
                ),
        )
}

fn ttl_label(expires_at: Option<i64>) -> String {
    let Some(expires_at) = expires_at else {
        return "no expiry".into();
    };
    let remaining = expires_at - now_secs();
    if remaining <= 0 {
        return "expired".into();
    }
    if remaining >= 3600 {
        format!("{} h left", (remaining / 3600).max(1))
    } else {
        format!("{} min left", (remaining / 60).max(1))
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let mut body = dunlin_ui::settings::page()
        .child(dunlin_ui::settings::section("Credential broker · this Mac", p))
        .child(dunlin_ui::settings::card(p).child(dunlin_ui::settings::row("Agent credential access",
            "Allow scoped use of configured keys without exposing their values to agents. Stored-only keys remain inaccessible to agents.",
            dunlin_ui::switch("vault-toggle", "Agent credential access", this.vault.enabled)
                .on_click(cx.listener(|this, enabled: &bool, _, cx| this.set_vault_enabled(*enabled, cx)))
                .into_any_element(), p)))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child("Keys stay in the macOS Keychain, never in model context or project files. Configure agent access only when needed."),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(format!("Status: {}", this.vault.status())),
                )
                .child(div().flex_1())
                .child(
                    dunlin_ui::settings::button("vault-refresh")
                        .relative().children({
                            #[cfg(debug_assertions)]
                            { crate::workspace_ui_smoke::probe("vault-refresh") }
                            #[cfg(not(debug_assertions))]
                            { None::<AnyElement> }
                        })
                        .label(if this.vault.ui.busy { "Refreshing…" } else { "Refresh" })
                        .loading(this.vault.ui.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_vault(cx))),
                )
                .child(
                    dunlin_ui::settings::button("vault-revoke-all")
                        .danger().outline()
                        .disabled(this.vault.ui.busy || this.vault.secrets.is_empty())
                        .label("Revoke all")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let answer = window.prompt(
                                PromptLevel::Warning,
                                "Delete all stored credentials?",
                                Some("Deletes every credential value and policy from the macOS Keychain. Active and parked harnesses lose vault access until restarted. Audit history is retained."),
                                &["Cancel", "Revoke all"],
                                cx,
                            );
                            let answer = this.overlay_backdrop.track_native(answer, cx);
                            cx.spawn(async move |this, cx| {
                                if answer.await == Ok(1) {
                                    let _ = this.update(cx, |this, cx| {
                                        if let Err(error) = this.revoke_all_vault_secrets(cx) {
                                            this.vault.ui.message = Some(error);
                                            cx.notify();
                                        }
                                    });
                                }
                            }).detach();
                        })),
                )
                .when(this.vault_needs_harness_restart(), |view| {
                    view.child(
                        dunlin_ui::settings::button("vault-restart-harness")
                            .label("Restart harness to attach")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let answer = window.prompt(
                                    PromptLevel::Warning,
                                    "Restart the harness?",
                                    Some("This cancels the current turn and reconnects the ACP harness so it can receive the vault MCP server. Parked agents keep their existing connection until separately restarted."),
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
                    )
                }),
        );
    if let Some(error) = &this.vault.error {
        body = body.child(
            div()
                .rounded(px(dunlin_ui::radii::CONTROL))
                .p(px(10.))
                .bg(p.red_soft)
                .text_color(p.red)
                .text_size(px(11.5))
                .child(error.clone()),
        );
    }
    if let Some(message) = &this.vault.ui.message {
        body = body.child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child(message.clone()),
        );
    }
    if let Some(operation_draft) = &this.vault.ui.operation_draft {
        return body
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(format!(
                "New HTTP operation for {}",
                operation_draft.secret_label
            )))
            .child(secret_field("Operation name", &operation_draft.name, p))
            .child(secret_field("Operation description", &operation_draft.description, p))
            .child(secret_field("Allowed host", &operation_draft.host, p))
            .child(secret_field("Path template", &operation_draft.path, p))
            .child(secret_field("Parameters", &operation_draft.parameters, p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(5.))
                    .child(section_label("Method", p))
                    .children(HttpMethod::ALL.into_iter().map(|method| {
                        chip(
                            format!("vault-operation-method-{}", method.as_str()),
                            method.as_str(),
                            operation_draft.method == method,
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(draft) = &mut this.vault.ui.operation_draft {
                                draft.method = method;
                            }
                            cx.notify();
                        }))
                        .into_any_element()
                    })),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child("The host must be in the credential allowlist. Path placeholders use {name}; values are percent-encoded. Benefits: the model cannot choose a different host, method or path template. Body requests can still use the free-form URL."),
            )
            .child(
                div().flex().flex_wrap().justify_end().gap(px(8.)).child(
                    dunlin_ui::settings::button("save-vault-operation")
                        .primary()
                        .label("Save operation")
                        .on_click(cx.listener(|this, _, _, cx| this.save_vault_operation(cx))),
                ).child(
                    dunlin_ui::settings::button("cancel-vault-operation")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.vault.ui.operation_draft = None;
                            cx.notify();
                        })),
                ),
            )
            .into_any_element();
    }
    if let Some(recipe_draft) = &this.vault.ui.recipe_draft {
        return body
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(format!(
                "New CLI recipe for {}",
                recipe_draft.secret_label
            )))
            .child(secret_field("Recipe name", &recipe_draft.name, p))
            .child(secret_field("Recipe description", &recipe_draft.description, p))
            .child(secret_field("Executable path", &recipe_draft.program, p))
            .child(div().text_size(px(10.5)).text_color(p.muted_fg).child("Arguments (one per line; {name} placeholders)"))
            .child(div().h(px(110.)).overflow_hidden().child(Editor::new(&recipe_draft.args).aria_label("Recipe arguments, one per line").h_full()))
            .child(secret_field("Credential environment variable", &recipe_draft.env_var, p))
            .child(secret_field("Timeout in seconds", &recipe_draft.timeout, p))
            .child(div().text_size(px(10.5)).text_color(p.muted_fg).child("Parameters (one per line: name or name:max_length)"))
            .child(div().h(px(90.)).overflow_hidden().child(Editor::new(&recipe_draft.parameters).aria_label("Recipe parameter definitions").h_full()))
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child("Arguments are executed directly, without a shell. Programs inside the agent's project are refused. Prefer a fixed binary outside the project."),
            )
            .child(
                div().flex().flex_wrap().justify_end().gap(px(8.)).child(
                    dunlin_ui::settings::button("save-vault-recipe")
                        .primary()
                        .label("Save recipe")
                        .on_click(cx.listener(|this, _, _, cx| this.save_vault_recipe(cx))),
                ).child(
                    dunlin_ui::settings::button("cancel-vault-recipe")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.vault.ui.recipe_draft = None;
                            cx.notify();
                        })),
                ),
            )
            .into_any_element();
    }
    if let Some(draft) = &this.vault.ui.draft {
        let editing = draft.secret_id.is_some();
        let form = dunlin_ui::settings::card(p).flex().flex_col().gap(px(14.)).p(px(14.))
            .relative().children({
                #[cfg(debug_assertions)] { crate::workspace_ui_smoke::probe("vault-secret-form") }
                #[cfg(not(debug_assertions))] { None::<AnyElement> }
            })
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if editing {
                        "Edit credential"
                    } else {
                        "New credential"
                    }),
            )
            .child(secret_field("Name", &draft.label, p))
            .child(secret_field("Key", &draft.value, p))
            .child(dunlin_ui::settings::button("vault-advanced")
                .self_end()
                .label(if draft.advanced { "Store only · remove agent access" } else { "Configure agent access…" })
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(draft) = &mut this.vault.ui.draft { draft.advanced = !draft.advanced; }
                    cx.notify();
                })))
            .when(!draft.advanced, |body| body.child(div().text_size(px(11.)).text_color(p.muted_fg)
                .child("Stored without expiry or agent access. You can configure access later.")))
            .when(draft.advanced, |body| body
            .child(secret_field("Allowed API hosts", &draft.hosts, p))
            .child(secret_field("Description for the agent", &draft.description, p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(5.))
                    .child(section_label("Injection", p))
                    .children(
                        [
                            (InjectionKind::Header, "Header"),
                            (InjectionKind::OAuth2, "OAuth 2.0 refresh"),
                            (InjectionKind::AwsSigV4, "AWS SigV4"),
                        ]
                        .into_iter()
                        .map(|(kind, label)| {
                            chip(
                                format!("vault-injection-{kind:?}"),
                                label,
                                draft.injection_kind == kind,
                                p,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(draft) = &mut this.vault.ui.draft {
                                    draft.injection_kind = kind;
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                        }),
                    ),
            )
            .when(draft.injection_kind == InjectionKind::Header, |view| {
                view.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(secret_field("Header name", &draft.header_name, p).flex_1())
                        .child(secret_field("Header prefix", &draft.header_prefix, p).flex_1()),
                )
            })
            .when(draft.injection_kind == InjectionKind::OAuth2, |view| {
                view.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(secret_field("Token URL", &draft.oauth_token_url, p).flex_1())
                        .child(secret_field("Client ID", &draft.oauth_client_id, p).flex_1()),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .child("The stored value is the refresh token. Public clients only: the broker exchanges it for a short-lived access token and never returns either token to the model."),
                )
            })
            .when(draft.injection_kind == InjectionKind::AwsSigV4, |view| {
                view.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(secret_field("Access key ID", &draft.aws_access_key_id, p).flex_1())
                        .child(secret_field("Region", &draft.aws_region, p).flex_1())
                        .child(secret_field("Service", &draft.aws_service, p).flex_1()),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .child("The stored value is the AWS secret access key. The broker signs each allowed request with SigV4; the secret never reaches the model."),
                )
            })
            .child(secret_field("Allowed path prefix", &draft.path_prefix, p))
            .child(secret_field("Allowed request headers", &draft.headers, p))
            .child(dunlin_ui::settings::row("Require approval for each use",
                "Ask before each allowed credential operation. Host, method, path and project restrictions still apply when this is off.",
                dunlin_ui::switch("vault-require-approval", "Require approval for each credential use", draft.require_approval)
                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                        if let Some(draft) = &mut this.vault.ui.draft { draft.require_approval = *enabled; }
                        cx.notify();
                    })).into_any_element(), p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(5.))
                    .child(section_label("Allowed methods", p))
                    .child(
                        chip(
                            "vault-method-read".into(),
                            "GET/HEAD",
                            !draft.allow_writes,
                            p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(draft) = &mut this.vault.ui.draft {
                                draft.allow_writes = false;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        chip(
                            "vault-method-write".into(),
                            "also writes",
                            draft.allow_writes,
                            p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(draft) = &mut this.vault.ui.draft {
                                draft.allow_writes = true;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(5.))
                    .child(section_label("Time to live", p))
                    .children(TTL_OPTIONS.into_iter().map(|(ttl, label)| {
                        chip(format!("vault-ttl-{ttl}"), label, draft.ttl_secs == ttl, p)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(draft) = &mut this.vault.ui.draft {
                                    draft.ttl_secs = ttl;
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(5.))
                    .child(section_label("Projects", p))
                    .children(
                        this.projects
                            .iter()
                            .filter(|project| !project.is_remote())
                            .map(|project| {
                                let id = project.id.clone();
                                let active = draft.project_ids.contains(&id);
                                dunlin_ui::checkbox(
                                    format!("vault-project-{id}"),
                                    project.name.clone(),
                                    active,
                                ).label(project.name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(draft) = &mut this.vault.ui.draft {
                                        if let Some(index) = draft
                                            .project_ids
                                            .iter()
                                            .position(|project| project == &id)
                                        {
                                            draft.project_ids.remove(index);
                                        } else {
                                            draft.project_ids.push(id.clone());
                                        }
                                    }
                                    cx.notify();
                                }))
                                .into_any_element()
                            }),
                    ),
            )
            )
            .child(
                div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                    .child(
                        dunlin_ui::settings::button("save-vault-secret")
                            .primary()
                            .label("Save credential")
                            .on_click(cx.listener(|this, _, _, cx| this.save_vault_draft(cx))),
                    )
                    .child(
                        dunlin_ui::settings::button("cancel-vault-secret")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.vault.ui.draft = None;
                                cx.notify();
                            })),
                    ),
            );
        return body.child(form).into_any_element();
    }

    body = body.child(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .child(dunlin_ui::settings::section("Stored credentials", p)),
            )
            .child(
                dunlin_ui::settings::button("add-vault-secret")
                    .primary()
                    .label("Add credential")
                    .on_click(cx.listener(|this, _, window, cx| this.new_vault_draft(window, cx))),
            ),
    );
    if this.vault.secrets.is_empty() {
        body = body.child(div().text_size(px(11.5)).text_color(p.muted_fg).child(
            "No credentials yet. Add a quick key in the bottom Secrets panel or create one here.",
        ));
    }
    for secret in &this.vault.secrets {
        let secret = secret.clone();
        let id = secret.id.clone();
        let edit = secret.clone();
        let delete = secret.clone();
        let add_recipe = secret.clone();
        let add_operation = secret.clone();
        let projects = this
            .vault_granted_projects(&secret.id)
            .iter()
            .filter_map(|grant| {
                this.projects
                    .iter()
                    .find(|project| project.id == grant.project_id)
                    .map(|project| project.name.clone())
            })
            .collect::<Vec<_>>();
        let recipes = this
            .vault
            .recipes
            .get(&secret.id)
            .cloned()
            .unwrap_or_default();
        let operations = this
            .vault
            .operations
            .get(&secret.id)
            .cloned()
            .unwrap_or_default();
        let audit = this.vault_audit_for(&secret.id);
        let expired = secret.is_expired(now_secs());
        let details = this.settings_ui.credential_details.as_deref() == Some(&id);
        let details_id = id.clone();
        let mut card = dunlin_ui::settings::card(p)
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().min_w(px(0.)).truncate().font_weight(FontWeight::MEDIUM).child(secret.label.clone()))
                    .child(
                        div()
                            .font_family(dunlin_ui::theme::code_font())
                            .max_w(px(140.)).truncate()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(secret.alias.clone()),
                    )
                    .child(div().text_size(px(10.5)).text_color(if expired { p.red } else { p.muted_fg }).child(ttl_label(secret.expires_at)))
                    .child(dunlin_ui::settings::button(format!("edit-vault-{id}")).label("Edit").on_click(cx.listener(move |this, _, window, cx| {
                        let projects = this.vault_granted_projects(&edit.id).iter().map(|grant| grant.project_id.clone()).collect();
                        this.vault.ui.recipe_draft = None;
                        this.vault.ui.operation_draft = None;
                        this.vault.ui.draft = Some(Draft::new(window, cx, Some(&edit), projects));
                        cx.notify();
                    })))
                    .child(dunlin_ui::settings::button(format!("delete-vault-{id}")).danger().outline().label("Delete").on_click(cx.listener(move |this, _, window, cx| {
                        let answer = window.prompt(PromptLevel::Warning, "Delete credential?", Some("The value is removed from the macOS Keychain. This cannot be undone; rotate the key at the provider if needed."), &["Cancel", "Delete"], cx);
                        let answer = this.overlay_backdrop.track_native(answer, cx);
                        let id = delete.id.clone();
                        cx.spawn(async move |this, cx| {
                            if answer.await == Ok(1) {
                                let _ = this.update(cx, |this, cx| {
                                    if let Err(error) = this.delete_vault_secret(&id, cx) {
                                        this.vault.ui.message = Some(error);
                                        cx.notify();
                                    }
                                });
                            }
                        }).detach();
                    }))),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted_fg)
                    .child(if secret.description.is_empty() {
                        "No agent description".into()
                    } else {
                        secret.description.clone()
                    }),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(if secret.allowed_hosts.is_empty() { "Stored only · configure agent access to use this key".into() } else { format!(
                        "{} · {} · {}{}",
                        secret.allowed_hosts.join(", "),
                        secret
                            .allowed_methods
                            .iter()
                            .map(|method| method.as_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                        secret.injection.label(),
                        if secret.auto_approve {
                            " · auto-approved"
                        } else {
                            " · requires approval"
                        }
                    ) }),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "Projects: {}",
                        if projects.is_empty() {
                            "none (inert)".into()
                        } else {
                            projects.join(", ")
                        }
                    )),
            )
            .child(div().flex().justify_end().child(dunlin_ui::settings::button(format!("vault-details-{id}"))
                .label(if details { "Hide policies and history" } else { "Access policies & history…" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_ui.credential_details = if details { None } else { Some(details_id.clone()) };
                    cx.notify();
                }))));
        if !details {
            body = body.child(card);
            continue;
        }
        card = card.child(dunlin_ui::settings::section("CLI recipes", p));
        let mut recipe_row = div().flex().flex_col().gap(px(4.));
        for recipe in &recipes {
            let recipe_id = recipe.id.clone();
            let recipe_secret_id = secret.id.clone();
            recipe_row = recipe_row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "CLI · {} · {} {}",
                                recipe.name,
                                recipe.program,
                                recipe.args.join(" ")
                            )),
                    )
                    .child(
                        dunlin_ui::settings::button(format!("remove-recipe-{recipe_id}"))
                            .danger()
                            .outline()
                            .label("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(error) =
                                    this.remove_vault_recipe(&recipe_secret_id, &recipe_id, cx)
                                {
                                    this.vault.ui.message = Some(error);
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        recipe_row = recipe_row.child(
            div().flex().justify_end().child(
                dunlin_ui::settings::button(format!("add-recipe-{id}"))
                    .label("Add CLI recipe")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault.ui.draft = None;
                        this.vault.ui.operation_draft = None;
                        this.vault.ui.recipe_draft =
                            Some(RecipeDraft::new(window, cx, &add_recipe));
                        cx.notify();
                    })),
            ),
        );
        card = card.child(recipe_row);
        let mut operation_row = div().flex().flex_col().gap(px(4.));
        for operation in &operations {
            let operation_name = operation.name.clone();
            let operation_secret_id = secret.id.clone();
            operation_row = operation_row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "HTTP · {} {}{}{}",
                                operation.method.as_str(),
                                operation.host,
                                operation.path,
                                if operation.parameters.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", operation.parameters.join(", "))
                                }
                            )),
                    )
                    .child(
                        dunlin_ui::settings::button(format!("remove-operation-{operation_name}"))
                            .danger()
                            .outline()
                            .label("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(error) = this.remove_vault_operation(
                                    &operation_secret_id,
                                    &operation_name,
                                    cx,
                                ) {
                                    this.vault.ui.message = Some(error);
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        operation_row = operation_row.child(
            div().flex().justify_end().child(
                dunlin_ui::settings::button(format!("add-operation-{}", secret.id))
                    .label("Add HTTP operation")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault.ui.draft = None;
                        this.vault.ui.recipe_draft = None;
                        this.vault.ui.operation_draft =
                            Some(OperationDraft::new(window, cx, &add_operation));
                        cx.notify();
                    })),
            ),
        );
        card = card
            .child(dunlin_ui::settings::section("HTTP operations", p))
            .child(operation_row);
        if !audit.is_empty() {
            let text = audit
                .iter()
                .take(5)
                .map(|event| {
                    format!(
                        "{} · {} · {} · {}",
                        event.detail,
                        event.decision,
                        event.status.unwrap_or_default(),
                        event.error.as_deref().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            card = card.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted_fg)
                    .child(format!("Recent activity:\n{text}")),
            );
        }
        body = body.child(card);
    }
    body.into_any_element()
}

impl DunlinApp {
    pub(crate) fn new_vault_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project().id.clone();
        let project_ids = if self.project().is_remote() || !self.project().trusted {
            Vec::new()
        } else {
            vec![project]
        };
        self.vault.ui.recipe_draft = None;
        self.vault.ui.operation_draft = None;
        self.vault.ui.draft = Some(Draft::new(window, cx, None, project_ids));
        self.vault.ui.message = None;
        cx.notify();
    }

    fn save_vault_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.vault.ui.draft.take() else {
            return;
        };
        let input = draft.read(cx);
        let value = input.value.clone();
        let result = if let Some(id) = &draft.secret_id {
            self.update_vault_secret(id, input, Some(value), cx)
        } else {
            self.create_vault_secret(input, cx).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.vault.ui.message = Some("Credential saved.".into());
            }
            Err(error) => {
                self.vault.ui.draft = Some(draft);
                self.vault.ui.message = Some(error);
            }
        }
        cx.notify();
    }

    fn save_vault_operation(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.vault.ui.operation_draft.take() else {
            return;
        };
        let operation = draft.read(cx);
        if let Err(error) = self.add_vault_operation(&draft.secret_id, operation, cx) {
            self.vault.ui.operation_draft = Some(draft);
            self.vault.ui.message = Some(error);
        }
        cx.notify();
    }

    fn save_vault_recipe(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.vault.ui.recipe_draft.take() else {
            return;
        };
        let recipe = draft.read(cx);
        if let Err(error) = self.add_vault_recipe(&draft.secret_id, recipe, cx) {
            self.vault.ui.recipe_draft = Some(draft);
            self.vault.ui.message = Some(error);
        }
        cx.notify();
    }
}
