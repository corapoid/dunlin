//! The production controller producer: private account binding -> common broker
//! -> SDK consent -> MCP HTTP. The runner receives only bounded protocol JSON.
use super::*;
use crate::{
    interactive::mcp::{BoundSession, runtime::Broker},
    worker_channel::{BrokerRegistration, SessionBroker},
};
use std::{future::Future, pin::Pin};
use wire::broker::{BrokerError, BrokerLimits, BrokerOperation, BrokerRequest};

#[derive(Default)]
pub(in crate::remote_sessions) struct State {
    started: bool,
    binding: Option<BrokerRegistration>,
    scope: Option<Arc<RemoteMcpScope>>,
}
impl State {
    pub(in crate::remote_sessions) fn scope(&self) -> Option<Arc<RemoteMcpScope>> {
        self.scope.clone()
    }

    pub(in crate::remote_sessions) fn close(&mut self) {
        self.binding.take();
        self.scope.take();
    }
    #[cfg(test)]
    pub(in crate::remote_sessions) fn weak_owner(&self) -> Weak<dyn SessionBroker> {
        self.binding.as_ref().unwrap().weak_owner()
    }
}
struct Setup {
    actor: Weak<Actor>,
    completed: bool,
}
impl Drop for Setup {
    fn drop(&mut self) {
        if !self.completed
            && let Some(actor) = self.actor.upgrade()
        {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
        }
    }
}
pub(in crate::remote_sessions) async fn install(
    state: &AppState,
    service: &Arc<Service>,
    actor: &Arc<Actor>,
) -> McpResult<()> {
    actor.running().map_err(|_| "mcp_actor_stopped")?;
    {
        let mut state = actor.mcp.lock().map_err(|_| "mcp_setup_unavailable")?;
        if state.started {
            return Err("mcp_setup_already_started");
        }
        state.started = true;
    }
    let mut setup = Setup {
        actor: Arc::downgrade(actor),
        completed: false,
    };
    let authority = Remote::capture(state, service, actor).await?;
    let current = actor.clone();
    let owner = service.clone();
    let config = state.config.clone();
    let scope = authority.scope.clone();
    let (connections, limits) = service
        .context
        .authority(move |ctx, store, policy| {
            store.remote_binding_transaction(false, |store| {
                scope.context_in_transaction(store, &config, |store, user, device| {
                    ctx.running()?;
                    current.running()?;
                    if owner.closing.load(Ordering::Acquire) {
                        return Err(denied());
                    }
                    ctx.caller(store, user, device, policy)
                })?;
                let snapshot = store.remote_session_snapshot(&current.id)?;
                current.authorize(&snapshot)?;
                let crate::store::remote_bindings::resolver::sessions::RemoteOpen::Broker(open) =
                    &snapshot.open
                else {
                    return Err(denied());
                };
                let bound = BoundSession::restore(
                    &config,
                    store,
                    &snapshot.owner,
                    &snapshot.workspace,
                    &open.open.definition,
                    snapshot.mcp_binding.as_deref().ok_or_else(denied)?,
                )
                .map_err(|_| denied())?;
                Ok((bound.into_connections(), open.broker_limits.clone()))
            })
        })
        .await
        .map_err(|_| "mcp_binding_changed")?;
    // Retaining the original OnceLock would create:
    // Actor -> registration -> Hosted/Core -> AppState -> Service -> Actor.
    let mut private_state = state.clone();
    private_state.remote_sessions = Arc::new(std::sync::OnceLock::new());
    let broker = Broker::prepare(
        private_state.clone(),
        authority.clone(),
        connections,
        actor.deadline,
    )
    .await?;
    broker.validate_reply_limit(limits.max_reply_bytes as usize)?;
    let scope = authority.scope.clone();
    let handler = Arc::new(Hosted {
        work: Arc::new(Semaphore::new(usize::from(limits.max_pending))),
        state: private_state,
        authority,
        broker,
        limits,
    });
    actor.running().map_err(|_| "mcp_actor_stopped")?;
    let binding = actor
        .transport
        .bind_broker(handler)
        .map_err(|_| "mcp_bind_unavailable")?;
    let mut state = actor.mcp.lock().map_err(|_| "mcp_setup_unavailable")?;
    actor.running().map_err(|_| "mcp_actor_stopped")?;
    state.binding = Some(binding);
    state.scope = Some(scope);
    setup.completed = true;
    Ok(())
}
struct Hosted {
    state: AppState,
    authority: Arc<Remote>,
    broker: Arc<Broker>,
    limits: BrokerLimits,
    work: Arc<Semaphore>,
}
impl SessionBroker for Hosted {
    fn session(&self) -> &str {
        &self.authority.pins().session
    }
    fn is_live(&self) -> bool {
        self.authority.live(Some(&self.state)).is_ok()
    }
    fn interrupt(&self) {
        self.authority.interrupt();
    }
    fn request(
        self: Arc<Self>,
        request: BrokerRequest,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, BrokerError>> + Send>> {
        Box::pin(async move {
            request
                .validate()
                .map_err(|_| BrokerError::InvalidRequest)?;
            let slot = self
                .work
                .clone()
                .try_acquire_owned()
                .map_err(|_| BrokerError::CapacityReached)?;
            if self
                .authority
                .wait_request(request.scope.clone(), &self.state, deadline)
                .await
                .is_err()
            {
                self.interrupt();
                return Err(BrokerError::Denied);
            }
            let (method, params) = match &request.operation {
                BrokerOperation::ToolsList { cursor } => (
                    "tools/list",
                    cursor.as_ref().map_or_else(
                        || serde_json::json!({}),
                        |cursor| serde_json::json!({"cursor":cursor}),
                    ),
                ),
                BrokerOperation::ToolsCall {
                    name,
                    arguments_json,
                } => (
                    "tools/call",
                    serde_json::json!({"name":name,"arguments":serde_json::from_str::<serde_json::Value>(arguments_json).map_err(|_| BrokerError::InvalidRequest)?}),
                ),
            };
            let active = self.clone();
            let id = request.request_id;
            let work = tokio::task::spawn_blocking(move || {
                // Owned by actual work, including after its async waiter drops.
                let _slot = slot;
                active.broker.request_bounded(
                    &id,
                    method,
                    params,
                    deadline,
                    active.limits.max_reply_bytes as usize,
                )
            });
            let result = tokio::time::timeout_at(deadline, work).await;
            let value = match result {
                Ok(Ok(Ok(value))) => value,
                // These codes originate before the common broker can consume
                // consent/dispatch MCP. Let the agent correct a fresh request.
                Ok(Ok(Err(
                    "mcp_invalid_parameters"
                    | "mcp_invalid_cursor"
                    | "mcp_tool_not_allowed"
                    | "mcp_arguments_invalid"
                    | "mcp_request_invalid"
                    | "mcp_method_unavailable"
                    | "mcp_request_changed"
                    | "mcp_request_already_used"
                    | "mcp_request_limit",
                ))) => return Err(BrokerError::InvalidRequest),
                _ => {
                    self.interrupt();
                    return Err(BrokerError::OutcomeUnknown);
                }
            };
            if self
                .authority
                .validate_request(request.scope, &self.state)
                .await
                .is_err()
                || Instant::now() >= deadline
            {
                self.interrupt();
                return Err(BrokerError::OutcomeUnknown);
            }
            let payload = serde_json::to_string(&value).map_err(|_| BrokerError::OutcomeUnknown)?;
            if payload.len() > self.limits.max_reply_bytes as usize {
                self.interrupt();
                return Err(BrokerError::OutcomeUnknown);
            }
            Ok(payload)
        })
    }
}
