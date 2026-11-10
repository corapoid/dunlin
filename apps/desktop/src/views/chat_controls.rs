//! Inline, keyboard-accessible composer dropdowns. Only advertise features
//! actually supported by the current environment and ACP harness.
use dunlin_ui::assets::IconName;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use dunlin_ui::component::{Disableable, Icon, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};

actions!(dunlin_chat, [NextAgentMode]);
struct ModeBindings;
impl Global for ModeBindings {}
pub(crate) fn init_mode_keys(cx: &mut App) {
    if !cx.has_global::<ModeBindings>() {
        cx.set_global(ModeBindings);
        cx.bind_keys([KeyBinding::new(
            "shift-tab",
            NextAgentMode,
            Some("DunlinChat"),
        )]);
    }
}

#[derive(Clone)]
enum Action {
    Settings(SettingsTab),
    Harness(String),
    Mode(String),
    Model(String),
    Provider(String),
    ConnectIntegration,
    RefreshProviders,
    Connect,
    RefreshMcp,
    ToggleMcp(String, bool),
    Attach,
    ToggleReasoning,
    ToggleSkill(String),
    ConfigOption(String, String),
}

fn trigger(id: &'static str, label: String, icon: IconName) -> Button {
    dunlin_ui::button(id)
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
    let branded = matches!(action, Action::Harness(_));
    #[cfg(debug_assertions)]
    let probe = match &action {
        Action::ToggleReasoning => Some("chat-option-reasoning"),
        Action::ToggleSkill(id) if id == crate::skills::COMMIT_SKILL => {
            Some("chat-option-commit-skill")
        }
        Action::ConfigOption(id, value) if id == "reasoning-effort" && value == "high" => {
            Some("chat-option-effort-high")
        }
        _ => None,
    };
    PopupMenuItem::element(move |window, cx| {
        let p = palette(cx);
        crate::effects::menu_surface(
            div()
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        probe.and_then(crate::workspace_ui_smoke::probe)
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .min_w(px(240.))
                .max_w(px(350.))
                .py(px(4.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .opacity(if disabled { 0.5 } else { 1.0 })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .when(branded, |row| {
                            row.child(crate::harness_brand::logo(&text.0, 16., p.fg))
                        })
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(p.fg)
                                .child(text.0.clone()),
                        ),
                )
                .when(!text.1.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .line_height(relative(1.4))
                            .text_color(p.muted_fg)
                            .child(text.1.clone()),
                    )
                }),
            window,
            cx,
        )
    })
    .when(!branded, |item| item.icon(icon))
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
                Action::RefreshMcp => this.refresh_mcp(cx),
                Action::Attach => this.attach_files(window, cx),
                Action::ToggleReasoning => {
                    this.reasoning_open = !this.reasoning_open;
                    this.persist_settings(cx);
                    cx.notify();
                }
                Action::ToggleSkill(id) => this.toggle_conversation_skill(id, cx),
                _ if this.chat_controls_busy() => {
                    this.set_notice(
                        "Wait for the current operation before changing chat settings",
                        cx,
                    );
                }
                Action::Harness(name) => this.select_chat_harness(name, cx),
                Action::Mode(id) => this.select_mode(id, cx),
                Action::Model(id) => this.select_model(id, cx),
                Action::Provider(id) => this.select_provider(id, cx),
                Action::ConnectIntegration => this.open_provider_catalog(window, cx),
                Action::RefreshProviders => this.reload_provider_models(cx),
                Action::Connect => this.connect_harness(cx),
                Action::ToggleMcp(name, disabled) => this.toggle_mcp(name, *disabled, cx),
                Action::ConfigOption(id, value) => this.select_config_option(id, value, cx),
            }
            if matches!(
                action,
                Action::Mode(_) | Action::Model(_) | Action::Harness(_)
            ) {
                this.composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
        });
    })
}

