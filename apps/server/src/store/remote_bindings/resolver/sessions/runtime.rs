//! SQL-only positive cuts. Fresh Describe belongs outside Store; each supplied
//! observation is checked here against the immutable open pins and live v4/v5 slot.
use super::*;
use dunlin_domain::collaboration::AccessContext;
use dunlin_session_runtime::{HarnessInfo, TurnPhase};
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RemoteStreamCut {
    Gap {
        after_sequence: u64,
        after_digest: Option<String>,
    },
    Persisted {
        sequence: u64,
        digest: String,
        duplicate: bool,
        stopped: bool,
    },
}

impl Store {
    pub(super) fn session_pins(&self, id: &str) -> Result<Pins> {
        let (raw,owner,device,workspace,project,host):(String,String,String,String,String,String)=self.connection.query_row("SELECT pins,owner,device,workspace,project,host FROM controller_remote_sessions WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        if raw.len() > 32768 {
            return Err(session_conflict());
        }
        let pins: Pins = serde_json::from_str(&raw)?;
        if pins.owner != owner
            || pins.device != device
            || pins.workspace != workspace
            || pins.project != project
            || pins.binding.host_id != host
        {
            return Err(session_conflict());
        }
        Ok(pins)
    }

    #[cfg(test)]
    pub(crate) fn dispatch_remote_session_open_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<wire::SessionOpenCommand> {
        self.remote_session_runtime()
            .dispatch_remote_session_open_in_transaction(id, proof, now, authorize)?
            .into_legacy()
    }
    #[cfg(test)]
    pub(crate) fn accept_remote_session_in_transaction(
        &self,
        id: &str,
        receipt: &wire::SessionReceipt,
        ready: bool,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.remote_session_runtime()
            .accept_remote_session_in_transaction(id, receipt, ready, authorize)
    }
    /// Receipt capture for negative close/reconcile after interruption. This
    /// never marks Ready, recreates a lease, or dispatches another operation.
    pub(crate) fn capture_remote_session_receipt_in_transaction(
        &self,
        id: &str,
        receipt: &wire::SessionReceipt,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        source(self, &snapshot)?;
        snapshot.open.validate_receipt(receipt)?;
        if !snapshot.dispatched {
            return Err(session_conflict());
        }
        if let Some(existing) = snapshot.receipt
            && existing != *receipt
        {
            return Err(session_conflict());
        }
        self.connection.execute(
            "UPDATE controller_remote_sessions SET receipt=?2 WHERE session=?1",
            params![id, serde_json::to_string(receipt)?],
        )?;
        Ok(())
    }
    /// Queue claim and remote intent are one transaction; caller notifies after
    /// COMMIT. A completed worker turn does not release the session reservation.
    #[cfg(test)]
    pub(crate) fn claim_remote_session_turn_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::TurnCommand>> {
        self.remote_session_runtime()
            .claim_remote_session_turn_in_transaction(id, proof, now, authorize)
    }
    pub(super) fn remote_turn_command(
        &self,
        id: &str,
        turn: &str,
    ) -> Result<(wire::TurnCommand, Option<wire::TurnReceipt>, String)> {
        let (raw,receipt,phase):(String,Option<String>,String)=self.connection.query_row("SELECT command,receipt,phase FROM controller_remote_session_turns WHERE session=?1 AND turn=?2",params![id,turn],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if raw.len() > 65536 {
            return Err(session_conflict());
        }
        let command: wire::TurnCommand = serde_json::from_str(&raw)?;
        valid(command.validate())?;
        if command.identity.turn_id != turn
            || valid(wire::session_turn_fingerprint(&command))? != command.identity.fingerprint
        {
            return Err(session_conflict());
        }
        let receipt: Option<wire::TurnReceipt> =
            receipt.map(|raw| serde_json::from_str(&raw)).transpose()?;
        if let Some(receipt) = &receipt {
            valid(receipt.validate_for_command(&command))?;
        }
        Ok((command, receipt, phase))
    }
    /// A lost RPC reply may be resolved only from a receipt already persisted
    /// by the authenticated, ordered source stream. This issues no command and
    /// never changes a turn phase, transcript clock or lease.
    #[cfg(test)]
    pub(crate) fn resolve_remote_turn_receipt_in_transaction(
        &self,
        id: &str,
        identity: &wire::TurnIdentity,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::TurnReceipt>> {
        self.remote_session_runtime()
            .resolve_remote_turn_receipt_in_transaction(id, identity, authorize)
    }
    #[cfg(test)]
    pub(crate) fn dispatch_remote_session_turn_in_transaction(
        &self,
        id: &str,
        turn: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<wire::TurnCommand> {
        self.remote_session_runtime()
            .dispatch_remote_session_turn_in_transaction(id, turn, proof, now, authorize)
    }
    fn accept_remote_turn_inner(
        &self,
        snapshot: &RemoteSessionSnapshot,
        context: &AccessContext,
        receipt: &wire::TurnReceipt,
    ) -> Result<()> {
        let id = &snapshot.session;
        let turn = &receipt.identity.turn_id;
        let (command, existing, phase) = self.remote_turn_command(id, turn)?;
        valid(
            command.validate_for_session(snapshot.receipt.as_ref().ok_or_else(session_conflict)?),
        )?;
        valid(receipt.validate_for_command(&command))?;
        if let Some(existing) = existing
            && existing != *receipt
        {
            return Err(session_conflict());
        }
        if matches!(phase.as_str(), "accepted" | "terminal") {
            return Ok(());
        }
        if phase != "sent" {
            return Err(session_conflict());
        }
        self.interactive_journal()
            .transition_timed_in_transaction(
                context,
                id,
                &snapshot.lease,
                turn,
                TurnPhase::Running,
                None,
            )
            .map_err(journal_error)?;
        self.connection.execute("UPDATE controller_remote_session_turns SET receipt=?3,phase='accepted' WHERE session=?1 AND turn=?2",params![id,turn,serde_json::to_string(receipt)?])?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn accept_remote_session_turn_in_transaction(
        &self,
        id: &str,
        receipt: &wire::TurnReceipt,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.remote_session_runtime()
            .accept_remote_session_turn_in_transaction(id, receipt, authorize)
    }
    fn interrupt_owned_remote_journal(
        &self,
        snapshot: &RemoteSessionSnapshot,
        reason: &'static str,
    ) -> Result<()> {
        let (epoch, runtime, phase): (i64, Option<String>, String) = self.connection.query_row(
            "SELECT epoch,runtime,phase FROM interactive_sessions WHERE id=?1",
            [&snapshot.session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if integer(snapshot.lease.epoch)? != epoch {
            return Err(session_conflict());
        }
        if phase == "interrupted" && runtime.is_none() {
            return Ok(());
        }
        if runtime.as_deref() != Some(snapshot.lease.instance.as_str()) {
            return Err(session_conflict());
        }
        self.interactive_journal()
            .interrupt_in_transaction(&snapshot.session, &snapshot.lease, reason)
            .map_err(journal_error)
    }
    /// The source callback attests the exact live v4 transport. The caller
    /// callback separately checks current device/OIDC/host policy for projection.
    /// Already persisted duplicate ACKs and stop cleanup remain negative cuts.
    #[cfg(test)]
    pub(crate) fn record_remote_session_event_in_transaction(
        &self,
        id: &str,
        event: &wire::SessionEvent,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<RemoteStreamCut> {
        self.remote_session_runtime()
            .record_remote_session_event_in_transaction(id, event, source, caller)
    }
    /// Only the exact opaque transport's matching durable worker ACK receipt may
    /// advance this watermark. Socket writes and arbitrary peer JSON cannot.
    pub(crate) fn confirm_remote_session_ack_in_transaction(
        &self,
        id: &str,
        request: &wire::SessionControllerMessage,
        attest_ack: impl FnOnce(
            &Store,
            &RemoteSessionSnapshot,
            &wire::SessionControllerMessage,
        ) -> Result<()>,
    ) -> Result<()> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        let wire::SessionControllerMessage::AckStream {
            session,
            through_sequence,
            event_digest,
            ..
        } = request
        else {
            return Err(session_conflict());
        };
        valid(request.validate())?;
        if snapshot.receipt.as_ref().map(|receipt| &receipt.fence) != Some(session)
            || *through_sequence > snapshot.ack_sequence
        {
            return Err(session_conflict());
        }
        let digest: String = self.connection.query_row(
            "SELECT digest FROM controller_remote_session_stream WHERE session=?1 AND sequence=?2",
            params![id, integer(*through_sequence)?],
            |r| r.get(0),
        )?;
        if &digest != event_digest {
            return Err(session_conflict());
        }
        attest_ack(self, &snapshot, request)?;
        self.connection.execute("UPDATE controller_remote_sessions SET confirmed_ack=max(confirmed_ack,?2) WHERE session=?1",params![id,integer(*through_sequence)?])?;
        self.connection.execute("UPDATE controller_remote_session_stream SET event=NULL WHERE session=?1 AND sequence<=?2 AND kind='text'",params![id,integer(*through_sequence)?])?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn remote_session_queue_allowed_in_transaction(
        &self,
        id: &str,
        owner: &str,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.remote_session_runtime()
            .remote_session_queue_allowed_in_transaction(id, owner, authorize)
    }
    /// Persist renewal before sending. A caller that loses its reply stops and
    /// reconciles instead of inventing a new positive renewal after timeout.
    #[cfg(test)]
    pub(crate) fn renew_remote_session_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<wire::SessionControllerMessage> {
        self.remote_session_runtime()
            .renew_remote_session_in_transaction(id, proof, now, authorize)
    }
    #[cfg(test)]
    pub(crate) fn renew_remote_session_turn_in_transaction(
        &self,
        id: &str,
        turn: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::SessionControllerMessage>> {
        self.remote_session_runtime()
            .renew_remote_session_turn_in_transaction(id, turn, proof, now, authorize)
    }
    pub(crate) fn close_remote_session_in_transaction(
        &self,
        id: &str,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::SessionControllerMessage>> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        source(self, &snapshot)?;
        if snapshot.phase == "stopped" {
            return Ok(None);
        }
        self.connection.execute(
            "UPDATE controller_remote_sessions SET closing=1 WHERE session=?1",
            [id],
        )?;
        Ok(snapshot
            .receipt
            .map(|receipt| wire::SessionControllerMessage::CloseSession {
                session: receipt.fence,
            }))
    }
    pub(crate) fn interrupt_remote_session_in_transaction(
        &self,
        id: &str,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        source(self, &snapshot)?;
        if snapshot.phase == "stopped" {
            return Ok(());
        }
        self.interrupt_owned_remote_journal(&snapshot, "remote_runtime_interrupted")?;
        self.connection.execute(
            if !snapshot.dispatched && snapshot.receipt.is_none() {
                "UPDATE controller_remote_sessions SET phase='stopped',closing=1,reservation=0 WHERE session=?1"
            } else {
                "UPDATE controller_remote_sessions SET phase='interrupted',closing=1 WHERE session=?1"
            },
            [id],
        )?;
        Ok(())
    }
    /// Canonical signed worker stop may release quarantine even if a preceding
    /// stream gap or revoked caller prevents transcript projection. It never
    /// advances an ACK over missing content or restores the Journal lease.
    pub(crate) fn reconcile_remote_session_stop_in_transaction(
        &self,
        id: &str,
        record: &wire::SessionRecord,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot, &wire::SessionRecord) -> Result<()>,
    ) -> Result<()> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        source(self, &snapshot, record)?;
        valid(record.validate())?;
        snapshot.open.validate_receipt(&record.receipt)?;
        if record.phase != wire::SessionPhase::Stopped || record.stop.is_none() {
            return Err(session_conflict());
        }
        if let Some(receipt) = &snapshot.receipt
            && receipt != &record.receipt
        {
            return Err(session_conflict());
        }
        let raw = serde_json::to_string(record)?;
        let existing: Option<String> = self.connection.query_row(
            "SELECT stop_record FROM controller_remote_sessions WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        if let Some(existing) = existing
            && existing != raw
        {
            return Err(session_conflict());
        }
        let stop = record.stop.as_ref().ok_or_else(session_conflict)?;
        let known:Option<String>=self.connection.query_row("SELECT digest FROM controller_remote_session_stream WHERE session=?1 AND sequence=?2",params![id,integer(stop.final_sequence)?],|r|r.get(0)).optional()?;
        if known
            .as_ref()
            .is_some_and(|digest| digest != &stop.event_digest)
        {
            return Err(session_conflict());
        }
        self.interrupt_owned_remote_journal(&snapshot, "remote_session_stopped")?;
        self.connection.execute("UPDATE controller_remote_sessions SET phase='stopped',closing=1,reservation=0,receipt=?2,stop_record=?3 WHERE session=?1",params![id,serde_json::to_string(&record.receipt)?,raw])?;
        Ok(())
    }
    pub(crate) fn remote_session_cleanup_ids(&self) -> Result<Vec<String>> {
        let mut query=self.connection.prepare("SELECT session FROM controller_remote_sessions WHERE phase!='stopped' OR confirmed_ack<ack_sequence OR coalesce(json_extract(stop_record,'$.stop.final_sequence'),0)>last_sequence ORDER BY session LIMIT 1025")?;
        let ids = query
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        if ids.len() > 1024 {
            return Err(StoreError::Limit(
                "Remote session cleanup capacity reached".into(),
            ));
        }
        Ok(ids)
    }
    /// UI observation only. It never returns a lease, receipt or command.
    pub(crate) fn remote_session_stop_observation(&self, id: &str) -> Result<Option<bool>> {
        Ok(self
            .connection
            .query_row(
                "SELECT phase='stopped' FROM controller_remote_sessions WHERE session=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?)
    }
    /// Internal scheduler hints only; no command, body or execution permission.
    pub(crate) fn remote_session_activity(&self, id: &str) -> Result<RemoteSessionActivity> {
        let queued: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND phase='queued')",
            [id],
            |r| r.get(0),
        )?;
        let active_turn:Option<String>=self.connection.query_row("SELECT turn FROM controller_remote_session_turns WHERE session=?1 AND phase IN('pending','sent','accepted') ORDER BY turn_index LIMIT 1",[id],|r|r.get(0)).optional()?;
        let cancellation:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND state='requested')",[id],|r|r.get(0))?;
        Ok(RemoteSessionActivity {
            queued,
            active_turn,
            cancellation,
        })
    }
    /// Exclusive producer startup only. Recovery never returns an executable
    /// Open/Submit, renews a lease, or releases a reservation from SQL state.
    pub(crate) fn recover_remote_sessions(&self) -> Result<usize> {
        self.remote_binding_transaction(false,|store| {
            let ids=store.remote_session_cleanup_ids()?;let mut count=0;
            for id in ids {
                let snapshot=store.remote_session_snapshot(&id)?;
                if matches!(snapshot.phase.as_str(),"pending"|"opening"|"active") {
                    store.interrupt_owned_remote_journal(&snapshot,"remote_controller_restarted")?;
                    if !snapshot.dispatched && snapshot.receipt.is_none() {
                        // The durable pre-send marker was never crossed. No
                        // worker could have admitted this intent, so only this
                        // local reservation may be released without OS proof.
                        store.connection.execute("UPDATE controller_remote_sessions SET phase='stopped',closing=1,reservation=0 WHERE session=?1",[id])?;
                    } else {
                        store.connection.execute("UPDATE controller_remote_sessions SET phase='interrupted',closing=1 WHERE session=?1",[id])?;
                    }
                    count+=1;
                }
            }Ok(count)
        })
    }
}

/// SQL-only runtime adapter. An MCP variant borrows the original live proof;
/// neither a snapshot nor an authorize closure can construct that variant.
pub(crate) struct RemoteSessionRuntime<'a> {
    store: &'a Store,
    authority: Option<(&'a authority::RemoteMcpScope, &'a crate::config::Config)>,
}
impl std::ops::Deref for RemoteSessionRuntime<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        self.store
    }
}
pub(super) fn scoped<'a>(
    store: &'a Store,
    scope: &'a authority::RemoteMcpScope,
    config: &'a crate::config::Config,
) -> RemoteSessionRuntime<'a> {
    RemoteSessionRuntime {
        store,
        authority: Some((scope, config)),
    }
}
impl Store {
    pub(crate) fn remote_session_runtime(&self) -> RemoteSessionRuntime<'_> {
        RemoteSessionRuntime {
            store: self,
            authority: None,
        }
    }
}
impl RemoteSessionRuntime<'_> {
    fn finish<T>(&self, value: T) -> Result<T> {
        if let Some((scope, _)) = self.authority {
            scope.check_deadline()?;
        }
        Ok(value)
    }

    fn current_remote_session(
        &self,
        id: &str,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<(RemoteSessionSnapshot, PreparedRemoteSession, AccessContext)> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        if let Some((scope, config)) = self.authority {
            if scope.pins().session != id {
                return Err(denied());
            }
            return scope.current(self.store, config, |store, _, _| {
                authorize(store, &snapshot)
            });
        }
        let pins = self.session_pins(id)?;
        if pins.mcp.is_some() {
            return Err(StoreError::Limit(
                "Remote MCP requires a live scoped broker authority".into(),
            ));
        }
        let fresh = self.prepare_remote_session_in_transaction(
            &pins.owner,
            &pins.device,
            &pins.workspace,
            &pins.project,
            pins.request.clone(),
            &pins.controller_node_id,
        )?;
        if fresh.pins.fingerprint != pins.fingerprint
            || fresh.pins.binding != pins.binding
            || fresh.pins.authority != pins.authority
            || snapshot.open.definition != fresh.definition
            || snapshot.open.budgets != fresh.budgets
        {
            return Err(session_conflict());
        }
        authorize(self, &snapshot)?;
        let context = self.team_project_context(&pins.workspace, &pins.project, &pins.owner)?;
        self.interactive_journal()
            .authorize_dispatch(&context, id, &snapshot.lease)
            .map_err(journal_error)?;
        Ok((snapshot, fresh, context))
    }
    fn current_remote_session_observation(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<(RemoteSessionSnapshot, AccessContext)> {
        let (snapshot, fresh, context) = self.current_remote_session(id, authorize)?;
        let cancelling:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND state IN('requested','unconfirmed'))",[id],|row|row.get(0))?;
        if snapshot.closing
            || cancelling
            || !matches!(snapshot.phase.as_str(), "pending" | "opening" | "active")
            || proof.descriptor() != &snapshot.open.descriptor
            || proof.lease().channel_id != snapshot.open.channel.channel_id
            || proof.lease().controller_boot_id != snapshot.open.channel.controller_boot_id
            || proof.lease().worker_boot_id != snapshot.open.channel.worker_boot_id
        {
            return Err(session_conflict());
        }
        let (raw, drain): (String, bool) = self.connection.query_row(
            "SELECT definition,drain FROM team_hosts WHERE id=?1",
            [&fresh.pins.binding.host_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let host: dunlin_domain::hosts::HostDefinition = serde_json::from_str(&raw)?;
        valid(host.validate())?;
        if drain
            || !host
                .capability_policy
                .harnesses
                .contains(&fresh.definition.harness)
        {
            return Err(denied());
        }
        self.finish_remote_target_in_transaction(
            &fresh.pins.owner,
            &fresh.pins.workspace,
            &fresh.pins.project,
            fresh.candidate,
            proof,
            &snapshot.open.channel.controller_boot_id,
            &fresh.pins.controller_node_id,
            now,
        )?;
        self.finish((snapshot, context))
    }
    pub(crate) fn dispatch_remote_session_open_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<RemoteOpen> {
        let (snapshot, _) = self.current_remote_session_observation(id, proof, now, authorize)?;
        if snapshot.phase != "pending" || snapshot.dispatched {
            return Err(session_conflict());
        }
        let open = snapshot.open;
        self.connection.execute(
            "UPDATE controller_remote_sessions SET phase='opening',dispatched=1 WHERE session=?1",
            [id],
        )?;
        self.finish(open)
    }
    pub(crate) fn accept_remote_session_in_transaction(
        &self,
        id: &str,
        receipt: &wire::SessionReceipt,
        ready: bool,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        let (snapshot, fresh, context) = self.current_remote_session(id, authorize)?;
        snapshot.open.validate_receipt(receipt)?;
        if !matches!(snapshot.phase.as_str(), "opening" | "active") || snapshot.closing {
            return Err(session_conflict());
        }
        if let Some(existing) = &snapshot.receipt
            && existing != receipt
        {
            return Err(session_conflict());
        }
        if ready && snapshot.phase != "active" {
            self.interactive_journal()
                .harness_ready_in_transaction(
                    &context,
                    id,
                    &snapshot.lease,
                    HarnessInfo {
                        profile: Some(snapshot.open.profile.clone()),
                        remote_session_id: receipt.fence.worker_session_id.clone(),
                        binding_revision: fresh.pins.request.binding.revision,
                        model: snapshot.open.definition.model.clone(),
                        mode: snapshot.open.definition.mode.clone(),
                        max_seconds: snapshot.open.budgets.max_seconds,
                        max_output_bytes: snapshot.open.budgets.max_output_bytes,
                    },
                )
                .map_err(journal_error)?;
        }
        self.connection.execute("UPDATE controller_remote_sessions SET receipt=?2,phase=CASE WHEN ?3 THEN 'active' ELSE phase END WHERE session=?1",params![id,serde_json::to_string(receipt)?,ready])?;
        self.finish(())
    }
    pub(crate) fn validate_remote_session_actor_in_transaction(
        &self,
        id: &str,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        let (snapshot, _, _) = self.current_remote_session(id, source)?;
        if !matches!(snapshot.phase.as_str(), "pending" | "opening" | "active") {
            return Err(session_conflict());
        }
        self.finish(())
    }
    pub(crate) fn claim_remote_session_turn_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::TurnCommand>> {
        let (snapshot, context) =
            self.current_remote_session_observation(id, proof, now, authorize)?;
        if snapshot.phase != "active" {
            return Err(session_conflict());
        }
        let receipt = snapshot.receipt.as_ref().ok_or_else(session_conflict)?;
        let unknown: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND phase='unknown') OR EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND state IN('requested','unconfirmed'))",
            [id],
            |r| r.get(0),
        )?;
        if unknown {
            return Err(session_conflict());
        }
        let Some(turn) = self
            .interactive_journal()
            .claim_next_in_transaction(&context, id, &snapshot.lease)
            .map_err(journal_error)?
        else {
            return Ok(None);
        };
        let (count,total):(i64,i64)=self.connection.query_row("SELECT (SELECT count(*) FROM controller_remote_session_turns WHERE session=?1),(SELECT count(*) FROM controller_remote_session_turns)",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if count >= integer(snapshot.open.budgets.max_turns)? || total >= 32768 {
            return Err(StoreError::Limit(
                "Remote turn ledger capacity reached".into(),
            ));
        }
        let mut command = wire::TurnCommand {
            session: receipt.fence.clone(),
            identity: wire::TurnIdentity {
                command_id: format!("turn-command-{}", uuid::Uuid::now_v7()),
                turn_id: turn.id.clone(),
                fingerprint: "0".repeat(64),
            },
            turn_index: u64::try_from(count + 1).map_err(|_| session_conflict())?,
            input: turn.prompt,
            budgets: dunlin_relay_protocol::worker::ExecutionBudgets {
                max_seconds: snapshot.open.budgets.max_seconds,
                max_output_bytes: snapshot.open.budgets.max_output_bytes,
                lease_duration_ms: snapshot.open.budgets.lease_duration_ms,
            },
        };
        command.identity.fingerprint = valid(wire::session_turn_fingerprint(&command))?;
        valid(command.validate_for_session(receipt))?;
        valid(
            wire::SessionControllerMessage::SubmitTurn {
                command: Box::new(command.clone()),
            }
            .validate(),
        )?;
        self.connection.execute("INSERT INTO controller_remote_session_turns(session,turn,command_id,turn_index,command,phase) VALUES(?1,?2,?3,?4,?5,'pending')",params![id,turn.id,command.identity.command_id,integer(command.turn_index)?,serde_json::to_string(&command)?])?;
        self.finish(Some(command))
    }
    pub(crate) fn resolve_remote_turn_receipt_in_transaction(
        &self,
        id: &str,
        identity: &wire::TurnIdentity,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::TurnReceipt>> {
        let (snapshot, _, _) = self.current_remote_session(id, authorize)?;
        if snapshot.phase != "active" || snapshot.closing {
            return Err(session_conflict());
        }
        let (command, receipt, phase) = self.remote_turn_command(id, &identity.turn_id)?;
        valid(
            command.validate_for_session(snapshot.receipt.as_ref().ok_or_else(session_conflict)?),
        )?;
        if command.identity != *identity {
            return Err(session_conflict());
        }
        let receipt = match (phase.as_str(), receipt) {
            ("accepted" | "terminal", Some(receipt)) => Some(receipt),
            ("sent", None) => None,
            _ => return Err(session_conflict()),
        };
        self.finish(receipt)
    }
    pub(crate) fn dispatch_remote_session_turn_in_transaction(
        &self,
        id: &str,
        turn: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<wire::TurnCommand> {
        let (snapshot, _) = self.current_remote_session_observation(id, proof, now, authorize)?;
        if snapshot.phase != "active" {
            return Err(session_conflict());
        }
        let (command, _, phase) = self.remote_turn_command(id, turn)?;
        valid(
            command.validate_for_session(snapshot.receipt.as_ref().ok_or_else(session_conflict)?),
        )?;
        if phase != "pending" {
            return Err(session_conflict());
        }
        let cancelling:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND turn_id=?2 AND state IN('requested','unconfirmed'))",params![id,turn],|r|r.get(0))?;
        if cancelling {
            return Err(session_conflict());
        }
        self.connection.execute(
            "UPDATE controller_remote_session_turns SET phase='sent' WHERE session=?1 AND turn=?2",
            params![id, turn],
        )?;
        self.finish(command)
    }
    pub(crate) fn accept_remote_session_turn_in_transaction(
        &self,
        id: &str,
        receipt: &wire::TurnReceipt,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        let (snapshot, _, context) = self.current_remote_session(id, authorize)?;
        if snapshot.phase != "active" || snapshot.closing {
            return Err(session_conflict());
        }
        self.accept_remote_turn_inner(&snapshot, &context, receipt)?;
        self.finish(())
    }
    pub(crate) fn record_remote_session_event_in_transaction(
        &self,
        id: &str,
        event: &wire::SessionEvent,
        source: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<RemoteStreamCut> {
        self.session_transaction_required()?;
        let snapshot = self.remote_session_snapshot(id)?;
        source(self, &snapshot)?;
        let receipt = snapshot.receipt.as_ref().ok_or_else(session_conflict)?;
        valid(event.validate())?;
        if event.session != receipt.fence
            || valid(wire::session_event_digest(event))? != event.digest
        {
            return Err(session_conflict());
        }
        if event.sequence <= snapshot.sequence {
            let (digest,raw):(String,Option<String>)=self.connection.query_row("SELECT digest,event FROM controller_remote_session_stream WHERE session=?1 AND sequence=?2",params![id,integer(event.sequence)?],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if digest != event.digest
                || raw
                    .as_ref()
                    .is_some_and(|raw| serde_json::to_string(event).ok().as_ref() != Some(raw))
            {
                return Err(session_conflict());
            }
            return Ok(RemoteStreamCut::Persisted {
                sequence: snapshot.sequence,
                digest: snapshot.digest.ok_or_else(session_conflict)?,
                duplicate: true,
                stopped: snapshot.phase == "stopped",
            });
        }
        if event.sequence
            != snapshot
                .sequence
                .checked_add(1)
                .ok_or_else(session_conflict)?
        {
            return Ok(RemoteStreamCut::Gap {
                after_sequence: snapshot.sequence,
                after_digest: snapshot.digest,
            });
        }
        valid(event.validate_after(
            &receipt.fence,
            snapshot.sequence,
            snapshot.digest.as_deref(),
        ))?;
        let stopped = matches!(event.event, wire::SessionEventKind::SessionStopped { .. });
        let draining = snapshot.phase == "stopped";
        if draining {
            // Reconcile may prove OS termination before the last stream page
            // arrives. Drain only to its exact authenticated stop anchor, with
            // no SDK projection or restoration of positive authority.
            let raw: Option<String> = self.connection.query_row(
                "SELECT stop_record FROM controller_remote_sessions WHERE session=?1",
                [id],
                |row| row.get(0),
            )?;
            let record: wire::SessionRecord =
                serde_json::from_str(&raw.ok_or_else(session_conflict)?)?;
            valid(record.validate())?;
            let stop = record.stop.as_ref().ok_or_else(session_conflict)?;
            if record.receipt != *receipt || event.sequence > stop.final_sequence {
                return Err(session_conflict());
            }
            if event.sequence == stop.final_sequence {
                valid(record.validate_for_stop_event(event))?;
            } else if stopped {
                return Err(session_conflict());
            }
        }
        // A late text page is needed only to verify its chain digest. Never
        // retain or expose its contents after authority/session termination.
        let raw = if draining && matches!(&event.event, wire::SessionEventKind::Text { .. }) {
            None
        } else {
            Some(serde_json::to_string(event)?)
        };
        let (count,bytes):(i64,i64)=self.connection.query_row("SELECT count(*),coalesce(sum(length(CAST(event AS BLOB))),0) FROM controller_remote_session_stream",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let reserve = if stopped { 0 } else { 65536 };
        if count >= 32768 - if stopped { 0 } else { 32 }
            || bytes
                + i64::try_from(raw.as_ref().map_or(0, String::len))
                    .map_err(|_| session_conflict())?
                > 32 * 1024 * 1024 - reserve
        {
            return Err(StoreError::Limit(
                "Remote session stream ledger capacity reached".into(),
            ));
        }
        let mut projected = false;
        let kind = match &event.event {
            wire::SessionEventKind::Text { .. } => "text",
            wire::SessionEventKind::TurnCompleted { .. } => "turn_completed",
            wire::SessionEventKind::SessionStopped { .. } => "session_stopped",
        };
        self.connection.execute("INSERT INTO controller_remote_session_stream(session,sequence,digest,kind,event) VALUES(?1,?2,?3,?4,?5)",params![id,integer(event.sequence)?,event.digest,kind,raw])?;
        if stopped {
            self.interrupt_owned_remote_journal(&snapshot, "remote_session_stopped")?;
            self.connection.execute("UPDATE controller_remote_sessions SET phase='stopped',closing=1,reservation=0 WHERE session=?1",[id])?;
        } else if !draining
            && let wire::SessionEventKind::TurnCompleted { receipt, status } = &event.event
            && matches!(
                status,
                wire::TurnTerminalStatus::Unknown | wire::TurnTerminalStatus::Cancelled
            )
            && (snapshot.phase == "interrupted" || snapshot.closing)
        {
            let (command, existing, phase) =
                self.remote_turn_command(id, &receipt.identity.turn_id)?;
            valid(
                command
                    .validate_for_session(snapshot.receipt.as_ref().ok_or_else(session_conflict)?),
            )?;
            valid(receipt.validate_for_command(&command))?;
            if !matches!(phase.as_str(), "sent" | "accepted" | "unknown")
                || existing
                    .as_ref()
                    .is_some_and(|existing| existing != receipt)
            {
                return Err(session_conflict());
            }
            // A canonical negative terminal can complete the inbox after local
            // interruption. It neither projects new text nor restores authority;
            // reservation stays held until the following actual StopProof.
            self.interrupt_owned_remote_journal(&snapshot, "remote_turn_interrupted")?;
            self.connection.execute("UPDATE controller_remote_session_turns SET receipt=?3,phase='terminal',terminal_sequence=?4 WHERE session=?1 AND turn=?2",params![id,receipt.identity.turn_id,serde_json::to_string(receipt)?,integer(event.sequence)?])?;
            self.connection.execute("UPDATE controller_remote_sessions SET phase='interrupted',closing=1 WHERE session=?1",[id])?;
        } else if !draining {
            let (current, _, context) = self.current_remote_session(id, |store, snapshot| {
                caller(store, &snapshot.owner, &snapshot.device)
            })?;
            if current.phase != "active" {
                return Err(session_conflict());
            }
            match &event.event {
                wire::SessionEventKind::Text { turn, kind, text } => {
                    let (command, _, _) = self.remote_turn_command(id, &turn.turn_id)?;
                    let turn_receipt = wire::TurnReceipt {
                        identity: command.identity,
                        budgets: command.budgets,
                        fence: turn.clone(),
                    };
                    self.accept_remote_turn_inner(&snapshot, &context, &turn_receipt)?;
                    let update = serde_json::json!({"sessionUpdate":match kind { wire::TextKind::Text=>"agent_message_chunk",wire::TextKind::Reasoning=>"agent_thought_chunk" },"content":{"type":"text","text":text}});
                    self.interactive_journal()
                        .agent_update_in_transaction(
                            &context,
                            id,
                            &snapshot.lease,
                            &turn.turn_id,
                            update,
                        )
                        .map_err(journal_error)?;
                }
                wire::SessionEventKind::TurnCompleted { receipt, status } => {
                    self.accept_remote_turn_inner(&snapshot, &context, receipt)?;
                    let phase = match status {
                        wire::TurnTerminalStatus::Completed => TurnPhase::Completed,
                        wire::TurnTerminalStatus::Failed => TurnPhase::Failed,
                        wire::TurnTerminalStatus::Cancelled => TurnPhase::Cancelled,
                        wire::TurnTerminalStatus::Unknown => TurnPhase::Unknown,
                    };
                    self.interactive_journal()
                        .transition_timed_in_transaction(
                            &context,
                            id,
                            &snapshot.lease,
                            &receipt.identity.turn_id,
                            phase,
                            None,
                        )
                        .map_err(journal_error)?;
                    self.connection.execute("UPDATE controller_remote_session_turns SET phase='terminal',terminal_sequence=?3 WHERE session=?1 AND turn=?2",params![id,receipt.identity.turn_id,integer(event.sequence)?])?;
                    if matches!(phase, TurnPhase::Unknown | TurnPhase::Cancelled) {
                        self.interrupt_owned_remote_journal(&snapshot, "remote_turn_interrupted")?;
                        self.connection.execute("UPDATE controller_remote_sessions SET phase='interrupted',closing=1 WHERE session=?1",[id])?;
                    }
                }
                wire::SessionEventKind::SessionStopped { .. } => unreachable!(),
            }
            projected = true;
        }
        self.connection.execute("UPDATE controller_remote_sessions SET last_sequence=?2,last_digest=?3 WHERE session=?1",params![id,integer(event.sequence)?,event.digest])?;
        // Do not ACK a speculative late prefix until it reaches the stop
        // record's exact final digest. No gaps or fabricated ACK jumps.
        if !draining || stopped {
            self.connection.execute("UPDATE controller_remote_sessions SET ack_sequence=?2,ack_digest=?3 WHERE session=?1",params![id,integer(event.sequence)?,event.digest])?;
        }
        if projected {
            self.finish(())?;
        }
        Ok(RemoteStreamCut::Persisted {
            sequence: event.sequence,
            digest: event.digest.clone(),
            duplicate: false,
            stopped,
        })
    }
    pub(crate) fn remote_session_queue_allowed_in_transaction(
        &self,
        id: &str,
        owner: &str,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<()> {
        let (snapshot, _, _) = self.current_remote_session(id, authorize)?;
        if snapshot.owner != owner
            || snapshot.closing
            || !matches!(snapshot.phase.as_str(), "pending" | "opening" | "active")
        {
            return Err(denied());
        }
        let cancelling:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND state IN('requested','unconfirmed'))",[id],|r|r.get(0))?;
        if cancelling {
            return Err(denied());
        }
        self.finish(())
    }
    pub(crate) fn renew_remote_session_in_transaction(
        &self,
        id: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<wire::SessionControllerMessage> {
        let (snapshot, _) = self.current_remote_session_observation(id, proof, now, authorize)?;
        if !matches!(snapshot.phase.as_str(), "opening" | "active") || snapshot.receipt.is_none() {
            return Err(session_conflict());
        }
        let sequence: i64 = self.connection.query_row(
            "SELECT renew_sequence FROM controller_remote_sessions WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        let sequence = u64::try_from(sequence)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(session_conflict)?;
        integer(sequence)?;
        let message = wire::SessionControllerMessage::RenewSession {
            session: snapshot.receipt.ok_or_else(session_conflict)?.fence,
            sequence,
            lease_duration_ms: snapshot.open.budgets.lease_duration_ms,
        };
        valid(message.validate())?;
        self.connection.execute(
            "UPDATE controller_remote_sessions SET renew_sequence=?2 WHERE session=?1",
            params![id, integer(sequence)?],
        )?;
        self.finish(message)
    }
    pub(crate) fn renew_remote_session_turn_in_transaction(
        &self,
        id: &str,
        turn: &str,
        proof: ObservedExport,
        now: i64,
        authorize: impl FnOnce(&Store, &RemoteSessionSnapshot) -> Result<()>,
    ) -> Result<Option<wire::SessionControllerMessage>> {
        let (snapshot, _) = self.current_remote_session_observation(id, proof, now, authorize)?;
        if snapshot.phase != "active" {
            return Err(session_conflict());
        }
        let (command, receipt, phase) = self.remote_turn_command(id, turn)?;
        if phase == "terminal" {
            return Ok(None);
        }
        if phase != "accepted" {
            return Err(session_conflict());
        }
        let sequence:i64=self.connection.query_row("SELECT renew_sequence FROM controller_remote_session_turns WHERE session=?1 AND turn=?2",params![id,turn],|r|r.get(0))?;
        let sequence = u64::try_from(sequence)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(session_conflict)?;
        integer(sequence)?;
        let message = wire::SessionControllerMessage::RenewTurn {
            turn: receipt.ok_or_else(session_conflict)?.fence,
            sequence,
            lease_duration_ms: command.budgets.lease_duration_ms,
        };
        valid(message.validate())?;
        self.connection.execute("UPDATE controller_remote_session_turns SET renew_sequence=?3 WHERE session=?1 AND turn=?2",params![id,turn,integer(sequence)?])?;
        self.finish(Some(message))
    }
}
