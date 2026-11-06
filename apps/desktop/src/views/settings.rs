//! Settings sheet. Everything that changes how Dunlin connects to a harness,
//! asks for permissions or measures usage lives here.

use dunlin_domain::{MainView, UsageSource};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::{Editor, Input};
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};
use dunlin_domain::PermissionMode;

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
    fn security_workspace_is_enabled_for_old_settings_and_roundtrips_when_disabled() {
        let mut settings: crate::app::StoredSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.security_enabled);
        settings.security_enabled = false;
        settings.security_triage = vec![crate::security::TriageRecord {
            source: "Cloudflare security-audit".into(),
            fingerprint: "fp-1".into(),
            status: crate::security::TriageStatus::Accepted,
        }];
        let saved: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!saved.security_enabled);
        assert_eq!(saved.security_triage.len(), 1);
        assert_eq!(
            saved.security_triage[0].status,
            crate::security::TriageStatus::Accepted
        );
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
    if tab != SettingsTab::Secrets {
        this.vault.ui.draft = None;
        this.vault.ui.recipe_draft = None;
        this.vault.ui.operation_draft = None;
    }
    let (width, height) = settings_size(
        f32::from(window.viewport_size().width),
        f32::from(window.viewport_size().height),
    );
    let viewport = window.viewport_size();
    let (left, top) = settings_position(
        (f32::from(viewport.width), f32::from(viewport.height)),
        (width, height),
        (
            f32::from(this.settings_offset.x),
            f32::from(this.settings_offset.y),
        ),
    );
    this.settings_offset = point(
        px(left - (f32::from(viewport.width) - width) / 2.),
        px(top - (f32::from(viewport.height) - height) / 2.),
    );

    let panel = div()
        .id("settings-panel")
        .absolute()
        .left(px(left))
        .top(px(top))
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("settings-panel")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(crate::effects::borderless_probe(
            &this.overlay_backdrop,
            20.,
        ))
        .w(px(width))
        .h(px(height))
        .rounded(px(20.))
        .shadow_lg()
        .bg(p.subtle)
        .overflow_hidden()
        .flex()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(settings_nav(this, cx))
        .child(
            div()
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("settings-content")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .m(px(8.))
                .rounded(px(16.))
                .overflow_hidden()
                .bg(p.bg)
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
                            SettingsTab::Sidebar => sidebar_settings(this, cx),
                            SettingsTab::Chat => chat_settings(this, cx),
                            SettingsTab::Harnesses => harnesses(this, cx),
                            SettingsTab::Skills => crate::skills::render(this, cx),
                            SettingsTab::Security => security(this, window, cx),
                            SettingsTab::Secrets => {
                                crate::vault::settings::render(this, window, cx)
                            }
                            SettingsTab::Extensions => crate::plugins::render(this, cx),
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
            this.settings_drag = None;
            cx.notify();
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            if let Some((start, offset)) = this.settings_drag {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.settings_offset = offset + (event.position - start);
                    cx.notify();
                } else {
                    this.settings_drag = None;
                }
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.settings_drag = None),
        )
        .child(panel)
        .into_any_element()
}

fn settings_position(viewport: (f32, f32), panel: (f32, f32), offset: (f32, f32)) -> (f32, f32) {
    let position = |view: f32, size: f32, delta: f32| {
        ((view - size) / 2. + delta).clamp(8., (view - size - 8.).max(8.))
    };
    (
        position(viewport.0, panel.0, offset.0),
        position(viewport.1, panel.1, offset.1),
    )
}

#[test]
fn settings_drag_stays_reachable_after_drag_and_resize() {
    assert_eq!(
        settings_position((1200., 900.), (900., 700.), (0., 0.)),
        (150., 100.)
    );
    assert_eq!(
        settings_position((1200., 900.), (900., 700.), (10000., -10000.)),
        (292., 8.)
    );
    assert_eq!(
        settings_position((600., 500.), (590., 490.), (10000., 10000.)),
        (8., 8.)
    );
}

