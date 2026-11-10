//! Settings sheet. Everything that changes how Dunlin connects to a harness,
//! asks for permissions or measures usage lives here.

use dunlin_domain::UsageSource;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::Editor;
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::component::{Disableable, Selectable, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};
use dunlin_domain::PermissionMode;

mod data;
mod navigation;
pub(crate) use navigation::State;

pub(crate) fn search_entries(query: &str) -> Vec<crate::app::CommandItem> {
    let query = query
        .split_whitespace()
        .filter(|term| {
            !term.eq_ignore_ascii_case("settings") && !term.eq_ignore_ascii_case("preferences")
        })
        .collect::<Vec<_>>()
        .join(" ");
    navigation::GROUPS
        .iter()
        .flat_map(|(_, tabs)| tabs.iter().copied())
        .filter(|tab| tab.matches_search(&query))
        .map(|tab| crate::app::CommandItem {
            command: crate::app::Command::Settings(tab),
            label: format!("Settings · {}", tab.label()),
            hint: Some("settings".into()),
            icon: tab.icon(),
            keywords: tab.subtitle().into(),
        })
        .collect()
}

#[test]
fn global_settings_search_uses_the_settings_page_aliases() {
    let entries = search_entries("font");
    assert_eq!(search_entries("settings").len(), 15);
    assert_eq!(search_entries("settings font").len(), entries.len());
    assert!(
        entries
            .iter()
            .any(|item| item.command == crate::app::Command::Settings(SettingsTab::Design))
    );
    assert!(
        entries
            .iter()
            .any(|item| item.command == crate::app::Command::Settings(SettingsTab::Editor))
    );
    assert!(
        !entries
            .iter()
            .any(|item| item.command == crate::app::Command::Settings(SettingsTab::Data))
    );
}
#[cfg(debug_assertions)]
pub(crate) mod smoke;

pub(crate) fn settings_size(width: f32, height: f32) -> (f32, f32) {
    (
        (width - 48.).clamp(360., 1120.),
        (height - 48.).clamp(320., 760.),
    )
}

#[cfg(test)]
mod design_tests {
    #[test]
    fn skin_settings_migrate_roundtrip_and_remain_independent_of_appearance() {
        use crate::app::{Appearance, StoredSettings};
        use dunlin_ui::Skin;
        let old: StoredSettings = serde_json::from_str(r#"{"appearance":"Dark"}"#).unwrap();
        assert_eq!(old.skin, Skin::Default);
        let future: StoredSettings = serde_json::from_str(r#"{"skin":"FutureSkin"}"#).unwrap();
        assert_eq!(future.skin, Skin::Default);
        for skin in Skin::ALL {
            for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
                let settings = StoredSettings {
                    skin,
                    appearance: Some(appearance),
                    ..Default::default()
                };
                let saved: StoredSettings =
                    serde_json::from_value(serde_json::to_value(settings).unwrap()).unwrap();
                assert_eq!(saved.skin, skin);
                assert_eq!(saved.appearance, Some(appearance));
            }
        }
    }

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
    this.ensure_settings_search(window, cx);
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

    let panel = SurfaceGeometry::SETTINGS
        .paint_material(p, dunlin_ui::settings::Tokens::new(p).sidebar, p.border)
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
        .child(crate::effects::panel_probe(
            &this.overlay_backdrop,
            SurfaceGeometry::SETTINGS,
            false,
        ))
        .w(px(width))
        .h(px(height))
        .shadow_lg()
        .overflow_hidden()
        .flex()
        .on_click(|_, _, cx| cx.stop_propagation())
        .role(Role::Dialog)
        .aria_label("Application settings")
        .tab_group()
        .child(settings_nav(this, cx))
        .child(
            SurfaceGeometry::SETTINGS_CONTENT
                .paint_material(p, p.bg, p.border)
                .rounded_tl(px(dunlin_ui::radii::SQUARE))
                .rounded_bl(px(dunlin_ui::radii::SQUARE))
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
                .overflow_hidden()
                .child(settings_header(this, cx))
                .child(
                    div()
                        .id(format!("settings-scroll:{:?}", tab))
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(settings_page(
                            match tab {
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
                                SettingsTab::Remote => remote_settings(this, window, cx),
                                SettingsTab::Editor => editor(this, cx),
                                SettingsTab::Usage => usage(this, cx),
                                SettingsTab::Data => data::render(this, window, cx),
                                SettingsTab::Advanced => advanced(this, cx),
                            },
                            tab,
                        )),
                ),
        );

    div()
        .id("settings-overlay")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(black().opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(|this, _, window, cx| {
            this.request_settings_location(None, window, cx);
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
    let search = this.settings_ui.search.as_ref().unwrap();
    let query = search.read(cx).value().to_string();
    let mut list = div()
        .id("settings-navigation-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(12.))
        .pb(px(12.));
    let mut found = false;
    for (group, tabs) in navigation::GROUPS {
        let visible: Vec<_> = tabs
            .iter()
            .copied()
            .filter(|tab| tab.matches_search(&query))
            .collect();
        if visible.is_empty() {
            continue;
        }
        found = true;
        list = list.child(
            div()
                .flex_shrink_0()
                .pt(px(8.))
                .pb(px(4.))
                .px(px(10.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted_fg)
                .child(*group),
        );
        for tab in visible {
            list = list.child(
                dunlin_ui::button(format!("settings-tab:{}", tab.label()))
                    .w_full()
                    .h(px(29.))
                    .rounded(px(dunlin_ui::settings::Tokens::CONTROL_RADIUS))
                    .flex_shrink_0()
                    .justify_start()
                    .px(px(10.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(Icon::new(tab.icon()).size(px(14.)))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(dunlin_ui::settings::Tokens::BODY_SIZE))
                                    .child(tab.label()),
                            ),
                    )
                    .selected(this.settings_tab == tab)
                    .toggled(this.settings_tab == tab)
                    .accessibility_label(format!("{} settings", tab.label()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request_settings_location(Some(tab), window, cx);
                    })),
            );
        }
    }
    if !found {
        list = list.child(
            dunlin_ui::settings::description(
                "No matching settings. Try ‘agent’, ‘backup’ or ‘font’.",
                p,
            )
            .p(px(10.)),
        );
    }
    div()
        .w(px(dunlin_ui::settings::Tokens::NAV_WIDTH))
        .min_h(px(0.))
        .overflow_hidden()
        .flex_shrink_0()
        // The shell owns the background. A square child fill covers its left curves.
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(54.))
                .flex_shrink_0()
                .px(px(18.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Settings"),
                ),
        )
        .child(
            div().px(px(12.)).pb(px(8.)).flex_shrink_0().child(
                dunlin_ui::settings::search(search, "Search all settings", p).when(
                    this.show_shortcut_hints,
                    |input| {
                        input.suffix(div().text_size(px(10.)).text_color(p.muted_fg).child("⌘F"))
                    },
                ),
            ),
        )
        .child(list)
        .into_any_element()
}

fn security(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let enabled = this.security_enabled;
    let mut tabs = segment_group(&p);
    for (audits, label) in [(false, "Configuration"), (true, "Project audits")] {
        tabs = tabs.child(
            segment(
                format!("security-page:{audits}"),
                label,
                this.settings_ui.audits == audits,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_ui.audits = audits;
                cx.notify();
            })),
        );
    }
    let body = dunlin_ui::settings::page().child(tabs);
    if this.settings_ui.audits {
        return body.child(dunlin_ui::settings::notice(format!("Project: {} · audit actions are explicit and may launch an external CLI or local sandboxed check.", this.project().name), p))
            .when(enabled, |body| body.child(crate::security::render(this, window, cx)))
            .when(!enabled, |body| body.child(dunlin_ui::settings::notice("Audit tools are disabled. Enable them in Configuration; saved artifacts are not deleted.", p)))
            .into_any_element();
    }
    body
        .child(section_label("Security tools", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "Enable Security tools",
                    "Show project audit actions and saved findings. Disabling does not remove skills or artifacts, or change agent permissions.",
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
                "Explicit tools, not an agent sandbox",
                "Dunlin reads audit artifacts and can explicitly launch external Codex Security scans or bounded local sandboxed checks. These do not sandbox your ACP agent; empty findings are not proof of security.",
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .max_w(px(230.))
                    .child("Explicit actions only")
                    .into_any_element(),
                &p,
            )),
        )
        .child(section_label("Tool availability", &p))
        .child(crate::security::configuration_status(this, cx))
    .into_any_element()
}

