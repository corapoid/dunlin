//! Settings sheet. Everything that changes how Dunlin connects to a harness,
//! asks for permissions or measures usage lives here.

use dunlin_domain::{MainView, ProjectTone, UsageSource};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::ThemeMode;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::{Editor, Input};
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::component::{ActiveTheme, Icon};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab, tone_color, tone_soft};
use dunlin_domain::PermissionMode;
use dunlin_ui::component::Disableable;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum DesignSection {
    #[default]
    Application,
    Sidebar,
    Agent,
    Editor,
}
impl DesignSection {
    const ALL: [Self; 4] = [Self::Application, Self::Sidebar, Self::Agent, Self::Editor];
    fn label(self) -> &'static str {
        match self {
            Self::Application => "Application",
            Self::Sidebar => "Sidebar",
            Self::Agent => "Agent",
            Self::Editor => "Editor",
        }
    }
}
pub(crate) fn settings_size(width: f32, height: f32) -> (f32, f32) {
    (
        (width - 48.).clamp(360., 1120.),
        (height - 48.).clamp(320., 760.),
    )
}

#[cfg(test)]
mod design_tests {
    #[test]
    fn larger_settings_still_fit_the_supported_window_sizes() {
        assert_eq!(super::settings_size(1360., 860.), (1120., 760.));
        assert_eq!(super::settings_size(1100., 720.), (1052., 672.));
        for (w, h) in [(1100., 720.), (1360., 860.), (1920., 1080.)] {
            let (width, height) = super::settings_size(w, h);
            assert!(width <= w - 48.);
            assert!(height <= h - 48.);
            assert!(width > 872.);
            assert!(height > 602.);
        }
    }
    #[test]
    fn sidebar_branch_preference_migrates_and_roundtrips_without_resetting_other_design_choices() {
        let mut settings: crate::app::StoredSettings = serde_json::from_str(
            r#"{"dark":false,"editor_minimap":false,"sidebar_width":410,"chat_logo":false}"#,
        )
        .unwrap();
        assert!(settings.show_project_branches);
        settings.show_project_branches = false;
        let saved: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!saved.show_project_branches);
        assert!(!saved.dark);
        assert!(!saved.editor_minimap);
        assert!(!saved.chat_logo);
        assert_eq!(saved.sidebar_width, 410.);
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let tab = this.settings_tab;
    let (width, height) = settings_size(
        f32::from(window.viewport_size().width),
        f32::from(window.viewport_size().height),
    );

    let panel = div()
        .id("settings-panel")
        .child(crate::effects::rounded_probe(&this.overlay_backdrop, 14.))
        .w(px(width))
        .h(px(height))
        .rounded(px(14.))
        .border_1()
        .border_color(crate::effects::panel_border(&this.overlay_backdrop, &p))
        .shadow(crate::effects::panel_shadow())
        // GPUI clips overflow rectangularly. Inset child backgrounds enough
        // that they cannot paint into the outer rounded corners in light mode.
        .p(px(6.))
        .bg(crate::effects::panel_fill(&this.overlay_backdrop, p.bg))
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
                .rounded_tr(px(13.))
                .rounded_br(px(13.))
                .bg(crate::effects::panel_fill(&this.overlay_backdrop, p.bg))
                .child(settings_header(this, cx))
                .child(
                    div()
                        .id("settings-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(match tab {
                            SettingsTab::General => general(this, window, cx),
                            SettingsTab::Design => design(this, window, cx),
                            SettingsTab::Chat => chat_settings(this, cx),
                            SettingsTab::Harnesses => harnesses(this, cx),
                            SettingsTab::Skills => crate::skills::render(this, cx),
                            SettingsTab::Permissions => permissions(this, cx),
                            SettingsTab::Remote => remote_settings(this, cx),
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

fn settings_nav(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut nav = div()
        .w(px(186.))
        .min_h(px(0.))
        .overflow_hidden()
        .flex_shrink_0()
        .border_r_1()
        .border_color(p.border)
        .bg(crate::effects::panel_fill(
            &this.overlay_backdrop,
            p.sidebar,
        ))
        .rounded_tl(px(13.))
        .rounded_bl(px(13.))
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
                        .text_color(p.bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Bird).size(px(12.5)).text_color(p.bg)),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child("Settings"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family(dunlin_ui::theme::code_font())
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
                        .text_size(px(12.0))
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
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg.opacity(0.85))
                                .child("Local-first"),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("Sessions, traces and usage stay on this Mac."),
                ),
        )
        .into_any_element()
}

