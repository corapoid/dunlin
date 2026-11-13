use super::*;
use dunlin_session_runtime::TurnPhase;

#[tokio::test]
#[ignore = "requires production web build, Node, Python, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance_ahp_permissions() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store
        .create_user("permissions-browser-owner", "admin")
        .unwrap();
    let workspace = store
        .create_workspace(&owner, "Permissions browser")
        .unwrap();
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
                "/../../tests/fixtures/acp/permission_peer.py"
            )
            .into(),
            "normal".into(),
        ],
    ));
    let (origin, _server) = super::live_transport::start(&state).await;
    let approve = "approve AHP_PERMISSION_ALLOWED";
    let deny = "deny AHP_PERMISSION_DENIED";
    let input = json!({"origin":origin,"ownerCode":code,"workspace":workspace.id,"project":project.id,"approve":approve,"deny":deny});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/ahp-permissions.mjs"
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
            let decisions = || -> Vec<Value> {
                std::fs::read_to_string(checkout.join("received-decisions"))
                    .unwrap_or_default()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            };
            match command {
                "before-approval" => {
                    assert!(!checkout.join("allowed.txt").exists());
                    assert!(!checkout.join("denied.txt").exists());
                    assert!(!checkout.join("performed-writes").exists());
                    assert!(decisions().is_empty());
                }
                "after-approval" | "after-stale" => {
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("allowed.txt")).unwrap(),
                        approve
                    );
                    assert!(!checkout.join("denied.txt").exists());
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("performed-writes")).unwrap(),
                        "allowed.txt\n"
                    );
                    let rows = decisions();
                    assert_eq!(
                        rows.len(),
                        1,
                        "A stale nonce must never produce a second ACP decision"
                    );
                    assert_eq!(
                        rows[0]["response"]["result"]["outcome"],
                        json!({"outcome":"selected","optionId":"once"})
                    );
                }
                "verify" => {
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("received-prompts")).unwrap(),
                        format!("{approve}\n{deny}\n")
                    );
                    assert_eq!(
                        std::fs::read_to_string(checkout.join("performed-writes")).unwrap(),
                        "allowed.txt\n"
                    );
                    assert!(!checkout.join("denied.txt").exists());
                    let rows = decisions();
                    assert_eq!(rows.len(), 2);
                    assert_eq!(rows[0]["prompt"], approve);
                    assert_eq!(rows[1]["prompt"], deny);
                    let outcome = &rows[1]["response"]["result"]["outcome"];
                    assert!(
                        outcome == &json!({"outcome":"selected","optionId":"deny"})
                            || outcome == &json!({"outcome":"cancelled"})
                    );
                    assert!(!rows.iter().any(
                            |row| row["response"]["result"]["outcome"]["optionId"] == "always"
                        ));
                    let store = state.store.lock().unwrap();
                    let authority = store
                        .team_project_context(&workspace.id, &project.id, &owner.id)
                        .unwrap();
                    let turns = store
                        .interactive_journal()
                        .snapshot(&authority, request["session"].as_str().unwrap())
                        .unwrap()
                        .1;
                    assert_eq!(turns.len(), 2);
                    assert!(turns.iter().all(|turn| turn.phase == TurnPhase::Completed));
                }
                _ => panic!("Unexpected permission browser command {command}"),
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
            "AHP permissions browser failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium AHP permissions"));
        println!("{log}");
    })
    .await;
    state.interactive.shutdown().await;
    result.expect("AHP permissions browser deadline");
}