/// Compact, single-line rows without decorative icons or wire identifiers.
fn compact_item(
    app: &WeakEntity<DunlinApp>,
    label: String,
    selected: bool,
    disabled: bool,
    action: Action,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::element(move |window, cx| {
        crate::effects::menu_surface(
            div()
                .min_w(px(220.))
                .max_w(px(350.))
                .py(px(2.))
                .text_size(px(12.5))
                .text_color(palette(cx).fg)
                .truncate()
                .child(label.clone()),
            window,
            cx,
        )
    })
    .checked(selected)
    .disabled(disabled)
    .on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| {
            match &action {
                Action::Model(id) => this.select_model(id, cx),
                Action::ToggleMcp(name, disabled) => this.toggle_mcp(name, *disabled, cx),
                Action::RefreshProviders => this.reload_provider_models(cx),
                Action::Connect => this.connect_harness(cx),
                _ => {}
            }
            if matches!(action, Action::Model(_)) {
                this.composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
        });
    })
}

pub(super) fn harness_label(this: &DunlinApp) -> String {
    if this.project().is_remote() {
        match &this.harness.status {
            crate::harness::HarnessStatus::Ready { agent, .. } => agent.clone(),
            _ => "Relay harness".into(),
        }
    } else {
        this.session_harness()
    }
}

pub(crate) fn harness(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let label = harness_label(this);
    let app = cx.entity().downgrade();
    dunlin_ui::button("composer-harness")
        .ghost()
        .small()
        .label(label)
        .dropdown_caret(true)
        .max_w(px(240.))
        .tooltip("Choose a harness for a new conversation")
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            if this.project().is_remote() {
                return menu.label("Harness is configured on the relay");
            }
            menu = menu.label("Switching harness starts a new conversation");
            let configured = this.ready_chat_harness_names();
            if configured.is_empty() {
                menu = menu.label("No verified harness connections");
            }
            for name in configured {
                menu = menu.item(item(
                    &app,
                    (name.clone(), String::new()),
                    IconName::Bot,
                    this.session_harness() == name,
                    this.chat_controls_busy(),
                    Action::Harness(name),
                ));
            }
            menu
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

pub(crate) fn mode(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let label = mode_label(this.harness.modes.as_ref(), this.harness.mode_changing);
    let app = cx.entity().downgrade();
    trigger("composer-mode", label, IconName::ShieldCheck)
        .tooltip(this.shortcut_label(
            "Next agent mode · approval policy is in Permissions settings",
            "⇧Tab",
        ))
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
            } else {
                menu = menu
                    .label("This harness has not advertised agent modes")
                    .item(item(
                        &app,
                        (
                            "Connect / retry harness".into(),
                            "Discover the modes supported by this harness".into(),
                        ),
                        IconName::Plug,
                        false,
                        busy,
                        Action::Connect,
                    ));
            }
            menu.separator()
                .item(item(
                    &app,
                    (
                        "Tool approval settings…".into(),
                        "Dunlin safety restrictions are separate from the harness mode".into(),
                    ),
                    IconName::Settings2,
                    false,
                    false,
                    Action::Settings(SettingsTab::Permissions),
                ))
                .scrollable(true)
                .max_h(px(440.))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

fn mode_label(modes: Option<&dunlin_agent::SessionModes>, changing: bool) -> String {
    if changing {
        "Changing mode…".into()
    } else {
        modes
            .map(|m| m.label().to_string())
            .unwrap_or_else(|| "Agent mode unavailable".into())
    }
}

pub(crate) fn next_mode(modes: &dunlin_agent::SessionModes) -> Option<&str> {
    let next = modes
        .choices
        .iter()
        .position(|choice| choice.id == modes.current)
        .map_or(0, |index| (index + 1) % modes.choices.len());
    modes.choices.get(next).map(|choice| choice.id.as_str())
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
            .map(|models| {
                crate::providers::model_name(models.label(), this.selected_provider()).to_string()
            })
            .unwrap_or_else(|| "Choose model".into())
    };
    let app = cx.entity().downgrade();
    dunlin_ui::button("composer-model").ghost().label(label).dropdown_caret(true).max_w(px(240.))
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
                let models_label = if this.local_opencode() { "Provider models" } else { "Harness models" };
                for (label, choices) in [("Recently used", recent), (models_label, remaining)] {
                    if choices.is_empty() {
                        continue;
                    }
                    menu = menu.label(label);
                    for choice in choices {
                        menu = menu.item(compact_item(
                            &app,
                            crate::providers::model_name(&choice.name, this.selected_provider()).to_string(),
                            Some(choice.id.as_str()) == current,
                            this.chat_controls_busy(),
                            Action::Model(choice.id.clone()),
                        ));
                    }
                }
            } else {
                menu = menu
                    .label(if this.local_opencode() {
                        this.provider_catalog.error.clone().unwrap_or_else(|| if this.selected_provider() == Some(crate::providers::FREE) {
                            "No verified free models available. Refresh providers / models; paid models will not be substituted.".into()
                        } else { "No models available for this provider. Connect your account and refresh.".into() })
                    } else { "No selectable models advertised by the harness".into() })
                    .item(compact_item(
                        &app,
                        "Connect / retry harness".into(),
                        false,
                        this.chat_controls_busy(),
                        Action::Connect,
                    ));
            }
            if this.local_opencode() {
                menu = menu.separator().item(compact_item(
                    &app,
                    "Refresh providers / models".into(),
                    false, this.chat_controls_busy() || this.provider_catalog.loading, Action::RefreshProviders,
                ));
            }
            menu.scrollable(true).max_h(px(420.))
        })
        .on_open_change(track_menu(cx))
        .into_any_element()
}

