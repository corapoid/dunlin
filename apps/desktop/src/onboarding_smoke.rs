//! Debug-only fresh-install/TCC smoke. No harness, provider request, screenshot
//! or simulated system prompt: exercise onboarding state and the permission
//! center in an isolated data directory, then quit that process only.
use crate::app::{DunlinApp, SettingsTab};
use dunlin_domain::SidebarSection;
use dunlin_ui::{assets, *};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--onboarding-smoke requires a fresh absolute fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-onboarding-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture directory must not already exist");
    // SAFETY: main invokes this before starting any application/worker threads.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application()
        .with_assets(assets::AllAssets)
        .run(|cx| {
            cx.set_app_identity(
                "dev.dunlin.ide.onboarding-smoke",
                "Dunlin Onboarding Smoke",
            );
            dunlin_ui::init(cx);
            crate::logging::init();
            let bounds = Bounds::new(point(px(80.), px(80.)), size(px(1200.), px(800.)));
            let (parent, owner) = open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..dunlin_ui::window_options()
                },
                cx,
                |window, cx| cx.new(|cx| DunlinApp::new(window, cx)),
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let started = Instant::now();
                let mut stage = 0;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    let done = cx.update(|cx| {
                        parent
                            .update(cx, |_, window, cx| {
                                window.refresh();
                                window.draw(cx).clear(cx);
                            })
                            .unwrap();
                        assert!(
                            started.elapsed() < Duration::from_secs(30),
                            "onboarding smoke stalled at stage {stage}"
                        );
                        match stage {
                            0 => {
                                owner.update(cx, |app, cx| {
                                    assert!(app.projects.is_empty());
                                    assert_eq!(app.onboarding_step, 0);
                                    assert!(app.store.is_some(), "fresh store was not created");
                                    app.harness_discovery.clear();
                                    app.section = SidebarSection::Projects;
                                    app.onboarding_step = 1;
                                    cx.notify();
                                });
                                stage = 1;
                            }
                            1 => {
                                let report = owner.read_with(cx, |app, _| {
                                    assert_eq!(app.onboarding_step, 1);
                                    assert!(app.harness_discovery.is_empty());
                                    let statuses = crate::platform::permission_statuses();
                                    assert_eq!(statuses.len(), crate::platform::PermissionKind::ALL.len());
                                    assert!(statuses.iter().all(|status| !status.detail.is_empty()));
                                    crate::qa::capture(app)
                                });
                                assert!(report.contains("# Dunlin QA report"));
                                assert!(report.contains("Onboarding: visible"));
                                assert!(report.contains("Manual R11 status: NOT evaluated"));
                                owner.update(cx, |app, cx| {
                                    app.settings_open = true;
                                    app.settings_tab = SettingsTab::Permissions;
                                    cx.notify();
                                });
                                stage = 2;
                            }
                            2 => {
                                assert!(owner.read(cx).settings_open);
                                assert_eq!(owner.read(cx).settings_tab, SettingsTab::Permissions);
                                parent
                                    .update(cx, |_, window, cx| {
                                        window.refresh();
                                        window.draw(cx).clear(cx);
                                    })
                                    .unwrap();
                                println!("PASS: fresh data directory, empty project/harness onboarding, five permission states, copyable QA report and Permissions settings rendered; Quit");
                                cx.dispatch_action(&crate::app::Quit);
                                return true;
                            }
                            _ => {}
                        }
                        false
                    });
                    if done {
                        break;
                    }
                }
            })
            .detach();
        });
}
