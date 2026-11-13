//! Controller-owned adapter. Wire/SQL alone cannot create this proof. The host
//! bridge also detaches AppState's remote-service root to avoid ownership cycles.
use super::*;
use crate::interactive::mcp::authority::{Authority, SessionPins, SqlAuthority, TurnScope};
use crate::store::remote_bindings::resolver::sessions::authority::RemoteMcpScope;
use dunlin_domain::collaboration::AccessContext;
type McpResult<T> = std::result::Result<T, &'static str>;
mod bridge;
pub(super) use bridge::State;
pub(super) fn scope(actor: &Actor) -> crate::store::Result<Option<Arc<RemoteMcpScope>>> {
    actor
        .mcp
        .lock()
        .map(|state| state.scope())
        .map_err(|_| denied())
}
pub(super) async fn install(
    state: &AppState,
    service: &Arc<Service>,
    actor: &Arc<Actor>,
) -> McpResult<()> {
    bridge::install(state, service, actor).await
}

pub(crate) struct Remote {
    scope: Arc<RemoteMcpScope>,
    service: Weak<Service>,
    actor: Weak<Actor>,
    runtime: tokio::runtime::Handle,
}
impl Remote {
    pub(super) async fn capture(
        state: &AppState,
        service: &Arc<Service>,
        actor: &Arc<Actor>,
    ) -> McpResult<Arc<Self>> {
        if !service.context.matches_state(state) || service.closing.load(Ordering::Acquire) {
            return Err("mcp_service_changed");
        }
        let current = actor.clone();
        let owner = service.clone();
        let config = state.config.clone();
        let scope = service
            .context
            .authority(move |ctx, store, policy| {
                ctx.running()?;
                current.running()?;
                store.remote_binding_transaction(false, |store| {
                    RemoteMcpScope::capture(
                        store,
                        &config,
                        &current.id,
                        &current.lease,
                        current.transport.clone(),
                        current.deadline.into_std(),
                        |store, user, device| {
                            ctx.running()?;
                            current.running()?;
                            if owner.closing.load(Ordering::Acquire) {
                                return Err(denied());
                            }
                            ctx.caller(store, user, device, policy)
                        },
                    )
                })
            })
            .await
            .map_err(|_| "mcp_capture_denied")?;
        let adapter = Arc::new(Self {
            scope: Arc::new(scope),
            service: Arc::downgrade(service),
            actor: Arc::downgrade(actor),
            runtime: tokio::runtime::Handle::try_current()
                .map_err(|_| "mcp_runtime_unavailable")?,
        });
        adapter.live(Some(state))?;
        Ok(adapter)
    }
    fn live(&self, state: Option<&AppState>) -> McpResult<(Arc<Service>, Arc<Actor>)> {
        let service = self.service.upgrade().ok_or("mcp_service_stopped")?;
        let actor = self.actor.upgrade().ok_or("mcp_actor_stopped")?;
        service
            .context
            .running()
            .map_err(|_| "mcp_service_stopped")?;
        actor.running().map_err(|_| "mcp_actor_stopped")?;
        let pins = self.scope.pins();
        if service.closing.load(Ordering::Acquire)
            || actor.finished.load(Ordering::Acquire)
            || state.is_some_and(|state| !service.context.matches_state(state))
            || actor.id != pins.session
            || actor.owner != pins.owner
            || actor.lease.epoch != pins.lease.epoch
            || actor.lease.instance != pins.lease.instance
        {
            return Err("mcp_actor_changed");
        }
        Ok((service, actor))
    }
    /// Correlation only; the common broker still obtains one-use SDK consent
    /// and checks fresh source/credentials before and after external HTTP.
    pub(crate) async fn validate_request(
        &self,
        request: wire::broker::BrokerScope,
        state: &AppState,
    ) -> McpResult<()> {
        if self.request_ready(request, state).await? {
            Ok(())
        } else {
            Err("mcp_native_receipt_pending")
        }
    }
    async fn request_ready(
        &self,
        request: wire::broker::BrokerScope,
        state: &AppState,
    ) -> McpResult<bool> {
        let (service, actor) = self.live(Some(state))?;
        actor
            .turn
            .lock()
            .map_err(|_| "mcp_actor_stopped")?
            .matches_scope(&request, Instant::now())
            .map_err(|_| "mcp_turn_changed")?;
        let config = state.config.clone();
        let scope = self.scope.clone();
        let owner = service.clone();
        let live_request = request.clone();
        let ready = service
            .context
            .authority(move |ctx, store, policy| {
                store.remote_binding_transaction(false, |store| {
                    scope
                        .request_ready_in_transaction(
                            store,
                            &config,
                            &request,
                            |store, user, device| {
                                ctx.running()?;
                                actor.running()?;
                                actor
                                    .turn
                                    .lock()
                                    .map_err(|_| denied())?
                                    .matches_scope(&request, Instant::now())?;
                                if owner.closing.load(Ordering::Acquire) {
                                    return Err(denied());
                                }
                                ctx.caller(store, user, device, policy)
                            },
                        )
                        .map(|context| context.is_some())
                })
            })
            .await
            .map_err(|_| "mcp_request_denied")?;
        let (_, actor) = self.live(Some(state))?;
        actor
            .turn
            .lock()
            .map_err(|_| "mcp_actor_stopped")?
            .matches_scope(&live_request, Instant::now())
            .map_err(|_| "mcp_turn_changed")?;
        Ok(ready)
    }
    async fn wait_request(
        &self,
        request: wire::broker::BrokerScope,
        state: &AppState,
        deadline: Instant,
    ) -> McpResult<()> {
        let deadline = deadline.min(Instant::now() + Duration::from_secs(2));
        loop {
            let (_, actor) = self.live(Some(state))?;
            let changed = actor.changed.notified();
            if Instant::now() >= deadline {
                return Err("mcp_native_receipt_deadline");
            }
            let ready =
                tokio::time::timeout_at(deadline, self.request_ready(request.clone(), state))
                    .await
                    .map_err(|_| "mcp_native_receipt_deadline")??;
            if ready {
                return Ok(());
            }
            tokio::select! {
                _=changed=>{},
                _=tokio::time::sleep_until(deadline)=>return Err("mcp_native_receipt_deadline"),
                _=tokio::time::sleep(Duration::from_millis(50))=>{},
            }
        }
    }
}

