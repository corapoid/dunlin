//! Opt-in Linux worker executor, wired to the private v3 worker transport.
//! The socket owner supplies a verified live-channel callback; actors own no WS.
use crate::worker_exports::{
    self as local, WorkerStore,
    execution::{ExecutionChannel, ExecutionFilesystem, ExecutionLease},
};
use dunlin_relay_protocol::worker::{
    self as wire, ExecutionControllerMessage as Request, ExecutionErrorCode as Code,
    ExecutionWorkerMessage as Reply,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, Semaphore};
mod acp;
mod actor;
#[cfg(test)]
mod broker_tests;
mod guard;
pub(crate) mod mcp;
mod ownership;
#[cfg(target_os = "linux")]
mod process;
#[cfg(not(target_os = "linux"))]
#[path = "runtime/process_other.rs"]
mod process;
mod sessions;
#[cfg(test)]
mod tests;
static RUNTIME_ACTIVE: AtomicBool = AtomicBool::new(false);
struct RuntimeOwnership {
    _file: Option<ownership::FileLease>,
}
impl Drop for RuntimeOwnership {
    fn drop(&mut self) {
        RUNTIME_ACTIVE.store(false, Ordering::Release);
    }
}
type Guard = Arc<dyn Fn() -> local::Result<()> + Send + Sync>;
#[derive(Clone, Debug)]
pub(crate) struct LocalHarness {
    pub program: PathBuf,
    pub args: Vec<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    pub state_dir: PathBuf,
    pub scope: local::ExportScope,
    pub worker_boot_id: String,
    pub server_config: crate::config::Config,
    pub harnesses: BTreeMap<String, LocalHarness>,
}
#[derive(Clone)]
pub(crate) struct Connected {
    fence: wire::ChannelFence,
    guard: Guard,
    open: Arc<AtomicBool>,
    broker: Option<crate::worker_client::broker::Client>,
}
impl Connected {
    fn authorize(&self) -> local::Result<()> {
        if !self.open.load(Ordering::Acquire) {
            return Err(local::Error::Denied);
        }
        (self.guard)()?;
        if let Some(client) = &self.broker {
            client
                .attest(&self.fence)
                .map_err(|_| local::Error::Denied)?;
        }
        Ok(())
    }
    fn context<'a>(
        &'a self,
        scope: &'a local::ExportScope,
        guard: &'a dyn Fn() -> local::Result<()>,
    ) -> local::Result<ExecutionChannel<'a>> {
        match &self.broker {
            Some(client) => {
                self.authorize()?;
                ExecutionChannel::verify_broker(scope, &self.fence, guard, client.limits())
            }
            None => ExecutionChannel::verify(scope, &self.fence, guard),
        }
    }
}
pub(crate) struct Runtime {
    config: RuntimeConfig,
    _ownership: RuntimeOwnership,
    platform: process::Platform,
    protected: Vec<PathBuf>,
    origin: Instant,
    closed: AtomicBool,
    active: Mutex<Option<Arc<Active>>>,
    session: Mutex<Option<Arc<sessions::SessionActive>>>,
    sessions_available: bool,
    requests: Arc<Semaphore>,
    fs: Arc<Semaphore>,
    proxy_slots: Arc<Semaphore>,
}
struct Active {
    command: wire::ExecutionCommand,
    connection: Connected,
    lease: Mutex<ExecutionLease>,
    filesystem: ExecutionFilesystem,
    deadline_ms: AtomicU64,
    stop: AtomicBool,
    group: Arc<process::Group>,
    done: Notify,
    finished: AtomicBool,
    stopped: AtomicBool,
}
impl Active {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.group.terminate();
    }
}
fn code(error: local::Error) -> Code {
    match error {
        local::Error::Denied => Code::AuthorityRevoked,
        local::Error::Invalid(_) => Code::UnsupportedProfile,
        local::Error::Conflict(message) if message.contains("capacity") => Code::CapacityReached,
        local::Error::Conflict(message)
            if message.contains("expired") || message.contains("elapsed") =>
        {
            Code::LeaseExpired
        }
        local::Error::Conflict(message)
            if message.contains("identity") || message.contains("payload") =>
        {
            Code::IdentityConflict
        }
        _ => Code::Unavailable,
    }
}
impl Runtime {
    pub fn new(mut config: RuntimeConfig) -> Result<Arc<Self>, Code> {
        if !cfg!(target_os = "linux")
            || !config.server_config.sandbox
            || config.harnesses.is_empty()
            || config.harnesses.len() > 3
        {
            return Err(Code::UnsupportedProfile);
        }
        RUNTIME_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Code::CapacityReached)?;
        let mut ownership = RuntimeOwnership { _file: None };
        let probe = wire::ChannelFence {
            channel_id: "configuration-check".into(),
            controller_boot_id: "configuration-check".into(),
            worker_boot_id: config.worker_boot_id.clone(),
        };
        probe.validate().map_err(|_| Code::UnsupportedProfile)?;
        for (id, harness) in &config.harnesses {
            if !dunlin_domain::agent::AGENT_HARNESS_IDS.contains(&id.as_str())
                || !harness.program.is_absolute()
                || !harness.program.is_file()
                || harness.args.len() > 32
                || harness.args.iter().map(String::len).sum::<usize>() > 16384
                || harness.args.iter().any(|a| a.contains('\0'))
            {
                return Err(Code::UnsupportedProfile);
            }
        }
        config.state_dir = config
            .state_dir
            .canonicalize()
            .map_err(|_| Code::Unavailable)?;
        let store =
            WorkerStore::open_existing(&config.state_dir, config.scope.clone()).map_err(code)?;
        ownership._file = Some(ownership::FileLease::acquire(&config.state_dir)?);
        let sessions_available = store.session_schema_ready().map_err(code)?;
        store
            .recover_execution_boot(&config.scope, &config.worker_boot_id)
            .map_err(code)?;
        // Peer open never migrates older operator state. V3 remains usable there.
        store
            .recover_session_boot(&config.scope, &config.worker_boot_id)
            .map_err(code)?;
        // Dedicated worker homes never borrow host relay/admin-user credentials.
        let private = config.state_dir.join("execution-runtime");
        crate::management::private_dir(&private).map_err(|_| Code::Unavailable)?;
        config.server_config.db = private.join("config.sqlite3");
        let mut protected = vec![
            config.state_dir.clone(),
            crate::management::root(&config.server_config),
            PathBuf::from("/etc"),
            PathBuf::from("/proc"),
            PathBuf::from("/sys"),
            PathBuf::from("/dev"),
            PathBuf::from("/run"),
            PathBuf::from("/var/run"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/usr/lib64"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/lib"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home)
                .canonicalize()
                .map_err(|_| Code::Unavailable)?;
            for entry in [".ssh", ".aws", ".gnupg", ".config", ".local/share/keyrings"] {
                protected.push(home.join(entry));
            }
        }
        if let Some(socket) = std::env::var_os("SSH_AUTH_SOCK") {
            let socket = PathBuf::from(socket);
            if socket.is_absolute() {
                protected.push(socket);
            }
        }
        for harness in config.harnesses.values_mut() {
            let executable = harness
                .program
                .canonicalize()
                .map_err(|_| Code::UnsupportedProfile)?;
            harness.program = executable.clone();
            protected.push(executable.clone());
            protected.push(
                executable
                    .parent()
                    .ok_or(Code::UnsupportedProfile)?
                    .to_owned(),
            );
        }
        let platform = process::Platform::preflight(&config).map_err(|error| {
            #[cfg(test)]
            eprintln!(
                "worker execution preflight failed: kind={:?}, diagnostic={}",
                error.kind(),
                error.to_string().chars().take(240).collect::<String>()
            );
            #[cfg(not(test))]
            let _ = error;
            Code::UnsupportedProfile
        })?;
        protected.push(platform.program_directory());
        Ok(Arc::new(Self {
            config,
            _ownership: ownership,
            platform,
            protected,
            origin: Instant::now(),
            closed: AtomicBool::new(false),
            active: Mutex::new(None),
            session: Mutex::new(None),
            sessions_available,
            requests: Arc::new(Semaphore::new(8)),
            fs: Arc::new(Semaphore::new(2)),
            proxy_slots: Arc::new(Semaphore::new(8)),
        }))
    }
    pub fn connection(&self, fence: wire::ChannelFence, guard: Guard) -> Result<Connected, Code> {
        fence.validate().map_err(|_| Code::StaleChannel)?;
        if self.closed.load(Ordering::Acquire) || fence.worker_boot_id != self.config.worker_boot_id
        {
            return Err(Code::StaleChannel);
        }
        guard().map_err(code)?;
        Ok(Connected {
            fence,
            guard,
            open: Arc::new(AtomicBool::new(true)),
            broker: None,
        })
    }
    pub(crate) fn broker_connection(
        &self,
        fence: wire::ChannelFence,
        guard: Guard,
        client: crate::worker_client::broker::Client,
    ) -> Result<Connected, Code> {
        client.attest(&fence).map_err(|_| Code::StaleChannel)?;
        let mut connection = self.connection(fence, guard)?;
        connection.broker = Some(client);
        connection.authorize().map_err(code)?;
        Ok(connection)
    }
    pub fn supports_sessions(&self) -> bool {
        self.sessions_available && !self.closed.load(Ordering::Acquire)
    }
    fn milliseconds(&self, deadline: Instant) -> u64 {
        u64::try_from(deadline.saturating_duration_since(self.origin).as_millis())
            .unwrap_or(u64::MAX)
    }
    fn current(&self, active: &Active) -> Result<(), Code> {
        if self.closed.load(Ordering::Acquire)
            || active.stop.load(Ordering::Acquire)
            || self.milliseconds(Instant::now()) >= active.deadline_ms.load(Ordering::Acquire)
        {
            return Err(Code::LeaseExpired);
        }
        active.connection.authorize().map_err(code)
    }
    fn store(&self) -> Result<WorkerStore, Code> {
        WorkerStore::open_existing(&self.config.state_dir, self.config.scope.clone()).map_err(code)
    }
    pub async fn handle(
        self: &Arc<Self>,
        connection: &Connected,
        request: Request,
    ) -> Result<Option<Reply>, Code> {
        request.validate().map_err(|_| Code::IdentityConflict)?;
        connection.authorize().map_err(code)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(Code::StaleChannel);
        }
        match request {
            Request::Execute { command } => {
                // Preparation remains server-owned if the socket waiter drops.
                let permit = self
                    .requests
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Code::CapacityReached)?;
                let runtime = self.clone();
                let connection = connection.clone();
                let (send, receive) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let _permit = permit;
                    let result = runtime.submit(&connection, *command).await.map(Some);
                    let _ = send.send(result);
                });
                tokio::time::timeout(Duration::from_secs(8), receive)
                    .await
                    .map_err(|_| Code::Unavailable)?
                    .map_err(|_| Code::Unavailable)?
            }
            Request::Renew {
                fence,
                sequence,
                lease_duration_ms,
            } => {
                if connection.fence != fence.channel {
                    return Err(Code::StaleChannel);
                }
                let active = self
                    .active
                    .lock()
                    .map_err(|_| Code::Unavailable)?
                    .as_ref()
                    .filter(|a| a.command.identity.attempt_id == fence.attempt_id)
                    .cloned()
                    .ok_or(Code::StaleAttempt)?;
                self.current(&active)?;
                let runtime = self.clone();
                let connection = connection.clone();
                let task = tokio::task::spawn_blocking(move || {
                    runtime.current(&active)?;
                    let store = runtime.store()?;
                    let guard = || connection.authorize();
                    let context = connection
                        .context(&runtime.config.scope, &guard)
                        .map_err(code)?;
                    let mut lease = active.lease.lock().map_err(|_| Code::Unavailable)?;
                    if lease.fence() != &fence {
                        return Err(Code::StaleAttempt);
                    }
                    store
                        .renew_execution(&context, &mut lease, sequence, lease_duration_ms)
                        .map_err(code)?;
                    runtime.current(&active)?;
                    let deadline = Instant::now() + lease.remaining().map_err(code)?;
                    let monotonic = process::monotonic_ns().map_err(|_| Code::Unavailable)?;
                    let remaining = lease.remaining().unwrap_or_default();
                    active
                        .group
                        .renew(monotonic.saturating_add(remaining.as_nanos() as u64))
                        .map_err(|_| Code::LeaseExpired)?;
                    active
                        .deadline_ms
                        .store(runtime.milliseconds(deadline), Ordering::Release);
                    if active.stop.load(Ordering::Acquire) {
                        return Err(Code::LeaseExpired);
                    }
                    Ok(Some(Reply::Renewed {
                        fence,
                        sequence,
                        lease_duration_ms,
                    }))
                });
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .map_err(|_| Code::Unavailable)?
                    .map_err(|_| Code::Unavailable)?
            }
            Request::Cancel { fence } => {
                if connection.fence != fence.channel {
                    return Err(Code::StaleChannel);
                }
                if let Some(active) = self
                    .active
                    .lock()
                    .map_err(|_| Code::Unavailable)?
                    .as_ref()
                    .filter(|a| a.command.identity.attempt_id == fence.attempt_id)
                {
                    active.stop();
                }
                let runtime = self.clone();
                let connection = connection.clone();
                tokio::task::spawn_blocking(move || {
                    let store = runtime.store()?;
                    let guard = || connection.authorize();
                    let context = connection
                        .context(&runtime.config.scope, &guard)
                        .map_err(code)?;
                    let requested = store.cancel_execution(&context, &fence).map_err(code)?;
                    Ok(Some(Reply::CancelAck {
                        fence,
                        disposition: if requested {
                            wire::ExecutionCancelDisposition::Requested
                        } else {
                            wire::ExecutionCancelDisposition::AlreadyTerminal
                        },
                    }))
                })
                .await
                .map_err(|_| Code::Unavailable)?
            }
            Request::Reconcile { channel, identity } => {
                if channel != connection.fence {
                    return Err(Code::StaleChannel);
                }
                let runtime = self.clone();
                let connection = connection.clone();
                tokio::task::spawn_blocking(move || {
                    let store = runtime.store()?;
                    let guard = || connection.authorize();
                    let context = connection
                        .context(&runtime.config.scope, &guard)
                        .map_err(code)?;
                    let record = store
                        .reconcile_execution(&context, &identity)
                        .map_err(code)?;
                    Ok(Some(Reply::Reconciled {
                        channel,
                        identity,
                        record,
                    }))
                })
                .await
                .map_err(|_| Code::Unavailable)?
            }
            Request::OutcomeAck {
                channel,
                identity,
                fence,
                outcome_id,
                outcome_digest,
            } => {
                if channel != connection.fence {
                    return Err(Code::StaleChannel);
                }
                let runtime = self.clone();
                let connection = connection.clone();
                tokio::task::spawn_blocking(move || {
                    let store = runtime.store()?;
                    let guard = || connection.authorize();
                    let context = connection
                        .context(&runtime.config.scope, &guard)
                        .map_err(code)?;
                    store
                        .acknowledge_execution(
                            &context,
                            &identity,
                            &fence,
                            &outcome_id,
                            &outcome_digest,
                        )
                        .map_err(code)?;
                    Ok(Some(Reply::OutcomeAcknowledged {
                        channel,
                        identity,
                        fence,
                        outcome_id,
                        outcome_digest,
                    }))
                })
                .await
                .map_err(|_| Code::Unavailable)?
            }
        }
    }
    async fn submit(
        self: &Arc<Self>,
        connection: &Connected,
        command: wire::ExecutionCommand,
    ) -> Result<Reply, Code> {
        let harness = self
            .config
            .harnesses
            .get(&command.definition.harness)
            .ok_or(Code::UnsupportedProfile)?
            .clone();
        let runtime = self.clone();
        let connection = connection.clone();
        let permit = tokio::time::timeout(Duration::from_secs(5), self.fs.clone().acquire_owned())
            .await
            .map_err(|_| Code::Unavailable)?
            .map_err(|_| Code::Unavailable)?;
        let admitted = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            connection.authorize().map_err(code)?;
            let store = runtime.store()?;
            let principal = local::ExportPrincipal {
                kind: local::PrincipalKind::Human,
                id: command.descriptor.grant.principal.id.clone(),
            };
            let validated = store
                .candidate(
                    &runtime.config.scope,
                    &command.descriptor.export.id,
                    &principal,
                    local::ExportPermission::Execute,
                )
                .map_err(code)?
                .validate_filesystem()
                .map_err(code)?;
            validated
                .reject_private_overlap(&runtime.protected)
                .map_err(code)?;
            let guard = || connection.authorize();
            let context = connection
                .context(&runtime.config.scope, &guard)
                .map_err(code)?;
            let admission = store
                .admit_execution(&context, &command, validated)
                .map_err(code)?;
            Ok::<_, Code>((command, connection, admission))
        })
        .await
        .map_err(|_| Code::Unavailable)??;
        let (command, connection, admission) = admitted;
        if let Some(lease) = admission.lease {
            let remaining = lease.remaining().unwrap_or_default();
            let active = Arc::new(Active {
                filesystem: lease.filesystem_guard(),
                command,
                connection,
                lease: Mutex::new(lease),
                deadline_ms: AtomicU64::new(self.milliseconds(Instant::now() + remaining)),
                stop: AtomicBool::new(false),
                group: Arc::new(process::Group::default()),
                done: Notify::new(),
                finished: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
            });
            let mut slot = self.active.lock().map_err(|_| Code::Unavailable)?;
            if slot
                .as_ref()
                .is_some_and(|a| !a.stopped.load(Ordering::Acquire))
            {
                active.stop();
                return Err(Code::CapacityReached);
            }
            *slot = Some(active.clone());
            drop(slot);
            let runtime = self.clone();
            tokio::spawn(async move {
                runtime.actor(active, harness).await;
            });
        }
        Ok(Reply::Accepted {
            record: admission.record,
        })
    }
    pub async fn outbox(
        self: &Arc<Self>,
        connection: &Connected,
    ) -> Result<Vec<wire::ExecutionRecord>, Code> {
        let runtime = self.clone();
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
            let store = runtime.store()?;
            let guard = || connection.authorize();
            let context = connection
                .context(&runtime.config.scope, &guard)
                .map_err(code)?;
            store.execution_outbox(&context).map_err(code)
        })
        .await
        .map_err(|_| Code::Unavailable)?
    }
    pub fn disconnect(&self, connection: &Connected) {
        connection.open.store(false, Ordering::Release);
        self.disconnect_session(connection);
        if let Ok(slot) = self.active.lock()
            && let Some(active) = slot.as_ref()
            && active.connection.fence == connection.fence
        {
            active.stop();
        }
    }
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        self.shutdown_session().await;
        let active = self.active.lock().ok().and_then(|slot| slot.clone());
        if let Some(active) = active {
            active.stop();
            if !active.finished.load(Ordering::Acquire) {
                let _ = tokio::time::timeout(Duration::from_secs(10), async {
                    while !active.finished.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await;
            }
        }
    }
}

