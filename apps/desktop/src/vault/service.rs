//! Local credential broker service and session-scoped MCP hand-off.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dunlin_domain::vault::{SecretAudit, SecretPolicy, now_secs};
use dunlin_vault::broker::{ApprovalRequest, Approver, Broker, SessionContext};
use dunlin_vault::http::UreqExecutor;
use dunlin_vault::protocol::{
    PROTOCOL_VERSION, WireRequest, WireResponse, new_token, read_frame, write_frame,
};
use dunlin_vault::recipe::CommandExecutor;
use dunlin_vault::store::{KeychainStore, SecretStore};

type SharedSessions = Arc<Mutex<HashMap<String, Arc<SessionRecord>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalDecision {
    Deny,
    Once,
    Session,
}

pub(crate) struct ApprovalJob {
    pub request: ApprovalRequest,
    pub reply: std::sync::mpsc::Sender<ApprovalDecision>,
}

/// Bridges the synchronous broker approver to the GPUI approval dialog.
pub(crate) struct UiApprover {
    sender: async_channel::Sender<ApprovalJob>,
    allowed: Mutex<HashSet<(String, String, i64)>>,
}

impl UiApprover {
    pub fn new(sender: async_channel::Sender<ApprovalJob>) -> Self {
        Self {
            sender,
            allowed: Mutex::new(HashSet::new()),
        }
    }
}

impl Approver for UiApprover {
    fn approve(&self, request: &ApprovalRequest) -> bool {
        let key = (
            request.session_id.clone().unwrap_or_default(),
            request.alias.clone(),
            request.revision,
        );
        if self
            .allowed
            .lock()
            .map(|allowed| allowed.contains(&key))
            .unwrap_or(false)
        {
            return true;
        }
        let (reply, answer) = std::sync::mpsc::channel();
        if self
            .sender
            .send_blocking(ApprovalJob {
                request: request.clone(),
                reply,
            })
            .is_err()
        {
            return false;
        }
        match answer.recv_timeout(Duration::from_secs(120)) {
            Ok(ApprovalDecision::Once) => true,
            Ok(ApprovalDecision::Session) => {
                if let Ok(mut allowed) = self.allowed.lock() {
                    allowed.insert(key);
                }
                true
            }
            _ => false,
        }
    }
}

pub struct SessionRecord {
    pub project_id: String,
    pub project_path: PathBuf,
    session_id: Mutex<Option<String>>,
    trusted: AtomicBool,
    read_only: AtomicBool,
    allowed: AtomicBool,
    revoked: AtomicBool,
    bound: AtomicBool,
}

#[derive(Clone)]
pub struct SessionHandle {
    pub token: String,
    record: Arc<SessionRecord>,
    sessions: SharedSessions,
}

impl SessionHandle {
    pub fn set_session_id(&self, session_id: &str) {
        if let Ok(mut current) = self.record.session_id.lock() {
            *current = Some(session_id.to_string());
        }
    }

    pub fn set_read_only(&self, read_only: bool) {
        self.record.read_only.store(read_only, Ordering::Relaxed);
    }

    pub fn set_allowed(&self, allowed: bool) {
        self.record.allowed.store(allowed, Ordering::Relaxed);
    }

    pub fn revoke(&self) {
        self.record.revoked.store(true, Ordering::Relaxed);
        self.record.allowed.store(false, Ordering::Relaxed);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&self.token);
        }
    }

    fn context(&self) -> SessionContext {
        SessionContext {
            project_id: self.record.project_id.clone(),
            project_path: self.record.project_path.clone(),
            session_id: self
                .record
                .session_id
                .lock()
                .ok()
                .and_then(|session| session.clone()),
            trusted: self.record.trusted.load(Ordering::Relaxed),
            read_only: self.record.read_only.load(Ordering::Relaxed),
            allowed: self.record.allowed.load(Ordering::Relaxed)
                && !self.record.revoked.load(Ordering::Relaxed),
        }
    }
}

pub struct ServiceHandle {
    socket: PathBuf,
    sessions: SharedSessions,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<Vec<JoinHandle<()>>>>,
}

impl ServiceHandle {
    pub fn start_with_approver(approver: Arc<dyn Approver>) -> Result<Self, String> {
        let data_dir = crate::app::data_dir()
            .ok_or_else(|| "Local credential storage is unavailable".to_string())?;
        Self::start_at_with_approver(data_dir, approver)
    }

