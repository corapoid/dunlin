//! Internal one-turn controller orchestration. No HTTP Remote/AHP activation.
//! Transport, profile and ledger data are never sufficient without fresh caller
//! policy and opaque live-v3 attestation at each positive SQL cut.
#![expect(
    dead_code,
    reason = "Internal v3 compatibility API is retained and exercised by transport/runtime tests; public interactive sessions use v4"
)]
use crate::{
    api::{AppState, AuthUser},
    config::Config,
    management::Management,
    store::remote_bindings::resolver::execution::{
        PreparedRemoteExecution, RemoteExecutionRequest, RemoteExecutionReservation,
        RemoteExecutionSnapshot,
    },
    store::{Store, StoreError},
    worker_channel::{
        ExecutionEvent, ExecutionProducerRegistration, ExecutionTransport, Service as Channels,
    },
};
use dunlin_relay_protocol::worker::{
    ExecutionCommand, ExecutionControllerMessage as Request, ExecutionRecord,
    ExecutionWorkerMessage as Reply,
};
use serde::Deserialize;
use std::io::Read;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::Instant,
};
const SQL_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Disabled,
    Unavailable,
    Capacity,
    Unauthorized,
    Conflict,
    Uncertain,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disabled => "Remote execution service disabled",
            Self::Unavailable => "Remote execution service unavailable",
            Self::Capacity => "Remote execution capacity reached",
            Self::Unauthorized => "Remote execution authority denied",
            Self::Conflict => "Remote execution identity or authority changed",
            Self::Uncertain => "Remote execution outcome uncertain; reconcile the same intent",
        })
    }
}
impl std::error::Error for Error {}
pub(crate) type Result<T> = std::result::Result<T, Error>;
fn map(error: StoreError) -> Error {
    match error {
        StoreError::Policy(_) => Error::Unauthorized,
        StoreError::Conflict(_) => Error::Conflict,
        StoreError::Limit(_) => Error::Capacity,
        _ => Error::Unavailable,
    }
}
fn denied() -> StoreError {
    dunlin_domain::collaboration::PolicyError::PermissionDenied.into()
}
fn now() -> crate::store::Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .ok_or_else(denied)
}
#[derive(Deserialize)]
struct HostPolicySettings {
    #[serde(default)]
    host_accounts: bool,
}
pub(crate) fn host_accounts(config: &Config) -> bool {
    let path = crate::management::root(config).join("settings.json");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024)
    {
        return true;
    }
    let mut bytes = Vec::new();
    if file.take(64 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 64 * 1024 {
        return true;
    }
    serde_json::from_slice::<HostPolicySettings>(&bytes)
        .map(|s| s.host_accounts)
        .unwrap_or(true)
}
pub(crate) struct Context {
    store: Arc<Mutex<Store>>,
    config: Arc<Config>,
    management: Arc<Mutex<Management>>,
    channels: Arc<Channels>,
    stop: Arc<AtomicBool>,
    stopped: Notify,
    sql: Arc<Semaphore>,
    fs: Arc<Semaphore>,
}
impl Context {
    pub(crate) fn policy_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.fs
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)
    }
    pub(crate) fn matches_state(&self, state: &AppState) -> bool {
        Arc::ptr_eq(&self.store, &state.store)
            && Arc::ptr_eq(&self.config, &state.config)
            && Arc::ptr_eq(&self.management, &state.management)
            && Arc::ptr_eq(&self.channels, &state.worker_channels)
    }
    pub(crate) fn new(state: &AppState) -> Arc<Self> {
        Arc::new(Self {
            store: state.store.clone(),
            config: state.config.clone(),
            management: state.management.clone(),
            channels: state.worker_channels.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            stopped: Notify::new(),
            sql: Arc::new(Semaphore::new(8)),
            fs: Arc::new(Semaphore::new(4)),
        })
    }
    pub(crate) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.stopped.notify_waiters();
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    fn node(&self) -> crate::store::Result<&str> {
        self.config
            .worker_controller
            .as_ref()
            .filter(|c| c.execution_enabled)
            .map(|c| c.node_id.as_str())
            .ok_or_else(denied)
    }
    pub(crate) fn caller(
        &self,
        store: &Store,
        owner: &str,
        device: &str,
        host_accounts: bool,
    ) -> crate::store::Result<()> {
        let user = store
            .active_device_user(device)?
            .filter(|u| u.id == owner)
            .ok_or_else(denied)?;
        if (host_accounts && user.role != "admin")
            || !store.oidc_device_allowed(device, self.config.oidc.as_ref())?
        {
            return Err(denied());
        }
        self.node()?;
        Ok(())
    }
    pub(crate) fn running(&self) -> crate::store::Result<()> {
        if self.stop.load(Ordering::Acquire) {
            Err(denied())
        } else {
            Ok(())
        }
    }
    fn producer_scope(
        &self,
        _store: &Store,
        command: &ExecutionCommand,
    ) -> crate::store::Result<()> {
        if command.descriptor.scope.controller_node_id != self.node()? {
            return Err(denied());
        }
        Ok(())
    }
    fn producer(
        &self,
        store: &Store,
        command: &ExecutionCommand,
        transport: &ExecutionTransport,
    ) -> crate::store::Result<()> {
        self.producer_scope(store, command)?;
        if command.descriptor.scope.host_id != transport.host_id()
            || command.descriptor.scope.workspace_id != transport.identity().workspace_id()
            || command.descriptor.scope.worker_node_id
                != transport.identity().host().definition.node_id
            || command.descriptor.scope.enrollment_epoch
                != transport.identity().host().enrollment_epoch
            || command.descriptor.scope.credential_revision
                != transport.identity().host().credential_revision
        {
            return Err(denied());
        }
        transport.attest_in_transaction(
            store,
            transport.identity(),
            transport.lease(),
            transport.fence(),
        )
    }
    fn spawn<T, F>(
        self: &Arc<Self>,
        policy: bool,
        operation: F,
    ) -> Result<tokio::task::JoinHandle<crate::store::Result<T>>>
    where
        T: Send + 'static,
        F: FnOnce(&Context, &Store, bool) -> crate::store::Result<T> + Send + 'static,
    {
        let sql = self
            .sql
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let fs = if policy {
            Some(
                self.fs
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::Capacity)?,
            )
        } else {
            None
        };
        let context = self.clone();
        Ok(tokio::task::spawn_blocking(move || {
            let _sql = sql;
            let _fs = fs;
            // Management mutations use Management -> Store. Capture filesystem
            // policy before Store, holding the same management cut through SQL.
            let management = if policy {
                Some(context.management.lock().map_err(|_| denied())?)
            } else {
                None
            };
            let host_accounts = policy && host_accounts(&context.config);
            let store = context.store.lock().map_err(|_| denied())?;
            let result = operation(&context, &store, host_accounts);
            drop(store);
            drop(management);
            result
        }))
    }
    pub(crate) async fn sql<T, F>(self: &Arc<Self>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Context, &Store) -> crate::store::Result<T> + Send + 'static,
    {
        let job = self.spawn(false, move |ctx, store, _| operation(ctx, store))?;
        tokio::time::timeout(SQL_TIMEOUT, job)
            .await
            .map_err(|_| Error::Uncertain)?
            .map_err(|_| Error::Unavailable)?
            .map_err(map)
    }
    pub(crate) async fn authority<T, F>(self: &Arc<Self>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Context, &Store, bool) -> crate::store::Result<T> + Send + 'static,
    {
        let job = self.spawn(true, operation)?;
        tokio::time::timeout(SQL_TIMEOUT, job)
            .await
            .map_err(|_| Error::Uncertain)?
            .map_err(|_| Error::Unavailable)?
            .map_err(map)
    }
    async fn snapshot(self: &Arc<Self>, attempt: String) -> Result<RemoteExecutionSnapshot> {
        self.sql(move |ctx, store| {
            store.remote_execution_snapshot(&attempt, |store, command| {
                ctx.producer_scope(store, command)
            })
        })
        .await
    }
    async fn prepare(
        self: &Arc<Self>,
        owner: String,
        device: String,
        workspace: String,
        project: String,
        request: RemoteExecutionRequest,
    ) -> Result<(PreparedRemoteExecution, Option<RemoteExecutionReservation>)> {
        self.authority(move |ctx, store, policy| {
            store.remote_binding_transaction(false, |store| {
                ctx.caller(store, &owner, &device, policy)?;
                let prepared = store.prepare_remote_execution_in_transaction(
                    &owner,
                    &device,
                    &workspace,
                    &project,
                    request,
                    ctx.node()?,
                )?;
                let existing = store.lookup_remote_execution_intent_in_transaction(
                    &prepared,
                    |store, user, device| ctx.caller(store, user, device, policy),
                )?;
                Ok((prepared, existing))
            })
        })
        .await
    }
    async fn describe(
        &self,
        prepared: &PreparedRemoteExecution,
        transport: &ExecutionTransport,
    ) -> Result<crate::worker_channel::ObservedExport> {
        self.channels
            .describe_execution_export(
                transport,
                prepared.export_id(),
                prepared.principal(),
                Some(prepared.export_revision()),
            )
            .await
            .map_err(|_| Error::Unavailable)
    }
    async fn persist(
        self: &Arc<Self>,
        transport: ExecutionTransport,
        attempt: String,
        record: ExecutionRecord,
    ) -> Result<()> {
        self.sql(move |ctx, store| {
            store
                .record_remote_execution(&attempt, record, |store, command, _| {
                    ctx.producer(store, command, &transport)
                })
                .map(|_| ())
        })
        .await
    }
    async fn acknowledge(
        self: &Arc<Self>,
        transport: ExecutionTransport,
        attempt: String,
    ) -> Result<()> {
        let acknowledged_attempt = attempt.clone();
        let source = transport.clone();
        let message = self
            .sql(move |ctx, store| {
                store.remote_execution_outcome_ack(
                    &attempt,
                    source.fence().clone(),
                    source.identity(),
                    source.lease(),
                    now()?,
                    |store, command, _, _, _| ctx.producer(store, command, &source),
                )
            })
            .await?;
        transport
            .send_ack(message.clone())
            .await
            .map_err(|_| Error::Unavailable)?;
        self.sql(move |ctx, store| {
            store
                .confirm_remote_execution_ack(
                    &acknowledged_attempt,
                    &message,
                    |store, command, message| {
                        ctx.producer(store, command, &transport)?;
                        transport.attest_outcome_ack_in_transaction(store, message)
                    },
                )
                .map(|_| ())
        })
        .await
    }
    async fn reconcile_producer(
        self: &Arc<Self>,
        attempt: String,
        transport: ExecutionTransport,
    ) -> Result<Option<ExecutionRecord>> {
        let source = transport.clone();
        let query_attempt = attempt.clone();
        let query = self
            .sql(move |ctx, store| {
                store.reconcile_remote_execution_from_guard(
                    &query_attempt,
                    source.fence().clone(),
                    source.identity(),
                    source.lease(),
                    now()?,
                    |store, command, _, _, _| ctx.producer(store, command, &source),
                )
            })
            .await?;
        match transport
            .request(query)
            .await
            .map_err(|_| Error::Uncertain)?
        {
            Reply::Reconciled { record, .. } => Ok(record),
            _ => Err(Error::Uncertain),
        }
    }
    async fn reconcile(
        self: &Arc<Self>,
        owner: String,
        device: String,
        attempt: String,
        transport: ExecutionTransport,
    ) -> Result<Option<ExecutionRecord>> {
        let source = transport.clone();
        let query_attempt = attempt.clone();
        let query = self
            .authority(move |ctx, store, policy| {
                store.reconcile_remote_execution(
                    &owner,
                    &device,
                    &query_attempt,
                    source.fence().clone(),
                    source.identity(),
                    source.lease(),
                    now()?,
                    |store, owner, device, identity, lease, fence| {
                        ctx.caller(store, owner, device, policy)?;
                        source.attest_in_transaction(store, identity, lease, fence)
                    },
                )
            })
            .await?;
        match transport
            .request(query)
            .await
            .map_err(|_| Error::Uncertain)?
        {
            Reply::Reconciled { record, .. } => {
                if let Some(record) = &record {
                    self.persist(transport.clone(), attempt.clone(), record.clone())
                        .await?;
                    if record.outcome.is_some() {
                        let _ = self.acknowledge(transport, attempt).await;
                    }
                }
                Ok(record)
            }
            _ => Err(Error::Uncertain),
        }
    }
}
struct Attempt {
    id: String,
    owner: String,
    device: String,
    workspace: String,
    project: String,
    request: RemoteExecutionRequest,
    transport: ExecutionTransport,
    stop: AtomicBool,
    changed: Notify,
    record: Mutex<Option<ExecutionRecord>>,
}
impl Attempt {
    fn terminal(&self) -> bool {
        self.record
            .lock()
            .is_ok_and(|r| r.as_ref().is_some_and(|r| r.outcome.is_some()))
    }
    fn observe(&self, record: ExecutionRecord) {
        if let Ok(mut current) = self.record.lock()
            && current.as_ref().is_none_or(|old| old.outcome.is_none())
        {
            *current = Some(record);
        }
        self.changed.notify_waiters();
    }
    fn receipt(&self) -> Option<dunlin_relay_protocol::worker::AttemptFence> {
        self.record
            .lock()
            .ok()?
            .as_ref()
            .map(|r| r.receipt.fence.clone())
    }
}
pub(crate) struct Service {
    context: Arc<Context>,
    registration: Mutex<Option<ExecutionProducerRegistration>>,
    attempts: Mutex<BTreeMap<String, Arc<Attempt>>>,
    capacity: Arc<Semaphore>,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}
