//! Independent, disposable behavior scenarios. This inventory drives CLI
//! validation, dispatch and QA discovery; do not maintain a second runner list.
use crate::{
    app::{DunlinApp, StoredSettings},
    gui_test::Step,
    harness::ChatItem,
};
use dunlin_ui::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(crate) struct Scenario {
    pub flag: &'static str,
    kind: Kind,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Save,
    CloseSave,
    CloseDiscard,
    CloseCancel,
    Conflict,
    CloseClean,
    SendClick,
    SendEnter,
    Queue,
    QueueClear,
    Stop,
    Retry,
    Allow,
    Deny,
}
pub(crate) const SCENARIOS: &[Scenario] = &[
    Scenario {
        flag: "--gui-save-reopen-smoke",
        kind: Kind::Save,
    },
    Scenario {
        flag: "--gui-close-save-smoke",
        kind: Kind::CloseSave,
    },
    Scenario {
        flag: "--gui-close-discard-smoke",
        kind: Kind::CloseDiscard,
    },
    Scenario {
        flag: "--gui-close-cancel-smoke",
        kind: Kind::CloseCancel,
    },
    Scenario {
        flag: "--gui-save-conflict-smoke",
        kind: Kind::Conflict,
    },
    Scenario {
        flag: "--gui-close-clean-smoke",
        kind: Kind::CloseClean,
    },
    Scenario {
        flag: "--gui-chat-send-click-smoke",
        kind: Kind::SendClick,
    },
    Scenario {
        flag: "--gui-chat-send-enter-smoke",
        kind: Kind::SendEnter,
    },
    Scenario {
        flag: "--gui-chat-queue-smoke",
        kind: Kind::Queue,
    },
    Scenario {
        flag: "--gui-chat-queue-clear-smoke",
        kind: Kind::QueueClear,
    },
    Scenario {
        flag: "--gui-chat-stop-smoke",
        kind: Kind::Stop,
    },
    Scenario {
        flag: "--gui-chat-retry-smoke",
        kind: Kind::Retry,
    },
    Scenario {
        flag: "--gui-permission-allow-smoke",
        kind: Kind::Allow,
    },
    Scenario {
        flag: "--gui-permission-deny-smoke",
        kind: Kind::Deny,
    },
];
pub(crate) fn find(flag: &str) -> Option<&'static Scenario> {
    SCENARIOS.iter().find(|s| s.flag == flag)
}

