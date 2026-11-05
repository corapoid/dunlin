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
    let source = dir.join("claude/projects/fixture");
    std::fs::create_dir_all(&source).unwrap();
    for index in 0..700 {
        let content = serde_json::json!({
            "type":"user", "sessionId": format!("fixture-{index}"), "cwd":project_path,
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
                            return;
                        }
                        match stage {
                            0 | 2 => {
                                let preview = app.session_import.preview.as_ref().unwrap();
                                assert_eq!(preview.rows.len(), 700);
                                assert_eq!(preview.warnings.len(), 1);
                                assert!(preview.rows.iter().all(|row| row.candidate.transcript.is_empty() && row.project.as_deref() == Some("fixture-project")));
                                assert_eq!(preview.rows.iter().filter(|row| row.update).count(), if stage == 0 { 0 } else { 700 });
                                staging = Some(preview.staging.path().to_path_buf());
                                app.apply_session_import(cx);
                                assert!(app.session_import.applying);
                                assert!(app.session_import.progress.as_ref().unwrap().lock().unwrap().label().contains("Importing sessions"));
                                stage += 1;
                            }
                            1 | 3 => {
                                assert!(!app.session_import.applying);
                                assert!(app.session_import.preview.is_none());
                                assert!(app.session_import.error.is_none());
                                assert_eq!(app.all_sessions.iter().filter(|s| s.id.starts_with("import-")).count(), 700);
                                let store = app.store.as_ref().unwrap();
                                assert_eq!(store.sessions(None).unwrap().iter().filter(|s| s.id.starts_with("import-")).count(), 700);
                                if stage == 1 {
                                    app.scan_session_import("Claude Code", cx);
                                }
                                stage += 1;
                            }
                            _ => {}
                        }
                    });
                    if owner.read(cx).session_import.applying {
                        // Normal Quit is blocked while the background transaction is running.
                        cx.dispatch_action(&crate::app::Quit);
                    }
                    if stage == 4 && !staging.as_ref().unwrap().exists() {
                        assert!(frames >= 4);
                        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 701);
                        std::fs::write(dir.join("PASS"), "700-session native preview/progress, background atomic import, duplicate reimport, temporary cleanup and guarded Quit passed\n").unwrap();
                        println!("PASS: 700-session native import UI, progress and background writes, duplicate reimport, cache cleanup, source preservation and Quit");
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.update(cx, |app, cx| {
            let p = palette(cx);
            div()
                .size_full()
                .p(px(24.))
                .bg(p.bg)
                .text_color(p.fg)
                .font_family(dunlin_ui::theme::ui_font())
                .track_focus(&app.focus_handle)
                .child(super::render(app, cx))
        })
    }
}
