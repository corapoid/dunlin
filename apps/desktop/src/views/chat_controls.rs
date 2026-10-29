//! Inline, keyboard-accessible composer dropdowns. Only advertise features
//! actually supported by the current environment and ACP harness.
use dunlin_domain::PermissionMode;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::component::{Disableable, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};
use crate::harness::{command_available, harness_command};

#[derive(Clone)]
enum Action {
    Local,
    Remote(String),
    Settings(SettingsTab),
    Harness(String),
    Mode(String),
    Permission(PermissionMode),
    Model(String),
    Provider(String),
    ConnectProvider,
    RefreshProviders,
    Connect,
}

fn trigger(id: &'static str, label: String, icon: IconName) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .label(label)
        .dropdown_caret(true)
        .max_w(px(240.))
}

fn track_menu(cx: &Context<DunlinApp>) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let app = cx.entity().downgrade();
    move |open, _, cx| {
        let _ = app.update(cx, |this, cx| {
            this.chat_menu_open = *open;
            cx.notify();
        });
    }
}

fn item(
    app: &WeakEntity<DunlinApp>,
    text: (String, String),
    icon: IconName,
    selected: bool,
    disabled: bool,
    action: Action,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::element(move |_, cx| {
        let p = palette(cx);
        div()
            .min_w(px(240.))
            .max_w(px(350.))
            .py(px(4.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .opacity(if disabled { 0.5 } else { 1.0 })
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.fg)
                    .child(text.0.clone()),
            )
            .when(!text.1.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(px(11.))
                        .line_height(relative(1.4))
                        .text_color(p.muted_fg)
                        .child(text.1.clone()),
                )
            })
    })
    .icon(icon)
    .checked(selected)
    .disabled(disabled)
    .on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| {
            match &action {
                Action::Settings(tab) => {
                    this.settings_tab = *tab;
                    this.settings_open = true;
                    cx.notify();
                }
                _ if this.chat_controls_busy() => {
                    this.set_notice(
                        "Wait for the current operation before changing chat settings",
                        cx,
                    );
                }
                Action::Local => {
                    if this.project().is_remote() {
                        this.open_folder(cx);
                    }
                }
                Action::Remote(id) => {
                    if this
                        .project()
                        .remote
                        .as_ref()
                        .map(|project| &project.host_id)
                        != Some(id)
                    {
                        this.open_remote_picker_for(id, cx);
                    }
                }
                Action::Harness(name) => this.select_chat_harness(name, cx),
                Action::Mode(id) => this.select_mode(id, cx),
                Action::Permission(mode) => {
                    this.set_project_permission(this.selected_project, Some(*mode), cx)
                }
                Action::Model(id) => this.select_model(id, cx),
                Action::Provider(id) => this.select_provider(id, cx),
                Action::ConnectProvider => this.connect_provider(cx),
                Action::RefreshProviders => this.reload_provider_models(cx),
                Action::Connect => this.connect_harness(cx),
            }
            if matches!(
                action,
                Action::Mode(_) | Action::Permission(_) | Action::Model(_) | Action::Harness(_)
            ) {
                this.composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
        });
    })
}

pub(crate) fn environment(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let remote = this.project().remote.as_ref();
    let label = remote
        .and_then(|project| this.remote_host(&project.host_id))
        .map(|host| host.name)
        .unwrap_or_else(|| {
            if remote.is_some() {
                "Remote relay".into()
            } else {
                "This Computer".into()
            }
        });
    let app = cx.entity().downgrade();
    trigger(
        "composer-environment",
        label,
        if remote.is_some() {
            IconName::Cloud
        } else {
            IconName::Monitor
        },
    )
    .tooltip("Choose an environment; existing conversations stay in their project")
    .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
        let Some(entity) = app.upgrade() else {
            return menu;
        };
        let this = entity.read(cx);
        let busy = this.chat_controls_busy();
        let active_host = this
            .project()
            .remote
            .as_ref()
            .map(|project| &project.host_id);
        menu = menu.label("Environment").item(item(
            &app,
            ("This Computer".into(), "Local projects on this Mac".into()),
            IconName::Monitor,
            active_host.is_none(),
            busy,
            Action::Local,
        ));
        for host in &this.remote_hosts {
            menu = menu.item(item(
                &app,
                (host.name.clone(), host.url.clone()),
                IconName::Cloud,
                active_host == Some(&host.id),
                busy,
                Action::Remote(host.id.clone()),
            ));
        }
        menu.separator()
            .label("Changing environments opens a project there")
            .item(item(
                &app,
                (
                    "Manage remote environments…".into(),
                    "Pair or remove a self-hosted relay".into(),
                ),
                IconName::Settings2,
                false,
                false,
                Action::Settings(SettingsTab::Remote),
            ))
    })
    .on_open_change(track_menu(cx))
    .into_any_element()
}

