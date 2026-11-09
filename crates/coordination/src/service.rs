//! Session-capability Unix broker. Tokens are memory-only, rechecked on every
//! call, and cannot choose another project/user. The proxy never opens SQLite.
use crate::*;
use rand::Rng;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::{
    fs::{DirBuilderExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub type SharedStore = Arc<Mutex<Store>>;
const MAX_FRAME: usize = 256 * 1024;
#[derive(Clone)]
struct Grant {
    scope: Scope,
    session: String,
    root: String,
    allowed: bool,
    authorize: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
type Grants = Arc<Mutex<HashMap<String, Grant>>>;

pub struct SessionHandle {
    pub token: String,
    grants: Grants,
}
impl SessionHandle {
    pub fn scope(&self) -> Option<Scope> {
        self.grants
            .lock()
            .ok()?
            .get(&self.token)
            .map(|g| g.scope.clone())
    }
    pub fn bind_session(&self, session: &str) {
        if let Ok(mut grants) = self.grants.lock()
            && let Some(g) = grants.get_mut(&self.token)
        {
            g.session = session.into();
        }
    }
    pub fn set_allowed(&self, allowed: bool) {
        if let Ok(mut grants) = self.grants.lock()
            && let Some(g) = grants.get_mut(&self.token)
        {
            g.allowed = allowed;
        }
    }
    pub fn revoke(&self) {
        if let Ok(mut g) = self.grants.lock() {
            g.remove(&self.token);
        }
    }
}
impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub struct Service {
    pub store: SharedStore,
    socket: PathBuf,
    grants: Grants,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _lock: std::fs::File,
}
impl Service {
    pub fn start(directory: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        if !std::fs::symlink_metadata(directory)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("Coordination storage must be a real directory, not a symlink".into());
        }
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("coordination.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|_| "Another coordinator owns this data directory".to_string())?;
        let mut store = Store::open(&directory.join("coordination.sqlite"))?;
        store.recover()?;
        let store = Arc::new(Mutex::new(store));
        // Short socket path also works when the user's data directory is long.
        let runtime = runtime_directory(&std::env::temp_dir())?;
        let socket = runtime.join("broker.sock");
        let listener = UnixListener::bind(&socket).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let grants = Arc::new(Mutex::new(HashMap::<String, Grant>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (db, capabilities, stop) = (store.clone(), grants.clone(), shutdown.clone());
        let worker = thread::spawn(move || {
            let mut workers: Vec<JoinHandle<()>> = Vec::new();
            while !stop.load(Ordering::Acquire) {
                let mut i = 0;
                while i < workers.len() {
                    if workers[i].is_finished() {
                        let _ = workers.swap_remove(i).join();
                    } else {
                        i += 1;
                    }
                }
                match listener.accept() {
                    Ok((mut stream, _)) if workers.len() < 16 => {
                        let (db, capabilities) = (db.clone(), capabilities.clone());
                        workers.push(thread::spawn(move || {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                            let response = (|| {
                                let request: Request = read_frame(&mut stream)?;
                                let grants = capabilities
                                    .lock()
                                    .map_err(|_| "Capability lock unavailable")?;
                                let grant = grants
                                    .get(&request.token)
                                    .filter(|g| {
                                        g.allowed
                                            && g.authorize.as_ref().is_none_or(|check| check())
                                    })
                                    .ok_or("Session capability is revoked or paused")?;
                                // Hold the grant lock through commit: revocation and use are ordered.
                                let mut db = db
                                    .lock()
                                    .map_err(|_| "Coordination store lock unavailable")?;
                                call(&mut db, grant, &request.name, request.arguments)
                            })();
                            let value = match response {
                                Ok(result) => json!({"result":result}),
                                Err(error) => json!({"error":error}),
                            };
                            let value=if serde_json::to_vec(&value).is_ok_and(|v|v.len()<=MAX_FRAME){value}else{json!({"error":"Coordination response exceeds limit; paginate results"})};
                            let _ = write_frame(&mut stream, &value);
                        }));
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::park_timeout(Duration::from_millis(25))
                    }
                    Err(_) => break,
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });
        Ok(Self {
            store,
            socket,
            grants,
            shutdown,
            worker: Some(worker),
            _lock: lock,
        })
    }
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    pub fn grant(
        &self,
        scope: Scope,
        session: String,
        root: String,
    ) -> Result<SessionHandle, String> {
        self.grant_guarded(scope, session, root, None)
    }
    pub fn grant_guarded(
        &self,
        scope: Scope,
        session: String,
        root: String,
        authorize: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<SessionHandle, String> {
        let token = random_token();
        self.grants
            .lock()
            .map_err(|_| "Capability lock unavailable")?
            .insert(
                token.clone(),
                Grant {
                    scope,
                    session,
                    root,
                    allowed: true,
                    authorize,
                },
            );
        Ok(SessionHandle {
            token,
            grants: self.grants.clone(),
        })
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Ok(mut g) = self.grants.lock() {
            g.clear();
        }
        self.shutdown.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            w.thread().unpark();
            let _ = w.join();
        }
        let _ = std::fs::remove_file(&self.socket);
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn runtime_directory(preferred: &Path) -> Result<PathBuf, String> {
    let name = format!("dunlin-coord-{}", &random_token()[..16]);
    let mut directory = preferred.join(&name);
    if directory
        .join("broker.sock")
        .as_os_str()
        .as_encoded_bytes()
        .len()
        > 100
    {
        directory = Path::new("/tmp").join(name);
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    Ok(directory)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    name: String,
    arguments: Value,
}
pub fn request(socket: &Path, token: &str, name: &str, arguments: Value) -> Result<Value, String> {
    let mut stream =
        UnixStream::connect(socket).map_err(|e| format!("Coordinator unavailable: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    write_frame(
        &mut stream,
        &Request {
            token: token.into(),
            name: name.into(),
            arguments,
        },
    )?;
    let response: Value = read_frame(&mut stream)?;
    if let Some(error) = response.get("error").and_then(Value::as_str) {
        Err(error.into())
    } else {
        response
            .get("result")
            .cloned()
            .ok_or("Invalid broker response".into())
    }
}
fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut impl Read) -> Result<T, String> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).map_err(|e| e.to_string())?;
    let n = u32::from_be_bytes(length) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err("Coordination frame exceeds limit".into());
    }
    let mut buf = vec![0; n];
    stream.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}
fn write_frame(stream: &mut impl Write, v: &impl Serialize) -> Result<(), String> {
    let data = serde_json::to_vec(v).map_err(|e| e.to_string())?;
    if data.len() > MAX_FRAME {
        return Err("Coordination response exceeds limit; paginate results".into());
    }
    stream
        .write_all(&(data.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(&data))
        .and_then(|_| stream.flush())
        .map_err(|e| e.to_string())
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default)]
    offset: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    prompt: String,
    idempotency_key: String,
    #[serde(default)]
    dependencies: Option<Vec<Dependency>>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    3600
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendMessage {
    to: String,
    body: String,
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunReport {
    report: Report,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskCreate {
    title: String,
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskReport {
    id: String,
    succeeded: bool,
    report: Report,
}
fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("Invalid arguments: {e}"))
}
fn value(v: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn call(db: &mut Store, g: &Grant, name: &str, args: Value) -> Result<Value, String> {
    let active = |l: &Ledger| -> Result<Run, String> {
        l.current_run(&g.scope, &g.session)
            .filter(|r| r.status.active() && r.execution.root == g.root)
            .cloned()
            .ok_or("No observed active run for this capability".into())
    };
    match name {
        "agents_list" => {
            let page: Page = parse(args)?;
            let mut seen = std::collections::HashSet::new();
            let rows: Vec<_> = db
                .ledger()
                .runs
                .iter()
                .rev()
                .filter(|r| r.scope == g.scope && seen.insert(&r.session))
                .collect();
            let count = rows.len();
            let rows: Vec<_> = rows.into_iter().skip(page.offset).take(32).map(|r| json!({"session":r.session,"run_id":r.id,"title":r.title,"execution":r.execution,"status":r.status,"queued_prompts":r.queued_prompts,"updated_ms":r.updated_ms,"source":"observed_acp","has_report":r.report.is_some()})).collect();
            Ok(
                json!({"agents":rows,"next_offset":(page.offset.saturating_add(32)<count).then_some(page.offset.saturating_add(32)),"self_session":g.session,"coverage":"Only sessions observed by this coordinator in this authorized project. External agents and opaque harness children are unknown. turn_finished is NOT verified task success."}),
            )
        }
        "runs_get" => {
            let id: Id = parse(args)?;
            value(
                db.ledger()
                    .runs
                    .iter()
                    .find(|r| r.id == id.id && r.scope == g.scope)
                    .ok_or("Run unavailable in this project")?,
            )
        }
        "continuations_create" => {
            let input: Create = parse(args)?;
            text(&input.prompt, MAX_TEXT)?;
            text(&input.idempotency_key, 128)?;
            if !(1..=86400).contains(&input.timeout_seconds) {
                return Err("Timeout must be 1–86,400 seconds".into());
            }
            db.mutate(|l| {
                let run = active(l)?;
                if let Some(c) = l.continuations.iter().find(|c| c.scope == g.scope && c.session == g.session && c.key == input.idempotency_key) {
                    if c.prompt != input.prompt || input.dependencies.as_ref().is_some_and(|d| d != &c.dependencies) { return Err("Idempotency key reused with different content".into()); }
                    return value(c);
                }
                if l.continuations.iter().any(|c| c.scope == g.scope && c.session == g.session && matches!(c.status, ContinuationStatus::PendingApproval | ContinuationStatus::Waiting | ContinuationStatus::Dispatching)) { return Err("Resolve the existing continuation before creating another for this session".into()); }
                let dependencies = input.dependencies.unwrap_or_else(|| l.runs.iter().filter(|r| r.scope == g.scope && r.session != g.session && r.status.active()).map(|r| Dependency::Run(r.id.clone())).collect());
                l.check_dependencies(&g.scope, &g.session, &dependencies)?;
                let now = now_ms();
                let c = Continuation { id: new_id(), scope: g.scope.clone(), session: g.session.clone(), requester_run: run.id,
                    execution: run.execution, prompt: input.prompt, dependencies, status: ContinuationStatus::PendingApproval,
                    created_ms: now, deadline_ms: now + input.timeout_seconds * 1000, approved_ms: None, reason: None, dispatched_run: None, executor_device: None, executor_connection: None, key: input.idempotency_key };
                l.audit(&g.scope, &g.session, "continuation_proposed", &c.id);
                let result = value(&c)?; l.continuations.push(c); Ok(result)
            })
        }
        "continuations_list" => {
            let page: Page = parse(args)?;
            value(
                db.ledger()
                    .continuations
                    .iter()
                    .rev()
                    .filter(|c| c.scope == g.scope && c.session == g.session)
                    .skip(page.offset)
                    .take(8)
                    .collect::<Vec<_>>(),
            )
        }
        "continuations_cancel" => {
            let id: Id = parse(args)?;
            if !db
                .ledger()
                .continuations
                .iter()
                .any(|c| c.id == id.id && c.scope == g.scope && c.session == g.session)
            {
                return Err("Continuation unavailable".into());
            }
            db.set_continuation(
                &id.id,
                ContinuationStatus::Cancelled,
                Some("Cancelled by owner session".into()),
            )?;
            Ok(json!({"cancelled":true}))
        }
        "messages_send" => {
            let input: SendMessage = parse(args)?;
            text(&input.body, MAX_TEXT)?;
            text(&input.idempotency_key, 128)?;
            db.mutate(|l| {
                active(l)?;
                if input.to == g.session || l.current_run(&g.scope, &input.to).is_none() {
                    return Err("Recipient unavailable in this project".into());
                }
                if let Some(m) = l.messages.iter().find(|m| {
                    m.scope == g.scope && m.from == g.session && m.key == input.idempotency_key
                }) {
                    if m.to != input.to || m.body != input.body {
                        return Err("Idempotency key reused with different content".into());
                    }
                    return value(m);
                }
                let m = Message {
                    id: new_id(),
                    scope: g.scope.clone(),
                    from: g.session.clone(),
                    to: input.to,
                    body: input.body,
                    created_ms: now_ms(),
                    read_ms: None,
                    key: input.idempotency_key,
                };
                l.audit(&g.scope, &g.session, "message_sent", &m.id);
                let result = value(&m)?;
                l.messages.push(m);
                Ok(result)
            })
        }
        "inbox_read" => {
            let page: Page = parse(args)?;
            value(
                db.ledger()
                    .messages
                    .iter()
                    .rev()
                    .filter(|m| m.scope == g.scope && m.to == g.session)
                    .skip(page.offset)
                    .take(8)
                    .collect::<Vec<_>>(),
            )
        }
        "inbox_ack" => {
            let id: Id = parse(args)?;
            db.mutate(|l| {
                let m = l
                    .messages
                    .iter_mut()
                    .find(|m| m.id == id.id && m.scope == g.scope && m.to == g.session)
                    .ok_or("Message unavailable")?;
                m.read_ms = Some(now_ms());
                Ok(json!({"acknowledged":true}))
            })
        }
        "runs_report" => {
            let input: RunReport = parse(args)?;
            input.report.validate()?;
            db.mutate(|l| {
                let run = active(l)?;
                l.runs.iter_mut().find(|r| r.id == run.id).unwrap().report = Some(input.report);
                l.audit(&g.scope, &g.session, "run_reported", &run.id);
                Ok(json!({"recorded":true,"verified":false}))
            })
        }
        "tasks_create" => {
            let input: TaskCreate = parse(args)?;
            text(&input.title, 512)?;
            text(&input.idempotency_key, 128)?;
            db.mutate(|l| {
                let run = active(l)?;
                if let Some(t) = l.tasks.iter().find(|t| {
                    t.scope == g.scope && t.owner == g.session && t.key == input.idempotency_key
                }) {
                    if t.title != input.title {
                        return Err("Idempotency key reused with different content".into());
                    }
                    return value(t);
                }
                let t = Task {
                    id: new_id(),
                    scope: g.scope.clone(),
                    owner: g.session.clone(),
                    title: input.title,
                    runs: vec![run.id],
                    status: TaskStatus::Open,
                    report: None,
                    key: input.idempotency_key,
                };
                l.audit(&g.scope, &g.session, "task_created", &t.id);
                let result = value(&t)?;
                l.tasks.push(t);
                Ok(result)
            })
        }
        "tasks_list" => {
            let page: Page = parse(args)?;
            value(db.ledger().tasks.iter().rev().filter(|t| t.scope == g.scope).skip(page.offset).take(32).map(|t| json!({"id":t.id,"title":t.title,"owner":t.owner,"runs":t.runs,"status":t.status,"has_report":t.report.is_some()})).collect::<Vec<_>>())
        }
        "tasks_get" => {
            let id: Id = parse(args)?;
            value(
                db.ledger()
                    .tasks
                    .iter()
                    .find(|t| t.id == id.id && t.scope == g.scope)
                    .ok_or("Task unavailable in this project")?,
            )
        }
        "tasks_report" => {
            let input: TaskReport = parse(args)?;
            input.report.validate()?;
            db.mutate(|l| {
                let run = active(l)?;
                let task = l
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == input.id && t.scope == g.scope && t.owner == g.session)
                    .ok_or("Task unavailable to this owner")?;
                if task.status != TaskStatus::Open {
                    return Err("Task already has a terminal report".into());
                }
                if !task.runs.contains(&run.id) {
                    task.runs.push(run.id);
                }
                task.status = if input.succeeded && input.report.blockers.is_empty() {
                    TaskStatus::ReportedSucceeded
                } else {
                    TaskStatus::ReportedFailed
                };
                task.report = Some(input.report);
                l.audit(&g.scope, &g.session, "task_reported", &input.id);
                Ok(json!({"recorded":true,"verified":false}))
            })
        }
        _ => Err("Unknown coordination tool".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_temporary_roots_get_a_private_short_socket_directory() {
        let directory =
            runtime_directory(&Path::new("/nonexistent-long-root").join("x".repeat(120))).unwrap();
        assert!(
            directory
                .join("broker.sock")
                .as_os_str()
                .as_encoded_bytes()
                .len()
                <= 100
        );
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let socket = directory.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        drop(listener);
        std::fs::remove_file(socket).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    fn run(session: &str, scope: &Scope) -> Run {
        Run {
            id: new_id(),
            session: session.into(),
            scope: scope.clone(),
            execution: Execution {
                root: "/repo".into(),
                harness: "mock".into(),
                model: "model".into(),
                mode: "plan".into(),
                permission: "ask".into(),
                configuration: "digest".into(),
            },
            title: session.into(),
            status: RunStatus::Running,
            queued_prompts: Some(0),
            created_ms: now_ms(),
            updated_ms: now_ms(),
            stop_reason: None,
            report: None,
            continuation: None,
        }
    }
    #[test]
    fn snapshot_is_atomic_scoped_idempotent_and_excludes_self() {
        let mut db = Store::memory().unwrap();
        let scope = Scope {
            authority: "local".into(),
            project: "p".into(),
        };
        let a = run("a", &scope);
        let b = run("b", &scope);
        let c = run("c", &scope);
        for r in [&a, &b, &c] {
            db.start_run(r.clone()).unwrap();
        }
        let g = Grant {
            scope: scope.clone(),
            session: "c".into(),
            root: "/repo".into(),
            allowed: true,
            authorize: None,
        };
        let args = json!({"prompt":"Implement after peers","idempotency_key":"once"});
        let result = call(&mut db, &g, "continuations_create", args.clone()).unwrap();
        let c: Continuation = serde_json::from_value(result.clone()).unwrap();
        assert_eq!(
            c.dependencies,
            vec![Dependency::Run(a.id.clone()), Dependency::Run(b.id.clone())]
        );
        assert_eq!(c.status, ContinuationStatus::PendingApproval);
        db.start_run(run("late", &scope)).unwrap();
        assert_eq!(
            call(&mut db, &g, "continuations_create", args).unwrap(),
            result
        );
        assert!(
            call(
                &mut db,
                &g,
                "runs_get",
                json!({"id":a.id,"project":"other"})
            )
            .is_err()
        );
        let other = Grant {
            scope: Scope {
                authority: "other-user".into(),
                project: "p".into(),
            },
            ..g.clone()
        };
        assert!(call(&mut db, &other, "runs_get", json!({"id":a.id})).is_err());
    }
    #[test]
    fn success_barrier_and_failure_are_not_confused() {
        for reason in ["end_turn", "cancelled", "refusal", "max_tokens", "error"] {
            let mut db = Store::memory().unwrap();
            let scope = Scope {
                authority: "local".into(),
                project: "p".into(),
            };
            let a = run("a", &scope);
            let c = run("c", &scope);
            db.start_run(a.clone()).unwrap();
            db.start_run(c.clone()).unwrap();
            let g = Grant {
                scope: scope.clone(),
                session: "c".into(),
                root: "/repo".into(),
                allowed: true,
                authorize: None,
            };
            let result = call(
                &mut db,
                &g,
                "continuations_create",
                json!({"prompt":"next","idempotency_key":"once"}),
            )
            .unwrap();
            let proposed: Continuation = serde_json::from_value(result).unwrap();
            db.approve(&proposed.id, &c.execution, now_ms() + 60_000)
                .unwrap();
            let continuation = db.ledger().continuations[0].clone();
            assert!(!db.ledger().readiness(&continuation, now_ms()).unwrap());
            db.finish_run(&c.id, RunStatus::TurnFinished, "end_turn", Some(0), None)
                .unwrap();
            db.finish_run(&a.id, RunStatus::from_stop(reason), reason, Some(0), None)
                .unwrap();
            assert_eq!(
                db.ledger().readiness(&continuation, now_ms()).is_ok(),
                reason == "end_turn"
            );
        }
    }
    #[test]
    fn broker_revokes_capabilities_and_survives_restart_without_replay() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::start(dir.path()).unwrap();
        assert!(Service::start(dir.path()).is_err());
        let h = service
            .grant(
                Scope {
                    authority: "local".into(),
                    project: "p".into(),
                },
                "s".into(),
                "/repo".into(),
            )
            .unwrap();
        assert!(request(service.socket(), &h.token, "agents_list", json!({})).is_ok());
        h.set_allowed(false);
        assert!(request(service.socket(), &h.token, "agents_list", json!({})).is_err());
        h.set_allowed(true);
        h.revoke();
        assert!(request(service.socket(), &h.token, "agents_list", json!({})).is_err());
        drop(service);
        assert!(Service::start(dir.path()).is_ok());
    }
    #[test]
    fn messages_dedupe_and_do_not_dispatch_anything() {
        let mut db = Store::memory().unwrap();
        let scope = Scope {
            authority: "local".into(),
            project: "p".into(),
        };
        db.start_run(run("a", &scope)).unwrap();
        db.start_run(run("b", &scope)).unwrap();
        let a = Grant {
            scope: scope.clone(),
            session: "a".into(),
            root: "/repo".into(),
            allowed: true,
            authorize: None,
        };
        let b = Grant {
            session: "b".into(),
            ..a.clone()
        };
        let args = json!({"to":"b","body":"Review result","idempotency_key":"one"});
        let sent = call(&mut db, &a, "messages_send", args.clone()).unwrap();
        assert_eq!(sent, call(&mut db, &a, "messages_send", args).unwrap());
        assert!(
            call(&mut db, &a, "inbox_read", json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            call(&mut db, &b, "inbox_read", json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(db.ledger().continuations.is_empty());
    }
}
