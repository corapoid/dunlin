//! Settings sheet. Everything that changes how Dunlin connects to a harness,
//! asks for permissions or measures usage lives here.

use dunlin_domain::{MainView, UsageSource};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::ThemeMode;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::{Editor, Input};
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::component::{ActiveTheme, Icon};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};
use dunlin_domain::PermissionMode;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum DesignSection {
    #[default]
    Application,
    Sidebar,
}
impl DesignSection {
    const ALL: [Self; 2] = [Self::Application, Self::Sidebar];
    fn label(self) -> &'static str {
        match self {
            Self::Application => "Application",
            Self::Sidebar => "Sidebar",
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
        .child(crate::effects::rounded_probe(&this.overlay_backdrop, 12.))
        .w(px(width))
        .h(px(height))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.panel)
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
                .rounded_tr(px(12.))
                .rounded_br(px(12.))
                .bg(p.panel)
                .child(settings_header(this, cx))
                .child(
                    div()
                        .id("settings-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(settings_page(match tab {
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
                        })),
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
        .w(px(200.))
        .min_h(px(0.))
        .overflow_hidden()
        .flex_shrink_0()
        .border_r_1()
        .border_color(p.border)
        .bg(p.panel)
        .rounded_tl(px(12.))
        .rounded_bl(px(12.))
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(56.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(8.))
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
                ),
        )
        .child(div().h(px(8.)));

    let groups: [(&str, &[SettingsTab]); 3] = [
        (
            "Workspace",
            &[
                SettingsTab::General,
                SettingsTab::Design,
                SettingsTab::Chat,
                SettingsTab::Editor,
            ],
        ),
        (
            "Agent",
            &[
                SettingsTab::Harnesses,
                SettingsTab::Skills,
                SettingsTab::Permissions,
            ],
        ),
        (
            "System",
            &[
                SettingsTab::Remote,
                SettingsTab::Usage,
                SettingsTab::Advanced,
            ],
        ),
    ];
    for (group, tabs) in groups {
        nav = nav.child(
            div()
                .h(px(26.))
                .px(px(18.))
                .flex()
                .items_end()
                .pb(px(5.))
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted_fg.opacity(0.85))
                .child(group),
        );
        for tab in tabs {
            let active = this.settings_tab == *tab;
            nav = nav.child(
                div()
                    .id(format!("settings-tab:{}", tab.label()))
                    .relative()
                    .mx(px(8.))
                    .h(px(28.))
                    .px(px(11.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(active, |this| this.bg(p.card))
                    .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_tab = *tab;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .absolute()
                            .left(px(0.))
                            .top(px(7.))
                            .bottom(px(7.))
                            .w(px(2.))
                            .rounded_full()
                            .bg(if active { p.fg } else { p.bg.opacity(0.0) }),
                    )
                    .child(Icon::new(tab.icon()).size(px(13.)).text_color(if active {
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
        nav = nav.child(div().h(px(4.)));
    }

    nav.child(div().flex_1())
        .child(
            div()
                .px(px(18.))
                .pb(px(14.))
                .text_size(px(10.5))
                .line_height(relative(1.45))
                .text_color(p.muted_fg.opacity(0.75))
                .child("Local-first · sessions and data stay on this Mac"),
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
            "Theme, interface density and the project sidebar.",
        ),
        SettingsTab::Chat => (
            "Chat",
            "Message appearance, timestamps and response details.",
        ),
        SettingsTab::Harnesses => (
            "Harnesses",
            "Connect Dunlin to ACP agents and choose which one new sessions use.",
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
        .h(px(56.))
        .flex_shrink_0()
        .rounded_tr(px(12.))
        .px(px(24.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .bg(p.panel)
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
                .size(px(28.))
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
                .child(Icon::new(IconName::X).size(px(14.)).text_color(p.muted_fg)),
        )
        .into_any_element()
}

fn general(this: &mut DunlinApp, _window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let view = this.default_view;
    let harness = this.default_harness.clone();

    let mut startup = segment_group(&p);
    for (dashboard, label) in [(true, "Dashboard"), (false, "Last project")] {
        let active = this.dashboard_settings.start_at_dashboard == dashboard;
        startup = startup.child(
            segment(
                format!("startup-destination:{dashboard}"),
                label,
                active,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.dashboard_settings.start_at_dashboard = dashboard;
                this.persist_settings(cx);
                cx.notify();
            })),
        );
    }

    let mut views = segment_group(&p);
    for candidate in MainView::ALL {
        views = views.child(
            segment(
                format!("startup-view:{}", candidate.label()),
                candidate.label(),
                candidate == view,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.default_view = candidate;
                this.persist_settings(cx);
                cx.notify();
            })),
        );
    }

    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Startup", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "Startup destination",
                    "Choose Dashboard or your last project. Conversation history is preserved.",
                    startup.into_any_element(),
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
                .child(setting_row(
                    "Dashboard live agents",
                    "Show working, waiting and connecting agents.",
                    toggle_switch(
                        "dashboard-agents",
                        this.dashboard_settings.show_agents,
                        |this| this.dashboard_settings.show_agents = !this.dashboard_settings.show_agents,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Dashboard activity",
                    "Show recorded metrics and the configurable heatmap.",
                    toggle_switch(
                        "dashboard-activity",
                        this.dashboard_settings.show_activity,
                        |this| this.dashboard_settings.show_activity = !this.dashboard_settings.show_activity,
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Default view",
                    "Where a new session opens.",
                    views.into_any_element(),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Default harness",
                    "New sessions connect through this ACP adapter.",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(harness),
                        )
                        .child(
                            settings_button("general-change-harness", "Change…", false, &p)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings_tab = SettingsTab::Harnesses;
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                    &p,
                )),
        )
        .child(section_label("Behavior", &p))
        .child(
            settings_card(&p)
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
                .child(notification_permission_row(this, cx, &p))
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
        )
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    Icon::new(IconName::ShieldCheck)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.0))
                        .line_height(relative(1.5))
                        .text_color(p.muted_fg)
                        .child("Dunlin never uploads project contents. Harness adapters run as local child processes and speak ACP over stdio."),
                ),
        );

    body.into_any_element()
}