struct SqlCut<'a> {
    remote: &'a Remote,
    state: &'a AppState,
    policy: bool,
    _management: std::sync::MutexGuard<'a, crate::management::Management>,
    _slot: OwnedSemaphorePermit,
}
impl SqlAuthority for SqlCut<'_> {
    fn permission_device(
        &self,
        store: &Store,
        owner: &str,
        decision: &dunlin_session_runtime::PermissionDecision,
    ) -> McpResult<()> {
        let (service, actor) = self.remote.live(Some(self.state))?;
        if owner != self.remote.pins().owner {
            return Err("mcp_permission_owner_changed");
        }
        service
            .context
            .caller(store, owner, &decision.device_id, self.policy)
            .map_err(|_| "mcp_permission_device_revoked")?;
        actor.running().map_err(|_| "mcp_actor_stopped")
    }
    fn context(&self, store: &Store, in_transaction: bool) -> McpResult<AccessContext> {
        let (service, actor) = self.remote.live(Some(self.state))?;
        let caller = |store: &Store, user: &str, device: &str| {
            service.context.running()?;
            actor.running()?;
            if service.closing.load(Ordering::Acquire) {
                return Err(denied());
            }
            service.context.caller(store, user, device, self.policy)
        };
        let scope = &self.remote.scope;
        let result = if in_transaction {
            scope.context_in_transaction(store, &self.state.config, caller)
        } else {
            scope.context(store, &self.state.config, caller)
        };
        let context = result.map_err(|_| "mcp_authority_changed")?;
        actor.running().map_err(|_| "mcp_actor_stopped")?;
        Ok(context)
    }
}
impl Authority for Remote {
    fn pins(&self) -> &SessionPins {
        self.scope.pins()
    }
    /// Called by the common broker only on blocking workers / its socket
    /// thread, never on the WebSocket owner or a Tokio async worker.
    fn check(&self, state: &AppState, deadline: Instant) -> McpResult<()> {
        let (service, actor) = self.live(Some(state))?;
        let deadline = deadline.min(actor.deadline);
        if Instant::now() >= deadline {
            return Err("mcp_scope_expired");
        }
        let scope = self.scope.clone();
        let config = state.config.clone();
        self.runtime.block_on(async move {
            tokio::time::timeout_at(deadline, async move {
                let snapshot = service.snapshot(&actor.id).await?;
                actor
                    .authorize(&snapshot)
                    .map_err(|_| Error::Unauthorized)?;
                let proof = service.describe(&snapshot, &actor.transport).await?;
                let owner = service.clone();
                service
                    .context
                    .authority(move |ctx, store, policy| {
                        if Instant::now() >= deadline {
                            return Err(denied());
                        }
                        store.remote_binding_transaction(false, |store| {
                            scope
                                .observe_in_transaction(
                                    store,
                                    &config,
                                    proof,
                                    now()?,
                                    |store, user, device| {
                                        ctx.running()?;
                                        actor.running()?;
                                        if owner.closing.load(Ordering::Acquire)
                                            || Instant::now() >= deadline
                                        {
                                            return Err(denied());
                                        }
                                        ctx.caller(store, user, device, policy)
                                    },
                                )
                                .map(|_| ())
                        })
                    })
                    .await
            })
            .await
            .map_err(|_| "mcp_source_deadline")?
            .map_err(|_| "mcp_source_changed")
        })?;
        self.live(Some(state))?;
        if Instant::now() >= deadline {
            return Err("mcp_scope_expired");
        }
        Ok(())
    }
    fn sql<'a>(&'a self, state: &'a AppState) -> McpResult<Box<dyn SqlAuthority + 'a>> {
        let (service, _) = self.live(Some(state))?;
        let slot = service
            .context
            .policy_slot()
            .map_err(|_| "mcp_authority_busy")?;
        let management = state
            .management
            .lock()
            .map_err(|_| "mcp_policy_unavailable")?;
        let policy = crate::remote_execution::host_accounts(&state.config);
        self.live(Some(state))?;
        Ok(Box::new(SqlCut {
            remote: self,
            state,
            policy,
            _management: management,
            _slot: slot,
        }))
    }
    fn context(&self, _: &AppState, _: &Store) -> McpResult<AccessContext> {
        Err("mcp_sql_cut_required")
    }
    fn context_in_transaction(&self, _: &AppState, _: &Store) -> McpResult<AccessContext> {
        Err("mcp_sql_cut_required")
    }
    fn turn(&self) -> McpResult<TurnScope> {
        let (_, actor) = self.live(None)?;
        let (id, deadline) = actor
            .turn
            .lock()
            .map_err(|_| "mcp_actor_stopped")?
            .scope(Instant::now())
            .map_err(|_| "mcp_call_outside_turn")?;
        Ok(TurnScope { id, deadline })
    }
    fn changed(&self, store: &Store) {
        if let Some(actor) = self.actor.upgrade() {
            if store
                .interactive_journal()
                .notify_committed_update()
                .is_err()
            {
                actor.stop.store(true, Ordering::Release);
            }
            actor.changed.notify_waiters();
        }
    }
    fn interrupt(&self) {
        if let Some(actor) = self.actor.upgrade() {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
        }
    }
}