fn settings_nav(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut nav = div()
        .w(px(200.))
        .min_h(px(0.))
        .overflow_hidden()
        .flex_shrink_0()
        .bg(p.subtle)
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
                SettingsTab::Sidebar,
                SettingsTab::Chat,
                SettingsTab::Editor,
            ],
        ),
        (
            "Agent",
            &[
                SettingsTab::Harnesses,
                SettingsTab::Skills,
                SettingsTab::Security,
                SettingsTab::Secrets,
                SettingsTab::Extensions,
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

fn security(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let enabled = this.security_enabled;
    let body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(section_label("Security tools", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "Enable Security tools",
                    "Audit tools and saved findings live here in Settings. Disabling does not remove skills or artifacts, or change permissions.",
                    toggle_switch(
                        "security-enabled",
                        enabled,
                        |this| {
                            this.security_enabled = !this.security_enabled;
                            if this.security_enabled {
                                this.security.checked_project = None;
                            } else if this.section == dunlin_domain::SidebarSection::Security {
                                this.section = dunlin_domain::SidebarSection::Projects;
                            }
                        },
                        cx,
                        &p,
                    ),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Audit skill and artifacts",
                    "The Cloudflare Security Audit bundle and any codex-security/SARIF artifacts remain on disk. Re-enable the workspace to inspect them again.",
                    div()
                        .text_size(px(11.))
                        .text_color(if enabled { p.green } else { p.muted_fg })
                        .child(if enabled { "Visible" } else { "Hidden" })
                        .into_any_element(),
                    &p,
                )),
        )
        .child(section_label("Boundaries", &p))
        .child(
            settings_card(&p).child(setting_row(
                "Not a sandbox or scanner",
                "Dunlin reads and validates audit artifacts; it does not execute target code, run Codex Security, or turn an empty findings file into a security claim.",
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .max_w(px(230.))
                    .child("Local-only workspace")
                    .into_any_element(),
                &p,
            )),
        );
    body.when(enabled, |body| {
        body.child(crate::security::sidebar_body(this, cx))
            .child(crate::security::render(this, window, cx))
    })
    .into_any_element()
}

fn settings_header(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let (title, subtitle) = match this.settings_tab {
        SettingsTab::General => (
            "General",
            "Startup behavior and the default workspace. Appearance lives in Design.",
        ),
        SettingsTab::Design => ("Design", "Theme, typography and interface density."),
        SettingsTab::Sidebar => (
            "Sidebar",
            "Project list appearance, sorting and sidebar width.",
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
        SettingsTab::Security => (
            "Security",
            "Audit tools, scan artifacts and findings — all in Settings.",
        ),
        SettingsTab::Secrets => (
            "Secrets",
            "Use-only credentials in the macOS Keychain. Agents can use them but never read them.",
        ),
        SettingsTab::Extensions => (
            "Extensions",
            "Declarative methodology packs. No pack code runs inside Dunlin.",
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
        .id("settings-drag-handle")
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("settings-drag-handle")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .h(px(56.))
        .flex_shrink_0()
        .px(px(32.))
        .flex()
        .items_center()
        .gap(px(8.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.settings_drag = Some((event.position, this.settings_offset));
                cx.stop_propagation();
            }),
        )
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
            icon_button("settings-close", IconName::X, "Close settings").on_click(cx.listener(
                |this, _, _, cx| {
                    this.settings_open = false;
                    cx.notify();
                },
            )),
        )
        .into_any_element()
}

fn general(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
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
        .child(crate::project_import::settings(this, cx))
        .child(crate::backup::settings(this, window, cx))
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
                .child(setting_row(
                    "Spec & intent tools",
                    "Show the Spec workspace, Intent Inbox, agent contract and related pack commands. Existing files remain untouched when disabled.",
                    toggle_switch(
                        "spec-tools",
                        this.spec_tools_enabled,
                        toggle_spec_tools,
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
    let mut density = segment_group(&p);
    for (value, label) in [
        (crate::app::UiDensity::Comfortable, "Comfortable"),
        (crate::app::UiDensity::Compact, "Compact"),
    ] {
        density = density.child(
            segment(format!("density:{label}"), label, this.density == value, &p).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.set_ui_density(value, cx);
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
                        .child(theme_card(crate::app::Appearance::System, this.appearance, window, cx, &p))
                        .child(theme_card(crate::app::Appearance::Light, this.appearance, window, cx, &p))
                        .child(theme_card(crate::app::Appearance::Dark, this.appearance, window, cx, &p)),
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
                            "Compact tightens project, session and Explorer rows, spacing and controls throughout the interface.",
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

fn sidebar_settings(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
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
                            "Show branch and local-change status in Comfortable. Your preference is kept in Compact.",
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
                        .child(setting_row("Show primary language", "Shown below topics in Comfortable. Your preference is kept in Compact.",
                            toggle_switch("project-language", this.project_list.show_language, |this| {
                                this.project_list.show_language = !this.project_list.show_language;
                                this.project_catalog.language = None;
                            }, cx, &p), &p))
                        .child(divider(&p))
                        .child(setting_row("Show repository topics", "Shown above language in Comfortable. Your preference is kept in Compact; click a topic to filter projects.",
                            toggle_switch("project-topics", this.project_list.show_topics, |this| {
                                this.project_list.show_topics = !this.project_list.show_topics;
                                this.project_catalog.topic = None;
                            }, cx, &p), &p))
                        .child(divider(&p))
                        .child(setting_row("Wrap project names", "Wrap long names in Comfortable. Compact keeps one-line names without changing this preference.",
                            toggle_switch("project-wrap-names", this.project_list.wrap_names, |this| this.project_list.wrap_names = !this.project_list.wrap_names, cx, &p), &p))
                        .child(divider(&p))
                        .child(setting_row("Project hover preview", "Show repository name, primary language, storage size, topics and an inert README preview on hover.",
                            toggle_switch("project-hover-preview", this.project_list.show_preview, |this| this.project_list.show_preview = !this.project_list.show_preview, cx, &p), &p))
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
                        )),
                )
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
            "Show the session harness logo next to its responses. Custom or unknown harnesses use initials.",
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
    appearance: crate::app::Appearance,
    current: crate::app::Appearance,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let selected = appearance == current;
    let preview = if appearance.mode(window.appearance()).is_dark() {
        dunlin_ui::theme::dark()
    } else {
        dunlin_ui::theme::light()
    };
    let label = match appearance {
        crate::app::Appearance::System => "System",
        crate::app::Appearance::Light => "Light",
        crate::app::Appearance::Dark => "Dark",
    };

    div()
        .id(format!("theme-card-{label}"))
        .flex_1()
        .min_w(px(0.))
        .rounded(px(9.))
        .border_1()
        .border_color(if selected { p.fg } else { p.border })
        .bg(p.card)
        .overflow_hidden()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.set_appearance(appearance, window, cx);
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
                    Icon::new(match appearance {
                        crate::app::Appearance::System => IconName::Monitor,
                        crate::app::Appearance::Dark => IconName::Moon,
                        crate::app::Appearance::Light => IconName::Sun,
                    })
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
    if let Some(project) = this.project().remote.clone() {
        if this.server_panel.host.as_deref() != Some(&project.host_id) {
            if this.server_panel.transfer.is_some()
                || this
                    .server_panel
                    .job
                    .as_ref()
                    .is_some_and(|job| !job.finished)
            {
                return div().p(px(24.)).child("Another server operation is still running. Open Remote settings to finish or cancel it before switching server configuration.").into_any_element();
            }
            this.server_panel = crate::server_panel::State {
                host: Some(project.host_id),
                ..Default::default()
            };
            this.refresh_server(cx);
        }
        return div()
            .p(px(24.))
            .child(crate::server_panel::render(this, cx))
            .into_any_element();
    }
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
        let detected = discovery.iter().find(|entry| entry.name == *name);
        let installed = detected
            .map(|entry| entry.adapter.is_some())
            .unwrap_or_else(|| crate::harness::command_available(command));
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
        let visibility_name = name.to_string();
        let visible = this.harness_visible(name);
        let scanning =
            this.session_import.loading && this.session_import.scanning.as_deref() == Some(*name);

        let action = if installed {
            settings_button(
                format!("import-sessions:{name}"),
                "Import sessions…",
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
        } else if docs.is_some() {
            settings_button(format!("install-harness:{name}"), "Set up…", false, &p)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.show_harness_setup(&install_name, cx);
                }))
                .into_any_element()
        } else {
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("Configure below")
                .into_any_element()
        };

        #[cfg(debug_assertions)]
        let action = if *name == "Claude Code" {
            div()
                .relative()
                .child(action)
                .children(crate::workspace_ui_smoke::probe("claude-harness-action"))
                .into_any_element()
        } else {
            action
        };
        let row = div()
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
            .when(!scanning, |row| row.child(
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
            ))
            .child(
                div()
                    .when(!scanning, |view| view.w(px(124.)))
                    .when(scanning, |view| view.flex_1().min_w(px(0.)))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .child(if scanning {
                        div().min_w(px(0.)).truncate().text_size(px(11.)).text_color(p.muted_fg)
                            .child(this.session_import.progress_label().unwrap_or_else(|| "Scanning…".into()))
                            .into_any_element()
                    } else { action }),
            )
            .child(settings_button(format!("harness-visible:{name}"), if visible { "Shown" } else { "Hidden" }, visible, &p)
                .toggled(visible)
                .accessibility_label(format!("Show {name} in harness choices"))
                .tooltip("Show this harness in selection lists; existing conversations are unchanged")
                .relative().children({
                    #[cfg(debug_assertions)] { if *name == "Claude Code" { crate::workspace_ui_smoke::probe("claude-harness-visible") } else { None } }
                    #[cfg(not(debug_assertions))] { None::<AnyElement> }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_harness_visibility(&visibility_name, cx);
                })))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.default_harness = name_owned.clone();
                this.persist_settings(cx);
                this.set_notice(
                    if installed {
                        format!("Default harness · {name_owned} · applies to new conversations")
                    } else {
                        format!("Default harness · {name_owned} · install or configure its ACP adapter before starting a conversation")
                    },
                    cx,
                );
            }));
        list = list
            .when(this.harness_setup.as_deref() == Some(*name), |list| {
                list.child(harness_setup(this, name, cx, &p))
            })
            .child(row);
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
                    settings_button("rescan-harnesses", if this.harness_scanning { "Scanning…" } else { "Scan again" }, false, &p)
                    .relative().children({
                        #[cfg(debug_assertions)]
                        { crate::workspace_ui_smoke::probe("rescan-harnesses") }
                        #[cfg(not(debug_assertions))]
                        { None::<AnyElement> }
                    }).on_click(
                        cx.listener(|this, _, _, cx| this.rescan_harnesses(cx)),
                    ),
                ),
        )
        .when_some(this.harness_setup_notice.clone(), |body, notice| {
            body.child(div().text_size(px(11.)).text_color(p.muted_fg).child(notice))
        })
        .child(
            div()
                .text_size(px(11.0))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("ACP agents run locally over stdio. Nothing is installed or authorized automatically. Shown / Hidden controls selection lists without uninstalling adapters or changing existing conversations and defaults. The full catalog stays here."),
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
                        .shadow_sm()
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

fn harness_setup(
    this: &DunlinApp,
    name: &str,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let docs_name = name.to_string();
    let mut card = settings_card(p).p(px(12.)).flex().flex_col().gap(px(9.))
        .child(div().flex().items_center()
            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(format!("Set up {name}")))
            .child(settings_text_button("harness-setup-close", "Close", p.muted_fg, p)
                .on_click(cx.listener(|this, _, _, cx| { this.harness_setup = None; cx.notify(); }))))
        .child(div().text_size(px(11.)).line_height(relative(1.45)).text_color(p.muted_fg)
            .child("Nothing is installed automatically. Follow these instructions in your terminal, then Scan again."));
    if let Some(command) = crate::harness_discovery::install_command(name) {
        card = card
            .child(div().text_size(px(11.)).font_family(dunlin_ui::theme::code_font()).child(command))
            .child(settings_button("copy-harness-install", "Copy install command", false, p)
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.into()))))
            .child(div().text_size(px(11.)).line_height(relative(1.45)).text_color(p.muted_fg)
                .child("Requires Node.js 22+. The -g option installs globally. Without -g, npm installs in the current folder's node_modules/.bin. Dunlin also checks ~/node_modules/.bin; for another local install, set Custom ACP to its absolute executable path."));
    }
    if let Some(path) = this
        .harness_discovery
        .iter()
        .find(|entry| entry.name == name)
        .and_then(|entry| entry.adapter.as_ref())
    {
        card = card.child(
            div()
                .text_size(px(11.))
                .text_color(p.green)
                .child(format!("Detected executable: {}", path.display())),
        );
    }
    card.child(
        settings_button("harness-setup-guide", "Open setup guide", false, p)
            .on_click(cx.listener(move |this, _, _, cx| this.open_harness_docs(&docs_name, cx))),
    )
    .into_any_element()
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
                                if this.permissions_refreshing { "Checking…" } else { "Recheck" },
                                p.muted_fg,
                                &p,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_permissions(cx);
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
                ))
                .child(divider(&p))
                .child(setting_row("Inline blame", "Remember this choice for every source file, including after restarting the app. Uncommitted lines are never attributed.",
                    toggle_switch("inline-blame", this.inline_blame, |this| {
                        this.inline_blame = !this.inline_blame;
                        for tab in &mut this.workbench.tabs { tab.blame.invalidate(); }
                    }, cx, &p), &p)),
        )
        .child(section_label("Terminal", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "Shell integration",
                    "Sources your zsh/bash rc through a private wrapper that emits OSC 133/OSC 7 prompt and directory markers. Disable if a custom prompt misbehaves.",
                    toggle_switch(
                        "shell-integration",
                        this.shell_integration,
                        |this| this.shell_integration = !this.shell_integration,
                        cx,
                        &p,
                    ),
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
    this.ensure_usage(cx);
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
    use dunlin_usage::billing::TimeRange;
    let mut ranges = segment_group(p);
    for range in TimeRange::ALL {
        ranges = ranges.child(
            segment(
                format!("billing-range:{}", range.label()),
                range.label(),
                this.billing_range == range,
                p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.billing_range = range;
                cx.notify();
            })),
        );
    }
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
                )
                .when_some(this.billing.clone(), |row, summary| {
                    row.child(
                        settings_button("billing-qa-report", "Copy QA report", false, p).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    summary.qa_report(),
                                ));
                                this.set_notice("Copied billing evidence QA report", cx);
                            }),
                        ),
                    )
                })
                .when(!this.remote_hosts.is_empty(), |row| {
                    row.child(
                        settings_button(
                            "billing-sync",
                            if this.billing_syncing {
                                "Syncing…"
                            } else {
                                "Sync evidence"
                            },
                            false,
                            p,
                        )
                        .when(this.billing_syncing, |this| this.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.billing_syncing {
                                this.sync_billing_evidence(cx);
                            }
                        })),
                    )
                })
                .when(
                    this.remote_hosts
                        .iter()
                        .any(|host| host.user_role == "admin"),
                    |row| {
                        row.child(
                            settings_button(
                                "billing-relay-evidence",
                                if this.remote_billing_loading {
                                    "Loading relay…"
                                } else {
                                    "Relay evidence"
                                },
                                false,
                                p,
                            )
                            .when(this.remote_billing_loading, |this| this.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.remote_billing_loading {
                                    this.load_remote_billing_evidence(cx);
                                }
                            })),
                        )
                    },
                ),
        )
        .child(
            div()
                .text_size(px(10.5))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("Read-only, bounded scan of local Claude Code, Codex and OpenCode logs. Deduped; unknown models stay unpriced and are never counted as $0. Main/subagent/internal attribution is explicit; unknown is never assumed to be main work. Relay sync sends aggregate evidence only, never raw logs or prompts."),
        );
    body = body.child(ranges);
    if let Some(error) = &this.billing_error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    let selected = TimeRange::ALL
        .iter()
        .position(|range| *range == this.billing_range)
        .unwrap();
    let Some(summary) = (if this.billing_range == TimeRange::All {
        this.billing.clone()
    } else {
        this.billing_ranges.get(selected).cloned()
    }) else {
        return body
            .child(
                div()
                    .rounded(px(8.))
                    .shadow_sm()
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
                    "Selected period",
                    format!("${:.2}", totals.priced_cost_usd),
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
                usage_metric(
                    "Subagents",
                    totals.subagent_events.to_string(),
                    &format!(
                        "≥ ${:.4} known · {} internal",
                        totals.subagent_cost_usd, totals.internal_events
                    ),
                    p.fg,
                    p,
                ),
                usage_metric(
                    "Unknown attribution",
                    totals.unknown_attribution_events.to_string(),
                    if totals.unknown_attribution_events > 0 {
                        "never assumed main work"
                    } else {
                        "all records attributed"
                    },
                    if totals.unknown_attribution_events > 0 {
                        p.amber
                    } else {
                        p.green
                    },
                    p,
                ),
            ]),
        );
    let mut budget_row = div().flex().items_center().gap(px(5.));
    budget_row = budget_row.child(
        div()
            .text_size(px(10.5))
            .text_color(p.muted_fg)
            .child("Monthly alert budget"),
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
    let mut alerts = summary.alerts(None);
    if this.billing_alerts
        && let Some(all) = &this.billing
    {
        alerts.extend(all.budget_alerts(this.billing_budget()));
    }
    for alert in alerts {
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
            .flex_col()
            .w_full()
            .min_w(px(0.))
            .gap(px(10.))
            .child(billing_slices(
                "By work attribution",
                &summary.by_attribution,
                p,
            ))
            .child(billing_slices("By harness", &summary.by_harness, p))
            .child(billing_slices("By provider", &summary.by_provider, p))
            .child(billing_slices("By model", &summary.by_model, p))
            .child(billing_slices("By project", &summary.by_project, p)),
    );
    body = body.child(billing_days(&summary.by_day, this.billing_range.label(), p));
    if !this.billing_sync.is_empty() {
        body = body.child(billing_sync_results(&this.billing_sync, p));
    }
    if let Some(error) = &this.remote_billing_error {
        body = body.child(
            div()
                .rounded(px(7.))
                .border_1()
                .border_color(p.amber.opacity(0.25))
                .bg(p.card)
                .px(px(10.))
                .py(px(7.))
                .text_size(px(10.5))
                .text_color(p.amber)
                .child(error.clone()),
        );
    }
    if !this.remote_billing.is_empty() {
        body = body.child(billing_remote_evidence(&this.remote_billing, p));
    }
    body.into_any_element()
}