fn design(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let selected = this.design_section;
    let mut tabs = segment_group(&p);
    for section in DesignSection::ALL {
        tabs = tabs.child(
            segment(
                format!("settings-design:{}", section.label()),
                section.label(),
                section == selected,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.design_section = section;
                cx.notify();
            })),
        );
    }
    let content = match selected {
        DesignSection::Application => {
            let dark = cx.theme().is_dark();
            let mut density = segment_group(&p);
            for (value, label) in [
                (crate::app::UiDensity::Comfortable, "Comfortable"),
                (crate::app::UiDensity::Compact, "Compact"),
            ] {
                density = density.child(
                    segment(format!("density:{label}"), label, this.density == value, &p).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.density = value;
                            dunlin_ui::scale::set_density(match value {
                                crate::app::UiDensity::Compact => {
                                    dunlin_ui::scale::Density::Compact
                                }
                                crate::app::UiDensity::Comfortable => {
                                    dunlin_ui::scale::Density::Comfortable
                                }
                            });
                            this.persist_settings(cx);
                            cx.notify();
                        }),
                    ),
                );
            }
            div()
                .px(px(24.))
                .py(px(20.))
                .flex()
                .flex_col()
                .gap(px(22.))
                .child(section_label("Theme", &p))
                .child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(theme_card(false, !dark, window, cx, &p))
                        .child(theme_card(true, dark, window, cx, &p)),
                )
                .child(section_label("Interface", &p))
                .child(
                    settings_card(&p)
                        .child(setting_row(
                            "UI font",
                            "Interface typography. Agent and editor configuration have dedicated sections above.",
                            font_picker(this, false, cx),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Interface density",
                            "Compact tightens semantic spacing without shrinking hit targets. Legacy views with fixed sizes migrate incrementally.",
                            density.into_any_element(),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Show file sizes",
                            "Show file and folder sizes in Explorer tooltips, the project total and Dashboard tiles.",
                            toggle_switch(
                                "show-file-sizes",
                                this.show_file_sizes,
                                |this| this.show_file_sizes = !this.show_file_sizes,
                                cx,
                                &p,
                            ),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Reduce motion",
                            "Disable decorative graph/trace entrance and pulse animations. Status remains visible without motion.",
                            toggle_switch(
                                "reduce-motion",
                                this.reduce_motion,
                                |this| this.reduce_motion = !this.reduce_motion,
                                cx,
                                &p,
                            ),
                            &p,
                        ))
                )
                .into_any_element()
        }
        DesignSection::Sidebar => {
            let mut view = segment_group(&p);
            for (candidate, label) in [
                (crate::project_tree::View::List, "List"),
                (crate::project_tree::View::Tree, "Tree"),
            ] {
                view = view.child(
                    segment(
                        format!("design-view:{label}"),
                        label,
                        this.project_view == candidate,
                        &p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_view = candidate;
                        this.persist_settings(cx);
                        cx.notify();
                    })),
                );
            }
            let mut sort = segment_group(&p);
            for (candidate, label) in [
                (crate::project_tree::Sort::Name, "Name ↑"),
                (crate::project_tree::Sort::Modified, "Modified ↓"),
            ] {
                sort = sort.child(
                    segment(
                        format!("design-sort:{label}"),
                        label,
                        this.project_sort == candidate,
                        &p,
                    )
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
                    settings_button(format!("design-width:{label}"), label, false, &p).on_click(
                        cx.listener(move |this, _, window, cx| {
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
                        }),
                    ),
                );
            }
            div()
                .px(px(24.))
                .py(px(20.))
                .flex()
                .flex_col()
                .gap(px(22.))
                .child(section_label("Project list", &p))
                .child(
                    settings_card(&p)
                        .child(setting_row(
                            "Show branch names",
                            "Show or hide branch/Git labels below project names. Dirty-change indicators remain visible.",
                            toggle_switch(
                                "project-branches",
                                this.show_project_branches,
                                |this| this.show_project_branches = !this.show_project_branches,
                                cx,
                                &p,
                            ),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "View",
                            "List retains group breadcrumbs; Tree shows the hierarchy.",
                            view.into_any_element(),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Sort",
                            "Pinned projects stay first. Modification dates use cached repository metadata.",
                            sort.into_any_element(),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Width",
                            &format!(
                                "{} points. You can also drag the sidebar edge; double-click to reset.",
                                this.sidebar_width.round()
                            ),
                            width.into_any_element(),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                            "Collapse account usage",
                            "Start with the compact account usage footer.",
                            toggle_switch(
                                "design-usage-collapse",
                                this.account_usage_collapsed,
                                |this| this.account_usage_collapsed = !this.account_usage_collapsed,
                                cx,
                                &p,
                            ),
                            &p,
                        )),
                )
                .into_any_element()
        }
    };
    div()
        .flex()
        .flex_col()
        .child(div().px(px(24.)).pt(px(18.)).flex().child(tabs))
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
    let mut card = settings_card(&p);
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
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Chat appearance", &p))
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
            // Refreshing every window during an appearance change must not
            // hand key-window status back to the application behind Settings.
            window.activate_window();
            this.persist_settings(cx);
            cx.notify();
        }))
        .child(
            div()
                .h(px(72.))
                .rounded_tl(px(8.))
                .rounded_tr(px(8.))
                .bg(preview.bg)
                .flex()
                .child(
                    div()
                        .w(px(40.))
                        .h_full()
                        .rounded_tl(px(8.))
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
                    this.child(Icon::new(IconName::Check).size(px(12.)).text_color(p.fg))
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
    let entries = crate::harness_discovery::catalog_names()
        .map(|name| (name, crate::harness::harness_command(name, "")))
        .chain(std::iter::once(("Custom ACP", custom_command)))
        .collect::<Vec<_>>();

    let mut list = settings_card(&p);
    for (index, (name, command)) in entries.iter().enumerate() {
        let selected = current == *name;
        let installed = crate::harness::command_available(command);
        let detected = discovery.iter().find(|entry| entry.name == *name);
        let cli_only = !installed && detected.is_some_and(|entry| entry.cli.is_some());
        let docs = crate::harness_discovery::catalog_docs(name)
            .or_else(|| detected.map(|entry| entry.docs));
        let live_status = (!this.project().is_remote() && this.session_harness() == *name)
            .then_some(&this.harness.status);
        let (status, status_color) = match live_status {
            Some(crate::harness::HarnessStatus::Ready { .. }) => ("Ready", p.green),
            Some(crate::harness::HarnessStatus::Starting) => ("Connecting", p.amber),
            Some(crate::harness::HarnessStatus::Failed(_))
            | Some(crate::harness::HarnessStatus::Exited(_)) => ("Failed", p.red),
            Some(crate::harness::HarnessStatus::Offline) => ("Offline", p.muted_fg),
            None if *name == "Custom ACP" && this.custom_harness_command.trim().is_empty() => {
                ("Not configured", p.muted_fg)
            }
            None if installed => ("Installed", p.muted_fg),
            None if cli_only => ("Adapter needed", p.amber),
            None => ("Not installed", p.muted_fg),
        };
        let name_owned = name.to_string();
        let import_name = name.to_string();
        let install_name = name.to_string();
        let scanning =
            this.session_import.loading && this.session_import.scanning.as_deref() == Some(*name);

        let action = if installed {
            settings_button(
                format!("import-sessions:{name}"),
                if scanning {
                    "Scanning…"
                } else {
                    "Import sessions…"
                },
                false,
                &p,
            )
            .when(this.session_import.loading, |this| this.opacity(0.5))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                if !this.session_import.loading {
                    this.scan_session_import(&import_name, cx);
                }
            }))
            .into_any_element()
        } else if let Some(docs) = docs {
            settings_button(format!("install-harness:{name}"), "Install…", false, &p)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_notice(
                        format!("Opened installation instructions for {install_name}"),
                        cx,
                    );
                    cx.open_url(docs);
                }))
                .into_any_element()
        } else {
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("Configure below")
                .into_any_element()
        };

        let mut row = div()
            .id(format!("harness-row:{name}"))
            .h(px(54.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(index > 0, |this| this.border_t_1().border_color(p.border))
            .child(radio(selected, &p))
            .child(
                div()
                    .size(px(28.))
                    .flex_shrink_0()
                    .rounded(px(7.))
                    .bg(p.muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(if *name == "Custom ACP" {
                            IconName::Terminal
                        } else {
                            IconName::Bot
                        })
                        .size(px(13.5))
                        .text_color(p.muted_fg),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.fg)
                            .child(name.to_string()),
                    )
                    .when(selected, |this| this.child(settings_chip("Default", &p))),
            )
            .child(
                div()
                    .w(px(102.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(dot(status_color, 5.0))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(status),
                    ),
            )
            .child(
                div()
                    .w(px(124.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .child(action),
            );
        if installed {
            row = row
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.default_harness = name_owned.clone();
                    this.persist_settings(cx);
                    this.set_notice(
                        format!("Default harness · {name_owned} · applies to new conversations"),
                        cx,
                    );
                }));
        }
        list = list.child(row);
    }

    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(20.))
        .child(
            div()
                .flex()
                .items_center()
                .child(section_label("ACP harnesses", &p))
                .child(div().flex_1())
                .child(
                    settings_button("rescan-harnesses", "Scan again", false, &p).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.harness_discovery = crate::harness_discovery::discover();
                            cx.notify();
                        }),
                    ),
                ),
        )
        .child(
            div()
                .text_size(px(11.0))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("ACP agents run locally over stdio. Install a harness, then import its existing sessions. Nothing is installed or authorized automatically."),
        )
        .child(list)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    settings_text_button("browse-acp-agents", "Browse all ACP agents", p.fg, &p)
                        .on_click(|_, _, cx| {
                            cx.open_url("https://agentclientprotocol.com/get-started/agents");
                        }),
                )
                .child(
                    Icon::new(IconName::ExternalLink)
                        .size(px(11.))
                        .text_color(p.muted_fg),
                ),
        )
        .child(
            div()
                .text_size(px(11.0))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("Import sessions scans the standard Claude Code and Codex session folders and reads OpenCode's local session database read-only, then shows a preview. Portable dunlin.session.v1 JSON is still available separately. Histories are copied, not resumed."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    settings_text_button(
                        "import-session-portable",
                        "Import a portable file…",
                        p.muted_fg,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        let harness = this.default_harness.clone();
                        this.choose_session_import(&harness, cx);
                    })),
                ),
        )
        .child(crate::session_import::render(this, cx))
        .when(this.local_opencode(), |body| {
            body.child(section_label("OpenCode provider accounts", &p))
                .child(super::chat_controls::provider_accounts(this, cx))
        })
        .child(section_label("Custom ACP command", &p))
        .child(
            settings_card(&p)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(9.))
                .child(
                    div()
                        .h(px(32.))
                        .px(px(9.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.bg)
                        .flex()
                        .items_center()
                        .gap(px(6.))
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
                                    .h(px(30.)),
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
        )
        .child({
            let connected = this.harness.harness.is_some();
            let connection_detail = this
                .harness
                .status
                .detail()
                .unwrap_or_else(|| this.harness.status.label());
            let connect_label = if connected { "Restart" } else { "Connect" };
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section_label("Connection", &p))
                .child(
                    settings_card(&p)
                        .child(setting_row(
                            "Connection",
                            &connection_detail,
                            settings_button("harness-connect", connect_label, false, &p)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.harness.harness.is_some() {
                                        this.restart_harness(cx);
                                    } else {
                                        this.connect_harness(cx);
                                    }
                                }))
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
                            settings_button("add-custom-harness", "Add…", false, &p)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_notice(
                                        "Custom ACP · enter a command and environment",
                                        cx,
                                    );
                                }))
                                .into_any_element(),
                            &p,
                        )),
                )
                .into_any_element()
        });

    body.into_any_element()
}