fn settings_header(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let title = this.settings_tab.label();
    let subtitle = this.settings_tab.subtitle();

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
        .min_h(px(82.))
        .flex_shrink_0()
        .px(px(dunlin_ui::settings::Tokens::PAGE_INSET))
        .flex()
        .items_center()
        .gap(px(16.))
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
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(dunlin_ui::settings::Tokens::TITLE_SIZE))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title),
                )
                .child(dunlin_ui::settings::description(subtitle, p)),
        )
        .child(
            icon_button("settings-close", IconName::X, "Close settings")
                .tooltip(this.shortcut_label("Close settings", "⌘W / Esc"))
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("settings-close")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.request_settings_location(None, window, cx);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn general(this: &mut DunlinApp, _window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);

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

    let body = dunlin_ui::settings::page()
        .child(section_label("Startup", &p))
        .child(
            dunlin_ui::settings::group()
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
                )),
        )
        .child(section_label("Notifications", &p))
        .child(
            dunlin_ui::settings::group()
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
                        .min_w(px(0.))
                        .text_size(px(11.0))
                        .line_height(relative(1.5))
                        .text_color(p.muted_fg)
                        .child("Local projects stay on this Mac unless you explicitly copy them to a server. Agents may use their providers’ network services. Remote projects and agents run on the server you select."),
                ),
        );

    body.into_any_element()
}