fn settings_header(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let (title, subtitle) = match this.settings_tab {
        SettingsTab::General => (
            "General",
            "Startup behavior and the default workspace. Appearance lives in Design.",
        ),
        SettingsTab::Design => (
            "Design",
            "Configure the application, sidebar, agent conversations and editor.",
        ),
        SettingsTab::Chat => (
            "Chat",
            "Message appearance, timestamps and response details.",
        ),
        SettingsTab::Harnesses => (
            "Harnesses",
            "Connect Dunlin to ACP agents. OpenCode is the default.",
        ),
        SettingsTab::Permissions => ("Permissions", "Decide what an agent may do without asking."),
        SettingsTab::Skills => (
            "Skills",
            "Create reusable instructions and install them globally for your harnesses.",
        ),
        SettingsTab::Remote => (
            "Remote",
            "Pair this Mac with a relay and continue sessions on your other devices.",
        ),
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
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.5))
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

fn general(this: &mut DunlinApp, _window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let view = this.default_view;
    let harness = this.default_harness.clone();

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(section_label("STARTUP", &p))
        .child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(setting_row(
                    "Startup destination",
                    "Choose Dashboard or your last project. Conversation history is preserved.",
                    div().flex().gap(px(4.)).children([(true, "Dashboard"), (false, "Last project")].map(|(dashboard, label)| div().id(format!("startup-destination:{dashboard}")).px(px(8.)).py(px(4.)).rounded(px(5.)).cursor_pointer().bg(if this.dashboard_settings.start_at_dashboard == dashboard { p.accent } else { p.card })
                        .on_click(cx.listener(move |this, _, _, cx| { this.dashboard_settings.start_at_dashboard = dashboard; this.persist_settings(cx); cx.notify(); })).child(label))).into_any_element(),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Restore the last session",
                    "Reopen the most recent conversation and its project.",
                    toggle_switch(
                        "restore-session",
                        this.restore_session,
                        |this| this.restore_session = !this.restore_session,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row("Dashboard live agents", "Show working, waiting and connecting agents.", toggle_switch("dashboard-agents", this.dashboard_settings.show_agents, |this| this.dashboard_settings.show_agents = !this.dashboard_settings.show_agents, cx, &p), &p))
                .child(divider(&p))
                .child(setting_row("Dashboard activity", "Show recorded metrics and the configurable heatmap.", toggle_switch("dashboard-activity", this.dashboard_settings.show_activity, |this| this.dashboard_settings.show_activity = !this.dashboard_settings.show_activity, cx, &p), &p))
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
                                    this.default_view = candidate;
                                    this.persist_settings(cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(11.0))
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
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.green)
                                        .child(harness),
                                ),
                        )
                        .child(
                            div()
                                .id("general-change-harness")
                                .text_size(px(11.0))
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
                    "Desktop notifications",
                    "Show a macOS notification when a turn finishes or a permission is waiting.",
                    toggle_switch(
                        "notifications",
                        this.notifications_enabled,
                        |this| this.notifications_enabled = !this.notifications_enabled,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Show keyboard shortcuts",
                    "Show shortcut hints in the title bar, menus and command palette. Shortcuts keep working when hidden.",
                    toggle_switch(
                        "shortcut-hints",
                        this.show_shortcut_hints,
                        |this| this.show_shortcut_hints = !this.show_shortcut_hints,
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
                    .text_size(px(11.5))
                    .line_height(relative(1.5))
                    .text_color(p.blue)
                    .child("Dunlin never uploads project contents. Harness adapters run as local child processes and speak ACP over stdio."),
            ),
    );

    body.into_any_element()
}

