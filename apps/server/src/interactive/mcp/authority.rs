//! Host-owned session authority. Transport messages cannot construct this proof.
//! A remote adapter must attest its live worker fence/export in both SQL cuts;
//! the broker itself retains ownership of OAuth and one-use human approvals.
use super::*;

#[derive(Clone)]
pub(crate) struct SessionPins {
    pub session: String,
    pub workspace: String,
    pub owner: String,
    pub lease: Lease,
    pub starter_device: String,
    pub binding_revision: u64,
    pub profile_digest: String,
}
impl SessionPins {
    pub fn matches(&self, other: &Self) -> bool {
        self.session == other.session
            && self.workspace == other.workspace
            && self.owner == other.owner
            && self.lease.epoch == other.lease.epoch
            && self.lease.instance == other.lease.instance
            && self.starter_device == other.starter_device
            && self.binding_revision == other.binding_revision
            && self.profile_digest == other.profile_digest
    }
}
pub(crate) struct TurnScope {
    pub id: String,
    pub deadline: Instant,
}
/// Acquired before Store. A remote implementation retains the management lock
/// and its filesystem policy through the entire SQL/OAuth cut.
pub(crate) trait SqlAuthority {
    fn context(&self, store: &Store, in_transaction: bool) -> Result<AccessContext>;
    fn permission_device(
        &self,
        store: &Store,
        owner: &str,
        decision: &dunlin_session_runtime::PermissionDecision,
    ) -> Result<()>;
}
struct DirectSql<'a, A: Authority + ?Sized> {
    authority: &'a A,
    state: &'a AppState,
}
impl<A: Authority + ?Sized> SqlAuthority for DirectSql<'_, A> {
    fn permission_device(
        &self,
        store: &Store,
        owner: &str,
        decision: &dunlin_session_runtime::PermissionDecision,
    ) -> Result<()> {
        permission_device_allowed(self.state, store, owner, decision)
    }
    fn context(&self, store: &Store, in_transaction: bool) -> Result<AccessContext> {
        if in_transaction {
            self.authority.context_in_transaction(self.state, store)
        } else {
            self.authority.context(self.state, store)
        }
    }
}
pub(crate) trait Authority: Send + Sync {
    fn pins(&self) -> &SessionPins;
    /// Includes filesystem/transport checks outside the Store mutex.
    fn check(&self, state: &AppState, deadline: Instant) -> Result<()>;
    /// Fresh SQL cut, outside any caller-owned SQL transaction.
    fn context(&self, state: &AppState, store: &Store) -> Result<AccessContext>;
    /// Used within the OAuth journal's IMMEDIATE transaction; never start a
    /// nested authority transaction or perform network/filesystem work here.
    fn context_in_transaction(&self, state: &AppState, store: &Store) -> Result<AccessContext>;
    fn sql<'a>(&'a self, state: &'a AppState) -> Result<Box<dyn SqlAuthority + 'a>> {
        Ok(Box::new(DirectSql {
            authority: self,
            state,
        }))
    }
    fn turn(&self) -> Result<TurnScope>;
    fn changed(&self, store: &Store);
    fn interrupt(&self);
}
pub(super) struct Local {
    task: Task,
    pins: SessionPins,
}
impl Local {
    pub fn new(task: &Task) -> Result<Self> {
        Ok(Self {
            task: task.clone(),
            pins: SessionPins {
                session: task.id.clone(),
                workspace: task.workspace.clone(),
                owner: task.user.clone(),
                lease: task.lease.clone(),
                starter_device: task.device.clone(),
                binding_revision: task.binding,
                profile_digest: task
                    .profile
                    .as_ref()
                    .ok_or("mcp_profile_required")?
                    .digest
                    .clone(),
            },
        })
    }
}
impl Authority for Local {
    fn pins(&self) -> &SessionPins {
        &self.pins
    }
    fn check(&self, state: &AppState, deadline: Instant) -> Result<()> {
        let _slot = state
            .interactive
            .fs_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "mcp_authority_busy")?;
        guard_sync(state, &self.task, deadline)
    }
    fn context(&self, state: &AppState, store: &Store) -> Result<AccessContext> {
        context(state, store, &self.task)
    }
    fn context_in_transaction(&self, state: &AppState, store: &Store) -> Result<AccessContext> {
        context_in_transaction(state, store, &self.task)
    }
    fn turn(&self) -> Result<TurnScope> {
        let active = self
            .task
            .handle
            .active_turn
            .lock()
            .map_err(|_| "mcp_scope_unavailable")?;
        let turn = active.as_ref().ok_or("mcp_call_outside_turn")?;
        Ok(TurnScope {
            id: turn.id.clone(),
            deadline: turn.started + Duration::from_secs(self.task.options.max_seconds),
        })
    }
    fn changed(&self, _: &Store) {
        let _ = self.task.handle.events.send(());
    }
    fn interrupt(&self) {
        self.task.handle.stop.store(true, Ordering::Release);
    }
}
