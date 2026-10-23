//! Settings sheet. Everything that changes how Kestrel connects to a harness,
//! asks for permissions or measures usage lives here.

use kestrel_domain::{MainView, ProjectTone, UsageSource};
use kestrel_ui::assets::IconName;
use kestrel_ui::component::ThemeMode;
use kestrel_ui::component::input::Input;
use kestrel_ui::component::{ActiveTheme, Icon};
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;

use crate::app::{KestrelApp, PermissionMode, SettingsTab, tone_color, tone_soft};

pub(crate) fn render(
    this: &mut KestrelApp,
    window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let tab = this.settings_tab;

    let panel = div()
        .id("settings-panel")
        .w(px(872.))
        .h(px(602.))
        .rounded(px(14.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.bg)
        .overflow_hidden()
        .flex()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(settings_nav(this, cx))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(settings_header(this, cx))
                .child(
                    div()
                        .id("settings-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(match tab {
                            SettingsTab::General => general(this, window, cx),
                            SettingsTab::Harnesses => harnesses(this, cx),
                            SettingsTab::Permissions => permissions(this, cx),
                            SettingsTab::Editor => editor(this, cx),
                            SettingsTab::Usage => usage(this, cx),
                            SettingsTab::Advanced => advanced(this, cx),
                        }),
                ),
        );

    div()
        .id("settings-overlay")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(p.overlay)
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(|this, _, _, cx| {
            this.settings_open = false;
            cx.notify();
        }))
        .child(panel)
        .into_any_element()
}

fn settings_nav(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let mut nav = div()
        .w(px(186.))
        .h_full()
        .flex_shrink_0()
        .border_r_1()
        .border_color(p.border)
        .bg(p.sidebar)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(48.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(7.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .size(px(22.))
                        .rounded(px(6.))
                        .bg(p.fg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Bird).size(px(12.5)).text_color(p.bg)),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child("Settings"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("v0.1"),
                ),
        )
        .child(div().h(px(6.)));

    for tab in SettingsTab::ALL {
        let active = this.settings_tab == tab;
        nav = nav.child(
            div()
                .id(format!("settings-tab:{}", tab.label()))
                .mx(px(6.))
                .h(px(29.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .when(active, |this| this.bg(p.accent))
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_tab = tab;
                    cx.notify();
                }))
                .child(Icon::new(tab.icon()).size(px(12.5)).text_color(if active {
                    p.fg
                } else {
                    p.muted_fg
                }))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if active { p.fg } else { p.muted_fg })
                        .child(tab.label()),
                ),
        );
    }

    nav.child(div().flex_1())
        .child(
            div()
                .mx(px(10.))
                .mb(px(10.))
                .p(px(8.))
                .rounded(px(7.))
                .bg(p.muted)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            Icon::new(IconName::ShieldCheck)
                                .size(px(11.))
                                .text_color(p.green),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg.opacity(0.85))
                                .child("Local-first"),
                        ),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("Sessions, traces and usage stay on this Mac."),
                ),
        )
        .into_any_element()
}

fn settings_header(this: &KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let (title, subtitle) = match this.settings_tab {
        SettingsTab::General => (
            "General",
            "Appearance, startup behavior and the default workspace.",
        ),
        SettingsTab::Harnesses => (
            "Harnesses",
            "Connect Kestrel to ACP agents. OpenCode is the default.",
        ),
        SettingsTab::Permissions => ("Permissions", "Decide what an agent may do without asking."),
        SettingsTab::Editor => (
            "Editor",
            "Typography and editing behavior for the lightweight editor.",
        ),
        SettingsTab::Usage => (
            "Usage",
            "Subscription windows, limits and where the numbers come from.",
        ),
        SettingsTab::Advanced => ("Advanced", "Local data, logs, cache and recovery."),
    };

    div()
        .h(px(58.))
        .flex_shrink_0()
        .px(px(16.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .id("settings-close")
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings_open = false;
                    cx.notify();
                }))
                .child(Icon::new(IconName::X).size(px(13.5)).text_color(p.muted_fg)),
        )
        .into_any_element()
}

