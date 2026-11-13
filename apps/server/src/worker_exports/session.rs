//! Passive v4 persistent-session ledger. Runtime/Hello4 activation is separate.
//! SQL never reconstructs a process, monotonic lease or filesystem authority.
use super::execution::{ExecutionChannel, Renewal};
use super::{
    Error, ExportAuthority, ExportPermission, ExportPrincipal, ExportScope, MAX_REVISION,
    PrincipalKind, Result, ValidatedExport, WorkerStore, sql_revision,
};
use dunlin_relay_protocol::worker::{ExportDescriptor, session as wire};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
const MAX_SESSIONS: i64 = 1024;
const MAX_OWNER_SESSIONS: i64 = 128;
const MAX_EVENTS: i64 = 4096;
const MAX_RETAINED: i64 = 8 * 1024 * 1024;
const MAX_SERIALIZED: i64 = 16 * 1024 * 1024;
const RESERVE_BYTES: i64 = 16 * 1024;
const RESERVE_EVENTS: i64 = 2;
/// A single process/session launch permission; never serde or Clone.
pub(crate) struct WorkerSessionLease {
    receipt: wire::SessionReceipt,
    authority: ExportAuthority,
    deadline: Instant,
    lifetime: Instant,
    idle: Instant,
    poisoned: bool,
    broker: Option<wire::broker::BrokerLimits>,
}
impl WorkerSessionLease {
    pub(crate) fn broker_limits(&self) -> Option<&wire::broker::BrokerLimits> {
        self.broker.as_ref()
    }

    pub fn fence(&self) -> &wire::SessionFence {
        &self.receipt.fence
    }
    pub fn root(&self) -> &std::path::Path {
        self.authority.candidate.checkout.canonical_root()
    }
    pub(crate) fn sandbox_only_checkout(&self) -> &super::CapturedCheckout {
        &self.authority.candidate.checkout
    }
    pub fn sandbox_metadata_roots(
        &self,
    ) -> Result<crate::store::native_checkout::SandboxMetadataRoots> {
        crate::store::native_checkout::SandboxMetadataRoots::from_worker_session(self)
            .map_err(|_| Error::Denied)
    }
    pub fn filesystem_guard(&self) -> SessionFilesystem {
        SessionFilesystem(self.authority.candidate.checkout.clone())
    }
    pub fn budget_remaining(&self) -> Result<Duration> {
        self.lifetime
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Worker session lifetime expired"))
    }
    pub fn remaining(&self) -> Result<Duration> {
        if self.poisoned {
            return Err(Error::Denied);
        }
        self.deadline
            .min(self.lifetime)
            .min(self.idle)
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Worker session lease expired"))
    }
}
#[derive(Clone)]
pub(crate) struct SessionFilesystem(super::CapturedCheckout);
impl SessionFilesystem {
    pub fn validate(&self) -> Result<()> {
        self.0.revalidate().map_err(Error::from)
    }
}
pub(crate) struct WorkerTurnLease {
    receipt: wire::TurnReceipt,
    deadline: Instant,
    budget: Instant,
    poisoned: bool,
    terminal: bool,
}
impl WorkerTurnLease {
    /// Correlation data only; obtaining a receipt never recreates this lease.
    pub(crate) fn receipt(&self) -> &wire::TurnReceipt {
        &self.receipt
    }
    pub(crate) fn budget_remaining(&self) -> Result<Duration> {
        if self.poisoned || self.terminal {
            return Err(Error::Denied);
        }
        self.budget
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Worker turn budget expired"))
    }

