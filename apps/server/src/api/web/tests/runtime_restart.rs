//! Actual executable crash/relaunch acceptance; no Store-only restart substitute.
#![cfg(target_os = "linux")]
use super::*;
use dunlin_session_runtime::{SessionPhase, TurnPhase};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
const CLIENT: &str = "production-restart-client";
const AMBIGUOUS: &str = "ambiguous-effect-before-crash";
const QUEUED: &str = "queued-never-dispatched";
const FRESH: &str = "fresh-explicit-session";
const CREATE_KEY: &str = "real-runtime-restart-admission-v1";
const PEER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/runtime-restart/acp_peer.py"
));
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    db: PathBuf,
    config: PathBuf,
    checkout: PathBuf,
    origin: String,
    owner: String,
    workspace: String,
    project: String,
    cookie: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        let peer = checkout.join("peer.py");
        std::fs::write(&peer, PEER).unwrap();
        assert!(!peer.to_string_lossy().contains(char::is_whitespace));
        let db = dir.path().join("relay.db");
        let store = Store::open(&db).unwrap();
        let owner = store.create_user("real-restart-owner", "admin").unwrap();
        let (device, _) = store.create_device(&owner.id, "restart-browser").unwrap();
        let cookie = format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        );
        let workspace = store
            .create_workspace(&owner, "Restart acceptance")
            .unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Controlled runtime")
            .unwrap();
        let native = store
            .create_project("Controlled checkout", checkout.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &owner.id).unwrap();
        store
            .set_local_binding(&owner.id, &workspace.id, &project.id, &native.id, None)
            .unwrap();
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let config = dir.path().join("relay.toml");
        std::fs::write(&config,format!("db = {}\nlisten = {}\nharness = {}\nsandbox = true\nstore_transcripts = true\nexperimental_interactive_runtime = true\n",json!(db),json!(address.to_string()),json!(format!("python3 {}",peer.display())))).unwrap();
        Self {
            _dir: dir,
            store,
            db,
            config,
            checkout,
            origin: format!("http://{address}"),
            owner: owner.id,
            workspace: workspace.id,
            project: project.id,
            cookie,
        }
    }
    fn context(&self) -> dunlin_domain::collaboration::AccessContext {
        self.store
            .team_project_context(&self.workspace, &self.project, &self.owner)
            .unwrap()
    }
    fn phase(&self, session: &str) -> SessionPhase {
        self.store
            .interactive_journal()
            .snapshot(&self.context(), session)
            .unwrap()
            .0
            .phase
    }
    fn turns(&self, session: &str) -> Vec<dunlin_session_runtime::Turn> {
        self.store
            .interactive_journal()
            .snapshot(&self.context(), session)
            .unwrap()
            .1
    }
    fn permission(&self, session: &str) -> Option<Value> {
        let chat = serde_json::to_value(
            self.store
                .interactive_journal()
                .projected_chat(&self.context(), session)
                .unwrap(),
        )
        .unwrap();
        let active = chat["activeTurn"].as_object()?;
        let tool = active
            .get("responseParts")?
            .as_array()?
            .iter()
            .find(|p| p["toolCall"]["status"] == "pending-confirmation")?
            .get("toolCall")?;
        let selected = tool["options"]
            .as_array()?
            .iter()
            .find(|o| o["kind"] == "approve")?["id"]
            .clone();
        Some(
            json!({"type":"chat/toolCallConfirmed","turnId":active["id"],"toolCallId":tool["toolCallId"],"approved":true,"selectedOptionId":selected}),
        )
    }
    fn lines(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.checkout.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
#[derive(Clone)]
struct ProcessIdentity {
    pid: i32,
    start: String,
}
fn process_missing(error: &std::io::Error) -> bool {
    // Only ENOENT is a normal /proc process/thread exit race. Other errors must
    // never turn a live but unreadable process into a successful orphan check.
    error.kind() == std::io::ErrorKind::NotFound
}
fn identity(pid: i32) -> Option<ProcessIdentity> {
    let path = format!("/proc/{pid}/stat");
    let stat = match std::fs::read_to_string(&path) {
        Ok(stat) => stat,
        Err(error) if process_missing(&error) => return None,
        Err(error) => panic!("Cannot inspect process {pid} at {path}: {error}"),
    };
    let (prefix, tail) = stat
        .rsplit_once(") ")
        .unwrap_or_else(|| panic!("Malformed stat for process {pid}: missing comm/state boundary"));
    let recorded_pid = prefix
        .split_once(" (")
        .and_then(|(number, _)| number.parse::<i32>().ok())
        .unwrap_or_else(|| panic!("Malformed stat for process {pid}: invalid PID/comm"));
    assert_eq!(recorded_pid, pid, "Mismatched stat PID at {path}");
    let fields = tail.split_whitespace().collect::<Vec<_>>();
    assert!(
        fields.len() >= 20,
        "Malformed stat for process {pid}: missing start time"
    );
    let state = fields[0];
    assert!(
        state.len() == 1 && "RSDZTtXxKWPI".contains(state),
        "Malformed stat for process {pid}: invalid state {state:?}"
    );
    for (index, field) in fields.iter().enumerate().skip(1) {
        assert!(
            field.parse::<i128>().is_ok(),
            "Malformed stat for process {pid}: invalid numeric field {}",
            index + 3
        );
    }
    let start = fields[19].parse::<u64>().unwrap_or_else(|error| {
        panic!("Malformed stat for process {pid}: invalid start time: {error}")
    });
    if matches!(state, "Z" | "X") {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        start: start.to_string(),
    })
}
fn descendants(pid: i32) -> Vec<ProcessIdentity> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![pid];
    let mut result = Vec::new();
    while let Some(parent) = queue.pop() {
        let task_path = format!("/proc/{parent}/task");
        let tasks = match std::fs::read_dir(&task_path) {
            Ok(tasks) => tasks,
            Err(error) if process_missing(&error) => continue,
            Err(error) => {
                panic!("Cannot enumerate tasks for process {parent} at {task_path}: {error}")
            }
        };
        for entry in tasks {
            let task = match entry {
                Ok(task) => task,
                Err(error) if process_missing(&error) => continue,
                Err(error) => {
                    panic!("Cannot iterate tasks for process {parent} at {task_path}: {error}")
                }
            };
            let task_id = task
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<i32>().ok())
                .filter(|tid| *tid > 0)
                .unwrap_or_else(|| {
                    panic!("Invalid task entry for process {parent}: {:?}", task.path())
                });
            let children_path = task.path().join("children");
            let children = match std::fs::read_to_string(&children_path) {
                Ok(children) => children,
                Err(error) if process_missing(&error) => continue,
                Err(error) => panic!(
                    "Cannot inspect children of process {parent}, task {task_id}, at {}: {error}",
                    children_path.display()
                ),
            };
            for raw in children.split_whitespace() {
                let child = raw.parse::<i32>().ok().filter(|child|*child>0).unwrap_or_else(|| panic!("Malformed child PID {raw:?} for process {parent}, task {task_id}, at {}", children_path.display()));
                if seen.insert(child)
                    && let Some(stamp) = identity(child)
                {
                    result.push(stamp);
                    queue.push(child);
                }
            }
        }
    }
    result
}
struct ServerProcess {
    child: Child,
    known: Vec<ProcessIdentity>,
    log: PathBuf,
}
impl ServerProcess {
    fn start(f: &Fixture, boot: &str) -> Self {
        let binary = std::env::var_os("DUNLIN_RELAY_E2E_BIN")
            .expect("DUNLIN_RELAY_E2E_BIN must point to the built production executable");
        assert!(Path::new(&binary).is_file());
        let log = f.db.with_file_name(format!("server-{boot}.log"));
        let output = std::fs::File::create(&log).unwrap();
        let child = Command::new(binary)
            .arg("serve")
            .arg("--config")
            .arg(&f.config)
            .env_remove("DUNLIN_RELAY_WEB_ROOT")
            .env_remove("DUNLIN_RELAY_LISTEN")
            .env_remove("DUNLIN_RELAY_DB")
            .env_remove("DUNLIN_RELAY_HARNESS")
            .env_remove("DUNLIN_RELAY_SANDBOX")
            .env_remove("DUNLIN_RELAY_STORE_TRANSCRIPTS")
            .env("TOKIO_WORKER_THREADS", "2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap();
        Self {
            child,
            known: Vec::new(),
            log,
        }
    }
    fn record_children(&mut self) {
        self.known = descendants(self.child.id() as i32);
        assert!(!self.known.is_empty(), "No real harness process observed");
    }
    fn crash(&mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(!status.success());
    }
    fn diagnostics(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}
impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.known.extend(descendants(self.child.id() as i32));
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        // Failure-only hygiene happens AFTER orphan assertions, never instead of
        // proving production parent-death/process ownership on the crash path.
        for old in &self.known {
            if identity(old.pid).is_some_and(|current| current.start == old.start) {
                unsafe {
                    libc::kill(old.pid, libc::SIGKILL);
                }
            }
        }
    }
}
async fn http(f: &Fixture, method: &str, path: &str, body: Value) -> Result<(u16, Value), String> {
    http_with_key(f, method, path, body, None).await
}
async fn http_with_key(
    f: &Fixture,
    method: &str,
    path: &str,
    body: Value,
    key: Option<&str>,
) -> Result<(u16, Value), String> {
    let host = f.origin.strip_prefix("http://").unwrap();
    let mut socket = tokio::net::TcpStream::connect(host)
        .await
        .map_err(|e| e.to_string())?;
    let body = if method == "GET" {
        String::new()
    } else {
        body.to_string()
    };
    // Private fixture callers supply only this fixed, valid admission identity.
    let admission = key
        .map(|key| format!("Idempotency-Key: {key}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {}\r\nCookie: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{admission}Connection: close\r\n\r\n{body}",
        f.origin,
        f.cookie,
        body.len()
    );
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    socket
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    let boundary = bytes
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .ok_or("Invalid HTTP response")?;
    let headers = String::from_utf8_lossy(&bytes[..boundary]);
    let status = headers
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .ok_or("Missing HTTP status")?
        .parse::<u16>()
        .map_err(|e| e.to_string())?;
    let raw = &bytes[boundary + 4..];
    let mut body = Vec::new();
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut remaining = raw;
        loop {
            let end = remaining
                .windows(2)
                .position(|v| v == b"\r\n")
                .ok_or("Invalid HTTP chunk")?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&remaining[..end])
                    .map_err(|e| e.to_string())?
                    .split(';')
                    .next()
                    .unwrap(),
                16,
            )
            .map_err(|e| e.to_string())?;
            remaining = &remaining[end + 2..];
            if size == 0 {
                break;
            }
            if remaining.len() < size + 2 {
                return Err("Short HTTP chunk".into());
            }
            body.extend_from_slice(&remaining[..size]);
            remaining = &remaining[size + 2..];
        }
    } else {
        body.extend_from_slice(raw);
    }
    let value =
        serde_json::from_slice(&body).map_err(|e| format!("Invalid HTTP JSON ({status}): {e}"))?;
    Ok((status, value))
}
async fn ready(f: &Fixture, server: &mut ServerProcess) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "Actual server exited: {}",
            server.diagnostics()
        );
        if tokio::time::timeout(
            Duration::from_secs(2),
            http(f, "GET", "/v1/web/session", Value::Null),
        )
        .await
        .is_ok_and(|r| r.is_ok_and(|(s, _)| s == 200))
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Server readiness deadline: {}",
            server.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn create(f: &Fixture) -> String {
    create_with_key(f, None).await["session"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn create_with_key(f: &Fixture, key: Option<&str>) -> Value {
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/interactive-sessions",
        f.workspace, f.project
    );
    let (status, value) = http_with_key(
        f,
        "POST",
        &path,
        json!({"model":"fixture-model","mode":"plan","max_seconds":60,"max_output_bytes":1048576}),
        key,
    )
    .await
    .unwrap();
    assert_eq!(status, 202, "Actual actor create: {value}");
    value
}
async fn connect(f: &Fixture) -> Socket {
    let mut request = format!("{}/v1/web/ahp", f.origin.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", f.cookie.parse().unwrap());
    request
        .headers_mut()
        .insert("origin", f.origin.parse().unwrap());
    connect_async(request).await.unwrap().0
}
async fn receive(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await.expect("AHP stream closed").unwrap() {
                ClientMessage::Text(text) => return serde_json::from_str(&text).unwrap(),
                ClientMessage::Close(frame) => panic!("AHP policy close: {frame:?}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}
async fn rpc(socket: &mut Socket, id: u64, method: &str, params: Value) -> Value {
    socket
        .send(ClientMessage::Text(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let value = receive(socket).await;
        if value["id"] == id {
            assert!(value.get("error").is_none(), "AHP RPC failure: {value}");
            return value["result"].clone();
        }
    }
}
async fn dispatch(socket: &mut Socket, channel: &str, seq: u64, action: Value) -> Value {
    socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","method":"dispatchAction","params":{"channel":channel,"clientSeq":seq,"action":action}}).to_string().into())).await.unwrap();
    loop {
        let value = receive(socket).await;
        if value["method"] == "action"
            && value["params"]["origin"]["clientId"] == CLIENT
            && value["params"]["origin"]["clientSeq"] == seq
        {
            return value["params"].clone();
        }
    }
}
fn prompt(id: &str, text: &str) -> Value {
    json!({"type":"chat/pendingMessageSet","kind":"queued","id":id,"message":{"text":text,"origin":{"kind":"user"}}})
}
async fn wait_until(mut check: impl FnMut() -> bool, description: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Deadline: {description}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn production_worker_cli_rejects_fifo_and_never_echoes_stdin() {
    let binary = std::env::var_os("DUNLIN_RELAY_E2E_BIN").expect("production executable required");
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory.path().join("worker-fifo");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let child = tokio::process::Command::new(&binary)
        .args(["worker", "run", "--config"])
        .arg(&fifo)
        .arg("--credential-stdin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
        .await
        .expect("CLI must reject FIFO before stdin or LocalAPI")
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("worker_config_regular_file_required")
    );
    let config = directory.path().join("worker.toml");
    std::fs::write(&config,format!("host_id=\"host-cli\"\nworkspace_id=\"workspace-cli\"\nenrollment_epoch=1\ncredential_revision=1\n[network]\nsocket={}\nnode_id=\"n-worker\"\nserve_origin=\"https://worker.tail123.ts.net\"\nlisten=\"127.0.0.1:8788\"\n[controller]\nnode_id=\"n-controller\"\nserve_origin=\"https://controller.tail123.ts.net\"\n[capabilities]\nprotocol_version=1\nversion=\"fixture\"\nos=\"linux\"\narchitecture=\"aarch64\"\nharnesses=[]\nsandbox=\"none\"\nmax_parallel=1\n",json!(directory.path().join("not-a-real-daemon.sock")))).unwrap();
    let mut child = tokio::process::Command::new(&binary)
        .args(["worker", "run", "--config"])
        .arg(&config)
        .arg("--credential-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    const DUMMY: &str = "dummy-credential-input-must-never-appear-in-output";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{DUMMY}\n").as_bytes())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
        .await
        .expect("CLI input rejection deadline")
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid_worker_credential"));
    assert!(!stderr.contains(DUMMY));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(DUMMY));
    production_worker_export_cli(&binary, &config, directory.path()).await;
    production_local_metadata_cli(&binary, directory.path()).await;
}

async fn export_cli(
    binary: &std::ffi::OsStr,
    config: &Path,
    state: &Path,
    arguments: &[&str],
    success: bool,
) -> Value {
    let child = tokio::process::Command::new(binary)
        .args(["worker", "export", arguments[0], "--config"])
        .arg(config)
        .arg("--state-dir")
        .arg(state)
        .args(&arguments[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
        .await
        .expect("Operator-local export CLI must not contact network or wait for credentials")
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The actual executable must preserve the descriptor redaction boundary.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(config.parent().unwrap().to_str().unwrap()));
    assert!(!stdout.contains("PRIVATE_EXPORT_ROOT"));
    if success {
        serde_json::from_slice(&output.stdout).expect("Export CLI emits structured metadata")
    } else {
        assert!(output.stdout.is_empty());
        Value::Null
    }
}

async fn metadata_cli(
    binary: &std::ffi::OsStr,
    db: &Path,
    arguments: &[&str],
    success: bool,
) -> Value {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(binary)
            .args(["local-metadata", arguments[0], "--db"])
            .arg(db)
            .args(&arguments[1..])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("Local metadata CLI must finish without network or credentials")
    .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(db.parent().unwrap().to_str().unwrap()));
    if success {
        serde_json::from_slice(&output.stdout).expect("Path-free metadata JSON")
    } else {
        assert!(output.stdout.is_empty());
        Value::Null
    }
}

async fn production_local_metadata_cli(binary: &std::ffi::OsStr, directory: &Path) {
    let repo = directory.join("metadata-source");
    let linked = directory.join("PRIVATE_METADATA_CHECKOUT");
    std::fs::create_dir(&repo).unwrap();
    let git = |arguments: &[&str]| {
        let output = Command::new("/usr/bin/git")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Metadata fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "fixture"]);
    git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "linked",
        linked.to_str().unwrap(),
    ]);
    let common = repo.join(".git");
    let pointer = std::fs::read_to_string(linked.join(".git")).unwrap();
    let git_dir = PathBuf::from(pointer.trim().strip_prefix("gitdir: ").unwrap());
    let db = directory.join("metadata-relay.db");
    let store = Store::open(&db).unwrap();
    let user = store
        .create_user("binary-metadata-member", "member")
        .unwrap();
    let project = store
        .create_project("binary-linked", linked.to_str().unwrap())
        .unwrap();
    store.grant(&project.id, &user.id).unwrap();
    let p = project.id.as_str();
    let u = user.id.as_str();
    let checked = || {
        [
            "check",
            "--project-id",
            p,
            "--user-id",
            u,
            "--permission",
            "read",
        ]
    };
    metadata_cli(binary, &db, &checked(), false).await;
    let approved = metadata_cli(
        binary,
        &db,
        &[
            "approve",
            "--project-id",
            p,
            "--expected-revision",
            "none",
            "--git-dir",
            git_dir.to_str().unwrap(),
            "--common-dir",
            common.to_str().unwrap(),
        ],
        true,
    )
    .await;
    assert_eq!(approved["revision"], 1);
    let shown = metadata_cli(binary, &db, &["show", "--project-id", p], true).await;
    assert_eq!(shown["grants"], json!([]));
    metadata_cli(binary, &db, &checked(), false).await;
    let grant = |revision, approval, native| {
        [
            "grant",
            "--project-id",
            p,
            "--user-id",
            u,
            "--expected-revision",
            revision,
            "--expected-approval-revision",
            approval,
            "--expected-native-grant-revision",
            native,
            "--permissions",
            "read",
        ]
    };
    metadata_cli(binary, &db, &grant("none", "1", "1"), true).await;
    let observed = metadata_cli(binary, &db, &checked(), true).await;
    assert_eq!(observed["stamp"]["grant"]["user_id"], u);
    assert_eq!(observed["stamp"]["grant"]["revision"], 1);
    assert_eq!(observed["execution_ready"], false);
    assert_eq!(observed["observed_state_only"], true);
    metadata_cli(
        binary,
        &db,
        &[
            "check",
            "--project-id",
            p,
            "--user-id",
            u,
            "--permission",
            "write",
        ],
        false,
    )
    .await;
    store.revoke_grant(p, u).unwrap();
    store.grant(p, u).unwrap();
    metadata_cli(binary, &db, &checked(), false).await;
    metadata_cli(binary, &db, &grant("1", "1", "3"), true).await;
    assert_eq!(
        metadata_cli(binary, &db, &checked(), true).await["stamp"]["grant"]["revision"],
        2
    );
    metadata_cli(
        binary,
        &db,
        &[
            "ungrant",
            "--project-id",
            p,
            "--user-id",
            u,
            "--expected-revision",
            "2",
        ],
        true,
    )
    .await;
    metadata_cli(binary, &db, &checked(), false).await;
    metadata_cli(binary, &db, &grant("3", "1", "3"), true).await;
    metadata_cli(
        binary,
        &db,
        &[
            "rebind",
            "--project-id",
            p,
            "--expected-revision",
            "1",
            "--git-dir",
            git_dir.to_str().unwrap(),
            "--common-dir",
            common.to_str().unwrap(),
        ],
        true,
    )
    .await;
    metadata_cli(binary, &db, &checked(), false).await;
    metadata_cli(binary, &db, &grant("4", "2", "3"), true).await;
    metadata_cli(binary, &db, &checked(), true).await;
    std::fs::rename(common.join("objects"), common.join("old-objects")).unwrap();
    std::fs::create_dir(common.join("objects")).unwrap();
    metadata_cli(binary, &db, &checked(), false).await;
    std::fs::remove_dir_all(&linked).unwrap();
    metadata_cli(
        binary,
        &db,
        &[
            "ungrant",
            "--project-id",
            p,
            "--user-id",
            u,
            "--expected-revision",
            "5",
        ],
        true,
    )
    .await;
    assert_eq!(
        metadata_cli(
            binary,
            &db,
            &["revoke", "--project-id", p, "--expected-revision", "2"],
            true
        )
        .await["active"],
        false
    );
    metadata_cli(
        binary,
        &db,
        &["show", "--project-id", p, "--unknown", "value"],
        false,
    )
    .await;
    let missing = directory.join("must-not-create.db");
    metadata_cli(binary, &missing, &["show", "--project-id", p], false).await;
    assert!(!missing.exists());
}

async fn production_worker_export_cli(binary: &std::ffi::OsStr, config: &Path, directory: &Path) {
    let state = directory.join("operator-private-state");
    let root = directory.join("PRIVATE_EXPORT_ROOT");
    std::fs::create_dir(&root).unwrap();
    let settings = "a".repeat(64);
    let export = export_cli(
        binary,
        config,
        &state,
        &[
            "register",
            "--name",
            "Controlled checkout",
            "--root",
            root.to_str().unwrap(),
            "--settings-digest",
            &settings,
        ],
        true,
    )
    .await;
    assert_eq!(export["repo_kind"], "directory");
    assert_eq!(export["revision"], 1);
    assert_eq!(export["active"], true);
    let id = export["id"].as_str().unwrap();
    let grant = export_cli(
        binary,
        config,
        &state,
        &[
            "grant",
            "--id",
            id,
            "--principal-kind",
            "human",
            "--principal-id",
            "operator-approved-user",
            "--permissions",
            "read,execute",
            "--expected-revision",
            "0",
        ],
        true,
    )
    .await;
    assert_eq!(grant["revision"], 1);
    let checked = export_cli(
        binary,
        config,
        &state,
        &[
            "check",
            "--id",
            id,
            "--principal-kind",
            "human",
            "--principal-id",
            "operator-approved-user",
            "--permission",
            "execute",
        ],
        true,
    )
    .await;
    assert_eq!(checked["checkout_identity_current"], true);
    assert_eq!(checked["grant_current"], true);
    assert_eq!(checked["execution_ready"], false);
    // Revocation survives an unavailable checkout and a new CLI/SQLite process.
    std::fs::remove_dir(&root).unwrap();
    let revoked = export_cli(
        binary,
        config,
        &state,
        &["revoke", "--id", id, "--expected-revision", "1"],
        true,
    )
    .await;
    assert_eq!(revoked["active"], false);
    export_cli(
        binary,
        config,
        &state,
        &[
            "check",
            "--id",
            id,
            "--principal-kind",
            "human",
            "--principal-id",
            "operator-approved-user",
            "--permission",
            "execute",
        ],
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires actual dunlin-relay executable and Linux bubblewrap sandbox; run the runtime sandbox suite"]
async fn production_browser_acceptance_runtime_restart() {
    tokio::time::timeout(Duration::from_secs(90), async {
        production_worker_cli_rejects_fifo_and_never_echoes_stdin().await;
        let f = Fixture::new();
        let mut first = ServerProcess::start(&f, "first");
        ready(&f, &mut first).await;
        let original_acceptance = create_with_key(&f, Some(CREATE_KEY)).await;
        assert_eq!(original_acceptance["admission"]["replayed"], false);
        assert_eq!(original_acceptance["admission"]["session_is_acceptance_snapshot"], true);
        let original_session = original_acceptance["session"].clone();
        assert_eq!(original_session["phase"], "active");
        let session = original_session["id"].as_str().unwrap().to_owned();
        let chat = format!("ahp-chat:/{session}");
        let mut socket = connect(&f).await;
        rpc(&mut socket, 1, "initialize", json!({"channel":"ahp-root://","clientId":CLIENT,"protocolVersions":["1.0.0"],"initialSubscriptions":["ahp-root://",chat]})).await;
        let first_action = prompt("accepted-before-crash", AMBIGUOUS);
        let accepted = dispatch(&mut socket, &chat, 1, first_action.clone()).await;
        assert!(accepted.get("rejectionReason").is_none(), "Actual actor unavailable: {accepted}; {}", first.diagnostics());
        wait_until(|| {
            if f.phase(&session) != SessionPhase::Active {
                let events = f.store.interactive_journal().replay(&f.context(), &session, 0).unwrap();
                panic!("Production actor stopped before checkpoint: {}; {}", serde_json::to_string(&events).unwrap(), first.diagnostics());
            }
            f.lines("performed-effects") == [AMBIGUOUS] && f.permission(&session).is_some()
        }, "actual sideeffect and pending permission").await;
        let old_permission = f.permission(&session).unwrap();
        let queued = dispatch(&mut socket, &chat, 2, prompt("queued-before-crash", QUEUED)).await;
        assert!(queued.get("rejectionReason").is_none());
        let before = f.turns(&session);
        assert_eq!(before.len(), 2);
        assert_eq!(before[0].phase, TurnPhase::Running);
        assert_eq!(before[1].phase, TurnPhase::Queued);
        assert_eq!(f.lines("received-prompts"), [AMBIGUOUS]);
        first.record_children();
        first.crash();
        drop(socket);
        wait_until(|| first.known.iter().all(|old| identity(old.pid).is_none_or(|now| now.start != old.start)), "production crash must terminate real harness descendants; cleanup has not run").await;
        let mut restarted = ServerProcess::start(&f, "restarted");
        ready(&f, &mut restarted).await;
        assert_eq!(f.phase(&session), SessionPhase::Interrupted);
        let recovered = f.turns(&session);
        assert_eq!(recovered[0].phase, TurnPhase::Unknown);
        assert_eq!(recovered[1].phase, TurnPhase::Queued);
        // The accepted HTTP receipt remains historical across an actual process
        // death. Retrying it must neither resurrect its actor nor refresh phase.
        let replayed_acceptance = create_with_key(&f, Some(CREATE_KEY)).await;
        assert_eq!(replayed_acceptance["admission"]["replayed"], true);
        assert_eq!(replayed_acceptance["admission"]["session_is_acceptance_snapshot"], true);
        assert_eq!(replayed_acceptance["session"], original_session);
        assert_eq!(f.phase(&session), SessionPhase::Interrupted);
        let database = rusqlite::Connection::open(&f.db).unwrap();
        let sessions: i64 = database.query_row("SELECT count(*) FROM interactive_sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(sessions, 1, "Restart retry created another durable session");
        assert_eq!(f.lines("peer-starts").len(), 1, "Restart retry spawned another ACP harness");
        let mut socket = connect(&f).await;
        let session_channel = format!("ahp-session:/{session}");
        let snapshot = rpc(&mut socket, 2, "reconnect", json!({"channel":"ahp-root://","clientId":CLIENT,"lastSeenServerSeq":accepted["serverSeq"],"subscriptions":["ahp-root://",session_channel,chat]})).await;
        assert_eq!(snapshot["type"], "snapshot");
        let snapshots = snapshot["snapshots"].as_array().unwrap();
        let current_session = &snapshots.iter().find(|s| s["resource"] == session_channel).unwrap()["state"];
        assert_eq!(current_session["_meta"]["dunlin.runtimePhase"], "interrupted", "Current AHP state must override historical HTTP acceptance");
        let recovered_chat = &snapshots.iter().find(|s| s["resource"] == chat).unwrap()["state"];
        assert!(recovered_chat["activeTurn"].is_null());
        assert!(recovered_chat.to_string().contains("unknown_outcome"));
        assert_eq!(recovered_chat["queuedMessages"][0]["message"]["text"], QUEUED);
        let repeated = dispatch(&mut socket, &chat, 1, first_action).await;
        assert_eq!(repeated, accepted, "Restart must re-emit durable acceptance without replaying the prompt");
        assert_eq!(f.turns(&session).len(), 2);
        let stale = dispatch(&mut socket, &chat, 3, old_permission).await;
        assert!(stale.get("rejectionReason").is_some());
        assert!(f.lines("received-decisions").is_empty());
        assert!(f.lines("permission-effects").is_empty());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(f.lines("received-prompts"), [AMBIGUOUS]);
        assert_eq!(f.lines("performed-effects"), [AMBIGUOUS]);
        assert_eq!(f.lines("peer-starts").len(), 1, "No actor may start asynchronously after receipt replay");
        assert_eq!(f.turns(&session)[1].phase, TurnPhase::Queued);
        let fresh = create(&f).await;
        let fresh_chat = format!("ahp-chat:/{fresh}");
        rpc(&mut socket, 3, "subscribe", json!({"channel":fresh_chat})).await;
        let fresh_echo = dispatch(&mut socket, &fresh_chat, 4, prompt("fresh-explicit-prompt", FRESH)).await;
        assert!(fresh_echo.get("rejectionReason").is_none());
        wait_until(|| f.turns(&fresh).iter().any(|t| t.phase == TurnPhase::Completed), "fresh explicit session must execute").await;
        assert_eq!(f.lines("performed-effects"), [AMBIGUOUS, FRESH]);
        assert_eq!(f.lines("received-prompts"), [AMBIGUOUS, FRESH]);
        assert_eq!(f.lines("peer-starts").len(), 2, "Only a fresh explicit create starts a new actor");
        assert_eq!(f.turns(&session)[0].phase, TurnPhase::Unknown);
        assert_eq!(f.turns(&session)[1].phase, TurnPhase::Queued);
        restarted.record_children();
        let _ = socket.close(None).await;
    }).await.expect("Production crash/relaunch acceptance exceeded 90 seconds");
}
