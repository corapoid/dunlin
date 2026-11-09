//! Real AppKit/GPUI windows and production controls, with a deployed relay.
use super::Manifest;
use crate::{
    app::{DunlinApp, StoredSettings},
    gui_test::Step,
    harness::ChatItem,
};
use dunlin_ui::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::OnceLock};

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn capture_failure(window: &Window) {
    let Some(directory) = DIRECTORY.get() else {
        return;
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // GPUI owns this NSView on the AppKit main thread. Capture only our own
    // fixture window, never the user's desktop or another application.
    let view: &objc2_app_kit::NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
    let Some(native) = view.window() else {
        return;
    };
    let number = native.windowNumber();
    let result = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-x", "-l", &number.to_string()])
        .arg(directory.join("failure.png"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = result {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = fs::write(
        directory.join("screenshot-unavailable.txt"),
        "Window capture unavailable; no TCC approval was automated.",
    );
}

pub(crate) fn journal_keychain(id: &str) -> Result<(), String> {
    if let Some(directory) = DIRECTORY.get() {
        use std::io::Write;
        let path = directory.join("keychain-ids");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "{id}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn typing(text: String) -> Step {
    Step::act("type into measured remote control", move |_, window, cx| {
        for ch in text.chars() {
            assert!(window.dispatch_keystroke(
                Keystroke {
                    key: ch.to_string(),
                    key_char: Some(ch.to_string()),
                    ..Default::default()
                },
                cx
            ));
        }
    })
}

pub(super) fn run(spec: Manifest) -> Result<Value, String> {
    if spec.payload["keychain_consent"] != true {
        return Err(
            "Native pairing writes a fixture token to Keychain; explicit consent is required"
                .into(),
        );
    }
    if !spec.fixture.is_absolute()
        || !spec
            .fixture
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("remote-ui-"))
    {
        return Err("Expected a private absolute remote-ui-* fixture".into());
    }
    let restart = spec.payload["restart"] == true;
    let previous = if restart {
        Some(
            serde_json::from_slice::<Value>(
                &fs::read(spec.fixture.join("first-launch.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    if !restart {
        fs::create_dir(&spec.fixture).map_err(|e| e.to_string())?;
        fs::create_dir(spec.fixture.join("local-project")).map_err(|e| e.to_string())?;
        fs::write(spec.fixture.join("local-project/file.txt"), "local-canary")
            .map_err(|e| e.to_string())?;
        let store = dunlin_storage::Store::open(spec.fixture.join("dunlin.db"))
            .map_err(|e| e.to_string())?;
        store
            .save_setting(
                "app",
                &StoredSettings {
                    auto_connect: false,
                    restore_session: true,
                    notifications_enabled: false,
                    vault_enabled: false,
                    security_enabled: false,
                    autosave: false,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        store
            .upsert_project(&dunlin_domain::Project {
                id: "manual-local-canary".into(),
                name: "Local canary".into(),
                path: spec
                    .fixture
                    .join("local-project")
                    .to_string_lossy()
                    .into_owned(),
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
            .map_err(|e| e.to_string())?;
    }
    DIRECTORY
        .set(spec.fixture.clone())
        .map_err(|_| "UI fixture already initialized")?;
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &spec.fixture);
        std::env::set_var("DUNLIN_LOG_FILE", spec.fixture.join("dunlin.log"));
    }
    crate::gui_probe::enable();
    application().with_assets(crate::assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.remote-e2e", "Dunlin Remote E2E");
        dunlin_ui::init(cx);
        crate::native_menu::install(cx);
        crate::logging::init();
        let (window, owner) = open_window(WindowOptions {
            window_bounds:Some(WindowBounds::Windowed(Bounds::new(point(px(80.),px(50.)),size(px(1200.),px(800.))))),
            ..dunlin_ui::window_options()
        }, cx, |window,cx| cx.new(|cx|DunlinApp::new(window,cx))).unwrap();
        cx.activate(true);
        let mut steps = Vec::new();
        if !restart {
            steps.push(Step::act("open production connection sheet", |owner, window, cx|
                owner.update(cx,|app,cx|app.open_remote_connection(window,cx))));
            steps.push(Step::click("remote-e2e-url"));
            steps.push(typing(spec.url.clone()));
            steps.push(Step::click("remote-e2e-code"));
            steps.push(typing(spec.code.clone()));
            steps.push(Step::click("remote-e2e-connect"));
            steps.push(Step::until("pairing completes and authenticated server workspace loads", |owner,_,cx| {
                let app=owner.read(cx);
                assert!(app.remote_connection.error.is_none(), "remote pairing failed: {:?}", app.remote_connection.error);
                if app.remote_loading || app.remote_hosts.is_empty() || app.server_panel.loading { return false; }
                assert!(app.server_panel.error.is_none(), "server workspace failed");
                assert!(app.server_panel.info.is_some(), "authenticated server info missing");
                true
            }));
            steps.push(Step::act("open production remote project picker", |owner,_,cx| {
                owner.update(cx,|app,cx| { app.settings_open=false; app.open_remote_picker(cx); });
            }));
            steps.push(Step::click(format!("remote-project:{}", spec.project_id)));
        }
        let project_id=spec.project_id.clone();
        steps.push(Step::until("remote project selected, never local fallback", move |owner,_,cx| {
            let app=owner.read(cx);
            app.project().remote.as_ref().is_some_and(|p|p.project_id==project_id)
        }));
        let path=format!("{}/file.txt",spec.cwd);
        steps.push(Step::act("open remote fixture file",move |owner,window,cx|
            owner.update(cx,|app,cx|app.open_file(&path,window,cx))));
        steps.push(Step::until("remote editor loaded",|owner,_,cx|
            owner.read(cx).active_file_tab().is_some_and(|tab|!tab.loading)));
        if !restart {
            steps.push(Step::act("focus remote editor",|owner,window,cx|
                owner.read(cx).editor.read(cx).focus_handle(cx).focus(window,cx)));
            steps.push(Step::key("cmd-a"));
            steps.push(typing("Remote UI persisted".into()));
            steps.push(Step::key("cmd-s"));
        }
        let check=spec.clone();
        steps.push(Step::until("saved remote contents and local canary unchanged",move |owner,_,cx| {
            let app=owner.read(cx);
            if app.editor_dirty || app.editor.read(cx).value().as_ref()!="Remote UI persisted" { return false; }
            let file=super::workspace(&check).read_file("file.txt").unwrap();
            assert_eq!(file.contents,"Remote UI persisted");
            assert_eq!(fs::read_to_string(check.fixture.join("local-project/file.txt")).unwrap(),"local-canary");
            true
        }));
        if let Some(previous) = previous.clone() {
            steps.push(Step::until("same remote conversation selected after fresh app launch",move |owner,_,cx| {
                let app=owner.read(cx);
                if app.remote_sessions_loading { return false; }
                assert_eq!(app.session().acp_session_id.as_deref(), previous["session_id"].as_str());
                assert_eq!(app.project().remote.as_ref().unwrap().host_id, previous["host_id"].as_str().unwrap());
                true
            }));
        } else {
            steps.push(Step::until("remote project metadata loaded before new conversation",|owner,_,cx|!owner.read(cx).remote_sessions_loading));
            steps.push(Step::act("create a fresh remote conversation",|owner,_,cx|owner.update(cx,|app,cx|app.new_session(cx))));
        }
        steps.push(Step::act("connect production remote harness",|owner,_,cx|owner.update(cx,|app,cx|app.connect_harness(cx))));
        let expected_model = spec.model.clone();
        steps.push(Step::until("real remote ACP ready with anonymous free model",move |owner,_,cx| {
            let app=owner.read(cx);
            assert!(!matches!(app.harness.status, crate::harness::HarnessStatus::Failed(_)), "remote ACP failed: {:?}", app.harness.status);
            if !app.harness.status.is_ready() || app.harness.local_session_loading || app.harness.session_metadata_pending || app.harness.session_id.is_none() || app.harness.models.is_none() { return false; }
            assert_eq!(app.harness.models.as_ref().map(|m|m.current.as_str()),Some(expected_model.as_str()));
            true
        }));
        if let Some(previous) = previous.clone() {
            let earlier_reply=previous["expected_reply"].as_str().unwrap().to_owned();
            steps.push(Step::until("earlier remote reply restored",move |owner,_,cx| {
                owner.read(cx).harness.transcript.iter().any(|item|
                    matches!(item,ChatItem::AgentText{text,streaming:false,..} if text.contains(&earlier_reply)))
            }));
        }
        // A retry can see an earlier server conversation. Unique markers must
        // not let a cached reply satisfy this run's new prompt assertion.
        let nonce=spec.fixture.file_name().unwrap().to_string_lossy().rsplit('-').next().unwrap().to_owned();
        let expected=format!("DUNLIN_REMOTE_UI_{}_{nonce}_OK",if restart {"RESUMED"} else {"FIRST"});
        steps.push(Step::act("open remote conversation tab",|owner,window,cx|owner.update(cx,|app,cx|app.show_chat_tab(window,cx))));
        steps.push(Step::act("focus composer",|owner,window,cx|owner.read(cx).composer.read(cx).focus_handle(cx).focus(window,cx)));
        steps.push(typing(format!("Reply with exactly {expected}. Do not use tools.")));
        steps.push(Step::click("composer-send"));
        let expected_reply=expected.clone();
        steps.push(Step::until("real anonymous OpenCode reply displayed",move |owner,_,cx| {
            let app=owner.read(cx);
            assert!(!app.harness.transcript.iter().any(|item|matches!(item,ChatItem::Error{..})),"remote GUI turn reported an error");
            !app.harness.running && app.harness.transcript.iter().any(|item|
                matches!(item,ChatItem::AgentText{text,..} if text.contains(&expected_reply)))
        }));
        let evidence_spec = spec.clone();
        steps.push(Step::act("record real GUI session and Keychain persistence evidence",move |owner,_,cx| {
            let app=owner.read(cx);
            let host_id=&app.project().remote.as_ref().unwrap().host_id;
            let token=crate::remote::load_token(host_id).unwrap().expect("paired token must persist in Keychain");
            assert!(!token.is_empty());
            assert!(std::env::var_os("DUNLIN_REMOTE_TOKEN").is_none());
            let (agent,version)=match &app.harness.status {
                crate::harness::HarnessStatus::Ready{agent,version}=>(agent,version),
                other=>panic!("remote agent not ready: {other:?}"),
            };
            assert!(agent.to_lowercase().contains("opencode"));
            let session_id=app.harness.session_id.as_deref().unwrap();
            assert_eq!(app.session().acp_session_id.as_deref(),Some(session_id));
            if let Some(previous)=&previous { assert_eq!(Some(session_id),previous["session_id"].as_str()); }
            let evidence=json!({"phase":if restart {"restart"} else {"first-launch"},"session_id":session_id,
                "host_id":host_id,"project_id":evidence_spec.project_id,"agent":agent,"version":version,
                "model":app.harness.models.as_ref().unwrap().current,"expected_reply":expected,"keychain_read":true,"pid":std::process::id()});
            fs::write(evidence_spec.fixture.join(if restart {"restart.json"} else {"first-launch.json"}),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }));
        crate::gui_test::run_with_timeout("manual-remote-e2e",spec.fixture.clone(),steps,window,owner,cx,240);
    });
    Ok(json!({"ui_completed":true}))
}