fn mcp_menu(mut menu: PopupMenu, app: &WeakEntity<DunlinApp>, cx: &App) -> PopupMenu {
    let Some(entity) = app.upgrade() else {
        return menu;
    };
    let this = entity.read(cx);
    if !this.local_opencode() {
        return menu.label("MCP inventory is managed by this harness / relay");
    }
    menu = menu.label("MCP · current session");
    if this.mcp.loading {
        menu = menu.label("Loading MCP servers…");
    } else if this.mcp_current() {
        if let Some(error) = &this.mcp.error {
            menu = menu.label(error.clone());
        }
        if this.mcp.servers.is_empty() {
            menu = menu.label("No MCP servers configured");
        }
        for server in &this.mcp.servers {
            let enabled = server.connected && !server.disabled;
            menu = menu.item(compact_item(
                app,
                format!("{} · {}", server.name, server.label()),
                enabled,
                this.chat_controls_busy() || this.mcp.changing || !server.toggleable,
                Action::ToggleMcp(server.name.clone(), enabled),
            ));
        }
    }
    menu.separator()
        .label("Applies next turn · no session restart")
        .label("Earlier tool output stays in context")
        .item(item(
            app,
            ("Refresh MCP".into(), String::new()),
            IconName::RefreshCw,
            false,
            this.mcp.loading || this.mcp.changing,
            Action::RefreshMcp,
        ))
        .scrollable(true)
        .max_h(px(420.))
}

fn skills_menu(mut menu: PopupMenu, app: &WeakEntity<DunlinApp>, cx: &App) -> PopupMenu {
    let Some(entity) = app.upgrade() else {
        return menu;
    };
    let this = entity.read(cx);
    menu = menu.label("Attach instructions to this conversation");
    let choices = this.conversation_skill_choices();
    for skill in &choices {
        menu = menu.item(item(
            app,
            (skill.title.clone(), skill.description.clone()),
            IconName::BookOpen,
            this.selected_conversation_skills().contains(&skill.id),
            false,
            Action::ToggleSkill(skill.id.clone()),
        ));
    }
    // Deleted skill selections remain removable instead of becoming invisible.
    for id in this
        .selected_conversation_skills()
        .iter()
        .filter(|id| !choices.iter().any(|skill| &skill.id == *id))
    {
        menu = menu.item(item(
            app,
            (
                format!("{id} · unavailable"),
                "Remove this missing skill".into(),
            ),
            IconName::TriangleAlert,
            true,
            false,
            Action::ToggleSkill(id.clone()),
        ));
    }
    menu.separator()
        .label("Applies to future messages; earlier context is not erased")
        .item(item(
            app,
            ("Manage skills…".into(), String::new()),
            IconName::Settings2,
            false,
            false,
            Action::Settings(SettingsTab::Skills),
        ))
        .scrollable(true)
        .max_h(px(440.))
}