pub(crate) fn harness(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let label = if this.project().is_remote() {
        match &this.harness.status {
            crate::harness::HarnessStatus::Ready { agent, .. } => agent.clone(),
            _ => "Relay harness".into(),
        }
    } else {
        this.session_harness()
    };
    let app = cx.entity().downgrade();
    trigger("composer-harness", label, IconName::Bot)
        .tooltip("Choose a harness for a new conversation")
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            if this.project().is_remote() {
                return menu.label("Harness is configured on the relay").item(item(
                    &app,
                    (
                        "Remote settings…".into(),
                        "Local executables cannot run on a relay".into(),
                    ),
                    IconName::Settings2,
                    false,
                    false,
                    Action::Settings(SettingsTab::Remote),
                ));
            }
            menu = menu.label("Switching harness starts a new conversation");
            for name in ["OpenCode", "Claude Code", "OpenAI Codex", "Custom ACP"] {
                let command = harness_command(name, &this.custom_harness_command);
                let available = command_available(&command);
                menu = menu.item(item(
                    &app,
                    (
                        name.into(),
                        if available {
                            command
                        } else {
                            format!("Not installed · {command}")
                        },
                    ),
                    IconName::Bot,
                    this.session_harness() == name,
                    this.chat_controls_busy() || !available,
                    Action::Harness(name.into()),
                ));
            }
            menu.separator().item(item(
                &app,
                (
                    "Configure harnesses…".into(),
                    "Custom commands and defaults".into(),
                ),
                IconName::Settings2,
                false,
                false,
                Action::Settings(SettingsTab::Harnesses),
            ))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

pub(crate) fn mode(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let permission = this.effective_permission_mode();
    let label = if this.harness.mode_changing {
        "Changing mode…".into()
    } else if let Some(modes) = &this.harness.modes {
        format!("{} · {}", modes.label(), permission_short(permission))
    } else {
        permission.label().into()
    };
    let app = cx.entity().downgrade();
    trigger("composer-mode", label, IconName::ShieldCheck)
        .tooltip("Agent mode and tool approval policy")
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let busy = this.chat_controls_busy();
            if let Some(modes) = &this.harness.modes {
                menu = menu.label("Agent mode · provided by the harness");
                for choice in &modes.choices {
                    menu = menu.item(item(
                        &app,
                        (
                            choice.name.clone(),
                            choice.description.clone().unwrap_or_default(),
                        ),
                        IconName::Bot,
                        choice.id == modes.current,
                        busy,
                        Action::Mode(choice.id.clone()),
                    ));
                }
                menu = menu.separator();
            } else {
                menu = menu.label("This harness has not advertised agent modes");
            }
            menu = menu.label("Tool permissions · Dunlin approval policy");
            for (mode, description, icon) in [
                (
                    PermissionMode::Ask,
                    "Ask before each tool permission request.",
                    IconName::ShieldCheck,
                ),
                (
                    PermissionMode::ReadOnly,
                    "Reject tool permission requests. This is not a sandbox.",
                    IconName::Lock,
                ),
                (
                    PermissionMode::AutoApprove,
                    "Approve requested tools automatically. They may edit files or run commands.",
                    IconName::TriangleAlert,
                ),
            ] {
                menu = menu.item(item(
                    &app,
                    (mode.label().into(), description.into()),
                    icon,
                    mode == this.effective_permission_mode(),
                    busy,
                    Action::Permission(mode),
                ));
            }
            menu.scrollable(true).max_h(px(440.))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

fn permission_short(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "Ask",
        PermissionMode::ReadOnly => "Read-only",
        PermissionMode::AutoApprove => "Auto-approve",
    }
}

pub(crate) fn model(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let visible = this.visible_models();
    let label = if this.harness.model_changing {
        "Changing model…".into()
    } else if this.local_opencode() && this.provider_catalog.loading {
        "Loading models…".into()
    } else {
        this.harness
            .models
            .as_ref()
            .filter(|models| visible.iter().any(|choice| choice.id == models.current))
            .map(|models| models.label().to_string())
            .unwrap_or_else(|| "Choose model".into())
    };
    let app = cx.entity().downgrade();
    trigger("composer-model", label, IconName::Sparkles)
        .tooltip("Choose a model from the connected harness")
        .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let choices = this.visible_models();
            let scope = this.model_scope();
            let (recent, remaining) = crate::providers::sections(&choices, &this.recent_models, &scope);
            let current = this.harness.models.as_ref().map(|models| models.current.as_str());
            if !choices.is_empty() {
                for (label, choices) in [("Recently used", recent), ("Provider models", remaining)] {
                    if choices.is_empty() {
                        continue;
                    }
                    menu = menu.label(label);
                    for choice in choices {
                        menu = menu.item(item(
                            &app,
                            (
                                choice.name.clone(),
                                choice.description.clone().unwrap_or_else(|| choice.id.clone()),
                            ),
                            IconName::Sparkles,
                            Some(choice.id.as_str()) == current,
                            this.chat_controls_busy(),
                            Action::Model(choice.id.clone()),
                        ));
                    }
                }
            } else {
                menu = menu
                    .label(if this.local_opencode() {
                        this.provider_catalog.error.clone().unwrap_or_else(|| "No models available for this provider. Connect your account and refresh.".into())
                    } else { "No selectable models advertised by the harness".into() })
                    .item(item(
                        &app,
                        (
                            "Connect / retry harness".into(),
                            "Model availability depends on your harness and provider configuration"
                                .into(),
                        ),
                        IconName::RefreshCw,
                        false,
                        this.chat_controls_busy(),
                        Action::Connect,
                    ));
            }
            if this.local_opencode() {
                menu = menu.separator().item(item(
                    &app,
                    ("Connect provider account…".into(), "Official OpenCode sign-in in Terminal".into()),
                    IconName::KeyRound, false, this.chat_controls_busy(), Action::ConnectProvider,
                )).item(item(
                    &app,
                    ("Refresh providers / models".into(), "Use after signing in or changing accounts".into()),
                    IconName::RefreshCw, false, this.chat_controls_busy() || this.provider_catalog.loading, Action::RefreshProviders,
                ));
            }
            menu.scrollable(true).max_h(px(420.))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

/// Providers are inside OpenCode, never additional ACP harnesses. Remote
/// credentials belong on the relay, not on this Mac.
pub(crate) fn provider(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let selected = this.selected_provider();
    let label = this
        .provider_catalog
        .providers
        .iter()
        .find(|provider| Some(provider.id.as_str()) == selected)
        .map(|provider| provider.name.clone())
        .unwrap_or_else(|| {
            if selected == Some(crate::providers::GO) {
                "OpenCode Go".into()
            } else {
                selected.unwrap_or("Choose provider").into()
            }
        });
    let app = cx.entity().downgrade();
    trigger("composer-provider", label, IconName::Cloud)
        .tooltip("Model provider and billing account inside OpenCode")
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let busy = this.chat_controls_busy();
            menu = menu.label("Provider · inside the OpenCode harness");
            if !this
                .provider_catalog
                .providers
                .iter()
                .any(|provider| provider.id == crate::providers::GO)
            {
                menu = menu.item(item(
                    &app,
                    (
                        "OpenCode Go".into(),
                        "Connect a Go account, then refresh".into(),
                    ),
                    IconName::Cloud,
                    this.selected_provider() == Some(crate::providers::GO),
                    busy,
                    Action::Provider(crate::providers::GO.into()),
                ));
            }
            for provider in &this.provider_catalog.providers {
                menu = menu.item(item(
                    &app,
                    (
                        provider.name.clone(),
                        format!(
                            "{} · {} account connection(s)",
                            provider.id, provider.accounts
                        ),
                    ),
                    IconName::Cloud,
                    this.selected_provider() == Some(provider.id.as_str()),
                    busy,
                    Action::Provider(provider.id.clone()),
                ));
            }
            menu.separator()
                .label("Availability does not confirm balance or subscription limits")
                .item(item(
                    &app,
                    (
                        "Connect provider account…".into(),
                        "Sign in or add an account using OpenCode in Terminal".into(),
                    ),
                    IconName::KeyRound,
                    false,
                    busy,
                    Action::ConnectProvider,
                ))
                .item(item(
                    &app,
                    (
                        "Refresh providers / models".into(),
                        "Use after completing sign-in".into(),
                    ),
                    IconName::RefreshCw,
                    false,
                    busy || this.provider_catalog.loading,
                    Action::RefreshProviders,
                ))
                .scrollable(true)
                .max_h(px(420.))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

pub(crate) fn provider_accounts(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let local = this.local_opencode();
    div().flex().flex_col().gap(px(8.))
        .child(div().text_size(px(11.5)).text_color(p.muted_fg).child(
            "OpenCode is the harness; OpenCode Go is a model provider within it. Account credentials stay in OpenCode, not in Dunlin's settings or conversation history."
        ))
        .when(local, |view| view
            .child(provider(this, cx))
            .child(Button::new("provider-account-connect").label("Connect account in Terminal…").small()
                .disabled(this.chat_controls_busy())
                .on_click(cx.listener(|this, _, _, cx| this.connect_provider(cx))))
            .child(Button::new("provider-account-refresh").label("Refresh providers / models").small()
                .disabled(this.chat_controls_busy() || this.provider_catalog.loading)
                .on_click(cx.listener(|this, _, _, cx| this.reload_provider_models(cx))))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child(
                "Finish OpenCode's official sign-in in Terminal (browser login or API key, depending on the provider), then refresh here. A listed model does not guarantee account funds or Go entitlement."
            ))
            .when_some(this.provider_catalog.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error))))
        .when(!local, |view| view.child(div().text_size(px(11.5)).text_color(p.muted_fg).child(
            "Select a local OpenCode conversation to manage its providers. For remote conversations, connect the provider on the relay."
        )))
        .into_any_element()
}
