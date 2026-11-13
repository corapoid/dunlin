use super::*;
use dunlin_session_runtime::{HarnessInfo, TurnPhase};

/// Controlled source journal; the browser still uses the production HTTP/WS router.
#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store.create_user("ahp-browser-owner", "admin").unwrap();
    let hidden_user = store.create_user("ahp-browser-hidden", "admin").unwrap();
    let initial = "AHP_INITIAL_PRIVATE_MARKER";
    let streamed = "AHP_STREAMED_PRIVATE_MARKER";
    let offline = "AHP_OFFLINE_REPLAY_PRIVATE_MARKER";
    let hidden = "AHP_OTHER_USER_HIDDEN_MARKER";
    let mut source = None;
    for (user, marker) in [(&owner, initial), (&hidden_user, hidden)] {
        let workspace = store.create_workspace(user, "Browser AHP").unwrap();
        let project = store
            .create_team_project(&user.id, &workspace.id, "Journal fixture")
            .unwrap();
        let authority = store
            .team_project_context(&workspace.id, &project.id, &user.id)
            .unwrap();
        let journal = store.interactive_journal();
        let session = journal
            .create(&authority, &workspace.id, &project.id)
            .unwrap();
        let lease = journal
            .acquire(&authority, &session.id, 0, "browser-fixture")
            .unwrap();
        journal
            .harness_ready(
                &authority,
                &session.id,
                &lease,
                HarnessInfo {
                    profile: None,
                    remote_session_id: "controlled-journal-only".into(),
                    binding_revision: 1,
                    model: "fixture".into(),
                    mode: "fixture".into(),
                    max_seconds: 120,
                    max_output_bytes: 32768,
                },
            )
            .unwrap();
        let turn = journal
            .enqueue(
                &authority,
                &session.id,
                "browser-fixture-turn",
                "Controlled browser prompt",
            )
            .unwrap();
        journal.claim_next(&authority, &session.id, &lease).unwrap();
        journal
            .transition(&authority, &session.id, &lease, &turn, TurnPhase::Running)
            .unwrap();
        journal
            .output(&authority, &session.id, &lease, &turn, marker)
            .unwrap();
        if user.id == owner.id {
            source = Some((workspace.id, project.id, session.id, lease, turn));
        }
    }
    let (workspace, project, session, lease, turn) = source.unwrap();
    let code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            web_root: Some(root.into()),
            db: dir.path().join("relay.db"),
            experimental_interactive_runtime: true,
            store_transcripts: true,
            sandbox: false,
            ..Config::default()
        },
    );
    let (origin, _server) = super::live_transport::start(&state).await;
    let input = json!({"origin":origin,"ownerCode":code,"sessionUri":format!("ahp-session:/{session}"),"initial":initial,"streamed":streamed,"offline":offline,"hidden":hidden});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/e2e/ahp.mjs"))
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
            {
                let store = state.store.lock().unwrap();
                let authority = store
                    .team_project_context(&workspace, &project, &owner.id)
                    .unwrap();
                let journal = store.interactive_journal();
                match command {
                    "append" | "offline" => journal
                        .output(
                            &authority,
                            &session,
                            &lease,
                            &turn,
                            if command == "append" {
                                streamed
                            } else {
                                offline
                            },
                        )
                        .unwrap(),
                    "complete" => journal
                        .transition_timed(
                            &authority,
                            &session,
                            &lease,
                            &turn,
                            TurnPhase::Completed,
                            Some(42),
                        )
                        .unwrap(),
                    "revoke" => {
                        let device = request["device"].as_str().unwrap();
                        assert_eq!(
                            store.active_device_user(device).unwrap().unwrap().id,
                            owner.id
                        );
                        store.revoke_device(device).unwrap();
                    }
                    _ => panic!("Unexpected browser fixture command {command}"),
                }
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
            "Browser AHP acceptance failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium AHP observer"));
        println!("{log}");
    })
    .await;
    result.expect("browser AHP acceptance deadline");
}