fn design(this: &mut DunlinApp, _window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
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
    dunlin_ui::settings::page()
                .child(section_label("Theme", &p))
                 .child(setting_row("Appearance", "Use the system appearance or choose a light or dark interface.",
                     appearance_picker(this, cx), &p))
                 .child(setting_row("Skin", "Surface style is independent of light and dark appearance. Changes are saved on this Mac.",
                     skin_picker(this, cx), &p))
                .child(section_label("Interface", &p))
                .child(
                    dunlin_ui::settings::group()
                        .child(setting_row(
                            "UI font",
                             "Used by application controls. Code typography is in Editor & Terminal.",
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
                             "Show keyboard shortcut hints",
                             "Show shortcut hints in menus and controls. Shortcuts work even when hints are hidden.",
                             toggle_switch(
                                 "shortcut-hints",
                                 this.show_shortcut_hints,
                                 |this| this.show_shortcut_hints = !this.show_shortcut_hints,
                                cx,
                                &p,
                            ),
                            &p,
                        ))
                        .child(divider(&p))
                        .child(setting_row(
                             "Reduce motion",
                             "Disable decorative entrance, pulse and import animations. Status remains visible without motion.",
                            toggle_switch(
                                "reduce-motion",
                                this.reduce_motion,
                                 |this| this.reduce_motion = !this.reduce_motion,
                                cx,
                                &p,
                            ),
                             &p,
                         ))
                         .when(cfg!(target_os = "macos"), |card| card
                             .child(divider(&p))
                             .child(setting_row(
                                 "GPU acceleration",
                                 "The interface is already accelerated by Metal. This macOS renderer has no CPU fallback, so GPU acceleration cannot be disabled.",
                                 div().text_size(px(11.)).text_color(p.muted_fg).child("Metal · Active").into_any_element(),
                                 &p,
                             )))
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
    dunlin_ui::settings::page()
                .child(section_label("Dashboard", &p))
                .child(dunlin_ui::settings::group()
                    .child(setting_row("Live agents", "Show working, waiting and connecting agents.",
                        toggle_switch("dashboard-agents", this.dashboard_settings.show_agents,
                            |this| this.dashboard_settings.show_agents = !this.dashboard_settings.show_agents, cx, &p), &p))
                    .child(divider(&p))
                    .child(setting_row("Activity", "Show recorded metrics and the activity heatmap.",
                        toggle_switch("dashboard-activity", this.dashboard_settings.show_activity,
                            |this| this.dashboard_settings.show_activity = !this.dashboard_settings.show_activity, cx, &p), &p)))
                .child(section_label("Project list", &p))
                .child(
                    dunlin_ui::settings::group()
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
                        .child(setting_row("Project information", "Show an info button in the upper-right corner of each project. Click it to see repository metadata and a scrollable README preview.",
                            toggle_switch("project-info-preview", this.project_list.show_preview, |this| this.project_list.show_preview = !this.project_list.show_preview, cx, &p), &p))
                        .child(divider(&p))
                        .child(setting_row(
                            "Sort",
                            "Pinned projects stay first. Modification dates use cached repository metadata.",
                            sort.into_any_element(),
                            &p,
                        )),
                )
                .child(section_label("Explorer", &p))
                .child(dunlin_ui::settings::group().child(setting_row("Show file sizes",
                    "Show sizes in Explorer tooltips, project totals and Dashboard tiles.",
                    toggle_switch("show-file-sizes", this.show_file_sizes,
                        |this| this.show_file_sizes = !this.show_file_sizes, cx, &p), &p)))
                .child(crate::project_import::settings(this, cx))
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
            "Hide intermediate text, reasoning, tools, plans and subagent output. Permission requests and errors remain visible; recorded history is preserved, with agent work in Graph and file edits in Timeline.",
            "final-answer-only",
            this.final_answer_only,
            |this| this.final_answer_only = !this.final_answer_only,
        ),
    ];
    let mut behavior = dunlin_ui::settings::group();
    let mut appearance = dunlin_ui::settings::group();
    for (index, (title, description, id, value, action)) in controls.into_iter().enumerate() {
        let row = setting_row(
            title,
            description,
            toggle_switch(id, value, action, cx, &p),
            &p,
        );
        if matches!(index, 0 | 4 | 5) {
            if index != 0 {
                behavior = behavior.child(divider(&p));
            }
            behavior = behavior.child(row);
        } else {
            if index != 1 {
                appearance = appearance.child(divider(&p));
            }
            appearance = appearance.child(row);
        }
    }
    dunlin_ui::settings::page()
        .child(section_label("Response behavior", &p))
        .child(behavior)
        .when(this.final_answer_only, |body| body.child(dunlin_ui::settings::notice(
            "Final answer only hides reasoning and tool details in Chat. Your reasoning preference is retained for when this mode is off; recorded history and the Graph and Timeline views are unaffected.", p)))
        .child(section_label("Message appearance", &p))
        .child(appearance)
        .into_any_element()
}

fn skin_picker(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let app = cx.entity().downgrade();
    dunlin_ui::settings::button("skin-picker")
        .label(this.skin.label())
        .accessibility_label("Skin")
        .dropdown_caret(true)
        .w(px(180.))
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("skin-picker")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let selected = entity.read(cx).skin;
            for skin in Skin::ALL {
                let app = app.clone();
                menu = menu.item(
                    crate::effects::menu_item(move |_, cx| {
                        div()
                            .min_w(px(150.))
                            .text_size(px(12.))
                            .text_color(palette(cx).fg)
                            .child(skin.label())
                            .children({
                                #[cfg(debug_assertions)]
                                {
                                    crate::workspace_ui_smoke::probe(match skin {
                                        Skin::Default => "skin-choice-Default",
                                        Skin::Solid => "skin-choice-Solid",
                                        Skin::Grain => "skin-choice-Grain",
                                    })
                                }
                                #[cfg(not(debug_assertions))]
                                {
                                    None::<AnyElement>
                                }
                            })
                    })
                    .checked(skin == selected)
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| this.set_skin(skin, cx));
                        window.activate_window();
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

pub(crate) fn appearance_picker(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    appearance_picker_sized(this, false, cx)
}

pub(crate) fn profile_appearance_picker(
    this: &DunlinApp,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    appearance_picker_sized(this, true, cx)
}

fn appearance_picker_sized(
    this: &DunlinApp,
    fixed: bool,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    segment_group(&palette(cx))
        .children(
            [
                crate::app::Appearance::System,
                crate::app::Appearance::Light,
                crate::app::Appearance::Dark,
            ]
            .map(|appearance| theme_card(appearance, this.appearance, fixed, cx)),
        )
        .into_any_element()
}

fn theme_card(
    appearance: crate::app::Appearance,
    current: crate::app::Appearance,
    fixed: bool,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let selected = appearance == current;
    let label = match appearance {
        crate::app::Appearance::System => "System",
        crate::app::Appearance::Light => "Light",
        crate::app::Appearance::Dark => "Dark",
    };

    dunlin_ui::settings::segment(format!("theme-card-{label}"), label, selected)
        .when(fixed, |button| {
            button
                .small()
                .px(px(7.))
                .h(px(dunlin_ui::settings::Tokens::CONTROL_HEIGHT - 4.))
        })
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe(match appearance {
                    crate::app::Appearance::System => "theme-choice-System",
                    crate::app::Appearance::Light => "theme-choice-Light",
                    crate::app::Appearance::Dark => "theme-choice-Dark",
                })
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .accessibility_label(format!("{label} appearance"))
        .icon(match appearance {
            crate::app::Appearance::System => IconName::Monitor,
            crate::app::Appearance::Light => IconName::Sun,
            crate::app::Appearance::Dark => IconName::Moon,
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.set_appearance(appearance, window, cx);
            // Refreshing every window during an appearance change must not
            // hand key-window status back to the application behind Settings.
            window.activate_window();
            this.persist_settings(cx);
            cx.notify();
        }))
        .into_any_element()
}

/// Local history readers do not require ACP verification, but absent agents
/// must never get a nonfunctional import action in the setup catalog.
fn agent_import_visible(name: &str, adapter: bool, cli_only: bool) -> bool {
    crate::session_import::supports_scan(name) && (adapter || cli_only)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentAction {
    Install,
    Import,
    SetDefault,
    Default,
}

impl AgentAction {
    fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Import => "Import sessions",
            Self::SetDefault => "Set default",
            Self::Default => "Default",
        }
    }
}