fn design(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let selected = this.design_section;
    let tabs = div()
        .px(px(16.))
        .pt(px(14.))
        .flex()
        .gap(px(6.))
        .children(DesignSection::ALL.map(|section| {
            div()
                .id(format!("design:{}", section.label()))
                .px(px(12.))
                .py(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(if selected == section {
                    p.border_strong
                } else {
                    p.border
                })
                .bg(if selected == section {
                    p.accent
                } else {
                    p.card
                })
                .text_size(px(12.))
                .text_color(p.fg)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.design_section = section;
                    cx.notify();
                }))
                .child(section.label())
        }));
    let content = match selected {
        DesignSection::Application => {
            let dark = cx.theme().is_dark();
            div().p(px(16.)).flex().flex_col().gap(px(14.))
                .child(section_label("THEME", &p))
                .child(div().flex().gap(px(12.)).child(theme_card(false, !dark, window, cx, &p)).child(theme_card(true, dark, window, cx, &p)))
                .child(section_label("INTERFACE", &p))
                .child(div().rounded(px(8.)).border_1().border_color(p.border).bg(p.card)
                    .child(setting_row("UI font", "Interface typography. Agent and editor configuration have dedicated sections above.", font_picker(this, false, cx), &p))
                    .child(divider(&p))
                    .child(setting_row("Keyboard hints", "Hide visual hints without disabling shortcuts.", toggle_switch("design-shortcuts", this.show_shortcut_hints, |this| this.show_shortcut_hints = !this.show_shortcut_hints, cx, &p), &p)))
                .into_any_element()
        }
        DesignSection::Sidebar => {
            let mut view = div().flex().gap(px(4.));
            for (candidate, label) in [
                (crate::project_tree::View::List, "List"),
                (crate::project_tree::View::Tree, "Tree"),
            ] {
                view = view.child(
                    Button::new(format!("design-view:{label}"))
                        .small()
                        .label(label)
                        .when(this.project_view == candidate, |b| b.bg(p.accent))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project_view = candidate;
                            this.persist_settings(cx);
                            cx.notify();
                        })),
                );
            }
            let mut sort = div().flex().gap(px(4.));
            for (candidate, label) in [
                (crate::project_tree::Sort::Name, "Name ↑"),
                (crate::project_tree::Sort::Modified, "Modified ↓"),
            ] {
                sort = sort.child(
                    Button::new(format!("design-sort:{label}"))
                        .small()
                        .label(label)
                        .when(this.project_sort == candidate, |b| b.bg(p.accent))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project_sort = candidate;
                            this.persist_settings(cx);
                            cx.notify();
                        })),
                );
            }
            let mut width = div().flex().items_center().gap(px(6.));
            for (label, delta) in [("−", -24.), ("+", 24.), ("Reset", 0.)] {
                width = width.child(
                    Button::new(format!("design-width:{label}"))
                        .small()
                        .label(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sidebar_width = if delta == 0. {
                                crate::sidebar_layout::DEFAULT_WIDTH
                            } else {
                                crate::sidebar_layout::width(
                                    this.sidebar_width + delta,
                                    f32::from(window.viewport_size().width),
                                )
                            };
                            this.persist_settings(cx);
                            cx.notify();
                        })),
                );
            }
            div().p(px(16.)).flex().flex_col().gap(px(14.)).child(section_label("PROJECT LIST", &p))
                .child(div().rounded(px(8.)).border_1().border_color(p.border).bg(p.card)
                    .child(setting_row("Show branch names", "Show or hide branch/Git labels below project names. Dirty-change indicators remain visible.", toggle_switch("project-branches", this.show_project_branches, |this| this.show_project_branches = !this.show_project_branches, cx, &p), &p))
                    .child(divider(&p)).child(setting_row("View", "List retains group breadcrumbs; Tree shows the hierarchy.", view.into_any_element(), &p))
                    .child(divider(&p)).child(setting_row("Sort", "Pinned projects stay first. Modification dates use cached repository metadata.", sort.into_any_element(), &p))
                    .child(divider(&p)).child(setting_row("Width", &format!("{} points. You can also drag the sidebar edge; double-click to reset.", this.sidebar_width.round()), width.into_any_element(), &p))
                    .child(divider(&p)).child(setting_row("Collapse account usage", "Start with the compact account usage footer.", toggle_switch("design-usage-collapse", this.account_usage_collapsed, |this| this.account_usage_collapsed = !this.account_usage_collapsed, cx, &p), &p)))
                .into_any_element()
        }
        DesignSection::Agent => chat_settings(this, cx),
        DesignSection::Editor => editor(this, cx),
    };
    div()
        .flex()
        .flex_col()
        .child(tabs)
        .child(content)
        .into_any_element()
}

