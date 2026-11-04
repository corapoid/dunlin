//! Relay-side use-only credential broker.
//!
//! The relay host owns a separate vault database and Keychain namespace. A
//! per-ACP-session stdio MCP proxy is injected into `session/new`; the harness
//! starts it and talks to this local Unix socket. The proxy never receives the
//! credential value.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dunlin_domain::vault::{SecretAudit, SecretPolicy, now_secs};
use dunlin_storage::Store;
use dunlin_vault::broker::{Broker, DenyAll, SessionContext};
use dunlin_vault::http::UreqExecutor;
use dunlin_vault::protocol::{
    PROTOCOL_VERSION, WireRequest, WireResponse, new_token, read_frame, write_frame,
};
use dunlin_vault::recipe::CommandExecutor;
use dunlin_vault::store::{SecretStore, ServiceKeychainStore};

type SharedSessions = Arc<Mutex<HashMap<String, Arc<Session>>>>;

struct Session {
    project_id: String,
    project_path: PathBuf,
    session_id: Mutex<Option<String>>,
    read_only: AtomicBool,
    allowed: AtomicBool,
    revoked: AtomicBool,
    bound: AtomicBool,
}

#[derive(Clone)]
pub struct VaultSession {
    pub token: String,
    record: Arc<Session>,
    sessions: SharedSessions,
}

impl VaultSession {
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
            trusted: true,
            read_only: self.record.read_only.load(Ordering::Relaxed),
            allowed: self.record.allowed.load(Ordering::Relaxed)
                && !self.record.revoked.load(Ordering::Relaxed),
        }
    }
}

pub struct RelayVault {
    socket: PathBuf,
    sessions: SharedSessions,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

pub fn vault_database(relay_db: &Path) -> PathBuf {
    relay_db.with_file_name("vault.db")
}

impl RelayVault {
    pub fn start(relay_db: &Path) -> Result<Arc<Self>, String> {
        let database = vault_database(relay_db);
        Store::open(&database)
            .map_err(|error| format!("Could not open relay vault storage: {error}"))?;
        let socket =
            std::env::temp_dir().join(format!("dunlin-relay-vault-{}.sock", std::process::id()));
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not replace the relay vault socket: {error}")),
        }
        #[cfg(unix)]
        let listener = {
            use std::os::unix::fs::PermissionsExt;
            let listener = std::os::unix::net::UnixListener::bind(&socket)
                .map_err(|error| format!("Could not start the relay vault broker: {error}"))?;
            let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("Could not configure the relay vault broker: {error}"))?;
            listener
        };
        #[cfg(not(unix))]
        {
            return Err("The relay vault broker requires Unix sockets".into());
        }
        let sessions: SharedSessions = Arc::new(Mutex::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread_sessions = sessions.clone();
        let thread_socket = socket.clone();
        let thread_database = database.clone();
        let join = thread::Builder::new()
            .name("dunlin-relay-vault".into())
            .spawn(move || {
                let secrets: Arc<dyn SecretStore> = Arc::new(ServiceKeychainStore::relay());
                let http = Arc::new(UreqExecutor::new());
                let recipes = Arc::new(CommandExecutor);
                let approver = Arc::new(DenyAll);
                let cleanup_secrets = secrets.clone();
                let cleanup_database = thread_database.clone();
                let cleanup_shutdown = thread_shutdown.clone();
                let _ = thread::Builder::new()
                    .name("dunlin-relay-vault-cleanup".into())
                    .spawn(move || {
                        let mut last_cleanup = Instant::now() - Duration::from_secs(61);
                        while !cleanup_shutdown.load(Ordering::Relaxed) {
                            if last_cleanup.elapsed() >= Duration::from_secs(60) {
                                cleanup_expired(&cleanup_secrets, &cleanup_database);
                                last_cleanup = Instant::now();
                            }
                            thread::sleep(Duration::from_secs(1));
                        }
                    });
                while !thread_shutdown.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let sessions = thread_sessions.clone();
                            let secrets = secrets.clone();
                            let http = http.clone();
                            let recipes = recipes.clone();
                            let approver = approver.clone();
                            let database = thread_database.clone();
                            let _ = thread::Builder::new()
                                .name("dunlin-relay-vault-client".into())
                                .spawn(move || {
                                    handle_connection(
                                        stream, sessions, secrets, http, recipes, approver,
                                        database,
                                    );
                                });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40));
                        }
                        Err(_) => break,
                    }
                }
                let _ = std::fs::remove_file(&thread_socket);
            })
            .map_err(|error| format!("Could not start the relay vault broker: {error}"))?;
        Ok(Arc::new(Self {
            socket,
            sessions,
            shutdown,
            join: Some(join),
        }))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn new_session(
        &self,
        project_id: impl Into<String>,
        project_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> VaultSession {
        let token = new_token();
        let record = Arc::new(Session {
            project_id: project_id.into(),
            project_path: project_path.into(),
            session_id: Mutex::new(None),
            read_only: AtomicBool::new(read_only),
            allowed: AtomicBool::new(true),
            revoked: AtomicBool::new(false),
            bound: AtomicBool::new(false),
        });
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(token.clone(), record.clone());
        }
        VaultSession {
            token,
            record,
            sessions: self.sessions.clone(),
        }
    }
}