fn general(this: &mut KestrelApp, window: &mut Window, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let dark = cx.theme().is_dark();
    let view = this.view;
    let harness = this.default_harness.clone();

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(section_label("APPEARANCE", &p))
        .child(
            div()
                .flex()
                .gap(px(10.))
                .child(theme_card(false, !dark, window, cx, &p))
                .child(theme_card(true, dark, window, cx, &p)),
        )
        .child(section_label("STARTUP", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(setting_row(
                    "Restore the last session",
                    "Reopen the most recent conversation and its project.",
                    toggle_switch(
                        "restore-session",
                        this.auto_connect,
                        |this| this.auto_connect = !this.auto_connect,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Default view",
                    "Where a new session opens.",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .children(MainView::ALL.map(|candidate| {
                            let active = candidate == view;
                            div()
                                .id(format!("startup-view:{}", candidate.label()))
                                .h(px(21.))
                                .px(px(6.))
                                .rounded(px(4.))
                                .bg(if active { p.accent } else { p.bg.opacity(0.0) })
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.view = candidate;
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(if active { p.fg } else { p.muted_fg })
                                        .child(candidate.label()),
                                )
                                .into_any_element()
                        }))
                        .into_any_element(),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Default harness",
                    "New sessions connect through this ACP adapter.",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .h(px(21.))
                                .px(px(7.))
                                .rounded(px(4.))
                                .bg(p.green_soft)
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(dot(p.green, 4.5))
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.green)
                                        .child(harness),
                                ),
                        )
                        .child(
                            div()
                                .id("general-change-harness")
                                .text_size(px(9.))
                                .text_color(p.blue)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings_tab = SettingsTab::Harnesses;
                                    cx.notify();
                                }))
                                .child("Change"),
                        )
                        .into_any_element(),
                    &p,
                )),
        )
        .child(section_label("BEHAVIOR", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(setting_row(
                    "Connect automatically",
                    "Start the ACP process when a trusted project opens.",
                    toggle_switch(
                        "auto-connect",
                        this.auto_connect,
                        |this| this.auto_connect = !this.auto_connect,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Restart the harness on crash",
                    "Resume the session instead of dropping the turn.",
                    toggle_switch(
                        "restart-harness",
                        this.restart_harness,
                        |this| this.restart_harness = !this.restart_harness,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Show reasoning by default",
                    "Expand the model's thinking block in chat.",
                    toggle_switch(
                        "show-reasoning",
                        this.reasoning_open,
                        |this| this.reasoning_open = !this.reasoning_open,
                        cx,
                        &p,
                    ),
                    &p,
                )),
        );

    body = body.child(
        div()
            .p(px(10.))
            .rounded(px(8.))
            .bg(p.blue_soft)
            .flex()
            .items_start()
            .gap(px(7.))
            .child(Icon::new(IconName::Info).size(px(13.)).text_color(p.blue))
            .child(
                div()
                    .text_size(px(9.5))
                    .line_height(relative(1.5))
                    .text_color(p.blue)
                    .child("Kestrel never uploads project contents. Harness adapters run as local child processes and speak ACP over stdio."),
            ),
    );

    body.into_any_element()
}

fn theme_card(
    dark: bool,
    selected: bool,
    window: &mut Window,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    let preview = if dark {
        kestrel_ui::theme::dark()
    } else {
        kestrel_ui::theme::light()
    };
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let label = if dark { "Dark" } else { "Light" };
    let _ = window;

    div()
        .id(format!("theme-card-{label}"))
        .w(px(216.))
        .rounded(px(9.))
        .border_1()
        .border_color(if selected { p.fg } else { p.border })
        .bg(p.card)
        .overflow_hidden()
        .cursor_pointer()
        .on_click(cx.listener(move |_, _, window, cx| {
            set_mode(mode, Some(window), cx);
            cx.notify();
        }))
        .child(
            div()
                .h(px(72.))
                .bg(preview.bg)
                .flex()
                .child(
                    div()
                        .w(px(40.))
                        .h_full()
                        .bg(preview.panel)
                        .border_r_1()
                        .border_color(preview.border)
                        .p(px(6.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .children((0..5).map(|index| {
                            div()
                                .h(px(3.))
                                .w(if index == 0 {
                                    relative(0.75)
                                } else {
                                    relative(0.55)
                                })
                                .rounded_full()
                                .bg(if index == 0 {
                                    preview.fg
                                } else {
                                    preview.muted_fg.opacity(0.5)
                                })
                                .into_any_element()
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .p(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(
                            div()
                                .h(px(5.))
                                .w(relative(0.55))
                                .rounded_full()
                                .bg(preview.fg.opacity(0.8)),
                        )
                        .child(div().h(px(18.)).w_full().rounded(px(4.)).bg(preview.muted))
                        .child(
                            div()
                                .h(px(5.))
                                .w(relative(0.72))
                                .rounded_full()
                                .bg(preview.muted_fg.opacity(0.55)),
                        ),
                ),
        )
        .child(
            div()
                .h(px(32.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(if dark { IconName::Moon } else { IconName::Sun })
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(label),
                )
                .child(div().flex_1())
                .when(selected, |this| {
                    this.child(Icon::new(IconName::Check).size(px(12.)).text_color(p.green))
                }),
        )
        .into_any_element()
}

fn harnesses(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let current = this.default_harness.clone();
    let custom_command =
        crate::harness::harness_command("Custom ACP", &this.custom_harness_command);
    let harnesses = vec![
        (
            "OpenCode",
            "opencode acp".to_string(),
            "Auto",
            ProjectTone::Teal,
        ),
        (
            "Claude Code",
            "claude-agent-acp".to_string(),
            "Auto",
            ProjectTone::Violet,
        ),
        (
            "OpenAI Codex",
            "codex-acp".to_string(),
            "Auto",
            ProjectTone::Blue,
        ),
        (
            "Custom ACP",
            custom_command,
            "Any ACP agent",
            ProjectTone::Amber,
        ),
    ];

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("DEFAULT HARNESS", &p));

    let live_status = (current == "OpenCode").then(|| this.harness.status.clone());

    for (name, command, model, tone) in harnesses {
        let selected = current == name;
        let installed = crate::harness::command_available(&command);
        let (status, version) = match (name, live_status.as_ref()) {
            ("OpenCode", Some(crate::harness::HarnessStatus::Ready { version, .. })) => {
                ("Ready".to_string(), version.clone())
            }
            ("OpenCode", Some(crate::harness::HarnessStatus::Starting)) => {
                ("Connecting".to_string(), "—".to_string())
            }
            ("OpenCode", Some(crate::harness::HarnessStatus::Failed(_)))
            | ("OpenCode", Some(crate::harness::HarnessStatus::Exited(_))) => {
                ("Failed".to_string(), "—".to_string())
            }
            ("OpenCode", Some(crate::harness::HarnessStatus::Offline)) => {
                ("Offline".to_string(), "—".to_string())
            }
            ("OpenCode", None) if installed => ("Ready".to_string(), "—".to_string()),
            ("Custom ACP", _) if this.custom_harness_command.trim().is_empty() => {
                ("Not configured".to_string(), "—".to_string())
            }
            (_, _) if installed => ("Installed".to_string(), "—".to_string()),
            _ => ("Not installed".to_string(), "—".to_string()),
        };
        let color = tone_color(&p, tone);
        let soft = tone_soft(&p, tone);
        let name_owned = name.to_string();

        body = body.child(
            div()
                .id(format!("harness-card:{name}"))
                .rounded(px(9.))
                .border_1()
                .border_color(if selected {
                    color.opacity(0.55)
                } else {
                    p.border
                })
                .bg(if selected { soft.opacity(0.55) } else { p.card })
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    let changed = this.default_harness != name_owned;
                    this.default_harness = name_owned.clone();
                    if changed && this.auto_connect {
                        this.restart_harness(cx);
                    } else {
                        this.set_notice(format!("Default harness · {name_owned}"), cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(
                            div()
                                .size(px(26.))
                                .rounded(px(7.))
                                .bg(soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(if name == "Custom ACP" {
                                        IconName::Terminal
                                    } else {
                                        IconName::Bot
                                    })
                                    .size(px(13.5))
                                    .text_color(color),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.fg)
                                        .child(name.to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(8.5))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg)
                                        .child(if version == "—" {
                                            model.to_string()
                                        } else {
                                            format!("{model} · v{version}")
                                        }),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .h(px(17.))
                                .px(px(5.))
                                .rounded_full()
                                .bg(if status == "Ready" {
                                    p.green_soft
                                } else if status == "Installed" {
                                    p.blue_soft
                                } else {
                                    p.muted
                                })
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(dot(
                                    if status == "Ready" {
                                        p.green
                                    } else if status == "Installed" {
                                        p.blue
                                    } else {
                                        p.muted_fg
                                    },
                                    4.0,
                                ))
                                .child(
                                    div()
                                        .text_size(px(8.))
                                        .font_family("Geist Mono")
                                        .text_color(if status == "Ready" {
                                            p.green
                                        } else if status == "Installed" {
                                            p.blue
                                        } else {
                                            p.muted_fg
                                        })
                                        .child(status),
                                ),
                        )
                        .when(selected, |this| {
                            this.child(
                                div()
                                    .size(px(18.))
                                    .rounded_full()
                                    .bg(color)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Check).size(px(10.)).text_color(p.bg),
                                    ),
                            )
                        }),
                )
                .child(
                    div()
                        .h(px(26.))
                        .px(px(7.))
                        .rounded(px(5.))
                        .bg(p.subtle)
                        .border_1()
                        .border_color(p.border)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("$"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.fg.opacity(0.8))
                                .child(command),
                        )
                        .child(
                            div()
                                .id(format!("copy-command:{name}"))
                                .size(px(18.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(p.hover))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_notice("ACP command copied", cx);
                                }))
                                .child(
                                    Icon::new(IconName::Copy)
                                        .size(px(9.5))
                                        .text_color(p.muted_fg),
                                ),
                        ),
                ),
        );
    }

    body = body
        .child(section_label("CUSTOM ACP COMMAND", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .h(px(30.))
                        .px(px(7.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.bg)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("$"),
                        )
                        .child(
                            div().flex_1().min_w(px(0.)).child(
                                Input::new(&this.custom_command_input)
                                    .appearance(false)
                                    .bordered(false)
                                    .h(px(28.)),
                            ),
                        ),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("Kestrel spawns this command over stdio and speaks ACP. Select Custom ACP above to use it."),
                ),
        );

    let connected = this.harness.harness.is_some();
    let connection_detail = this
        .harness
        .status
        .detail()
        .unwrap_or_else(|| this.harness.status.label());
    let connect_label = if connected { "Restart" } else { "Connect" };

    body = body.child(section_label("CONNECTION", &p)).child(
        div()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(setting_row(
                "Connection",
                &connection_detail,
                div()
                    .id("harness-connect")
                    .h(px(22.))
                    .px(px(9.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.bg)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.harness.harness.is_some() {
                            this.restart_harness(cx);
                        } else {
                            this.connect_harness(cx);
                        }
                    }))
                    .child(
                        Icon::new(IconName::RefreshCw)
                            .size(px(10.5))
                            .text_color(p.muted_fg),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(p.fg)
                            .child(connect_label),
                    )
                    .into_any_element(),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Auto-connect",
                "Launch the selected harness when a project opens.",
                toggle_switch(
                    "harness-auto-connect",
                    this.auto_connect,
                    |this| this.auto_connect = !this.auto_connect,
                    cx,
                    &p,
                ),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Restart on crash",
                "Resume the same ACP session after an adapter exits unexpectedly.",
                toggle_switch(
                    "harness-restart",
                    this.restart_harness,
                    |this| this.restart_harness = !this.restart_harness,
                    cx,
                    &p,
                ),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Custom harnesses",
                "Any executable that speaks ACP over stdio can be added.",
                div()
                    .id("add-custom-harness")
                    .h(px(22.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.bg)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_notice("Custom ACP · enter a command and environment", cx);
                    }))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(10.5))
                            .text_color(p.muted_fg),
                    )
                    .child(div().text_size(px(9.)).text_color(p.fg).child("Add…"))
                    .into_any_element(),
                &p,
            )),
    );

    body.into_any_element()
}

