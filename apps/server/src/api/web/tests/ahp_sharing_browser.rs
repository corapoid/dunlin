use super::*;
use dunlin_domain::collaboration::{Permission, WorkspaceRole};
use dunlin_session_runtime::TurnPhase;

#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_sharing() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store.create_user("sharing-browser-owner", "admin").unwrap();
    let reader = store
        .create_user("sharing-browser-reader", "member")
        .unwrap();
    let stranger = store
        .create_user("sharing-browser-stranger-admin", "admin")
        .unwrap();
    let workspace = store.create_workspace(&owner, "Sharing browser").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Shared project")
        .unwrap();
    store
        .set_team_member(
            &owner.id,
            &workspace.id,
            &reader.id,
            WorkspaceRole::Member,
            true,
            None,
        )
        .unwrap();
    store
        .set_team_project_grant(
            &owner.id,
            &workspace.id,
            &project.id,
            &reader.id,
            [
                Permission::ViewProject,
                Permission::ViewSharedSession,
                Permission::CreateSession,
            ]
            .into_iter()
            .collect(),
            None,
        )
        .unwrap();
    let initial = "AHP_SHARING_PRIVATE_INITIAL";
    let streamed = "AHP_SHARING_AUTHORIZED_DELTA";
    let unshared = "AHP_SHARING_REVOKED_PRIVATE_DELTA";
    let authority = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let journal = store.interactive_journal();
    let session = journal
        .create(&authority, &workspace.id, &project.id)
        .unwrap();
    let lease = journal
        .acquire(&authority, &session.id, 0, "sharing-browser-fixture")
        .unwrap();
    let turn = journal
        .enqueue(
            &authority,
            &session.id,
            "sharing-fixture-turn",
            "Private owner prompt",
        )
        .unwrap();
    journal.claim_next(&authority, &session.id, &lease).unwrap();
    journal
        .transition(&authority, &session.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    journal
        .output(&authority, &session.id, &lease, &turn, initial)
        .unwrap();
    drop(journal);
    let owner_code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let reader_code = store.create_pairing_code(&reader.id, 900).unwrap().0;
    let stranger_code = store.create_pairing_code(&stranger.id, 900).unwrap().0;
    let state = AppState::new(
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
    let (origin, _server) = super::live_transport::start(&state).await;
    let input = json!({"origin":origin,"ownerCode":owner_code,"readerCode":reader_code,"strangerCode":stranger_code,"readerId":reader.id,"readerName":reader.name,"session":session.id,"sessionUri":format!("ahp-session:/{}",session.id),"initial":initial,"streamed":streamed,"unshared":unshared});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-sharing.mjs"
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
                "append" | "unshared" => {
                    let store = state.store.lock().unwrap();
                    let authority = store
                        .team_project_context(&workspace.id, &project.id, &owner.id)
                        .unwrap();
                    store
                        .interactive_journal()
                        .output(
                            &authority,
                            &session.id,
                            &lease,
                            &turn,
                            if command == "append" {
                                streamed
                            } else {
                                unshared
                            },
                        )
                        .unwrap();
                }
                _ => panic!("Unexpected sharing browser command {command}"),
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
            "AHP sharing browser failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium AHP sharing"));
        println!("{log}");
    })
    .await;
    result.expect("AHP sharing browser deadline");
}