impl Drop for RelayVault {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(unix)]
fn handle_connection(
    mut stream: std::os::unix::net::UnixStream,
    sessions: SharedSessions,
    secrets: Arc<dyn SecretStore>,
    http: Arc<UreqExecutor>,
    recipes: Arc<CommandExecutor>,
    approver: Arc<DenyAll>,
    database: PathBuf,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let hello = match read_frame::<WireRequest>(&mut stream) {
        Ok(Some(WireRequest::Hello {
            protocol,
            token,
            pid: _,
        })) => (protocol, token),
        _ => {
            let _ = write_frame(
                &mut stream,
                &WireResponse::Hello {
                    ok: false,
                    message: Some("The relay credential handshake was invalid".into()),
                },
            );
            return;
        }
    };
    let (protocol, token) = hello;
    if protocol != PROTOCOL_VERSION {
        let _ = write_frame(
            &mut stream,
            &WireResponse::Hello {
                ok: false,
                message: Some("The relay credential protocol version is unsupported".into()),
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
                message: Some("The relay credential session is not valid".into()),
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
                message: Some("The relay credential session is already connected or closed".into()),
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
    let _ = stream.set_read_timeout(None);
    let handle = VaultSession {
        token: token.clone(),
        record: record.clone(),
        sessions: sessions.clone(),
    };
    loop {
        let request = match read_frame::<WireRequest>(&mut stream) {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => break,
        };
        let response = match request {
            WireRequest::Hello { .. } => WireResponse::Error {
                message: "The relay credential connection is already initialized".into(),
            },
            request => {
                let store = match Store::open(&database) {
                    Ok(store) => store,
                    Err(error) => {
                        let _ = write_frame(
                            &mut stream,
                            &WireResponse::Error {
                                message: format!(
                                    "Relay credential storage is unavailable: {error}"
                                ),
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

fn cleanup_expired(secrets: &Arc<dyn SecretStore>, database: &Path) {
    let Ok(store) = Store::open(database) else {
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
        let _ = secrets.delete_policy(&secret.id);
        let _ = store.record_secret_audit(&SecretAudit {
            id: 0,
            secret_id: Some(secret.id.clone()),
            alias: secret.alias.clone(),
            project_id: None,
            session_id: None,
            operation: "expiry".into(),
            detail: "relay credential removed after its TTL".into(),
            decision: "expired".into(),
            status: None,
            error: None,
            created_at: now,
        });
        let _ = store.delete_secret(&secret.id);
    }
}
