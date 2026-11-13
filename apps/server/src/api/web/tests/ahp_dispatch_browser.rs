use super::*;
use dunlin_session_runtime::TurnPhase;

/// A real browser dispatches into the server-owned ACP actor. Only the ACP peer is local.
#[tokio::test]
#[ignore = "requires production web build, Node, Python, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_dispatch() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    // Extend the existing ACP fixture in this test's temporary directory, preserving
    // the actual initialize/configuration/prompt protocol and process boundaries.
    let harness = include_str!("../../../interactive/tests/fake_acp.py");
    let gate = "        reply(request, {\"stopReason\":\"end_turn\"})";
    assert_eq!(harness.matches(gate).count(), 1);
    let harness = harness.replace(gate, r#"        while not os.path.exists("release-completion"):
            time.sleep(0.02)
        send({"jsonrpc":"2.0", "method":"session/update", "params":{"sessionId":"fixture-interactive", "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":" AHP_ACTOR_COMPLETED_OFFLINE"}}}})
        reply(request, {"stopReason":"end_turn"})"#);
    let harness_path = dir.path().join("browser_acp.py");
    std::fs::write(&harness_path, harness).unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store
        .create_user("dispatch-browser-owner", "admin")
        .unwrap();
    let workspace = store.create_workspace(&owner, "Dispatch browser").unwrap();
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
        vec![harness_path.to_str().unwrap().into(), "normal".into()],
    ));
    let (origin, _server) = super::live_transport::start(&state).await;
    let prompt = "AHP_BROWSER_ACTUAL_DISPATCH";
    let input = json!({"origin":origin,"ownerCode":code,"workspace":workspace.id,"project":project.id,"prompt":prompt,"completion":"AHP_ACTOR_COMPLETED_OFFLINE"});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-dispatch.mjs"
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
        stdin
            .write_all(format!("{input}\n").as_bytes())
            .await
            .unwrap();
        let mut log = String::new();
        while let Some(line) = stdout.next_line().await.unwrap() {
            if !line.starts_with('{') {
                log.push_str(&line);
                log.push('\n');
                continue;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            let command = request["command"].as_str().unwrap();
            match command {
                "release" => {
                    std::fs::write(checkout.join("release-completion"), "release").unwrap();
                    let session = request["session"].as_str().unwrap();
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                    loop {
                        let completed = {
                            let store = state.store.lock().unwrap();
                            let authority = store
                                .team_project_context(&workspace.id, &project.id, &owner.id)
                                .unwrap();
                            let turns = store
                                .interactive_journal()
                                .snapshot(&authority, session)
                                .unwrap()
                                .1;
                            turns.len() == 1 && turns[0].phase == TurnPhase::Completed
                        };
                        if completed {
                            break;
                        }
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "ACP completion deadline"
                        );
                        tokio::time::sleep(Duration::from_millis(30)).await;
                    }
                }
                "verify" => {
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("received-prompts")).unwrap(),
                        format!("{prompt}\n"),
                        "Reconnect must never redispatch a prompt"
                    );
                    let store = state.store.lock().unwrap();
                    let authority = store
                        .team_project_context(&workspace.id, &project.id, &owner.id)
                        .unwrap();
                    let journal = store.interactive_journal();
                    let session = request["session"].as_str().unwrap();
                    let turns = journal.snapshot(&authority, session).unwrap().1;
                    assert_eq!(turns.len(), 1);
                    assert_eq!(turns[0].phase, TurnPhase::Completed);
                    let events = journal.replay(&authority, session, 0).unwrap();
                    assert!(events.iter().any(|event| event.kind == "harness_ready"));
                    assert!(events.iter().any(|event| event.kind == "agent_update"));
                }
                "revoke" => {
                    let store = state.store.lock().unwrap();
                    let device = request["device"].as_str().unwrap();
                    assert_eq!(
                        store.active_device_user(device).unwrap().unwrap().id,
                        owner.id
                    );
                    store.revoke_device(device).unwrap();
                }
                _ => panic!("Unexpected browser control command {command}"),
            }
            stdin
                .write_all(format!("{}\n", json!({"ok":command})).as_bytes())
                .await
                .unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().await.unwrap();
        assert!(
            output.status.success(),
            "AHP dispatch browser failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium AHP dispatch"));
        println!("{log}");
    })
    .await;
    state.interactive.shutdown().await;
    result.expect("AHP dispatch browser deadline");
}
