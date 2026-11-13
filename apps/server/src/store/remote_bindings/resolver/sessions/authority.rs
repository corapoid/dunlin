//! Live controller-side proof, never serializable or recoverable from SQL alone.
//! Caller guards must attest the actor/IdP/host policy without IO under SQL.
use super::*;
use crate::{
    config::Config, interactive::mcp::authority::SessionPins, worker_channel::SessionTransport,
};
use dunlin_domain::collaboration::AccessContext;
use std::time::{Duration, Instant};

pub(crate) struct RemoteMcpScope {
    pins: SessionPins,
    intent_fingerprint: String,
    open_identity: wire::SessionIdentity,
    transport: SessionTransport,
    deadline: Instant,
}
impl RemoteMcpScope {
    pub(crate) fn runtime<'a>(
        &'a self,
        store: &'a Store,
        config: &'a Config,
    ) -> super::runtime::RemoteSessionRuntime<'a> {
        super::runtime::scoped(store, self, config)
    }

    pub(crate) fn capture(
        store: &Store,
        config: &Config,
        id: &str,
        lease: &Lease,
        transport: SessionTransport,
        deadline: Instant,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<Self> {
        store.session_transaction_required()?;
        let snapshot = store.remote_session_snapshot(id)?;
        let saved = store.session_pins(id)?;
        if snapshot.lease.epoch != lease.epoch
            || snapshot.lease.instance != lease.instance
            || saved.mcp.is_none()
        {
            return Err(denied());
        }
        let scope = Self {
            pins: SessionPins {
                session: id.into(),
                workspace: snapshot.workspace.clone(),
                owner: snapshot.owner.clone(),
                lease: lease.clone(),
                starter_device: snapshot.device.clone(),
                binding_revision: saved.request.binding.revision,
                profile_digest: snapshot.open.profile.digest.clone(),
            },
            intent_fingerprint: saved.fingerprint,
            open_identity: snapshot.open.identity.clone(),
            transport,
            deadline: deadline
                .min(Instant::now() + Duration::from_secs(snapshot.open.budgets.max_seconds)),
        };
        scope.current(store, config, caller)?;
        Ok(scope)
    }
    pub(super) fn check_deadline(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(denied())
        } else {
            Ok(())
        }
    }
    pub(crate) fn pins(&self) -> &SessionPins {
        &self.pins
    }
    pub(super) fn current(
        &self,
        store: &Store,
        config: &Config,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<(RemoteSessionSnapshot, PreparedRemoteSession, AccessContext)> {
        store.session_transaction_required()?;
        if Instant::now() >= self.deadline {
            return Err(denied());
        }
        let snapshot = store.remote_session_snapshot(&self.pins.session)?;
        let saved = store.session_pins(&self.pins.session)?;
        let private = saved.mcp.as_ref().ok_or_else(denied)?;
        if snapshot.owner != self.pins.owner
            || snapshot.workspace != self.pins.workspace
            || snapshot.device != self.pins.starter_device
            || snapshot.lease.epoch != self.pins.lease.epoch
            || snapshot.lease.instance != self.pins.lease.instance
            || saved.request.binding.revision != self.pins.binding_revision
            || snapshot.open.profile.digest != self.pins.profile_digest
            || saved.fingerprint != self.intent_fingerprint
            || snapshot.open.identity != self.open_identity
            || snapshot.open.channel != *self.transport.fence()
            || snapshot.closing
            || !matches!(snapshot.phase.as_str(), "pending" | "opening" | "active")
        {
            return Err(denied());
        }
        caller(store, &snapshot.owner, &snapshot.device)?;
        let fresh = store.prepare_remote_session_mode(
            &snapshot.owner,
            &snapshot.device,
            &snapshot.workspace,
            &snapshot.project,
            saved.request.clone(),
            &saved.controller_node_id,
            Some(McpInput::Live {
                config,
                transport: &self.transport,
                pins: private,
            }),
        )?;
        if fresh.pins.fingerprint != saved.fingerprint
            || fresh.pins.binding != saved.binding
            || fresh.pins.authority != saved.authority
            || fresh.definition != snapshot.open.definition
            || fresh.budgets != snapshot.open.budgets
        {
            return Err(denied());
        }
        let cancelling: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND state IN('requested','unconfirmed'))", [&self.pins.session], |row|row.get(0))?;
        if cancelling {
            return Err(denied());
        }
        let context =
            store.team_project_context(&snapshot.workspace, &snapshot.project, &snapshot.owner)?;
        store
            .interactive_journal()
            .authorize_dispatch(&context, &self.pins.session, &self.pins.lease)
            .map_err(journal_error)?;
        if Instant::now() >= self.deadline {
            return Err(denied());
        }
        Ok((snapshot, fresh, context))
    }
    /// This variant starts its own transaction; use only outside OAuth's cut.
    pub(crate) fn context(
        &self,
        store: &Store,
        config: &Config,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<AccessContext> {
        if !store.connection.is_autocommit() {
            return Err(session_conflict());
        }
        store.remote_binding_transaction(false, |store| {
            self.context_in_transaction(store, config, caller)
        })
    }
    /// SQL only. An OAuth IMMEDIATE transaction may already be in progress.
    pub(crate) fn context_in_transaction(
        &self,
        store: &Store,
        config: &Config,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<AccessContext> {
        self.current(store, config, caller)
            .map(|(_, _, context)| context)
    }
    /// Fresh source Describe must be collected outside Store/SQL by the host.
    pub(crate) fn observe_in_transaction(
        &self,
        store: &Store,
        config: &Config,
        proof: ObservedExport,
        now: i64,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<AccessContext> {
        let (snapshot, fresh, context) = self.current(store, config, caller)?;
        if proof.descriptor() != &snapshot.open.descriptor
            || proof.lease().channel_id != self.transport.fence().channel_id
            || proof.lease().controller_boot_id != self.transport.fence().controller_boot_id
            || proof.lease().worker_boot_id != self.transport.fence().worker_boot_id
            || !proof
                .descriptor()
                .grant
                .permissions
                .contains(&ExportPermission::Execute)
        {
            return Err(denied());
        }
        store.finish_remote_target_in_transaction(
            &snapshot.owner,
            &snapshot.workspace,
            &snapshot.project,
            fresh.candidate,
            proof,
            &snapshot.open.channel.controller_boot_id,
            &fresh.pins.controller_node_id,
            now,
        )?;
        if Instant::now() >= self.deadline {
            return Err(denied());
        }
        Ok(context)
    }
    /// Correlation and current SDK turn only. The host must additionally enforce
    /// its monotonic turn deadline and the common broker's one-use approval.
    pub(crate) fn request_in_transaction(
        &self,
        store: &Store,
        config: &Config,
        scope: &wire::broker::BrokerScope,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<AccessContext> {
        self.request_ready_in_transaction(store, config, scope, caller)?
            .ok_or_else(denied)
    }
    /// None means only the original sent command's acceptance is still being
    /// imported. This never authorizes discovery, a prompt or a tools/call.
    pub(crate) fn request_ready_in_transaction(
        &self,
        store: &Store,
        config: &Config,
        scope: &wire::broker::BrokerScope,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<Option<AccessContext>> {
        let (snapshot, _, context) = self.current(store, config, caller)?;
        let Some(receipt) = snapshot.receipt.as_ref() else {
            if snapshot.phase == "opening"
                && snapshot.dispatched
                && matches!(scope, wire::broker::BrokerScope::Catalog { .. })
                && scope.session().controller_session_id == self.pins.session
                && scope.session().channel == *self.transport.fence()
            {
                return Ok(None);
            }
            return Err(denied());
        };
        if scope.session() != &receipt.fence {
            return Err(denied());
        }
        if let wire::broker::BrokerScope::Turn { turn } = scope {
            if snapshot.phase != "active" {
                return Err(denied());
            }
            let (command, accepted, phase) =
                store.remote_turn_command(&snapshot.session, &turn.turn_id)?;
            valid(command.validate_for_session(receipt))?;
            if command.session != turn.session
                || command.identity.turn_id != turn.turn_id
                || command.turn_index != turn.turn_index
            {
                return Err(denied());
            }
            if accepted.is_none() && phase == "sent" {
                let dispatching: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='dispatching')", params![snapshot.session,turn.turn_id], |row|row.get(0))?;
                if dispatching {
                    return Ok(None);
                }
            }
            let accepted = accepted.ok_or_else(denied)?;
            valid(accepted.validate_for_command(&command))?;
            if phase != "accepted" || accepted.fence != *turn {
                return Err(denied());
            }
            let running: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='running')", params![snapshot.session,turn.turn_id], |row|row.get(0))?;
            if !running {
                return Err(denied());
            }
        }
        if Instant::now() >= self.deadline {
            return Err(denied());
        }
        Ok(Some(context))
    }
}
