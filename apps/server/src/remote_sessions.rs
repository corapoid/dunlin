//! Persistent remote sessions. A browser never owns the actor, and neither a
//! SQL snapshot nor a replacement channel can recreate positive worker leases.
use crate::store::remote_bindings::resolver::sessions::{
    RemoteOpen, RemoteSessionRequest, RemoteSessionSnapshot, RemoteStreamCut,
};
use crate::{
    api::{AppState, AuthUser},
    remote_execution::{Context, Error, Result},
    store::{Store, StoreError},
    worker_channel::{
        Service as Channels, SessionEvent, SessionProducerRegistration, SessionTransport,
    },
};
use dunlin_relay_protocol::worker::session::{
    self as wire, SessionControllerMessage as Request, SessionWorkerMessage as Reply,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::Instant;
mod actor;
mod events;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Remote MCP producer integration is in progress")
)]
pub(crate) mod mcp;
mod turns;

fn denied() -> StoreError {
    dunlin_domain::collaboration::PolicyError::PermissionDenied.into()
}
fn now() -> crate::store::Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .ok_or_else(denied)
}
struct Actor {
    id: String,
    owner: String,
    lease: dunlin_session_runtime::Lease,
    transport: SessionTransport,
    deadline: Instant,
    lease_deadline: Mutex<Instant>,
    turn: Mutex<turns::Clock>,
    mcp: Mutex<mcp::State>,
    receipt: Mutex<Option<wire::SessionReceipt>>,
    stop: AtomicBool,
    finished: AtomicBool,
    changed: Notify,
    _permit: OwnedSemaphorePermit,
}
impl Actor {
    fn running(&self) -> crate::store::Result<()> {
        if self.stop.load(Ordering::Acquire)
            || Instant::now() >= self.deadline
            || self
                .lease_deadline
                .lock()
                .map_or(true, |deadline| Instant::now() >= *deadline)
        {
            return Err(denied());
        }
        self.turn
            .lock()
            .map_err(|_| denied())?
            .check(Instant::now())?;
        Ok(())
    }
    fn authorize(&self, snapshot: &RemoteSessionSnapshot) -> crate::store::Result<()> {
        self.running()?;
        if snapshot.session != self.id
            || snapshot.owner != self.owner
            || snapshot.lease.epoch != self.lease.epoch
            || snapshot.lease.instance != self.lease.instance
        {
            return Err(denied());
        }
        Ok(())
    }
}
pub(crate) struct Service {
    config: Arc<crate::config::Config>,
    context: Arc<Context>,
    channels: Arc<Channels>,
    node: String,
    instance: String,
    registration: Mutex<Option<SessionProducerRegistration>>,
    actors: Mutex<BTreeMap<String, Arc<Actor>>>,
    capacity: Arc<Semaphore>,
    creating: Arc<Semaphore>,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    closing: AtomicBool,
}
impl Service {
    pub(crate) async fn start(state: &AppState) -> Result<Arc<Self>> {
        if !state.config.store_transcripts {
            return Err(Error::Disabled);
        }
        let config = state
            .config
            .worker_controller
            .as_ref()
            .filter(|config| config.execution_enabled && config.persistent_sessions)
            .ok_or(Error::Disabled)?;
        let (sender, events) = mpsc::channel(32);
        let registration = state
            .worker_channels
            .register_session_producer(sender)
            .map_err(|_| Error::Unavailable)?;
        let context = Context::new(state);
        // Registration and receiver belong to the actual SQL job across a
        // timeout or dropped startup waiter, just as for the v3 controller.
        let (registration, events) = context
            .sql(move |_, store| {
                store.recover_remote_sessions()?;
                Ok((registration, events))
            })
            .await?;
        let service = Arc::new(Self {
            config: state.config.clone(),
            context,
            channels: state.worker_channels.clone(),
            node: config.node_id.clone(),
            instance: format!("remote-v4-{}", uuid::Uuid::now_v7()),
            registration: Mutex::new(Some(registration)),
            actors: Mutex::new(BTreeMap::new()),
            capacity: Arc::new(Semaphore::new(8)),
            creating: Arc::new(Semaphore::new(4)),
            tasks: Mutex::new(Vec::new()),
            closing: AtomicBool::new(false),
        });
        service.track(tokio::spawn(events::consume(
            Arc::downgrade(&service),
            events,
        )));
        service.track(tokio::spawn(events::cleanup(Arc::downgrade(&service))));
        Ok(service)
    }
    fn track(&self, task: tokio::task::JoinHandle<()>) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|task| !task.is_finished());
            tasks.push(task.abort_handle());
        } else {
            task.abort();
        }
    }
    fn scope(&self, snapshot: &RemoteSessionSnapshot) -> crate::store::Result<()> {
        // Source correlation and negative cleanup are independent of OAuth.
        // Positive MCP operations additionally require the sealed runtime proof.
        if snapshot.open.descriptor.scope.controller_node_id != self.node {
            return Err(denied());
        }
        Ok(())
    }
    fn source(
        &self,
        store: &Store,
        snapshot: &RemoteSessionSnapshot,
        transport: &SessionTransport,
        historical: bool,
    ) -> crate::store::Result<()> {
        self.scope(snapshot)?;
        let scope = &snapshot.open.descriptor.scope;
        let host = transport.identity().host();
        let (workspace, project) = store.interactive_scope(&snapshot.session)?;
        if workspace != snapshot.workspace
            || project != snapshot.project
            || workspace != scope.workspace_id
            || scope.host_id != transport.host_id()
            || scope.workspace_id != host.workspace_id
            || scope.worker_node_id != host.definition.node_id
            || scope.enrollment_epoch != host.enrollment_epoch
            || scope.credential_revision != host.credential_revision
            || (!historical && &snapshot.open.channel != transport.fence())
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
    async fn snapshot(&self, id: &str) -> Result<RemoteSessionSnapshot> {
        let id = id.to_owned();
        self.context
            .sql(move |_, store| store.remote_session_snapshot(&id))
            .await
    }
    async fn describe(
        &self,
        snapshot: &RemoteSessionSnapshot,
        transport: &SessionTransport,
    ) -> Result<crate::worker_channel::ObservedExport> {
        self.channels
            .describe_session_export(
                transport,
                &snapshot.open.descriptor.export.id,
                snapshot.open.descriptor.grant.principal.clone(),
                Some(snapshot.open.descriptor.export.revision),
            )
            .await
            .map_err(|_| Error::Unavailable)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_with_mcp(
        self: &Arc<Self>,
        state: &AppState,
        auth: &AuthUser,
        workspace: &str,
        project: &str,
        request: RemoteSessionRequest,
        key: &str,
        selections: Vec<crate::interactive::mcp::Selection>,
    ) -> Result<crate::interactive::CreateAcceptance> {
        if !self.context.matches_state(state) {
            return Err(Error::Unauthorized);
        }
        // The startup task owns this only until installation, with no root
        // back to Service. A dropped HTTP waiter cannot cancel committed setup.
        let setup = if selections.is_empty() {
            None
        } else {
            let mut private = state.clone();
            private.remote_sessions = Arc::new(std::sync::OnceLock::new());
            Some(private)
        };
        self.create_mode(auth, workspace, project, request, key, setup, selections)
            .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn create_mode(
        self: &Arc<Self>,
        auth: &AuthUser,
        workspace: &str,
        project: &str,
        request: RemoteSessionRequest,
        key: &str,
        setup: Option<AppState>,
        selections: Vec<crate::interactive::mcp::Selection>,
    ) -> Result<crate::interactive::CreateAcceptance> {
        if self.closing.load(Ordering::Acquire) || !crate::interactive::valid_admission_key(key) {
            return Err(Error::Unavailable);
        }
        let owner = auth.user.id.clone();
        let device = auth.device.id.clone();
        let lookup_workspace = workspace.to_owned();
        let lookup_project = project.to_owned();
        let key = key.to_owned();
        let lookup_key = key.clone();
        let node = self.node.clone();
        let config = self.config.clone();
        let lookup_request = request.clone();
        let lookup_selections = selections.clone();
        let (host, receipt) = self
            .context
            .authority(move |ctx, store, host_accounts| {
                ctx.running()?;
                store.remote_binding_transaction(false, |store| {
                    ctx.caller(store, &owner, &device, host_accounts)?;
                    let receipt = store.lookup_remote_session_request_in_transaction(
                        &owner,
                        &device,
                        &lookup_workspace,
                        &lookup_project,
                        &lookup_request,
                        &node,
                        &config,
                        &lookup_selections,
                        &lookup_key,
                        |store, owner, device| ctx.caller(store, owner, device, host_accounts),
                    )?;
                    let host = if receipt.is_some() {
                        None
                    } else {
                        Some(
                            store
                                .capture_remote_target_in_transaction(
                                    &owner,
                                    &lookup_workspace,
                                    &lookup_project,
                                    &lookup_request.binding,
                                    &node,
                                )?
                                .host_id()
                                .to_owned(),
                        )
                    };
                    Ok((host, receipt))
                })
            })
            .await?;
        if let Some(session) = receipt {
            return Ok(crate::interactive::CreateAcceptance {
                session,
                replayed: true,
            });
        }
        let creating = self
            .creating
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let transport = self
            .channels
            .session_transport(host.as_deref().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        let owner = auth.user.id.clone();
        let device = auth.device.id.clone();
        let workspace = workspace.to_owned();
        let project = project.to_owned();
        let node = self.node.clone();
        let config = self.config.clone();
        let channel = transport.clone();
        let prepared = self
            .context
            .authority(move |ctx, store, policy| {
                ctx.running()?;
                store.remote_binding_transaction(false, |store| {
                    ctx.caller(store, &owner, &device, policy)?;
                    if selections.is_empty() {
                        store.prepare_remote_session_in_transaction(
                            &owner, &device, &workspace, &project, request, &node,
                        )
                    } else {
                        store.prepare_remote_mcp_session_in_transaction(
                            &owner,
                            &device,
                            &workspace,
                            &project,
                            request,
                            &node,
                            crate::store::remote_bindings::resolver::sessions::RemoteMcpAdmission {
                                config: &config,
                                selections: &selections,
                                transport: channel,
                            },
                        )
                    }
                })
            })
            .await?;
        let proof = self
            .channels
            .describe_session_export(
                &transport,
                prepared.export_id(),
                prepared.principal(),
                Some(prepared.export_revision()),
            )
            .await
            .map_err(|_| Error::Unavailable)?;
        let service = self.clone();
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::Unavailable)?;
        self.context
            .authority(move |ctx, store, host_accounts| {
                let _creating = creating;
                ctx.running()?;
                let admitted = store.remote_binding_transaction(false, |store| {
                    store.admit_remote_session_in_transaction(
                        prepared,
                        proof,
                        transport.fence().clone(),
                        &key,
                        &service.instance,
                        now()?,
                        |store, owner, device, identity, lease, fence| {
                            ctx.running()?;
                            ctx.caller(store, owner, device, host_accounts)?;
                            transport.attest_in_transaction(store, identity, lease, fence)
                        },
                    )
                })?;
                let replayed = admitted.command.is_none();
                if let Some(command) = admitted.command {
                    let actor = Arc::new(Actor {
                        id: command.controller_session_id.clone(),
                        owner: command.descriptor.grant.principal.id.clone(),
                        lease: admitted.admission.lease.clone().ok_or_else(denied)?,
                        transport,
                        deadline: Instant::now() + Duration::from_secs(command.budgets.max_seconds),
                        lease_deadline: Mutex::new(
                            Instant::now()
                                + Duration::from_millis(command.budgets.lease_duration_ms),
                        ),
                        turn: Mutex::new(turns::Clock::default()),
                        mcp: Mutex::new(mcp::State::default()),
                        receipt: Mutex::new(None),
                        stop: AtomicBool::new(
                            service.closing.load(Ordering::Acquire) || ctx.is_stopped(),
                        ),
                        finished: AtomicBool::new(false),
                        changed: Notify::new(),
                        _permit: permit,
                    });
                    let registered = service
                        .actors
                        .lock()
                        .map(|mut actors| {
                            if actors.contains_key(&actor.id) || actors.len() >= 8 {
                                false
                            } else {
                                actors.insert(actor.id.clone(), actor.clone());
                                true
                            }
                        })
                        .unwrap_or(false);
                    if !registered {
                        store.remote_binding_transaction(false, |store| {
                            store.interrupt_remote_session_in_transaction(
                                &actor.id,
                                |_, snapshot| service.scope(snapshot),
                            )
                        })?;
                        return Err(denied());
                    }
                    // The actual blocking admission job owns registration and spawn,
                    // even if its HTTP waiter times out after the database commit.
                    service.track(runtime.spawn(actor::run(
                        Arc::downgrade(&service),
                        actor,
                        setup,
                    )));
                    store
                        .interactive_journal()
                        .notify_committed_admission()
                        .map_err(|_| denied())?;
                }
                Ok(crate::interactive::CreateAcceptance {
                    session: admitted.admission.session,
                    replayed,
                })
            })
            .await
    }
    pub(crate) fn queue_allowed(
        &self,
        state: &AppState,
        store: &Store,
        id: &str,
        owner: &str,
    ) -> bool {
        if self.closing.load(Ordering::Acquire)
            || self.context.is_stopped()
            || !self.context.matches_state(state)
        {
            return false;
        }
        let actor = self
            .actors
            .lock()
            .ok()
            .and_then(|actors| actors.get(id).cloned());
        let Some(actor) = actor else {
            return false;
        };
        if actor.owner != owner || actor.running().is_err() {
            return false;
        }
        store
            .remote_binding_transaction(false, |store| {
                let mut scope = mcp::scope(&actor)?;
                if scope.is_none() {
                    let snapshot = store.remote_session_snapshot(id)?;
                    if matches!(&snapshot.open, RemoteOpen::Broker(_)) {
                        // Accept queued input while the actual startup task
                        // prepares its broker. This temporary proof is never
                        // installed on Actor and cannot authorize its dispatch.
                        if snapshot.phase != "pending" || snapshot.dispatched || snapshot.receipt.is_some() {
                            return Err(denied());
                        }
                        scope = Some(Arc::new(crate::store::remote_bindings::resolver::sessions::authority::RemoteMcpScope::capture(
                            store, &self.config, id, &actor.lease, actor.transport.clone(), actor.deadline.into_std(),
                            |_, user, device| {
                                actor.authorize(&snapshot)?;
                                if user != owner || device != snapshot.device { return Err(denied()); }
                                Ok(())
                            },
                        )?));
                    }
                }
                let runtime = scope
                    .as_ref()
                    .map(|scope| scope.runtime(store, &self.config))
                    .unwrap_or_else(|| store.remote_session_runtime());
                runtime.remote_session_queue_allowed_in_transaction(id, owner, |store, snapshot| {
                    actor.authorize(snapshot)?;
                    self.source(store, snapshot, &actor.transport, false)?;
                    let user = store
                        .active_device_user(&snapshot.device)?
                        .filter(|user| user.id == snapshot.owner)
                        .ok_or_else(denied)?;
                    if !store.oidc_device_allowed(&snapshot.device, state.config.oidc.as_ref())?
                        || user.id != owner
                    {
                        return Err(denied());
                    }
                    Ok(())
                })
            })
            .is_ok()
    }
    /// Only an installed MCP factory enables remote tool consent. A temporary
    /// pending-queue proof and ordinary ACP remote sessions cannot enable it.
    pub(crate) fn approvals_allowed(
        &self,
        state: &AppState,
        store: &Store,
        id: &str,
        owner: &str,
    ) -> bool {
        if !self.queue_allowed(state, store, id, owner) {
            return false;
        }
        self.actors
            .lock()
            .ok()
            .and_then(|actors| actors.get(id).cloned())
            .is_some_and(|actor| mcp::scope(&actor).is_ok_and(|scope| scope.is_some()))
    }
    pub(crate) fn signal_stop(&self, id: &str) {
        if let Ok(actors) = self.actors.lock()
            && let Some(actor) = actors.get(id)
        {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
        }
    }
    pub(crate) fn signal_cancellation(
        &self,
        store: &Store,
        id: &str,
        owner: &str,
        context: &dunlin_domain::collaboration::AccessContext,
        turn: &str,
    ) {
        let Ok(snapshot) = store.remote_session_snapshot(id) else {
            return;
        };
        if snapshot.owner == owner
            && store
                .interactive_journal()
                .cancellation_requested(context, id, &snapshot.lease, turn)
                .unwrap_or(false)
        {
            self.signal_stop(id);
        }
    }
    fn finished(&self, id: &str) {
        if let Ok(mut actors) = self.actors.lock()
            && let Some(actor) = actors.remove(id)
        {
            actor.finished.store(true, Ordering::Release);
            if let Ok(mut mcp) = actor.mcp.lock() {
                mcp.close();
            }
            actor.changed.notify_waiters();
        }
    }
    pub(crate) async fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        self.context.request_stop();
        if let Ok(actors) = self.actors.lock() {
            for actor in actors.values() {
                actor.stop.store(true, Ordering::Release);
                actor.changed.notify_waiters();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(12);
        while self.actors.lock().is_ok_and(|actors| !actors.is_empty()) && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Ok(mut registration) = self.registration.lock() {
            registration.take();
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.context.request_stop();
        if let Ok(registration) = self.registration.get_mut() {
            registration.take();
        }
        if let Ok(tasks) = self.tasks.get_mut() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests;