fn billing_sync_results(results: &[crate::usage::BillingSync], p: &Palette) -> AnyElement {
    let mut card = settings_card(p).w_full().overflow_hidden().child(
        div()
            .px(px(9.))
            .py(px(6.))
            .border_b_1()
            .border_color(p.border)
            .bg(p.subtle)
            .text_size(px(10.))
            .font_family(dunlin_ui::theme::code_font())
            .text_color(p.muted_fg)
            .child("Relay sync · aggregate evidence only"),
    );
    for result in results {
        card = card.child(
            div()
                .px(px(9.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(120.))
                        .truncate()
                        .text_size(px(10.))
                        .text_color(p.fg)
                        .child(result.host.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .text_color(if result.ok { p.green } else { p.red })
                        .child(result.message.clone()),
                ),
        );
    }
    card.into_any_element()
}

fn billing_remote_evidence(
    rows: &[crate::remote::RemoteBillingEvidence],
    p: &Palette,
) -> AnyElement {
    let mut card = settings_card(p).w_full().overflow_hidden().child(
        div()
            .px(px(9.))
            .py(px(6.))
            .border_b_1()
            .border_color(p.border)
            .bg(p.subtle)
            .text_size(px(10.))
            .font_family(dunlin_ui::theme::code_font())
            .text_color(p.muted_fg)
            .child("Relay evidence · administrator view"),
    );
    for row in rows {
        let totals = &row.evidence.summary.totals;
        card = card.child(
            div()
                .px(px(9.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(180.))
                        .truncate()
                        .text_size(px(10.))
                        .text_color(p.fg)
                        .child(format!("{} · {}", row.user_name, row.device_name)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{} ev · {} subagent · {} unknown · {}",
                            totals.events,
                            totals.subagent_events,
                            totals.unknown_attribution_events,
                            totals.cost_label()
                        )),
                ),
        );
    }
    card.into_any_element()
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
    let mut card = settings_card(p)
        .w_full()
        .min_w(px(0.))
        .flex_shrink_0()
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

fn billing_days(days: &[dunlin_usage::billing::Day], range: &str, p: &Palette) -> AnyElement {
    let mut card = settings_card(p).w_full().overflow_hidden().child(
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
            .child(format!("Daily usage · {range}")),
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
    for day in days {
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
                    let button = settings_text_button("close-qa-report", "Close", p.muted_fg, &p)
                        .on_click(cx.listener(|this, _, _, cx| { this.qa_report = None; this.qa_error = None; cx.notify(); }));
                    let button = div().relative().child(button);
                    #[cfg(debug_assertions)]
                    let button = button.children(crate::workspace_ui_smoke::probe("close-qa-report"));
                    card.child(button).child(
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
        .child(
            settings_card(&p)
                .p(px(14.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.red)
                                .child("Reset Dunlin to factory defaults"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(relative(1.45))
                                .text_color(p.muted_fg)
                                .child("Permanently remove local settings, projects, groups, history and Dunlin's saved credentials. Repository files and installed agents stay. Dunlin closes after resetting."),
                        ),
                )
                .child(
                    settings_text_button("factory-reset", "Reset…", p.red, &p)
                        .on_click(cx.listener(|this, _, window, cx| this.request_factory_reset(window, cx))),
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

    let mut body = div()
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(crate::server_panel::render(this, cx))
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
                        .child("Use Connect to server to enter an endpoint and a relay login key, or choose an SSH identity."),
                ),
        );
    } else {
        for host in this.remote_hosts.clone() {
            let host_id = host.id.clone();
            let host_for_picker = host_id.clone();
            let host_for_management = host_id.clone();
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
                            settings_button(
                                format!("remote-manage:{host_id}"),
                                "Manage",
                                false,
                                &p,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.manage_server(&host_for_management, window, cx)
                                },
                            )),
                        )
                        .child(
                            settings_button(
                                format!("remote-add-project:{host_id}"),
                                "Add project",
                                false,
                                &p,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.open_remote_picker_for(&host_for_picker, cx);
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                format!("remote-remove:{host_id}"),
                                IconName::X,
                                "Remove remote host",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.remove_remote_host(&host_id, cx);
                                },
                            )),
                        ),
                ),
            );
        }
    }

    body = body.child(
        dunlin_ui::button("settings-connect-server")
            .primary()
            .label("Connect to server…")
            .on_click(cx.listener(|this, _, window, cx| this.open_remote_connection(window, cx))),
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
        .px(px(8.))
        .flex()
        .child(
            div()
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("settings-page")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .w_full()
                .child(content),
        )
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
    card_surface(*p).rounded(px(16.)).shadow_sm()
}