fn agent_action(
    installed: bool,
    supports_import: bool,
    imported: bool,
    selected: bool,
) -> AgentAction {
    if !installed {
        AgentAction::Install
    } else if supports_import && !imported {
        AgentAction::Import
    } else if !selected {
        AgentAction::SetDefault
    } else {
        AgentAction::Default
    }
}

#[test]
fn agent_action_follows_setup_import_and_default_state() {
    assert_eq!(agent_action(false, true, false, true), AgentAction::Install);
    assert_eq!(agent_action(true, true, false, true), AgentAction::Import);
    assert_eq!(
        agent_action(true, true, true, false),
        AgentAction::SetDefault
    );
    assert_eq!(agent_action(true, true, true, true), AgentAction::Default);
    assert_eq!(
        agent_action(true, false, false, false),
        AgentAction::SetDefault
    );
    assert_eq!(agent_action(true, false, false, true), AgentAction::Default);
}

#[test]
fn history_actions_only_appear_for_detected_supported_agents() {
    for name in ["OpenCode", "Claude Code", "OpenAI Codex"] {
        assert!(!agent_import_visible(name, false, false));
        assert!(agent_import_visible(name, true, false));
        assert!(agent_import_visible(name, false, true));
    }
    for name in ["Gemini CLI", "Custom ACP", "Cursor"] {
        assert!(!agent_import_visible(name, true, true));
    }
}