fn permissions(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let current = this.permission_mode;
    let modes = [
        (
            PermissionMode::Ask,
            "Ask every time",
            "Dunlin does not auto-answer. The harness decides which actions request permission.",
        ),
        (
            PermissionMode::ReadOnly,
            "Read-only",
            "Dunlin rejects ACP allow requests. This is not a filesystem sandbox; a harness that does not ask is not constrained.",
        ),
        (
            PermissionMode::AutoApprove,
            "Auto-approve",
            "Dunlin approves ACP permission requests automatically. Use only in trusted, disposable projects.",
        ),
    ];

    let mut choices = div().flex().flex_col().gap(px(8.));
    for (mode, title, description) in modes {
        let selected = current == mode;
        let risky = mode == PermissionMode::AutoApprove;
        choices = choices.child(
            div()
                .id(format!("permission-mode:{title}"))
                .rounded(px(10.))
                .border_1()
                .border_color(if selected { p.border_strong } else { p.border })
                .bg(p.card)
                .p(px(14.))
                .flex()
                .items_start()
                .gap(px(12.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.subtle))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.permission_mode = mode;
                    this.persist_settings(cx);
                    this.set_notice(format!("Permission mode · {}", mode.label()), cx);
                }))
                .child(radio(selected, &p))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.fg)
                                        .child(title),
                                )
                                .when(risky, |this| this.child(risk_chip(&p))),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(relative(1.5))
                                .text_color(p.muted_fg)
                                .child(description),
                        ),
                )
                .when(selected, |this| this.child(settings_chip("Current", &p))),
        );
    }

    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Default permission mode", &p))
        .child(choices)
        .child(section_label("Scope", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "Workspace writes",
                    "Dunlin's editor can write inside the project. Agent writes depend on the harness and its permission prompts; this is not a filesystem sandbox.",
                    scope_value("Editor / harness", p.muted_fg, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Home directory",
                    "Local harnesses run with your user permissions. The relay's seatbelt denies common paths but is a best-effort boundary, not a container.",
                    scope_value("Not enforced", p.red, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Network access",
                    "The harness can use the network. Dunlin does not currently enforce a network sandbox.",
                    scope_value("Not enforced", p.red, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Secrets",
                    "Tool output and logs are not redacted. Do not paste secrets; inspect content before sharing.",
                    scope_value("Not redacted", p.red, &p),
                    &p,
                )),
        )
        .child(section_label("macOS permission center", &p))
        .child({
            let mut card = settings_card(&p);
            let statuses = crate::platform::permission_statuses();
            let count = statuses.len();
            for (index, status) in statuses.into_iter().enumerate() {
                let color = match status.state {
                    crate::platform::PermissionState::Granted => p.green,
                    crate::platform::PermissionState::Denied => p.red,
                    crate::platform::PermissionState::NotDetermined => p.amber,
                    crate::platform::PermissionState::NotGranted => p.amber,
                    crate::platform::PermissionState::Unknown => p.muted_fg,
                };
                let kind = status.kind;
                card = card.child(
                    div()
                        .id(format!("permission-status:{}", kind.label()))
                        .px(px(14.))
                        .py(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .w(px(112.))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(kind.label()),
                        )
                        .child(
                            div()
                                .w(px(84.))
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(dot(color, 5.0))
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.muted_fg)
                                        .child(status.state.label()),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(10.5))
                                .line_height(relative(1.4))
                                .text_color(p.muted_fg)
                                .child(status.detail),
                        )
                        .child(
                            settings_text_button(
                                format!("permission-open:{}", index),
                                "Open Settings",
                                p.fg,
                                &p,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match crate::platform::open_permission_settings(kind) {
                                    Ok(()) => this.set_notice(
                                        format!("Opened {} settings", kind.label()),
                                        cx,
                                    ),
                                    Err(error) => this.set_notice(error, cx),
                                }
                            })),
                        )
                        .child(
                            settings_text_button(
                                format!("permission-recheck:{}", index),
                                "Recheck",
                                p.muted_fg,
                                &p,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let statuses = crate::platform::refresh_permission_statuses();
                                this.set_notice(
                                    format!(
                                        "Permission status rechecked ({} entries); no prompt was requested",
                                        statuses.len()
                                    ),
                                    cx,
                                );
                                cx.notify();
                            })),
                        ),
                );
                if index + 1 < count {
                    card = card.child(divider(&p));
                }
            }
            card.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(p.border)
                    .text_size(px(10.5))
                    .line_height(relative(1.45))
                    .text_color(p.muted_fg)
                    .child("Dunlin never grants TCC permissions itself. Status comes from macOS APIs; Automation stays unknown until the system prompt is shown for a target app."),
            )
        });

    body.into_any_element()
}

