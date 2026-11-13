//! Actual child/stdin/stdout tests, not sandbox or kernel containment evidence.
use super::*;
use std::process::Stdio;
use std::{
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::process::{Child, Command};
struct Fixture {
    directory: tempfile::TempDir,
    child: Child,
    engine: Arc<Engine>,
    live: Arc<AtomicBool>,
    chunks: Arc<Mutex<Vec<StreamChunk>>>,
}
fn definition() -> AgentDefinition {
    serde_json::from_value(json!({"schema_version":1,"name":"Fixture","description":"","harness":"opencode",
        "model":"fixture-model","mode":"fixture-mode","instructions":"literal-instructions",
        "skills":[{"id":"fixture","version":"1","instructions":"literal-skill"}],"max_seconds":60,"max_output_bytes":65536})).unwrap()
}
fn spec(id: &str, text: &str) -> TurnSpec {
    TurnSpec {
        turn_id: id.into(),
        text: text.into(),
        max_seconds: 5,
        max_output_bytes: 65536,
    }
}
impl Fixture {
    async fn new(behavior: &str, slow_intent: bool, stall_sink: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().canonicalize().unwrap();
        let script = cwd.join("agent.py");
        std::fs::write(&script, include_str!("fixtures/agent.py")).unwrap();
        let mut child = Command::new("/usr/bin/python3")
            .args([
                "-u",
                script.to_str().unwrap(),
                behavior,
                cwd.to_str().unwrap(),
            ])
            .current_dir(&cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        if behavior == "blocked" {
            use std::os::fd::AsRawFd;
            assert_eq!(
                unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) },
                4096
            );
        }
        let output = child.stdout.take().unwrap();
        let live = Arc::new(AtomicBool::new(true));
        let guard = live.clone();
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let captured = chunks.clone();
        let intent = cwd.join("intents");
        let engine=Engine::start(input,output,EngineConfig{cwd,definition:definition(),tools:None,max_session_seconds:30,max_output_bytes:65536,max_turns:8,idle_timeout_ms:10000},Hooks{
            guard:Arc::new(move|| if guard.load(Ordering::Acquire){Ok(Instant::now()+Duration::from_secs(2))}else{Err("test_live_guard_revoked")}),
            before_prompt:Arc::new(move|value|{
                let path=intent.clone();Box::pin(async move {
                    if slow_intent {tokio::time::sleep(Duration::from_millis(1500)).await;}
                    tokio::task::spawn_blocking(move||{
                        let mut file=std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
                        serde_json::to_writer(&mut file,&json!({"turn":value.turn.turn_id,"input":value.turn.text,"rpc_id":value.rpc_id,"first_turn":value.first_turn,"prompt":value.prompt})).unwrap();
                        file.write_all(b"\n").unwrap();file.sync_data().unwrap();Ok(())
                    }).await.map_err(|_|"test_intent_failed")?
                })
            }),
            stream:Arc::new(move|chunk|{
                let captured=captured.clone();Box::pin(async move{
                    captured.lock().unwrap().push(chunk);
                    if stall_sink {std::future::pending::<()>().await;}
                    Ok(())
                })
            }),
        }).await.unwrap();
        Self {
            directory,
            child,
            engine: Arc::new(engine),
            live,
            chunks,
        }
    }
    fn root(&self) -> &std::path::Path {
        self.directory.path()
    }
    async fn stop(mut self) {
        drop(self.engine);
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn real_two_turns_share_one_acp_session_memory_and_monotonic_ids() {
    let f = Fixture::new("remember", false, false).await;
    assert_eq!(
        f.engine.ready().remote_session_id,
        "persistent-fixture-session"
    );
    assert_eq!(
        f.engine
            .turn(spec("first", "nonce:unique-memory-19"))
            .await
            .unwrap()
            .status,
        TurnTerminalStatus::Completed
    );
    assert_eq!(
        f.engine
            .turn(spec("second", "recall the previous nonce"))
            .await
            .unwrap()
            .status,
        TurnTerminalStatus::Completed
    );
    let chunks = f.chunks.lock().unwrap().clone();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].turn_id, "first");
    assert_eq!(chunks[1].turn_id, "second");
    assert_eq!(chunks[1].text, "remembered:unique-memory-19");
    let calls: Vec<Value> = std::fs::read_to_string(f.root().join("calls"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        calls.iter().filter(|v| v["method"] == "initialize").count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "session/new")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "session/set_config_option")
            .count(),
        2
    );
    assert!(
        calls
            .windows(2)
            .all(|pair| pair[0]["id"].as_u64().unwrap() < pair[1]["id"].as_u64().unwrap())
    );
    assert_eq!(std::fs::read(f.root().join("effects")).unwrap(), b"x\nx\n");
    f.stop().await;
}

#[tokio::test]
async fn dropped_waiter_does_not_cancel_owned_prompt_or_reinitialize_next_turn() {
    let f = Fixture::new("slow", false, false).await;
    let engine = f.engine.clone();
    let waiter =
        tokio::spawn(async move { engine.turn(spec("first", "nonce:owned-memory")).await });
    until(|| f.root().join("effects").exists()).await;
    waiter.abort();
    let _ = waiter.await;
    f.engine.turn(spec("second", "recall")).await.unwrap();
    assert_eq!(std::fs::read(f.root().join("effects")).unwrap(), b"x\nx\n");
    assert!(
        f.chunks
            .lock()
            .unwrap()
            .iter()
            .any(|v| v.turn_id == "second" && v.text == "remembered:owned-memory")
    );
    f.stop().await;
}

