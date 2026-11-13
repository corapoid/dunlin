//! PASSIVE durable worker execution ledger. Not wired into schema/open or WSS.
//! A live monotonic handle cannot be deserialized or recovered from SQL. This
//! module does not start, contain, terminate or certify death of any process.
use super::{
    Error, ExportAuthority, ExportPermission, ExportScope, MAX_REVISION, PrincipalKind, Result,
    ValidatedExport, WorkerStore, sql_revision,
};
use dunlin_relay_protocol::worker::{
    AttemptFence, ChannelFence, ExecutionCommand, ExecutionIdentity, ExecutionOutcome,
    ExecutionReceipt, ExecutionRecord, ExecutionStatus, ExecutionTerminalStatus,
};
use rusqlite::{OptionalExtension, params};
use std::time::{Duration, Instant};
const MAX_INBOX: i64 = 1024;
const MAX_EXPORT_INBOX: i64 = 128;
// First passive primitive has one trusted local slot globally.
const MAX_ACTIVE: i64 = 1;
const MAX_OUTCOME_BYTES: usize = 40 * 1024;

/// Trusted adapter constructs this using its actual authenticated channel guard.
/// No Deserialize and no wire-provided boolean can construct this proof.
pub(crate) struct ExecutionChannel<'a> {
    scope: &'a ExportScope,
    channel: &'a ChannelFence,
    authorize: &'a dyn Fn() -> Result<()>,
    broker_offer: Option<&'a dunlin_relay_protocol::worker::session::broker::BrokerLimits>,
}
impl<'a> ExecutionChannel<'a> {
    pub fn verify(
        scope: &'a ExportScope,
        channel: &'a ChannelFence,
        authorize: &'a dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        scope.validate()?;
        channel
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution channel"))?;
        authorize()?;
        Ok(Self {
            scope,
            channel,
            authorize,
            broker_offer: None,
        })
    }
    pub(super) fn channel(&self) -> &ChannelFence {
        self.channel
    }
    /// Host adapter only: the offer comes from the authenticated v5 channel,
    /// never from an incoming session command or a reported capability boolean.
    pub fn verify_broker(
        scope: &'a ExportScope,
        channel: &'a ChannelFence,
        authorize: &'a dyn Fn() -> Result<()>,
        offer: &'a dunlin_relay_protocol::worker::session::broker::BrokerLimits,
    ) -> Result<Self> {
        offer
            .validate()
            .map_err(|_| Error::Invalid("Invalid broker offer"))?;
        let mut context = Self::verify(scope, channel, authorize)?;
        context.broker_offer = Some(offer);
        Ok(context)
    }
    pub(super) fn broker_offer(
        &self,
    ) -> Option<&dunlin_relay_protocol::worker::session::broker::BrokerLimits> {
        self.broker_offer
    }
    pub(super) fn validate(&self, store: &WorkerStore) -> Result<()> {
        store.check_scope(self.scope)?;
        (self.authorize)()
    }
}
/// A one-process launch permit, only returned for a new durable acceptance.
/// It is deliberately neither Clone nor serde; expiry is exclusively monotonic.
pub(crate) struct ExecutionLease {
    receipt: ExecutionReceipt,
    authority: ExportAuthority,
    deadline: Instant,
    budget_deadline: Instant,
}
impl ExecutionLease {
    pub(crate) fn root(&self) -> &std::path::Path {
        self.authority.candidate.checkout.canonical_root()
    }
    /// Narrow typed sandbox bridge; never add these paths to workspace IO.
    pub(crate) fn sandbox_metadata_roots(
        &self,
    ) -> Result<crate::store::native_checkout::SandboxMetadataRoots> {
        crate::store::native_checkout::SandboxMetadataRoots::from_worker_execution(self)
            .map_err(|_| Error::Denied)
    }
    pub(crate) fn sandbox_only_checkout(&self) -> &super::CapturedCheckout {
        &self.authority.candidate.checkout
    }
    pub(crate) fn filesystem_guard(&self) -> ExecutionFilesystem {
        ExecutionFilesystem(self.authority.candidate.checkout.clone())
    }
    pub fn fence(&self) -> &AttemptFence {
        &self.receipt.fence
    }
    #[cfg(test)]
    pub fn validate_filesystem(&self) -> Result<()> {
        self.authority.validate_filesystem()
    }
    pub(crate) fn budget_remaining(&self) -> Result<Duration> {
        self.budget_deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Execution budget expired"))
    }
    pub fn remaining(&self) -> Result<Duration> {
        let now = Instant::now();
        self.deadline
            .min(self.budget_deadline)
            .checked_duration_since(now)
            .filter(|d| !d.is_zero())
            .ok_or(Error::Conflict("Execution lease expired"))
    }
}
/// Cloneable validation only; contains no Store, launch permission or raw roots.
#[derive(Clone)]
pub(crate) struct ExecutionFilesystem(super::CapturedCheckout);
impl ExecutionFilesystem {
    pub(crate) fn validate(&self) -> Result<()> {
        self.0.revalidate().map_err(Error::from)
    }
}
pub(crate) struct ExecutionAdmission {
    pub record: ExecutionRecord,
    pub lease: Option<ExecutionLease>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Renewal {
    Fresh,
    Duplicate,
}
struct Stored {
    command: ExecutionCommand,
    record: ExecutionRecord,
    phase: String,
    sequence: u64,
    renewal_ms: u64,
    cancelled: bool,
    was_started: bool,
    reservation: bool,
}
struct RawStored {
    raw: String,
    receipt: String,
    phase: String,
    sequence: i64,
    renewal_ms: i64,
    cancelled: bool,
    was_started: bool,
    reservation: bool,
}
fn terminal_status(status: ExecutionTerminalStatus) -> ExecutionStatus {
    match status {
        ExecutionTerminalStatus::Completed => ExecutionStatus::Completed,
        ExecutionTerminalStatus::Failed => ExecutionStatus::Failed,
        ExecutionTerminalStatus::Cancelled => ExecutionStatus::Cancelled,
        ExecutionTerminalStatus::Unknown => ExecutionStatus::Unknown,
    }
}
fn profile_valid(command: &ExecutionCommand) -> Result<()> {
    command
        .validate()
        .map_err(|_| Error::Invalid("Invalid execution command"))?;
    if crate::worker_execution::command_fingerprint(command)? != command.identity.fingerprint
        || crate::worker_execution::profile_digest(&command.definition)? != command.profile.digest
    {
        return Err(Error::Conflict(
            "Execution command or profile digest differs",
        ));
    }
    Ok(())
}
impl WorkerStore {
    fn execution_stored(&self, identity: &ExecutionIdentity) -> Result<Option<Stored>> {
        // Both IDs are lifetime unique: neither half can be reassigned after ACK.
        let row:Option<RawStored> = self.connection.query_row("SELECT command,receipt,phase,renew_sequence,renew_duration_ms,cancel_requested,was_started,reservation FROM worker_execution_inbox WHERE command_id=?1 OR attempt_id=?2",params![identity.command_id,identity.attempt_id],|r|Ok(RawStored{raw:r.get(0)?,receipt:r.get(1)?,phase:r.get(2)?,sequence:r.get(3)?,renewal_ms:r.get(4)?,cancelled:r.get(5)?,was_started:r.get(6)?,reservation:r.get(7)?})).optional()?;
        row.map(
            |RawStored {
                 raw,
                 receipt,
                 phase,
                 sequence,
                 renewal_ms,
                 cancelled,
                 was_started,
                 reservation,
             }| {
                if raw.len() > 65536 || receipt.len() > 4096 {
                    return Err(Error::Denied);
                }
                let command: ExecutionCommand = serde_json::from_str(&raw)?;
                profile_valid(&command)?;
                if command.identity != *identity {
                    return Err(Error::Conflict("Execution identity was already used"));
                }
                let receipt: ExecutionReceipt = serde_json::from_str(&receipt)?;
                receipt.validate().map_err(|_| Error::Denied)?;
                if receipt.identity != command.identity
                    || receipt.fence.channel != command.channel
                    || receipt.lease_duration_ms != command.budgets.lease_duration_ms
                {
                    return Err(Error::Denied);
                }
                let outcome: Option<String> = self.connection.query_row(
                    "SELECT outcome FROM worker_execution_inbox WHERE command_id=?1",
                    [&identity.command_id],
                    |r| r.get(0),
                )?;
                let outcome = outcome
                    .map(|raw| -> Result<ExecutionOutcome> {
                        if raw.len() > MAX_OUTCOME_BYTES {
                            return Err(Error::Denied);
                        }
                        let outcome: ExecutionOutcome = serde_json::from_str(&raw)?;
                        outcome.validate().map_err(|_| Error::Denied)?;
                        if crate::worker_execution::outcome_digest(&outcome)? != outcome.digest {
                            return Err(Error::Denied);
                        }
                        Ok(outcome)
                    })
                    .transpose()?;
                if (phase != "terminal" && !reservation)
                    || (phase == "accepted" && was_started)
                    || ((phase == "start_intent" || phase == "running") && !was_started)
                {
                    return Err(Error::Denied);
                }
                let status = match phase.as_str() {
                    "accepted" | "start_intent" => ExecutionStatus::Accepted,
                    "running" => ExecutionStatus::Running,
                    "terminal" => terminal_status(outcome.as_ref().ok_or(Error::Denied)?.status),
                    _ => return Err(Error::Denied),
                };
                let record = ExecutionRecord {
                    receipt,
                    status,
                    outcome,
                };
                record.validate().map_err(|_| Error::Denied)?;
                let sequence = u64::try_from(sequence)
                    .ok()
                    .filter(|v| *v <= MAX_REVISION)
                    .ok_or(Error::Denied)?;
                let renewal_ms = u64::try_from(renewal_ms)
                    .ok()
                    .filter(|v| (1000..=10000).contains(v))
                    .ok_or(Error::Denied)?;
                Ok(Stored {
                    command,
                    record,
                    phase,
                    sequence,
                    renewal_ms,
                    cancelled,
                    was_started,
                    reservation,
                })
            },
        )
        .transpose()
    }
    fn execution_pins(&self, command: &ExecutionCommand) -> Result<ExportAuthority> {
        let principal = super::ExportPrincipal {
            kind: PrincipalKind::Human,
            id: command.descriptor.grant.principal.id.clone(),
        };
        let candidate = self.candidate_snapshot(
            &self.scope,
            &command.descriptor.export.id,
            &principal,
            ExportPermission::Execute,
        )?;
        if candidate.grant.principal.kind != PrincipalKind::Human
            || serde_json::to_value(&command.descriptor)?
                != serde_json::json!({"scope":candidate.scope,"export":candidate.export,"grant":candidate.grant})
        {
            return Err(Error::Denied);
        }
        if candidate.checkout.identity_digest()? != candidate.export.checkout_identity_digest
            || candidate.checkout.settings_digest() != candidate.export.settings_digest
            || candidate.checkout.repo_kind() != candidate.export.repo_kind
        {
            return Err(Error::Denied);
        }
        self.execution_policy(command)?;
        Ok(ExportAuthority { candidate })
    }
    fn execution_policy(&self, command: &ExecutionCommand) -> Result<()> {
        let id = &command.descriptor.export.id;
        // Existing policy reader revalidates raw digest, strict schema and revision.
        self.policy_snapshot(id, &command.descriptor.export.settings_digest)?
            .ok_or(Error::Denied)?;
        let raw: Vec<u8> = self.connection.query_row(
            "SELECT raw FROM export_execution_policies WHERE export_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let policy: serde_json::Value = serde_json::from_slice(&raw)?;
        if !policy["harnesses"]
            .as_array()
            .ok_or(Error::Denied)?
            .iter()
            .any(|h| h.as_str() == Some(command.definition.harness.as_str()))
            || command.budgets.max_seconds > policy["max_seconds"].as_u64().ok_or(Error::Denied)?
            || command.budgets.max_output_bytes as u64
                > policy["max_output_bytes"].as_u64().ok_or(Error::Denied)?
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    /// Exact retry returns the original immutable receipt and never a launch handle.
    pub fn admit_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        command: &ExecutionCommand,
        validated: ValidatedExport,
    ) -> Result<ExecutionAdmission> {
        profile_valid(command)?;
        let observed = validated.0;
        if observed.permission != ExportPermission::Execute {
            return Err(Error::Denied);
        }
        let accepted_at = Instant::now();
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        channel.validate(self)?;
        let authority = self.execution_pins(command)?;
        if observed.scope != authority.candidate.scope
            || observed.export != authority.candidate.export
            || observed.grant != authority.candidate.grant
        {
            return Err(Error::Denied);
        }
        if let Some(stored) = self.execution_stored(&command.identity)? {
            if stored.command != *command {
                return Err(Error::Conflict("Execution identity payload differs"));
            }
            Self::execution_deliverable(&stored)?;
            channel.validate(self)?;
            tx.commit()?;
            return Ok(ExecutionAdmission {
                record: stored.record,
                lease: None,
            });
        }
        if command.channel != *channel.channel {
            return Err(Error::Conflict("Execution channel changed"));
        }
        let (total,per_export,active,export_active):(i64,i64,i64,i64)=self.connection.query_row("SELECT count(*),coalesce(sum(export_id=?1),0),coalesce(sum(reservation),0),coalesce(sum(export_id=?1 AND reservation=1),0) FROM worker_execution_inbox",[&command.descriptor.export.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let schema: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let (session_active, session_export): (i64, i64) = if schema >= 4 {
            self.connection.query_row("SELECT coalesce(sum(reservation),0),coalesce(sum(export_id=?1 AND reservation=1),0) FROM worker_sessions",[&command.descriptor.export.id],|row|Ok((row.get(0)?,row.get(1)?)))?
        } else {
            (0, 0)
        };
        if total >= MAX_INBOX
            || per_export >= MAX_EXPORT_INBOX
            || active + session_active >= MAX_ACTIVE
            || export_active + session_export != 0
        {
            return Err(Error::Conflict("Worker execution capacity reached"));
        }
        let epoch: i64 = self.connection.query_row(
            "SELECT next_epoch FROM worker_execution_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let epoch = u64::try_from(epoch)
            .ok()
            .filter(|e| (1..MAX_REVISION).contains(e))
            .ok_or(Error::Conflict("Execution epochs exhausted"))?;
        let receipt = ExecutionReceipt {
            fence: AttemptFence {
                channel: channel.channel.clone(),
                attempt_id: command.identity.attempt_id.clone(),
                execution_epoch: epoch,
            },
            identity: command.identity.clone(),
            lease_duration_ms: command.budgets.lease_duration_ms,
        };
        let record = ExecutionRecord {
            receipt: receipt.clone(),
            status: ExecutionStatus::Accepted,
            outcome: None,
        };
        self.connection.execute(
            "UPDATE worker_execution_meta SET next_epoch=?1 WHERE singleton=1",
            [sql_revision(epoch + 1)?],
        )?;
        self.connection.execute("INSERT INTO worker_execution_inbox(command_id,attempt_id,export_id,worker_boot_id,command,receipt,phase,renew_sequence,renew_duration_ms,cancel_requested,outcome) VALUES(?1,?2,?3,?4,?5,?6,'accepted',0,?7,0,NULL)",params![command.identity.command_id,command.identity.attempt_id,command.descriptor.export.id,command.channel.worker_boot_id,serde_json::to_string(command)?,serde_json::to_string(&receipt)?,i64::try_from(command.budgets.lease_duration_ms).map_err(|_|Error::Denied)?])?;
        // Reserve the only terminal outbox row now. No quota check may prevent
        // terminal/Unknown publication after irreversible work has begun.
        self.connection.execute(
            "INSERT INTO worker_execution_outbox(attempt_id,outcome) VALUES(?1,NULL)",
            [&command.identity.attempt_id],
        )?;
        channel.validate(self)?;
        if Instant::now() >= accepted_at + Duration::from_millis(command.budgets.lease_duration_ms)
        {
            return Err(Error::Conflict("Execution acceptance lease elapsed"));
        }
        tx.commit()?;
        let lease = ExecutionLease {
            receipt,
            authority,
            deadline: accepted_at + Duration::from_millis(command.budgets.lease_duration_ms),
            budget_deadline: accepted_at + Duration::from_secs(command.budgets.max_seconds),
        };
        Ok(ExecutionAdmission {
            record,
            lease: Some(lease),
        })
    }
    fn current_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
    ) -> Result<Stored> {
        channel.validate(self)?;
        lease.remaining()?;
        if channel.channel != &lease.receipt.fence.channel {
            return Err(Error::Conflict("Execution channel changed"));
        }
        let stored = self
            .execution_stored(&lease.receipt.identity)?
            .ok_or(Error::Denied)?;
        if stored.record.receipt != lease.receipt || stored.phase == "terminal" || stored.cancelled
        {
            return Err(Error::Denied);
        }
        let current = self.execution_pins(&stored.command)?;
        if current.candidate.scope != lease.authority.candidate.scope
            || current.candidate.export != lease.authority.candidate.export
            || current.candidate.grant != lease.authority.candidate.grant
        {
            return Err(Error::Denied);
        }
        Ok(stored)
    }
    pub fn validate_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.current_execution(channel, lease)?;
        lease.remaining()?;
        tx.commit()?;
        lease.remaining()?;
        Ok(())
    }
    /// Persist before spawn. A second caller/retry cannot obtain this transition.
    pub fn execution_start_intent(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
    ) -> Result<()> {
        self.execution_transition(channel, lease, "accepted", "start_intent")
    }
    pub fn execution_running(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
    ) -> Result<()> {
        self.execution_transition(channel, lease, "start_intent", "running")
    }
    fn execution_transition(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let stored = self.current_execution(channel, lease)?;
        if stored.phase != from {
            return Err(Error::Conflict("Execution transition already consumed"));
        }
        self.connection.execute(
            "UPDATE worker_execution_inbox SET phase=?2,was_started=1 WHERE attempt_id=?1",
            params![lease.receipt.identity.attempt_id, to],
        )?;
        channel.validate(self)?;
        lease.remaining()?;
        tx.commit()?;
        lease.remaining()?;
        Ok(())
    }
    /// Duplicate sequence returns Duplicate and MUST NOT extend the live clock.
    /// A new sequence advances the in-memory deadline only after its SQL commit.
    pub fn renew_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &mut ExecutionLease,
        sequence: u64,
        duration_ms: u64,
    ) -> Result<Renewal> {
        if sequence == 0 || sequence > MAX_REVISION || !(1000..=10000).contains(&duration_ms) {
            return Err(Error::Invalid("Invalid execution renewal"));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let stored = self.current_execution(channel, lease)?;
        if sequence == stored.sequence {
            if duration_ms != stored.renewal_ms {
                return Err(Error::Conflict("Renewal identity differs"));
            }
            lease.remaining()?;
            tx.commit()?;
            lease.remaining()?;
            return Ok(Renewal::Duplicate);
        }
        if stored.sequence.checked_add(1) != Some(sequence) {
            return Err(Error::Conflict("Invalid execution sequence"));
        }
        let now = Instant::now();
        let deadline = now + Duration::from_millis(duration_ms);
        if deadline > lease.budget_deadline {
            return Err(Error::Conflict("Renewal exceeds original execution budget"));
        }
        self.connection.execute("UPDATE worker_execution_inbox SET renew_sequence=?2,renew_duration_ms=?3 WHERE attempt_id=?1",params![lease.receipt.identity.attempt_id,sql_revision(sequence)?,i64::try_from(duration_ms).map_err(|_|Error::Denied)?])?;
        channel.validate(self)?;
        lease.remaining()?;
        tx.commit()?;
        // COMMIT may wait for a different SQLite reader. Persistence of a new
        // sequence never revives an old monotonic lease which expired meanwhile.
        // A lost renewal acknowledgement reconciles through termination/Unknown,
        // not by reconstructing or extending a deadline from the durable row.
        lease.remaining()?;
        if Instant::now() >= deadline || deadline > lease.budget_deadline {
            return Err(Error::Conflict("Execution renewal elapsed during commit"));
        }
        lease.deadline = deadline;
        Ok(Renewal::Fresh)
    }
    pub fn cancel_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        fence: &AttemptFence,
    ) -> Result<bool> {
        fence
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution fence"))?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        channel.validate(self)?;
        if &fence.channel != channel.channel {
            return Err(Error::Denied);
        }
        let raw: String = self.connection.query_row(
            "SELECT receipt FROM worker_execution_inbox WHERE attempt_id=?1",
            [&fence.attempt_id],
            |r| r.get(0),
        )?;
        let receipt: ExecutionReceipt = serde_json::from_str(&raw)?;
        if receipt.fence != *fence {
            return Err(Error::Denied);
        }
        let changed=self.connection.execute("UPDATE worker_execution_inbox SET cancel_requested=1 WHERE attempt_id=?1 AND phase!='terminal'",[&fence.attempt_id])?;
        tx.commit()?;
        Ok(changed == 1)
    }
    fn terminal_execution(
        &self,
        stored: &Stored,
        outcome: &ExecutionOutcome,
    ) -> Result<ExecutionRecord> {
        outcome
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution outcome"))?;
        if crate::worker_execution::outcome_digest(outcome)? != outcome.digest {
            return Err(Error::Conflict("Execution outcome digest differs"));
        }
        if stored.phase == "terminal" {
            if stored.record.outcome.as_ref() != Some(outcome) {
                return Err(Error::Conflict("Execution already has another outcome"));
            }
            return Ok(stored.record.clone());
        }
        if outcome.status == ExecutionTerminalStatus::Completed
            && (stored.phase != "running" || stored.cancelled)
        {
            return Err(Error::Conflict(
                "Completion requires uncancelled running execution",
            ));
        }
        let encoded = serde_json::to_string(outcome)?;
        if encoded.len() > MAX_OUTCOME_BYTES {
            return Err(Error::Invalid("Execution outcome exceeds storage limit"));
        }
        self.connection.execute(
            "UPDATE worker_execution_inbox SET phase='terminal',outcome=?2,reservation=CASE WHEN was_started=0 THEN 0 ELSE 1 END WHERE attempt_id=?1",
            params![stored.command.identity.attempt_id, encoded],
        )?;
        let changed = self.connection.execute(
            "UPDATE worker_execution_outbox SET outcome=?2 WHERE attempt_id=?1 AND outcome IS NULL",
            params![stored.command.identity.attempt_id, encoded],
        )?;
        if changed != 1 {
            return Err(Error::Conflict("Reserved terminal outcome is missing"));
        }
        Ok(ExecutionRecord {
            receipt: stored.record.receipt.clone(),
            status: terminal_status(outcome.status),
            outcome: Some(outcome.clone()),
        })
    }
    /// Finalization is permitted after authority loss/lease expiry so a guardian
    /// can persist its truthful Failed/Cancelled/Unknown result. Caller must
    /// establish actual process death; SQL cannot supply that evidence. The
    /// returned internal record must not be sent to a peer: only the guarded
    /// outbox/reconcile/retry methods establish terminal wire deliverability.
    pub fn finish_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        lease: &ExecutionLease,
        outcome: &ExecutionOutcome,
    ) -> Result<ExecutionRecord> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(&lease.authority.candidate.scope)?;
        let stored = self
            .execution_stored(&lease.receipt.identity)?
            .ok_or(Error::Denied)?;
        if stored.record.receipt != lease.receipt {
            return Err(Error::Denied);
        }
        if outcome.status == ExecutionTerminalStatus::Completed && stored.phase != "terminal" {
            self.current_execution(channel, lease)?;
        }
        let result = self.terminal_execution(&stored, outcome)?;
        if outcome.status == ExecutionTerminalStatus::Completed && stored.phase != "terminal" {
            channel.validate(self)?;
            lease.remaining()?;
        }
        tx.commit()?;
        Ok(result)
    }
    /// Every terminal wire record also attests that the worker reservation is
    /// no longer quarantined. A durable outcome alone never proves process death.
    fn execution_deliverable(stored: &Stored) -> Result<()> {
        if stored.phase == "terminal" && stored.was_started && stored.reservation {
            return Err(Error::Conflict("Execution process stop is unconfirmed"));
        }
        Ok(())
    }
    fn execution_readback(&self, stored: &Stored) -> Result<()> {
        let command = &stored.command;
        let principal = super::ExportPrincipal {
            kind: PrincipalKind::Human,
            id: command.descriptor.grant.principal.id.clone(),
        };
        let current = self.candidate_snapshot(
            &self.scope,
            &command.descriptor.export.id,
            &principal,
            ExportPermission::Read,
        )?;
        // Read may be freshly regranted, but cannot select another export/rebind.
        if serde_json::to_value(&current.scope)? != serde_json::to_value(&command.descriptor.scope)?
            || serde_json::to_value(&current.export)?
                != serde_json::to_value(&command.descriptor.export)?
            || current.grant.principal.kind != PrincipalKind::Human
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    /// Unsolicited delivery enumerates only current-fence readable, stopped
    /// records. Historical or currently denied records stay durable and can
    /// never starve another principal's deliverable result. Explicit reconcile
    /// still reports its permission/stop error for a named attempt.
    pub fn execution_outbox(&self, channel: &ExecutionChannel<'_>) -> Result<Vec<ExecutionRecord>> {
        let tx = self.connection.unchecked_transaction()?;
        channel.validate(self)?;
        let mut query = self.connection.prepare(
            "SELECT i.command FROM worker_execution_outbox o JOIN worker_execution_inbox i ON i.attempt_id=o.attempt_id
             WHERE o.outcome IS NOT NULL
               AND json_extract(i.command,'$.channel.channel_id')=?1
               AND json_extract(i.command,'$.channel.controller_boot_id')=?2
               AND json_extract(i.command,'$.channel.worker_boot_id')=?3
             ORDER BY i.command_id LIMIT ?4",
        )?;
        let candidates = query.query_map(
            params![
                channel.channel.channel_id,
                channel.channel.controller_boot_id,
                channel.channel.worker_boot_id,
                MAX_INBOX
            ],
            |row| row.get::<_, String>(0),
        )?;
        let mut records = Vec::new();
        for raw in candidates {
            let raw = raw?;
            if raw.len() > 65536 {
                return Err(Error::Denied);
            }
            let command: ExecutionCommand = serde_json::from_str(&raw)?;
            let stored = self
                .execution_stored(&command.identity)?
                .ok_or(Error::Denied)?;
            match self.execution_readback(&stored) {
                Ok(()) => {}
                Err(Error::Denied) => continue,
                Err(error) => return Err(error),
            }
            if stored.phase == "terminal" && stored.was_started && stored.reservation {
                continue;
            }
            Self::execution_deliverable(&stored)?;
            records.push(stored.record);
            if records.len() == 8 {
                break;
            }
        }
        drop(query);
        channel.validate(self)?;
        tx.commit()?;
        Ok(records)
    }
    pub fn reconcile_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        identity: &ExecutionIdentity,
    ) -> Result<Option<ExecutionRecord>> {
        identity
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution identity"))?;
        let tx = self.connection.unchecked_transaction()?;
        channel.validate(self)?;
        let result = self
            .execution_stored(identity)?
            .map(|s| {
                self.execution_readback(&s)?;
                Self::execution_deliverable(&s)?;
                Ok::<_, Error>(s.record)
            })
            .transpose()?;
        channel.validate(self)?;
        tx.commit()?;
        Ok(result)
    }
    /// Fresh-channel historical ACK is readback-only, never a launch/renewal.
    pub fn acknowledge_execution(
        &self,
        channel: &ExecutionChannel<'_>,
        identity: &ExecutionIdentity,
        fence: &AttemptFence,
        outcome_id: &str,
        digest: &str,
    ) -> Result<()> {
        fence
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution fence"))?;
        identity
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution identity"))?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        channel.validate(self)?;
        let stored = self.execution_stored(identity)?.ok_or(Error::Denied)?;
        let outcome = stored.record.outcome.as_ref().ok_or(Error::Denied)?;
        if stored.record.receipt.fence != *fence
            || outcome.outcome_id != outcome_id
            || outcome.digest != digest
        {
            return Err(Error::Conflict("Outcome acknowledgement differs"));
        }
        self.connection.execute(
            "DELETE FROM worker_execution_outbox WHERE attempt_id=?1",
            [&identity.attempt_id],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Guardian-only crash finalization after an already-obtained kernel stop
    /// attestation. This never reconstructs an executable lease or grants access
    /// to output. A missing main process must not prevent durable Unknown and
    /// reservation release, while an existing terminal result stays immutable.
    pub fn finish_stopped_execution(
        &self,
        scope: &ExportScope,
        identity: &ExecutionIdentity,
        fence: &AttemptFence,
        attest: impl FnOnce(&AttemptFence) -> Result<()>,
    ) -> Result<ExecutionRecord> {
        identity
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution identity"))?;
        fence
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution fence"))?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let stored = self.execution_stored(identity)?.ok_or(Error::Denied)?;
        if stored.record.receipt.fence != *fence {
            return Err(Error::Denied);
        }
        // Only trusted local code may supply this callback. It must inspect an
        // acquired stop proof; no OS waits or peer assertions occur in this cut.
        attest(&stored.record.receipt.fence)?;
        let record = if stored.phase == "terminal" {
            stored.record.clone()
        } else {
            let mut outcome = ExecutionOutcome {
                outcome_id: format!("stopped-{}", uuid::Uuid::now_v7()),
                digest: "0".repeat(64),
                status: ExecutionTerminalStatus::Unknown,
                text: "Execution stopped without a confirmed terminal result".into(),
            };
            outcome.digest = crate::worker_execution::outcome_digest(&outcome)?;
            self.terminal_execution(&stored, &outcome)?
        };
        self.connection.execute(
            "UPDATE worker_execution_inbox SET reservation=0 WHERE attempt_id=?1",
            [&identity.attempt_id],
        )?;
        tx.commit()?;
        Ok(record)
    }
    /// Release only after terminal publication AND a trusted local actor/operator
    /// attestation of complete process/namespace shutdown. The callback is not
    /// supplied by a peer and must inspect an already-obtained stop proof, not
    /// kill/reap or await under this SQLite transaction. This API does not itself
    /// establish OS containment or validate a PID. Old-boot Unknown quarantine
    /// deliberately survives until such a proof is available.
    pub fn confirm_execution_stopped(
        &self,
        scope: &ExportScope,
        identity: &ExecutionIdentity,
        fence: &AttemptFence,
        attest: impl FnOnce(&AttemptFence) -> Result<()>,
    ) -> Result<()> {
        fence
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution fence"))?;
        identity
            .validate()
            .map_err(|_| Error::Invalid("Invalid execution identity"))?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let stored = self.execution_stored(identity)?.ok_or(Error::Denied)?;
        if stored.record.receipt.fence != *fence || stored.phase != "terminal" {
            return Err(Error::Denied);
        }
        attest(&stored.record.receipt.fence)?;
        self.connection.execute(
            "UPDATE worker_execution_inbox SET reservation=0 WHERE attempt_id=?1",
            [&identity.attempt_id],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// No wall timestamp reconstructs a lease. Every unfinished old-boot attempt
    /// is Unknown with a reserved durable outcome, never automatically retried.
    /// The process supervisor/OS containment must separately kill surviving work.
    pub fn recover_execution_boot(
        &self,
        scope: &ExportScope,
        worker_boot_id: &str,
    ) -> Result<usize> {
        if !super::identifier(worker_boot_id) {
            return Err(Error::Invalid("Invalid worker boot"));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let identities:Vec<(String,String)>=self.connection.prepare("SELECT command_id,attempt_id FROM worker_execution_inbox WHERE worker_boot_id!=?1 AND phase!='terminal' ORDER BY command_id LIMIT 8")?.query_map([worker_boot_id],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<std::result::Result<_,_>>()?;
        for (command_id, attempt_id) in &identities {
            let fingerprint:String=self.connection.query_row("SELECT json_extract(command,'$.identity.fingerprint') FROM worker_execution_inbox WHERE command_id=?1",[command_id],|r|r.get(0))?;
            let stored = self
                .execution_stored(&ExecutionIdentity {
                    command_id: command_id.clone(),
                    attempt_id: attempt_id.clone(),
                    fingerprint,
                })?
                .ok_or(Error::Denied)?;
            let mut outcome = ExecutionOutcome {
                outcome_id: format!("unknown-{}", uuid::Uuid::now_v7()),
                digest: "0".repeat(64),
                status: ExecutionTerminalStatus::Unknown,
                text: "Worker restarted before a confirmed terminal result".into(),
            };
            outcome.digest = crate::worker_execution::outcome_digest(&outcome)?;
            self.terminal_execution(&stored, &outcome)?;
        }
        tx.commit()?;
        Ok(identities.len())
    }
}
/// Applied only by the explicit operator schema transaction. Creating passive
/// ledger tables grants neither execution authority nor a live process lease.
pub(super) const SCHEMA:&str="
CREATE TABLE worker_execution_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),next_epoch INTEGER NOT NULL CHECK(next_epoch BETWEEN 1 AND 9007199254740991));
INSERT INTO worker_execution_meta VALUES(1,1);
CREATE TABLE worker_execution_inbox(command_id TEXT PRIMARY KEY,attempt_id TEXT NOT NULL UNIQUE,export_id TEXT NOT NULL REFERENCES worker_exports(id),worker_boot_id TEXT NOT NULL,command TEXT NOT NULL CHECK(length(CAST(command AS BLOB))<=65536),receipt TEXT NOT NULL CHECK(length(CAST(receipt AS BLOB))<=4096),phase TEXT NOT NULL CHECK(phase IN('accepted','start_intent','running','terminal')),renew_sequence INTEGER NOT NULL CHECK(renew_sequence BETWEEN 0 AND 9007199254740991),renew_duration_ms INTEGER NOT NULL CHECK(renew_duration_ms BETWEEN 1000 AND 10000),cancel_requested INTEGER NOT NULL CHECK(cancel_requested IN(0,1)),was_started INTEGER NOT NULL DEFAULT 0 CHECK(was_started IN(0,1)),reservation INTEGER NOT NULL DEFAULT 1 CHECK(reservation IN(0,1)),outcome TEXT CHECK(outcome IS NULL OR length(CAST(outcome AS BLOB))<=40960));
CREATE TABLE worker_execution_outbox(attempt_id TEXT PRIMARY KEY REFERENCES worker_execution_inbox(attempt_id),outcome TEXT CHECK(outcome IS NULL OR length(CAST(outcome AS BLOB))<=40960));
PRAGMA user_version=3;
";
#[cfg(test)]
mod tests;