pub(crate) fn model_options(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Vec<AnyElement> {
    this.harness
        .config_options
        .iter()
        .map(|option| {
            let id = option.id.clone();
            let app = cx.entity().downgrade();
            dunlin_ui::button(format!("composer-config:{}", option.id))
                .ghost()
                .small()
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        if option.id == "reasoning-effort" {
                            crate::workspace_ui_smoke::probe("composer-effort")
                        } else {
                            None
                        }
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .icon(IconName::SlidersHorizontal)
                .label(option.selector.label())
                .accessibility_label(format!("{} · {}", option.name, option.selector.label()))
                .dropdown_caret(true)
                .disabled(this.chat_controls_busy())
                .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
                    let Some(entity) = app.upgrade() else {
                        return menu;
                    };
                    let this = entity.read(cx);
                    let Some(current) = this
                        .harness
                        .config_options
                        .iter()
                        .find(|option| option.id == id)
                    else {
                        return menu;
                    };
                    menu = menu.label(current.name.clone());
                    for choice in &current.selector.choices {
                        menu = menu.item(item(
                            &app,
                            (
                                choice.name.clone(),
                                choice.description.clone().unwrap_or_default(),
                            ),
                            IconName::SlidersHorizontal,
                            choice.id == current.selector.current,
                            this.chat_controls_busy(),
                            Action::ConfigOption(current.id.clone(), choice.id.clone()),
                        ));
                    }
                    menu
                })
                .on_open_change(track_menu(cx))
                .into_any_element()
        })
        .collect()
}

pub(crate) fn options(_this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let app = cx.entity().downgrade();
    let menu_app = app.clone();
    icon_button("composer-options", IconName::Plus, "Chat options")
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("composer-options")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, window, cx| {
            let Some(entity) = menu_app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let mcp_label = if this.mcp_current() && !this.mcp.loading && this.mcp.error.is_none() {
                format!("MCP servers · {}", this.mcp.active_count())
            } else {
                "MCP servers".into()
            };
            let menu = menu
                .item(item(
                    &menu_app,
                    ("Add photos & files…".into(), String::new()),
                    IconName::Paperclip,
                    false,
                    false,
                    Action::Attach,
                ))
                .item(item(
                    &menu_app,
                    (
                        "Show reasoning".into(),
                        if this.final_answer_only {
                            "Disabled by Final answer only".into()
                        } else {
                            "Show thought details in Chat".into()
                        },
                    ),
                    IconName::Brain,
                    this.reasoning_open && !this.final_answer_only,
                    this.final_answer_only,
                    Action::ToggleReasoning,
                ));
            let skills = menu_app.clone();
            let mcp = menu_app.clone();
            menu.separator()
                .submenu_with_icon(
                    Some(Icon::new(IconName::BookOpen)),
                    "Skills",
                    window,
                    cx,
                    move |menu, _, cx| skills_menu(menu, &skills, cx),
                )
                .submenu_with_icon(
                    Some(Icon::new(IconName::Plug)),
                    mcp_label,
                    window,
                    cx,
                    move |menu, _, cx| mcp_menu(menu, &mcp, cx),
                )
                .separator()
                .item(item(
                    &menu_app,
                    ("Chat settings…".into(), String::new()),
                    IconName::Settings2,
                    false,
                    false,
                    Action::Settings(SettingsTab::Chat),
                ))
        })
        .on_open_change(move |open, _, cx| {
            let _ = app.update(cx, |this, cx| {
                this.chat_menu_open = *open;
                if *open && (!this.mcp_current() || this.mcp.error.is_some()) {
                    this.refresh_mcp(cx);
                }
                cx.notify();
            });
        })
        .into_any_element()
}

/// Providers are inside OpenCode, never additional ACP harnesses. Remote
/// credentials belong on the relay, not on this Mac.
pub(crate) fn provider_controls(
    this: &DunlinApp,
    cx: &mut Context<DunlinApp>,
) -> Option<AnyElement> {
    if !this.local_opencode() {
        return None;
    }
    let p = palette(cx);
    let connected = this.provider_connected();
    Some(
        div()
            .id("composer-provider-controls")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(2.))
            .rounded(px(dunlin_ui::radii::ROW))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(provider(this, cx))
            .when(!connected && !this.provider_catalog.loading && !this.harness.status.is_busy(), |view| view.child(div().w(px(1.)).h(px(14.)).bg(p.border)).child(
                dunlin_ui::action_button("composer-provider-sign-in")
                    .icon(IconName::KeyRound)
                    .label("Sign in…")
                    .tooltip("Authorize the selected OpenCode provider; credentials are saved by OpenCode")
                    .disabled(this.chat_controls_busy() || this.selected_provider().is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.connect_provider(window, cx))),
            ))
            .into_any_element(),
    )
}

