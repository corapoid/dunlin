use super::*;
use dunlin_session_runtime::TurnPhase;

#[tokio::test]
#[ignore = "requires production web build, Node, Python, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_controls() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store
        .create_user("controls-browser-owner", "admin")
        .unwrap();
    let workspace = store.create_workspace(&owner, "Controls browser").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Actor project")
        .unwrap();
    let native = store
        .create_project("Controlled checkout", checkout.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &owner.id).unwrap();
    store
        .set_local_binding(&owner.id, &workspace.id, &project.id, &native.id, None)
        .unwrap();
    let code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let mut state = AppState::new(
        store,
        Config {
            web_root: Some(root.into()),
            db: dir.path().join("relay.db"),
            sandbox: false,
            experimental_interactive_runtime: true,
            store_transcripts: true,
            ..Config::default()
        },
    );
    std::sync::Arc::get_mut(&mut state.interactive)
        .unwrap()
        .command = Some((
        "python3".into(),
        vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/acp/control_peer.py"
            )
            .into(),
            "cooperative".into(),
            "--gate-cancel".into(),
        ],
    ));
    let (origin, _server) = super::live_transport::start(&state).await;
    let active = "hold AHP_ACTIVE_CANCEL_MARKER";
    let removed = "AHP_REMOVED_NEVER_EXECUTED";
    let original = "AHP_QUEUED_BEFORE_EDIT";
    let edited = "AHP_EDITED_EXECUTED_AFTER_CANCEL";
    let input = json!({"origin":origin,"ownerCode":code,"workspace":workspace.id,"project":project.id,"active":active,"removed":removed,"original":original,"edited":edited});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-controls.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Node is required");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        stdin.write_all(format!("{input}\n").as_bytes()).await.unwrap();
        let mut log = String::new();
        while let Some(line) = stdout.next_line().await.unwrap() {
            if !line.starts_with('{') { log.push_str(&line); log.push('\n'); continue; }
            let request: Value = serde_json::from_str(&line).unwrap();
            let command = request["command"].as_str().unwrap();
            match command {
                "cancel-barrier" => {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                    while !checkout.join("received-cancels").is_file() {
                        assert!(tokio::time::Instant::now() < deadline, "ACP session/cancel deadline");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert_eq!(std::fs::read_to_string(checkout.join("received-cancels")).unwrap(), "fixture-controls\n");
                    assert_eq!(std::fs::read_to_string(checkout.join("received-prompts")).unwrap(), format!("{active}\n"));
                    // Receipt of UI cancellation cannot release the next ACP request.
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    assert_eq!(std::fs::read_to_string(checkout.join("control-events")).unwrap(), format!("prompt:{active}\ncancel\n"));
                    std::fs::write(checkout.join("release-cancel"), "release").unwrap();
                }
                "verify" => {
                    let session = request["session"].as_str().unwrap();
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                    loop {
                        let done = {
                            let store = state.store.lock().unwrap();
                            let authority = store.team_project_context(&workspace.id, &project.id, &owner.id).unwrap();
                            let turns = store.interactive_journal().snapshot(&authority, session).unwrap().1;
                            turns.iter().any(|t| t.prompt == edited && t.phase == TurnPhase::Completed)
                        };
                        if done { break; }
                        assert!(tokio::time::Instant::now() < deadline, "Remaining queue completion deadline");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert_eq!(std::fs::read_to_string(checkout.join("received-prompts")).unwrap(), format!("{active}\n{edited}\n"));
                    assert_eq!(std::fs::read_to_string(checkout.join("control-events")).unwrap(), format!("prompt:{active}\ncancel\ncancelled-result\nprompt:{edited}\ncompleted\n"));
                    let store = state.store.lock().unwrap();
                    let authority = store.team_project_context(&workspace.id, &project.id, &owner.id).unwrap();
                    let turns = store.interactive_journal().snapshot(&authority, session).unwrap().1;
                    assert_eq!(turns.len(), 3);
                    for (text, phase) in [(active, TurnPhase::Cancelled), (removed, TurnPhase::Cancelled), (edited, TurnPhase::Completed)] {
                        assert_eq!(turns.iter().find(|t| t.prompt == text).unwrap().phase, phase);
                    }
                    assert!(!turns.iter().any(|t| t.prompt == original));
                }
                _ => panic!("Unexpected browser control command {command}"),
            }
            stdin.write_all(format!("{}\n", json!({"ok":command})).as_bytes()).await.unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success(), "AHP controls browser failed: {}\n{log}", String::from_utf8_lossy(&output.stderr));
        assert!(log.contains("PASS Chromium AHP controls"));
        println!("{log}");
    }).await;
    state.interactive.shutdown().await;
    result.expect("AHP controls browser deadline");
}
