//! Isolated native import UI: synthetic histories only, no real harness or account.
use super::*;
use crate::app::StoredSettings;
use dunlin_ui::{assets, component::TitleBar};
use std::time::Instant;

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("fresh import fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-session-import-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture directory must not already exist");
    let project_path = dir.join("project");
    std::fs::create_dir(&project_path).unwrap();
    let other_path = dir.join("other-project");
    std::fs::create_dir(&other_path).unwrap();
    let source = dir.join("claude/projects/fixture");
    std::fs::create_dir_all(&source).unwrap();
    for index in 0..700 {
        let content = serde_json::json!({
            "type":"user", "sessionId": format!("fixture-{index}"),
            "cwd": if index == 699 { &other_path } else { &project_path },
            "message":{"content":format!("Question {index}")}
        });
        std::fs::write(
            source.join(format!("{index}.jsonl")),
            serde_json::to_vec(&content).unwrap(),
        )
        .unwrap();
    }
    std::fs::write(source.join("journal.jsonl"), "{}\n").unwrap();
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
            id: "fixture-project".into(),
            name: "Import fixture".into(),
            path: project_path.to_string_lossy().into(),
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
    let mut other = store.projects().unwrap().pop().unwrap();
    other.id = "other-project".into();
    other.name = "Other repository".into();
    other.path = other_path.to_string_lossy().into();
    drop(store);
    // SAFETY: main invokes the fixture before starting application/worker threads.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("claude"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.import-smoke", "Dunlin Import Smoke");
        dunlin_ui::init(cx);
        crate::logging::init();
        let mut owner = None;
        let (window, _) = open_window(WindowOptions {
            window_bounds:Some(WindowBounds::Windowed(Bounds::new(point(px(80.),px(80.)),size(px(1100.),px(720.))))),
            ..TitleBar::window_options()
        }, cx, |window, cx| {
            let app = cx.new(|cx| DunlinApp::new(window, cx));
            owner = Some(app.clone());
            cx.new(|cx| {
                let changes = cx.observe(&app, |_, _, cx| cx.notify());
                ImportView { app, _changes:changes }
            })
        }).unwrap();
        let owner = owner.unwrap();
        owner.update(cx, |app, cx| {
            let project = app.projects.iter().position(|p| p.id == "fixture-project").unwrap();
             app.select_project(project, cx);
             app.settings_open = true;
             app.settings_tab = crate::app::SettingsTab::Harnesses;
            app.scan_session_import("Claude Code", cx);
            assert!(app.session_import.loading);
            assert!(app.session_import.progress.is_some());
        });
        cx.activate(true);
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let mut stage = 0;
            let mut frames = 0;
            let mut staging = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(40)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(30), "import smoke stalled at stage {stage}");
                    window.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                    frames += 1;
                    owner.update(cx, |app, cx| {
                         if app.session_import.loading {
                             assert!(app.session_import.progress.is_some());
                             if let Some(preview) = &app.session_import.preview {
                                 staging = Some(preview.staging.path().to_path_buf());
                             }
                             return;
                         }
                         match stage {
                             0 | 1 => {
                                 assert!(!app.session_import.applying);
                                 assert!(app.session_import.preview.is_none());
                                 assert!(app.session_import.error.is_none());
                                 if stage == 0 {
                                     assert_eq!(app.all_sessions.iter().filter(|s| s.id.starts_with("import-")).count(), 699);
                                     assert_eq!(app.session_import.pending.len(), 1);
                                     assert!(!app.session_import.notices["Claude Code"].is_empty());
                                     app.store.as_ref().unwrap().upsert_project(&other).unwrap();
                                     app.projects.push(other.clone());
                                     app.assign_imported_projects(cx);
                                     assert!(app.session_import.pending.is_empty());
                                 }
                                 assert_eq!(app.all_sessions.iter().filter(|s| s.id.starts_with("import-")).count(), 700);
                                let histories: Vec<_> = app.all_sessions.iter().filter(|s| s.id.starts_with("import-")).collect();
                                assert!(histories.iter().all(|s| s.workspace.is_some() && app.session_visible(s)));
                                assert_eq!(histories.iter().filter(|s| app.session_scope(s) == Some(crate::session_list::Scope::Project)).count(), 699);
                                assert_eq!(histories.iter().filter(|s| s.project_id == "other-project" && app.session_scope(s).is_none()).count(), 1);
                                 let store = app.store.as_ref().unwrap();
                                 let last_import = app.session_import.last_import["Claude Code"];
                                 assert!(last_import > 0);
                                 assert_eq!(State::load(Some(store)).last_import["Claude Code"], last_import);
                                assert_eq!(store.sessions(None).unwrap().iter().filter(|s| s.id.starts_with("import-")).count(), 700);
                                 if stage == 0 {
                                    let id = imported_id("Claude Code", "fixture-0");
                                    let before = store.load_transcript::<Vec<ChatItem>>(&id).unwrap().unwrap();
                                    let selected = app.session().id.clone();
                                    assert!(app.open_session_id(&id, cx));
                                    assert!(app.assign_imported_location(&id, "other-project", Some(other_path.to_str().unwrap())).is_err());
                                    assert!(app.open_session_id(&selected, cx));
                                    assert!(app.assign_imported_location(&id, "other-project", Some(other_path.to_str().unwrap())).unwrap());
                                    assert!(!app.sessions.iter().any(|s| s.id == id));
                                    let moved = app.all_sessions.iter().find(|s| s.id == id).unwrap();
                                    assert!(app.session_scope(moved).is_none());
                                    assert_eq!(app.store.as_ref().unwrap().sessions(Some("other-project")).unwrap().len(), 2);
                                    assert_eq!(serde_json::to_value(app.store.as_ref().unwrap().load_transcript::<Vec<ChatItem>>(&id).unwrap().unwrap()).unwrap(), serde_json::to_value(before).unwrap());
                                    app.scan_session_import("Claude Code", cx);
                                } else {
                                    let id = imported_id("Claude Code", "fixture-0");
                                    assert!(app.sessions.iter().any(|s| s.id == id && s.project_id == "fixture-project"));
                                    let mut preview = Preview::new("Claude Code".into()).unwrap();
                                    preview.push(Candidate {
                                        source_id: "fixture-0".into(), title: "No directory".into(), directory: None,
                                        model: "Auto".into(), transcript: vec![ChatItem::User { text: "Hello".into(), timing: None, at_ms: 0 }], at_ms: 0,
                                    }, &app.projects).unwrap();
                                     finalize_import(app, Ok(preview), cx);
                                     let row = &app.session_import.preview.as_ref().unwrap().rows[0];
                                    assert!(row.update);
                                    assert_eq!(row.project.as_deref(), Some("fixture-project"));
                                 }
                                 stage += 1;
                             }
                             2 => {
                                 assert!(app.session_import.preview.is_none());
                                 assert!(app.session_import.error.is_none());
                                 assert_eq!(app.all_sessions.iter().filter(|s| s.id.starts_with("import-")).count(), 700);
                                 stage = 3;
                             }
                            _ => {}
                        }
                    });
                    if owner.read(cx).session_import.applying {
                        // Normal Quit is blocked while the background transaction is running.
                        cx.dispatch_action(&crate::app::Quit);
                    }
                     if stage == 3 && staging.as_ref().is_none_or(|path| !path.exists()) {
                        assert!(frames >= 4);
                        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 701);
                         std::fs::write(dir.join("PASS"), "700-session automatic import, late project assignment, per-harness progress, duplicate reimport, temporary cleanup and guarded Quit passed\n").unwrap();
                         println!("PASS: automatic import, late project assignment, per-harness progress, duplicate reimport, cache cleanup, source preservation and Quit");
                        cx.dispatch_action(&crate::app::Quit);
                        return true;
                    }
                    false
                });
                if done { break; }
            }
        }).detach();
    });
}

struct ImportView {
    app: Entity<DunlinApp>,
    _changes: Subscription,
}
impl Render for ImportView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.update(cx, |app, cx| {
            let p = palette(cx);
            div()
                .size_full()
                .p(px(24.))
                .bg(p.bg)
                .text_color(p.fg)
                .font_family(dunlin_ui::theme::ui_font())
                .track_focus(&app.focus_handle)
                .child(crate::views::settings::render(app, window, cx))
        })
    }
}
