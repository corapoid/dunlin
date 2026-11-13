//! Real kernel acceptance runs in dedicated subprocesses. No ordinary test
//! invokes bubblewrap or pretends a sandbox adapter succeeded.
#![cfg(target_os = "linux")]
use super::*;
use crate::worker_exports::{
    CapturedCheckout, ExportPermission, ExportPrincipal, PrincipalKind, policy::PolicySnapshot,
};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef, AgentSkill};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    process::{Child, Command, Stdio},
};
const AGENT: &str = include_str!("fixtures/acp_agent.py");
const FIXTURE: &str = "worker_execution::runtime::tests::private_worker_execution_fixture";
#[path = "session_tests.rs"]
mod session_tests;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    #[serde(default)]
    fault: String,
    nonce: String,
    state: PathBuf,
    program: PathBuf,
    behavior: String,
    common: Option<PathBuf>,
    boot: String,
    action: String,
    command: wire::ExecutionCommand,
    output: PathBuf,
    ready: PathBuf,
}
static BLOCKED_WRITE: AtomicBool = AtomicBool::new(false);
static BLOCKED_MARKER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
pub(super) fn configure_stdin_backpressure(input: &tokio::process::ChildStdin) {
    if BLOCKED_WRITE.load(Ordering::Acquire) {
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) },
            4096
        );
    }
}
pub(super) fn blocked_write_observed() {
    if BLOCKED_WRITE.swap(false, Ordering::AcqRel) {
        fs::write(
            BLOCKED_MARKER.get().expect("private blocked-writer marker"),
            b"partial-write-then-pending",
        )
        .unwrap();
    }
}
static FAULT: AtomicU64 = AtomicU64::new(0);
fn fault_value(name: &str) -> u64 {
    match name {
        "main-bootstrap" => 1,
        "guard-bootstrap" => 2,
        "main-handoff" => 3,
        "guard-handoff" => 4,
        "main-running" => 5,
        "guard-running" => 6,
        _ => 0,
    }
}
pub(super) fn fault_checkpoint(stage: &str, group: &process::Group) {
    let value = FAULT.load(Ordering::Acquire);
    if !matches!(
        (stage, value),
        ("bootstrap", 1 | 2) | ("handoff", 3 | 4) | ("running", 5 | 6)
    ) {
        return;
    }
    if FAULT
        .compare_exchange(value, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    if value % 2 == 1 {
        unsafe {
            libc::kill(std::process::id() as i32, libc::SIGKILL);
        }
    } else {
        group.fault_kill_guard();
    }
}
fn scope() -> local::ExportScope {
    local::ExportScope {
        controller_node_id: "controller-fixture".into(),
        worker_node_id: "worker-fixture".into(),
        workspace_id: "workspace-fixture".into(),
        host_id: "host-fixture".into(),
        enrollment_epoch: 1,
        credential_revision: 1,
    }
}
fn principal() -> ExportPrincipal {
    ExportPrincipal {
        kind: PrincipalKind::Human,
        id: "human-fixture".into(),
    }
}
fn fence(boot: &str) -> wire::ChannelFence {
    wire::ChannelFence {
        channel_id: format!("channel-{boot}"),
        controller_boot_id: "controller-boot".into(),
        worker_boot_id: boot.into(),
    }
}
fn git(root: &std::path::Path, args: &[&str]) {
    let mut command = Command::new("/usr/bin/git");
    dunlin_workspace::git::scrub_repository_environment(&mut command);
    let output = command
        .current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    spec: Spec,
}
impl Fixture {
    fn new(behavior: &str, execute: bool, linked: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        let root = base.join("checkout");
        let state = base.join("state");
        let store = WorkerStore::open(&state, scope()).unwrap();
        let common = if linked {
            let source = base.join("source");
            fs::create_dir(&source).unwrap();
            git(&source, &["init", "-q"]);
            git(
                &source,
                &[
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "fixture",
                ],
            );
            git(
                &source,
                &["worktree", "add", "-qb", "linked", root.to_str().unwrap()],
            );
            fs::write(source.join("source-private"), "must stay invisible").unwrap();
            Some(source.join(".git").canonicalize().unwrap())
        } else {
            fs::create_dir(&root).unwrap();
            git(&root, &["init", "-q"]);
            None
        };
        let policy = base.join("policy.json");
        fs::write(&policy,serde_json::to_vec(&serde_json::json!({"schema_version":1,"harnesses":["codex"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1,"acknowledge_repository_wide_metadata_write":linked})).unwrap()).unwrap();
        fs::set_permissions(&policy, fs::Permissions::from_mode(0o600)).unwrap();
        let policy = PolicySnapshot::load(&policy).unwrap();
        let linked = common.as_ref().map(|common| local::LinkedRoots {
            git_dir: common.join("worktrees/checkout"),
            common_dir: common.clone(),
        });
        let checkout = CapturedCheckout::capture(&root, policy.digest(), linked.as_ref()).unwrap();
        let export = store
            .register_captured_with_policy(&scope(), "Kernel fixture", &checkout, Some(&policy))
            .unwrap();
        let permissions = if execute {
            vec![ExportPermission::Read, ExportPermission::Execute]
        } else {
            vec![ExportPermission::Read]
        };
        let grant = store
            .set_grant(&scope(), &export.id, &principal(), &permissions, true, None)
            .unwrap();
        let bin = state.join("execution-runtime/server/tools/bin");
        crate::management::private_dir(&bin).unwrap();
        let program = bin.join("fixture-agent");
        fs::write(&program, AGENT).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let definition = AgentDefinition {
            schema_version: 1,
            name: "Known fixture profile".into(),
            description: String::new(),
            harness: "codex".into(),
            model: "fixture-model".into(),
            mode: "fixture-mode".into(),
            instructions: Some("literal-instructions".into()),
            skills: vec![AgentSkill {
                id: "fixture".into(),
                version: "1".into(),
                instructions: "literal-skill".into(),
            }],
            mcp: vec![],
            max_seconds: 60,
            max_output_bytes: 65536,
        };
        let mut command = wire::ExecutionCommand {
            channel: fence("boot-one"),
            identity: wire::ExecutionIdentity {
                command_id: "command-one".into(),
                attempt_id: "attempt-one".into(),
                fingerprint: "0".repeat(64),
            },
            descriptor: serde_json::from_value(
                serde_json::json!({"scope":scope(),"export":export,"grant":grant}),
            )
            .unwrap(),
            profile: AgentProfileRef {
                agent_id: "agent-fixture".into(),
                version: 1,
                digest: crate::worker_execution::profile_digest(&definition).unwrap(),
            },
            definition,
            input: "Perform the controlled single effect".into(),
            budgets: wire::ExecutionBudgets {
                max_seconds: 60,
                max_output_bytes: 65536,
                lease_duration_ms: 10000,
            },
        };
        command.identity.fingerprint =
            crate::worker_execution::command_fingerprint(&command).unwrap();
        // A Read-only local grant cannot manufacture an Execute descriptor. The
        // request is valid shape with the original claimed grant; worker must
        // reject it against its own current SQL row before launch/admission.
        if !execute {
            command
                .descriptor
                .grant
                .permissions
                .push(wire::ExportPermission::Execute);
            command.identity.fingerprint =
                crate::worker_execution::command_fingerprint(&command).unwrap();
        }
        let spec = Spec {
            fault: String::new(),
            nonce: uuid::Uuid::now_v7().simple().to_string(),
            state,
            program,
            behavior: behavior.into(),
            common,
            boot: "boot-one".into(),
            action: "submit".into(),
            command,
            output: base.join("result.json"),
            ready: base.join("accepted.json"),
        };
        Self {
            _directory: directory,
            root,
            spec,
        }
    }
    fn launch(&self) -> Child {
        let path = self._directory.path().join("fixture.json");
        fs::write(&path, serde_json::to_vec(&self.spec).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", FIXTURE, "--nocapture"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env(
                "DUNLIN_RELAY_E2E_BIN",
                std::env::var_os("DUNLIN_RELAY_E2E_BIN")
                    .expect("actual production executable required"),
            )
            .env("DUNLIN_PRIVATE_WORKER_EXECUTION_FIXTURE", &path)
            .env("DUNLIN_PRIVATE_WORKER_EXECUTION_NONCE", &self.spec.nonce)
            .env("DUNLIN_TEST_AMBIENT_SECRET", "must-not-reach-agent")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
    fn store(&self) -> WorkerStore {
        WorkerStore::open_existing(&self.spec.state, scope()).unwrap()
    }
}
async fn observe(mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "bounded kernel observation timed out"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
// Intentionally does NOT match production_browser_acceptance_runtime_* and
// refuses accidental execution without a private, owned bounded fixture file.
#[tokio::test]
#[ignore]
async fn private_worker_execution_fixture() {
    let path = PathBuf::from(
        std::env::var_os("DUNLIN_PRIVATE_WORKER_EXECUTION_FIXTURE")
            .expect("private fixture file required"),
    );
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .unwrap();
    let metadata = file.metadata().unwrap();
    assert!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.len() <= 65536
    );
    let mut raw = Vec::new();
    (&mut file).take(65537).read_to_end(&mut raw).unwrap();
    let spec: Spec = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        std::env::var("DUNLIN_PRIVATE_WORKER_EXECUTION_NONCE").unwrap(),
        spec.nonce
    );
    FAULT.store(fault_value(&spec.fault), Ordering::Release);
    if spec.behavior == "blocked-write" {
        BLOCKED_MARKER
            .set(spec.ready.with_extension("writing"))
            .unwrap();
        BLOCKED_WRITE.store(true, Ordering::Release);
    }
    if spec.action.starts_with("version-probe") {
        let result =
            process::fault_version_probe(&spec.program, spec.action == "version-probe-eintr");
        assert!(result.is_err());
        fs::write(&spec.output, format!("{:?}", result.unwrap_err().kind())).unwrap();
        return;
    }
    let mut harnesses = BTreeMap::new();
    let mut args = vec![spec.behavior.clone()];
    if let Some(common) = &spec.common {
        args.push(common.to_str().unwrap().into());
    }
    harnesses.insert(
        "codex".into(),
        LocalHarness {
            program: spec.program.clone(),
            args,
        },
    );
    let runtime = Runtime::new(RuntimeConfig {
        state_dir: spec.state.clone(),
        scope: scope(),
        worker_boot_id: spec.boot.clone(),
        server_config: crate::config::Config {
            sandbox: true,
            ..Default::default()
        },
        harnesses,
    })
    .expect("actual runtime containment preflight");
    let connection = runtime
        .connection(fence(&spec.boot), Arc::new(|| Ok(())))
        .unwrap();
    if spec.action.starts_with("session-") {
        session_tests::run(&runtime, &connection, &spec).await;
        return;
    }
    if spec.action == "preflight" {
        fs::write(&spec.output, b"preflight-ok").unwrap();
        runtime.shutdown().await;
        return;
    }
    if spec.action == "reconcile" {
        let reply = runtime
            .handle(
                &connection,
                Request::Reconcile {
                    channel: fence(&spec.boot),
                    identity: spec.command.identity.clone(),
                },
            )
            .await
            .unwrap();
        fs::write(&spec.output, serde_json::to_vec(&reply).unwrap()).unwrap();
        runtime.shutdown().await;
        return;
    }
    match runtime
        .handle(
            &connection,
            Request::Execute {
                command: Box::new(spec.command.clone()),
            },
        )
        .await
    {
        Ok(Some(Reply::Accepted { record })) => {
            fs::write(&spec.ready, serde_json::to_vec(&record).unwrap()).unwrap()
        }
        error => {
            fs::write(&spec.output, format!("{error:?}")).unwrap();
            runtime.shutdown().await;
            return;
        }
    }
    let started = Instant::now();
    let mut renewal = Instant::now();
    let mut sequence = 0;
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(35),
            "fixture did not obtain a deliverable stop-confirmed outcome"
        );
        if renewal.elapsed() >= Duration::from_secs(1) {
            let receipt: wire::ExecutionRecord =
                serde_json::from_slice(&fs::read(&spec.ready).unwrap()).unwrap();
            sequence += 1;
            let _ = runtime
                .handle(
                    &connection,
                    Request::Renew {
                        fence: receipt.receipt.fence,
                        sequence,
                        lease_duration_ms: 5000,
                    },
                )
                .await;
            renewal = Instant::now();
        }
        if let Ok(records) = runtime.outbox(&connection).await
            && let Some(record) = records.first()
        {
            fs::write(&spec.output, serde_json::to_vec(record).unwrap()).unwrap();
            if matches!(record.status, wire::ExecutionStatus::Completed) {
                let replay = runtime
                    .handle(
                        &connection,
                        Request::Execute {
                            command: Box::new(spec.command.clone()),
                        },
                    )
                    .await
                    .unwrap();
                assert!(matches!(replay,Some(Reply::Accepted{record:r}) if r==*record));
            }
            runtime.shutdown().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_execution() {
    let _ = std::env::var_os("DUNLIN_RELAY_E2E_BIN")
        .expect("build the actual production executable first");
    // Every runtime lives in a separate dedicated subreaper process; the Rust
    // test runner itself never becomes a wait-any reaper of unrelated tests.
    for (behavior, expected, effect) in [
        ("success", wire::ExecutionStatus::Completed, true),
        ("large", wire::ExecutionStatus::Unknown, true),
        ("bad-mode", wire::ExecutionStatus::Failed, false),
        ("permission", wire::ExecutionStatus::Unknown, false),
    ] {
        let fixture = Fixture::new(behavior, true, true);
        let mut child = fixture.launch();
        observe(|| fixture.spec.output.exists()).await;
        observe(|| child.try_wait().unwrap().is_some()).await;
        assert!(child.wait().unwrap().success());
        let record: wire::ExecutionRecord =
            serde_json::from_slice(&fs::read(&fixture.spec.output).unwrap()).unwrap();
        assert_eq!(record.status, expected, "{behavior}: {:?}", record.outcome);
        assert!(record.outcome.as_ref().unwrap().text.len() <= 16 * 1024);
        assert_eq!(fixture.root.join("effects").exists(), effect);
        if behavior == "success" {
            assert_eq!(fs::read(fixture.root.join("effects")).unwrap(), b"x\n");
            assert_eq!(
                fs::read_to_string(fixture.spec.common.as_ref().unwrap().join("worker-write"))
                    .unwrap(),
                "allowed"
            );
        }
    }
    let fixture = Fixture::new("success", false, false);
    let mut child = fixture.launch();
    observe(|| fixture.spec.output.exists()).await;
    observe(|| child.try_wait().unwrap().is_some()).await;
    assert!(child.wait().unwrap().success());
    assert!(!fixture.root.join("effects").exists());
    assert!(
        fs::read_to_string(&fixture.spec.output)
            .unwrap()
            .contains("AuthorityRevoked")
    );
    assert_eq!(
        rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3"))
            .unwrap()
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn retained(fixture: &Fixture) -> Option<wire::ExecutionRecord> {
    let store = fixture.store();
    let scope = scope();
    let wire = fixture.spec.command.channel.clone();
    let guard = || Ok(());
    let context = ExecutionChannel::verify(&scope, &wire, &guard).unwrap();
    store
        .reconcile_execution(&context, &fixture.spec.command.identity)
        .ok()
        .flatten()
}
async fn result_file(fixture: &Fixture, child: &mut Child) {
    let until = tokio::time::Instant::now() + Duration::from_secs(25);
    while !fixture.spec.output.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let mut error = String::new();
            if let Some(stderr) = child.stderr.as_mut() {
                stderr.take(16384).read_to_string(&mut error).unwrap();
            }
            panic!("dedicated fixture exited before result: {status}; {error}");
        }
        assert!(
            tokio::time::Instant::now() < until,
            "dedicated fixture result timed out"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_execution_preflight() {
    let mut fixture = Fixture::new("success", true, false);
    fixture.spec.action = "preflight".into();
    let mut child = fixture.launch();
    result_file(&fixture, &mut child).await;
    observe(|| child.try_wait().unwrap().is_some()).await;
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read(&fixture.spec.output).unwrap(), b"preflight-ok");
    assert!(!fixture.root.join("effects").exists());
}
#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_execution_faults() {
    // Stage hooks only select when to inject a real SIGKILL into a dedicated
    // test supervisor/its owned guardian pidfd. They do not replace bwrap, grant
    // checks, fd handoff, namespace teardown or the durable stop attestation.
    for fault in [
        "main-bootstrap",
        "guard-bootstrap",
        "main-handoff",
        "guard-handoff",
        "main-running",
        "guard-running",
    ] {
        let mut fixture = Fixture::new("hang", true, true);
        fixture.spec.fault = fault.into();
        let mut child = fixture.launch();
        observe(|| {
            retained(&fixture).is_some_and(|record| record.status == wire::ExecutionStatus::Unknown)
        })
        .await;
        let record = retained(&fixture).unwrap();
        assert_eq!(record.receipt.identity, fixture.spec.command.identity);
        observe(|| child.try_wait().unwrap().is_some()).await;
        let status = child.wait().unwrap();
        if fault.starts_with("main-") {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(libc::SIGKILL));
        } else {
            assert!(status.success());
        }
        assert_eq!(
            fixture.root.join("effects").exists(),
            fault.ends_with("running"),
            "{fault}"
        );
        if fault.ends_with("running") {
            let held = fixture
                .spec
                .common
                .as_ref()
                .unwrap()
                .join("objects/held-effect");
            let size = fs::metadata(&held).unwrap().len();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(
                fs::metadata(&held).unwrap().len(),
                size,
                "{fault}: held FD survived namespace teardown"
            );
        }
        let database =
            rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3")).unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT reservation FROM worker_execution_inbox WHERE attempt_id=?1",
                    [&fixture.spec.command.identity.attempt_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        // A real new supervisor reads the original receipt/outcome; it never
        // reconstructs a lease or spawns a second paid turn for this identity.
        fixture.spec.action = "reconcile".into();
        fixture.spec.boot = "boot-restarted".into();
        fixture.spec.fault.clear();
        fs::remove_file(&fixture.spec.output).ok();
        let mut restarted = fixture.launch();
        result_file(&fixture, &mut restarted).await;
        observe(|| restarted.try_wait().unwrap().is_some()).await;
        assert!(restarted.wait().unwrap().success());
        let reply: Option<Reply> =
            serde_json::from_slice(&fs::read(&fixture.spec.output).unwrap()).unwrap();
        assert!(matches!(reply,Some(Reply::Reconciled{record:Some(ref old),..}) if old==&record));
    }
}
#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_execution_revocation() {
    for change in ["grant", "policy", "objects", "checkout", "blocked-writer"] {
        let mut fixture = Fixture::new(
            if change == "blocked-writer" {
                "blocked-write"
            } else {
                "hang"
            },
            true,
            true,
        );
        if change == "blocked-writer" {
            fixture.spec.command.input = format!("valid-large-input\n{}", "u".repeat(14000));
            fixture.spec.command.definition.instructions =
                Some(format!("literal-instructions\n{}", "i".repeat(14000)));
            fixture.spec.command.profile.digest =
                crate::worker_execution::profile_digest(&fixture.spec.command.definition).unwrap();
            fixture.spec.command.identity.fingerprint =
                crate::worker_execution::command_fingerprint(&fixture.spec.command).unwrap();
        }
        let mut child = fixture.launch();
        observe(|| fixture.root.join("held-ready").exists()).await;
        if change == "blocked-writer" {
            observe(|| fixture.spec.ready.with_extension("writing").exists()).await;
        }
        let store = fixture.store();
        let original_digest = fixture
            .spec
            .command
            .descriptor
            .export
            .settings_digest
            .clone();
        let mut held = fixture
            .spec
            .common
            .as_ref()
            .unwrap()
            .join("objects/held-effect");
        match change {
            "grant" | "blocked-writer" => {
                store
                    .set_grant(
                        &scope(),
                        &fixture.spec.command.descriptor.export.id,
                        &principal(),
                        &[],
                        false,
                        Some(1),
                    )
                    .unwrap();
            }
            "policy" => {
                rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3"))
                    .unwrap()
                    .execute(
                        "UPDATE export_execution_policies SET digest=?1",
                        ["0".repeat(64)],
                    )
                    .unwrap();
            }
            "objects" => {
                let objects = fixture.spec.common.as_ref().unwrap().join("objects");
                let old = objects.with_file_name("old-objects");
                fs::rename(&objects, &old).unwrap();
                fs::create_dir(&objects).unwrap();
                held = old.join("held-effect");
            }
            "checkout" => {
                let old = fixture.root.with_file_name("old-checkout");
                fs::rename(&fixture.root, &old).unwrap();
                fs::create_dir(&fixture.root).unwrap();
            }
            _ => unreachable!(),
        }
        let database =
            rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3")).unwrap();
        observe(|| {
            database
                .query_row(
                    "SELECT reservation FROM worker_execution_inbox WHERE attempt_id=?1",
                    [&fixture.spec.command.identity.attempt_id],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                == Some(0)
        })
        .await;
        let size = fs::metadata(&held).unwrap().len();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            fs::metadata(&held).unwrap().len(),
            size,
            "{change}: silent child kept writing"
        );
        // Retained output needs a fresh Read grant and an intact export policy;
        // neither repair restores the original execution grant/stop flag.
        if matches!(change, "grant" | "blocked-writer") {
            store
                .set_grant(
                    &scope(),
                    &fixture.spec.command.descriptor.export.id,
                    &principal(),
                    &[ExportPermission::Read],
                    true,
                    Some(2),
                )
                .unwrap();
        }
        if change == "policy" {
            database
                .execute(
                    "UPDATE export_execution_policies SET digest=?1",
                    [original_digest],
                )
                .unwrap();
        }
        result_file(&fixture, &mut child).await;
        observe(|| child.try_wait().unwrap().is_some()).await;
        assert!(child.wait().unwrap().success());
        let record: wire::ExecutionRecord =
            serde_json::from_slice(&fs::read(&fixture.spec.output).unwrap()).unwrap();
        assert_eq!(record.status, wire::ExecutionStatus::Unknown);
    }
}

#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_execution_version_bounds() {
    for (behavior, interrupt) in [("hang", false), ("flood", false), ("hang", true)] {
        let mut fixture = Fixture::new("success", true, false);
        fixture.spec.action = if interrupt {
            "version-probe-eintr"
        } else {
            "version-probe"
        }
        .into();
        let ticks = fixture._directory.path().join("version-ticks");
        let ready = fixture._directory.path().join("version-child-ready");
        let body = format!(
            "#!/usr/bin/python3\nimport os,time\nfrom pathlib import Path\nf=open({},'ab',buffering=0)\nr,w=os.pipe()\nif os.fork()==0:\n os.close(r)\n os.setsid()\n f.write(b'x')\n os.write(w,b'R')\n while True: f.write(b'x'); time.sleep(.02)\nos.close(w)\nassert os.read(r,1)==b'R'\nPath({}).write_text('ready')\nif {}=='hang':\n while True: time.sleep(60)\nelse:\n while True: os.write(1,b'x'*4096)\n",
            serde_json::to_string(&ticks).unwrap(),
            serde_json::to_string(&ready).unwrap(),
            serde_json::to_string(behavior).unwrap()
        );
        fs::write(&fixture.spec.program, body).unwrap();
        let started = Instant::now();
        let mut child = fixture.launch();
        result_file(&fixture, &mut child).await;
        observe(|| child.try_wait().unwrap().is_some()).await;
        assert!(child.wait().unwrap().success());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!fixture.root.join("effects").exists());
        assert!(
            ready.exists(),
            "forked setsid child actually held stdout and a ticking FD"
        );
        let stopped_size = fs::metadata(&ticks).unwrap().len();
        assert!(stopped_size > 0);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            fs::metadata(&ticks).unwrap().len(),
            stopped_size,
            "version probe orphan kept writing after fixture reported cleanup"
        );
        if behavior == "hang" {
            assert_eq!(
                fs::read_to_string(&fixture.spec.output).unwrap(),
                "TimedOut"
            );
        }
    }
}