fn editor(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let word_wrap = this.word_wrap;
    let autosave = this.autosave;

    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Typography", &p))
        .child(
            settings_card(&p)
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
        .child(section_label("Preview", &p))
        .child(
            settings_card(&p)
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
                                .text_color(p.muted_fg),
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
    this.ensure_billing(cx);
    let mut usages = Vec::new();
    if let Some(live) = this.session_usage() {
        usages.push(live);
    }
    usages.extend(this.active_usages());

    let mut body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(
            div()
                .flex()
                .items_center()
                .child(section_label("Subscription limits", &p))
                .child(div().flex_1())
                .child(
                    settings_button("refresh-usage", "Refresh", false, &p).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.refresh_usage(cx);
                            this.set_notice("Refreshing provider usage…", cx);
                        },
                    )),
                ),
        );

    for usage in usages {
        let mut card = settings_card(&p)
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(7.))
                            .bg(p.muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Gauge)
                                    .size(px(14.))
                                    .text_color(p.muted_fg),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(usage.provider.clone()),
                    )
                    .child(settings_chip(usage.plan.clone(), &p))
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
                    .gap(px(5.))
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
                            .h(px(4.))
                            .w_full()
                            .rounded_full()
                            .bg(p.muted)
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(percent / 100.0))
                                    .rounded_full()
                                    .bg(if percent > 85. {
                                        p.red
                                    } else {
                                        p.fg.opacity(0.65)
                                    }),
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

    body = body.child(billing_usage(this, cx, &p));

    body.into_any_element()
}

