//! Fresh, isolated native settings acceptance. No agent, credential or network writes.
use super::*;
use crate::app::{Appearance, StoredSettings, UiDensity};
use dunlin_ui::component::TitleBar;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

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
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                default_harness: "Custom ACP".into(),
                opencode_provider: None,
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
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let pages: Vec<_> = navigation::GROUPS.iter().flat_map(|(_, pages)| pages.iter().copied()).collect();
             let mut index = 0;
             let mut matrix_prepared = false;
            let total = pages.len() * 8;
            let mut stage = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(90), "settings smoke stalled: page {index}, stage {stage}");
                    parent.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                    let Some(child) = owner.read(cx).overlay_window.debug_window() else { return false; };
                    child.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                     if index < total {
                         if matrix_prepared {
                         child.update(cx, |_, window, _| {
                             crate::workspace_ui_smoke::assert_painted_corners(window, "settings-panel", true);
                             crate::workspace_ui_smoke::assert_painted_corners(window, "settings-content", true);
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
                             fs::write(dir.join("PASS"), format!("{total} settings render combinations; form discard/keep; search/theme keyboard activation and preference restoration passed\n")).unwrap();
                             println!("PASS: {} pages × 2 sizes × 2 themes × 2 densities; form discard/keep; search/theme keyboard activation and preference restoration", pages.len());
                            cx.quit();
                            return true;
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