    pub fn fence(&self) -> &wire::TurnFence {
        &self.receipt.fence
    }
    pub fn remaining(&self) -> Result<Duration> {
        if self.poisoned || self.terminal {
            return Err(Error::Denied);
        }
        self.deadline
            .min(self.budget)
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Worker turn lease expired"))
    }
}
pub(crate) struct SessionAdmission {
    pub receipt: wire::SessionReceipt,
    pub lease: Option<WorkerSessionLease>,
}
pub(crate) struct TurnAdmission {
    pub receipt: wire::TurnReceipt,
    pub lease: Option<WorkerTurnLease>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    descriptor: ExportDescriptor,
    profile: dunlin_domain::agent::AgentProfileRef,
    harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    broker: Option<wire::broker::BrokerLimits>,
}
enum OpenRequest<'a> {
    Legacy(&'a wire::SessionOpenCommand),
    Broker(&'a wire::broker::OpenBrokerSessionCommand),
}
impl OpenRequest<'_> {
    fn command(&self) -> &wire::SessionOpenCommand {
        match self {
            Self::Legacy(command) => command,
            Self::Broker(command) => &command.open,
        }
    }
    fn fingerprint(&self) -> Result<String> {
        match self {
            Self::Legacy(command) => wire::session_open_fingerprint(command),
            Self::Broker(command) => wire::broker::broker_session_open_fingerprint(command),
        }
        .map_err(|_| invalid())
    }
    fn encoded(&self) -> Result<String> {
        match self {
            Self::Legacy(command) => serde_json::to_string(command),
            Self::Broker(command) => serde_json::to_string(command),
        }
        .map_err(Error::from)
    }
    fn broker(&self) -> Option<wire::broker::BrokerLimits> {
        match self {
            Self::Legacy(_) => None,
            Self::Broker(command) => Some(command.broker_limits.clone()),
        }
    }
    fn validate_receipt(&self, receipt: &wire::SessionReceipt) -> Result<()> {
        wire_valid(match self {
            Self::Legacy(command) => receipt.validate_for_open(command),
            Self::Broker(command) => command.validate_receipt(receipt),
        })
    }
}
struct Stored {
    receipt: wire::SessionReceipt,
    pins: Pins,
    phase: String,
    active: Option<String>,
    closing: bool,
    reservation: bool,
    spawned: bool,
}
fn status(status: wire::TurnTerminalStatus) -> wire::TurnStatus {
    match status {
        wire::TurnTerminalStatus::Completed => wire::TurnStatus::Completed,
        wire::TurnTerminalStatus::Failed => wire::TurnStatus::Failed,
        wire::TurnTerminalStatus::Cancelled => wire::TurnStatus::Cancelled,
        wire::TurnTerminalStatus::Unknown => wire::TurnStatus::Unknown,
    }
}
fn invalid() -> Error {
    Error::Invalid("Invalid v4 worker session data")
}
fn wire_valid(
    value: std::result::Result<(), dunlin_relay_protocol::worker::WorkerProtocolError>,
) -> Result<()> {
    value.map_err(|_| invalid())
}
impl WorkerStore {
    pub fn session_schema_ready(&self) -> Result<bool> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(version == 4)
    }
    fn session_receipt(&self, fence: &wire::SessionFence) -> Result<wire::SessionReceipt> {
        wire_valid(fence.validate())?;
        let raw: String = self.connection.query_row(
            "SELECT receipt FROM worker_sessions WHERE id=?1",
            [&fence.worker_session_id],
            |row| row.get(0),
        )?;
        if raw.len() > 8192 {
            return Err(Error::Denied);
        }
        let receipt: wire::SessionReceipt = serde_json::from_str(&raw)?;
        wire_valid(receipt.validate())?;
        if receipt.fence != *fence {
            return Err(Error::Conflict("Worker session fence differs"));
        }
        Ok(receipt)
    }
    fn stored_session(&self, fence: &wire::SessionFence) -> Result<Stored> {
        let receipt = self.session_receipt(fence)?;
        let (raw,phase,active,closing,reservation,spawned):(String,String,Option<String>,bool,bool,bool)=self.connection.query_row("SELECT pins,phase,active_turn,closing,reservation,spawned FROM worker_sessions WHERE id=?1",[&fence.worker_session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        if raw.len() > 16384 {
            return Err(Error::Denied);
        }
        let pins: Pins = serde_json::from_str(&raw)?;
        if let Some(limits) = &pins.broker {
            wire_valid(limits.validate())?;
        }
        wire_valid(pins.descriptor.validate())?;
        if serde_json::to_value(&pins.descriptor.scope)? != serde_json::to_value(&self.scope)? {
            return Err(Error::Denied);
        }
        Ok(Stored {
            receipt,
            pins,
            phase,
            active,
            closing,
            reservation,
            spawned,
        })
    }
    fn authority_for_session(&self, pins: &Pins) -> Result<ExportAuthority> {
        let principal = ExportPrincipal {
            kind: PrincipalKind::Human,
            id: pins.descriptor.grant.principal.id.clone(),
        };
        let candidate = self.candidate_snapshot(
            &self.scope,
            &pins.descriptor.export.id,
            &principal,
            ExportPermission::Execute,
        )?;
        if candidate.grant.principal.kind != PrincipalKind::Human
            || serde_json::to_value(&pins.descriptor)?
                != serde_json::json!({"scope":candidate.scope,"export":candidate.export,"grant":candidate.grant})
            || candidate.checkout.identity_digest()? != candidate.export.checkout_identity_digest
            || candidate.checkout.settings_digest() != candidate.export.settings_digest
            || candidate.checkout.repo_kind() != candidate.export.repo_kind
        {
            return Err(Error::Denied);
        }
        let policy = self
            .policy_snapshot(
                &pins.descriptor.export.id,
                &pins.descriptor.export.settings_digest,
            )?
            .ok_or(Error::Denied)?;
        if pins.broker.is_some() && !policy.permits_mcp_broker()? {
            return Err(Error::Denied);
        }
        Ok(ExportAuthority { candidate })
    }
    fn session_policy(&self, command: &wire::SessionOpenCommand) -> Result<()> {
        let raw: Vec<u8> = self.connection.query_row(
            "SELECT raw FROM export_execution_policies WHERE export_id=?1",
            [&command.descriptor.export.id],
            |r| r.get(0),
        )?;
        let policy: serde_json::Value = serde_json::from_slice(&raw)?;
        if !policy["harnesses"]
            .as_array()
            .ok_or(Error::Denied)?
            .iter()
            .any(|v| v.as_str() == Some(command.definition.harness.as_str()))
            || command.budgets.max_seconds > policy["max_seconds"].as_u64().ok_or(Error::Denied)?
            || command.budgets.max_output_bytes as u64
                > policy["max_output_bytes"].as_u64().ok_or(Error::Denied)?
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    fn session_current(
        &self,
        context: &ExecutionChannel<'_>,
        lease: &WorkerSessionLease,
    ) -> Result<Stored> {
        context.validate(self)?;
        lease.remaining()?;
        if context.channel() != &lease.fence().channel {
            return Err(Error::Conflict("Worker session channel changed"));
        }
        let stored = self.stored_session(lease.fence())?;
        if stored.pins.broker != lease.broker {
            return Err(Error::Denied);
        }
        if let Some(selected) = &lease.broker {
            let offer = context.broker_offer().ok_or(Error::Denied)?;
            if selected.max_pending > offer.max_pending
                || selected.max_reply_bytes > offer.max_reply_bytes
            {
                return Err(Error::Denied);
            }
        }
        if stored.receipt != lease.receipt
            || stored.closing
            || !stored.reservation
            || stored.phase == "stopped"
            || stored.phase == "unknown"
        {
            return Err(Error::Denied);
        }
        let current = self.authority_for_session(&stored.pins)?;
        if current.candidate.scope != lease.authority.candidate.scope
            || current.candidate.export != lease.authority.candidate.export
            || current.candidate.grant != lease.authority.candidate.grant
        {
            return Err(Error::Denied);
        }
        Ok(stored)
    }
    fn storage(&self) -> Result<(i64, i64, i64)> {
        self.connection.query_row("SELECT (SELECT count(*) FROM worker_session_events),(SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM worker_session_events),(SELECT coalesce(sum(coalesce(length(CAST(open_payload AS BLOB)),0)+length(CAST(pins AS BLOB))+length(CAST(receipt AS BLOB))+coalesce(length(CAST(stop AS BLOB)),0)+1024),0) FROM worker_sessions)+(SELECT coalesce(sum(coalesce(length(CAST(command AS BLOB)),0)+length(CAST(record AS BLOB))+1024),0) FROM worker_session_turns)+(SELECT coalesce(sum(coalesce(length(CAST(payload AS BLOB)),0)+1024),0) FROM worker_session_events)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(Error::from)
    }
    fn ordinary_capacity(
        &self,
        extra_bytes: i64,
        extra_events: i64,
        retained_extra: i64,
    ) -> Result<()> {
        let (events, retained, serialized) = self.storage()?;
        if events + extra_events + RESERVE_EVENTS > MAX_EVENTS
            || retained + retained_extra + RESERVE_BYTES > MAX_RETAINED
            || serialized + extra_bytes + RESERVE_BYTES > MAX_SERIALIZED
        {
            return Err(Error::Conflict("Worker session storage capacity reached"));
        }
        Ok(())
    }
    pub fn open_session(
        &self,
        context: &ExecutionChannel<'_>,
        command: &wire::SessionOpenCommand,
        validated: ValidatedExport,
    ) -> Result<SessionAdmission> {
        wire_valid(command.validate())?;
        self.admit_session(context, OpenRequest::Legacy(command), validated)
    }
    pub fn open_broker_session(
        &self,
        context: &ExecutionChannel<'_>,
        command: &wire::broker::OpenBrokerSessionCommand,
        validated: ValidatedExport,
    ) -> Result<SessionAdmission> {
        let offer = context.broker_offer().ok_or(Error::Denied)?;
        wire_valid(command.validate_for_channel(context.channel(), offer))?;
        self.admit_session(context, OpenRequest::Broker(command), validated)
    }
    fn admit_session(
        &self,
        context: &ExecutionChannel<'_>,
        request: OpenRequest<'_>,
        validated: ValidatedExport,
    ) -> Result<SessionAdmission> {
        let command = request.command();
        if request.fingerprint()? != command.identity.fingerprint
            || crate::worker_execution::profile_digest(&command.definition)?
                != command.profile.digest
        {
            return Err(Error::Conflict("Session payload fingerprint differs"));
        }
        let pins = Pins {
            descriptor: command.descriptor.clone(),
            profile: command.profile.clone(),
            harness: command.definition.harness.clone(),
            broker: request.broker(),
        };
        let now = Instant::now();
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        context.validate(self)?;
        let authority = self.authority_for_session(&pins)?;
        self.session_policy(command)?;
        let observed = validated.0;
        if observed.permission != ExportPermission::Execute
            || observed.scope != authority.candidate.scope
            || observed.export != authority.candidate.export
            || observed.grant != authority.candidate.grant
        {
            return Err(Error::Denied);
        }
        let existing:Option<String>=self.connection.query_row("SELECT receipt FROM worker_sessions WHERE intent_id=?1 OR controller_session_id=?2",params![command.identity.intent_id,command.controller_session_id],|r|r.get(0)).optional()?;
        if let Some(raw) = existing {
            let receipt: wire::SessionReceipt = serde_json::from_str(&raw)?;
            request.validate_receipt(&receipt)?;
            context.validate(self)?;
            tx.commit()?;
            return Ok(SessionAdmission {
                receipt,
                lease: None,
            });
        }
        if command.channel != *context.channel() {
            return Err(Error::Conflict("Session channel changed"));
        }
        if command.budgets.retained_bytes < (RESERVE_BYTES as usize) {
            return Err(Error::Conflict(
                "Session retention cannot reserve terminal outcomes",
            ));
        }
        let (total, owner): (i64, i64) = self.connection.query_row(
            "SELECT count(*),coalesce(sum(owner_id=?1),0) FROM worker_sessions",
            [&command.descriptor.grant.principal.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (live,per_export):(i64,i64)=self.connection.query_row("SELECT (SELECT coalesce(sum(reservation),0) FROM worker_sessions)+(SELECT coalesce(sum(reservation),0) FROM worker_execution_inbox),(SELECT coalesce(sum(reservation),0) FROM worker_sessions WHERE export_id=?1)+(SELECT coalesce(sum(reservation),0) FROM worker_execution_inbox WHERE export_id=?1)",[&command.descriptor.export.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if total >= MAX_SESSIONS || owner >= MAX_OWNER_SESSIONS || live != 0 || per_export != 0 {
            return Err(Error::Conflict("Worker session admission capacity reached"));
        }
        let epoch: i64 = self.connection.query_row(
            "SELECT session_epoch FROM worker_session_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let epoch = u64::try_from(epoch)
            .ok()
            .filter(|n| (1..MAX_REVISION).contains(n))
            .ok_or(Error::Conflict("Session epochs exhausted"))?;
        let receipt = wire::SessionReceipt {
            identity: command.identity.clone(),
            fence: wire::SessionFence {
                channel: command.channel.clone(),
                controller_session_id: command.controller_session_id.clone(),
                worker_session_id: format!("session-{}", uuid::Uuid::now_v7()),
                session_epoch: epoch,
            },
            budgets: command.budgets.clone(),
        };
        let raw = request.encoded()?;
        let encoded = serde_json::to_string(&receipt)?;
        let pin_json = serde_json::to_string(&pins)?;
        self.ordinary_capacity(
            (raw.len() + encoded.len() + pin_json.len() + 1024) as i64,
            0,
            0,
        )?;
        self.connection.execute(
            "UPDATE worker_session_meta SET session_epoch=?1 WHERE singleton=1",
            [sql_revision(epoch + 1)?],
        )?;
        self.connection.execute("INSERT INTO worker_sessions(id,intent_id,controller_session_id,owner_id,export_id,pins,open_payload,receipt,phase,reservation,spawned,closing,renew_sequence,renew_duration,tip_sequence,ack_sequence,output_bytes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'accepted',1,0,0,0,?9,0,0,0)",params![receipt.fence.worker_session_id,command.identity.intent_id,command.controller_session_id,command.descriptor.grant.principal.id,command.descriptor.export.id,pin_json,raw,encoded,i64::try_from(command.budgets.lease_duration_ms).map_err(|_|invalid())?])?;
        context.validate(self)?;
        tx.commit()?;
        let deadline = now + Duration::from_millis(command.budgets.lease_duration_ms);
        let lifetime = now + Duration::from_secs(command.budgets.max_seconds);
        Ok(SessionAdmission {
            receipt: receipt.clone(),
            lease: Some(WorkerSessionLease {
                receipt,
                authority,
                deadline,
                lifetime,
                idle: lifetime,
                poisoned: false,
                broker: pins.broker,
            }),
        })
    }
    pub fn validate_worker_session(
        &self,
        context: &ExecutionChannel<'_>,
        lease: &WorkerSessionLease,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.session_current(context, lease)?;
        tx.commit()?;
        lease.remaining()?;
        Ok(())
    }
    pub fn session_spawn_intent(
        &self,
        context: &ExecutionChannel<'_>,
        lease: &WorkerSessionLease,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let stored = self.session_current(context, lease)?;
        if stored.phase != "accepted" || stored.spawned {
            return Err(Error::Conflict("Session launch intent already consumed"));
        }
        self.connection.execute(
            "UPDATE worker_sessions SET spawned=1 WHERE id=?1",
            [&lease.fence().worker_session_id],
        )?;
        context.validate(self)?;
        lease.remaining()?;
        tx.commit()?;
        lease.remaining()?;
        Ok(())
    }
    pub fn session_ready(
        &self,
        context: &ExecutionChannel<'_>,
        lease: &mut WorkerSessionLease,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let stored = self.session_current(context, lease)?;
        if stored.phase != "accepted" || !stored.spawned {
            return Err(Error::Conflict("Session readiness phase differs"));
        }
        let idle = Instant::now() + Duration::from_millis(lease.receipt.budgets.idle_timeout_ms);
        self.connection.execute(
            "UPDATE worker_sessions SET phase='idle' WHERE id=?1",
            [&lease.fence().worker_session_id],
        )?;
        context.validate(self)?;
        tx.commit()?;
        if lease.remaining().is_err() || Instant::now() >= idle {
            lease.poisoned = true;
            return Err(Error::Conflict("Session readiness lease elapsed"));
        }
        lease.idle = idle;
        Ok(())
    }
    fn stored_turn(
        &self,
        session: &wire::SessionReceipt,
        identity: &wire::TurnIdentity,
    ) -> Result<Option<wire::TurnRecord>> {
        let row:Option<(String,Option<String>)>=self.connection.query_row("SELECT record,command FROM worker_session_turns WHERE session_id=?1 AND (command_id=?2 OR turn_id=?3)",params![session.fence.worker_session_id,identity.command_id,identity.turn_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(raw, command)| {
            if raw.len() > 8192 {
                return Err(Error::Denied);
            }
            let record: wire::TurnRecord = serde_json::from_str(&raw)?;
            wire_valid(record.validate())?;
            wire_valid(record.receipt.validate_for_session(session))?;
            if record.receipt.identity != *identity {
                return Err(Error::Conflict("Turn identity was already used"));
            }
            if let Some(raw) = command {
                if raw.len() > 65536 {
                    return Err(Error::Denied);
                }
                let command: wire::TurnCommand = serde_json::from_str(&raw)?;
                if wire::session_turn_fingerprint(&command).map_err(|_| invalid())?
                    != identity.fingerprint
                {
                    return Err(Error::Denied);
                }
                wire_valid(record.receipt.validate_for_command(&command))?;
            } else if record.completion.is_none() {
                return Err(Error::Denied);
            }
            if let Some(completion) = &record.completion {
                let event = self.load_session_event(&session.fence, completion.final_sequence)?;
                wire_valid(record.validate_for_event(&event))?;
            }
            Ok(record)
        })
        .transpose()
    }
    fn turn_by_id(&self, session: &wire::SessionReceipt, id: &str) -> Result<wire::TurnRecord> {
        let raw: String = self.connection.query_row(
            "SELECT record FROM worker_session_turns WHERE session_id=?1 AND turn_id=?2",
            params![session.fence.worker_session_id, id],
            |r| r.get(0),
        )?;
        let record: wire::TurnRecord = serde_json::from_str(&raw)?;
        self.stored_turn(session, &record.receipt.identity)?
            .ok_or(Error::Denied)
    }
    fn current_turn(
        &self,
        context: &ExecutionChannel<'_>,
        session: &WorkerSessionLease,
        turn: &WorkerTurnLease,
    ) -> Result<wire::TurnRecord> {
        let stored = self.session_current(context, session)?;
        turn.remaining()?;
        if turn.receipt.fence.session != *session.fence()
            || stored.phase != "busy"
            || stored.active.as_deref() != Some(&turn.receipt.identity.turn_id)
        {
            return Err(Error::Denied);
        }
        let record = self
            .stored_turn(&session.receipt, &turn.receipt.identity)?
            .ok_or(Error::Denied)?;
        if record.receipt != turn.receipt || record.completion.is_some() {
            return Err(Error::Denied);
        }
        Ok(record)
    }
    /// Canonical positive-channel retry only. Returns no process/lease permission,
    /// including after SessionStopped. Historical replacement channels reconcile.
    pub fn lookup_session_turn(
        &self,
        context: &ExecutionChannel<'_>,
        command: &wire::TurnCommand,
    ) -> Result<Option<wire::TurnRecord>> {
        wire_valid(command.validate())?;
        if wire::session_turn_fingerprint(command).map_err(|_| invalid())?
            != command.identity.fingerprint
        {
            return Err(Error::Conflict("Turn fingerprint differs"));
        }
        let tx = self.connection.unchecked_transaction()?;
        context.validate(self)?;
        if context.channel() != &command.session.channel {
            return Err(Error::Denied);
        }
        let stored = self.stored_session(&command.session)?;
        wire_valid(command.validate_for_session(&stored.receipt))?;
        self.authority_for_session(&stored.pins)?;
        let record = self.stored_turn(&stored.receipt, &command.identity)?;
        if let Some(record) = &record {
            wire_valid(record.receipt.validate_for_command(command))?;
        }
        context.validate(self)?;
        tx.commit()?;
        Ok(record)
    }
    pub fn admit_session_turn(
        &self,
        context: &ExecutionChannel<'_>,
        session: &mut WorkerSessionLease,
        command: &wire::TurnCommand,
    ) -> Result<TurnAdmission> {
        wire_valid(command.validate_for_session(&session.receipt))?;
        if wire::session_turn_fingerprint(command).map_err(|_| invalid())?
            != command.identity.fingerprint
        {
            return Err(Error::Conflict("Turn fingerprint differs"));
        }
        let now = Instant::now();
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let stored = self.session_current(context, session)?;
        if let Some(record) = self.stored_turn(&session.receipt, &command.identity)? {
            context.validate(self)?;
            tx.commit()?;
            return Ok(TurnAdmission {
                receipt: record.receipt,
                lease: None,
            });
        }
        if stored.phase != "idle" || stored.active.is_some() {
            return Err(Error::Conflict("Worker session already has an active turn"));
        }
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM worker_session_turns WHERE session_id=?1",
            [&session.fence().worker_session_id],
            |r| r.get(0),
        )?;
        if command.turn_index != u64::try_from(count + 1).map_err(|_| invalid())?
            || command.turn_index > session.receipt.budgets.max_turns
        {
            return Err(Error::Conflict("Turn index or capacity differs"));
        }
        let epoch: i64 = self.connection.query_row(
            "SELECT turn_epoch FROM worker_session_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let epoch = u64::try_from(epoch)
            .ok()
            .filter(|e| (1..MAX_REVISION).contains(e))
            .ok_or(Error::Conflict("Turn epochs exhausted"))?;
        let receipt = wire::TurnReceipt {
            identity: command.identity.clone(),
            fence: wire::TurnFence {
                session: command.session.clone(),
                turn_id: command.identity.turn_id.clone(),
                turn_index: command.turn_index,
                turn_epoch: epoch,
            },
            budgets: command.budgets.clone(),
        };
        let record = wire::TurnRecord {
            receipt: receipt.clone(),
            status: wire::TurnStatus::Accepted,
            completion: None,
        };
        let raw = serde_json::to_string(command)?;
        let encoded = serde_json::to_string(&record)?;
        self.ordinary_capacity((raw.len() + encoded.len() + 1024) as i64, 0, 0)?;
        self.connection.execute(
            "UPDATE worker_session_meta SET turn_epoch=?1 WHERE singleton=1",
            [sql_revision(epoch + 1)?],
        )?;
        self.connection.execute("INSERT INTO worker_session_turns(session_id,turn_id,command_id,command,record,phase,renew_sequence,renew_duration,output_bytes) VALUES(?1,?2,?3,?4,?5,'accepted',0,?6,0)",params![session.fence().worker_session_id,command.identity.turn_id,command.identity.command_id,raw,encoded,i64::try_from(command.budgets.lease_duration_ms).map_err(|_|invalid())?])?;
        self.connection.execute(
            "UPDATE worker_sessions SET phase='busy',active_turn=?2 WHERE id=?1",
            params![session.fence().worker_session_id, command.identity.turn_id],
        )?;
        context.validate(self)?;
        tx.commit()?;
        if session.remaining().is_err() {
            session.poisoned = true;
            return Err(Error::Conflict("Session admission clock elapsed"));
        }
        session.idle = session.lifetime;
        Ok(TurnAdmission {
            receipt: receipt.clone(),
            lease: Some(WorkerTurnLease {
                receipt,
                deadline: now + Duration::from_millis(command.budgets.lease_duration_ms),
                budget: now + Duration::from_secs(command.budgets.max_seconds),
                poisoned: false,
                terminal: false,
            }),
        })
    }
    pub fn turn_write_intent(
        &self,
        context: &ExecutionChannel<'_>,
        session: &WorkerSessionLease,
        turn: &WorkerTurnLease,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut record = self.current_turn(context, session, turn)?;
        if record.status != wire::TurnStatus::Accepted {
            return Err(Error::Conflict("Turn write intent already consumed"));
        }
        record.status = wire::TurnStatus::Running;
        self.connection.execute("UPDATE worker_session_turns SET phase='running',record=?3 WHERE session_id=?1 AND turn_id=?2",params![session.fence().worker_session_id,turn.fence().turn_id,serde_json::to_string(&record)?])?;
        context.validate(self)?;
        session.remaining()?;
        turn.remaining()?;
        tx.commit()?;
        session.remaining()?;
        turn.remaining()?;
        Ok(())
    }
    pub fn validate_worker_turn(
        &self,
        context: &ExecutionChannel<'_>,
        session: &WorkerSessionLease,
        turn: &WorkerTurnLease,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.current_turn(context, session, turn)?;
        tx.commit()?;
        session.remaining()?;
        turn.remaining()?;
        Ok(())
    }
    fn load_session_event(
        &self,
        session: &wire::SessionFence,
        sequence: u64,
    ) -> Result<wire::SessionEvent> {
        let (raw, digest): (Option<String>, String) = self.connection.query_row(
            "SELECT payload,digest FROM worker_session_events WHERE session_id=?1 AND sequence=?2",
            params![session.worker_session_id, sql_revision(sequence)?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let raw = raw.ok_or(Error::Conflict(
            "Requested stream history was acknowledged and pruned",
        ))?;
        if raw.len() > 65536 {
            return Err(Error::Denied);
        }
        let event: wire::SessionEvent = serde_json::from_str(&raw)?;
        wire_valid(event.validate())?;
        if event.session != *session
            || event.sequence != sequence
            || event.digest != digest
            || wire::session_event_digest(&event).map_err(|_| invalid())? != digest
        {
            return Err(Error::Denied);
        }
        Ok(event)
    }
    fn append_event(
        &self,
        receipt: &wire::SessionReceipt,
        kind: wire::SessionEventKind,
        critical: bool,
    ) -> Result<wire::SessionEvent> {
        let tip: i64 = self.connection.query_row(
            "SELECT tip_sequence FROM worker_sessions WHERE id=?1",
            [&receipt.fence.worker_session_id],
            |r| r.get(0),
        )?;
        let tip = u64::try_from(tip)
            .ok()
            .filter(|v| *v < MAX_REVISION)
            .ok_or(Error::Conflict("Session stream exhausted"))?;
        let previous = if tip == 0 {
            None
        } else {
            let digest: String = self.connection.query_row(
                "SELECT digest FROM worker_session_events WHERE session_id=?1 AND sequence=?2",
                params![receipt.fence.worker_session_id, sql_revision(tip)?],
                |r| r.get(0),
            )?;
            Some(wire::StreamPosition {
                session: receipt.fence.clone(),
                sequence: tip,
                digest,
            })
        };
        let mut event = wire::SessionEvent {
            session: receipt.fence.clone(),
            sequence: tip + 1,
            previous,
            digest: "0".repeat(64),
            event: kind,
        };
        event.digest = wire::session_event_digest(&event).map_err(|_| invalid())?;
        let raw = serde_json::to_string(&event)?;
        let size = i64::try_from(raw.len()).map_err(|_| invalid())?;
        let (events, retained, serialized) = self.storage()?;
        let local:i64=self.connection.query_row("SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM worker_session_events WHERE session_id=?1",[&receipt.fence.worker_session_id],|r|r.get(0))?;
        let (reserve, slots) = if critical {
            (0, 0)
        } else {
            (RESERVE_BYTES, RESERVE_EVENTS)
        };
        if events + 1 + slots > MAX_EVENTS
            || retained + size + reserve > MAX_RETAINED
            || serialized + size + 1024 + reserve > MAX_SERIALIZED
            || local + size + reserve > receipt.budgets.retained_bytes as i64
        {
            return Err(Error::Conflict("Worker stream retention capacity reached"));
        }
        let tag = match &event.event {
            wire::SessionEventKind::Text { .. } => "text",
            wire::SessionEventKind::TurnCompleted { .. } => "turn",
            wire::SessionEventKind::SessionStopped { .. } => "stop",
        };
        self.connection.execute("INSERT INTO worker_session_events(session_id,sequence,digest,payload,kind) VALUES(?1,?2,?3,?4,?5)",params![receipt.fence.worker_session_id,sql_revision(event.sequence)?,event.digest,raw,tag])?;
        self.connection.execute(
            "UPDATE worker_sessions SET tip_sequence=?2 WHERE id=?1",
            params![
                receipt.fence.worker_session_id,
                sql_revision(event.sequence)?
            ],
        )?;
        Ok(event)
    }
    fn close_for_budget(&self, session: &wire::SessionFence) -> Result<()> {
        self.connection.execute(
            "UPDATE worker_sessions SET phase='unknown',closing=1 WHERE id=?1",
            [&session.worker_session_id],
        )?;
        Ok(())
    }
    pub fn append_session_text(
        &self,
        context: &ExecutionChannel<'_>,
        session: &mut WorkerSessionLease,
        turn: &WorkerTurnLease,
        kind: wire::TextKind,
        text: &str,
    ) -> Result<wire::SessionEvent> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let record = self.current_turn(context, session, turn)?;
        if record.status != wire::TurnStatus::Running {
            return Err(Error::Denied);
        }
        let kind = wire::SessionEventKind::Text {
            turn: turn.fence().clone(),
            kind,
            text: text.into(),
        };
        // Validate before mutation; malformed text is not a quota stop event.
        let probe = wire::SessionEvent {
            session: session.fence().clone(),
            sequence: 1,
            previous: None,
            digest: "0".repeat(64),
            event: kind.clone(),
        };
        wire_valid(probe.validate())?;
        let (session_bytes,turn_bytes):(i64,i64)=self.connection.query_row("SELECT s.output_bytes,t.output_bytes FROM worker_sessions s JOIN worker_session_turns t ON t.session_id=s.id WHERE s.id=?1 AND t.turn_id=?2",params![session.fence().worker_session_id,turn.fence().turn_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if session_bytes + text.len() as i64 > session.receipt.budgets.max_output_bytes as i64
            || turn_bytes + text.len() as i64 > turn.receipt.budgets.max_output_bytes as i64
        {
            self.close_for_budget(session.fence())?;
            tx.commit()?;
            session.poisoned = true;
            return Err(Error::Conflict("Session output budget exhausted"));
        }
        let event = match self.append_event(&session.receipt, kind, false) {
            Ok(event) => event,
            Err(Error::Conflict(_)) => {
                self.close_for_budget(session.fence())?;
                tx.commit()?;
                session.poisoned = true;
                return Err(Error::Conflict("Session stream budget exhausted"));
            }
            Err(error) => return Err(error),
        };
        self.connection.execute(
            "UPDATE worker_sessions SET output_bytes=output_bytes+?2 WHERE id=?1",
            params![session.fence().worker_session_id, text.len() as i64],
        )?;
        self.connection.execute("UPDATE worker_session_turns SET output_bytes=output_bytes+?3 WHERE session_id=?1 AND turn_id=?2",params![session.fence().worker_session_id,turn.fence().turn_id,text.len() as i64])?;
        context.validate(self)?;
        tx.commit()?;
        session.remaining()?;
        turn.remaining()?;
        Ok(event)
    }
    fn persist_turn_terminal(
        &self,
        receipt: &wire::SessionReceipt,
        record: &mut wire::TurnRecord,
        terminal: wire::TurnTerminalStatus,
        critical: bool,
    ) -> Result<wire::SessionEvent> {
        let event = self.append_event(
            receipt,
            wire::SessionEventKind::TurnCompleted {
                receipt: record.receipt.clone(),
                status: terminal,
            },
            critical,
        )?;
        record.status = status(terminal);
        record.completion = Some(wire::TurnCompletion {
            status: terminal,
            final_sequence: event.sequence,
            event_digest: event.digest.clone(),
        });
        self.connection.execute("UPDATE worker_session_turns SET phase='terminal',record=?3,terminal_sequence=?4 WHERE session_id=?1 AND turn_id=?2",params![receipt.fence.worker_session_id,record.receipt.fence.turn_id,serde_json::to_string(record)?,sql_revision(event.sequence)?])?;
        Ok(event)
    }
    pub fn complete_session_turn(
        &self,
        context: &ExecutionChannel<'_>,
        session: &mut WorkerSessionLease,
        turn: &mut WorkerTurnLease,
        terminal: wire::TurnTerminalStatus,
    ) -> Result<wire::TurnRecord> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut record = self.current_turn(context, session, turn)?;
        if record.status != wire::TurnStatus::Running {
            return Err(Error::Denied);
        }
        if let Err(error) =
            self.persist_turn_terminal(&session.receipt, &mut record, terminal, false)
        {
            if matches!(error, Error::Conflict(_)) {
                self.close_for_budget(session.fence())?;
                tx.commit()?;
                session.poisoned = true;
            }
            return Err(error);
        }
        let reusable = matches!(
            terminal,
            wire::TurnTerminalStatus::Completed | wire::TurnTerminalStatus::Failed
        );
        if reusable {
            self.connection.execute(
                "UPDATE worker_sessions SET phase='idle',active_turn=NULL WHERE id=?1",
                [&session.fence().worker_session_id],
            )?;
        } else {
            self.connection.execute(
                "UPDATE worker_sessions SET phase='unknown',closing=1,active_turn=NULL WHERE id=?1",
                [&session.fence().worker_session_id],
            )?;
        }
        let idle = Instant::now() + Duration::from_millis(session.receipt.budgets.idle_timeout_ms);
        context.validate(self)?;
        tx.commit()?;
        if session.remaining().is_err() || turn.remaining().is_err() || Instant::now() >= idle {
            session.poisoned = true;
            turn.poisoned = true;
            return Err(Error::Conflict("Turn completion clock elapsed"));
        }
        turn.terminal = true;
        session.poisoned = !reusable;
        session.idle = idle;
        Ok(record)
    }
    pub fn renew_worker_session(
        &self,
        context: &ExecutionChannel<'_>,
        lease: &mut WorkerSessionLease,
        sequence: u64,
        duration_ms: u64,
    ) -> Result<Renewal> {
        if sequence == 0 || sequence > MAX_REVISION || !(1000..=10000).contains(&duration_ms) {
            return Err(invalid());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.session_current(context, lease)?;
        let (old, duration): (i64, i64) = self.connection.query_row(
            "SELECT renew_sequence,renew_duration FROM worker_sessions WHERE id=?1",
            [&lease.fence().worker_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if sequence == old as u64 {
            if duration_ms != duration as u64 {
                return Err(Error::Conflict("Renewal identity differs"));
            }
            tx.commit()?;
            lease.remaining()?;
            return Ok(Renewal::Duplicate);
        }
        if old.checked_add(1) != Some(sql_revision(sequence)?) {
            return Err(Error::Conflict("Session renewal sequence differs"));
        }
        let deadline = Instant::now() + Duration::from_millis(duration_ms);
        if deadline > lease.lifetime {
            return Err(Error::Conflict("Renewal exceeds session lifetime"));
        }
        self.connection.execute(
            "UPDATE worker_sessions SET renew_sequence=?2,renew_duration=?3 WHERE id=?1",
            params![
                lease.fence().worker_session_id,
                sql_revision(sequence)?,
                duration_ms as i64
            ],
        )?;
        context.validate(self)?;
        lease.remaining()?;
        tx.commit()?;
        lease.remaining()?;
        if Instant::now() >= deadline {
            return Err(Error::Conflict("Session renewal elapsed during commit"));
        }
        lease.deadline = deadline;
        Ok(Renewal::Fresh)
    }
    pub fn renew_worker_turn(
        &self,
        context: &ExecutionChannel<'_>,
        session: &WorkerSessionLease,
        turn: &mut WorkerTurnLease,
        sequence: u64,
        duration_ms: u64,
    ) -> Result<Renewal> {
        if sequence == 0 || sequence > MAX_REVISION || !(1000..=10000).contains(&duration_ms) {
            return Err(invalid());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.current_turn(context, session, turn)?;
        let (old,duration):(i64,i64)=self.connection.query_row("SELECT renew_sequence,renew_duration FROM worker_session_turns WHERE session_id=?1 AND turn_id=?2",params![session.fence().worker_session_id,turn.fence().turn_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if sequence == old as u64 {
            if duration_ms != duration as u64 {
                return Err(Error::Conflict("Renewal identity differs"));
            }
            tx.commit()?;
            session.remaining()?;
            turn.remaining()?;
            return Ok(Renewal::Duplicate);
        }
        if old.checked_add(1) != Some(sql_revision(sequence)?) {
            return Err(Error::Conflict("Turn renewal sequence differs"));
        }
        let deadline = Instant::now() + Duration::from_millis(duration_ms);
        if deadline > turn.budget {
            return Err(Error::Conflict("Renewal exceeds turn budget"));
        }
        self.connection.execute("UPDATE worker_session_turns SET renew_sequence=?3,renew_duration=?4 WHERE session_id=?1 AND turn_id=?2",params![session.fence().worker_session_id,turn.fence().turn_id,sql_revision(sequence)?,duration_ms as i64])?;
        context.validate(self)?;
        session.remaining()?;
        turn.remaining()?;
        tx.commit()?;
        session.remaining()?;
        turn.remaining()?;
        if Instant::now() >= deadline {
            return Err(Error::Conflict("Turn renewal elapsed during commit"));
        }
        turn.deadline = deadline;
        Ok(Renewal::Fresh)
    }
    /// Negative control closes the entire context; it never creates a new turn.
    pub fn request_session_close(
        &self,
        context: &ExecutionChannel<'_>,
        fence: &wire::SessionFence,
        turn: Option<&wire::TurnFence>,
    ) -> Result<bool> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        context.validate(self)?;
        if context.channel() != &fence.channel {
            return Err(Error::Denied);
        }
        let stored = self.stored_session(fence)?;
        if let Some(target) = turn {
            wire_valid(target.validate())?;
            if target.session != *fence {
                return Err(Error::Denied);
            }
            let record = self.turn_by_id(&stored.receipt, &target.turn_id)?;
            if record.receipt.fence != *target {
                return Err(Error::Denied);
            }
            // Cancelling an already terminal turn cannot close its later sibling.
            if record.completion.is_some() {
                tx.commit()?;
                return Ok(false);
            }
            if stored.active.as_deref() != Some(target.turn_id.as_str()) {
                return Err(Error::Denied);
            }
        }
        if stored.phase == "stopped" || stored.closing {
            tx.commit()?;
            return Ok(false);
        }
        self.connection.execute(
            "UPDATE worker_sessions SET closing=1 WHERE id=?1",
            [&fence.worker_session_id],
        )?;
        context.validate(self)?;
        tx.commit()?;
        Ok(true)
    }
    fn session_readable(&self, stored: &Stored) -> Result<()> {
        let principal = ExportPrincipal {
            kind: PrincipalKind::Human,
            id: stored.pins.descriptor.grant.principal.id.clone(),
        };
        let current = self.candidate_snapshot(
            &self.scope,
            &stored.pins.descriptor.export.id,
            &principal,
            ExportPermission::Read,
        )?;
        if serde_json::to_value(&current.scope)?
            != serde_json::to_value(&stored.pins.descriptor.scope)?
            || serde_json::to_value(&current.export)?
                != serde_json::to_value(&stored.pins.descriptor.export)?
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    fn session_record(&self, fence: &wire::SessionFence) -> Result<wire::SessionRecord> {
        let receipt = self.session_receipt(fence)?;
        let (phase, active, stop): (String, Option<String>, Option<String>) =
            self.connection.query_row(
                "SELECT phase,active_turn,stop FROM worker_sessions WHERE id=?1",
                [&fence.worker_session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
        let phase = match phase.as_str() {
            "accepted" => wire::SessionPhase::Accepted,
            "idle" => wire::SessionPhase::Idle,
            "busy" => wire::SessionPhase::Busy,
            "unknown" => wire::SessionPhase::Unknown,
            "stopped" => wire::SessionPhase::Stopped,
            _ => return Err(Error::Denied),
        };
        let active_turn = active
            .map(|id| self.turn_by_id(&receipt, &id).map(|v| v.receipt))
            .transpose()?;
        let stop = stop
            .map(|raw| serde_json::from_str::<wire::SessionStop>(&raw))
            .transpose()?;
        let record = wire::SessionRecord {
            receipt,
            phase,
            active_turn,
            stop,
        };
        wire_valid(record.validate())?;
        if let Some(stop) = &record.stop {
            wire_valid(
                record
                    .validate_for_stop_event(&self.load_session_event(fence, stop.final_sequence)?),
            )?;
        }
        Ok(record)
    }
    pub fn reconcile_worker_session(
        &self,
        context: &ExecutionChannel<'_>,
        identity: &wire::SessionIdentity,
    ) -> Result<Option<wire::SessionRecord>> {
        wire_valid(identity.validate())?;
        let tx = self.connection.unchecked_transaction()?;
        context.validate(self)?;
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT receipt FROM worker_sessions WHERE intent_id=?1",
                [&identity.intent_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            tx.commit()?;
            return Ok(None);
        };
        let receipt: wire::SessionReceipt = serde_json::from_str(&raw)?;
        wire_valid(receipt.validate())?;
        if receipt.identity != *identity {
            return Err(Error::Conflict("Session identity differs"));
        }
        let stored = self.stored_session(&receipt.fence)?;
        let result = self.session_record(&receipt.fence)?;
        // Exact stopped metadata is negative capacity evidence, without text or
        // renewed authority. It must remain reconcilable after Read revocation.
        // Live records and transcript replay still require the current Read grant.
        if result.phase != wire::SessionPhase::Stopped {
            self.session_readable(&stored)?;
        }
        context.validate(self)?;
        tx.commit()?;
        Ok(Some(result))
    }
    pub fn reconcile_worker_turn(
        &self,
        context: &ExecutionChannel<'_>,
        fence: &wire::SessionFence,
        identity: &wire::TurnIdentity,
    ) -> Result<Option<wire::TurnRecord>> {
        wire_valid(identity.validate())?;
        let tx = self.connection.unchecked_transaction()?;
        context.validate(self)?;
        let stored = self.stored_session(fence)?;
        self.session_readable(&stored)?;
        let record = self.stored_turn(&stored.receipt, identity)?;
        context.validate(self)?;
        tx.commit()?;
        Ok(record)
    }
    fn stream_digest(&self, fence: &wire::SessionFence, sequence: u64) -> Result<String> {
        self.connection
            .query_row(
                "SELECT digest FROM worker_session_events WHERE session_id=?1 AND sequence=?2",
                params![fence.worker_session_id, sql_revision(sequence)?],
                |r| r.get(0),
            )
            .map_err(Error::from)
    }
    /// Exact contiguous replay. A pruned payload is an explicit gap, never empty success.
    pub fn replay_session_stream(
        &self,
        context: &ExecutionChannel<'_>,
        fence: &wire::SessionFence,
        after: u64,
        digest: Option<&str>,
        limit: usize,
    ) -> Result<(u64, Vec<wire::SessionEvent>, bool)> {
        if limit == 0 || limit > 8 || after > MAX_REVISION || ((after == 0) != digest.is_none()) {
            return Err(invalid());
        }
        let tx = self.connection.unchecked_transaction()?;
        context.validate(self)?;
        let stored = self.stored_session(fence)?;
        self.session_readable(&stored)?;
        let tip: i64 = self.connection.query_row(
            "SELECT tip_sequence FROM worker_sessions WHERE id=?1",
            [&fence.worker_session_id],
            |r| r.get(0),
        )?;
        if after > tip as u64 {
            return Err(Error::Conflict("Replay starts beyond stream cut"));
        }
        if let Some(digest) = digest
            && self.stream_digest(fence, after)? != digest
        {
            return Err(Error::Conflict("Replay predecessor differs"));
        }
        let end = (after + limit as u64).min(tip as u64);
        let mut events = Vec::new();
        let mut previous = digest.map(str::to_owned);
        for sequence in after + 1..=end {
            let event = self.load_session_event(fence, sequence)?;
            if event.previous.as_ref().map(|v| v.digest.as_str()) != previous.as_deref() {
                return Err(Error::Conflict("Stream chain differs"));
            }
            let mut trial = events.clone();
            trial.push(event.clone());
            let page = wire::SessionWorkerMessage::StreamReplayed {
                channel: context.channel().clone(),
                session: fence.clone(),
                after_sequence: after,
                after_digest: digest.map(str::to_owned),
                limit: limit as u8,
                through_sequence: tip as u64,
                events: trial,
                more: sequence < tip as u64,
            };
            if serde_json::to_vec(&page)?.len()
                > dunlin_relay_protocol::worker::MAX_WORKER_FRAME_BYTES
            {
                if events.is_empty() {
                    return Err(Error::Conflict(
                        "One retained event exceeds replay wire budget",
                    ));
                }
                break;
            }
            previous = Some(event.digest.clone());
            events.push(event);
        }
        let delivered = events.last().map_or(after, |event| event.sequence);
        context.validate(self)?;
        tx.commit()?;
        Ok((tip as u64, events, delivered < tip as u64))
    }
    /// Negative capacity acknowledgement: current channel, historical exact fence/hash;
    /// no result text and no renewed execution authority, even after Read revocation.
    pub fn acknowledge_session_stream(
        &self,
        context: &ExecutionChannel<'_>,
        fence: &wire::SessionFence,
        through: u64,
        digest: &str,
    ) -> Result<()> {
        if through == 0 || through > MAX_REVISION {
            return Err(invalid());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        context.validate(self)?;
        self.session_receipt(fence)?;
        if self.stream_digest(fence, through)? != digest {
            return Err(Error::Conflict("Stream acknowledgement differs"));
        }
        let old: i64 = self.connection.query_row(
            "SELECT ack_sequence FROM worker_sessions WHERE id=?1",
            [&fence.worker_session_id],
            |r| r.get(0),
        )?;
        if through > old as u64 {
            self.connection.execute(
                "UPDATE worker_sessions SET ack_sequence=?2,ack_digest=?3 WHERE id=?1",
                params![fence.worker_session_id, sql_revision(through)?, digest],
            )?;
            // Keep immutable receipts and hash tombstones forever. Only already
            // acknowledged text/input can be discarded, never canonical terminals.
            self.connection.execute("UPDATE worker_session_events SET payload=NULL WHERE session_id=?1 AND sequence<=?2 AND kind='text'",params![fence.worker_session_id,sql_revision(through)?])?;
            self.connection.execute("UPDATE worker_session_turns SET command=NULL WHERE session_id=?1 AND terminal_sequence<=?2",params![fence.worker_session_id,sql_revision(through)?])?;
            self.connection.execute(
                "UPDATE worker_sessions SET open_payload=NULL WHERE id=?1 AND stop_sequence<=?2",
                params![fence.worker_session_id, sql_revision(through)?],
            )?;
        }
        context.validate(self)?;
        tx.commit()?;
        Ok(())
    }
    /// Current-channel unsolicited delivery only. Historical boots require an
    /// explicit authorized ReplayStream; unreadable rows cannot starve delivery.
    pub fn session_outbox(
        &self,
        context: &ExecutionChannel<'_>,
    ) -> Result<Vec<wire::SessionEvent>> {
        let tx = self.connection.unchecked_transaction()?;
        context.validate(self)?;
        let pending = {
            let mut query = self.connection.prepare(
                "SELECT receipt,ack_sequence FROM worker_sessions WHERE tip_sequence>ack_sequence ORDER BY id",
            )?;
            query
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if pending.len() > MAX_SESSIONS as usize {
            return Err(Error::Denied);
        }
        let mut events = Vec::new();
        for (raw, after) in pending {
            if raw.len() > 8192 {
                return Err(Error::Denied);
            }
            let receipt: wire::SessionReceipt = serde_json::from_str(&raw)?;
            wire_valid(receipt.validate())?;
            if &receipt.fence.channel != context.channel() {
                continue;
            }
            let stored = self.stored_session(&receipt.fence)?;
            match self.session_readable(&stored) {
                Ok(()) => {}
                Err(Error::Denied | Error::Conflict(_)) => continue,
                Err(error) => return Err(error),
            }
            let mut query = self.connection.prepare(
                "SELECT sequence FROM worker_session_events WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
            )?;
            let sequences = query
                .query_map(
                    params![
                        receipt.fence.worker_session_id,
                        after,
                        8 - events.len() as i64
                    ],
                    |row| row.get::<_, i64>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for sequence in sequences {
                events.push(self.load_session_event(
                    &receipt.fence,
                    u64::try_from(sequence).map_err(|_| invalid())?,
                )?);
            }
            if events.len() == 8 {
                break;
            }
        }
        context.validate(self)?;
        tx.commit()?;
        Ok(events)
    }
    /// Only the trusted OS stop producer invokes this, after namespace/group cleanup.
    /// This does not reconstruct positive authority or infer death from SQL state.
    pub fn finish_stopped_session(
        &self,
        scope: &ExportScope,
        identity: &wire::SessionIdentity,
        fence: &wire::SessionFence,
        attest: impl FnOnce(&wire::SessionFence) -> Result<()>,
    ) -> Result<wire::SessionRecord> {
        self.finish_stopped_session_reason(
            scope,
            identity,
            fence,
            wire::SessionStopReason::Unknown,
            attest,
        )
    }
    pub fn finish_stopped_session_reason(
        &self,
        scope: &ExportScope,
        identity: &wire::SessionIdentity,
        fence: &wire::SessionFence,
        reason: wire::SessionStopReason,
        attest: impl FnOnce(&wire::SessionFence) -> Result<()>,
    ) -> Result<wire::SessionRecord> {
        wire_valid(identity.validate())?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let receipt = self.session_receipt(fence)?;
        if receipt.identity != *identity {
            return Err(Error::Denied);
        }
        attest(fence)?;
        let (phase, active): (String, Option<String>) = self.connection.query_row(
            "SELECT phase,active_turn FROM worker_sessions WHERE id=?1",
            [&fence.worker_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if phase == "stopped" {
            let record = self.session_record(fence)?;
            tx.commit()?;
            return Ok(record);
        }
        if let Some(active) = active {
            let mut record = self.turn_by_id(&receipt, &active)?;
            if record.completion.is_none() {
                self.persist_turn_terminal(
                    &receipt,
                    &mut record,
                    wire::TurnTerminalStatus::Unknown,
                    true,
                )?;
            }
        }
        let event = self.append_event(
            &receipt,
            wire::SessionEventKind::SessionStopped { reason },
            true,
        )?;
        let stop = wire::SessionStop {
            reason,
            final_sequence: event.sequence,
            event_digest: event.digest,
        };
        self.connection.execute("UPDATE worker_sessions SET phase='stopped',active_turn=NULL,closing=1,reservation=0,stop=?2,stop_sequence=?3 WHERE id=?1",params![fence.worker_session_id,serde_json::to_string(&stop)?,sql_revision(stop.final_sequence)?])?;
        let result = self.session_record(fence)?;
        tx.commit()?;
        Ok(result)
    }
    /// A new boot never restores deadlines, restarts a process or releases quarantine.
    /// Unknown is a turn result; it is explicitly not a SessionStopped proof.
    pub fn recover_session_boot(&self, scope: &ExportScope, current_boot: &str) -> Result<usize> {
        self.check_scope(scope)?;
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 4 {
            // A peer may read operator state from before session support; it
            // must not migrate it or make the existing v3 runtime unavailable.
            return Ok(0);
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let receipts = {
            let mut statement = self.connection.prepare(
                "SELECT receipt FROM worker_sessions WHERE phase!='stopped' AND reservation=1",
            )?;
            statement
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut count = 0;
        for raw in receipts {
            let receipt: wire::SessionReceipt = serde_json::from_str(&raw)?;
            wire_valid(receipt.validate())?;
            if receipt.fence.channel.worker_boot_id == current_boot {
                continue;
            }
            let active: Option<String> = self.connection.query_row(
                "SELECT active_turn FROM worker_sessions WHERE id=?1",
                [&receipt.fence.worker_session_id],
                |r| r.get(0),
            )?;
            if let Some(id) = active {
                let mut record = self.turn_by_id(&receipt, &id)?;
                if record.completion.is_none() {
                    self.persist_turn_terminal(
                        &receipt,
                        &mut record,
                        wire::TurnTerminalStatus::Unknown,
                        true,
                    )?;
                }
            }
            self.connection.execute(
                "UPDATE worker_sessions SET phase='unknown',active_turn=NULL,closing=1 WHERE id=?1",
                [&receipt.fence.worker_session_id],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }
}
/// Operator-only migration. Peer open_existing never runs this SQL.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE worker_session_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),session_epoch INTEGER NOT NULL CHECK(session_epoch BETWEEN 1 AND 9007199254740991),turn_epoch INTEGER NOT NULL CHECK(turn_epoch BETWEEN 1 AND 9007199254740991));
INSERT INTO worker_session_meta VALUES(1,1,1);
CREATE TABLE worker_sessions(
 id TEXT PRIMARY KEY,intent_id TEXT NOT NULL UNIQUE,controller_session_id TEXT NOT NULL UNIQUE,
 owner_id TEXT NOT NULL,export_id TEXT NOT NULL REFERENCES worker_exports(id),pins TEXT NOT NULL CHECK(length(CAST(pins AS BLOB))<=16384),
 open_payload TEXT CHECK(open_payload IS NULL OR length(CAST(open_payload AS BLOB))<=65536),receipt TEXT NOT NULL CHECK(length(CAST(receipt AS BLOB))<=8192),
 phase TEXT NOT NULL CHECK(phase IN('accepted','idle','busy','unknown','stopped')),reservation INTEGER NOT NULL CHECK(reservation IN(0,1)),spawned INTEGER NOT NULL CHECK(spawned IN(0,1)),closing INTEGER NOT NULL CHECK(closing IN(0,1)),active_turn TEXT,
 renew_sequence INTEGER NOT NULL CHECK(renew_sequence BETWEEN 0 AND 9007199254740991),renew_duration INTEGER NOT NULL CHECK(renew_duration BETWEEN 1000 AND 10000),
 tip_sequence INTEGER NOT NULL DEFAULT 0 CHECK(tip_sequence BETWEEN 0 AND 9007199254740991),ack_sequence INTEGER NOT NULL DEFAULT 0 CHECK(ack_sequence BETWEEN 0 AND tip_sequence),ack_digest TEXT,
 output_bytes INTEGER NOT NULL DEFAULT 0 CHECK(output_bytes>=0),stop TEXT,stop_sequence INTEGER,
 CHECK((phase='stopped' AND reservation=0 AND stop IS NOT NULL AND stop_sequence IS NOT NULL AND active_turn IS NULL) OR (phase!='stopped' AND reservation=1 AND stop IS NULL AND stop_sequence IS NULL)));
CREATE TABLE worker_session_turns(
 session_id TEXT NOT NULL REFERENCES worker_sessions(id),turn_id TEXT NOT NULL,command_id TEXT NOT NULL,
 command TEXT CHECK(command IS NULL OR length(CAST(command AS BLOB))<=65536),record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192),phase TEXT NOT NULL CHECK(phase IN('accepted','running','terminal')),
 renew_sequence INTEGER NOT NULL CHECK(renew_sequence BETWEEN 0 AND 9007199254740991),renew_duration INTEGER NOT NULL CHECK(renew_duration BETWEEN 1000 AND 10000),output_bytes INTEGER NOT NULL DEFAULT 0 CHECK(output_bytes>=0),terminal_sequence INTEGER,
 PRIMARY KEY(session_id,turn_id),UNIQUE(session_id,command_id),CHECK((phase='terminal')=(terminal_sequence IS NOT NULL)));
CREATE TABLE worker_session_events(session_id TEXT NOT NULL REFERENCES worker_sessions(id),sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),digest TEXT NOT NULL CHECK(length(digest)=64),payload TEXT CHECK(payload IS NULL OR length(CAST(payload AS BLOB))<=65536),kind TEXT NOT NULL CHECK(kind IN('text','turn','stop')),PRIMARY KEY(session_id,sequence));
PRAGMA user_version=4;
"#;
#[cfg(test)]
mod tests;