fn chat_settings(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    type ChatToggle = (
        &'static str,
        &'static str,
        &'static str,
        bool,
        fn(&mut DunlinApp),
    );
    let controls: [ChatToggle; 6] = [
        (
            "Stream responses",
            "Show response text as the harness sends it, including in Final answer only mode. Turn off to wait for the completed response. A harness that buffers text cannot be made to stream by the UI.",
            "stream-responses",
            this.stream_responses,
            |this| this.stream_responses = !this.stream_responses,
        ),
        (
            "Message times",
            "Show the local time for your messages and agent responses. Older messages without timestamps stay unlabeled.",
            "chat-timestamps",
            this.chat_timestamps,
            |this| this.chat_timestamps = !this.chat_timestamps,
        ),
        (
            "Agent turn duration",
            "Show elapsed turn time in parentheses after the response, including tools and approval waits. Available for newly recorded turns.",
            "chat-duration",
            this.chat_duration,
            |this| this.chat_duration = !this.chat_duration,
        ),
        (
            "Response logo",
            "Show the Dunlin logo next to agent responses.",
            "chat-logo",
            this.chat_logo,
            |this| this.chat_logo = !this.chat_logo,
        ),
        (
            "Show reasoning by default",
            "Expand the model's thinking block in Chat.",
            "show-reasoning",
            this.reasoning_open,
            |this| this.reasoning_open = !this.reasoning_open,
        ),
        (
            "Final answer only",
            "Hide intermediate text, reasoning, tools, plans and subagent output. Permission requests and errors remain visible; full history stays in Graph / Trace.",
            "final-answer-only",
            this.final_answer_only,
            |this| this.final_answer_only = !this.final_answer_only,
        ),
    ];
    let mut card = div()
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.card);
    for (index, (title, description, id, value, action)) in controls.into_iter().enumerate() {
        if index > 0 {
            card = card.child(divider(&p));
        }
        card = card.child(setting_row(
            title,
            description,
            toggle_switch(id, value, action, cx, &p),
            &p,
        ));
    }
    div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("CHAT APPEARANCE", &p))
        .child(card)
        .into_any_element()
}

