//! Isolated native acceptance: no real repositories, agents, credentials or data.
use crate::{
    app::{DunlinApp, StoredSettings},
    harness::ChatItem,
};
use dunlin_domain::{MainView, SidebarSection};
use dunlin_ui::{assets, component::TitleBar, *};
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
fn measured(name: &str) -> Bounds<Pixels> {
    BOUNDS.with(|map| {
        *map.borrow()
            .as_ref()
            .unwrap()
            .get(name)
            .unwrap_or_else(|| panic!("missing probe: {name}"))
    })
}
fn click(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
    window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx);
    window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
    window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
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
    std::fs::write(root.join("README.md"), "# Fixture repository\n\nInert README hover preview.\n").unwrap();
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
        dunlin_ui::init(cx); crate::logging::init();
        BOUNDS.with(|map| *map.borrow_mut() = Some(HashMap::new()));
        let (parent, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(60.), px(60.)), size(px(1600.), px(950.))))),
            ..TitleBar::window_options() }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        parent.update(cx, |_, window, cx| crate::terminal::smoke_glyphs(window, cx)).unwrap();
        owner.update(cx, |app, cx| {
            app.section = SidebarSection::Projects; app.view = MainView::Chat; app.sidebar_collapsed = false;
            app.harness.transcript.push(ChatItem::User { text: "Fixture prompt".into(), at_ms: 1, timing: None });
            app.harness.transcript.push(ChatItem::AgentText { text: "A short answer should not constrain the conversation width.".into(), streaming: false, message_id: None, at_ms: 2 });
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0; let mut hovered = false;
            let mut comfortable_project_height = px(0.);
            let mut settings_before: Option<Bounds<Pixels>> = None;
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
                        100 => Some("group-settings-button"), 101 => Some("group-icon-kubernetes"),
                        102 => Some("group-color-teal"), 103 | 107 => Some("group-settings-save"),
                        104 => Some("group-settings-button"),
                        106 => Some("group-icon-dot"),
                        31 | 33 => Some("project-menu-root"),
                        190 => Some("host-menu-button"),
                        81 => Some("close-chat-tab"),
                        209 => Some("close-qa-report"),
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
                    assert!(started.elapsed() < Duration::from_secs(90), "workspace smoke stalled at {stage}: {:?}", owner.read(cx).notice);
                    parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                        match stage {
                              0 if !app.git_loading => {
                                  app.smoke_navigation_layout(window, cx);
                                 crate::project_catalog::smoke_filters(app);
                                 app.smoke_group_appearance(window, cx);
                                assert!(measured("conversation").size.width > px(824.));
                                assert!(measured("composer").size.width <= px(824.));
                                 assert!(measured("project-sessions").size.height > px(10.));
                                 comfortable_project_height = measured("active-project-row").size.height;
                                 app.density = crate::app::UiDensity::Compact;
                                 dunlin_ui::scale::set_density(dunlin_ui::scale::Density::Compact);
                                 stage = 5001;
                             }
                             5001 => {
                                 assert!(measured("active-project-row").size.height < comfortable_project_height);
                                 assert!(measured("active-project-row").size.height >= px(32.));
                                 app.density = crate::app::UiDensity::Comfortable;
                                 dunlin_ui::scale::set_density(dunlin_ui::scale::Density::Comfortable);
                                 stage = 5002;
                             }
                             5002 => {
                                  app.project_list.wrap_names = true;
                                 app.projects[app.selected_project].name = "A deliberately long repository name that must wrap onto several lines in the project list instead of ending in an ellipsis".into();
                                 stage = 11;
                             }
                             11 => {
                                 assert!(measured("active-project-row").size.height > px(46.));
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
                                 measured("group-image");
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
                                 measured("group-dot");
                                 assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("group-image")));
                                 stage = 12;
                             }
                             12 if BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("project-preview")) => {
                                 assert!(measured("active-project-row").size.height > px(46.));
                                 assert!(measured("project-preview").size.width >= px(450.));
                                 app.projects[app.selected_project].name = "UI fixture".into(); app.project_list.wrap_names = false;
                                 stage = 1;
                             }
                             1 => { measured("project-sessions"); app.view = MainView::Graph; stage = 2; }
                            2 => {
                                let background = measured("graph-background"); let viewport = app.activity.graph_scroll.bounds();
                                assert!((background.size.width - viewport.size.width).abs() < px(2.));
                                assert!((background.size.height - viewport.size.height).abs() < px(2.));
                                app.section = SidebarSection::Dashboard; stage = 3;
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
                                  assert!(measured("project-menu-child").left() > measured("project-menu-root").left());
                                  assert!(measured("project-menu-active-row").left() > measured("project-menu-child").left());
                                  measured("project-menu-ungrouped-row");
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
                                std::fs::write(root.join("file.rs"), "uncommitted\n").unwrap();
                                app.request_git_action(Action::Switch { branch: "feature".into(), discard: false }, window, cx); stage = 4;
                            }
                             4 if app.pending_git_action.is_some() => {
                                 assert!(measured("active-project-row").left() >= measured("sidebar").left() + px(40.));
                                assert!(app.has_modal_overlay()); app.cancel_git_action(window, cx);
                                assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "main");
                                app.request_git_action(Action::Switch { branch: "feature".into(), discard: false }, window, cx); stage = 5;
                            }
                            5 if app.pending_git_action.is_some() => { app.smoke_confirm_git_action(true, window, cx); stage = 6; }
                            6 if !app.git_history.running && !app.git_loading => {
                                assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "feature");
                                assert_eq!(std::fs::read_to_string(root.join("file.rs")).unwrap(), "original\n");
                                std::fs::write(root.join("file.rs"), "approved\n").unwrap();
                                app.request_git_action(Action::Discard { paths: vec!["file.rs".into()] }, window, cx); stage = 7;
                            }
                            7 if app.pending_git_action.is_some() => {
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
                                use crate::session_list::Scope;
                                assert!(measured("general-sessions").size.height > px(24.));
                                assert!(measured("current-project-sessions").size.height > px(24.));
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
                                assert_eq!(app.visible_session_count(), 2);
                                measured("general-sessions");
                                assert!(!BOUNDS.with(|m| m.borrow().as_ref().unwrap().contains_key("current-project-sessions")));
                                assert!(app.open_session_id("group-conversation", cx));
                                assert_eq!(app.project().id, "other-repo");
                                assert_eq!(app.session().id, "group-conversation");
                                assert_eq!(app.visible_session_count(), 3);
                                let current = app.session().id.clone();
                                app.request_delete_session_id("general-conversation", cx);
                                assert!(app.pending_delete.is_some());
                                app.cancel_delete(cx);
                                assert!(app.all_sessions.iter().any(|s| s.id == "general-conversation"));
                                app.request_delete_session_id("general-conversation", cx);
                                app.confirm_delete(cx);
                                assert!(!app.all_sessions.iter().any(|s| s.id == "general-conversation"));
                                assert_eq!(app.session().id, current);
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
                              210 => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                  assert!(cx.theme().is_dark());
                                  cx.set_window_appearance(Some(WindowAppearance::Light));
                                  stage = 211;
                              }
                              211 => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                  assert!(!cx.theme().is_dark());
                                  app.set_appearance(crate::app::Appearance::Dark, window, cx);
                                  cx.set_window_appearance(Some(WindowAppearance::Light));
                                  stage = 212;
                              }
                              212 => {
                                  use dunlin_ui::component::ActiveTheme as _;
                                  assert!(cx.theme().is_dark(), "explicit theme must not follow native changes");
                                  app.set_appearance(crate::app::Appearance::System, window, cx);
                                  app.settings_open = false;
                                  std::fs::write(dir.join("PASS"), "PASS: workspace, density project-row geometry, draggable Settings, dismissible QA, native system appearance and explicit theme override, content-only navigation/resize, nested project picker, themed confirmations, session and Git safety\n").unwrap();
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
