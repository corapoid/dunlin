//! Rendered remote MCP creation and approval against the real runtime fixture.
use super::*;
use tokio::io::AsyncWriteExt;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    fixture: &Fixture,
    origin: &str,
    request: &crate::store::remote_bindings::resolver::sessions::RemoteSessionRequest,
    project: &str,
    root: &std::path::Path,
    state_dir: &std::path::Path,
    selections: &[crate::interactive::mcp::Selection],
    cookie: &str,
    form_action: Option<&str>,
    no_effects: bool,
) {
    let code = fixture
        .state
        .store
        .lock()
        .unwrap()
        .create_pairing_code(&fixture.owner, 900)
        .unwrap()
        .0;
    let input = json!({"origin":origin,"ownerCode":code,"workspace":fixture.workspace,"project":project,
        "binding":request.binding,"profile":request.profile,"selection":selections[0],"form_action":form_action});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/remote-mcp-session.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{input}\n").as_bytes())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
        .await
        .expect("remote MCP Chromium deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 4 * 1024 * 1024);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "passed");
    for name in ["desktop", "mobile"] {
        println!(
            "REMOTE_MCP_IMAGE_{} {}",
            name.to_ascii_uppercase(),
            result[name].as_str().unwrap()
        );
    }
    let id = result["session_id"].as_str().unwrap();
    let (status, stopped) = fixture_http(
        &fixture.state,
        cookie,
        &format!("/v1/web/interactive-sessions/{id}/stop"),
        json!({"expected_epoch":result["epoch"]}),
    )
    .await;
    assert_eq!(status, 202, "{stopped}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let done = {
                let store = fixture.state.store.lock().unwrap();
                let snapshot = store.remote_session_snapshot(id).unwrap();
                snapshot.phase == "stopped" && snapshot.sequence == 3 && snapshot.confirmed_ack == 3
            };
            if done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let store=fixture.state.store.lock().unwrap();
        let snapshot=store.remote_session_snapshot(id).unwrap();
        let database=rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
        let rows=database.prepare("SELECT phase,reservation,tip_sequence,ack_sequence FROM worker_sessions ORDER BY id").unwrap()
            .query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        panic!("browser MCP stop/ACK: controller={:?}, source={rows:?}",(snapshot.phase,snapshot.sequence,snapshot.ack_sequence,snapshot.confirmed_ack));
    });
    if no_effects {
        assert!(!root.join("effects").exists());
    } else {
        assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\nx\n");
    }
    let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
    let counts:(i64,i64,i64,i64)=database.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT sum(ack_sequence) FROM worker_sessions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(counts, (2, 3, 0, 8));
    let calls = fs::read_to_string(root.join("session-calls")).unwrap();
    let calls: Vec<Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (method, count) in [("initialize", 2), ("session/new", 2), ("session/prompt", 3)] {
        assert_eq!(
            calls.iter().filter(|call| call["method"] == method).count(),
            count
        );
    }
    let held = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        held
    );
}