fn theme_card(
    dark: bool,
    selected: bool,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let preview = if dark {
        dunlin_ui::theme::dark()
    } else {
        dunlin_ui::theme::light()
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
        .on_click(cx.listener(move |this, _, window, cx| {
            set_mode(mode, Some(window), cx);
            this.persist_settings(cx);
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
                        .text_size(px(11.5))
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

fn harnesses(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let current = this.default_harness.clone();
    let discovery = this.harness_discovery.clone();
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
        .child(div().flex().items_center().child(section_label("DETECTED ON THIS MAC", &p)).child(div().flex_1()).child(
            Button::new("rescan-harnesses").ghost().small().icon(IconName::RefreshCw).label("Scan again")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.harness_discovery = crate::harness_discovery::discover();
                    cx.notify();
                }))
        ))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Import sessions: OpenCode export JSON · Claude/Codex session JSONL · Custom ACP portable dunlin.session.v1 JSON. Histories are copied, not resumed. Existing projects are matched by path; unmatched sessions can be assigned manually."))
        .child(crate::session_import::render(this, cx))
        .child(div().text_size(px(11.5)).text_color(p.muted_fg).child(
            "Detected automatically in PATH and common Homebrew/user install folders. Choose an available ACP adapter as the default for new conversations. CLI-only installations need an adapter; nothing is installed or authorized automatically.",
        ));

    for (name, command, model, tone) in harnesses {
        let selected = current == name;
        let installed = crate::harness::command_available(&command);
        let detected = discovery.iter().find(|entry| entry.name == name);
        let cli_only = !installed && detected.is_some_and(|entry| entry.cli.is_some());
        let found_path = detected
            .and_then(|entry| entry.adapter.as_ref().or(entry.cli.as_ref()))
            .map(|path| path.to_string_lossy().to_string());
        let adapter_docs = detected.filter(|_| cli_only).map(|entry| entry.docs);
        let copy_command = command.clone();
        let live_status = (!this.project().is_remote() && this.session_harness() == name)
            .then_some(&this.harness.status);
        let (status, version) = match (name, live_status) {
            (_, Some(crate::harness::HarnessStatus::Ready { version, .. })) => {
                ("Ready".to_string(), version.clone())
            }
            (_, Some(crate::harness::HarnessStatus::Starting)) => {
                ("Connecting".to_string(), "—".to_string())
            }
            (_, Some(crate::harness::HarnessStatus::Failed(_)))
            | (_, Some(crate::harness::HarnessStatus::Exited(_))) => {
                ("Failed".to_string(), "—".to_string())
            }
            (_, Some(crate::harness::HarnessStatus::Offline)) => {
                ("Offline".to_string(), "—".to_string())
            }
            ("Custom ACP", _) if this.custom_harness_command.trim().is_empty() => {
                ("Not configured".to_string(), "—".to_string())
            }
            (_, _) if installed => ("Installed".to_string(), "—".to_string()),
            (_, _) if cli_only => (
                "CLI found · ACP adapter needed".to_string(),
                "—".to_string(),
            ),
            _ => ("Not installed".to_string(), "—".to_string()),
        };
        let color = tone_color(&p, tone);
        let soft = tone_soft(&p, tone);
        let name_owned = name.to_string();
        let import_name = name.to_string();

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
                    if !installed {
                        this.set_notice("Install an ACP adapter before selecting this harness. Detecting a CLI alone does not confirm ACP support.", cx);
                        return;
                    }
                    this.default_harness = name_owned.clone();
                    this.persist_settings(cx);
                    this.set_notice(
                        format!("Default harness · {name_owned} · applies to new conversations"),
                        cx,
                    );
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
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.fg)
                                        .child(name.to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_family(dunlin_ui::theme::code_font())
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
                                        .text_size(px(10.5))
                                        .font_family(dunlin_ui::theme::code_font())
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
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child("$"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
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
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy_command.clone()));
                                    this.set_notice("ACP command copied", cx);
                                }))
                                .child(
                                    Icon::new(IconName::Copy)
                                        .size(px(9.5))
                                        .text_color(p.muted_fg),
                                ),
                        ),
                )
                .when_some(found_path, |view, path| view.child(div().text_size(px(10.5)).text_color(p.muted_fg).truncate().child(format!("Detected: {path}"))))
                .child(Button::new(format!("import-sessions:{name}")).small().label("Import sessions…").disabled(this.session_import.loading).on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.choose_session_import(&import_name, cx); })))
                .when_some(adapter_docs, |view, url| view.child(
                    Button::new(format!("adapter-setup:{name}")).ghost().small().label("Set up ACP adapter…")
                        .on_click(move |_, _, cx| { cx.stop_propagation(); cx.open_url(url); })
                )),
        );
    }

    body = body
        .when(this.local_opencode(), |body| {
            body.child(section_label("OPENCODE PROVIDER ACCOUNTS", &p))
                .child(super::chat_controls::provider_accounts(this, cx))
        })
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
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
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
                        .text_size(px(11.0))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("Dunlin spawns this command over stdio and speaks ACP. Select Custom ACP above to use it."),
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
                            .text_size(px(11.0))
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
                    .child(div().text_size(px(11.0)).text_color(p.fg).child("Add…"))
                    .into_any_element(),
                &p,
            )),
    );

    body.into_any_element()
}

fn permissions(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
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
                    this.persist_settings(cx);
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
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
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

fn editor(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let word_wrap = this.word_wrap;
    let autosave = this.autosave;

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
                    "Interface font. Geist is bundled; other fonts come from this Mac.",
                    font_picker(this, false, cx),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Code font",
                    "Monospaced face for files, traces and diffs.",
                    font_picker(this, true, cx),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Font size",
                    "Applies to the editor, diffs and the live preview below.",
                    stepper(this, cx, &p),
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
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Autosave",
                    "Write clean edits after a short pause.",
                    toggle_switch(
                        "autosave",
                        autosave,
                        |this| this.autosave = !this.autosave,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "File minimap",
                    "Clickable map on the right. Git: green/amber/red; unsaved edits: blue. Remote files show unsaved edits only.",
                    toggle_switch("editor-minimap", this.editor_minimap, |this| this.editor_minimap = !this.editor_minimap, cx, &p),
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
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child("supervisor.rs"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child("Rust"),
                        ),
                )
                .child(
                    Editor::new(&this.editor_preview)
                        .h(px(210.))
                        .appearance(false)
                        .bordered(false)
                        .readonly(true)
                        .aria_label("Live editor font preview"),
                ),
        );

    body.into_any_element()
}