fn events(root: &Path, event: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(root.join("requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["event"] == event)
        .collect()
}
fn focus_editor() -> Step {
    Step::act("focus loaded editor", |owner, window, cx| {
        owner
            .read(cx)
            .editor
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    })
}
fn focus_composer() -> Step {
    Step::act("focus composer", |owner, window, cx| {
        owner
            .read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    })
}
fn edited() -> Step {
    Step::until(
        "typed buffer is dirty; disk still original",
        |owner, _, cx| {
            let app = owner.read(cx);
            if !app.editor_dirty {
                return false;
            }
            assert_eq!(app.editor.read(cx).value().as_ref(), "GUI draft");
            assert_eq!(
                fs::read_to_string(Path::new(&app.project().path).join("file.txt")).unwrap(),
                "original"
            );
            true
        },
    )
}
fn loaded() -> Step {
    Step::until("file loaded without mutation", |owner, _, cx| {
        owner
            .read(cx)
            .active_file_tab()
            .is_some_and(|tab| !tab.loading)
    })
}
fn open_file(root: PathBuf) -> Step {
    Step::act("fixture: open existing file", move |owner, window, cx| {
        owner.update(cx, |app, cx| {
            app.open_file(root.join("file.txt").to_str().unwrap(), window, cx)
        });
    })
}
fn file_steps(kind: Kind, root: &Path) -> Vec<Step> {
    let mut steps = vec![open_file(root.into()), loaded()];
    if kind == Kind::CloseClean {
        steps.push(Step::click("close-file-tab"));
    } else {
        steps.extend([
            focus_editor(),
            Step::key("cmd-a"),
            Step::type_text("GUI draft"),
            edited(),
        ]);
        if matches!(
            kind,
            Kind::CloseSave | Kind::CloseDiscard | Kind::CloseCancel
        ) {
            steps.push(Step::click("close-file-tab"));
            steps.push(Step::until(
                "dirty close displays decision dialog",
                |owner, _, cx| owner.read(cx).pending_editor.is_some(),
            ));
            steps.push(Step::click(match kind {
                Kind::CloseSave => "editor-save",
                Kind::CloseDiscard => "editor-discard",
                _ => "editor-cancel",
            }));
        } else {
            if kind == Kind::Conflict {
                let root = root.to_path_buf();
                steps.push(Step::act(
                    "fixture: external writer changes file",
                    move |_, _, _| {
                        dunlin_workspace::fs::write_file(
                            root.to_str().unwrap(),
                            "file.txt",
                            "external change",
                        )
                        .unwrap();
                    },
                ));
            }
            steps.push(Step::key("cmd-shift-s"));
        }
    }
    let root = root.to_path_buf();
    let reopen_root = root.clone();
    steps.push(Step::until(
        "verify buffer, tab and actual disk outcome",
        move |owner, _, cx| {
            let app = owner.read(cx);
            if app.active_file_tab().is_some_and(|tab| tab.saving) {
                return false;
            }
            match kind {
                Kind::Conflict => {
                    if app.editor_conflict.is_none() {
                        return false;
                    }
                    assert!(app.editor_dirty && app.active_file_tab().is_some());
                    assert_eq!(app.editor.read(cx).value().as_ref(), "GUI draft");
                    assert_eq!(
                        fs::read_to_string(root.join("file.txt")).unwrap(),
                        "external change"
                    );
                }
                Kind::CloseCancel => {
                    if app.pending_editor.is_some() {
                        return false;
                    }
                    assert!(app.editor_dirty && app.active_file_tab().is_some());
                    assert_eq!(app.editor.read(cx).value().as_ref(), "GUI draft");
                    assert_eq!(
                        fs::read_to_string(root.join("file.txt")).unwrap(),
                        "original"
                    );
                }
                Kind::Save => {
                    if app.editor_dirty {
                        return false;
                    }
                    assert_eq!(
                        fs::read_to_string(root.join("file.txt")).unwrap(),
                        "GUI draft"
                    );
                }
                _ => {
                    if app.active_file_tab().is_some() {
                        return false;
                    }
                    assert!(app.pending_editor.is_none());
                    assert!(
                        app.workbench.tabs.is_empty(),
                        "close must not leave a hidden file tab"
                    );
                    assert_eq!(
                        fs::read_to_string(root.join("file.txt")).unwrap(),
                        if kind == Kind::CloseSave {
                            "GUI draft"
                        } else {
                            "original"
                        }
                    );
                }
            }
            assert!(!root.join("hello.txt").exists());
            true
        },
    ));
    if matches!(
        kind,
        Kind::Save | Kind::CloseSave | Kind::CloseDiscard | Kind::CloseClean
    ) {
        if kind == Kind::Save {
            steps.push(Step::click("close-file-tab"));
        }
        steps.extend([open_file(reopen_root), loaded()]);
        steps.push(Step::act(
            "reopened editor matches persisted file",
            move |owner, _, cx| {
                let app = owner.read(cx);
                assert_eq!(
                    app.editor.read(cx).value().as_ref(),
                    if matches!(kind, Kind::Save | Kind::CloseSave) {
                        "GUI draft"
                    } else {
                        "original"
                    }
                );
                assert!(!app.editor_dirty);
            },
        ));
    }
    steps
}
fn finished(expected: usize, retained_queued: usize, root: PathBuf) -> Step {
    Step::until(
        "reply complete, exact prompt count and persisted transcript",
        move |owner, window, cx| {
            let app = owner.read(cx);
            if app.harness.running || !app.harness.transcript.iter().any(|item| matches!(item, ChatItem::AgentText { text, .. } if text.contains("SCROLL END"))) { return false; }
            let Some(reply) = crate::gui_probe::bounds("agent-reply") else {
                return false;
            };
            if reply.bottom() <= px(0.) || reply.top() >= window.viewport_size().height {
                return false;
            }
            assert_eq!(events(&root, "prompt").len(), expected);
            assert_eq!(
                app.harness
                    .transcript
                    .iter()
                    .filter(|item| matches!(item, ChatItem::User { .. }))
                    .count(),
                expected + retained_queued
            );
            assert!(app.harness.queued_prompts().is_empty());
            assert!(app.composer.read(cx).value().is_empty());
            let stored: Vec<ChatItem> =
                dunlin_storage::Store::open(root.parent().unwrap().join("dunlin.db"))
                    .unwrap()
                    .load_transcript(&app.session().id)
                    .unwrap()
                    .unwrap();
            assert_eq!(
                stored
                    .iter()
                    .filter(|item| matches!(item, ChatItem::User { .. }))
                    .count(),
                expected + retained_queued
            );
            assert!(
                !root.join("hello.txt").exists(),
                "ordinary chat must not write files"
            );
            true
        },
    )
}
fn chat_steps(kind: Kind, root: &Path) -> Vec<Step> {
    let mut steps = vec![
        Step::until("mock ACP session ready", |owner, _, cx| {
            let app = owner.read(cx);
            app.harness.status.is_ready()
                && !app.harness.session_metadata_pending
                && app.harness.modes.is_some()
                && app.harness.models.is_some()
        }),
        focus_composer(),
        Step::type_text("First GUI prompt"),
        Step::act("composer contains typed prompt", |owner, _, cx| {
            assert_eq!(
                owner.read(cx).composer.read(cx).value().as_ref(),
                "First GUI prompt"
            )
        }),
        if kind == Kind::SendClick {
            Step::click("composer-send")
        } else {
            Step::key("enter")
        },
    ];
    let request_root = root.to_path_buf();
    steps.push(Step::until(
        "backend received exactly one real prompt",
        move |owner, _, cx| {
            let requests = events(&request_root, "prompt");
            if requests.is_empty() {
                return false;
            }
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0]["request"]["prompt"]
                    .to_string()
                    .contains("First GUI prompt")
            );
            assert!(owner.read(cx).composer.read(cx).value().is_empty());
            true
        },
    ));
    if matches!(kind, Kind::Allow | Kind::Deny) {
        let allow = kind == Kind::Allow;
        let gate_root = root.to_path_buf();
        steps.push(Step::until(
            "permission gate visible; no file before decision",
            move |owner, _, cx| {
                if !owner
                    .read(cx)
                    .harness
                    .transcript
                    .iter()
                    .any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }))
                {
                    return false;
                }
                assert!(!gate_root.join("hello.txt").exists());
                true
            },
        ));
        steps.push(Step::click(if allow {
            "permission-allow-once"
        } else {
            "permission-reject-once"
        }));
        let root = root.to_path_buf();
        steps.push(Step::until("permission decision reaches agent and persists", move |owner, _, cx| {
            let app = owner.read(cx);
            if app.harness.running || events(&root, "permission").is_empty() { return false; }
            assert_eq!(events(&root, "permission").len(), 1);
            assert!(app.harness.transcript.iter().any(|item| matches!(item, ChatItem::AgentText { text, .. } if text.contains(if allow { "Permission granted" } else { "Permission denied" }))));
            assert_eq!(root.join("hello.txt").exists(), allow);
            if allow { assert_eq!(fs::read_to_string(root.join("hello.txt")).unwrap(), "GUI permission granted\n"); }
            let stored: Vec<ChatItem> = dunlin_storage::Store::open(root.parent().unwrap().join("dunlin.db")).unwrap().load_transcript(&app.session().id).unwrap().unwrap();
            assert!(stored.iter().any(|item| matches!(item, ChatItem::Permission { resolved: Some(label), .. } if label.starts_with(if allow { "Allowed" } else { "Denied" }))));
            true
        }));
        return steps;
    }
    if kind == Kind::Stop {
        steps.push(Step::click("composer-send"));
        let root = root.to_path_buf();
        steps.push(Step::until(
            "Stop cancels transport, retains user message, writes nothing",
            move |owner, _, cx| {
                let app = owner.read(cx);
                if events(&root, "cancel").is_empty() || events(&root, "finished").is_empty() {
                    return false;
                }
                assert!(!app.harness.running);
                assert!(app.harness.queued_prompts().is_empty());
                assert_eq!(events(&root, "prompt").len(), 1);
                assert_eq!(app.harness.transcript.iter().filter(|item| matches!(item, ChatItem::User { text, .. } if text == "First GUI prompt")).count(), 1);
                let stored: Vec<ChatItem> = dunlin_storage::Store::open(root.parent().unwrap().join("dunlin.db")).unwrap().load_transcript(&app.session().id).unwrap().unwrap();
                assert_eq!(stored.iter().filter(|item| matches!(item, ChatItem::User { .. })).count(), 1);
                assert!(!stored.iter().any(|item| matches!(item, ChatItem::AgentText { .. })));
                assert!(
                    !app.harness
                        .transcript
                        .iter()
                        .any(|item| matches!(item, ChatItem::AgentText { .. }))
                );
                assert!(!root.join("hello.txt").exists());
                true
            },
        ));
        return steps;
    }
    if matches!(kind, Kind::Queue | Kind::QueueClear) {
        steps.extend([
            focus_composer(),
            Step::type_text("Second GUI prompt"),
            Step::key("enter"),
        ]);
        let queued_root = root.to_path_buf();
        steps.push(Step::until(
            "queued prompt stays local until first turn completes",
            move |owner, _, cx| {
                let app = owner.read(cx);
                if app.harness.queued_prompts().len() != 1 {
                    return false;
                }
                assert_eq!(app.harness.queued_prompts()[0].text, "Second GUI prompt");
                assert_eq!(events(&queued_root, "prompt").len(), 1);
                assert!(app.harness.running);
                true
            },
        ));
        if kind == Kind::QueueClear {
            steps.extend([Step::click("composer-queue"), Step::click("queue-clear")]);
            steps.push(Step::act(
                "cleared queue has no remaining prompts",
                |owner, _, cx| assert!(owner.read(cx).harness.queued_prompts().is_empty()),
            ));
        }
        let root = root.to_path_buf();
        steps.push(Step::act(
            "fixture: release first ACP response",
            move |_, _, _| fs::write(root.join("release"), "go").unwrap(),
        ));
    }
    steps.push(finished(
        if kind == Kind::Queue { 2 } else { 1 },
        usize::from(kind == Kind::QueueClear),
        root.into(),
    ));
    if kind == Kind::Retry {
        steps.push(Step::click("user-retry-0"));
        let retry_root = root.to_path_buf();
        steps.push(Step::until(
            "retry sends same prompt exactly once more",
            move |_, _, _| {
                let requests = events(&retry_root, "prompt");
                if requests.len() < 2 {
                    return false;
                }
                assert_eq!(requests.len(), 2);
                assert_eq!(
                    requests[0]["request"]["prompt"],
                    requests[1]["request"]["prompt"]
                );
                true
            },
        ));
        steps.push(finished(2, 0, root.into()));
    }
    steps
}