fn sandbox_command(
    config: &RuntimeConfig,
    harness: &LocalHarness,
    root: &std::path::Path,
    metadata: &crate::store::native_checkout::SandboxMetadataRoots,
    user: &str,
) -> std::io::Result<tokio::process::Command> {
    sandbox_command_with_proxy(config, harness, root, metadata, user, &[])
}
fn sandbox_command_with_proxy(
    config: &RuntimeConfig,
    harness: &LocalHarness,
    root: &std::path::Path,
    metadata: &crate::store::native_checkout::SandboxMetadataRoots,
    user: &str,
    broker_paths: &[PathBuf],
) -> std::io::Result<tokio::process::Command> {
    let root = root
        .to_str()
        .ok_or_else(|| std::io::Error::other("Non-UTF8 checkout root"))?;
    let program = harness
        .program
        .to_str()
        .ok_or_else(|| std::io::Error::other("Non-UTF8 harness path"))?;
    let mut command = if broker_paths.is_empty() {
        crate::acp::harness_command_with_metadata(
            &config.server_config,
            program,
            &harness.args,
            root,
            &[],
            &[],
            &dunlin_domain::ProjectSettings::default(),
            user,
            metadata,
        )
    } else {
        crate::acp::harness_command_with_brokers(
            &config.server_config,
            program,
            &harness.args,
            root,
            &[],
            &[],
            &dunlin_domain::ProjectSettings::default(),
            user,
            broker_paths,
            metadata,
        )
    };
    let home = crate::management::agent_home(&config.server_config, user);
    let path = std::env::join_paths([
        crate::management::tools(&config.server_config).join("bin"),
        crate::management::root(&config.server_config).join("node/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])
    .map_err(std::io::Error::other)?;
    command
        .env_clear()
        .env("PATH", path)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("TMPDIR", "/tmp")
        .current_dir(root);
    dunlin_workspace::git::scrub_repository_environment(command.as_std_mut());
    Ok(command)
}
pub(crate) fn execution_guard(args: &[String]) -> Result<(), String> {
    process::guard_command(args)
}