fn usage(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut usages = Vec::new();
    if let Some(live) = this.session_usage() {
        usages.push(live);
    }
    usages.extend(this.active_usages());

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
                .child(section_label("ACTIVE ACCOUNT & SESSION USAGE", &p))
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
                            this.refresh_usage(cx);
                            this.set_notice("Refreshing provider usage…", cx);
                        }))
                        .child(
                            Icon::new(if this.usage_loading {
                                IconName::LoaderCircle
                            } else {
                                IconName::RefreshCw
                            })
                            .size(px(10.5))
                            .text_color(p.muted_fg),
                        )
                        .child(div().text_size(px(11.0)).text_color(p.fg).child(
                            if this.usage_loading {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            },
                        )),
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
                            .text_size(px(12.5))
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
                                    .text_size(px(10.0))
                                    .font_family(dunlin_ui::theme::code_font())
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
                                    .text_size(px(11.0))
                                    .text_color(p.fg.opacity(0.8))
                                    .child(window.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .child(format!("resets in {}", window.resets_in)),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(dunlin_ui::theme::code_font())
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
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(usage.detail.clone()),
        );
        body = body.child(card);
    }

    body.into_any_element()
}

fn advanced(_this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
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
                    mono_value(&crate::app::data_dir_display(), &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Local storage",
                    "Database, WAL files and cached projections.",
                    mono_value(
                        &crate::app::format_bytes(crate::app::storage_size() as usize),
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "ACP logs",
                    &format!(
                        "Lifecycle and errors · {}",
                        crate::logging::display_path()
                    ),
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
                            let path = crate::logging::log_path();
                            cx.reveal_path(&path);
                            this.set_notice("Revealed the Dunlin log", cx);
                        }))
                        .child(Icon::new(IconName::FolderOpen).size(px(10.)).text_color(p.muted_fg))
                        .child(
                            div()
                                .text_size(px(11.0))
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
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.red)
                                .child("Clear conversation history"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(p.red.opacity(0.8))
                                .child("Delete every session and transcript on this Mac. Projects and settings stay."),
                        ),
                )
                .child(
                    div()
                        .id("clear-history")
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
                            this.request_clear_history(cx);
                        }))
                        .child(Icon::new(IconName::Trash).size(px(10.)).text_color(p.red))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(p.red)
                                .child("Clear…"),
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
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child(format!("Dunlin {}", env!("CARGO_PKG_VERSION"))),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child("Rust · GPUI · ACP · macOS arm64"),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child("MIT"),
                ),
        );

    body.into_any_element()
}

fn remote_settings(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let loading = this.remote_loading;

    let mut body = div()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(section_label("REMOTE SERVERS", &p));

    if this.remote_hosts.is_empty() {
        body = body.child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child("No relay paired yet"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(relative(1.5))
                        .text_color(p.muted_fg)
                        .child("Run `dunlin-relay pair <user>` on the machine that hosts your projects, then enter the code below."),
                ),
        );
    } else {
        for host in this.remote_hosts.clone() {
            let host_id = host.id.clone();
            let host_for_picker = host_id.clone();
            body = body.child(
                div()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(9.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .bg(p.blue_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Cloud).size(px(13.)).text_color(p.blue),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.fg)
                                            .child(host.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_family(dunlin_ui::theme::code_font())
                                            .text_color(p.muted_fg)
                                            .truncate()
                                            .child(format!("{} · {}", host.url, host.user_name)),
                                    ),
                            )
                            .child(
                                div()
                                    .id("remote-add-project")
                                    .h(px(23.))
                                    .px(px(7.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(p.border)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_remote_picker_for(&host_for_picker, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(p.fg)
                                            .child("Add project"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("remote-remove")
                                    .size(px(23.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.red_soft))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_remote_host(&host_id, cx);
                                    }))
                                    .child(
                                        Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg),
                                    ),
                            ),
                    ),
            );
        }
    }

    let field = |label: &'static str, input: AnyElement, p: &Palette| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .text_size(px(11.0))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg)
                    .child(label),
            )
            .child(
                div()
                    .h(px(30.))
                    .px(px(7.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.bg)
                    .flex()
                    .items_center()
                    .child(input),
            )
    };

    body = body.child(section_label("PAIR THIS MAC", &p)).child(
        div()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(field(
                "RELAY URL",
                div()
                    .child(
                        Input::new(&this.remote_url_input)
                            .appearance(false)
                            .bordered(false)
                            .cleanable(true),
                    )
                    .into_any_element(),
                &p,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div().flex_1().child(field(
                            "PAIRING CODE",
                            div()
                                .child(
                                    Input::new(&this.remote_code_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .cleanable(true),
                                )
                                .into_any_element(),
                            &p,
                        )),
                    )
                    .child(
                        div().flex_1().child(field(
                            "SERVER NAME",
                            div()
                                .child(
                                    Input::new(&this.remote_name_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .cleanable(true),
                                )
                                .into_any_element(),
                            &p,
                        )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.muted_fg)
                            .child("The relay must be reachable over Tailscale or HTTPS."),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("remote-pair")
                            .h(px(25.))
                            .px(px(9.))
                            .rounded(px(5.))
                            .bg(p.fg)
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .when(loading, |this| this.opacity(0.6))
                            .on_click(cx.listener(|this, _, _, cx| this.pair_remote(cx)))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.bg)
                                    .child(if loading { "Pairing…" } else { "Pair device" }),
                            ),
                    ),
            ),
    );

    body.into_any_element()
}