fn billing_usage(this: &mut DunlinApp, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let mut body = div()
        .pt(px(6.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .child(section_label("Local billing", p))
                .child(div().flex_1())
                .child(
                    settings_button(
                        "refresh-billing",
                        if this.billing_loading {
                            "Scanning…"
                        } else {
                            "Scan logs"
                        },
                        false,
                        p,
                    )
                    .when(this.billing_loading, |this| this.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.billing_loading {
                            this.refresh_billing(cx);
                        }
                    })),
                ),
        )
        .child(
            div()
                .text_size(px(10.5))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("Read-only, bounded scan of local Claude Code, Codex and OpenCode logs. Deduped; unknown models stay unpriced and are never counted as $0."),
        );
    if let Some(error) = &this.billing_error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    let Some(summary) = this.billing.clone() else {
        return body
            .child(
                div()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .p(px(12.))
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(if this.billing_loading {
                        "Scanning provider logs…"
                    } else {
                        "No scan yet. Scan logs to build the local billing summary."
                    }),
            )
            .into_any_element();
    };
    let totals = &summary.totals;
    let unpriced = totals.unpriced_events > 0;
    let mut summary_card = settings_card(p)
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(7.))
                        .bg(p.muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::ChartNoAxesColumn)
                                .size(px(14.))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child("Provider billing evidence"),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(p.muted_fg)
                                .child(format!(
                                    "{} priced events · {} unpriced · {} files / {}",
                                    totals.events.saturating_sub(totals.unpriced_events),
                                    totals.unpriced_events,
                                    summary.files,
                                    crate::app::format_bytes(
                                        summary.bytes.min(usize::MAX as u64) as usize
                                    ),
                                )),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(16.))
                        .font_family(dunlin_ui::theme::code_font())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if unpriced { p.amber } else { p.fg })
                        .child(totals.cost_label()),
                ),
        )
        .child(
            div().flex().flex_wrap().gap(px(8.)).children([
                usage_metric(
                    "Month to date",
                    format!("${:.2}", summary.month_to_date_cost()),
                    "known priced spend",
                    p.fg,
                    p,
                ),
                usage_metric(
                    "Tokens",
                    crate::app::format_tokens(totals.tokens()),
                    &format!(
                        "{} in · {} out",
                        crate::app::format_tokens(totals.input_tokens),
                        crate::app::format_tokens(totals.output_tokens)
                    ),
                    p.fg,
                    p,
                ),
                usage_metric(
                    "Cache reuse",
                    totals
                        .cache_reuse_percent()
                        .map(|percent| format!("{percent:.0}%"))
                        .unwrap_or_else(|| "unknown".into()),
                    &format!(
                        "{} cache read",
                        crate::app::format_tokens(totals.cache_read_tokens)
                    ),
                    p.fg,
                    p,
                ),
                usage_metric(
                    "Unpriced",
                    totals.unpriced_events.to_string(),
                    if unpriced {
                        "excluded from known cost"
                    } else {
                        "all events priced"
                    },
                    if unpriced { p.amber } else { p.green },
                    p,
                ),
            ]),
        );
    let mut budget_row = div().flex().items_center().gap(px(5.));
    budget_row = budget_row.child(
        div()
            .text_size(px(10.5))
            .text_color(p.muted_fg)
            .child("Alert budget"),
    );
    for (label, value) in [
        ("Off", None),
        ("$50", Some(50.)),
        ("$100", Some(100.)),
        ("$250", Some(250.)),
    ] {
        let active = match (value, this.billing_budget_usd) {
            (None, None) => true,
            (Some(value), Some(current)) => (value - current).abs() < f32::EPSILON,
            _ => false,
        };
        budget_row = budget_row.child(
            div()
                .id(format!("billing-budget:{label}"))
                .h(px(19.))
                .px(px(6.))
                .rounded(px(4.))
                .border_1()
                .border_color(if active {
                    p.border_strong
                } else {
                    p.border.opacity(0.)
                })
                .bg(if active { p.subtle } else { p.card.opacity(0.) })
                .flex()
                .items_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.billing_budget_usd = value;
                    this.persist_settings(cx);
                    this.set_notice(
                        value
                            .map(|value| format!("Billing alert budget · ${value:.0}"))
                            .unwrap_or_else(|| "Billing budget alerts off".into()),
                        cx,
                    );
                }))
                .child(
                    div()
                        .text_size(px(10.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(if active { p.fg } else { p.muted_fg })
                        .child(label),
                ),
        );
    }
    budget_row = budget_row.child(div().flex_1()).child(
        settings_button(
            if this.billing_alerts {
                "billing-alerts-off"
            } else {
                "billing-alerts-on"
            },
            if this.billing_alerts {
                "Alerts on"
            } else {
                "Alerts off"
            },
            false,
            p,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.billing_alerts = !this.billing_alerts;
            this.persist_settings(cx);
            cx.notify();
        })),
    );
    summary_card = summary_card.child(budget_row);
    body = body.child(summary_card);
    for alert in summary.alerts(if this.billing_alerts {
        this.billing_budget()
    } else {
        None
    }) {
        let (color, level) = match alert.level {
            dunlin_usage::billing::AlertLevel::Info => (p.blue, "INFO"),
            dunlin_usage::billing::AlertLevel::Warning => (p.amber, "WARN"),
            dunlin_usage::billing::AlertLevel::Critical => (p.red, "ALERT"),
        };
        body = body.child(
            div()
                .rounded(px(7.))
                .border_1()
                .border_color(color.opacity(0.25))
                .bg(p.card)
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_start()
                .gap(px(7.))
                .child(
                    div()
                        .text_size(px(9.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(color)
                        .child(level),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .line_height(relative(1.4))
                        .text_color(p.fg)
                        .child(alert.message),
                ),
        );
    }
    body = body.child(
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(10.))
            .child(billing_slices("By harness", &summary.by_harness, p))
            .child(billing_slices("By provider", &summary.by_provider, p))
            .child(billing_slices("By model", &summary.by_model, p))
            .child(billing_slices("By project", &summary.by_project, p)),
    );
    body = body.child(billing_days(&summary.by_day, p));
    body.into_any_element()
}

