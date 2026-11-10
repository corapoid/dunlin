//! Fresh, isolated native settings acceptance. No agent, credential or network writes.
use super::*;
use crate::app::{Appearance, StoredSettings, UiDensity};
use dunlin_ui::component::TitleBar;
use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

thread_local! {
    static PAINTED_PAGE: Cell<Option<SettingsTab>> = const { Cell::new(None) };
}

pub(super) fn page_marker(tab: SettingsTab) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |_, _, _, _| PAINTED_PAGE.with(|page| page.set(Some(tab))),
    )
    .absolute()
    .inset_0()
    .size_full()
    .into_any_element()
}

/// Opt-in visual evidence from the real settings window, using fixture data only.
/// Never captures the desktop or automates screen-recording consent.
fn capture(window: &Window, path: &std::path::Path) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).unwrap();
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // The fixture owns this NSView on the AppKit main thread.
    let view: &objc2_app_kit::NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
    let native = view.window().unwrap();
    let result = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-x", "-l", &native.windowNumber().to_string()])
        .arg(path)
        .status()
        .expect("capture fixture settings window");
    assert!(
        result.success(),
        "Window capture unavailable; no TCC consent was automated"
    );
}

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--settings-smoke needs a fresh fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-settings-smoke-")
    );
    fs::create_dir(&dir).expect("fixture directory must not already exist");
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("navigation-fixture.rs"),
        "// Global search fixture\n",
    )
    .unwrap();
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                default_harness: if std::env::var_os("DUNLIN_SETTINGS_CAPTURE").is_some() {
                    "OpenCode"
                } else {
                    "Custom ACP"
                }
                .into(),
                opencode_provider: None,
                custom_harness_command: String::new(),
                auto_connect: false,
                restore_session: false,
                vault_enabled: false,
                notifications_enabled: false,
                onboarding_step: 2,
                reduce_motion: true,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "settings-fixture".into(),
            name: "Settings fixture".into(),
            path: root.to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: false,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        })
        .unwrap();
    drop(store);
    // This entry point runs before application/worker threads. All scans see an
    // empty fixture home, not real account catalogs, plugins or provider logs.
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("PATH", "/usr/bin:/bin");
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    crate::workspace_ui_smoke::enable_probes();
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.settings-smoke", "Dunlin Settings Smoke");
        dunlin_ui::init(cx);
        crate::logging::init();
        let (parent, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(50.), px(50.)), size(px(1100.), px(720.))))),
            ..TitleBar::window_options()
        }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        cx.activate(true);
        owner.update(cx, |app, cx| { app.settings_open = true; app.settings_tab = SettingsTab::General; cx.notify(); });
        let reset_dir = dir.clone();
        cx.on_app_restart(move |_| {
            let store = dunlin_storage::Store::open(reset_dir.join("dunlin.db")).unwrap();
            assert!(store.projects().unwrap().is_empty());
            assert!(store.sessions(None).unwrap().is_empty());
            assert!(store.load_setting::<StoredSettings>("app").unwrap().is_none());
            assert!(reset_dir.join("project").is_dir(), "reset must preserve repository files");
            fs::write(reset_dir.join("PASS"), "Settings matrix; avatar positioning/theme/density; global search suggestions and file navigation; unified sidebar collapse/footer; reset blocking/cancellation and confirmed fixture reset passed\n").unwrap();
            println!("PASS: settings, avatar menu, global search, unified sidebar and fixture factory reset");
            // The fixture verifies the restart boundary without relaunching the
            // smoke executable (which would reuse its already-created directory).
            std::process::exit(0);
        }).detach();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let pages: Vec<_> = navigation::GROUPS.iter().flat_map(|(_, pages)| pages.iter().copied()).collect();
             let mut index = 0;
             let mut matrix_prepared = false;
             let mut visual_scrolled = false;
              let mut credential_phase = 0;
              let mut agent_catalog = false;
            let total = pages.len() * 8;
            let mut stage = 0;
            let mut expanded_footer = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(90), "settings smoke stalled: page {index}, stage {stage}");
                    let visual = std::env::var_os("DUNLIN_SETTINGS_CAPTURE").is_some();
                    if visual { cx.activate(true); }
                     parent.update(cx, |_, window, cx| { window.refresh(); if !visual { window.draw(cx).clear(cx); } }).unwrap();
                     match stage {
                         80 if !owner.read(cx).overlay_window.is_open() => {
                             parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("title-user-menu").center(), cx)).unwrap();
                             stage = 81;
                             return false;
                         }
                         81 => {
                             let avatar = crate::workspace_ui_smoke::measured("title-user-menu");
                             let menu = crate::workspace_ui_smoke::measured("profile-menu");
                              assert!(menu.top() > avatar.bottom(), "user menu must open below the avatar");
                              assert!(menu.size.width <= px(244.), "profile menu fits its contents without a wide empty right edge");
                             parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("theme-choice-System").center(), cx)).unwrap();
                             assert_eq!(owner.read(cx).appearance, Appearance::System, "profile menu must support System, not just a light/dark toggle");
                             stage = 82;
                             return false;
                         }
                         82 => {
                             parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("theme-choice-Dark").center(), cx)).unwrap();
                             assert_eq!(owner.read(cx).appearance, Appearance::Dark);
                             stage = 83;
                             return false;
                         }
                         83 => {
                             parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("profile-density").center(), cx)).unwrap();
                              assert_eq!(owner.read(cx).density, UiDensity::Compact, "Minimal view must enable compact density");
                              parent.update(cx, |_, window, cx| {
                                  let viewport = window.viewport_size();
                                  crate::workspace_ui_smoke::click(window, point(viewport.width - px(8.), viewport.height - px(8.)), cx);
                              }).unwrap();
                              stage = 85;
                              return false;
                          }
                          85 if !owner.read(cx).user_profile.menu_open => {
                              let search = crate::workspace_ui_smoke::measured("global-search-trigger");
                              parent.update(cx, |_, window, cx| {
                                  assert!((search.center().x - window.viewport_size().width / 2.).abs() < px(1.), "global search must be centered in the window");
                                  crate::workspace_ui_smoke::click(window, search.center(), cx);
                              }).unwrap();
                              assert!(owner.read(cx).command_open);
                              assert_eq!(owner.read(cx).filtered_commands()[0].command, crate::app::Command::Section(dunlin_domain::SidebarSection::Dashboard));
                              stage = 86;
                              return false;
                          }
                          86 => {
                              parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                  app.command_query.update(cx, |input, cx| input.set_value("navigation-fixture", window, cx));
                                  // Programmatic fixture assignment does not emit the
                                  // user-input Change event that maintains this mirror.
                                  app.command_text = "navigation-fixture".into();
                              })).unwrap();
                              stage = 87;
                              return false;
                          }
                          87 => {
                              let items = owner.read(cx).filtered_commands();
                              assert_eq!(items[0].command, crate::app::Command::Section(dunlin_domain::SidebarSection::Dashboard), "Dashboard stays pinned while searching");
                              assert_eq!(items[1].label, "navigation-fixture.rs");
                              parent.update(cx, |_, window, cx| {
                                  crate::controls_smoke::key(window, cx, "down");
                                  crate::controls_smoke::key(window, cx, "enter");
                              }).unwrap();
                              stage = 88;
                              return false;
                          }
                          88 if !owner.read(cx).command_open && owner.read(cx).workbench.show_file => {
                              assert!(owner.read(cx).workbench.show_file, "a file search result must open the editor");
                              parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("global-search-trigger").center(), cx)).unwrap();
                              stage = 89;
                              return false;
                          }
                          89 => {
                              parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "enter")).unwrap();
                              assert_eq!(owner.read(cx).section, dunlin_domain::SidebarSection::Dashboard);
                              parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("title-sidebar").center(), cx)).unwrap();
                              stage = 94;
                              return false;
                          }
                          94 => {
                              if crate::workspace_ui_smoke::measured("workspace-surface").left() > px(dunlin_ui::scale::WORKSPACE_INSET + 1.) { return false; }
                              assert!(owner.read(cx).sidebar_collapsed, "collapse must remove the entire sidebar width, including its navigation");
                              parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("title-sidebar").center(), cx)).unwrap();
                              stage = 95;
                              return false;
                          }
                          95 => {
                              if crate::workspace_ui_smoke::measured("workspace-surface").left() < px(200.) { return false; }
                              let footer = crate::workspace_ui_smoke::measured("sidebar-footer");
                              let host = crate::workspace_ui_smoke::measured("host-menu-button");
                              let settings = crate::workspace_ui_smoke::measured("sidebar-settings");
                               assert!(footer.contains(&host.center()) && settings.right() < host.left());
                               assert_eq!(settings.size, size(px(32.), px(32.)));
                               assert!((settings.center().y - host.center().y).abs() < px(1.));
                               expanded_footer = Some(footer);
                               parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("sidebar-workspace-toggle").center(), cx)).unwrap();
                               stage = 96;
                               return false;
                           }
                           96 => {
                               assert!(owner.read(cx).workspace_navigation_collapsed);
                               let footer = crate::workspace_ui_smoke::measured("sidebar-footer");
                               assert!(footer.top() > expanded_footer.unwrap().top() + px(60.), "workspace navigation collapses toward the bottom even with minimal rows and optional tools hidden");
                               assert_eq!(footer.bottom(), expanded_footer.unwrap().bottom(), "utility row stays pinned to the bottom");
                               let settings = crate::workspace_ui_smoke::measured("sidebar-settings");
                               let host = crate::workspace_ui_smoke::measured("host-menu-button");
                               assert!(footer.contains(&settings.center()) && footer.contains(&host.center()));
                               assert!(owner.read(cx).store.as_ref().unwrap().load_setting::<StoredSettings>("app").unwrap().unwrap().workspace_navigation_collapsed);
                               parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("sidebar-workspace-toggle").center(), cx)).unwrap();
                               stage = 97;
                               return false;
                           }
                           97 => {
                               assert!(!owner.read(cx).workspace_navigation_collapsed, "workspace navigation can be expanded again");
                               assert_eq!(crate::workspace_ui_smoke::measured("sidebar-footer"), expanded_footer.unwrap());
                               crate::workspace_ui_smoke::measured("sidebar-source-control");
                               assert!(!owner.read(cx).store.as_ref().unwrap().load_setting::<StoredSettings>("app").unwrap().unwrap().workspace_navigation_collapsed);
                               owner.update(cx, |app, cx| { app.sidebar_width = 220.; cx.notify(); });
                               stage = 98;
                               return false;
                           }
                           98 => {
                               let footer = crate::workspace_ui_smoke::measured("sidebar-footer");
                               let settings = crate::workspace_ui_smoke::measured("sidebar-settings");
                               let host = crate::workspace_ui_smoke::measured("host-menu-button");
                               assert!(settings.right() < host.left() && host.right() <= footer.right(), "utility row fits the narrowest sidebar");
                               assert!((settings.center().y - host.center().y).abs() < px(1.));
                               parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("sidebar-tab-projects").center(), cx)).unwrap();
                               stage = 100;
                               return false;
                           }
                           100 => {
                               parent.update(cx, |_, window, cx| {
                                   for key in ["s", "e", "t", "t", "i", "n", "g", "s"] { crate::controls_smoke::key(window, cx, key); }
                               }).unwrap();
                               stage = 101;
                               return false;
                           }
                           101 => {
                               assert_eq!(owner.read(cx).project_search.query, "settings");
                               assert_eq!(owner.read(cx).project_search.current(), Some("settings-fixture"));
                               assert!(!owner.read(cx).command_open, "typing in Projects uses inline name search, not the global palette");
                               parent.update(cx, |_, window, cx| {
                                   crate::controls_smoke::key(window, cx, "backspace");
                                   crate::controls_smoke::key(window, cx, "down");
                                   crate::controls_smoke::key(window, cx, "enter");
                                   crate::controls_smoke::key(window, cx, "escape");
                               }).unwrap();
                               assert!(owner.read(cx).project_search.query.is_empty());
                               parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("sidebar-settings").center(), cx)).unwrap();
                               stage = 99;
                               return false;
                           }
                           99 if owner.read(cx).settings_open => {
                               assert_eq!(owner.read(cx).settings_tab, SettingsTab::General, "Settings icon opens application settings");
                               owner.update(cx, |app, cx| { app.settings_open = true; app.settings_tab = SettingsTab::Data; cx.notify(); });
                              stage = 84;
                              return false;
                          }
                         _ => {}
                     }
                     let Some(child) = owner.read(cx).overlay_window.debug_window() else { return false; };
                    child.update(cx, |_, window, cx| { if visual { window.activate_window(); } window.refresh(); if !visual { window.draw(cx).clear(cx); } }).unwrap();
                     if index < total {
                         if matrix_prepared {
                         if visual && !PAINTED_PAGE.with(|page| page.get() == Some(pages[index % pages.len()])) { return false; }
                         child.update(cx, |_, window, _| {
                             #[cfg(target_os = "macos")]
                             {
                                 use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                                 let RawWindowHandle::AppKit(handle) = window.window_handle().unwrap().as_raw() else { panic!("native settings window") };
                                 let view: &objc2_app_kit::NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
                                 let layer = view.layer().unwrap();
                                 assert!(layer.masksToBounds(), "whole overlay plane must clip its desktop corners");
                                 assert_eq!(layer.cornerRadius(), 12.);
                             }
                             crate::workspace_ui_smoke::assert_painted_corners(window, "settings-panel", true);
                             crate::workspace_ui_smoke::assert_painted_corners(window, "settings-content", false);
                             dunlin_ui::surface_test::assert_region_corners(window,
                                 crate::workspace_ui_smoke::measured("settings-content"),
                                 &format!("settings page {:?}", pages[index % pages.len()]));
                         }).unwrap();
                         let bounds = crate::workspace_ui_smoke::measured("settings-panel");
                        let viewport = child.update(cx, |_, window, _| window.viewport_size()).unwrap();
                        assert!(bounds.left() >= px(0.) && bounds.top() >= px(0.));
                        assert!(bounds.right() <= viewport.width && bounds.bottom() <= viewport.height);
                        let page = crate::workspace_ui_smoke::measured("settings-page");
                        let content = crate::workspace_ui_smoke::measured("settings-content");
                         assert!(page.size.width <= content.size.width, "{:?} must fit the content width", owner.read(cx).settings_tab);
                          assert_eq!(owner.read(cx).settings_tab, pages[index % pages.len()]);
                          if pages[index % pages.len()] == SettingsTab::Harnesses {
                              let action = crate::workspace_ui_smoke::measured("claude-harness-action");
                              assert!(action.size.height >= px(28.), "agent actions need a usable click target");
                          }
                          if std::env::var_os("DUNLIN_SETTINGS_CAPTURE").is_some()
                               && matches!(index / pages.len(), 4 | 5) {
                               let suffix = if pages[index % pages.len()] == SettingsTab::Secrets {
                                   match credential_phase { 1 => "-hover", 2 => "-pressed", 3 => "-released", 4 => "-form", _ => "" }
                                } else if visual_scrolled { "-bottom" } else if agent_catalog { "-catalog" } else { "" };
                               let path = dir.join(format!("settings-{:?}-{}{}.png", pages[index % pages.len()], index / pages.len(), suffix));
                                child.update(cx, |_, window, _| capture(window, &path)).unwrap();
                                if pages[index % pages.len()] == SettingsTab::Harnesses && !agent_catalog && !visual_scrolled {
                                    child.update(cx, |_, window, cx| {
                                        crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("available-agents").center(), cx);
                                    }).unwrap();
                                    assert!(owner.read(cx).settings_ui.available_agents, "setup catalog must open on click");
                                    agent_catalog = true;
                                    return false;
                                }
                                if agent_catalog && !visual_scrolled {
                                    owner.update(cx, |app, cx| { app.settings_ui.available_agents = false; cx.notify(); });
                                    agent_catalog = false;
                                }
                               if pages[index % pages.len()] == SettingsTab::Secrets {
                                   child.update(cx, |_, window, cx| {
                                       let button = crate::workspace_ui_smoke::measured("vault-refresh");
                                       match credential_phase {
                                           0 => { window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: button.center(), ..Default::default() }), cx); }
                                           1 => { window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: button.center(), button: MouseButton::Left, click_count: 1, ..Default::default() }), cx); }
                                           2 => {
                                               let outside = point(content.left() + px(4.), content.top() + px(4.));
                                               window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: outside, pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                                               window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: outside, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                                           }
                                           3 => { owner.update(cx, |app, cx| app.new_vault_draft(window, cx)); }
                                           4 => { owner.update(cx, |app, cx| { app.vault.ui.draft = None; cx.notify(); }); }
                                           _ => unreachable!(),
                                       }
                                   }).unwrap();
                                   if credential_phase < 4 { credential_phase += 1; return false; }
                                   credential_phase = 0;
                               }
                              if !visual_scrolled && matches!(pages[index % pages.len()], SettingsTab::Harnesses | SettingsTab::Extensions) {
                                  child.update(cx, |_, window, cx| {
                                      window.dispatch_event(PlatformInput::Scroll(ScrollEvent {
                                          position: content.center(),
                                          delta: ScrollDelta::Pixels(point(px(0.), px(-10000.))),
                                          ..Default::default()
                                      }), cx);
                                  }).unwrap();
                                  visual_scrolled = true;
                                  return false;
                              }
                              visual_scrolled = false;
                          }
                         index += 1;
                         matrix_prepared = false;
                         return false;
                         }
                        let tab = pages[index % pages.len()];
                        let variant = index / pages.len();
                        parent.update(cx, |_, window, cx| {
                            if index % pages.len() == 0 {
                                window.resize(if variant < 4 { size(px(1100.), px(720.)) } else { size(px(1360.), px(860.)) });
                            }
                            owner.update(cx, |app, cx| {
                                if index % pages.len() == 0 {
                                    app.set_appearance(if variant % 2 == 0 { Appearance::Light } else { Appearance::Dark }, window, cx);
                                    app.set_ui_density(if variant % 4 < 2 { UiDensity::Comfortable } else { UiDensity::Compact }, cx);
                                }
                                 app.settings_tab = tab;
                                 if tab == SettingsTab::Harnesses {
                                     // Deterministic detected/CLI-only/absent cases, never executed.
                                     app.harness_discovery = crate::harness_discovery::SPECS.iter().map(|spec| crate::harness_discovery::Detected {
                                         name: spec.name,
                                         cli: matches!(spec.name, "OpenCode" | "Claude Code").then(|| "/usr/bin/false".into()),
                                         adapter: (spec.name == "OpenCode").then(|| "/usr/bin/false".into()),
                                     }).collect();
                                 }
                                app.settings_ui.audits = tab == SettingsTab::Security && variant % 2 == 1;
                                app.settings_ui.usage_report = tab == SettingsTab::Usage && variant % 2 == 1;
                                cx.notify();
                            });
                        }).unwrap();
                         matrix_prepared = true;
                        return false;
                    }
                    match stage {
                         0 => {
                             println!("Settings render matrix completed in {:?}", started.elapsed());
                             owner.update(cx, |app, cx| {
                                 app.settings_tab = SettingsTab::Skills;
                                 app.add_catalog_skill(crate::skills::PLUGIN_BUILDER_SKILL, cx);
                                 app.install_skill_scoped(crate::skills::PLUGIN_BUILDER_SKILL, crate::skills::Target::Shared,
                                     Some(dir.join("fixture-skills")), crate::skills::Scope::Global, "", cx);
                             });
                             stage = 90;
                         }
                         90 if !owner.read(cx).skills_ui.busy => {
                             let app = owner.read(cx);
                             let skill = app.skills.iter().find(|skill| skill.id == crate::skills::PLUGIN_BUILDER_SKILL).unwrap();
                             assert!(!skill.installations.is_empty() && !skill.disabled);
                             child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("builtin-plugin-builder-action").center(), cx)).unwrap();
                             stage = 91;
                         }
                         91 if !owner.read(cx).skills_ui.busy => {
                             let app = owner.read(cx);
                             let skill = app.skills.iter().find(|skill| skill.id == crate::skills::PLUGIN_BUILDER_SKILL).unwrap();
                             assert!(skill.disabled);
                             assert!(!std::path::Path::new(&skill.installations[0].path).exists());
                             assert!(!app.conversation_skill_choices().iter().any(|skill| skill.id == crate::skills::PLUGIN_BUILDER_SKILL));
                             child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, crate::workspace_ui_smoke::measured("builtin-plugin-builder-action").center(), cx)).unwrap();
                             stage = 92;
                         }
                         92 if !owner.read(cx).skills_ui.busy => {
                             let app = owner.read(cx);
                             let skill = app.skills.iter().find(|skill| skill.id == crate::skills::PLUGIN_BUILDER_SKILL).unwrap();
                             assert!(!skill.disabled);
                             assert!(std::path::Path::new(&skill.installations[0].path).is_file());
                             stage = 93;
                         }
                         93 => {
                             child.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                app.settings_tab = SettingsTab::Secrets;
                                app.new_vault_draft(window, cx);
                                assert!(app.settings_have_draft());
                                app.request_settings_location(Some(SettingsTab::General), window, cx);
                                assert!(app.settings_ui.confirming_leave && app.vault.ui.draft.is_some());
                            })).unwrap();
                            stage = 1;
                        }
                        1 if dunlin_ui::debug_confirmation_answer(0).is_some() => {
                            let bounds = dunlin_ui::debug_confirmation_answer(0).unwrap();
                            child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, bounds.center(), cx)).unwrap();
                            stage = 2;
                        }
                        2 if !owner.read(cx).settings_ui.confirming_leave => {
                            assert!(owner.read(cx).vault.ui.draft.is_some(), "Keep editing preserves the form");
                            child.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.request_settings_location(Some(SettingsTab::General), window, cx))).unwrap();
                            stage = 3;
                        }
                        3 if dunlin_ui::debug_confirmation_answer(1).is_some() => {
                            let bounds = dunlin_ui::debug_confirmation_answer(1).unwrap();
                            child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, bounds.center(), cx)).unwrap();
                            stage = 4;
                        }
                        4 if !owner.read(cx).settings_ui.confirming_leave => {
                            assert!(!owner.read(cx).settings_have_draft());
                            assert_eq!(owner.read(cx).settings_tab, SettingsTab::General);
                            child.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                let search = app.settings_ui.search.as_ref().unwrap();
                                search.update(cx, |input, cx| input.set_value("reduce motion", window, cx));
                                search.read(cx).focus_handle(cx).focus(window, cx);
                            })).unwrap();
                            stage = 5;
                        }
                        5 => {
                            child.update(cx, |_, window, cx| {
                                window.focus_next(cx);
                                crate::controls_smoke::key(window, cx, "enter");
                            }).unwrap();
                            stage = 6;
                        }
                        6 => {
                            assert_eq!(owner.read(cx).settings_tab, SettingsTab::Design, "search result must be keyboard-activatable");
                            println!("Keyboard search passed at {:?}", started.elapsed());
                            child.update(cx, |_, window, cx| {
                                // The content header's Close action precedes the
                                // first theme card in document/tab order.
                                window.focus_next(cx);
                                window.focus_next(cx);
                                crate::controls_smoke::key(window, cx, "space");
                            }).unwrap();
                            assert!(owner.read(cx).settings_open, "theme activation must not dismiss settings");
                            stage = 7;
                        }
                        7 => {
                            assert_eq!(owner.read(cx).appearance, Appearance::System, "theme cards must accept keyboard activation");
                            println!("Keyboard System theme passed at {:?}", started.elapsed());
                            child.update(cx, |_, window, cx| {
                                window.focus_next(cx);
                                crate::controls_smoke::key(window, cx, "space");
                            }).unwrap();
                            stage = 8;
                        }
                         8 => {
                             assert_eq!(owner.read(cx).appearance, Appearance::Light);
                             let stored: StoredSettings = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap().load_setting("app").unwrap().unwrap();
                             assert_eq!(stored.appearance, Some(Appearance::Light), "keyboard theme choice must persist to a reopened database");
                             child.update(cx, |_, window, cx| {
                                 let restored = cx.new(|cx| DunlinApp::new(window, cx));
                                 assert_eq!(restored.read(cx).appearance, Appearance::Light, "a new app instance must restore the selected theme");
                             }).unwrap();
                              owner.update(cx, |app, cx| {
                                  app.settings_open = false;
                                  app.set_ui_density(UiDensity::Comfortable, cx);
                                  cx.notify();
                              });
                              stage = 80;
                         }
                         84 => {
                              child.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                  app.session_import.loading = true;
                                  app.request_factory_reset(window, cx);
                                  assert!(!app.factory_resetting, "an active import must block reset");
                              })).unwrap();
                              stage = 9;
                         }
                         9 if dunlin_ui::debug_confirmation_answer(0).is_some() => {
                             child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, dunlin_ui::debug_confirmation_answer(0).unwrap().center(), cx)).unwrap();
                             stage = 10;
                         }
                         10 => {
                             child.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                 app.session_import.loading = false;
                                 app.request_factory_reset(window, cx);
                                 assert!(app.factory_resetting, "reset must open confirmation");
                             })).unwrap();
                             stage = 11;
                         }
                         11 if dunlin_ui::debug_confirmation_answer(0).is_some() => {
                             child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, dunlin_ui::debug_confirmation_answer(0).unwrap().center(), cx)).unwrap();
                             stage = 12;
                         }
                         12 | 14 | 16 if !owner.read(cx).factory_resetting => {
                             assert!(!owner.read(cx).store.as_ref().unwrap().projects().unwrap().is_empty(), "cancellation must preserve projects");
                             child.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.request_factory_reset(window, cx))).unwrap();
                             stage += 1;
                         }
                         13 | 15 if dunlin_ui::debug_confirmation_answer(0).is_some() => {
                             child.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, if stage == 13 { "escape" } else { "enter" })).unwrap();
                             stage += 1;
                         }
                         17 if dunlin_ui::debug_confirmation_answer(1).is_some() => {
                             child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, dunlin_ui::debug_confirmation_answer(1).unwrap().center(), cx)).unwrap();
                             stage = 18;
                         }
                        _ => {}
                    }
                    false
                });
                if done { break; }
            }
        }).detach();
    });
}