fn permissions(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let current = this.permission_mode;
    let modes = [
        (
            PermissionMode::Ask,
            "Ask every time",
            "Reads are automatic. Writes, shell commands and network calls need approval.",
            IconName::ShieldCheck,
            p.blue,
            p.blue_soft,
        ),
        (
            PermissionMode::ReadOnly,
            "Read-only",
            "The agent can inspect the workspace but cannot modify files or run commands.",
            IconName::Lock,
            p.amber,
            p.amber_soft,
        ),
        (
            PermissionMode::AutoApprove,
            "Auto-approve",
            "Every tool call runs without prompting. Use only in trusted, disposable projects.",
            IconName::Zap,
            p.red,
            p.red_soft,
        ),
    ];

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("DEFAULT PERMISSION MODE", &p));

    for (mode, title, description, icon_name, color, soft) in modes {
        let selected = current == mode;
        body = body.child(
            div()
                .id(format!("permission-mode:{title}"))
                .rounded(px(9.))
                .border_1()
                .border_color(if selected {
                    color.opacity(0.55)
                } else {
                    p.border
                })
                .bg(if selected { soft.opacity(0.5) } else { p.card })
                .p(px(10.))
                .flex()
                .items_start()
                .gap(px(9.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.permission_mode = mode;
                    this.set_notice(format!("Permission mode · {}", mode.label()), cx);
                }))
                .child(
                    div()
                        .size(px(28.))
                        .flex_shrink_0()
                        .rounded(px(7.))
                        .bg(soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(icon_name).size(px(14.)).text_color(color)),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(9.5))
                                .line_height(relative(1.5))
                                .text_color(p.muted_fg)
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .size(px(16.))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { color } else { p.border_strong })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(div().size(px(8.)).rounded_full().bg(color))
                        }),
                ),
        );
    }

    body = body.child(section_label("SCOPE", &p)).child(
        div()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(setting_row(
                "Workspace writes",
                "Files inside the selected project can be edited when a mode allows it.",
                scope_badge("allowed", p.green, p.green_soft, &p),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Home directory",
                "Reads outside the project require an explicit approval each time.",
                scope_badge("ask", p.amber, p.amber_soft, &p),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Network access",
                "Web fetch, package installs and remote APIs.",
                toggle_switch(
                    "allow-network",
                    this.allow_network,
                    |this| this.allow_network = !this.allow_network,
                    cx,
                    &p,
                ),
                &p,
            ))
            .child(divider(&p))
            .child(setting_row(
                "Secrets",
                "Values from .env files and the keychain stay redacted in tool output.",
                scope_badge("redacted", p.violet, p.violet_soft, &p),
                &p,
            )),
    );

    body.into_any_element()
}

