use super::*;
use dunlin_session_runtime::{HarnessInfo, TurnPhase};

/// Production React/router/AHP with a bounded real journal larger than one frame.
#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_history() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store.create_user("history-browser-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "History browser").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Paged journal")
        .unwrap();
    let authority = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let journal = store.interactive_journal();
    let session = journal
        .create(&authority, &workspace.id, &project.id)
        .unwrap();
    let lease = journal
        .acquire(&authority, &session.id, 0, "history-browser-fixture")
        .unwrap();
    journal
        .harness_ready(
            &authority,
            &session.id,
            &lease,
            HarnessInfo {
                profile: None,
                remote_session_id: "controlled-history-only".into(),
                binding_revision: 1,
                model: "fixture".into(),
                mode: "fixture".into(),
                max_seconds: 120,
                max_output_bytes: 4 * 1024 * 1024,
            },
        )
        .unwrap();
    let mut turn_ids = Vec::new();
    let mut markers = Vec::new();
    for index in 0..20 {
        let marker = format!("HISTORY_TURN_{index:02}");
        let turn = journal
            .enqueue(
                &authority,
                &session.id,
                &format!("history-{index}"),
                &format!("History prompt {index}"),
            )
            .unwrap();
        journal.claim_next(&authority, &session.id, &lease).unwrap();
        journal
            .transition(&authority, &session.id, &lease, &turn, TurnPhase::Running)
            .unwrap();
        for chunk in 0..4 {
            let text = format!("{marker}_CHUNK_{chunk}\n{}\n", "x".repeat(25 * 1024));
            journal
                .output(&authority, &session.id, &lease, &turn, &text)
                .unwrap();
        }
        journal
            .transition_timed(
                &authority,
                &session.id,
                &lease,
                &turn,
                TurnPhase::Completed,
                Some(10),
            )
            .unwrap();
        turn_ids.push(turn);
        markers.push(marker);
    }
    let full_bytes = serde_json::to_vec(&journal.projected_chat(&authority, &session.id).unwrap())
        .unwrap()
        .len();
    assert!(full_bytes > 900 * 1024);
    drop(journal);
    let code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            web_root: Some(root.into()),
            db: dir.path().join("relay.db"),
            experimental_interactive_runtime: true,
            store_transcripts: true,
            sandbox: false,
            ..Default::default()
        },
    );
    let (origin, _server) = super::live_transport::start(&state).await;
    let live = "HISTORY_LIVE_WHILE_PAGING";
    let offline = "HISTORY_COMPLETED_OFFLINE";
    let input = json!({"origin":origin,"ownerCode":code,"sessionUri":format!("ahp-session:/{}",session.id),"chatUri":format!("ahp-chat:/{}",session.id),"turnIds":turn_ids,"markers":markers,"live":live,"offline":offline,"fullBytes":full_bytes});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-history.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Node is required");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(Duration::from_secs(90), async {
        stdin
            .write_all(format!("{input}\n").as_bytes())
            .await
            .unwrap();
        let mut log = String::new();
        let mut live_turn = None;
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
                    .team_project_context(&workspace.id, &project.id, &owner.id)
                    .unwrap();
                let journal = store.interactive_journal();
                match command {
                    "live" | "offline" => {
                        let marker = if command == "live" { live } else { offline };
                        let turn = journal
                            .enqueue(&authority, &session.id, command, marker)
                            .unwrap();
                        journal.claim_next(&authority, &session.id, &lease).unwrap();
                        journal
                            .transition(&authority, &session.id, &lease, &turn, TurnPhase::Running)
                            .unwrap();
                        journal
                            .output(&authority, &session.id, &lease, &turn, marker)
                            .unwrap();
                        if command == "live" {
                            live_turn = Some(turn);
                        } else {
                            journal
                                .transition_timed(
                                    &authority,
                                    &session.id,
                                    &lease,
                                    &turn,
                                    TurnPhase::Completed,
                                    Some(20),
                                )
                                .unwrap();
                        }
                    }
                    "complete" => {
                        let turn = live_turn.take().expect("Live turn must precede completion");
                        journal
                            .transition_timed(
                                &authority,
                                &session.id,
                                &lease,
                                &turn,
                                TurnPhase::Completed,
                                Some(20),
                            )
                            .unwrap();
                    }
                    "verify" => {
                        let (_, turns) = journal.snapshot(&authority, &session.id).unwrap();
                        assert_eq!(turns.len(), 22);
                        assert!(turns.iter().all(|turn| turn.phase == TurnPhase::Completed));
                        assert_eq!(
                            turns
                                .iter()
                                .map(|turn| &turn.id)
                                .collect::<std::collections::BTreeSet<_>>()
                                .len(),
                            22
                        );
                    }
                    "revoke" => {
                        let device = request["device"].as_str().unwrap();
                        assert_eq!(
                            store.active_device_user(device).unwrap().unwrap().id,
                            owner.id
                        );
                        store.revoke_device(device).unwrap();
                    }
                    _ => panic!("Unexpected history fixture command {command}"),
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
            "Chromium history acceptance failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium AHP history"));
        println!("{log}");
    })
    .await
    .expect("Chromium history acceptance exceeded 90 seconds");
}