    /// Start against an explicit data directory. Tests use this and a deny-all
    /// approver; the app uses [`start_with_approver`].
    #[cfg(test)]
    pub fn start_at(data_dir: PathBuf) -> Result<Self, String> {
        Self::start_at_with_approver(data_dir, Arc::new(dunlin_vault::broker::DenyAll))
    }

    fn start_at_with_approver(
        data_dir: PathBuf,
        approver: Arc<dyn Approver>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| format!("Could not create the credential directory: {error}"))?;
        let database =
            if data_dir.join("dunlin.db").exists() || !data_dir.join("kestrel.db").exists() {
                data_dir.join("dunlin.db")
            } else {
                data_dir.join("kestrel.db")
            };
        // Migrate once before any cleanup or client thread opens the same new
        // database. The app normally does this first, but tests and a fresh data
        // directory must not race two concurrent first-open migrations.
        dunlin_storage::Store::open(&database)
            .map_err(|error| format!("Could not open credential storage: {error}"))?;
        let mut socket = data_dir.join("vault.sock");
        // AF_UNIX paths are short on macOS; fall back to the private per-user
        // temporary directory if the Application Support path is too long.
        if socket.as_os_str().as_encoded_bytes().len() > 100 {
            socket = std::env::temp_dir().join(format!("dunlin-vault-{}.sock", std::process::id()));
        }
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Could not replace the credential socket: {error}"));
            }
        }
        #[cfg(unix)]
        let listener = {
            use std::os::unix::fs::PermissionsExt;
            let listener = std::os::unix::net::UnixListener::bind(&socket)
                .map_err(|error| format!("Could not start the credential broker: {error}"))?;
            let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("Could not configure the credential broker: {error}"))?;
            listener
        };
        #[cfg(not(unix))]
        {
            return Err("The credential broker requires Unix sockets".into());
        }
        let sessions: SharedSessions = Arc::new(Mutex::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread_sessions = sessions.clone();
        let thread_socket = socket.clone();
        let thread_database = database.clone();
        let join = thread::Builder::new()
            .name("dunlin-vault".into())
            .spawn(move || {
                let mut workers = Vec::new();
                let secrets: Arc<dyn SecretStore> = Arc::new(KeychainStore);
                let http = Arc::new(UreqExecutor::new());
                let recipes = Arc::new(CommandExecutor);
                // Expiry cleanup must not block the handshake loop. Keychain
                // access can wait for an unlock or a system prompt.
                let cleanup_secrets = secrets.clone();
                let cleanup_database = thread_database.clone();
                let cleanup_shutdown = thread_shutdown.clone();
                if let Ok(cleanup) = thread::Builder::new()
                    .name("dunlin-vault-cleanup".into())
                    .spawn(move || {
                        let mut last_cleanup = Instant::now() - Duration::from_secs(61);
                        while !cleanup_shutdown.load(Ordering::Relaxed) {
                            if last_cleanup.elapsed() >= Duration::from_secs(60) {
                                cleanup_expired(&cleanup_secrets, &cleanup_database);
                                last_cleanup = Instant::now();
                            }
                            thread::sleep(Duration::from_secs(1));
                        }
                    })
                {
                    workers.push(cleanup);
                }
                while !thread_shutdown.load(Ordering::Relaxed) {
                    workers.retain(|worker| !worker.is_finished());
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let sessions = thread_sessions.clone();
                            let secrets = secrets.clone();
                            let http = http.clone();
                            let recipes = recipes.clone();
                            let approver = approver.clone();
                            let database = thread_database.clone();
                            if let Ok(client) = thread::Builder::new()
                                .name("dunlin-vault-client".into())
                                .spawn(move || {
                                    handle_connection(
                                        stream, sessions, secrets, http, recipes, approver,
                                        database,
                                    );
                                })
                            {
                                workers.push(client);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40));
                        }
                        Err(_) => break,
                    }
                }
                let _ = std::fs::remove_file(&thread_socket);
                workers
            })
            .map_err(|error| format!("Could not start the credential broker: {error}"))?;
        Ok(Self {
            socket,
            sessions,
            shutdown,
            join: Some(join),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Reset must drain writers before deleting records; call off the UI thread.
    pub(crate) fn shutdown_and_wait(mut self) -> Result<(), String> {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let workers = join
                .join()
                .map_err(|_| "Credential broker shutdown failed")?;
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| "Credential request shutdown failed")?;
            }
        }
        Ok(())
    }

    pub fn new_session(
        &self,
        project_id: impl Into<String>,
        project_path: impl Into<PathBuf>,
        trusted: bool,
        read_only: bool,
    ) -> SessionHandle {
        let token = new_token();
        let record = Arc::new(SessionRecord {
            project_id: project_id.into(),
            project_path: project_path.into(),
            session_id: Mutex::new(None),
            trusted: AtomicBool::new(trusted),
            read_only: AtomicBool::new(read_only),
            allowed: AtomicBool::new(true),
            revoked: AtomicBool::new(false),
            bound: AtomicBool::new(false),
        });
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(token.clone(), record.clone());
        }
        SessionHandle {
            token,
            record,
            sessions: self.sessions.clone(),
        }
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(unix)]
fn handle_connection(
    mut stream: std::os::unix::net::UnixStream,
    sessions: SharedSessions,
    secrets: Arc<dyn SecretStore>,
    http: Arc<UreqExecutor>,
    recipes: Arc<CommandExecutor>,
    approver: Arc<dyn Approver>,
    database: PathBuf,
) {
    use std::os::unix::io::AsRawFd;

    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let hello = match read_frame::<WireRequest>(&mut stream) {
        Ok(Some(WireRequest::Hello {
            protocol,
            token,
            pid,
        })) => (protocol, token, pid),
        _ => {
            let _ = write_frame(
                &mut stream,
                &WireResponse::Hello {
                    ok: false,
                    message: Some("The credential handshake was invalid".into()),
                },
            );
            return;
        }
    };
    let (protocol, token, claimed_pid) = hello;
    if protocol != PROTOCOL_VERSION {
        let _ = write_frame(
            &mut stream,
            &WireResponse::Hello {
                ok: false,
                message: Some("The credential protocol version is unsupported".into()),
            },
        );
        return;
    }
    if let Some(peer) = peer_pid(stream.as_raw_fd())
        && peer != claimed_pid
    {
        let _ = write_frame(
            &mut stream,
            &WireResponse::Hello {
                ok: false,
                message: Some("The credential connection did not match its process".into()),
            },
        );
        return;
    }
    let record = sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(&token).cloned());
    let Some(record) = record else {
        let _ = write_frame(
            &mut stream,
            &WireResponse::Hello {
                ok: false,
                message: Some("The credential session is not valid".into()),
            },
        );
        return;
    };
    if record.revoked.load(Ordering::Relaxed)
        || record
            .bound
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        let _ = write_frame(
            &mut stream,
            &WireResponse::Hello {
                ok: false,
                message: Some("The credential session is already connected or closed".into()),
            },
        );
        return;
    }
    if write_frame(
        &mut stream,
        &WireResponse::Hello {
            ok: true,
            message: None,
        },
    )
    .is_err()
    {
        return;
    }
    // MCP tools are often idle for minutes between turns. Keep the session
    // connection open until the proxy closes it or the process exits.
    let _ = stream.set_read_timeout(None);
    let handle = SessionHandle {
        token: token.clone(),
        record: record.clone(),
        sessions: sessions.clone(),
    };
    loop {
        let request = match read_frame::<WireRequest>(&mut stream) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(_) => break,
        };
        let response = match request {
            WireRequest::Hello { .. } => WireResponse::Error {
                message: "The credential connection is already initialized".into(),
            },
            request => {
                let store = match dunlin_storage::Store::open(&database) {
                    Ok(store) => store,
                    Err(error) => {
                        let _ = write_frame(
                            &mut stream,
                            &WireResponse::Error {
                                message: format!("Credential storage is unavailable: {error}"),
                            },
                        );
                        continue;
                    }
                };
                let context = handle.context();
                let broker = Broker {
                    store: &store,
                    secrets: secrets.as_ref(),
                    http: http.as_ref(),
                    recipes: recipes.as_ref(),
                    approver: approver.as_ref(),
                    session: &context,
                };
                match request {
                    WireRequest::List => match broker.list() {
                        Ok(secrets) => WireResponse::List { secrets },
                        Err(error) => WireResponse::Error {
                            message: error.message().into(),
                        },
                    },
                    WireRequest::Http(input) => match broker.http(input) {
                        Ok(output) => WireResponse::Http { output },
                        Err(error) => WireResponse::Error {
                            message: error.message().into(),
                        },
                    },
                    WireRequest::Run(input) => match broker.run(input) {
                        Ok(output) => WireResponse::Run { output },
                        Err(error) => WireResponse::Error {
                            message: error.message().into(),
                        },
                    },
                    WireRequest::Hello { .. } => unreachable!(),
                }
            }
        };
        if write_frame(&mut stream, &response).is_err() {
            break;
        }
    }
    record.revoked.store(true, Ordering::Relaxed);
    record.allowed.store(false, Ordering::Relaxed);
    if let Ok(mut sessions) = sessions.lock() {
        sessions.remove(&token);
    }
}