/// Controlled fixture owns an actual service/actor and captures the production
/// adapter. It deliberately does not dispatch Open/Submit or pretend to be WSS.
#[cfg(test)]
pub(crate) struct FixtureActor {
    pub(crate) authority: Arc<Remote>,
    actor: Arc<Actor>,
    service: Arc<Service>,
    cleanup: bool,
}
#[cfg(test)]
impl FixtureActor {
    pub(crate) async fn attach(
        state: &AppState,
        service: &Arc<Service>,
        admission: &crate::store::remote_bindings::resolver::sessions::RemoteSessionAdmission,
        transport: SessionTransport,
    ) -> McpResult<Self> {
        let command = admission
            .command
            .as_ref()
            .ok_or("fixture_requires_fresh_admission")?;
        let actor = Arc::new(Actor {
            id: admission.admission.session.id.clone(),
            owner: admission.admission.session.owner.clone(),
            lease: admission
                .admission
                .lease
                .clone()
                .ok_or("fixture_requires_original_lease")?,
            transport,
            deadline: Instant::now() + Duration::from_secs(command.budgets.max_seconds),
            lease_deadline: Mutex::new(
                Instant::now() + Duration::from_millis(command.budgets.lease_duration_ms),
            ),
            turn: Mutex::new(turns::Clock::default()),
            mcp: Mutex::new(State::default()),
            receipt: Mutex::new(None),
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            changed: Notify::new(),
            _permit: service
                .capacity
                .clone()
                .try_acquire_owned()
                .map_err(|_| "fixture_capacity")?,
        });
        let authority = Remote::capture(state, service, &actor).await?;
        service
            .actors
            .lock()
            .unwrap()
            .insert(actor.id.clone(), actor.clone());
        Ok(Self {
            authority,
            actor,
            service: service.clone(),
            cleanup: true,
        })
    }
    pub(crate) async fn dispatch_open(&self) -> McpResult<RemoteOpen> {
        match actor::positive(&self.service, &self.actor, actor::Cut::Open).await {
            Ok(actor::Output::Open(command)) => Ok(command),
            _ => Err("fixture_open_denied"),
        }
    }
    pub(crate) async fn claim_submit(&self) -> McpResult<wire::TurnCommand> {
        let actor::Output::Claim(Some(command)) =
            actor::positive(&self.service, &self.actor, actor::Cut::Claim)
                .await
                .map_err(|_| "fixture_claim_denied")?
        else {
            return Err("fixture_claim_missing");
        };
        self.begin_turn(command.clone(), Instant::now());
        let actor::Output::Request(Request::SubmitTurn { command }) = actor::positive(
            &self.service,
            &self.actor,
            actor::Cut::Submit(command.identity.turn_id),
        )
        .await
        .map_err(|_| "fixture_submit_denied")?
        else {
            return Err("fixture_submit_missing");
        };
        Ok(*command)
    }
    pub(crate) async fn persist(&self, message: Reply) -> McpResult<()> {
        events::persist(&self.service, self.actor.transport.clone(), message)
            .await
            .map_err(|_| "fixture_receipt_denied")
    }
    pub(crate) async fn renew(&self) -> McpResult<Request> {
        match actor::positive(&self.service, &self.actor, actor::Cut::Renew).await {
            Ok(actor::Output::Request(message)) => Ok(message),
            _ => Err("fixture_renew_denied"),
        }
    }
    pub(crate) fn begin_turn(&self, command: wire::TurnCommand, started: Instant) {
        self.actor
            .turn
            .lock()
            .unwrap()
            .begin(command, self.actor.deadline, started)
            .unwrap();
    }
    pub(crate) async fn install(&self, state: &AppState) -> McpResult<()> {
        install(state, &self.service, &self.actor).await
    }
    pub(crate) fn weak_broker(&self) -> Weak<dyn crate::worker_channel::SessionBroker> {
        self.actor.mcp.lock().unwrap().weak_owner()
    }
    pub(crate) fn abandon(mut self) {
        self.cleanup = false;
    }
    pub(crate) fn changed(&self) {
        self.actor.changed.notify_waiters();
    }
}
#[cfg(test)]
impl Drop for FixtureActor {
    fn drop(&mut self) {
        if !self.cleanup {
            return;
        }
        self.actor.stop.store(true, Ordering::Release);
        self.service.finished(&self.actor.id);
    }
}
