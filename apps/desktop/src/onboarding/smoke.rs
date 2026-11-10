//! Isolated native acceptance. npm/node/CLIs are fixtures; no network install,
//! real adapter, provider call or user-home change is permitted in this test.
use super::*;
use crate::{
    app::StoredSettings,
    workspace_ui_smoke::{click, measured, take_measured},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

pub(crate) fn run() {
    let dir = PathBuf::from(std::env::args_os().nth(2).unwrap());
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-onboarding-setup-smoke-")
    );
    fs::create_dir(&dir).expect("fresh fixture directory");
    let bin = dir.join("bin");
    fs::create_dir(&bin).unwrap();
    let project = dir.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("README.md"), "# Onboarding fixture\n").unwrap();
    let mock = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("mock_acp_agent");
    assert!(mock.is_file(), "Build mock_acp_agent before this smoke");
    for name in ["claude", "opencode", "gh", "glab"] {
        executable(&bin.join(name), "#!/bin/sh\nexit 1\n");
    }
    executable(&bin.join("node"), "#!/bin/sh\nprintf 'v22.20.0\\n'\n");
    let calls = dir.join("npm.calls");
    let fail = dir.join("fail-once");
    fs::write(&fail, "fixture").unwrap();
    executable(
        &bin.join("npm"),
        &format!(
            "#!/bin/sh\nprintf 'install\\n' >> {}\nif [ -f {} ]; then /bin/rm {}; exit 1; fi\nprefix=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = '--prefix' ]; then shift; prefix=\"$1\"; fi; shift; done\n[ -n \"$prefix\" ] || exit 3\n/bin/mkdir -p \"$prefix/bin\"\n/bin/cp {} \"$prefix/bin/claude-agent-acp\"\n/bin/chmod 755 \"$prefix/bin/claude-agent-acp\"\n",
            quote(&calls),
            quote(&fail),
            quote(&fail),
            quote(&mock)
        ),
    );
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        for key in [
            "DUNLIN_HARNESS_COMMAND",
            "DUNLIN_AUTOPROMPT",
            "DUNLIN_MOCK_GUI_ROOT",
            "DUNLIN_MOCK_RICH_CHAT",
            "DUNLIN_MOCK_CONFIG_MODES",
            "DUNLIN_MOCK_REJECT_MODEL",
            "DUNLIN_MOCK_DELAY_MS",
        ] {
            std::env::remove_var(key);
        }
    }
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                restore_session: false,
                vault_enabled: false,
                notifications_enabled: false,
                default_harness: "OpenCode".into(),
                ..Default::default()
            },
        )
        .unwrap();
    drop(store);
    crate::workspace_ui_smoke::enable_probes();
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.onboarding-setup-smoke", "Dunlin Onboarding Setup Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let (parent, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(80.)), size(px(1100.), px(800.))))), ..dunlin_ui::window_options() }, cx,
            |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        owner.update(cx, |app, cx| {
            // Simulate a CLI-only Claude installation, regardless of unrelated
            // system-wide binaries on the machine running the fixture.
            app.harness_discovery = crate::harness_discovery::SPECS.iter().map(|spec| crate::harness_discovery::Detected {
                name: spec.name,
                cli: match spec.name { "OpenCode" => Some(bin.join("opencode")), "Claude Code" => Some(bin.join("claude")), _ => None },
                adapter: (spec.name == "OpenCode").then(|| bin.join("opencode")),
            }).collect();
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0;
            let mut failed_at = None; let mut ready_at = None;
            let mut welcome_layout = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(55), "setup smoke stalled at {stage}: {:?}", owner.read(cx).onboarding.setup.as_ref().map(|s| &s.phase));
                    parent.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                    if let Some(child) = owner.read(cx).overlay_window.debug_window() { let _ = child.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }); }
                    if matches!(stage, 1 | 11 | 12) {
                        assert_eq!(Some((measured("onboarding-content"), measured("onboarding-continue"))), welcome_layout,
                            "showing or hiding the installation notice must not move welcome controls");
                    }
                    match stage {
                        0 => {
                            assert!(owner.read(cx).projects.is_empty());
                            assert!(take_measured("sidebar").is_none(), "welcome must never render the sidebar");
                             assert!(take_measured("title-sidebar").is_none());
                             assert!(take_measured("onboarding-adapter-notice").is_none());
                             welcome_layout = Some((measured("onboarding-content"), measured("onboarding-continue")));
                            parent.update(cx, |_, window, cx| click(window, measured("onboarding-claude").center(), cx)).unwrap();
                            stage = 1;
                        }
                        1 => {
                            assert_eq!(owner.read(cx).onboarding.selection.as_deref(), Some("Claude Code"));
                            assert!(!calls.exists(), "selecting a harness must not install anything");
                             take_measured("onboarding-adapter-notice").expect("selected CLI-only harness shows its installation notice");
                             parent.update(cx, |_, window, cx| click(window, measured("onboarding-opencode").center(), cx)).unwrap();
                             stage = 11;
                         }
                         11 => {
                             assert!(take_measured("onboarding-adapter-notice").is_none(), "ready harness hides the installation notice");
                             parent.update(cx, |_, window, cx| click(window, measured("onboarding-claude").center(), cx)).unwrap();
                             stage = 12;
                         }
                         12 => {
                             measured("onboarding-adapter-notice");
                            parent.update(cx, |_, window, cx| click(window, measured("onboarding-show-all").center(), cx)).unwrap();
                            assert!(owner.read(cx).onboarding_show_all_harnesses);
                            stage = 10;
                        }
                        10 => {
                            measured("onboarding-other-harness");
                            parent.update(cx, |_, window, cx| click(window, measured("onboarding-continue").center(), cx)).unwrap();
                            stage = 2;
                        }
                        2 => {
                            assert_eq!(owner.read(cx).onboarding_step, 1);
                            assert!(owner.read(cx).onboarding.plan.is_some());
                            assert!(!calls.exists(), "installation waits for the first project");
                            parent.update(cx, |_, window, cx| click(window, measured("onboarding-back").center(), cx)).unwrap();
                            assert!(owner.read(cx).onboarding.plan.is_none(), "Back withdraws pending install intent");
                            stage = 3;
                        }
                        3 => {
                            parent.update(cx, |_, window, cx| click(window, measured("onboarding-continue").center(), cx)).unwrap();
                            owner.update(cx, |app, cx| app.import_local_folders(vec![project.clone()], cx));
                            stage = 4;
                        }
                        4 if owner.read(cx).onboarding.setup.as_ref().is_some_and(|s| matches!(s.phase, Phase::Failed(_))) => {
                            assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 1);
                            assert!(take_measured("sidebar").is_none());
                            measured("onboarding-setup-dialog");
                            #[cfg(target_os = "macos")]
                            assert!(owner.read(cx).overlay_window.is_open(), "setup must have its own attached native window");
                            failed_at = Some(Instant::now()); stage = 5;
                        }
                        5 if failed_at.unwrap().elapsed() > Duration::from_millis(450) => {
                            assert!(owner.read(cx).onboarding.setup.is_some(), "a failed install cannot dismiss itself as success");
                            if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                                child.update(cx, |_, window, cx| click(window, measured("onboarding-setup-retry").center(), cx)).unwrap();
                            } else { parent.update(cx, |_, window, cx| click(window, measured("onboarding-setup-retry").center(), cx)).unwrap(); }
                            stage = 6;
                        }
                        6 if owner.read(cx).onboarding.setup.as_ref().is_some_and(|s| matches!(s.phase, Phase::Ready)) => {
                            assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);
                            assert!(owner.read(cx).harness.transcript.is_empty(), "verification must not send a model prompt");
                            assert!(owner.read(cx).harness_check_status("Claude Code").is_some_and(HarnessStatus::is_ready));
                            ready_at = Some(Instant::now()); stage = 7;
                        }
                        7 if owner.read(cx).onboarding.setup.is_none() => {
                            assert!(ready_at.unwrap().elapsed() > Duration::from_millis(1750), "verified success stays visible for approximately two seconds");
                            assert!(!owner.read(cx).onboarding_visible());
                            assert!(owner.read(cx).onboarding.plan.is_none());
                            assert_eq!(owner.read(cx).onboarding_step, 2);
                            assert_eq!(owner.read(cx).session_harness(), "Claude Code");
                            take_measured("onboarding-screen"); stage = 8;
                        }
                        8 if owner.read(cx).onboarding.reveal.is_some_and(|t| t.elapsed() > Duration::from_millis(450)) && !owner.read(cx).overlay_window.is_open() => {
                            assert!(measured("sidebar").size.width > px(200.));
                            assert!(take_measured("onboarding-screen").is_none());
                            let saved = owner.read(cx).store.as_ref().unwrap().load_setting::<StoredSettings>("app").unwrap().unwrap();
                            assert!(saved.pending_adapter_install.is_none());
                            println!("PASS: sidebar-free welcome, radio harness choice, disclosure arrow, deferred consent/project-scoped mock install, Back withdrawal, failed install/retry, empty ACP verification, native progress window, two-second success hold and animated application reveal. No network downloads or real adapters installed.");
                            cx.quit(); return true;
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