fn harnesses(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let current = this.default_harness.clone();
    let discovery = this.harness_discovery.clone();
    let entries = crate::harness_discovery::catalog_names()
        .map(|name| (name, crate::harness::harness_command(name, "")))
        .collect::<Vec<_>>();

    let mut list = dunlin_ui::settings::group();
    let mut available_list = dunlin_ui::settings::group();
    let mut detected_count = 0;
    let mut available_count = 0;
    for (name, command) in &entries {
        let selected = current == *name;
        let detected = discovery.iter().find(|entry| entry.name == *name);
        let installed = detected
            .map(|entry| entry.adapter.is_some())
            .unwrap_or_else(|| crate::harness::command_available(command));
        let cli_only = !installed && detected.is_some_and(|entry| entry.cli.is_some());
        let live_status = this.harness_check_status(name).or_else(|| {
            (installed
                && !this.project().is_remote()
                && this.session_harness() == *name
                && !matches!(this.harness.status, crate::harness::HarnessStatus::Offline))
            .then_some(&this.harness.status)
        });
        let (status, status_color) = match live_status {
            Some(crate::harness::HarnessStatus::Ready { .. }) => (
                if this.session_harness() == *name && this.harness.status.is_ready() {
                    "Ready"
                } else {
                    "Verified"
                },
                p.green,
            ),
            Some(crate::harness::HarnessStatus::Starting) => ("Connecting", p.amber),
            Some(crate::harness::HarnessStatus::Failed(_))
            | Some(crate::harness::HarnessStatus::Exited(_)) => ("Failed", p.red),
            Some(crate::harness::HarnessStatus::Offline) => ("Offline", p.muted_fg),
            None if *name == "Custom ACP" && this.custom_harness_command.trim().is_empty() => {
                ("Not configured", p.muted_fg)
            }
            None if installed => ("Not verified", p.amber),
            None if cli_only => ("Adapter needed", p.amber),
            None => ("Not installed", p.muted_fg),
        };
        let default_name = name.to_string();
        let import_name = name.to_string();
        let install_name = name.to_string();
        let scanning =
            this.session_import.loading && this.session_import.scanning.as_deref() == Some(*name);
        let imported = this
            .all_sessions
            .iter()
            .filter(|session| {
                session.harness == *name
                    && crate::session_import::imported_source(&session.id, name).is_some()
            })
            .count()
            + this
                .session_import
                .pending
                .iter()
                .filter(|session| session.harness == *name)
                .count();
        let last_import = this.session_import.last_import.get(*name).copied();
        let automatic = agent_import_visible(name, installed, cli_only);
        let stage = agent_action(
            installed,
            automatic,
            imported > 0 || last_import.is_some(),
            selected,
        );
        let checking = live_status.is_some_and(crate::harness::HarnessStatus::is_busy);

        let action = settings_button(format!("agent-action:{name}"), if scanning { "Importing…" } else if checking { "Connecting…" } else { stage.label() }, false, &p)
                .disabled(stage == AgentAction::Default || checking || (stage == AgentAction::Import && this.session_import.loading)
                    || (stage == AgentAction::SetDefault && this.harness_checks.values().any(crate::harness::HarnessStatus::is_busy))
                    || (stage == AgentAction::SetDefault && (this.project().is_remote() || !this.project().trusted)))
                .tooltip(if stage == AgentAction::SetDefault { "Verify the connection and use this agent for new conversations. Requires a trusted local project; no prompt is sent." } else { stage.label() })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    match stage {
                        AgentAction::Install => this.show_harness_setup(&install_name, cx),
                        AgentAction::Import => this.scan_session_import(&import_name, cx),
                        AgentAction::SetDefault => {
                            this.set_default_harness(&default_name, cx);
                        }
                        AgentAction::Default => {}
                    }
                }))
                .into_any_element();

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
        let detected_locally = installed
            || cli_only
            || selected
            || (*name == "Custom ACP"
                && !this.custom_harness_command.trim().is_empty()
                && this.custom_harness_command.trim() != "your-agent --acp");
        let row = div()
            .id(format!("harness-row:{name}"))
            .min_h(px(68.))
            .py(px(12.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .size(px(32.))
                    .flex_shrink_0()
                    .rounded(px(dunlin_ui::radii::CONTROL))
                    .bg(p.subtle)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::harness_brand::logo(name, 20., p.fg)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(140.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(dunlin_ui::settings::Tokens::BODY_SIZE))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.fg)
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(dot(status_color, 5.0))
                            .child(
                                div()
                                    .text_size(px(dunlin_ui::settings::Tokens::CAPTION_SIZE))
                                    .text_color(p.muted_fg)
                                    .child(status),
                            ),
                    ),
            )
            .child(div().flex_shrink_0().flex().justify_end().child(action));
        let import_status = if scanning {
            this.session_import
                .progress_label()
                .unwrap_or_else(|| "Importing…".into())
        } else {
            let time = last_import
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| "unknown".into());
            format!("{imported} sessions imported · Last import: {time}")
        };
        let refresh_name = name.to_string();
        let second = div()
            .pb(px(16.))
            .pl(px(42.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .when(
                automatic && (scanning || imported > 0 || last_import.is_some()),
                |view| {
                    view.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(dunlin_ui::settings::description(import_status, p).flex_1())
                            .child(
                                icon_button(
                                    format!("refresh-sessions:{name}"),
                                    IconName::RefreshCw,
                                    "Check for newer session data",
                                )
                                .when(
                                    scanning && !this.reduce_motion && !cx.reduce_motion(),
                                    |button| {
                                        button.icon(
                                            component::spinner::Spinner::new()
                                                .icon(IconName::RefreshCw),
                                        )
                                    },
                                )
                                .disabled(this.session_import.loading)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.scan_session_import(&refresh_name, cx);
                                    },
                                )),
                            ),
                    )
                },
            )
            .when_some(
                this.session_import
                    .notices
                    .get(*name)
                    .filter(|note| !note.is_empty())
                    .cloned(),
                |view, note| view.child(div().text_size(px(10.5)).text_color(p.amber).child(note)),
            );
        let row = div()
            .child(row)
            .when(detected_locally, |row| row.child(second))
            .when(this.harness_setup.as_deref() == Some(*name), |row| {
                row.child(harness_setup(this, name, cx, &p))
            });
        if detected_locally {
            if detected_count > 0 {
                list = list.child(divider(&p));
            }
            list = list.child(row);
            detected_count += 1;
        } else if *name != "Custom ACP" {
            if available_count > 0 {
                available_list = available_list.child(divider(&p));
            }
            available_list = available_list.child(row);
            available_count += 1;
        }
    }

    let body = dunlin_ui::settings::page()
        .when_some(this.project().remote.clone(), |body, remote| body.child(
            settings_card(&p).child(setting_row("Local agent settings", "This page always configures this Mac. Select a trusted local project to test adapters; server agents are managed separately.",
                settings_button("agents-manage-server", "Manage server…", false, &p)
                    .on_click(cx.listener(move |this, _, window, cx| this.manage_server(&remote.host_id, window, cx)))
                    .into_any_element(), &p))))
        .child(
            div()
                .flex()
                .items_center()
                .child(section_label("Local agents", &p))
                .child(div().flex_1())
                .child(
                    settings_button("rescan-harnesses", if this.harness_scanning { "Scanning…" } else { "Scan again" }, false, &p)
                    .icon(IconName::RefreshCw)
                    .loading(this.harness_scanning)
                    .disabled(this.harness_scanning)
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
        .when(!this.project().is_remote() && !this.project().trusted, |body| body.child(
            dunlin_ui::settings::description("Connection tests require a trusted local project. Setup instructions and read-only history imports remain available.", p)))
        .child(
            div()
                .text_size(px(11.0))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("Default for new conversations. Set default verifies the connection; projects and groups can override this choice."),
        )
        .child(crate::free_endpoint_consent::settings(this, cx))
        .child(list)
        .when(detected_count == 0, |body| body.child(dunlin_ui::settings::notice(
            "No local agents detected. Set up an agent below, then scan again. Nothing is installed or started automatically.", p)))
        .child(dunlin_ui::settings::actions().child(settings_button("available-agents", if this.settings_ui.available_agents { format!("Hide available agents ({available_count})") } else { format!("Set up another agent ({available_count})") }, false, &p)
            .self_start()
            .relative().children({
                #[cfg(debug_assertions)] { crate::workspace_ui_smoke::probe("available-agents") }
                #[cfg(not(debug_assertions))] { None::<AnyElement> }
            })
            .icon(if this.settings_ui.available_agents { IconName::ChevronUp } else { IconName::ChevronDown })
            .on_click(cx.listener(|this, _, _, cx| { this.settings_ui.available_agents = !this.settings_ui.available_agents; cx.notify(); })))
            .child(settings_button("browse-acp-agents", "Browse ACP catalog", false, &p)
                .icon(IconName::ExternalLink)
                .on_click(|_, _, cx| cx.open_url("https://agentclientprotocol.com/get-started/agents"))))
        .when(this.settings_ui.available_agents, |body| body.child(available_list))
        .child(
            div()
                .text_size(px(11.0))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child("Import copies local histories without changing source files or resuming agents. Sessions are automatically assigned by directory, including when a matching project is added later. Reimport enriches existing copies without replacing local follow-up turns."),
        )
        .child({
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section_label("Connection", &p))
                .child(
                    settings_card(&p)
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
            choice_card(
                format!("permission-mode:{title}"),
                format!("Default agent permissions: {title}"),
                selected,
                p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.permission_mode = mode;
                this.persist_settings(cx);
                this.set_notice(format!("Permission mode · {}", mode.label()), cx);
            }))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .whitespace_normal()
                    .child(choice_indicator(selected, p))
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
            ),
        );
    }

    let body = dunlin_ui::settings::page()
        .child(section_label("Agent approval policy", &p))
        .child(dunlin_ui::settings::description("Global default. Groups and projects may override this policy. It controls ACP approval requests, not operating-system access.", p))
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
                    "Local agents run with your user permissions. Remote servers apply their configured platform sandbox; this approval policy is not a container.",
                    scope_value("Not enforced", p.red, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Network access",
                    "This approval policy does not restrict a local agent’s network access. A sandboxed audit check is a separate, explicitly launched tool.",
                    scope_value("Not enforced", p.red, &p),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Secrets",
                    "Tool output can contain secrets. Log exports redact known credentials, not every possible secret. Review content before sharing.",
                    scope_value("Review required", p.amber, &p),
                    &p,
                )),
        )
        .child(section_label("macOS permissions · this Mac", &p))
        .child(dunlin_ui::settings::description("Separate from agent approval. These permissions are granted by macOS, not by an ACP permission mode.", p))
        .child({
            let mut card = settings_card(&p);
            let statuses = crate::platform::permission_statuses();
            let count = statuses.len();
            for (index, status) in statuses.into_iter().enumerate() {
                if matches!(status.kind, crate::platform::PermissionKind::Notifications) { continue; }
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
        })
        .child(settings_card(&p).child(notification_permission_row(this, cx, &p)));

    body.into_any_element()
}