fn section_label(label: &str, p: &Palette) -> AnyElement {
    div()
        .text_size(px(11.0))
        .font_family(dunlin_ui::theme::code_font())
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
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
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
    action: fn(&mut DunlinApp),
    cx: &mut Context<DunlinApp>,
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
        .on_click(cx.listener(move |this, _, window, cx| {
            action(this);
            this.apply_editor_preferences(window, cx);
            this.persist_settings(cx);
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

fn font_picker(this: &DunlinApp, code: bool, cx: &mut Context<DunlinApp>) -> AnyElement {
    let selected = if code {
        this.code_font_family.clone()
    } else {
        this.ui_font_family.clone()
    };
    let app = cx.entity().downgrade();
    Button::new(if code {
        "code-font-picker"
    } else {
        "ui-font-picker"
    })
    .ghost()
    .small()
    .label(selected)
    .dropdown_caret(true)
    .max_w(px(230.))
    .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, cx| {
        let Some(entity) = app.upgrade() else {
            return menu;
        };
        let this = entity.read(cx);
        let selected = if code {
            &this.code_font_family
        } else {
            &this.ui_font_family
        };
        menu = menu.label(if code {
            "Installed code fonts"
        } else {
            "Installed interface fonts"
        });
        for family in &this.available_fonts {
            if code && !code_font_candidate(family) && family != selected {
                continue;
            }
            let name = family.clone();
            let action_name = name.clone();
            let app = app.clone();
            menu = menu.item(
                PopupMenuItem::element(move |window, cx| {
                    crate::effects::menu_surface(
                        div()
                            .min_w(px(200.))
                            .max_w(px(300.))
                            .text_size(px(12.))
                            .font_family(name.clone())
                            .text_color(palette(cx).fg)
                            .truncate()
                            .child(name.clone()),
                        window,
                        cx,
                    )
                })
                .checked(family == selected)
                .on_click(move |_, _, cx| {
                    let _ = app.update(cx, |this, cx| this.select_font(&action_name, code, cx));
                }),
            );
        }
        menu.scrollable(true).max_h(px(340.))
    })
    .into_any_element()
}

fn code_font_candidate(family: &str) -> bool {
    let name = family.to_ascii_lowercase();
    name.contains("mono")
        || name == "monaco"
        || name.contains("code")
        || name.contains("courier")
        || name.contains("consolas")
        || name.contains("menlo")
        || name.contains("andale")
        || name.contains("fixed")
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
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .truncate()
                .child(label.to_string()),
        )
        .into_any_element()
}

fn stepper(this: &mut DunlinApp, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let size = this.code_font_size;
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
                    this.adjust_code_font_size(-1.0, cx);
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
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.fg.opacity(0.8))
                .child(format!("{size:.0} px")),
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
                    this.adjust_code_font_size(1.0, cx);
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
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
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
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(fg)
                .child(label),
        )
        .into_any_element()
}

#[cfg(test)]
mod font_tests {
    #[test]
    fn code_menu_recognizes_common_installed_monospace_families() {
        for name in [
            "Geist Mono",
            "Menlo",
            "Monaco",
            "SF Mono",
            "Courier New",
            "JetBrains Mono",
            "Fira Code",
            "Consolas",
            "Andale Mono",
        ] {
            assert!(super::code_font_candidate(name), "{name}");
        }
        for name in ["Geist", "Arial", "Helvetica Neue", "Apple Color Emoji"] {
            assert!(!super::code_font_candidate(name), "{name}");
        }
    }
}