fn editor(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let word_wrap = this.word_wrap;

    let body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("TYPOGRAPHY", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(setting_row(
                    "UI font",
                    "Bundled with Kestrel. No paid license required.",
                    value_chip("Geist", &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Code font",
                    "Monospaced face for files, traces and diffs.",
                    value_chip("Geist Mono", &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Font size",
                    "Applies to the editor and trace labels.",
                    stepper("13 px", cx, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Word wrap",
                    "Wrap long lines in the lightweight editor.",
                    toggle_switch(
                        "word-wrap",
                        word_wrap,
                        |this| this.word_wrap = !this.word_wrap,
                        cx,
                        &p,
                    ),
                    &p,
                )),
        )
        .child(section_label("PREVIEW", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .overflow_hidden()
                .child(
                    div()
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .border_b_1()
                        .border_color(p.border)
                        .bg(p.subtle)
                        .child(
                            Icon::new(IconName::FileCode)
                                .size(px(11.))
                                .text_color(p.blue),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("supervisor.rs"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(8.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("Rust"),
                        ),
                )
                .child(
                    div().p(px(9.)).flex().flex_col().gap(px(1.)).children(
                        [
                            ("42", "pub fn route_permission(", p.violet),
                            ("43", "    child: &SessionId,", p.fg.opacity(0.8)),
                            ("44", "    request: PermissionRequest,", p.fg.opacity(0.8)),
                            ("45", ") -> Result<()> {", p.fg.opacity(0.8)),
                            ("46", "    self.projection", p.blue),
                            ("47", "        .active_turn(child)?", p.fg.opacity(0.8)),
                            ("48", "        .send(request)?;", p.fg.opacity(0.8)),
                            ("49", "    Ok(())", p.fg.opacity(0.8)),
                            ("50", "}", p.violet),
                        ]
                        .map(|(number, line, color)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .w(px(18.))
                                        .text_right()
                                        .text_size(px(8.5))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg.opacity(0.5))
                                        .child(number),
                                )
                                .child(
                                    div()
                                        .text_size(px(9.5))
                                        .font_family("Geist Mono")
                                        .text_color(color)
                                        .child(line),
                                )
                                .into_any_element()
                        }),
                    ),
                ),
        );

    body.into_any_element()
}

fn usage(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let usages = this.usages.clone();

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .child(section_label("SUBSCRIPTION WINDOWS", &p))
                .child(div().flex_1())
                .child(
                    div()
                        .id("refresh-usage")
                        .h(px(22.))
                        .px(px(7.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.card)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_notice(
                                "Usage refreshed · Codex live, Claude and OpenCode estimated",
                                cx,
                            );
                        }))
                        .child(
                            Icon::new(IconName::RefreshCw)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.fg)
                                .child("Refresh all"),
                        ),
                ),
        );

    for usage in usages {
        let color = tone_color(&p, usage.accent);
        let soft = tone_soft(&p, usage.accent);
        let mut card = div()
            .rounded(px(9.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(6.))
                            .bg(soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Gauge).size(px(12.5)).text_color(color)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(usage.provider.clone()),
                    )
                    .child(
                        div()
                            .h(px(16.))
                            .px(px(5.))
                            .rounded(px(3.))
                            .bg(soft)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(7.5))
                                    .font_family("Geist Mono")
                                    .text_color(color)
                                    .child(usage.plan.clone()),
                            ),
                    )
                    .child(div().flex_1())
                    .child(source_badge(
                        usage
                            .primary()
                            .map(|window| window.source)
                            .unwrap_or(UsageSource::Unavailable),
                        &p,
                    )),
            );

        for window in &usage.windows {
            let percent = window.used_percent.clamp(0.0, 100.0);
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(
                                div()
                                    .text_size(px(9.))
                                    .text_color(p.fg.opacity(0.8))
                                    .child(window.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(8.))
                                    .font_family("Geist Mono")
                                    .text_color(p.muted_fg)
                                    .child(format!("resets in {}", window.resets_in)),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .font_family("Geist Mono")
                                    .text_color(if percent > 85. { p.red } else { p.muted_fg })
                                    .child(format!("{percent:.0}%")),
                            ),
                    )
                    .child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .rounded_full()
                            .bg(p.muted)
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(percent / 100.0))
                                    .rounded_full()
                                    .bg(if percent > 85. { p.red } else { color }),
                            ),
                    ),
            );
        }

        card = card.child(
            div()
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(usage.detail.clone()),
        );
        body = body.child(card);
    }

    body = body.child(
        div()
            .p(px(10.))
            .rounded(px(8.))
            .bg(p.amber_soft)
            .flex()
            .items_start()
            .gap(px(7.))
            .child(Icon::new(IconName::Info).size(px(13.)).text_color(p.amber))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(9.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.amber)
                            .child("Where these numbers come from"),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .line_height(relative(1.5))
                            .text_color(p.amber.opacity(0.9))
                            .child("Codex is exact, read from account/rateLimits/read. Claude Code windows are estimated from local session transcripts unless an OAuth probe is configured. OpenCode Go has no public usage API yet, so Kestrel estimates from token counts and the selected plan."),
                    ),
            ),
    );

    body.into_any_element()
}

