//! Isolated native acceptance: no real repositories, agents, credentials or data.
use crate::{
    app::{DunlinApp, StoredSettings},
    harness::ChatItem,
};
use dunlin_domain::{MainView, SidebarSection};
use dunlin_ui::{assets, *};
use dunlin_workspace::git::operations::Action;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

thread_local! {
    static BOUNDS: RefCell<Option<HashMap<&'static str, Bounds<Pixels>>>> = const { RefCell::new(None) };
    static PROMPT_ANSWER: RefCell<Option<usize>> = const { RefCell::new(None) };
}
pub(crate) fn enable_probes() {
    BOUNDS.with(|bounds| *bounds.borrow_mut() = Some(HashMap::new()));
}
pub(crate) fn probe(name: &'static str) -> Option<AnyElement> {
    BOUNDS.with(|bounds| bounds.borrow().is_some()).then(|| {
        canvas(
            move |bounds, _, _| {
                BOUNDS.with(|map| {
                    map.borrow_mut().as_mut().unwrap().insert(name, bounds);
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full()
        .into_any_element()
    })
}
pub(crate) fn measured(name: &str) -> Bounds<Pixels> {
    BOUNDS.with(|map| {
        *map.borrow()
            .as_ref()
            .unwrap()
            .get(name)
            .unwrap_or_else(|| panic!("missing probe: {name}"))
    })
}

// Inspect paint, not just layout: a square child fill can hide a rounded parent.
pub(crate) fn assert_painted_corners(window: &Window, name: &str, rounded_left: bool) {
    let bounds = if name == "settings-panel" {
        SurfaceGeometry::SETTINGS.outer_bounds(measured(name))
    } else {
        measured(name)
    };
    dunlin_ui::surface_test::assert_painted_corners(window, bounds, rounded_left, name);
}

fn assert_compact_session_row() {
    let row = measured("compact-session-row");
    let status = measured("compact-session-status");
    let date = measured("compact-session-date");
    let delete = measured("compact-session-delete");
    assert!(
        row.size.height <= px(60.),
        "session details must stay within two lines"
    );
    assert!(status.size.height <= px(14.));
    assert!(
        status.right() <= delete.left(),
        "status must precede the delete icon"
    );
    assert!((status.center().y - delete.center().y).abs() <= px(1.));
    assert!(date.right() <= status.left());
    assert!(
        (status.center().y - date.center().y).abs() <= px(1.),
        "status belongs next to the full timestamp"
    );
}
pub(crate) fn click(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
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
fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("fresh absolute fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-workspace-ui-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture must not exist");
    let root = dir.join("project");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--template=", "-qb", "main"]);
    git(&root, &["config", "core.hooksPath", "/dev/null"]);
    git(&root, &["config", "core.fsmonitor", "false"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("file.rs"), "original\n").unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Fixture repository\n\nInert README hover preview.\n",
    )
    .unwrap();
    git(&root, &["add", "file.rs", "README.md"]);
    git(&root, &["commit", "-qm", "fixture"]);
    git(&root, &["branch", "feature"]);
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                restore_session: false,
                notifications_enabled: false,
                default_harness: "Custom ACP".into(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "ui-fixture".into(),
            name: "UI fixture".into(),
            path: root.to_string_lossy().into(),
            branch: Some("main".into()),
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
    // SAFETY: called before application/worker threads are started.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        dunlin_ui::init(cx); crate::native_menu::install(cx); crate::logging::init();
        BOUNDS.with(|map| *map.borrow_mut() = Some(HashMap::new()));
        let (parent, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(60.), px(60.)), size(px(1600.), px(950.))))),
            ..dunlin_ui::window_options() }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        parent.update(cx, |_, window, cx| crate::terminal::smoke_glyphs(window, cx)).unwrap();
        owner.update(cx, |app, cx| {
            app.section = SidebarSection::Projects; app.view = MainView::Chat; app.sidebar_collapsed = false;
            app.harness.status = crate::harness::HarnessStatus::Starting;
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 7200; let mut hovered = false;
            let mut plus_pressed = None::<Instant>;
            let mut selector_pressed = None::<Instant>;
            let mut scratch = None::<String>;
            let mut dock_terminal = None::<String>;
            let mut closing_counts = (0, 0);
            let mut search_target = String::new();
            let mut recent_file = String::new();
            let mut zoom_before = 0.;
            let mut comfortable_project_height = px(0.);
            let mut rail_origin = point(px(0.), px(0.));
            let mut sidebar_width = px(0.);
            let mut settings_before: Option<Bounds<Pixels>> = None;
            let mut usage_before: Option<Bounds<Pixels>> = None;
            let mut dashboard_recent = None;
             let mut dashboard_summary = None;
             let mut activity_transcript = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let done = cx.update(|cx| {
                    BOUNDS.with(|map| map.borrow_mut().as_mut().unwrap().clear());
                    parent.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                        child.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    }
                    if stage == 12 && !hovered {
                        let position = measured("active-project-row").center();
                        parent.update(cx, |_, window, cx| { window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx); }).unwrap();
                        hovered = true;
                    }
                    let target = match stage {
                         7900 => Some("ctx-project-group"), 7902 => Some("project-group-none"),
                          7903 => Some("ctx-project-trust"), 7910 => Some("claude-harness-visible"),
                         10000 => Some("delete-cancel"), 10001 => Some("delete-confirm"),
                        100 => Some("group-settings-button"), 101 => Some("group-icon-kubernetes"),
                        102 => Some("group-color-teal"), 103 | 107 => Some("group-settings-save"),
                        104 => Some("group-settings-button"),
                        106 => Some("group-icon-dot"),
                        31 | 33 => Some("project-menu-root"),
                        190 => Some("host-menu-button"),
                        81 => Some("close-chat-tab"),
                        209 => Some("close-qa-report"),
                        5006 => Some("status-usage"),
                        35 => Some("project-menu-active-row"), _ => None,
                    };
                    if let Some(target) = target {
                        // Detached modal creation is deferred until after the owner frame.
                        if !BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key(target)) {
                            assert!(started.elapsed() < Duration::from_secs(90), "workspace smoke stalled waiting for {target} at {stage}");
                            return false;
                        }
                        let position = measured(target).center();
                        if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                            child.update(cx, |_, window, cx| click(window, position, cx)).unwrap();
                        } else {
                            parent.update(cx, |_, window, cx| click(window, position, cx)).unwrap();
                        }
                    }
                    if stage == 207 && BOUNDS.with(|map| map.borrow().as_ref().unwrap().contains_key("settings-drag-handle")) {
                        settings_before = Some(measured("settings-panel"));
                        let position = measured("settings-drag-handle").center();
                        let child = owner.read(cx).overlay_window.debug_window().unwrap();
                         child.update(cx, |_, window, cx| {
                             assert_painted_corners(window, "settings-panel", true);
                             assert_painted_corners(window, "settings-content", true);
                            let content = measured("settings-content");
                            let page = measured("settings-page");
                            assert!((page.left() - content.left() - px(8.)).abs() <= px(1.));
                            assert!((content.right() - page.right() - px(8.)).abs() <= px(1.), "settings must have equal left/right gutters, not a capped left-aligned page");
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
                            let position = position + point(px(20.), px(18.));
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                        stage = 208;
                        return false;
                    }
                    if stage == 201 {
                        if let Some(bounds) = dunlin_ui::debug_confirmation_answer(1) {
                            parent.update(cx, |_, window, cx| {
                                assert!(window.has_active_prompt(), "confirmation must be themed, not a native alert");
                                click(window, bounds.center(), cx);
                            }).unwrap();
                            stage = 202;
                        }
                    } else if stage == 203 || stage == 205 || stage == 192 {
                        parent.update(cx, |_, window, cx| {
                            if stage != 192 { assert!(window.has_active_prompt()); }
                            window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke: Keystroke::parse(if stage == 205 { "enter" } else { "escape" }).unwrap(), is_held: false, prefer_character_input: false }), cx);
                        }).unwrap();
                        stage = if stage == 203 { 204 } else if stage == 205 { 206 } else { 200 };
                    }
                    if stage == 5008 {
                        parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "escape")).unwrap();
                        stage = 5009;
                        return false;
                    }
                    if matches!(stage, 7202 | 7206) {
                        assert!(started.elapsed() < Duration::from_secs(90), "install confirmation stalled at {stage}");
                        if let Some(bounds) = dunlin_ui::debug_confirmation_answer(1) {
                            parent.update(cx, |_, window, cx| click(window, bounds.center(), cx)).unwrap();
                            stage += 1;
                        }
                        return false;
                    }
                    if matches!(stage, 7105 | 7107) {
                        let source = measured("file-tab-drag").center();
                        let left = measured("workspace-pane-0");
                        let right = measured("workspace-pane-1");
                        assert_eq!(right.left() - left.right(), px(6.));
                        let destination = if stage == 7105 { right } else { left };
                        let target = point(destination.center().x, destination.top() + px(18.));
                        for (attempt, target) in [destination.center(), target].into_iter().enumerate() {
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: source, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: source, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: source + point(px(20.), px(0.)), pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                             assert!(cx.has_active_drag(), "tab must expose a native drag payload");
                             assert!(cx.has_active_drag_of::<crate::workspace_panes::TabDrag>());
                             assert!(!cx.has_active_drag_of::<crate::project_groups::ProjectDrag>());
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: target, pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                             window.refresh(); window.draw(cx).clear(cx);
                             assert!(measured("dragged-tab-preview").size.width > px(40.), "drag must show the actual tab following the pointer");
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: target, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                        if attempt == 0 {
                            let app = owner.read(cx);
                            let key = crate::workspace_panes::TabKey::File(app.workbench.active.clone().unwrap());
                            assert_eq!(app.workbench.panes.owner(&key), if stage == 7105 { 0 } else { 1 }, "content outside a tab strip must reject tab drops");
                        }
                        }
                        stage += 1;
                        return false;
                    }
                    if matches!(stage, 7150 | 7152) {
                        if stage == 7152 {
                            parent.update(cx, |_, window, _| assert_painted_corners(window, "reading-surface", false)).unwrap();
                        }
                        let source = measured("file-tab-drag").center();
                        let pane = measured("workspace-pane-0");
                        let target = point(if stage == 7150 { pane.left() + px(35.) } else { pane.right() - px(160.) }, pane.top() + px(18.));
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: source, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: source, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: source + point(px(20.), px(0.)), pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                            assert!(cx.has_active_drag_of::<crate::workspace_panes::TabDrag>());
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: target, pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                            window.refresh(); window.draw(cx).clear(cx);
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: target, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                        let app = owner.read(cx);
                        let keys = app.workspace_tab_keys();
                        let file = crate::workspace_panes::TabKey::File(app.workbench.active.clone().unwrap());
                        assert_eq!(if stage == 7150 { keys.first() } else { keys.last() }, Some(&file), "drag must reorder tabs within an unsplit strip");
                        stage = if stage == 7150 { 7152 } else { 7102 };
                        return false;
                    }
                    if stage == 76021 && owner.read(cx).terminal.active_tab().is_some_and(|tab| tab.screen.screen().contents().lines().any(|line| line.trim() == "TAB_READY")) {
                        parent.update(cx, |_, window, cx| {
                            crate::controls_smoke::key(window, cx, "tab");
                            assert!(owner.read(cx).terminal.focus.is_focused(window), "bare Tab must stay in the terminal, not advance application focus");
                        }).unwrap();
                        stage = 76022;
                        return false;
                    }
                    if stage == 7700 {
                        parent.update(cx, |_, window, cx| click(window, measured("document-tabs-menu").center(), cx)).unwrap();
                        stage = 7701;
                        return false;
                    }
                    if stage == 7701 {
                        parent.update(cx, |_, window, cx| {
                            for _ in 0..4 { crate::controls_smoke::key(window, cx, "down"); }
                            crate::controls_smoke::key(window, cx, "enter");
                        }).unwrap();
                        stage = 7702;
                        return false;
                    }
                    if matches!(stage, 7702 | 7704) {
                        assert!(started.elapsed() < Duration::from_secs(90), "close-all prompt stalled at {stage}");
                        if let Some(bounds) = dunlin_ui::debug_confirmation_answer(if stage == 7702 { 2 } else { 1 }) {
                            parent.update(cx, |_, window, cx| {
                                assert!(window.has_active_prompt());
                                click(window, bounds.center(), cx);
                            }).unwrap();
                            stage += 1;
                        }
                        return false;
                    }
                    if stage == 7904 && let Some(child) = owner.read(cx).overlay_window.debug_window() {
                        child.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "escape")).unwrap();
                        stage = 7905;
                        return false;
                    }
                    if stage == 7730 && let Some(child) = owner.read(cx).overlay_window.debug_window() {
                        child.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.new_vault_draft(window, cx))).unwrap();
                        stage = 7731;
                        return false;
                    }
                    if matches!(stage, 7800 | 7802 | 7806 | 7811 | 7813 | 7821 | 7823 | 7831 | 7833) {
                        let key = match stage {
                            7800 => "cmd-alt-p", 7802 => "enter", 7806 => "escape",
                            7811 => "cmd-+", 7821 | 7831 => "cmd-=", _ => "cmd--",
                        };
                        parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, key)).unwrap();
                        stage += 1;
                        return false;
                    }
                    if stage == 7804 {
                        parent.update(cx, |_, window, cx| click(window, measured("search-projects").center(), cx)).unwrap();
                        stage = 7805;
                        return false;
                    }
                    if stage == 7605 {
                        let position = measured("new-chat-tab").center();
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                        plus_pressed = Some(Instant::now());
                        stage = 7606;
                        return false;
                    }
                    if stage == 7606 {
                        if plus_pressed.unwrap().elapsed() < Duration::from_millis(500) { return false; }
                        assert_eq!(owner.read(cx).workbench.new_tab_menu, Some(0), "holding + must open the new-tab menu");
                        let plus = measured("new-chat-tab");
                        let menu = measured("new-tab-menu");
                        assert!(menu.top() >= plus.bottom(), "new-tab menu belongs below its trigger");
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: plus.center(), button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                            crate::controls_smoke::key(window, cx, "down");
                            crate::controls_smoke::key(window, cx, "down");
                            crate::controls_smoke::key(window, cx, "enter");
                        }).unwrap();
                        stage = 7607;
                        return false;
                    }
                    if stage == 7400 {
                        let bounds = measured("toolbar-view-selector");
                        assert_eq!(bounds.size.height, px(30.), "selector must align without border-induced inset drift");
                        for (index, name) in ["view-label-chat", "view-label-graph", "view-label-trace"].into_iter().enumerate() {
                            let label = measured(name);
                            assert!((label.center().y - bounds.center().y).abs() <= px(0.5));
                            assert!((label.center().x - bounds.left() - px(3. + (index as f32 + 0.5) * 76.)).abs() <= px(0.5), "each label must be centered in its equal-width segment");
                        }
                        let source = point(bounds.left() + bounds.size.width / 6., bounds.center().y);
                        let target = point(bounds.right() - bounds.size.width / 6., bounds.center().y);
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: source, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: target, pressed_button: Some(MouseButton::Left), ..Default::default() }), cx);
                        }).unwrap();
                        assert_eq!(owner.read(cx).view, MainView::Chat, "scrubbing previews selection without replacing the active view");
                        selector_pressed = Some(Instant::now());
                        stage = 7401;
                        return false;
                    }
                    if stage == 7401 {
                        if selector_pressed.unwrap().elapsed() < Duration::from_millis(350) { return false; }
                        let bounds = measured("toolbar-view-selector");
                        let lens = measured("toolbar-view-lens");
                        assert!((lens.center().y - bounds.center().y).abs() <= px(1.), "glass lens must stay vertically centered while pressed");
                        if !owner.read(cx).reduce_motion && !cx.reduce_motion() {
                            assert!(lens.size.width > px(79.), "pressed glass lens must visibly expand");
                        }
                        let target = point(bounds.right() - bounds.size.width / 6., bounds.center().y);
                        parent.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: target, button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                        assert_eq!(owner.read(cx).view, MainView::Trace, "release must commit the dragged-to segment");
                        parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "left")).unwrap();
                        assert_eq!(owner.read(cx).view, MainView::Graph);
                        parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "home")).unwrap();
                        assert_eq!(owner.read(cx).view, MainView::Chat);
                        parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                            app.harness.modes = Some(dunlin_agent::SessionModes {
                                config_id: None, current: "ask".into(), choices: ["ask", "plan"].into_iter().map(|id| dunlin_agent::ModelChoice {
                                    id: id.into(), name: id.into(), description: None,
                                }).collect(),
                            });
                            app.harness.mode_changing = true;
                            app.composer.update(cx, |state, cx| state.focus(window, cx));
                        })).unwrap();
                        stage = 7402;
                        return false;
                    }
                    if stage == 7402 {
                        parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "shift-tab")).unwrap();
                        parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                            assert!(app.composer.read(cx).focus_handle(cx).is_focused(window), "Shift+Tab must route to chat mode cycling, not leave the composer");
                            assert_eq!(app.harness.modes.as_ref().unwrap().current, "ask", "busy mode changes must not invent a local harness state");
                            app.harness.mode_changing = false;
                            app.harness.modes = None;
                        })).unwrap();
                        stage = 7000;
                        return false;
                    }
                     assert!(started.elapsed() < Duration::from_secs(90), "workspace smoke stalled at {stage}: {:?}", owner.read(cx).notice);
                     if let Some(key) = match stage {
                         20001 | 20008 => Some("home"),
                         20002 | 20006 => Some("end"),
                         20004 => Some("down"),
                         20009 => Some("escape"),
                         _ => None,
                     } {
                         parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, key)).unwrap();
                     }
                    parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                        match stage {
                              7200 => {
                                  let root = std::path::Path::new(&app.project().path);
                                  std::fs::write(root.join("package.json"), r#"{"dependencies":{"fixture-dependency":"*"},"scripts":{"dev":"echo fixture"}}"#).unwrap();
                                  std::fs::write(root.join("install-fixture.sh"), "#!/bin/sh\nprintf 'Installing fixture dependency\\n'\n/bin/sleep 1\n/bin/mkdir -p node_modules/fixture-dependency\nprintf '{}' > node_modules/fixture-dependency/package.json\n").unwrap();
                                  app.toggle_bottom_panel(crate::app::BottomPanel::Run, window, cx);
                                  stage = 7201;
                              }
                              7201 if !app.run.loading && !app.run.dependencies.is_empty() => {
                                  assert!(!app.run.dependencies[0].installed);
                                  let mut candidate = app.run.dependencies[0].candidate.clone();
                                  candidate.program = "/bin/sh".into();
                                  candidate.args = vec!["install-fixture.sh".into()];
                                  app.start_run(window, candidate, cx);
                                  stage = 7202;
                              }
                              7203 if app.showing_dependency_terminal() => {
                                  assert_eq!(app.bottom_panel, crate::app::BottomPanel::Run);
                                  assert_eq!(app.run.history[0].status, "running");
                                  assert_eq!(app.terminal.tabs.len(), 1);
                                  stage = 7204;
                              }
                              7204 if !app.showing_dependency_terminal() && !app.run.loading && app.run.history.first().is_some_and(|r| r.success == Some(true)) => {
                                  assert_eq!(app.bottom_panel, crate::app::BottomPanel::Run);
                                  assert!(app.terminal.tabs.is_empty(), "successful install must remove its temporary terminal");
                                  assert!(app.run.dependencies[0].installed, "install completion must recheck dependency presence");
                                  let mut candidate = app.run.dependencies[0].candidate.clone();
                                  candidate.program = "/bin/sh".into();
                                  candidate.args = vec!["-c".into(), "printf 'fixture install failure\\n'; exit 1".into()];
                                  app.start_run(window, candidate, cx);
                                  stage = 7206;
                              }
                              7207 if app.run.history.first().is_some_and(|r| r.success == Some(false)) => {
                                  assert!(app.showing_dependency_terminal(), "failure must retain the output instead of returning to Run");
                                  assert_eq!(app.bottom_panel, crate::app::BottomPanel::Run);
                                  let tab = &app.terminal.tabs[0];
                                  assert!(tab.screen.plain_text().contains("fixture install failure"));
                                  let id = tab.id.clone();
                                  app.close_terminal_tab(&id, cx);
                                  app.run = crate::run::State::default();
                                  app.close_bottom_panel(cx);
                                  let root = std::path::Path::new(&app.project().path);
                                  std::fs::remove_file(root.join("package.json")).unwrap();
                                  std::fs::remove_file(root.join("install-fixture.sh")).unwrap();
                                  std::fs::remove_file(root.join("node_modules/fixture-dependency/package.json")).unwrap();
                                  std::fs::remove_dir(root.join("node_modules/fixture-dependency")).unwrap();
                                  std::fs::remove_dir(root.join("node_modules")).unwrap();
                                  stage = 7400;
                              }
                              7000 => {
                                  assert!(!app.session_visible(app.session()));
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-sessions")), "connecting a draft must not flash New session");
                                  app.harness.status = crate::harness::HarnessStatus::Ready { agent: "Fixture".into(), version: "1".into() };
                                  stage = 7001;
                              }
                              7001 => {
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-sessions")), "an idle empty connection is still a draft");
                                  app.harness.status = crate::harness::HarnessStatus::Offline;
                                  app.harness.transcript.push(ChatItem::User { text: "Fixture prompt".into(), at_ms: 1, timing: None });
                                  app.harness.transcript.push(ChatItem::AgentText { text: "A short answer should not constrain the conversation width.".into(), streaming: false, message_id: None, at_ms: 2 });
                                  assert!(app.session_visible(app.session()));
                                   app.open_workbench_file(&format!("{}/file.rs", app.project().path), None, window, cx);
                                   stage = 7100;
                               }
                                 7100 if app.active_file_tab().is_some_and(|tab| !tab.loading) => {
                                     let map = measured("editor-minimap");
                                     let surface = measured("reading-surface");
                                     assert!((map.top() - surface.top()).abs() <= px(1.) && (map.bottom() - surface.bottom()).abs() <= px(1.), "file map must reach both edges of its reading surface");
                                    assert!(app.render_workbench_tabs(0, cx).1, "a selected file after the chat tab needs a rounded left corner");
                                    assert_painted_corners(window, "reading-surface", true);
                                    app.editor_minimap = false;
                                    stage = 71009;
                                }
                                71009 => {
                                    assert_painted_corners(window, "reading-surface", true);
                                    app.editor_minimap = true;
                                   let file = app.workbench.active.clone().unwrap();
                                   let editor = app.editor.clone();
                                   app.split_workspace(window, cx);
                                    assert_eq!(app.workbench.panes.active[0], Some(crate::workspace_panes::TabKey::File(file.clone())));
                                    assert!(!app.render_workbench_tabs(0, cx).1, "the first tab in a split pane must join its surface");
                                   assert!(app.focus_workspace_pane(1, window, cx));
                                   assert!(!app.workbench.show_file);
                                   assert_eq!(app.harness.transcript.len(), 2, "file/chat split must retain the live transcript");
                                   app.composer.update(cx, |input, cx| input.set_value("left draft", window, cx));
                                   assert!(app.focus_workspace_pane(0, window, cx));
                                   assert_eq!(app.editor, editor, "pane switching must use the canonical editor");
                                   app.editor.update(cx, |input, cx| { input.set_selected_range(0..input.text().len(), cx); input.replace("unsaved pane edit\n", window, cx); });
                                    stage = 7105;
                                }
                                7106 | 7108 => {
                                    let file = app.workbench.active.clone().unwrap();
                                    assert_eq!(app.workbench.panes.owner(&crate::workspace_panes::TabKey::File(file.clone())), if stage == 7106 { 1 } else { 0 });
                                    assert_eq!(app.workbench.tabs.iter().find(|tab| tab.id == file).unwrap().editor, app.editor);
                                    assert_eq!(app.editor.read(cx).value().as_ref(), "unsaved pane edit\n", "drag must move the canonical buffer, not reopen the file");
                                    stage = if stage == 7106 { 7107 } else { 7101 };
                                }
                               7101 => {
                                   assert!(app.has_dirty_tabs(), "editing the focused pane must mark its canonical buffer dirty");
                                   let file = app.workbench.active.clone().unwrap();
                                   app.request_close_tab(&file, window, cx);
                                   assert!(app.pending_editor.is_some(), "split-pane dirty close must still require a decision");
                                   app.cancel_pending_editor(cx);
                                   assert!(app.workbench.tabs.iter().any(|tab| tab.id == file && tab.dirty));
                                   assert!(app.focus_workspace_pane(1, window, cx));
                                   assert_eq!(app.composer.read(cx).value().as_ref(), "left draft");
                                   app.harness.status = crate::harness::HarnessStatus::Starting;
                                   app.sync_workspace_panes(window, cx);
                                   assert_eq!(app.workbench.tabs.iter().find(|tab| tab.id == file).unwrap().editor.read(cx).value().as_ref(), "unsaved pane edit\n");
                                   app.harness.status = crate::harness::HarnessStatus::Offline;
                                   app.merge_workspace(cx);
                                   app.activate_file_tab(&file, window, cx);
                                   app.editor.update(cx, |input, cx| { input.set_selected_range(0..input.text().len(), cx); input.replace("original\n", window, cx); });
                                    stage = 7150;
                               }
                               7102 => {
                                   assert!(!app.has_dirty_tabs(), "restoring original buffer text must clear its dirty marker");
                                   let file = app.workbench.active.clone().unwrap();
                                    app.request_close_tab(&file, window, cx);
                                    app.open_workbench_file(&format!("{}/README.md", app.project().path), None, window, cx);
                                    stage = 71020;
                                }
                                71020 if app.active_file_tab().is_some_and(|tab| !tab.loading) => {
                                    assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                    let file = app.workbench.active.clone().unwrap();
                                    app.request_close_tab(&file, window, cx);
                                    app.show_chat_tab(window, cx);
                                   app.composer.update(cx, |input, cx| input.set_value("", window, cx));
                                   app.open_remote_connection(window, cx);
                                   assert!(app.remote_connection.open && app.navigation_blocked());
                                   assert!(!app.remote_loading && app.terminal.tabs.is_empty());
                                   stage = 7103;
                               }
                               7103 if app.overlay_window.is_open() => {
                                   app.remote_connection.ssh = true;
                                   cx.notify();
                                   stage = 7104;
                               }
                               7104 => {
                                   assert!(app.remote_connection.identity.is_some() && app.remote_connection.port.is_some());
                                   app.close_remote_connection(window, cx);
                                   assert!(!app.remote_connection.open && app.terminal.tabs.is_empty());
                                   app.new_session(cx);
                                   stage = 7110;
                               }
                                7110 => {
                                    assert_eq!(app.composer.read(cx).presentation().placeholder().as_ref(), format!("Ask {} to build, fix, or explain…", app.session_harness()));
                                   app.composer.update(cx, |input, cx| input.set_value("new-session draft", window, cx));
                                   let new_session = app.session().id.clone();
                                   app.split_workspace(window, cx);
                                   assert!(app.focus_workspace_pane(1, window, cx));
                                   assert_ne!(app.session().id, new_session);
                                   app.composer.update(cx, |input, cx| input.set_value("original-session draft", window, cx));
                                   let original = app.session().id.clone();
                                   assert!(app.focus_workspace_pane(0, window, cx));
                                   assert_eq!(app.session().id, new_session);
                                   assert_eq!(app.composer.read(cx).value().as_ref(), "new-session draft");
                                   assert!(app.focus_workspace_pane(1, window, cx));
                                   assert_eq!(app.session().id, original);
                                   assert_eq!(app.composer.read(cx).value().as_ref(), "original-session draft");
                                   stage = 7111;
                               }
                               7111 => {
                                   let new_session = match app.workbench.panes.active[0].clone().unwrap() { crate::workspace_panes::TabKey::Chat(id) => id, _ => panic!("expected independent chat pane") };
                                   let original = app.session().id.clone();
                                   app.harness.status = crate::harness::HarnessStatus::Starting;
                                   assert!(!app.focus_workspace_pane(0, window, cx), "connecting session must block input routing into a different session");
                                   assert_eq!(app.session().id, original);
                                   assert_eq!(app.workbench.panes.focused, 1);
                                   app.harness.status = crate::harness::HarnessStatus::Offline;
                                   assert!(app.focus_workspace_pane(0, window, cx));
                                   assert_eq!(app.composer.read(cx).value().as_ref(), "new-session draft");
                                   assert!(app.focus_workspace_pane(1, window, cx));
                                   assert_eq!(app.composer.read(cx).value().as_ref(), "original-session draft");
                                   app.merge_workspace(cx);
                                   app.close_session_tab(&new_session, window, cx);
                                   app.composer.update(cx, |input, cx| input.set_value("", window, cx));
                                   stage = 0;
                               }
                              0 if !app.git_loading => {
                                  app.smoke_navigation_layout(window, cx);
                                 crate::project_catalog::smoke_filters(app);
                                 app.smoke_group_appearance(window, cx);
                                assert!(measured("conversation").size.width > px(824.));
                                assert!(measured("composer").size.width <= px(824.));
                                 assert!(measured("project-sessions").size.height > px(10.));
                                  comfortable_project_height = measured("active-project-row").size.height;
                                  rail_origin = measured("rail-projects").origin;
                                  sidebar_width = measured("sidebar").size.width;
                                   assert!(measured("titlebar-content").size.height <= px(38.));
                                   for name in ["title-sidebar", "navigation-back"] {
                                       let control = measured(name);
                                        assert_eq!(control.size, size(px(28.), px(28.)));
                                       assert!((control.center().y - measured("titlebar-content").center().y).abs() < px(1.));
                                   }
                                    assert_eq!(measured("document-tabs-strip").size.height, px(36.));
                                    assert_eq!(measured("reading-surface").top(), measured("document-tabs-strip").bottom());
                                    let host = measured("host-menu-button");
                                    let icon = measured("host-icon");
                                    let dot = measured("host-status-dot");
                                    assert!((icon.center().x - host.center().x).abs() < px(1.));
                                    assert!((icon.center().y - host.center().y).abs() < px(1.));
                                    assert!(dot.left() < icon.right() && dot.right() > icon.right());
                                    assert!(dot.top() >= icon.center().y && dot.bottom() <= icon.bottom());
                                    assert!((measured("reading-surface").bottom() - measured("workspace-surface").bottom()).abs() < px(1.));
                                  assert_eq!(measured("rail-projects").size, size(px(36.), px(36.)));
                                  component::Root::update(window, cx, |root, _, _| {
                                      assert_eq!(root.style().background, Some(transparent_black().into()), "opaque kit root hides the native material");
                                  });
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("account-usage-panel")));
                                 app.density = crate::app::UiDensity::Compact;
                                 dunlin_ui::scale::set_density(dunlin_ui::scale::Density::Compact);
                                 stage = 5001;
                             }
                             5001 => {
                                 assert!(measured("active-project-row").size.height < comfortable_project_height);
                                  assert!(measured("active-project-row").size.height >= px(32.));
                                  assert!(measured("active-project-row").size.height <= px(34.));
                                  app.density = crate::app::UiDensity::Comfortable;
                                  dunlin_ui::scale::set_density(dunlin_ui::scale::Density::Comfortable);
                                  app.sidebar_collapsed = true;
                                  stage = 5003;
                              }
                              5003 => {
                                  assert_eq!(measured("rail-projects").origin, rail_origin, "rail jumped during collapse");
                                  if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("sidebar")) { return false; }
                                  app.sidebar_collapsed = false;
                                  stage = 5004;
                              }
                              5004 => {
                                  assert_eq!(measured("rail-projects").origin, rail_origin, "rail jumped during expansion");
                                  if measured("workspace-surface").left() < measured("navigation-rail").right() + sidebar_width - px(1.) { return false; }
                                  stage = 5006;
                              }
                               5006 => {
                                   let scope = app.usage_target();
                                   let cached = app.active_usages();
                                   app.usage_cache.record(scope.clone(), cached);
                                   app.usage_request = Some(scope);
                                   stage = 5007;
                               }
                              5007 => {
                                  let panel = measured("account-usage-panel");
                                  assert!(panel.left() > window.viewport_size().width - px(500.), "usage must open at bottom right, not in the sidebar");
                                  assert!(panel.bottom() <= window.viewport_size().height);
                                   usage_before = Some(panel);
                                   app.usage_request = None;
                                   stage = 50071;
                               }
                               50071 => {
                                   assert_eq!(measured("account-usage-panel"), usage_before.unwrap(), "usage moved when updating status disappeared");
                                   stage = 5008;
                              }
                              5009 => {
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("account-usage-panel")), "Escape did not close usage");
                                  stage = 5002;
                              }
                              5002 => {
                                   app.project_list.wrap_names = true;
                                   app.sidebar_width = 220.;
                                 app.projects[app.selected_project].name = "A deliberately long repository name that must wrap onto several lines in the project list instead of ending in an ellipsis".into();
                                 stage = 11;
                             }
                              11 => {
                                  assert!(measured("active-project-row").size.height <= px(80.), "narrow project names must not form a vertical letter tower");
                                 stage = 100;
                             }
                             100 => { assert!(app.group_settings_draft.is_some()); stage = 101; }
                             101 => { stage = 102; }
                             102 => { stage = 103; }
                             103 => {
                                 assert!(app.group_settings_draft.is_none());
                                 let saved = app.store.as_ref().unwrap().project_groups().unwrap().pop().unwrap();
                                 assert_eq!(saved.tone, dunlin_domain::ProjectTone::Teal);
                                 assert_eq!(saved.settings.icon.as_deref(), Some("kubernetes"));
                                 stage = 109;
                             }
                             109 => {
                                  assert_eq!(measured("group-image").size, size(px(16.), px(16.)));
                                 assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("group-dot")));
                                 stage = 104;
                             }
                             104 => { stage = 105; }
                             105 => { assert!(app.group_settings_draft.is_some()); stage = 106; }
                             106 => { stage = 107; }
                             107 => {
                                 assert!(app.group_settings_draft.is_none());
                                 let saved = app.store.as_ref().unwrap().project_groups().unwrap().pop().unwrap();
                                 assert!(saved.settings.icon.is_none());
                                 assert_eq!(saved.tone, dunlin_domain::ProjectTone::Teal);
                                 stage = 108;
                             }
                             108 => {
                                  assert_eq!(measured("group-dot").size, size(px(16.), px(16.)));
                                 assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("group-image")));
                                 stage = 12;
                             }
                              12 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-preview")) => {
                                  assert!(measured("active-project-row").size.height <= px(80.));
                                 assert!(measured("project-preview").size.width >= px(450.));
                                  app.projects[app.selected_project].name = "UI fixture".into(); app.project_list.wrap_names = false;
                                  app.sidebar_width = f32::from(sidebar_width);
                                 stage = 1;
                             }
                             1 => { measured("project-sessions"); app.view = MainView::Graph; stage = 2; }
                              2 => {
                                 let background = measured("graph-background"); let viewport = app.activity.graph_scroll.bounds();
                                 dunlin_ui::surface_test::assert_region_corners(window, background, "graph surfaces");
                                assert!((background.size.width - viewport.size.width).abs() < px(2.));
                                 assert!((background.size.height - viewport.size.height).abs() < px(2.));
                                 activity_transcript = Some(app.harness.transcript.clone());
                                 for index in 0..30 {
                                     app.harness.transcript.push(ChatItem::Child {
                                         id: format!("keyboard-child-{index}"),
                                         parent_id: if index == 0 { "root".into() } else { format!("keyboard-child-{}", index - 1) },
                                         title: Some(format!("Keyboard child {index}")), items: vec![], at_ms: 10 + index,
                                     });
                                     app.harness.transcript.push(ChatItem::User { text: format!("Keyboard request {index}"), timing: None, at_ms: 100 + index * 2 });
                                     app.harness.transcript.push(ChatItem::AgentText { text: "Fixture response".into(), streaming: false, message_id: None, at_ms: 101 + index * 2 });
                                 }
                                 stage = 20000;
                              }
                              20000 => { app.activity_focus.focus(window, cx); stage = 20001; }
                              20001 => {
                                  assert_eq!(app.selected_node.as_deref(), app.activity_visible_ids().first().map(String::as_str), "Graph Home must use the installed key handler");
                                  stage = 20002;
                              }
                              20002 => {
                                  let ids = app.activity_visible_ids();
                                  assert!(ids.len() > 20);
                                  assert_eq!(app.selected_node.as_deref(), ids.last().map(String::as_str));
                                  stage = 20003;
                              }
                              20003 => {
                                  assert!(app.activity.graph_scroll.offset().y < px(0.), "Graph End must reveal offscreen selection");
                                  app.activity_search.update(cx, |input, cx| input.replace_all("Keyboard child 0", window, cx));
                                  app.activity_search.read(cx).focus_handle(cx).focus(window, cx);
                                  stage = 20004;
                              }
                              20004 => {
                                  assert!(app.activity_search.read(cx).focus_handle(cx).is_focused(window), "Graph arrows must not steal search focus");
                                  let ids = app.activity_visible_ids();
                                  assert!(!ids.is_empty() && ids.len() < 10);
                                  assert!(app.selected_node.as_ref().is_none_or(|id| ids.contains(id)), "search must not retain a hidden graph selection");
                                  app.activity_search.update(cx, |input, cx| input.replace_all("", window, cx));
                                  app.view = MainView::Trace;
                                  app.selected_span = None;
                                  stage = 20005;
                              }
                              20005 => { app.activity_focus.focus(window, cx); stage = 20006; }
                              20006 => {
                                  let ids = app.activity_visible_ids();
                                  assert!(ids.len() > 20);
                                  assert_eq!(app.selected_span.as_deref(), ids.last().map(String::as_str), "Trace End must use the installed key handler");
                                  stage = 20007;
                              }
                              20007 => {
                                  assert!(app.activity.trace_scroll.0.borrow().base_handle.offset().y < px(0.), "Trace End must reveal its last virtual row");
                                  stage = 20008;
                              }
                              20008 => {
                                  assert_eq!(app.selected_span.as_deref(), app.activity_visible_ids().first().map(String::as_str));
                                  stage = 20009;
                              }
                              20009 => {
                                  assert!(app.selected_span.is_none() && app.activity_focus.is_focused(window), "Trace Escape must clear selection without losing tree focus");
                                  app.harness.transcript = activity_transcript.take().unwrap();
                                  app.selected_node = None;
                                  app.view = MainView::Graph;
                                  stage = 20010;
                              }
                              20010 => {
                                  dashboard_recent = Some(std::mem::take(&mut app.dashboard_settings.recent_projects));
                                 let project = app.project().id.clone();
                                 dashboard_summary = app.repo_summaries.values.get(&project).cloned();
                                 if let Some(summary) = app.repo_summaries.values.get_mut(&project) {
                                     summary.monorepo = Some("npm workspaces".into());
                                     summary.branch = Some("feature/a-long-branch-that-must-not-move-the-open-button".into());
                                 }
                                 for id in ["dashboard-tile-2", "dashboard-tile-3"] {
                                     let mut project = app.project().clone();
                                     project.id = id.into(); project.name = format!("A long project name that must not change the tile dimensions {id}");
                                     app.projects.push(project);
                                 }
                                 app.dashboard_settings.recent_projects = serde_json::from_value(serde_json::json!([
                                     {"id": app.project().id, "at_ms": 9999999999999u64},
                                     {"id": "dashboard-tile-2", "at_ms": 9999999999998u64},
                                     {"id": "dashboard-tile-3", "at_ms": 9999999999997u64}
                                 ])).unwrap();
                                 app.section = SidebarSection::Dashboard; stage = 7300;
                             }
                             7300 => {
                                 let first = measured("recent-tile-0");
                                 // Absolute probes measure inside the tile's 1 px border.
                                 assert_eq!(first.size, size(px(318.), px(130.)));
                                 assert_eq!(measured("recent-tile-1").size, first.size);
                                 assert_eq!(measured("recent-tile-2").size, first.size);
                                 assert_eq!(measured("recent-projects-heading").left(), first.left() - px(1.));
                                 app.projects.retain(|p| !p.id.starts_with("dashboard-tile-"));
                                 app.dashboard_settings.recent_projects = dashboard_recent.take().unwrap();
                                 if let Some(summary) = dashboard_summary.take() {
                                     app.repo_summaries.values.insert(app.project().id.clone(), summary);
                                 }
                                 stage = 3;
                             }
                             3 => {
                                 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("sidebar")) { return false; }
                                  assert!(!app.sidebar_collapsed);
                                  let picker_root = dunlin_domain::ProjectGroup { id: "picker-root".into(), name: "Picker root".into(),
                                      parent_id: None, tone: dunlin_domain::ProjectTone::Blue, settings: Default::default() };
                                  app.store.as_ref().unwrap().upsert_project_group(&picker_root).unwrap();
                                  app.project_groups.push(picker_root);
                                  let child = app.project_groups.iter_mut().find(|g| g.id == "appearance-fixture").unwrap();
                                  child.parent_id = Some("picker-root".into());
                                  app.store.as_ref().unwrap().upsert_project_group(child).unwrap();
                                  app.projects[app.selected_project].group_id = Some("appearance-fixture".into());
                                  app.store.as_ref().unwrap().upsert_project(app.project()).unwrap();
                                  let mut ungrouped = app.project().clone();
                                  ungrouped.id = "picker-top-level".into(); ungrouped.name = "Top-level folder".into();
                                  ungrouped.group_id = Some("missing-group".into()); ungrouped.trusted = false;
                                  app.projects.push(ungrouped);
                                  app.project_menu_open = true; stage = 30;
                             }
                             30 => {
                                  assert!(app.has_overlay() && !app.has_modal_overlay());
                                  assert!(measured("title-project-menu").top() >= px(44.));
                                   assert_eq!(measured("project-menu-child").left(), measured("project-menu-root").left());
                                   assert_eq!(measured("project-menu-active-row").left(), measured("project-menu-child").left());
                                   measured("project-menu-ungrouped-row");
                                   assert_eq!(measured("project-menu-active-row").size.height, px(42.));
                                  stage = 31;
                             }
                             31 => { assert!(app.project_menu_open); stage = 32; }
                             32 => {
                                  assert!(app.collapsed_project_groups.contains(&"picker-root".into()));
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-menu-child")));
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-menu-active-row")));
                                  measured("project-menu-ungrouped-row"); stage = 33;
                             }
                             33 => { assert!(app.project_menu_open); stage = 34; }
                             34 => { measured("project-menu-child"); measured("project-menu-active-row"); stage = 35; }
                             35 => {
                                  assert!(!app.project_menu_open);
                                  assert_eq!(app.project().id, "ui-fixture");
                                   app.projects.retain(|p| p.id != "picker-top-level");
                                   app.section = SidebarSection::Projects; app.view = MainView::Chat;
                                 app.request_git_action(Action::Switch { branch: "feature".into(), discard: false }, window, cx);
                                 stage = 350;
                             }
                             350 => {
                                 assert!(app.pending_git_action.is_none(), "a clean branch switch must not ask to keep changes");
                                 if app.git_history.running || app.git_loading { return false; }
                                 assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "feature");
                                 app.request_git_action(Action::Switch { branch: "main".into(), discard: false }, window, cx);
                                 stage = 351;
                             }
                             351 => {
                                 assert!(app.pending_git_action.is_none(), "returning to main with no changes must not prompt");
                                 if app.git_history.running || app.git_loading { return false; }
                                 assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "main");
                                 std::fs::write(root.join("file.rs"), "uncommitted\n").unwrap();
                                app.request_git_action(Action::Switch { branch: "feature".into(), discard: false }, window, cx); stage = 4;
                            }
                             4 if app.pending_git_action.is_some() => {
                                 assert!(measured("active-project-row").left() >= measured("sidebar").left() + px(40.));
                                assert!(app.has_modal_overlay()); app.cancel_git_action(window, cx);
                                assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "main");
                                 stage = 45;
                             }
                             45 if !app.git_loading && !app.git_history.running => {
                                 // Completing preflight also refreshes Git status.
                                 // A new preparation must wait for that read, just
                                 // like a user re-opening the action after Cancel.
                                 app.request_git_action(Action::Switch { branch: "feature".into(), discard: false }, window, cx);
                                 stage = 5;
                             }
                             5 if app.pending_git_action.is_some() && !app.git_loading && !app.git_history.running => { app.smoke_confirm_git_action(true, window, cx); stage = 6; }
                             5 if !app.git_loading && !app.git_history.running => {
                                 panic!("branch confirmation preparation failed: {:?}", app.toast.as_ref().map(|toast| &toast.message));
                             }
                            6 if !app.git_history.running && !app.git_loading => {
                                assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "feature");
                                assert_eq!(std::fs::read_to_string(root.join("file.rs")).unwrap(), "original\n");
                                 std::fs::write(root.join("file.rs"), "approved\n").unwrap();
                                 app.refresh_git_status(cx); stage = 60;
                             }
                             60 if !app.git_loading && !app.git_history.running => {
                                 app.request_git_action(Action::Discard { paths: vec!["file.rs".into()] }, window, cx); stage = 7;
                             }
                              7 if !app.git_history.running && app.pending_git_action.is_none() => {
                                  if app.toast.as_ref().is_some_and(|toast| toast.message == "Wait for existing repository/file operations to finish") {
                                      stage = 60;
                                      return false;
                                  }
                                  panic!("discard preparation failed: {:?}", app.toast.as_ref().map(|toast| &toast.message));
                             }
                             7 if app.pending_git_action.is_some() && !app.git_loading && !app.git_history.running => {
                                std::fs::write(root.join("file.rs"), "after approval\n").unwrap();
                                app.smoke_confirm_git_action(false, window, cx); stage = 8;
                            }
                             8 if !app.git_history.running => {
                                 assert_eq!(std::fs::read_to_string(root.join("file.rs")).unwrap(), "after approval\n");
                                 app.open_file("file.rs", window, cx);
                                 stage = 80;
                             }
                              80 if app.active_file_tab().is_some_and(|tab| !tab.loading) => {
                                   assert_eq!(app.workbench.tabs.len(), 1);
                                    let gap = measured("new-chat-tab").left() - measured("close-file-tab").right();
                                    assert!(gap >= px(0.) && gap < px(24.), "new-tab action must follow the tabs without overlapping them: {gap:?}");
                                    assert!((measured("new-chat-tab").center().y - measured("close-file-tab").center().y).abs() < px(1.));
                                  app.show_chat_tab(window, cx);
                                  app.harness.running = true;
                                  stage = 81;
                              }
                              81 => {
                                  assert!(app.workbench.show_file);
                                  assert!(app.workbench.closed_chats.contains(&app.session().id));
                                  assert!(app.harness.running, "closing Chat must not cancel the agent");
                                  assert!(app.harness.transcript.iter().any(|item| matches!(item, ChatItem::User { .. })));
                                  assert!((measured("chat-tab").center().y - measured("close-chat-tab").center().y).abs() < px(1.));
                                  app.harness.running = false;
                                  app.show_chat_tab(window, cx);
                                  assert!(!app.workbench.closed_chats.contains(&app.session().id));
                                   app.section = SidebarSection::SourceControl;
                                   app.open_git_history(window, cx);
                                  stage = 82;
                              }
                               82 => {
                                   assert!((measured("git-history-tab").center().y - measured("close-git-history-tab").center().y).abs() < px(1.));
                                   let history = measured("git-history-panel");
                                   let surface = measured("reading-surface");
                                    assert!((history.top() - surface.top()).abs() <= px(1.) && (history.bottom() - surface.bottom()).abs() <= px(1.), "Git history must fill its rounded reading surface without top/bottom bands");
                                    assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                  app.close_git_history(cx);
                                  app.show_chat_tab(window, cx);
                                  stage = 83;
                              }
                              83 => {
                                  app.section = SidebarSection::Sessions;
                                let mut chat = app.project().clone();
                                 chat.id = "general-chat".into(); chat.name = "General chat".into();
                                 chat.kind = dunlin_domain::ProjectKind::ChatSpace;
                                 chat.path = dir.join("general-chat").to_string_lossy().into_owned();
                                 std::fs::create_dir(&chat.path).unwrap();
                                 let mut other = chat.clone();
                                 other.id = "other-repo".into(); other.kind = dunlin_domain::ProjectKind::Repo;
                                 other.path = dir.join("other-repo").to_string_lossy().into_owned();
                                 std::fs::create_dir(&other.path).unwrap();
                                let mut group = crate::app::blank_session(&other.id, "Custom ACP", None);
                                group.id = "group-conversation".into(); group.title = "Group conversation".into();
                                app.group_contexts.insert(group.id.clone(), crate::project_groups::GroupContext {
                                    group: "work".into(), name: "Work".into(), roots: Vec::new(),
                                });
                                let general = crate::app::blank_session(&chat.id, "Custom ACP", None);
                                let mut general = dunlin_domain::SessionSummary { id: "general-conversation".into(), ..general };
                                let draft = crate::app::blank_session(&chat.id, "Custom ACP", None);
                                let mut unrelated = crate::app::blank_session(&other.id, "Custom ACP", None);
                                unrelated.title = "Other repository history".into();
                                // A default title with a persisted first message must be found across projects too.
                                general.title = "New session".into();
                                let store = app.store.as_ref().unwrap();
                                for project in [&chat, &other] { store.upsert_project(project).unwrap(); }
                                for session in [&group, &general, &draft, &unrelated] { store.upsert_session(session).unwrap(); }
                                store.save_transcript(&general.id, &vec![ChatItem::User { text: "General request".into(), timing: None, at_ms: 1 }]).unwrap();
                                store.save_transcript(&group.id, &vec![ChatItem::User { text: "Group request".into(), timing: None, at_ms: 1 }]).unwrap();
                                app.projects.extend([chat, other]);
                                app.all_sessions.extend([group, general, draft, unrelated]);
                                app.persist_settings(cx);
                                stage = 9;
                            }
                             9 if app.visible_session_count() == 3 => {
                                 assert_compact_session_row();
                                use crate::session_list::Scope;
                                 assert!(measured("general-sessions").size.height > px(24.));
                                 assert!(measured("current-project-sessions").size.height > px(24.));
                                 assert!((measured("general-sessions").size.height - measured("current-project-sessions").size.height).abs() <= px(1.), "session scopes must each receive half of the available panel height");
                                assert!(app.all_sessions.iter().any(|s| s.title == "Other repository history" && app.session_scope(s).is_none()));
                                assert_eq!(app.session_scope(app.all_sessions.iter().find(|s| s.id == "group-conversation").unwrap()), Some(Scope::General));
                                app.editor_dirty = true;
                                assert!(!app.open_session_id("general-conversation", cx));
                                assert_eq!(app.project().id, "ui-fixture");
                                assert!(app.pending_editor.is_some());
                                app.editor_dirty = false; app.pending_editor = None;
                                assert!(app.open_session_id("general-conversation", cx));
                                assert_eq!(app.project().id, "general-chat");
                                assert_eq!(app.session().id, "general-conversation");
                                assert!(crate::session_list::has_user_message(&app.harness.transcript));
                                stage = 10;
                            }
                             10 => {
                                 assert_compact_session_row();
                                assert_eq!(app.visible_session_count(), 2);
                                measured("general-sessions");
                                assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("current-project-sessions")));
                                assert!(app.open_session_id("group-conversation", cx));
                                assert_eq!(app.project().id, "other-repo");
                                assert_eq!(app.session().id, "group-conversation");
                                assert_eq!(app.visible_session_count(), 3);
                                 app.request_delete_session_id("general-conversation", cx);
                                 assert!(app.pending_delete.is_some());
                                 stage = 10000;
                             }
                             10000 => {
                                 assert!(app.pending_delete.is_none(), "Cancel control must dismiss deletion");
                                 assert!(app.all_sessions.iter().any(|s| s.id == "general-conversation"));
                                 assert!(app.store.as_ref().unwrap().sessions(None).unwrap().iter().any(|s| s.id == "general-conversation"), "Cancel must preserve persisted history");
                                 app.request_delete_session_id("general-conversation", cx);
                                 stage = 10001;
                             }
                              10001 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("delete-dialog")) => {
                                 let dialog = measured("delete-dialog");
                                 let message = measured("delete-message");
                                 assert!(message.left() >= dialog.left() && message.right() <= dialog.right(), "destructive-action copy must wrap inside the dialog");
                                 assert!(message.size.height > px(20.), "the full warning should wrap instead of escaping its panel");
                                  assert!(app.pending_delete.is_none(), "Delete control must complete deletion");
                                 assert!(!app.all_sessions.iter().any(|s| s.id == "general-conversation"));
                                 assert_eq!(app.session().id, "group-conversation");
                                assert_eq!(app.project().id, "other-repo");
                                assert_eq!(app.visible_session_count(), 2);
                                assert!(matches!(app.harness.transcript.first(), Some(ChatItem::User { text, .. }) if text == "Group request"));
                                assert!(!app.store.as_ref().unwrap().sessions(None).unwrap().iter().any(|s| s.id == "general-conversation"));
                                 stage = 190;
                             }
                             190 => { stage = 191; }
                             191 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("selected-host-status")) => {
                                 assert!(measured("selected-host-status").top() > measured("host-menu-button").bottom());
                                 stage = 192;
                             }
                             200 => {
                                 let answer = window.prompt(PromptLevel::Warning, "Commit on AI?", Some("Fixture only: no request or commit will run."), &["Cancel", "Analyze and commit"], cx);
                                 let answer = app.overlay_backdrop.track_native(answer, cx);
                                 cx.spawn(async move |_, _| { PROMPT_ANSWER.with(|value| *value.borrow_mut() = None); let result = answer.await.unwrap(); PROMPT_ANSWER.with(|value| *value.borrow_mut() = Some(result)); }).detach();
                                 stage = 201;
                             }
                             202 if !window.has_active_prompt() && PROMPT_ANSWER.with(|a| *a.borrow() == Some(1)) => {
                                 PROMPT_ANSWER.with(|a| *a.borrow_mut() = None);
                                 let answer = window.prompt(PromptLevel::Warning, "Three-answer safety check", Some("Escape must return the original Cancel index, not the destructive first answer."), &["Delete", "Cancel", "Keep"], cx);
                                 let answer = app.overlay_backdrop.track_native(answer, cx);
                                 cx.spawn(async move |_, _| { let result = answer.await.unwrap(); PROMPT_ANSWER.with(|value| *value.borrow_mut() = Some(result)); }).detach();
                                 stage = 203;
                             }
                             204 if !window.has_active_prompt() && PROMPT_ANSWER.with(|a| *a.borrow() == Some(1)) => {
                                 PROMPT_ANSWER.with(|a| *a.borrow_mut() = None);
                                 let answer = window.prompt(PromptLevel::Warning, "Default Enter must not delete", Some("Fixture: Enter on the initially focused dialog must cancel."), &[PromptButton::new("Delete"), PromptButton::cancel("Keep Running")], cx);
                                 let answer = app.overlay_backdrop.track_native(answer, cx);
                                 cx.spawn(async move |_, _| { let result = answer.await.unwrap(); PROMPT_ANSWER.with(|value| *value.borrow_mut() = Some(result)); }).detach();
                                 stage = 205;
                             }
                              206 if !window.has_active_prompt() && PROMPT_ANSWER.with(|a| *a.borrow() == Some(1)) => {
                                  app.settings_open = true;
                                  app.settings_tab = crate::app::SettingsTab::Advanced;
                                  app.qa_report = Some("Fixture environment details".into());
                                  stage = 207;
                              }
                              208 => {
                                  let before = settings_before.unwrap();
                                  let after = measured("settings-panel");
                                  assert!((after.left() - before.left() - px(20.)).abs() < px(1.));
                                  assert!((after.top() - before.top() - px(18.)).abs() < px(1.));
                                  stage = 209;
                              }
                              209 => {
                                  assert!(app.qa_report.is_none());
                                  app.set_appearance(crate::app::Appearance::System, window, cx);
                                  cx.set_window_appearance(Some(WindowAppearance::Dark));
                                  stage = 210;
                              }
                              210 if !dunlin_ui::theme::is_transitioning(cx) => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                   assert!(cx.theme().is_dark());
                                   assert!(chrome_background(palette(cx), false).a >= 0.8);
                                  cx.set_window_appearance(Some(WindowAppearance::Light));
                                   stage = 211;
                                   app.settings_tab = crate::app::SettingsTab::Harnesses;
                              }
                              211 if !dunlin_ui::theme::is_transitioning(cx) => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                   assert!(!cx.theme().is_dark());
                                    app.overlay_window.debug_window().unwrap()
                                        .update(cx, |_, window, _| assert_painted_corners(window, "settings-content", true)).unwrap();
                                    let tint: Rgba = chrome_background(palette(cx), false).into();
                                     assert!((0.8..=0.85).contains(&tint.a) && (tint.r - tint.g).abs() < 0.001 && (tint.g - tint.b).abs() < 0.001);
                                   app.set_appearance(crate::app::Appearance::Dark, window, cx);
                                   assert!(dunlin_ui::theme::is_transitioning(cx));
                                   assert_eq!(palette(cx).card, dunlin_ui::theme::light().card);
                                   cx.set_window_appearance(Some(WindowAppearance::Light));
                                  stage = 212;
                              }
                              212 if !dunlin_ui::theme::is_transitioning(cx) => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                   assert!(cx.theme().is_dark(), "explicit theme must not follow native changes");
                                    assert!(chrome_background(palette(cx), false).a >= 0.8);
                                   app.set_appearance(crate::app::Appearance::Light, window, cx);
                                   stage = 213;
                              }
                              213 => {
                                   let visible = palette(cx);
                                   app.set_appearance(crate::app::Appearance::Dark, window, cx);
                                   assert_eq!(palette(cx).card, visible.card, "reversing a theme transition must not jump");
                                   assert_eq!(palette(cx).chrome, visible.chrome);
                                   stage = 214;
                              }
                              214 if !dunlin_ui::theme::is_transitioning(cx) => {
                                   assert_eq!(palette(cx).card, dunlin_ui::theme::dark().card);
                                   let reduce_motion = app.reduce_motion;
                                   app.reduce_motion = true;
                                   app.set_appearance(crate::app::Appearance::Light, window, cx);
                                   assert!(!dunlin_ui::theme::is_transitioning(cx));
                                   assert_eq!(palette(cx).card, dunlin_ui::theme::light().card);
                                   app.reduce_motion = reduce_motion;
                                   app.set_appearance(crate::app::Appearance::System, window, cx);
                                  app.settings_open = false;
                                   app.merge_workspace(cx);
                                   app.close_bottom_panel(cx);
                                   app.show_chat_tab(window, cx);
                                   app.new_untitled_file(window, cx);
                                   scratch = app.workbench.active.clone();
                                   stage = 7600;
                             }
                             7600 => {
                                 assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                 let tab = app.active_file_tab().unwrap();
                                 assert!(tab.untitled && !tab.loading && !tab.readonly && !tab.dirty);
                                 assert!(!std::path::Path::new(&tab.inspector.path).exists(), "new buffers must not create a file");
                                 app.editor.update(cx, |state, cx| state.replace("scratch stays in memory\n", window, cx));
                                 stage = 7601;
                             }
                             7601 => {
                                 assert!(app.editor_dirty);
                                 app.request_close_inspector(cx);
                                 assert!(app.pending_editor.is_some(), "scratch edits must use the existing unsaved-changes guard");
                                 app.cancel_pending_editor(cx);
                                 app.open_terminal_tab(cx).unwrap();
                                 dock_terminal = app.terminal.active_tab().map(|tab| tab.id.clone());
                                 app.open_workspace_terminal(window, cx);
                                 stage = 7602;
                             }
                             7602 if app.terminal.active_tab().is_some_and(|tab| tab.workspace && tab.bounds.get().is_some()) => {
                                 assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                 let tab = app.terminal.active_tab().unwrap();
                                 assert!(matches!(app.current_workspace_tab(), Some(crate::workspace_panes::TabKey::Terminal(_))));
                                 assert_eq!(app.terminal.layout.as_ref().unwrap().leaves(), vec![dock_terminal.clone().unwrap()], "workspace terminal must not replace the existing dock layout");
                                  assert_eq!(tab.project, app.project().id);
                                  assert!(!BOUNDS.with(|bounds| bounds.borrow().as_ref().unwrap().contains_key("toolbar-view-selector")), "agent view selector must not overlay terminal tabs");
                                  app.terminal_write(b"printf 'TAB_READY\\n'; old=$(stty -g); stty -echo -icanon min 1 time 0; b=$(dd bs=1 count=1 2>/dev/null | od -An -tu1 | tr -d ' '); stty \"$old\"; printf 'TAB_BYTE:%s\\n' \"$b\"\r", cx);
                                  stage = 76021;
                              }
                              76022 if app.terminal.active_tab().is_some_and(|tab| tab.screen.screen().contents().contains("TAB_BYTE:9")) => {
                                  let tab = app.terminal.active_tab().unwrap();
                                  let key = crate::workspace_panes::TabKey::Terminal(tab.id.clone());
                                 app.terminal_write(b"printf 'workspace-tab-ready\\n'\r", cx);
                                 app.split_workspace(window, cx);
                                 app.drop_workspace_tab(&crate::workspace_panes::TabDrag { key, project: app.project().id.clone() }, 1, window, cx);
                                 stage = 7603;
                             }
                             7603 if app.terminal.active_tab().is_some_and(|tab| tab.screen.screen().contents().contains("workspace-tab-ready")) => {
                                 let id = app.terminal.active_tab().unwrap().id.clone();
                                 assert_eq!(app.workbench.panes.owner(&crate::workspace_panes::TabKey::Terminal(id.clone())), 1);
                                 app.close_terminal_tab(&id, cx);
                                 assert!(app.terminal.tabs.iter().any(|tab| Some(&tab.id) == dock_terminal.as_ref()), "closing a workspace terminal must leave the dock PTY running");
                                 app.close_terminal_tab(dock_terminal.as_deref().unwrap(), cx);
                                 app.merge_workspace(cx);
                                 app.activate_file_tab(scratch.as_deref().unwrap(), window, cx);
                                 assert_eq!(app.editor.read(cx).value().as_ref(), "scratch stays in memory\n");
                                 assert!(app.editor_dirty);
                                 app.request_close_inspector(cx);
                                 app.discard_pending_editor(window, cx);
                                 app.show_chat_tab(window, cx);
                                 stage = 7605;
                             }
                              7607 => {
                                  assert!(app.active_file_tab().is_some_and(|tab| tab.untitled && !tab.dirty), "held + menu must create the chosen empty buffer");
                                  assert!(app.workbench.new_tab_menu.is_none());
                                  app.editor.update(cx, |input, cx| input.replace("close-all draft\n", window, cx));
                                  closing_counts = (app.all_sessions.len(), app.harness.transcript.len());
                                  app.open_workspace_terminal(window, cx);
                                  app.open_git_history(window, cx);
                                  stage = 7700;
                              }
                              7703 if !window.has_active_prompt() => {
                                  assert!(app.has_dirty_tabs(), "Cancel must retain the unsaved buffer");
                                  assert!(app.git_history.is_open_for(app.project()));
                                  assert!(app.terminal.tabs.iter().any(|tab| tab.workspace));
                                  app.request_close_all_tabs(window, cx);
                                  stage = 7704;
                              }
                              7705 if !window.has_active_prompt() && app.workspace_tab_keys().is_empty() => {
                                  assert!(!app.has_dirty_tabs());
                                  assert!(app.terminal.tabs.is_empty());
                                  assert_eq!((app.all_sessions.len(), app.harness.transcript.len()), closing_counts, "closing tabs must not delete sessions or stop their transcript");
                                  app.show_chat_tab(window, cx);
                                  app.new_untitled_file(window, cx);
                                  app.open_workspace_terminal(window, cx);
                                  stage = 7706;
                              }
                              7706 => {
                                  app.request_close_all_tabs(window, cx);
                                  assert!(!window.has_active_prompt(), "clean mixed tabs close without a dirty-buffer prompt");
                                  assert!(app.workspace_tab_keys().is_empty());
                                  app.settings_open = true;
                                  app.settings_tab = crate::app::SettingsTab::Secrets;
                                  stage = 7730;
                              }
                              7731 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("vault-secret-form")) => {
                                  assert!(!app.vault.ui.draft.as_ref().unwrap().advanced);
                                  let form = measured("vault-secret-form");
                                  for field in ["Name", "Key"] {
                                      let bounds = measured(field);
                                      assert!(bounds.left() >= form.left() && bounds.right() <= form.right());
                                  }
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("Allowed API hosts")), "basic credentials need only name and key");
                                  app.vault.ui.draft.as_mut().unwrap().advanced = true;
                                  stage = 7732;
                              }
                              7732 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("Allowed API hosts")) => {
                                  app.vault.ui.draft = None;
                                  app.settings_open = false;
                                   app.set_vault_enabled(true, cx);
                                   assert!(app.vault.enabled, "credential broker must start in the isolated fixture: {:?}", app.vault.error);
                                  app.toggle_bottom_panel(crate::app::BottomPanel::Secrets, window, cx);
                                  stage = 7733;
                              }
                              7733 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("Key")) => {
                                  assert!(!app.vault.ui.panel.advanced);
                                  assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("Allowed API hosts")), "the bottom-panel form also starts with only name and key");
                                   app.vault.ui.panel.advanced = true;
                                   app.terminal_dock = crate::app::TerminalDock::Right;
                                   app.bottom_panel_width = 320.;
                                   stage = 7734;
                               }
                               7734 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("Allowed API hosts")) => {
                                   let panel = measured("vault-panel");
                                   for field in ["Name", "Key", "Allowed API hosts"] {
                                       let bounds = measured(field);
                                       assert!(bounds.left() >= panel.left() && bounds.right() <= panel.right(), "{field} escaped narrow Secrets panel");
                                   }
                                   assert!(measured("Key").top() >= measured("Name").bottom(), "narrow fields must stack");
                                   assert_eq!(measured("vault-create").size.height, measured("vault-name-input").size.height, "Secrets buttons must match input height");
                                   assert!(measured("vault-create").right() <= panel.right());
                                   app.bottom_panel_width = 480.;
                                   stage = 7735;
                               }
                               7735 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("vault-panel")) => {
                                   let panel = measured("vault-panel");
                                   for field in ["Name", "Key", "Allowed API hosts"] {
                                       let bounds = measured(field);
                                       assert!(bounds.left() >= panel.left() && bounds.right() <= panel.right());
                                   }
                                   app.set_vault_enabled(false, cx);
                                   app.close_bottom_panel(cx);
                                   app.section = SidebarSection::Projects;
                                   stage = 7800;
                              }
                              7801 => {
                                  assert!(app.command_open && app.command_projects_only);
                                  assert!(app.filtered_commands().iter().all(|item| matches!(item.command, crate::app::Command::SwitchProject(_))));
                                  let index = app.projects.iter().position(|project| !project.is_chat_space() && project.id != app.project().id).unwrap();
                                  search_target = app.projects[index].id.clone();
                                  app.projects[index].name = "Unique search destination".into();
                                  app.command_query.update(cx, |input, cx| input.set_value("unique search destination", window, cx));
                                  stage = 7802;
                              }
                              7803 => {
                                  assert_eq!(app.project().id, search_target, "project search must switch to the selected repository");
                                  assert!(!app.command_open);
                                  app.section = SidebarSection::Projects;
                                  stage = 7804;
                              }
                              7805 => {
                                  assert!(app.command_open && app.command_projects_only);
                                  assert!(app.command_query.read(cx).value().is_empty());
                                  app.command_query.update(cx, |input, cx| input.set_value("no-such-fixture-project", window, cx));
                                  stage = 7806;
                              }
                              7807 => {
                                  assert!(!app.command_open);
                                  assert_eq!(app.project().id, search_target, "Escape must not select a project");
                                  app.open_git_history(window, cx);
                                  app.new_untitled_file(window, cx);
                                  recent_file = app.workbench.active.clone().unwrap();
                                  app.open_git_history(window, cx);
                                  app.new_untitled_file(window, cx);
                                  stage = 7810;
                              }
                              7810 if app.editor.read(cx).line_height().is_some() => {
                                  zoom_before = f32::from(app.editor.read(cx).line_height().unwrap());
                                  stage = 7811;
                              }
                              7812 => {
                                  assert!(f32::from(app.editor.read(cx).line_height().unwrap()) > zoom_before, "code editor text must grow without scaling the sidebar");
                                  stage = 7813;
                              }
                              7814 => {
                                  let id = app.workbench.active.clone().unwrap();
                                  app.request_close_tab(&id, window, cx);
                                  stage = 7815;
                              }
                              7815 => {
                                  assert!(app.git_history.is_active_for(app.project()), "closing a new file must restore Git, not the first open file/chat");
                                  app.request_close_tab("missing-tab", window, cx);
                                  assert!(app.git_history.is_active_for(app.project()), "a stale file-close callback cannot close another tab");
                                  app.request_close_tab(&recent_file, window, cx);
                                  assert!(app.git_history.is_active_for(app.project()), "closing an inactive file must not steal focus");
                                  app.open_workspace_terminal(window, cx);
                                  stage = 7820;
                              }
                              7820 if app.terminal.active_tab().is_some_and(|tab| tab.bounds.get().is_some()) => {
                                  stage = 78201;
                              }
                              78201 => {
                                  zoom_before = f32::from(app.terminal.active_tab().unwrap().size.1);
                                  stage = 7821;
                              }
                              7822 => {
                                  assert!(f32::from(app.terminal.active_tab().unwrap().size.1) < zoom_before, "PTY columns must follow the larger terminal font");
                                  stage = 7823;
                              }
                              7824 => {
                                  let id = app.terminal.active_tab().unwrap().id.clone();
                                  app.close_terminal_tab(&id, cx);
                                  stage = 7825;
                              }
                              7825 => {
                                  assert!(app.git_history.is_active_for(app.project()), "closing a terminal must return to the previously selected Git tab");
                                  app.new_session(cx);
                                  app.harness.status = crate::harness::HarnessStatus::Offline;
                                  app.harness.transcript.push(ChatItem::AgentText { text: "Zoom line one\n\nZoom line two".into(), streaming: false, message_id: None, at_ms: 2 });
                                  stage = 7830;
                              }
                              7830 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("chat-reading-text")) => {
                                  zoom_before = f32::from(measured("chat-reading-text").size.height);
                                  stage = 7831;
                              }
                              7832 => {
                                  assert!(f32::from(measured("chat-reading-text").size.height) > zoom_before, "chat paragraphs must follow font zoom: before={zoom_before}, after={:?}, zoom={}, current={:?}", measured("chat-reading-text").size.height, crate::workspace_text::scale(cx), app.current_workspace_tab());
                                  stage = 7833;
                              }
                              7834 => {
                                  app.close_chat_tab(window, cx);
                                  stage = 7835;
                              }
                              7835 => {
                                  assert!(app.git_history.is_active_for(app.project()), "closing a new chat must restore the previous Git tab");
                                  assert_eq!(crate::workspace_text::scale(cx), 1.);
                                  let index = app.projects.iter().position(|project| project.id != app.project().id && !project.is_chat_space()).unwrap();
                                  search_target = app.projects[index].id.clone();
                                  app.projects[index].group_id = Some(app.project_groups[0].id.clone());
                                  app.projects[index].trusted = true;
                                  app.context_menu = Some(crate::app::ContextMenuState { target: crate::app::ContextTarget::Project(index), position: point(px(300.), px(150.)) });
                                  stage = 7900;
                              }
                              7900 => {
                                  assert!(app.project_group_editor.is_some() && app.navigation_blocked());
                                  assert_ne!(app.project().id, search_target);
                                  stage = 7901;
                              }
                              7901 if app.overlay_window.is_open() && BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-group-dialog")) => {
                                  assert!(measured("project-group-dialog").size.width >= px(500.));
                                  stage = 7902;
                              }
                              7902 => {
                                  assert!(app.project_group_editor.is_none());
                                  let index = app.projects.iter().position(|project| project.id == search_target).unwrap();
                                  assert!(app.projects[index].group_id.is_none(), "move must affect the right-clicked project");
                                  assert_ne!(index, app.selected_project);
                                  app.context_menu = Some(crate::app::ContextMenuState { target: crate::app::ContextTarget::Project(index), position: point(px(300.), px(150.)) });
                                  stage = 7903;
                              }
                              7903 => {
                                  let index = app.projects.iter().position(|project| project.id == search_target).unwrap();
                                  assert!(!app.projects[index].trusted);
                                  assert_ne!(index, app.selected_project);
                                  app.project_group_editor = Some(index);
                                  stage = 7904;
                              }
                              7905 => {
                                  assert!(app.project_group_editor.is_none());
                                  app.open_remote_connection(window, cx);
                                  app.remote_connection.ssh = true;
                                  stage = 7906;
                              }
                              7906 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("ssh-key-input")) => {
                                  let input = measured("ssh-key-input");
                                  let button = measured("ssh-key-button");
                                  assert_eq!(input.size.height, button.size.height);
                                  assert!((input.center().y - button.center().y).abs() <= px(1.), "key chooser must align with the input, not its label");
                                  app.close_remote_connection(window, cx);
                                  app.settings_open = true;
                                  app.settings_tab = crate::app::SettingsTab::Harnesses;
                                  recent_file = app.default_harness.clone();
                                  stage = 7910;
                              }
                              7910 => {
                                  assert!(!app.harness_visible("Claude Code"));
                                  assert_eq!(app.default_harness, recent_file, "visibility button must not select a new default");
                                  let harness = app.session_harness();
                                  let transcript = app.harness.transcript.len();
                                  app.custom_harness_command = "/usr/bin/false".into();
                                  assert!(app.visible_harness_names().iter().any(|name| name == "Custom ACP"));
                                  app.toggle_harness_visibility("Custom ACP", cx);
                                  assert!(!app.visible_harness_names().iter().any(|name| name == "Custom ACP"));
                                  assert_eq!(app.session_harness(), harness);
                                  assert_eq!(app.harness.transcript.len(), transcript);
                                  let stored: crate::app::StoredSettings = app.store.as_ref().unwrap().load_setting("app").unwrap().unwrap();
                                  assert!(stored.hidden_harnesses.iter().any(|name| name == "Custom ACP"));
                                  app.toggle_harness_visibility("Custom ACP", cx);
                                  assert!(app.visible_harness_names().iter().any(|name| name == "Custom ACP"));
                                  std::fs::write(dir.join("PASS"), "PASS: project actions/group dialog, harness visibility, SSH key geometry, project search, text zoom, MRU and workspace UI\n").unwrap();
                                 println!("PASS: workspace UI smoke"); cx.quit(); return true;
                            }
                            _ => {}
                        }
                        cx.notify(); false
                    })).unwrap()
                });
                if done { break; }
            }
        }).detach();
    });
}