fn editor(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let word_wrap = this.word_wrap;
    let autosave = this.autosave;

    let body = dunlin_ui::settings::page()
        .child(section_label("Typography", &p))
        .child(
            dunlin_ui::settings::group()
                .child(setting_row(
                    "Code font",
                    "Monospaced face for files, timelines and diffs.",
                    font_picker(this, true, cx),
                    &p,
                ))
                .child(divider(&p))
                .child(setting_row(
                    "Font size",
                    "Applies to the editor, diffs and the live preview below.",
                    stepper(this, cx, &p),
                    &p,
                )),
        )
        .child(editor_font_preview(this, &p))
        .child(section_label("Editing behavior", &p))
        .child(dunlin_ui::settings::group()
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
                    "Save edited files after a short pause. External-change conflicts still require review.",
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
            dunlin_ui::settings::group()
                .child(setting_row(
                    "Shell integration",
                    "Enable command and working-directory tracking for zsh/bash. Disable if your custom prompt behaves incorrectly. Applies to new terminals.",
                    toggle_switch(
                        "shell-integration",
                        this.shell_integration,
                        |this| this.shell_integration = !this.shell_integration,
                        cx,
                        &p,
                    ),
                    &p,
                )),
        );

    body.into_any_element()
}

fn editor_font_preview(this: &DunlinApp, p: &Palette) -> AnyElement {
    settings_card(p)
        .overflow_hidden()
        .child(
            dunlin_ui::settings::card_header(*p)
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
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
            // The editor paints an opaque gutter; keep that paint above
            // the bottom curves, without changing the ordinary fields.
            div()
                .pb(px(SurfaceGeometry::SETTINGS_CARD.inner_radius()))
                .child(
                    Editor::new(&this.editor_preview)
                        .h(px(210.))
                        .appearance(false)
                        .bordered(false)
                        .readonly(true)
                        .aria_label("Live editor font preview"),
                ),
        )
        .into_any_element()
}

