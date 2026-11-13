use super::*;
use dunlin_session_runtime::TurnPhase;

/// Production Agent Builder publishes a profile and creates an actual actor;
/// only the ACP peer is controlled and no external model/provider is required.
#[tokio::test]
#[ignore = "requires production web build, Node, Python, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_create_idempotency() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT required");
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let harness = include_str!("../../../interactive/tests/fake_acp.py");
    let harness_path = dir.path().join("create_acp.py");
    std::fs::write(&harness_path, harness).unwrap();
    let database = dir.path().join("relay.db");
    let store = Store::open(&database).unwrap();
    let owner = store.create_user("create-browser-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Create browser").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Actor")
        .unwrap();
    let native = store
        .create_project("Checkout", checkout.to_str().unwrap())
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
            db: database.clone(),
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
    let prompt = "IDEMPOTENT_CREATION_ONE_ACTOR";
    let input = json!({"origin":origin,"ownerCode":code,"workspace":workspace.id,"project":project.id,"prompt":prompt});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-create.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Node required");
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
                "verify-accepted" | "verify-completed" => {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                    loop {
                        if checkout.join("actor-starts").exists() {
                            break;
                        }
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "ACP launch deadline"
                        );
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("actor-starts"))
                            .unwrap()
                            .lines()
                            .count(),
                        1,
                        "Retry spawned a second ACP process"
                    );
                    let db = rusqlite::Connection::open(&database).unwrap();
                    let count: i64 = db
                        .query_row("SELECT count(*) FROM interactive_sessions", [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    assert_eq!(count, 1, "Retry created a second durable session");
                    if command == "verify-completed" {
                        assert_eq!(
                            std::fs::read_to_string(checkout.join("received-prompts")).unwrap(),
                            format!("BROWSER_TOP_INSTRUCTIONS\n\n[Skill browser-skill@1.0.0]\nBROWSER_SKILL_INSTRUCTIONS\n\n## User request\n{prompt}\n")
                        );
                        let store = state.store.lock().unwrap();
                        let authority = store
                            .team_project_context(&workspace.id, &project.id, &owner.id)
                            .unwrap();
                        let turns = store
                            .interactive_journal()
                            .snapshot(&authority, request["session"].as_str().unwrap())
                            .unwrap()
                            .1;
                        assert_eq!(turns.len(), 1);
                        assert_eq!(turns[0].phase, TurnPhase::Completed);
                    }
                }
                _ => panic!("Unexpected command {command}"),
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
            "Creation helper browser failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium production Agent Builder/creation UI"));
        println!("{log}");
    })
    .await;
    state.interactive.shutdown().await;
    result.expect("Creation helper browser deadline");
}