fn advanced(_this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);

    let body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("LOCAL DATA", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(setting_row(
                    "Data directory",
                    "SQLite database, session events, spans and the search index.",
                    mono_value("~/Library/Application Support/Kestrel", &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Cache",
                    "Icon atlas, font shaping and span projections.",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("1.2 GB"),
                        )
                        .child(
                            div()
                                .id("clear-cache")
                                .text_size(px(9.))
                                .text_color(p.blue)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_notice("Cache cleared · projections rebuild on demand", cx);
                                }))
                                .child("Clear"),
                        )
                        .into_any_element(),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "ACP logs",
                    "Raw JSON-RPC frames for the last seven days.",
                    div()
                        .id("open-logs")
                        .h(px(21.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(p.border)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_notice("ACP logs · ~/Library/Logs/Kestrel", cx);
                        }))
                        .child(Icon::new(IconName::FolderOpen).size(px(10.)).text_color(p.muted_fg))
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.fg)
                                .child("Open"),
                        )
                        .into_any_element(),
                    &p,
                )),
        )
        .child(section_label("RECOVERY", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.red.opacity(0.3))
                .bg(p.red_soft)
                .p(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.red)
                                .child("Reset local data"),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.red.opacity(0.8))
                                .child("Delete sessions, traces and settings. Projects on disk are untouched."),
                        ),
                )
                .child(
                    div()
                        .id("reset-data")
                        .h(px(23.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(p.red.opacity(0.35))
                        .bg(p.card)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_notice("Reset cancelled · this action needs a confirmation sheet", cx);
                        }))
                        .child(Icon::new(IconName::RotateCcw).size(px(10.)).text_color(p.red))
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.red)
                                .child("Reset…"),
                        ),
                ),
        )
        .child(section_label("ABOUT", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .p(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(26.))
                        .rounded(px(7.))
                        .bg(p.fg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Bird).size(px(13.)).text_color(p.bg)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child("Kestrel 0.1.0"),
                        )
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child("Rust · GPUI · ACP · macOS arm64"),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("Apache-2.0"),
                ),
        );

    body.into_any_element()
}