fn settings_chip(label: impl Into<SharedString>, p: &Palette) -> Div {
    badge(label, p.muted_fg, p.muted, *p)
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
    dunlin_ui::segment_group(*p)
}

fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    _p: &Palette,
) -> Button {
    segment_button(id, label, active)
}

fn settings_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    _p: &Palette,
) -> Button {
    dunlin_ui::button(id)
        .label(label)
        .when(primary, |button| button.primary())
}

fn settings_text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: Hsla,
    p: &Palette,
) -> Button {
    dunlin_ui::button(id)
        .ghost()
        .label(label)
        .when(color == p.red, |button| button.danger().outline())
        .when(color == p.amber, |button| button.warning().outline())
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

fn toggle_spec_tools(this: &mut DunlinApp) {
    this.spec_tools_enabled = !this.spec_tools_enabled;
    if !this.spec_tools_enabled {
        this.spec.active = false;
        this.spec.view = None;
    }
}

fn toggle_switch(
    id: &'static str,
    value: bool,
    action: fn(&mut DunlinApp),
    cx: &mut Context<DunlinApp>,
    _p: &Palette,
) -> AnyElement {
    dunlin_ui::switch(id, id.replace('-', " "), value)
        .on_click(cx.listener(move |this, _, window, cx| {
            action(this);
            this.apply_editor_preferences(window, cx);
            this.persist_settings(cx);
            cx.notify();
        }))
        .into_any_element()
}

fn font_picker(this: &DunlinApp, code: bool, cx: &mut Context<DunlinApp>) -> AnyElement {
    let selected = if code {
        this.code_font_family.clone()
    } else {
        this.ui_font_family.clone()
    };
    let app = cx.entity().downgrade();
    dunlin_ui::button(if code {
        "code-font-picker"
    } else {
        "ui-font-picker"
    })
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
        || name.contains("nerd")
        || name.contains("powerline")
        || name.contains("meslo")
        || name.ends_with(" nf")
        || name.ends_with(" nfm")
        || name.ends_with(" nfp")
        || name.contains("hack")
        || name.contains("iosevka")
        || name.contains("inconsolata")
        || name.contains("cascadia")
}

fn mono_value(label: &str, p: &Palette) -> AnyElement {
    div()
        .max_w(px(260.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(6.))
        .shadow_sm()
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
    dunlin_ui::segment_group(*p)
        .child(
            icon_button("font-decrease", IconName::Minus, "Decrease code font size").on_click(
                cx.listener(|this, _, _, cx| {
                    this.adjust_code_font_size(-1.0, cx);
                }),
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
            icon_button("font-increase", IconName::Plus, "Increase code font size").on_click(
                cx.listener(|this, _, _, cx| {
                    this.adjust_code_font_size(1.0, cx);
                }),
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
            "MesloLGS NF",
            "Hack Nerd Font",
            "Iosevka",
            "Cascadia Code",
            "Inconsolata",
        ] {
            assert!(super::code_font_candidate(name), "{name}");
        }
        for name in ["Geist", "Arial", "Helvetica Neue", "Apple Color Emoji"] {
            assert!(!super::code_font_candidate(name), "{name}");
        }
    }
}