fn usage_metric(label: &str, value: String, note: &str, color: Hsla, p: &Palette) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(132.))
        .rounded(px(6.))
        .bg(p.subtle)
        .px(px(9.))
        .py(px(7.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(
            div()
                .text_size(px(9.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(value),
        )
        .child(
            div()
                .text_size(px(9.5))
                .text_color(p.muted_fg.opacity(0.8))
                .truncate()
                .child(note.to_string()),
        )
        .into_any_element()
}

fn project_short_label(label: &str) -> String {
    let trimmed = label.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return label.to_string();
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

fn billing_slices(title: &str, slices: &[dunlin_usage::billing::Slice], p: &Palette) -> AnyElement {
    let mut card = div()
        .flex_1()
        .min_w(px(300.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .overflow_hidden()
        .child(
            div()
                .px(px(9.))
                .py(px(6.))
                .border_b_1()
                .border_color(p.border)
                .bg(p.subtle)
                .text_size(px(10.))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child(title.to_string()),
        );
    if slices.is_empty() {
        card = card.child(
            div()
                .p(px(8.))
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("No events."),
        );
    }
    for slice in slices.iter().take(8) {
        let label = if title == "By project" {
            project_short_label(&slice.label)
        } else {
            slice.label.clone()
        };
        card = card.child(
            div()
                .px(px(9.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(7.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(p.fg)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{} ev · {} tok",
                            slice.totals.events,
                            crate::app::format_tokens(slice.totals.tokens())
                        )),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(if slice.totals.unpriced_events > 0 {
                            p.amber
                        } else {
                            p.fg
                        })
                        .child(slice.totals.cost_label()),
                ),
        );
    }
    card.into_any_element()
}

fn billing_days(days: &[dunlin_usage::billing::Day], p: &Palette) -> AnyElement {
    let mut card = div()
        .w_full()
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .overflow_hidden()
        .child(
            div()
                .px(px(9.))
                .py(px(6.))
                .border_b_1()
                .border_color(p.border)
                .bg(p.subtle)
                .text_size(px(10.))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child("Last 14 days"),
        );
    if days.is_empty() {
        card = card.child(
            div()
                .p(px(8.))
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("No dated events."),
        );
    }
    for day in days.iter().take(14) {
        card = card.child(
            div()
                .px(px(9.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(82.))
                        .text_size(px(10.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(day.date.clone()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{} ev · {} tok",
                            day.totals.events,
                            crate::app::format_tokens(day.totals.tokens())
                        )),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(if day.totals.unpriced_events > 0 {
                            p.amber
                        } else {
                            p.fg
                        })
                        .child(day.totals.cost_label()),
                ),
        );
    }
    card.into_any_element()
}

fn advanced(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);

    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Local data", &p))
        .child(
            settings_card(&p)
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
                    settings_button(
                        "view-logs",
                        if this.acp_log_loading {
                            "Loading…"
                        } else {
                            "View log"
                        },
                        false,
                        &p,
                    )
                    .when(this.acp_log_loading, |this| this.opacity(0.55))
                    .on_click(cx.listener(|this, _, _, cx| this.open_acp_log(cx)))
                    .into_any_element(),
                    &p,
                )),
        )
        .when_some(this.acp_log_error.clone(), |body, error| {
            body.child(
                div()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.red.opacity(0.25))
                    .bg(p.card)
                    .p(px(12.))
                    .text_size(px(11.0))
                    .text_color(p.red)
                    .child(error),
            )
        })
        .when_some(this.acp_log_text.clone(), |body, text| {
            body.child(
                settings_card(&p)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .child("ACP log · last 256 KiB"),
                            )
                            .child(
                                settings_text_button(
                                    "close-acp-log",
                                    "Close",
                                    p.muted_fg,
                                    &p,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.acp_log_text = None;
                                    cx.notify();
                                })),
                            ),
                    )
                    .child(
                        div()
                            .id("acp-log-view")
                            .h(px(360.))
                            .overflow_y_scroll()
                            .rounded(px(6.))
                            .bg(p.subtle)
                            .p(px(10.))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .line_height(relative(1.35))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.fg)
                                    .child(text),
                            ),
                    ),
            )
        })
        .child(section_label("Release QA", &p))
        .child(
            settings_card(&p)
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child("Environment report"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(relative(1.45))
                                .text_color(p.muted_fg)
                                .child("Automatic facts for the R11 manual matrix: data directory, database schema/integrity, projects, sessions, harness discovery, permissions and remote hosts. It never claims manual TCC or GUI acceptance."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            settings_button("qa-run", "Run QA checks", false, &p).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.qa_report = Some(crate::qa::capture(this));
                                    this.qa_error = None;
                                    this.set_notice("QA report ready", cx);
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(
                            settings_button("qa-copy", "Copy report", false, &p).on_click(
                                cx.listener(|this, _, _, cx| {
                                    let report = this
                                        .qa_report
                                        .clone()
                                        .unwrap_or_else(|| crate::qa::capture(this));
                                    cx.write_to_clipboard(ClipboardItem::new_string(report.clone()));
                                    this.qa_report = Some(report);
                                    this.qa_error = None;
                                    this.set_notice("Copied the QA report", cx);
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .when_some(this.qa_error.clone(), |card, error| {
                    card.child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.red)
                            .child(error),
                    )
                })
                .when_some(this.qa_report.clone(), |card, report| {
                    card.child(
                        div()
                            .id("qa-report-view")
                            .h(px(260.))
                            .overflow_y_scroll()
                            .rounded(px(6.))
                            .bg(p.subtle)
                            .p(px(10.))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .line_height(relative(1.35))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.fg)
                                    .child(report),
                            ),
                    )
                }),
        )
        .child(section_label("Recovery", &p))
        .child(
            settings_card(&p)
                .p(px(14.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child("Clear conversation history"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(relative(1.45))
                                .text_color(p.muted_fg)
                                .child("Delete every session and transcript on this Mac. Projects and settings stay."),
                        ),
                )
                .child(
                    settings_text_button("clear-history", "Clear…", p.red, &p)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request_clear_history(cx);
                        })),
                ),
        )
        .child(section_label("About", &p))
        .child(
            settings_card(&p)
                .p(px(14.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(7.))
                        .bg(p.fg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Bird).size(px(14.)).text_color(p.bg)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
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
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(section_label("Remote servers", &p));

    if this.remote_hosts.is_empty() {
        body = body.child(
            settings_card(&p)
                .p(px(16.))
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
                settings_card(&p).child(
                    div()
                        .px(px(14.))
                        .py(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .size(px(30.))
                                .rounded(px(7.))
                                .bg(p.muted)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Cloud)
                                        .size(px(14.))
                                        .text_color(p.muted_fg),
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
                            settings_button("remote-add-project", "Add project", false, &p)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_remote_picker_for(&host_for_picker, cx);
                                })),
                        )
                        .child(
                            div()
                                .id("remote-remove")
                                .size(px(28.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(p.hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_remote_host(&host_id, cx);
                                }))
                                .child(Icon::new(IconName::X).size(px(12.)).text_color(p.muted_fg)),
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
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_fg)
                    .child(label),
            )
            .child(
                div()
                    .h(px(32.))
                    .px(px(9.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.bg)
                    .flex()
                    .items_center()
                    .child(input),
            )
    };

    body = body.child(section_label("Pair this Mac", &p)).child(
        settings_card(&p)
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(field(
                "Relay URL",
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
                            "Pairing code",
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
                            "Server name",
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
                        settings_button(
                            "remote-pair",
                            if loading { "Pairing…" } else { "Pair device" },
                            true,
                            &p,
                        )
                        .when(loading, |this| this.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| this.pair_remote(cx))),
                    ),
            ),
    );

    body.into_any_element()
}