fn usage(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    this.ensure_usage(cx);
    this.ensure_billing(cx);
    let mut tabs = segment_group(&p);
    for (report, label) in [(false, "Limits & alerts"), (true, "Cost report")] {
        tabs = tabs.child(
            segment(
                format!("usage-page:{report}"),
                label,
                this.settings_ui.usage_report == report,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_ui.usage_report = report;
                cx.notify();
            })),
        );
    }
    if this.settings_ui.usage_report {
        return dunlin_ui::settings::page()
            .child(tabs)
            .child(billing_usage(this, cx, &p))
            .into_any_element();
    }
    let mut usages = Vec::new();
    if let Some(live) = this.session_usage() {
        usages.push(live);
    }
    usages.extend(this.active_usages());

    let mut budgets = segment_group(&p);
    for (label, value) in [
        ("Off", None),
        ("$50", Some(50.)),
        ("$100", Some(100.)),
        ("$250", Some(250.)),
    ] {
        budgets = budgets.child(
            segment(
                format!("billing-budget:{label}"),
                label,
                this.billing_budget_usd == value,
                &p,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.billing_budget_usd = value;
                this.persist_settings(cx);
                cx.notify();
            })),
        );
    }
    let mut body = dunlin_ui::settings::page().child(tabs)
        .child(section_label("Budget alerts", &p))
        .child(settings_card(&p)
            .child(setting_row("Monthly alert budget", "An alert threshold based on known local costs, not a provider spending cap. Unknown prices are excluded.", budgets.into_any_element(), &p))
            .child(divider(&p))
            .child(setting_row("Budget alerts", "Show local budget alerts. This does not change your provider subscription or stop agent work.",
                dunlin_ui::switch("billing-alerts", "Budget alerts", this.billing_alerts)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.billing_alerts = *checked; this.persist_settings(cx); cx.notify();
                    })).into_any_element(), &p)))
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

    if usages.is_empty() {
        body = body.child(dunlin_ui::settings::notice("No subscription usage is available yet. Connect an agent or refresh provider usage. Budget settings above remain available without a log scan.", p));
    }
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
                            .rounded(px(dunlin_ui::radii::CONTROL))
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
                    .child(progress_bar(
                        percent,
                        if percent > 85. {
                            p.red
                        } else {
                            p.fg.opacity(0.65)
                        },
                        p.muted,
                    )),
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
                    .disabled(this.billing_loading)
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
                        .disabled(this.billing_syncing)
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
                            .disabled(this.remote_billing_loading)
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
                    .rounded(px(dunlin_ui::radii::PANEL))
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
    let summary_card = settings_card(p)
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
                        .rounded(px(dunlin_ui::radii::CONTROL))
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
                .rounded(px(dunlin_ui::radii::CONTROL))
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
                .rounded(px(dunlin_ui::radii::CONTROL))
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
        dunlin_ui::settings::card_header(*p)
            .px(px(9.))
            .py(px(6.))
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
        dunlin_ui::settings::card_header(*p)
            .px(px(9.))
            .py(px(6.))
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
        .rounded(px(dunlin_ui::radii::ROW))
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
            dunlin_ui::settings::card_header(*p)
                .px(px(9.))
                .py(px(6.))
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
        dunlin_ui::settings::card_header(*p)
            .px(px(9.))
            .py(px(6.))
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

    let body = dunlin_ui::settings::page()
        .child(section_label("Agent logs", &p))
        .child(
            settings_card(&p)
                .child(setting_row(
                    "ACP logs",
                    &format!(
                        "All harnesses · lifecycle, protocol diagnostics, stderr and errors · {}",
                        crate::logging::display_path()
                    ),
                    div().flex().items_center().gap(px(8.)).child(settings_button(
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
                    .on_click(cx.listener(|this, _, _, cx| this.open_acp_log(cx))))
                    .child(settings_button("export-acp-logs", if this.acp_log_exporting { "Exporting…" } else { "Export ZIP…" }, false, &p)
                        .disabled(this.acp_log_exporting)
                        .tooltip("Export current and rotated logs. Known credentials are redacted; review adapter messages before sharing.")
                        .on_click(cx.listener(|this, _, _, cx| this.export_acp_logs(cx))))
                    .into_any_element(),
                    &p,
                )),
        )
        .when_some(this.acp_log_error.clone(), |body, error| {
            body.child(
                div()
                    .rounded(px(dunlin_ui::radii::PANEL))
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
                            .child(settings_text_button("refresh-acp-log", "Refresh", p.muted_fg, &p)
                                .on_click(cx.listener(|this, _, _, cx| this.open_acp_log(cx))))
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
                            .rounded(px(dunlin_ui::radii::ROW))
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
        .child(section_label("Troubleshooting", &p))
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
                                .child("Collect local database health, project/session counts, agent discovery, permission status and paired hosts. Review before sharing; this does not test GUI behavior or grant macOS permissions."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            settings_button("qa-run", "Run diagnostics", false, &p).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.qa_report = Some(crate::qa::capture(this));
                                    this.qa_error = None;
                                    this.set_notice("Environment report ready", cx);
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
                                    this.set_notice("Copied the environment report", cx);
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
                            .rounded(px(dunlin_ui::radii::ROW))
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
                        .rounded(px(dunlin_ui::radii::CONTROL))
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

fn remote_settings(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);

    let mut body = dunlin_ui::settings::page()
        .child(crate::connection_map::view::render(this, window, cx))
        .child(
            dunlin_ui::settings::actions()
                .child(section_label("Paired servers", &p))
                .child(div().flex_1())
                .child(
                    dunlin_ui::settings::button("settings-connect-server")
                        .primary()
                        .label("Connect to server…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_remote_connection(window, cx)
                        })),
                ),
        );

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
                        .child("Use Connect to server with a private Tailscale HTTPS endpoint and a relay login key."),
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
                                .rounded(px(dunlin_ui::radii::CONTROL))
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

    if let Some(id) = &this.server_panel.host {
        let name = this
            .remote_host(id)
            .map(|host| host.name)
            .unwrap_or_else(|| id.clone());
        body = body
            .child(section_label(&format!("Manage server: {name}"), &p))
            .child(
                settings_card(&p)
                    .p(px(14.))
                    .child(crate::server_panel::render(this, cx)),
            );
    } else {
        body = body.child(section_label("Server setup", &p)).child(
            settings_card(&p)
                .p(px(14.))
                .child(crate::server_panel::render(this, cx)),
        );
    }

    body.into_any_element()
}

impl DunlinApp {
    pub(crate) fn export_acp_logs(&mut self, cx: &mut Context<Self>) {
        if self.acp_log_exporting {
            return;
        }
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let name = format!(
            "dunlin-acp-logs-{}.zip",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        self.acp_log_exporting = true;
        self.acp_log_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(path))) => {
                    cx.background_executor()
                        .spawn(async move { crate::logging::export_zip(&path).map(|_| Some(path)) })
                        .await
                }
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.acp_log_exporting = false;
                match result {
                    Ok(Some(path)) => this.set_notice(
                        format!(
                            "Logs exported to {} · review before sharing",
                            path.display()
                        ),
                        cx,
                    ),
                    Ok(None) => {}
                    Err(error) => {
                        this.acp_log_error = Some(format!("Could not export ACP logs: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

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

fn settings_page(content: AnyElement, _tab: SettingsTab) -> AnyElement {
    div()
        .w_full()
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
                .children({
                    #[cfg(debug_assertions)]
                    {
                        Some(smoke::page_marker(_tab))
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .child(content),
        )
        .into_any_element()
}

fn section_label(label: &str, p: &Palette) -> AnyElement {
    dunlin_ui::settings::section(label.to_owned(), *p).into_any_element()
}

fn settings_card(p: &Palette) -> Div {
    dunlin_ui::settings::card(*p).px(px(18.))
}

fn settings_chip(label: impl Into<SharedString>, p: &Palette) -> Div {
    badge(label, p.muted_fg, p.muted, *p)
}

fn risk_chip(p: &Palette) -> Div {
    div()
        .h(px(17.))
        .px(px(5.))
        .rounded(px(dunlin_ui::radii::CODE))
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
    dunlin_ui::settings::segments(*p)
}

fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    _p: &Palette,
) -> Button {
    dunlin_ui::settings::segment(id, label, active)
}

fn settings_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    _p: &Palette,
) -> Button {
    dunlin_ui::settings::button(id)
        .label(label)
        .when(primary, |button| button.primary())
}

fn settings_text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: Hsla,
    p: &Palette,
) -> Button {
    dunlin_ui::settings::button(id)
        .label(label)
        .when(color == p.red, |button| button.danger().outline())
        .when(color == p.amber, |button| button.warning().outline())
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
    dunlin_ui::settings::row(title, description, control, *p)
        .px(px(0.))
        .into_any_element()
}

fn divider(p: &Palette) -> AnyElement {
    dunlin_ui::settings::divider(*p).into_any_element()
}

fn toggle_switch(
    id: &'static str,
    value: bool,
    action: fn(&mut DunlinApp),
    cx: &mut Context<DunlinApp>,
    _p: &Palette,
) -> AnyElement {
    let label = match id {
        "restore-session" => "Restore the last session",
        "notifications" => "Desktop notifications",
        "shortcut-hints" => "Show keyboard shortcut hints",
        "reduce-motion" => "Reduce motion",
        "dashboard-agents" => "Dashboard live agents",
        "dashboard-activity" => "Dashboard activity",
        "show-file-sizes" => "Show file sizes",
        "project-branches" => "Show branch names",
        "project-language" => "Show primary language",
        "project-topics" => "Show repository topics",
        "project-wrap-names" => "Wrap project names",
        "project-info-preview" => "Project information",
        "stream-responses" => "Stream responses",
        "chat-timestamps" => "Message times",
        "chat-duration" => "Agent turn duration",
        "chat-logo" => "Response logo",
        "show-reasoning" => "Show reasoning by default",
        "final-answer-only" => "Final answer only",
        "harness-auto-connect" => "Auto-connect",
        "harness-restart" => "Restart on crash",
        "word-wrap" => "Word wrap",
        "autosave" => "Autosave",
        "editor-minimap" => "File minimap",
        "inline-blame" => "Inline blame",
        "shell-integration" => "Shell integration",
        "security-enabled" => "Enable security audit tools",
        _ => id,
    };
    dunlin_ui::switch(id, label, value)
        .on_click(cx.listener(move |this, _, window, cx| {
            action(this);
            dunlin_ui::set_motion_preference(this.reduce_motion, cx);
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
    dunlin_ui::settings::button(if code {
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
        let default_font = crate::app::StoredSettings::default().ui_font_family;
        let families = ordered_fonts(
            &this.available_fonts,
            (!code).then_some(default_font.as_str()),
        );
        for family in families {
            if code && !code_font_candidate(family) && family != selected {
                continue;
            }
            let name = family.clone();
            let is_default = !code && name == default_font;
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
                            .child(if is_default {
                                format!("{name} (Default)")
                            } else {
                                name.clone()
                            }),
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

fn ordered_fonts<'a>(fonts: &'a [String], default: Option<&str>) -> Vec<&'a String> {
    let mut ordered: Vec<_> = fonts.iter().collect();
    ordered.sort_by_key(|family| Some(family.as_str()) != default);
    ordered
}

#[test]
fn default_interface_font_is_always_first_without_duplicates() {
    let fonts = vec!["Arial".into(), "Geist".into(), "Helvetica".into()];
    assert_eq!(
        ordered_fonts(&fonts, Some("Geist")),
        [&fonts[1], &fonts[0], &fonts[2]]
    );
    assert_eq!(
        ordered_fonts(&fonts, None),
        fonts.iter().collect::<Vec<_>>()
    );
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
        .rounded(px(dunlin_ui::radii::ROW))
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