pub(crate) fn provider(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let selected = this.selected_provider();
    let label = this
        .provider_catalog
        .providers
        .iter()
        .find(|provider| Some(provider.id.as_str()) == selected)
        .map(|provider| provider.name.clone())
        .unwrap_or_else(|| {
            crate::providers::provider_name(selected.unwrap_or("Choose provider")).into()
        });
    let app = cx.entity().downgrade();
    trigger("composer-provider", label, IconName::Cloud)
        .accessibility_label("OpenCode model provider and billing account")
        .tooltip("Model provider and billing account inside OpenCode")
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let busy = this.chat_controls_busy();
            menu = menu.label("Connected OpenCode providers");
            if this.provider_catalog.has_free_models(&this.project().path) {
                menu = menu.item(item(
                    &app,
                    (
                        "OpenCode Free".into(),
                        "Only verified zero-cost OpenCode models · no Go subscription".into(),
                    ),
                    IconName::Cloud,
                    this.selected_provider() == Some(crate::providers::FREE),
                    busy,
                    Action::Provider(crate::providers::FREE.into()),
                ));
            }
            if this.provider_catalog.loading {
                menu = menu.label("Loading providers…");
            } else if let Some(error) = &this.provider_catalog.error {
                menu = menu.label(error.clone());
            }
            let connected = this
                .provider_catalog
                .connected_providers(&this.project().path);
            if connected.is_empty()
                && !this.provider_catalog.loading
                && this.provider_catalog.error.is_none()
            {
                menu = menu.label("No providers connected in OpenCode");
            }
            for provider in connected {
                menu = menu.item(item(
                    &app,
                    (
                        provider.name.clone(),
                        format!("{} connected account(s)", provider.accounts),
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
                        "Connect an integration…".into(),
                        "Browse providers available in your OpenCode installation".into(),
                    ),
                    IconName::KeyRound,
                    false,
                    busy,
                    Action::ConnectIntegration,
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

#[cfg(test)]
mod mode_tests {
    #[test]
    fn mode_cycle_wraps_and_handles_missing_current_and_empty_choices() {
        let mut modes = dunlin_agent::SessionModes {
            config_id: None,
            current: "a".into(),
            choices: vec![
                dunlin_agent::ModelChoice {
                    id: "a".into(),
                    name: "A".into(),
                    description: None,
                },
                dunlin_agent::ModelChoice {
                    id: "b".into(),
                    name: "B".into(),
                    description: None,
                },
            ],
        };
        assert_eq!(super::next_mode(&modes), Some("b"));
        modes.current = "b".into();
        assert_eq!(super::next_mode(&modes), Some("a"));
        modes.current = "unknown".into();
        assert_eq!(super::next_mode(&modes), Some("a"));
        modes.choices.clear();
        assert_eq!(super::next_mode(&modes), None);
    }
    #[test]
    fn auto_is_only_the_harness_mode_and_unknown_modes_are_never_invented() {
        let mut modes = dunlin_agent::SessionModes {
            config_id: Some("agent-mode".into()),
            current: "auto".into(),
            choices: vec![dunlin_agent::ModelChoice {
                id: "auto".into(),
                name: "Auto".into(),
                description: None,
            }],
        };
        assert_eq!(super::mode_label(Some(&modes), false), "Auto");
        assert_eq!(super::mode_label(Some(&modes), true), "Changing mode…");
        modes.current = "new-wire-mode".into();
        assert_eq!(super::mode_label(Some(&modes), false), "new-wire-mode");
        assert_eq!(modes.choices.len(), 1);
        assert_eq!(super::mode_label(None, false), "Agent mode unavailable");
    }
}