impl Service {
    pub(crate) async fn start(state: &AppState) -> Result<Arc<Self>> {
        if state
            .config
            .worker_controller
            .as_ref()
            .is_none_or(|c| !c.execution_enabled)
        {
            return Err(Error::Disabled);
        }
        let (sender, events) = mpsc::channel(32);
        // Exclusive producer first: a second start must not recover the first
        // live service's attempts before discovering duplicate ownership.
        let registration = state
            .worker_channels
            .register_execution_producer(sender)
            .map_err(|_| Error::Unavailable)?;
        let context = Context::new(state);
        // Recovery may outlive the startup waiter (SQL timeout or cancelled
        // future). Keep BOTH the exclusive registration and its receiver in
        // the actual blocking operation until recovery finishes. Otherwise a
        // replacement service could admit work before stale recovery runs.
        let (registration, events) = context
            .sql(move |_, store| {
                store.recover_remote_executions()?;
                Ok((registration, events))
            })
            .await?;
        let service = Arc::new(Self {
            context,
            registration: Mutex::new(Some(registration)),
            attempts: Mutex::new(BTreeMap::new()),
            capacity: Arc::new(Semaphore::new(8)),
            tasks: Mutex::new(Vec::new()),
        });
        service.track(tokio::spawn(events_loop(Arc::downgrade(&service), events)));
        service.track(tokio::spawn(cleanup_loop(
            Arc::downgrade(&service),
            service.context.clone(),
        )));
        Ok(service)
    }
    fn track(&self, task: tokio::task::JoinHandle<()>) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|task| !task.is_finished());
            tasks.push(task.abort_handle());
        }
    }
    pub(crate) async fn submit(
        self: &Arc<Self>,
        auth: &AuthUser,
        workspace: &str,
        project: &str,
        request: RemoteExecutionRequest,
    ) -> Result<RemoteExecutionReservation> {
        let (prepared, existing) = self
            .context
            .prepare(
                auth.user.id.clone(),
                auth.device.id.clone(),
                workspace.into(),
                project.into(),
                request.clone(),
            )
            .await?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        self.context.running().map_err(map)?;
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let context = self.context.clone();
        let weak = Arc::downgrade(self);
        let owner = auth.user.id.clone();
        let device = auth.device.id.clone();
        let workspace = workspace.to_owned();
        let project = project.to_owned();
        let (send, receive) = oneshot::channel();
        self.track(tokio::spawn(async move {
            let result = admit_owned(
                context, weak, owner, device, workspace, project, request, prepared, permit,
            )
            .await;
            let _ = send.send(result);
        }));
        // The owned admission continues if this browser waiter is dropped. A
        // timeout is uncertain and retains the same idempotency key/intent.
        tokio::time::timeout(Duration::from_secs(20), receive)
            .await
            .map_err(|_| Error::Uncertain)?
            .map_err(|_| Error::Uncertain)?
    }
    pub(crate) async fn status(
        &self,
        auth: &AuthUser,
        attempt: &str,
    ) -> Result<RemoteExecutionReservation> {
        let owner = auth.user.id.clone();
        let device = auth.device.id.clone();
        let attempt = attempt.to_owned();
        self.context
            .authority(move |ctx, store, policy| {
                store.remote_execution(&owner, &device, &attempt, |store, owner, device| {
                    ctx.caller(store, owner, device, policy)
                })
            })
            .await
    }
    pub(crate) async fn cancel(
        &self,
        auth: &AuthUser,
        attempt: &str,
    ) -> Result<RemoteExecutionReservation> {
        let owner = auth.user.id.clone();
        let device = auth.device.id.clone();
        let id = attempt.to_owned();
        let message = self
            .context
            .authority(move |ctx, store, policy| {
                store.cancel_remote_execution(&owner, &device, &id, |store, owner, device| {
                    ctx.caller(store, owner, device, policy)
                })
            })
            .await?;
        let actor = self
            .attempts
            .lock()
            .map_err(|_| Error::Unavailable)?
            .get(attempt)
            .cloned();
        if let Some(actor) = actor {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
        }
        if let Some(message) = message {
            let snapshot = self.context.snapshot(attempt.into()).await?;
            if let Ok(transport) = self
                .context
                .channels
                .execution_transport(&snapshot.command.descriptor.scope.host_id)
            {
                let _ = transport.request(message).await;
            }
        }
        self.status(auth, attempt).await
    }
    pub(crate) async fn reconcile(
        &self,
        auth: &AuthUser,
        attempt: &str,
    ) -> Result<RemoteExecutionReservation> {
        self.status(auth, attempt).await?;
        let snapshot = self.context.snapshot(attempt.into()).await?;
        let transport = self
            .context
            .channels
            .execution_transport(&snapshot.command.descriptor.scope.host_id)
            .map_err(|_| Error::Unavailable)?;
        let record = self
            .context
            .reconcile(
                auth.user.id.clone(),
                auth.device.id.clone(),
                attempt.into(),
                transport,
            )
            .await?;
        if let Some(record) = record
            && let Some(actor) = self
                .attempts
                .lock()
                .map_err(|_| Error::Unavailable)?
                .get(attempt)
                .cloned()
        {
            actor.observe(record);
        }
        // This method never creates an actor or schedules a renewal, including
        // when a new boot reports a historical running receipt.
        self.status(auth, attempt).await
    }
    pub(crate) async fn shutdown(&self) {
        self.context.stop.store(true, Ordering::Release);
        self.context.stopped.notify_waiters();
        if let Ok(attempts) = self.attempts.lock() {
            for actor in attempts.values() {
                actor.stop.store(true, Ordering::Release);
                actor.changed.notify_waiters();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.attempts.lock().is_ok_and(|a| !a.is_empty()) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Ok(mut registration) = self.registration.lock() {
            registration.take();
        }
        if let Ok(tasks) = self.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.context.stop.store(true, Ordering::Release);
        self.context.stopped.notify_waiters();
        if let Ok(attempts) = self.attempts.lock() {
            for actor in attempts.values() {
                actor.stop.store(true, Ordering::Release);
                actor.changed.notify_waiters();
            }
        }
        if let Ok(mut registration) = self.registration.lock() {
            registration.take();
        }
        if let Ok(tasks) = self.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn admit_owned(
    context: Arc<Context>,
    service: Weak<Service>,
    owner: String,
    device: String,
    workspace: String,
    project: String,
    request: RemoteExecutionRequest,
    prepared: PreparedRemoteExecution,
    permit: OwnedSemaphorePermit,
) -> Result<RemoteExecutionReservation> {
    let transport = context
        .channels
        .execution_transport(prepared.host_id())
        .map_err(|_| Error::Unavailable)?;
    let proof = context.describe(&prepared, &transport).await?;
    let source = transport.clone();
    // This observer deliberately owns the actual SQL completion, independent of
    // the browser timeout. It holds no strong Service reference while blocked.
    let job = context.spawn(true, move |ctx, store, policy| {
        ctx.running()?;
        let reservation = store.admit_remote_execution(
            prepared,
            proof,
            source.fence().clone(),
            now()?,
            |store, owner, device, identity, lease, fence| {
                ctx.running()?;
                ctx.caller(store, owner, device, policy)?;
                source.attest_in_transaction(store, identity, lease, fence)
            },
        )?;
        if ctx.stop.load(Ordering::Acquire) && !reservation.replayed {
            store.cancel_remote_execution_from_guard(
                &reservation.identity.attempt_id,
                |store, command| ctx.producer_scope(store, command),
            )?;
        }
        Ok(reservation)
    })?;
    let reservation = job.await.map_err(|_| Error::Uncertain)?.map_err(map)?;
    if reservation.replayed {
        return Ok(reservation);
    }
    let actor = Arc::new(Attempt {
        id: reservation.identity.attempt_id.clone(),
        owner,
        device,
        workspace,
        project,
        request,
        transport,
        stop: AtomicBool::new(false),
        changed: Notify::new(),
        record: Mutex::new(reservation.record.clone()),
    });
    if let Some(owner) = service.upgrade()
        && !context.stop.load(Ordering::Acquire)
    {
        owner
            .attempts
            .lock()
            .map_err(|_| Error::Unavailable)?
            .insert(actor.id.clone(), actor.clone());
        owner.track(tokio::spawn(run_attempt(
            service.clone(),
            context,
            actor,
            permit,
        )));
        Ok(reservation)
    } else {
        let attempt = actor.id.clone();
        let _ = context
            .sql(move |ctx, store| {
                store.cancel_remote_execution_from_guard(&attempt, |store, command| {
                    ctx.producer_scope(store, command)
                })
            })
            .await;
        Err(Error::Uncertain)
    }
}
async fn fresh(context: &Arc<Context>, actor: &Attempt) -> Result<PreparedRemoteExecution> {
    context
        .prepare(
            actor.owner.clone(),
            actor.device.clone(),
            actor.workspace.clone(),
            actor.project.clone(),
            actor.request.clone(),
        )
        .await
        .map(|(prepared, _)| prepared)
}
async fn monitor(context: Arc<Context>, actor: Arc<Attempt>) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tick.tick().await;
        if context.stop.load(Ordering::Acquire) || actor.stop.load(Ordering::Acquire) {
            return Err(Error::Unauthorized);
        }
        let owner = actor.owner.clone();
        let device = actor.device.clone();
        let attempt = actor.id.clone();
        let source = actor.transport.clone();
        let stop = actor.clone();
        context
            .authority(move |ctx, store, policy| {
                store.remote_binding_transaction(false, |store| {
                    ctx.running()?;
                    if stop.stop.load(Ordering::Acquire) {
                        return Err(denied());
                    }
                    ctx.caller(store, &owner, &device, policy)?;
                    source.attest_in_transaction(
                        store,
                        source.identity(),
                        source.lease(),
                        source.fence(),
                    )?;
                    store
                        .remote_execution_controller_authority_in_transaction(
                            &attempt,
                            &owner,
                            &device,
                            ctx.node()?,
                        )
                        .map(|_| ())
                })
            })
            .await?;
    }
}
async fn receive_record(context: &Arc<Context>, actor: &Attempt, reply: Reply) -> Result<()> {
    let Reply::Accepted { record } = reply else {
        return Err(Error::Uncertain);
    };
    context
        .persist(actor.transport.clone(), actor.id.clone(), record.clone())
        .await?;
    actor.observe(record);
    Ok(())
}
async fn control(context: Arc<Context>, actor: Arc<Attempt>) -> Result<()> {
    let prepared = fresh(&context, &actor).await?;
    let proof = context.describe(&prepared, &actor.transport).await?;
    let source = actor.transport.clone();
    let id = actor.id.clone();
    let stop = actor.clone();
    let command = context
        .authority(move |ctx, store, policy| {
            ctx.running()?;
            if stop.stop.load(Ordering::Acquire) {
                return Err(denied());
            }
            store.dispatch_remote_execution(
                &id,
                prepared,
                proof,
                source.fence().clone(),
                now()?,
                |store, owner, device, identity, lease, fence| {
                    ctx.running()?;
                    if stop.stop.load(Ordering::Acquire) {
                        return Err(denied());
                    }
                    ctx.caller(store, owner, device, policy)?;
                    source.attest_in_transaction(store, identity, lease, fence)
                },
            )
        })
        .await?
        .ok_or(Error::Uncertain)?;
    let sent = Instant::now();
    let ttl = Duration::from_millis(command.budgets.lease_duration_ms);
    let mut lease_deadline = sent + ttl;
    let reply = tokio::time::timeout_at(
        lease_deadline,
        actor.transport.request(Request::Execute {
            command: Box::new(command),
        }),
    )
    .await;
    let received = match reply {
        Ok(Ok(reply)) => receive_record(&context, &actor, reply).await.is_ok(),
        _ => false,
    };
    if !received {
        // Never resend Execute. Only the same live original channel may
        // reconcile an uncertain acceptance while the original safe lease
        // could still exist. Recovery/new-boot paths never enter this actor.
        if Instant::now() >= lease_deadline {
            return Err(Error::Uncertain);
        }
        let record = tokio::time::timeout_at(
            lease_deadline,
            context.reconcile(
                actor.owner.clone(),
                actor.device.clone(),
                actor.id.clone(),
                actor.transport.clone(),
            ),
        )
        .await
        .map_err(|_| Error::Uncertain)??
        .ok_or(Error::Uncertain)?;
        actor.observe(record);
    }
    let mut sequence = 0u64;
    loop {
        if actor.terminal() {
            return Ok(());
        }
        if actor.stop.load(Ordering::Acquire) || context.stop.load(Ordering::Acquire) {
            return Err(Error::Unauthorized);
        }
        let renew_at = lease_deadline - ttl.mul_f64(0.6);
        tokio::select! {_=tokio::time::sleep_until(renew_at)=>{},_=actor.changed.notified()=>{if actor.terminal(){return Ok(())}continue;}}
        if Instant::now() >= lease_deadline {
            return Err(Error::Uncertain);
        }
        let operation = async {
            let prepared = fresh(&context, &actor).await?;
            let proof = context.describe(&prepared, &actor.transport).await?;
            let source = actor.transport.clone();
            let id = actor.id.clone();
            let stop = actor.clone();
            sequence = sequence.checked_add(1).ok_or(Error::Conflict)?;
            let seq = sequence;
            let duration = actor.request.budgets.lease_duration_ms;
            let message = context
                .authority(move |ctx, store, policy| {
                    ctx.running()?;
                    if stop.stop.load(Ordering::Acquire) {
                        return Err(denied());
                    }
                    store.renew_remote_execution(
                        &id,
                        prepared,
                        proof,
                        source.fence().clone(),
                        now()?,
                        seq,
                        duration,
                        |store, owner, device, identity, lease, fence| {
                            ctx.running()?;
                            if stop.stop.load(Ordering::Acquire) {
                                return Err(denied());
                            }
                            ctx.caller(store, owner, device, policy)?;
                            source.attest_in_transaction(store, identity, lease, fence)
                        },
                    )
                })
                .await?;
            let sent = Instant::now();
            let Request::Renew {
                fence,
                sequence: expected,
                lease_duration_ms: expected_ttl,
            } = &message
            else {
                return Err(Error::Conflict);
            };
            match actor.transport.request(message.clone()).await {
                Ok(Reply::Renewed {
                    fence: actual,
                    sequence: ack,
                    lease_duration_ms: duration,
                }) if &actual == fence && ack == *expected && duration == *expected_ttl => {
                    Ok(sent + Duration::from_millis(duration))
                }
                _ => Err(Error::Uncertain),
            }
        };
        // A matching delayed duplicate ACK never extends from ACK arrival.
        lease_deadline = tokio::time::timeout_at(lease_deadline, operation)
            .await
            .map_err(|_| Error::Uncertain)??;
    }
}
async fn cancel_cached(actor: &Attempt) {
    let Some(fence) = actor.receipt() else { return };
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        if actor
            .transport
            .request(Request::Cancel {
                fence: fence.clone(),
            })
            .await
            .is_ok()
        {
            return;
        }
        if Instant::now() >= until {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn run_attempt(
    service: Weak<Service>,
    context: Arc<Context>,
    actor: Arc<Attempt>,
    _capacity: OwnedSemaphorePermit,
) {
    let deadline = Instant::now() + Duration::from_secs(actor.request.budgets.max_seconds);
    let terminal = {
        let exchange = control(context.clone(), actor.clone());
        let authority = monitor(context.clone(), actor.clone());
        tokio::pin!(exchange);
        tokio::pin!(authority);
        loop {
            tokio::select! {
                result=&mut exchange=>break result.is_ok()&&actor.terminal(),
                _=&mut authority=>break actor.terminal(),
                _=context.stopped.notified()=>break actor.terminal(),
                _=tokio::time::sleep_until(deadline)=>break actor.terminal(),
                _=actor.changed.notified()=>{if actor.terminal(){break true}else if actor.stop.load(Ordering::Acquire){break false}},
            }
        }
    }; // Drop positive RPC/SQL waiters before issuing negative cancellation.
    actor.stop.store(true, Ordering::Release);
    if terminal {
        let _ = context
            .acknowledge(actor.transport.clone(), actor.id.clone())
            .await;
    } else {
        // Independent of policy filesystem/SQL waits. A known original fence is
        // negative intent; no fresh grant is required to stop already owned work.
        let _ = tokio::time::timeout(Duration::from_secs(2), cancel_cached(&actor)).await;
        let id = actor.id.clone();
        let _ = context
            .sql(move |ctx, store| {
                store.cancel_remote_execution_from_guard(&id, |store, command| {
                    ctx.producer_scope(store, command)
                })?;
                store.mark_remote_execution_uncertain(&id, |store, command| {
                    ctx.producer_scope(store, command)
                })
            })
            .await;
    }
    if let Some(service) = service.upgrade()
        && let Ok(mut attempts) = service.attempts.lock()
        && attempts
            .get(&actor.id)
            .is_some_and(|current| Arc::ptr_eq(current, &actor))
    {
        attempts.remove(&actor.id);
    }
}
async fn events_loop(service: Weak<Service>, mut events: mpsc::Receiver<ExecutionEvent>) {
    while let Some(event) = events.recv().await {
        let Some(service) = service.upgrade() else {
            return;
        };
        let record = match event.message {
            Reply::Accepted { record } => Some(record),
            Reply::Outcome { fence, outcome } => {
                if let Ok(snapshot) = service.context.snapshot(fence.attempt_id.clone()).await {
                    if let Some(canonical) = snapshot.record {
                        if canonical.receipt.fence == fence {
                            let status=match outcome.status {
                                dunlin_relay_protocol::worker::ExecutionTerminalStatus::Completed=>dunlin_relay_protocol::worker::ExecutionStatus::Completed,
                                dunlin_relay_protocol::worker::ExecutionTerminalStatus::Failed=>dunlin_relay_protocol::worker::ExecutionStatus::Failed,
                                dunlin_relay_protocol::worker::ExecutionTerminalStatus::Cancelled=>dunlin_relay_protocol::worker::ExecutionStatus::Cancelled,
                                dunlin_relay_protocol::worker::ExecutionTerminalStatus::Unknown=>dunlin_relay_protocol::worker::ExecutionStatus::Unknown,
                            };
                            Some(ExecutionRecord {
                                receipt: canonical.receipt,
                                status,
                                outcome: Some(outcome),
                            })
                        } else {
                            None
                        }
                    } else {
                        service
                            .context
                            .reconcile_producer(fence.attempt_id, event.transport.clone())
                            .await
                            .ok()
                            .flatten()
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(record) = record else { continue };
        let id = record.receipt.identity.attempt_id.clone();
        if service
            .context
            .persist(event.transport.clone(), id.clone(), record.clone())
            .await
            .is_err()
        {
            continue;
        }
        if let Ok(attempts) = service.attempts.lock()
            && let Some(actor) = attempts.get(&id)
        {
            actor.observe(record.clone());
        }
        if record.outcome.is_some() {
            let _ = service.context.acknowledge(event.transport, id).await;
        }
    }
}
async fn cleanup_loop(service: Weak<Service>, context: Arc<Context>) {
    let mut after: Option<String> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {_=tick.tick()=>{},_=context.stopped.notified()=>return}
        if context.stop.load(Ordering::Acquire) || service.strong_count() == 0 {
            return;
        }
        let cursor = after.clone();
        let page = context
            .sql(move |ctx, store| {
                store.remote_execution_cleanup(8, cursor.as_deref(), |store, command| {
                    ctx.producer_scope(store, command)
                })
            })
            .await;
        let Ok(page) = page else { continue };
        if page.is_empty() {
            after = None;
            continue;
        }
        for snapshot in page {
            let id = snapshot.command.identity.attempt_id.clone();
            after = Some(id.clone());
            if let Ok(transport) = context
                .channels
                .execution_transport(&snapshot.command.descriptor.scope.host_id)
            {
                let _ = context.acknowledge(transport, id).await;
            }
        }
    }
}
#[cfg(test)]
mod tests;