fn section_label(label: &str, p: &Palette) -> AnyElement {
    div()
        .text_size(px(8.5))
        .font_family("Geist Mono")
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted_fg)
        .child(label.to_string())
        .into_any_element()
}

fn setting_row(title: &str, description: &str, control: AnyElement, p: &Palette) -> AnyElement {
    div()
        .min_h(px(44.))
        .px(px(10.))
        .py(px(7.))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .line_height(relative(1.4))
                        .text_color(p.muted_fg)
                        .child(description.to_string()),
                ),
        )
        .child(control)
        .into_any_element()
}

fn divider(p: &Palette) -> AnyElement {
    div().mx(px(10.)).h(px(1.)).bg(p.border).into_any_element()
}

fn toggle_switch(
    id: &'static str,
    value: bool,
    action: fn(&mut KestrelApp),
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    div()
        .id(id)
        .w(px(30.))
        .h(px(17.))
        .rounded_full()
        .bg(if value { p.green } else { p.border_strong })
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            action(this);
            cx.notify();
        }))
        .child(
            div()
                .size(px(13.))
                .rounded_full()
                .bg(p.card)
                .ml(px(if value { 15. } else { 2. }))
                .border_1()
                .border_color(p.border.opacity(0.5)),
        )
        .into_any_element()
}