#[tokio::test]
async fn utf8_chunks_are_bounded_and_sink_runs_before_turn_completion() {
    let f = Fixture::new("chunks", false, false).await;
    f.engine
        .turn(spec("first", "nonce:chunk-test"))
        .await
        .unwrap();
    let chunks = f.chunks.lock().unwrap().clone();
    assert_eq!(chunks.len(), 2);
    assert!(
        chunks
            .iter()
            .all(|v| v.text.len() <= CHUNK_BYTES && v.kind == StreamKind::Text)
    );
    assert_eq!(
        chunks.iter().map(|v| v.text.as_str()).collect::<String>(),
        "界".repeat(2000)
    );
    f.stop().await;
}

#[tokio::test]
async fn observed_idle_update_closes_engine_instead_of_becoming_next_turn_output() {
    let f = Fixture::new("idle-update", false, false).await;
    f.engine
        .turn(spec("first", "nonce:idle-test"))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), f.engine.closed())
            .await
            .unwrap(),
        "unexpected_idle_update"
    );
    assert!(f.engine.turn(spec("second", "must not run")).await.is_err());
    assert_eq!(std::fs::read(f.root().join("effects")).unwrap(), b"x\n");
    assert_eq!(f.chunks.lock().unwrap().len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn oversized_peer_frame_closes_without_sink_or_reuse() {
    let f = Fixture::new("oversized", false, false).await;
    assert!(f.engine.turn(spec("first", "nonce:large")).await.is_err());
    assert!(f.engine.turn(spec("second", "must not run")).await.is_err());
    assert!(f.chunks.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn live_guard_interrupts_partial_stdin_write_and_permanently_closes() {
    let f = Fixture::new("blocked", false, false).await;
    until(|| f.root().join("blocked-ready").exists()).await;
    let engine = f.engine.clone();
    let text = "x".repeat(12000);
    let waiter = tokio::spawn(async move { engine.turn(spec("first", &text)).await });
    until(|| f.root().join("intents").exists()).await;
    f.live.store(false, Ordering::Release);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(1), f.engine.closed())
        .await
        .unwrap();
    assert!(!f.root().join("effects").exists());
    assert!(
        f.engine
            .turn(spec("second", "must not retry partial input"))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn hard_turn_budget_starts_before_durable_intent_callback() {
    let f = Fixture::new("remember", true, false).await;
    let mut turn = spec("first", "nonce:deadline");
    turn.max_seconds = 1;
    assert!(f.engine.turn(turn).await.is_err());
    assert!(!f.root().join("effects").exists());
    assert!(!f.root().join("intents").exists());
    f.stop().await;
}

#[tokio::test]
async fn live_guard_interrupts_blocked_stream_sink_without_reporting_completed() {
    let f = Fixture::new("remember", false, true).await;
    let engine = f.engine.clone();
    let waiter = tokio::spawn(async move { engine.turn(spec("first", "nonce:sink")).await });
    until(|| !f.chunks.lock().unwrap().is_empty()).await;
    f.live.store(false, Ordering::Release);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        f.engine
            .turn(spec("second", "must not continue"))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn unsupported_human_permission_is_explicitly_denied_never_approved() {
    let f = Fixture::new("permission", false, false).await;
    assert!(
        f.engine
            .turn(spec("first", "nonce:permission"))
            .await
            .is_err()
    );
    until(|| f.root().join("denied").exists()).await;
    assert!(
        f.engine
            .turn(spec("second", "must not continue"))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn continuous_reader_rejects_idle_partial_frame_during_awaited_intent_before_prompt_write() {
    let f = Fixture::new("idle-during-intent", true, false).await;
    assert!(
        f.engine
            .turn(spec("first", "nonce:must-not-send"))
            .await
            .is_err()
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), f.engine.closed())
            .await
            .unwrap(),
        "unexpected_idle_update"
    );
    assert!(!f.root().join("intents").exists());
    assert!(!f.root().join("effects").exists());
    assert!(
        f.engine
            .turn(spec("second", "must not reuse"))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn reader_continues_past_prompt_response_while_sink_is_backpressured() {
    let f = Fixture::new("idle-update", false, true).await;
    // Peer sends a chunk, its ordered response, then an illegal idle update.
    // Sink remains pending: only the independent reader can observe the boundary.
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            f.engine.turn(spec("first", "nonce:sink-boundary"))
        )
        .await
        .unwrap()
        .is_err()
    );
    assert_eq!(f.engine.closed().await, "unexpected_idle_update");
    assert_eq!(f.chunks.lock().unwrap().len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn duplicate_turn_identity_never_writes_a_second_prompt() {
    let f = Fixture::new("remember", false, false).await;
    f.engine
        .turn(spec("first", "nonce:no-duplicate"))
        .await
        .unwrap();
    assert!(
        f.engine
            .turn(spec("first", "nonce:no-duplicate"))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(f.root().join("effects")).unwrap(), b"x\n");
    f.stop().await;
}