pub(crate) fn run(scenario: &'static Scenario) {
    let directory = PathBuf::from(std::env::args_os().nth(2).unwrap());
    assert!(
        directory.is_absolute()
            && directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-gui-smoke-")
    );
    fs::create_dir(&directory).expect("GUI fixture must be fresh");
    let root = directory.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.txt"), "original").unwrap();
    let chat = matches!(
        scenario.kind,
        Kind::SendClick
            | Kind::SendEnter
            | Kind::Queue
            | Kind::QueueClear
            | Kind::Stop
            | Kind::Retry
            | Kind::Allow
            | Kind::Deny
    );
    let mock = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("mock_acp_agent");
    if chat {
        assert!(mock.is_file(), "build mock_acp_agent alongside dunlin");
    }
    let store = dunlin_storage::Store::open(directory.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                default_harness: "Custom".into(),
                custom_harness_command: mock.to_string_lossy().into(),
                auto_connect: false,
                restore_session: false,
                notifications_enabled: false,
                vault_enabled: false,
                security_enabled: false,
                autosave: false,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "gui-fixture".into(),
            name: "GUI fixture".into(),
            path: root.to_string_lossy().into(),
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
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &directory);
        std::env::set_var("DUNLIN_LOG_FILE", directory.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        if chat {
            std::env::set_var("DUNLIN_HARNESS_COMMAND", &mock);
            std::env::set_var("DUNLIN_MOCK_GUI_ROOT", &root);
            if !matches!(scenario.kind, Kind::Allow | Kind::Deny) {
                std::env::set_var("DUNLIN_MOCK_STREAM_CHUNKS", "1");
            }
        }
    }
    if !matches!(scenario.kind, Kind::Queue | Kind::QueueClear | Kind::Stop) {
        fs::write(root.join("release"), "go").unwrap();
    }
    crate::gui_probe::enable();
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.gui-scenarios", "Dunlin GUI Scenarios");
        dunlin_ui::init(cx);
        crate::native_menu::install(cx);
        crate::logging::init();
        let (window, owner) = open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(80.), px(50.)),
                    size(px(1200.), px(800.)),
                ))),
                ..dunlin_ui::window_options()
            },
            cx,
            |window, cx| cx.new(|cx| DunlinApp::new(window, cx)),
        )
        .unwrap();
        cx.activate(true);
        if chat {
            owner.update(cx, |app, _| {
                let mut neighbor = app.session().clone();
                neighbor.id = "gui-untouched".into();
                neighbor.title = "Untouched conversation".into();
                app.store
                    .as_ref()
                    .unwrap()
                    .upsert_session(&neighbor)
                    .unwrap();
            });
            owner.update(cx, |app, cx| app.connect_harness(cx));
        }
        let steps = if chat {
            chat_steps(scenario.kind, &root)
        } else {
            file_steps(scenario.kind, &root)
        };
        let mut steps = steps;
        if chat {
            let database = directory.join("dunlin.db");
            steps.push(Step::act(
                "unrelated conversation and original project file remain unchanged",
                move |_, _, _| {
                    let store = dunlin_storage::Store::open(&database).unwrap();
                    assert!(
                        store
                            .load_transcript::<Vec<ChatItem>>("gui-untouched")
                            .unwrap()
                            .is_none()
                    );
                    assert_eq!(
                        store
                            .sessions(None)
                            .unwrap()
                            .iter()
                            .find(|s| s.id == "gui-untouched")
                            .unwrap()
                            .title,
                        "Untouched conversation"
                    );
                    assert_eq!(
                        fs::read_to_string(database.parent().unwrap().join("project/file.txt"))
                            .unwrap(),
                        "original"
                    );
                },
            ));
        }
        crate::gui_test::run(scenario.flag, directory.clone(), steps, window, owner, cx);
    });
}