fn value_chip(label: &str, p: &Palette) -> AnyElement {
    div()
        .h(px(21.))
        .px(px(6.))
        .rounded(px(4.))
        .bg(p.muted)
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(9.))
                .font_family("Geist Mono")
                .text_color(p.fg.opacity(0.8))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn mono_value(label: &str, p: &Palette) -> AnyElement {
    div()
        .max_w(px(230.))
        .h(px(21.))
        .px(px(6.))
        .rounded(px(4.))
        .bg(p.muted)
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .truncate()
                .child(label.to_string()),
        )
        .into_any_element()
}

fn stepper(label: &str, cx: &mut Context<KestrelApp>, p: &Palette) -> AnyElement {
    div()
        .h(px(21.))
        .rounded(px(4.))
        .border_1()
        .border_color(p.border)
        .flex()
        .items_center()
        .child(
            div()
                .id("font-decrease")
                .w(px(20.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_notice("Editor font size · 12 px", cx);
                }))
                .child(
                    Icon::new(IconName::Minus)
                        .size(px(9.))
                        .text_color(p.muted_fg),
                ),
        )
        .child(
            div()
                .px(px(5.))
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.fg.opacity(0.8))
                .child(label.to_string()),
        )
        .child(
            div()
                .id("font-increase")
                .w(px(20.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_notice("Editor font size · 14 px", cx);
                }))
                .child(
                    Icon::new(IconName::Plus)
                        .size(px(9.))
                        .text_color(p.muted_fg),
                ),
        )
        .into_any_element()
}

fn scope_badge(label: &str, fg: Hsla, bg: Hsla, _p: &Palette) -> AnyElement {
    div()
        .h(px(18.))
        .px(px(5.))
        .rounded_full()
        .bg(bg)
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(8.))
                .font_family("Geist Mono")
                .text_color(fg)
                .child(label.to_uppercase()),
        )
        .into_any_element()
}

fn source_badge(source: UsageSource, p: &Palette) -> AnyElement {
    let (label, fg, bg) = match source {
        UsageSource::Live => ("LIVE", p.green, p.green_soft),
        UsageSource::Session => ("SESSION", p.blue, p.blue_soft),
        UsageSource::Estimated => ("ESTIMATED", p.amber, p.amber_soft),
        UsageSource::Unavailable => ("N/A", p.muted_fg, p.muted),
    };
    div()
        .h(px(17.))
        .px(px(5.))
        .rounded_full()
        .bg(bg)
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(7.5))
                .font_family("Geist Mono")
                .text_color(fg)
                .child(label),
        )
        .into_any_element()
}