#[cfg(unix)]
fn peer_pid(fd: std::os::unix::io::RawFd) -> Option<u32> {
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 2;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut length,
        )
    };
    (result == 0 && pid > 0).then_some(pid as u32)
}

fn cleanup_expired(secrets: &Arc<dyn SecretStore>, database: &Path) {
    let Ok(store) = dunlin_storage::Store::open(database) else {
        return;
    };
    let now = now_secs();
    let Ok(secrets_list) = store.secrets() else {
        return;
    };
    for secret in secrets_list.into_iter().take(256) {
        let Ok(raw) = secrets.get_policy(&secret.id) else {
            continue;
        };
        let Ok(policy) = serde_json::from_str::<SecretPolicy>(&raw) else {
            continue;
        };
        if !policy
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            continue;
        }
        if secrets.delete(&secret.id).is_err() {
            continue;
        }
        let _ = store.record_secret_audit(&SecretAudit {
            id: 0,
            secret_id: Some(secret.id.clone()),
            alias: secret.alias.clone(),
            project_id: None,
            session_id: None,
            operation: "expiry".into(),
            detail: "credential removed after its TTL".into(),
            decision: "expired".into(),
            status: None,
            error: None,
            created_at: now,
        });
        let _ = secrets.delete_policy(&secret.id);
        let _ = store.delete_secret(&secret.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval_request() -> ApprovalRequest {
        ApprovalRequest {
            alias: "github-work".into(),
            label: "GitHub".into(),
            project_id: "project".into(),
            session_id: Some("session".into()),
            operation: "http".into(),
            detail: "GET api.github.com/user".into(),
            revision: 1,
        }
    }

    #[test]
    fn ui_approver_supports_once_session_and_deny() {
        let (tx, rx) = async_channel::unbounded();
        let approver = UiApprover::new(tx);
        let request = approval_request();
        let worker = std::thread::spawn(move || {
            let job = rx.recv_blocking().unwrap();
            job.reply.send(ApprovalDecision::Once).unwrap();
        });
        assert!(approver.approve(&request));
        worker.join().unwrap();

        let (tx, rx) = async_channel::unbounded();
        let approver = UiApprover::new(tx);
        let worker = std::thread::spawn(move || {
            let job = rx.recv_blocking().unwrap();
            job.reply.send(ApprovalDecision::Deny).unwrap();
        });
        assert!(!approver.approve(&request));
        worker.join().unwrap();

        let (tx, rx) = async_channel::unbounded();
        let worker_rx = rx.clone();
        let approver = UiApprover::new(tx);
        let worker = std::thread::spawn(move || {
            let job = worker_rx.recv_blocking().unwrap();
            job.reply.send(ApprovalDecision::Session).unwrap();
        });
        assert!(approver.approve(&request));
        worker.join().unwrap();
        assert!(approver.approve(&request));
        assert!(rx.try_recv().is_err());

        let mut edited = request;
        edited.revision += 1;
        let worker = std::thread::spawn(move || {
            let job = rx.recv_blocking().unwrap();
            job.reply.send(ApprovalDecision::Deny).unwrap();
        });
        assert!(!approver.approve(&edited));
        worker.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn service_binds_a_token_to_the_first_connection_and_lists_safely() {
        let temp = tempfile::tempdir().unwrap();
        let service = super::ServiceHandle::start_at(temp.path().to_path_buf()).unwrap();
        let session = service.new_session("project", temp.path(), true, false);
        let socket = service.socket_path().to_path_buf();
        let token = session.token.clone();
        let client = std::thread::spawn(move || {
            let mut client = dunlin_vault::client::VaultClient::connect(&socket, &token).unwrap();
            client.list().unwrap()
        });
        assert!(client.join().unwrap().is_empty());
        // The token is single-use: a second connection after the first is bound
        // must not be accepted.
        assert!(
            dunlin_vault::client::VaultClient::connect(service.socket_path(), &session.token)
                .is_err()
        );
        let socket = service.socket_path().to_path_buf();
        service.shutdown_and_wait().unwrap();
        assert!(!socket.exists());
    }
}
