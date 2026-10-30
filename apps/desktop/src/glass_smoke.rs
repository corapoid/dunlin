//! Debug-only, isolated integration smoke. No harness, real database, screenshot
//! or simulated user input: exercise our own windows/state on the main thread.
use crate::app::{DunlinApp, StoredSettings};
use dunlin_ui::{assets, component::TitleBar, *};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--glass-smoke requires a fresh absolute fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-glass-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture directory must not already exist");
    let project_path = dir.join("project");
    std::fs::create_dir(&project_path).unwrap();
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    let settings = StoredSettings {
        auto_connect: false,
        restore_session: false,
        default_harness: "Custom ACP".into(),
        allow_network: false,
        notifications_enabled: false,
        ..Default::default()
    };
    store.save_setting("app", &settings).unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "glass-fixture".into(),
            name: "Glass fixture".into(),
            path: project_path.to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        })
        .unwrap();
    drop(store);
    // SAFETY: main invokes this before starting any application/worker threads.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application().with_assets(assets::AllAssets).run(|cx| {
        cx.set_app_identity("dev.dunlin.ide.glass-smoke", "Dunlin Glass Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let bounds = Bounds::new(point(px(80.), px(80.)), size(px(1360.), px(860.)));
        let (parent, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..TitleBar::window_options() }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        owner.update(cx, |app, cx| { app.settings_open = true; cx.notify(); });
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(20), "glass smoke stalled at stage {stage}");
                    let (open, panel) = { let app = owner.read(cx); (app.overlay_window.is_open(), app.overlay_window.diagnostic(cx)) };
                    match stage {
                        0 if panel.is_some() => {
                            assert_eq!(panel.unwrap().size, size(px(1120.), px(760.)));
                            assert!(open);
                            parent.update(cx, |_, window, _| window.resize(size(px(1200.), px(800.)))).unwrap(); stage = 1;
                        }
                        1 if panel.is_some_and(|b| b.size.height < px(760.)) => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            let parent_size = parent.update(cx, |_, w, _| w.viewport_size()).unwrap();
                            let child_size = child.update(cx, |_, w, _| w.viewport_size()).unwrap();
                            assert_eq!(parent_size, child_size);
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| { app.settings_open = false; app.open_branch_picker("glass-fixture", window, cx); })).unwrap(); stage = 2;
                        }
                        2 if panel.is_some_and(|b| b.size == size(px(530.), px(460.))) => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            let focus = owner.read(cx).overlay_focus_handle(cx);
                            assert!(child.update(cx, |_, w, _| focus.is_focused(w)).unwrap());
                            child.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.close_branch_picker(window, cx))).unwrap(); stage = 3;
                        }
                        3 if !open => {
                            assert!(!owner.read(cx).has_modal_overlay());
                            owner.update(cx, |app, cx| { app.settings_open = true; cx.notify(); }); stage = 4;
                        }
                        4 if open && panel.is_some() => {
                            println!("PASS: Settings glass plane, resize synchronization, unchanged 530x460 branch picker, focus handoff, close/reopen cleanup and Quit with panel open");
                            // Exercise the normal guarded Quit action in this
                            // isolated process, never the user's running app.
                            cx.dispatch_action(&crate::app::Quit); return true;
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