impl DunlinApp {
    pub(crate) fn open_acp_log(&mut self, cx: &mut Context<Self>) {
        if self.acp_log_loading {
            return;
        }
        self.acp_log_loading = true;
        self.acp_log_error = None;
        self.acp_log_text = None;
        cx.notify();
        let path = crate::logging::log_path();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    use std::io::{Read, Seek, SeekFrom};
                    let mut file = std::fs::File::open(&path)
                        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
                    let len = file.metadata().map_err(|error| error.to_string())?.len();
                    let start = len.saturating_sub(256 * 1024);
                    file.seek(SeekFrom::Start(start))
                        .map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(256 * 1024)
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if text.trim().is_empty() {
                        Err("The ACP log is empty.".to_string())
                    } else {
                        Ok(text)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.acp_log_loading = false;
                match result {
                    Ok(text) => this.acp_log_text = Some(text),
                    Err(error) => this.acp_log_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn settings_page(content: AnyElement) -> AnyElement {
    div()
        .w_full()
        .flex()
        .child(div().w_full().max_w(px(760.)).child(content))
        .into_any_element()
}

fn section_label(label: &str, p: &Palette) -> AnyElement {
    div()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted_fg)
        .child(label.to_string())
        .into_any_element()
}

fn settings_card(p: &Palette) -> Div {
    div()
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
}

fn settings_chip(label: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .h(px(18.))
        .px(px(6.))
        .rounded(px(4.))
        .bg(p.muted)
        .flex()
        .items_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted_fg)
        .child(label.into())
}

fn risk_chip(p: &Palette) -> Div {
    div()
        .h(px(17.))
        .px(px(5.))
        .rounded(px(4.))
        .border_1()
        .border_color(p.red.opacity(0.35))
        .flex()
        .items_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.red)
        .child("Risky")
}

fn segment_group(p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(7.))
        .border_1()
        .border_color(p.border)
        .bg(p.subtle)
}

fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.))
        .px(px(9.))
        .rounded(px(5.))
        .border_1()
        .border_color(if active {
            p.border
        } else {
            p.border.opacity(0.0)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(px(11.0))
        .when(active, |this| {
            this.bg(p.accent)
                .text_color(p.fg)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |this| {
            this.text_color(p.muted_fg)
                .hover(move |style| style.bg(p.hover))
        })
        .child(label.into())
}

fn settings_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(11.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(primary, |this| {
            this.bg(p.primary)
                .text_color(p.primary_fg)
                .hover(move |style| style.opacity(0.88))
        })
        .when(!primary, |this| {
            this.border_1()
                .border_color(p.border)
                .bg(p.card)
                .text_color(p.fg)
                .hover(move |style| style.bg(p.hover))
        })
        .child(label.into())
}

fn settings_text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: Hsla,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(7.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .hover(move |style| style.bg(p.hover))
        .child(label.into())
}

fn radio(selected: bool, p: &Palette) -> Div {
    div()
        .mt(px(1.))
        .size(px(16.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(if selected { p.fg } else { p.border_strong })
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |this| {
            this.child(div().size(px(7.)).rounded_full().bg(p.fg))
        })
}

fn scope_value(label: &str, color: Hsla, _p: &Palette) -> AnyElement {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label.to_uppercase())
        .into_any_element()
}

fn notification_permission_row(
    _this: &mut DunlinApp,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let status = crate::platform::notification_permission();
    let control = match status.state {
        crate::platform::PermissionState::Granted => scope_value("Granted", p.green, p),
        crate::platform::PermissionState::NotDetermined => {
            settings_button("notification-allow", "Allow…", true, p)
                .on_click(cx.listener(|this, _, _, cx| this.request_notifications(cx)))
                .into_any_element()
        }
        _ => settings_text_button("notification-open-settings", "Open Settings", p.amber, p)
            .on_click(cx.listener(
                |this, _, _, cx| match crate::platform::open_permission_settings(
                    crate::platform::PermissionKind::Notifications,
                ) {
                    Ok(()) => this.set_notice("Opened notification settings", cx),
                    Err(error) => this.set_notice(error, cx),
                },
            ))
            .into_any_element(),
    };
    setting_row("Notification permission", &status.detail, control, p)
}

fn setting_row(title: &str, description: &str, control: AnyElement, p: &Palette) -> AnyElement {
    use dunlin_ui::scale;
    div()
        .min_h(px(scale::control(48.)))
        .px(px(scale::space(14.)))
        .py(px(scale::space(9.)))
        .flex()
        .items_center()
        .gap(px(scale::space(16.)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(scale::space(2.)))
                .child(
                    div()
                        .text_size(px(scale::text(11.5)))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(scale::text(11.0)))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child(description.to_string()),
                ),
        )
        .child(control)
        .into_any_element()
}

fn divider(p: &Palette) -> AnyElement {
    div().mx(px(14.)).h(px(1.)).bg(p.border).into_any_element()
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
        .w(px(32.))
        .h(px(18.))
        .flex_shrink_0()
        .rounded_full()
        .bg(if value { p.primary } else { p.border_strong })
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
                .size(px(14.))
                .rounded_full()
                .bg(if value { p.primary_fg } else { p.card })
                .ml(px(if value { 16. } else { 2. }))
                .border_1()
                .border_color(p.border.opacity(0.35)),
        )
        .into_any_element()
}

fn font_picker(this: &DunlinApp, code: bool, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
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
    .h(px(28.))
    .px(px(10.))
    .rounded(px(6.))
    .border_1()
    .border_color(p.border)
    .bg(p.card)
    .text_color(p.fg)
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
        .max_w(px(260.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(p.border)
        .bg(p.subtle)
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
        .h(px(28.))
        .rounded(px(6.))
        .border_1()
        .border_color(p.border)
        .flex()
        .items_center()
        .child(
            div()
                .id("font-decrease")
                .w(px(24.))
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
                        .size(px(10.))
                        .text_color(p.muted_fg),
                ),
        )
        .child(
            div()
                .px(px(7.))
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.fg.opacity(0.8))
                .child(format!("{size:.0} px")),
        )
        .child(
            div()
                .id("font-increase")
                .w(px(24.))
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
                        .size(px(10.))
                        .text_color(p.muted_fg),
                ),
        )
        .into_any_element()
}

fn source_badge(source: UsageSource, p: &Palette) -> AnyElement {
    let (label, color) = match source {
        UsageSource::Live => ("LIVE", p.green),
        UsageSource::Session => ("SESSION", p.blue),
        UsageSource::Estimated => ("ESTIMATED", p.amber),
        UsageSource::Unavailable => ("N/A", p.muted_fg),
    };
    div()
        .h(px(17.))
        .flex()
        .items_center()
        .gap(px(4.))
        .child(dot(color, 4.0))
        .child(
            div()
                .text_size(px(9.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
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
