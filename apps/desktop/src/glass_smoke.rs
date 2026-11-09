//! Debug-only, isolated integration smoke. No harness, real database, screenshot
//! or real provider calls: exercise our own windows and input on the main thread.
use crate::app::{DunlinApp, StoredSettings};
use dunlin_ui::{assets, *};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let settings_close_only = std::env::args().any(|arg| arg == "--settings-close-only");
    crate::workspace_ui_smoke::enable_probes();
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
    let nvm = dir.join("nvm");
    let adapter_bin = nvm.join("versions/node/v24.18.0/bin");
    std::fs::create_dir_all(&adapter_bin).unwrap();
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    let settings = StoredSettings {
        auto_connect: false,
        restore_session: false,
        default_harness: "Custom ACP".into(),
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
            kind: Default::default(),
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
        std::env::set_var("NVM_DIR", &nvm);
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.glass-smoke", "Dunlin Glass Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let bounds = Bounds::new(point(px(80.), px(80.)), size(px(1360.), px(860.)));
        let (parent, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..dunlin_ui::window_options() }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 9000;
            let mut settings_closes = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let done = cx.update(|cx| {
                    // Background fixture windows do not receive every native
                    // frame. Draw both dispatch trees before reading geometry,
                    // as the editor/Git native regressions already do.
                    parent.update(cx, |_, window, cx| {window.refresh();window.draw(cx).clear(cx);}).unwrap();
                    if let Some(child)=owner.read(cx).overlay_window.debug_window(){
                        child.update(cx, |_, window, cx| {window.refresh();window.draw(cx).clear(cx);}).unwrap();
                    }
                    let (open, panel) = { let app = owner.read(cx); (app.overlay_window.is_open(), app.overlay_window.diagnostic(cx)) };
                    assert!(started.elapsed() < Duration::from_secs(20), "glass smoke stalled at stage {stage}; panel={panel:?}");
                    match stage {
                        9000 => {
                            owner.update(cx, |app, cx| { app.settings_open = true; cx.notify(); });
                            stage = 9001;
                        }
                        9001 if open && panel.is_some() => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, cx| {
                                assert_native_input_responder(window);
                                click(window, crate::workspace_ui_smoke::measured("settings-close").center(), cx);
                            }).unwrap();
                            assert!(!owner.read(cx).settings_open, "X must dismiss Settings");
                            assert!(owner.read(cx).settings_drag.is_none());
                            stage = 9002;
                        }
                        9002 if !open => {
                            // Leave the native display/Touch Bar update a full
                            // run-loop turn after destroying the active panel.
                            stage = 9003;
                        }
                        9003 => {
                            assert!(!owner.read(cx).has_modal_overlay());
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                            parent.update(cx, |_, window, _| assert_native_input_responder(window)).unwrap();
                            settings_closes += 1;
                            if settings_close_only && settings_closes == 5 {
                                std::fs::write(dir.join("PASS"), "Settings X dismissal and native focus restoration passed five times\n").unwrap();
                                println!("PASS: Settings X dismissal and native focus restoration passed five times");
                                cx.dispatch_action(&crate::app::Quit);
                                return true;
                            }
                            stage = if settings_closes < 5 { 9000 } else { 0 };
                        }
                        0 => {
                            let rail = crate::workspace_ui_smoke::measured("navigation-rail");
                            let settings = crate::workspace_ui_smoke::measured("rail-settings");
                            assert!(settings.bottom() > rail.bottom() - px(16.), "Settings belongs at the bottom of the navigation rail");
                            assert_eq!(crate::workspace_ui_smoke::measured("user-avatar").size, size(px(22.), px(22.)));
                            assert_eq!(crate::workspace_ui_smoke::measured("title-user-menu").size, size(px(32.), px(32.)), "a smaller avatar must preserve its click target");
                            parent.update(cx, |_, window, cx| click(window, settings.center(), cx)).unwrap();
                            stage = 90;
                        }
                        90 if panel.is_some() => {
                            assert!(owner.read(cx).settings_open);
                            // Native material and GPUI share the complete outer geometry.
                            assert_eq!(panel.unwrap(), SurfaceGeometry::SETTINGS.outer_bounds(crate::workspace_ui_smoke::measured("settings-panel")));
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
                        2 if !open && owner.read(cx).branch_ui.is_open() => {
                            assert!(!owner.read(cx).has_modal_overlay());
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming(), "branch lists must not dim or blur the application");
                            let focus = owner.read(cx).overlay_focus_handle(cx);
                            assert!(parent.update(cx, |_, w, _| focus.is_focused(w)).unwrap());
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.close_branch_picker(window, cx))).unwrap(); stage = 3;
                        }
                        3 if !open => {
                            assert!(!owner.read(cx).has_modal_overlay());
                             parent.update(cx, |_, window, cx| {
                                 click(window, crate::workspace_ui_smoke::measured("status-branch-picker").center(), cx);
                            }).unwrap();
                            assert!(owner.read(cx).branch_ui.is_open(), "status-bar branch trigger must open its compact picker");
                            stage = 12;
                        }
                        12 => {
                            assert!(!open);
                            assert!(!owner.read(cx).has_modal_overlay());
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                            parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                            assert!(!owner.read(cx).branch_ui.is_open());
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.open_palette(window, cx))).unwrap();
                            stage = 13;
                        }
                         13 => {
                            assert!(!open);
                            assert!(owner.read(cx).command_open);
                            assert!(!owner.read(cx).has_modal_overlay());
                             assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                             parent.update(cx, |_, window, _| assert_search_shade(window)).unwrap();
                            let focus = owner.read(cx).overlay_focus_handle(cx);
                            assert!(parent.update(cx, |_, w, _| focus.is_focused(w)).unwrap());
                             parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                             assert!(!owner.read(cx).command_open);
                             parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.open_project_search(window, cx))).unwrap();
                             stage = 130;
                         }
                         130 => {
                             assert!(owner.read(cx).command_open && owner.read(cx).command_projects_only);
                             parent.update(cx, |_, window, _| assert_search_shade(window)).unwrap();
                             parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                             assert!(!owner.read(cx).command_open);
                             owner.update(cx, |app, cx| {
                                app.remote_picker = Some(crate::app::RemotePicker {
                                    host_id: "smoke-host".into(), host_name: "Fixture (no network)".into(),
                                    projects: Vec::new(), loading: false, error: None,
                                });
                                cx.notify();
                            });
                            stage = 14;
                        }
                        14 => {
                            assert!(!open);
                            assert!(!owner.read(cx).has_modal_overlay());
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                            parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                            assert!(owner.read(cx).remote_picker.is_none());
                            owner.update(cx, |app, cx| {
                                app.context_menu = Some(crate::app::ContextMenuState {
                                    target: crate::app::ContextTarget::Project(0), position: point(px(300.), px(150.)),
                                });
                                cx.notify();
                            });
                            stage = 15;
                        }
                        15 => {
                            assert!(!open);
                            assert!(!owner.read(cx).has_modal_overlay());
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                            parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                            assert!(owner.read(cx).context_menu.is_none());
                            owner.update(cx, |app, cx| { app.project_menu_open = true; cx.notify(); });
                            stage = 16;
                        }
                        16 => {
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming());
                            parent.update(cx, |_, window, cx| press_escape(window, cx)).unwrap();
                            assert!(!owner.read(cx).project_menu_open);
                            owner.update(cx, |app, cx| { app.settings_open = true; cx.notify(); });
                            stage = 4;
                        }
                        4 if open && panel.is_some() => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                app.settings_open = false;
                                app.view = dunlin_domain::MainView::Graph;
                                app.open_new_group_prompt(None, window, cx);
                            })).unwrap();
                            stage = 5;
                        }
                        5 if open && panel.is_some_and(|b| b.size.width == px(380.)) => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, cx| {
                                assert_native_input_responder(window);
                                assert!(owner.read(cx).group_input.read(cx).focus_handle(cx).is_focused(window));
                                let position = owner.read(cx).group_input.read(cx).input_bounds().center();
                                click(window, position, cx);
                                for (key, code) in [("t", 17), ("e", 14), ("s", 1), ("t", 17), (" ", 49), ("g", 5), ("r", 15), ("o", 31), ("u", 32), ("p", 35)] {
                                    post_native_key(window, key, code);
                                }
                            }).unwrap();
                            stage = 6;
                        }
                        6 => {
                            assert_eq!(owner.read(cx).group_input.read(cx).value(), "test group", "native typing, including spaces, must reach the group field instead of Graph shortcuts");
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, _| post_native_key(window, "\r", 36)).unwrap();
                            stage = 7;
                        }
                        7 if !open => {
                            assert!(owner.read(cx).group_prompt.is_none());
                            assert!(owner.read(cx).project_groups.iter().any(|group| group.name == "test group"));
                            owner.update(cx, |app, cx| {
                                app.section = dunlin_domain::SidebarSection::Projects;
                                app.project_menu_open = true;
                                cx.notify();
                            });
                            stage = 8;
                        }
                        8 => {
                            assert!(owner.read(cx).project_menu_open);
                            assert!(!owner.read(cx).overlay_backdrop.has_dimming(), "project lists must not dim or blur the application");
                            parent.update(cx, |_, window, cx| {
                                let viewport = window.viewport_size();
                                click(window, point(viewport.width - px(8.), viewport.height - px(8.)), cx);
                            }).unwrap();
                            assert!(!owner.read(cx).project_menu_open, "outside click must dismiss the project list");
                            owner.update(cx, |app, cx| {
                                app.settings_open = true;
                                app.settings_tab = crate::app::SettingsTab::Harnesses;
                                for entry in &mut app.harness_discovery { entry.adapter = None; }
                                cx.notify();
                            });
                            stage = 9;
                        }
                        9 if open && panel.is_some() => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                             // Follow the actual default control after layout changes,
                             // not a coordinate that used to point at a radio.
                             child.update(cx, |_, window, cx| click(window, crate::workspace_ui_smoke::measured("claude-harness-default").center(), cx)).unwrap();
                             assert_eq!(owner.read(cx).default_harness, "Custom ACP", "an unavailable/unverified adapter must not change the default");
                            assert_eq!(owner.read(cx).session_harness(), "Custom ACP", "selection must not replace an existing conversation's harness");
                            child.update(cx, |_, window, cx| click(window, crate::workspace_ui_smoke::measured("claude-harness-action").center(), cx)).unwrap();
                            assert_eq!(owner.read(cx).harness_setup.as_deref(), Some("Claude Code"), "Set up must show in-app instructions without launching a browser");
                            let adapter = adapter_bin.join("claude-agent-acp");
                            std::fs::write(&adapter, "#!/bin/sh\nexit 99\n").unwrap();
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
                            stage = 10;
                        }
                        10 => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, cx| click(window, crate::workspace_ui_smoke::measured("rescan-harnesses").center(), cx)).unwrap();
                            assert!(owner.read(cx).harness_scanning);
                            stage = 11;
                        }
                        11 if !owner.read(cx).harness_scanning => {
                            assert!(owner.read(cx).harness_discovery.iter().any(|entry| entry.name == "Claude Code" && entry.adapter.is_some()), "Scan again must detect a newly installed NVM adapter");
                            assert!(owner.read(cx).notice.as_ref().unwrap().starts_with("Scan complete"));
                            assert!(owner.read(cx).harness_setup_notice.as_ref().unwrap().starts_with("Scan complete"));
                            owner.update(cx, |app, cx| { app.factory_resetting = true; cx.notify(); });
                            stage = 17;
                        }
                        17 => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            for handle in [parent, child] {
                                handle.update(cx, |_, window, cx| {
                                    assert!(!window.is_action_available(&crate::native_menu::Commands, cx));
                                    assert!(!window.is_action_available(&crate::native_menu::OpenFolders, cx));
                                    press_escape(window, cx);
                                }).unwrap();
                            }
                            assert!(owner.read(cx).factory_resetting);
                            cx.dispatch_action(&crate::app::Quit);
                            stage = 18;
                        }
                        18 => {
                            // A normal Quit must not interrupt credential/SQLite reset work.
                            assert!(owner.read(cx).factory_resetting);
                            owner.update(cx, |app, cx| { app.factory_resetting = false; cx.notify(); });
                            stage = 19;
                        }
                        19 => {
                            owner.update(cx, |app, cx| {
                                app.settings_open = false;
                                app.request_delete_project(0, cx);
                            }); stage = 20;
                        }
                        20 if owner.read(cx).pending_delete.is_some() && panel.is_some() => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, cx| {
                                crate::controls_smoke::key(window, cx, "tab");
                                crate::controls_smoke::key(window, cx, "enter");
                            }).unwrap(); stage = 21;
                        }
                        21 if !open => {
                            assert!(owner.read(cx).pending_delete.is_none());
                            assert_eq!(owner.read(cx).projects.len(), 1, "Enter on Cancel removed the fixture project");
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.open_new_group_prompt(None, window, cx))).unwrap(); stage = 22;
                        }
                        22 if owner.read(cx).group_prompt.is_some() && panel.is_some() => {
                            let child = owner.read(cx).overlay_window.debug_window().unwrap();
                            child.update(cx, |_, window, cx| {
                                owner.update(cx, |app, cx| app.group_input.update(cx, |input, cx| {
                                    input.set_value("Keyboard fixture group", window, cx);
                                    input.focus(window, cx);
                                }));
                                crate::controls_smoke::key(window, cx, "enter");
                            }).unwrap(); stage = 23;
                        }
                        23 if !open => {
                            assert!(owner.read(cx).group_prompt.is_none());
                            assert_eq!(owner.read(cx).project_groups.iter().filter(|group| group.name == "Keyboard fixture group").count(), 1, "Enter in a form did not submit exactly once");
                            owner.update(cx, |app, cx| { app.settings_open = true; cx.notify(); }); stage = 24;
                        }
                        24 if open && panel.is_some() => {
                            std::fs::write(dir.join("PASS"), "Repeated Settings X dismissal, neutral lists/no dimming, Escape/focus, AppKit group typing/Enter, in-app setup and async scan passed\n").unwrap();
                            println!("PASS: repeated Settings X dismissal, neutral lists, native glass/resize/input, group Enter submits once, keyboard Cancel preserves project, outside dismissal, harness setup/rescan and guarded Quit");
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

fn assert_search_shade(window: &Window) {
    let bounds =
        Bounds::new(point(px(0.), px(0.)), window.viewport_size()).scale(window.scale_factor());
    let shade: Background = black().opacity(0.14).into();
    assert!(
        window
            .painted_quads()
            .iter()
            .any(|quad| quad.bounds == bounds && quad.background == shade),
        "search must paint a light fullscreen shade behind the palette"
    );
}

fn click(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
}

fn press_escape(window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke::parse("escape").unwrap(),
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
}

fn assert_native_input_responder(window: &Window) {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).unwrap();
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        unreachable!()
    };
    // SAFETY: The fixture window owns this view and runs on AppKit's main thread.
    let view: &NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
    let native = view.window().unwrap();
    let app =
        objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
    assert!(native.canBecomeKeyWindow());
    if app.isActive() {
        assert!(
            native.isKeyWindow(),
            "the active application's modal must own the native keyboard focus"
        );
    }
    let responder = native
        .firstResponder()
        .expect("modal has no native input responder");
    assert_eq!(
        std::ptr::from_ref(&*responder).cast::<()>(),
        std::ptr::from_ref(view).cast::<()>(),
        "native key events must reach the modal's GPUI view"
    );
}

fn post_native_key(window: &Window, key: &str, code: u16) {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::{NSOperationQueue, NSPoint, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).unwrap();
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        unreachable!()
    };
    // SAFETY: Only this isolated test application's live window is targeted.
    let view: &NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
    let native = view.window().unwrap();
    let text = NSString::from_str(key);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0.,
        native.windowNumber(), None, &text, &text, false, code,
    ).unwrap();
    // Send only to our fixture, even if macOS leaves the user's app frontmost.
    let block = block2::RcBlock::new(move || native.sendEvent(&event));
    // SAFETY: AppKit delivery must run on the main queue, outside GPUI borrows.
    unsafe {
        NSOperationQueue::mainQueue().addOperationWithBlock(&block);
    }
}
