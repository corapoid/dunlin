//! Proposed v3 execution DTOs, deliberately separate from the active v1/v2
//! handshake and parsers. Validation establishes shape and bounds, not authority,
//! profile/fingerprint integrity, freshness, leases, dedupe or execution safety.
use super::{
    ExportDescriptor, ExportPermission, PrincipalKind, Result, decode, identifier, invalid,
};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use serde::{Deserialize, Serialize};

pub const WORKER_EXECUTION_SUBPROTOCOL: &str = "dunlin.worker.v3";
pub const WORKER_EXECUTION_PROTOCOL_VERSION: u16 = 3;
/// V3 includes fresh Describe plus execution renewal for legal 1-second
/// leases. Legacy v1/v2 retain their independent 120-frame/minute bound.
pub const MAX_EXECUTION_FRAMES_PER_MINUTE: usize = 1024;
pub fn worker_rate_limit(protocol_version: u16) -> usize {
    if protocol_version == super::session::WORKER_SESSION_PROTOCOL_VERSION
        || protocol_version == super::session::broker::WORKER_BROKER_PROTOCOL_VERSION
    {
        4096
    } else if protocol_version == WORKER_EXECUTION_PROTOCOL_VERSION {
        MAX_EXECUTION_FRAMES_PER_MINUTE
    } else {
        120
    }
}
pub const MAX_EXECUTION_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_EXECUTION_INTEGER: u64 = 9_007_199_254_740_991;
fn positive(value: u64, path: &str) -> Result<()> {
    if !(1..=MAX_EXECUTION_INTEGER).contains(&value) {
        return Err(invalid(path, "A positive safe integer is required"));
    }
    Ok(())
}
fn digest(value: &str, path: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid(path, "A lowercase SHA-256 digest is required"));
    }
    Ok(())
}
fn duration(value: u64) -> Result<()> {
    if !(1000..=10_000).contains(&value) {
        return Err(invalid(
            "lease_duration_ms",
            "An execution lease must be 1–10 seconds",
        ));
    }
    Ok(())
}
fn text(value: &str, path: &str) -> Result<()> {
    if value.len() > MAX_EXECUTION_TEXT_BYTES
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            path,
            "Plain text must fit 16 KiB without other control characters",
        ));
    }
    Ok(())
}
fn frame<T: Serialize>(value: &T) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|_| invalid("frame", "Invalid execution JSON"))?
        .len()
        > super::MAX_WORKER_FRAME_BYTES
    {
        return Err(invalid("frame", "Worker frame exceeds 64 KiB"));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelFence {
    pub channel_id: String,
    pub controller_boot_id: String,
    pub worker_boot_id: String,
}
impl ChannelFence {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.channel_id, "channel.channel_id")?;
        identifier(&self.controller_boot_id, "channel.controller_boot_id")?;
        identifier(&self.worker_boot_id, "channel.worker_boot_id")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIdentity {
    pub command_id: String,
    pub attempt_id: String,
    pub fingerprint: String,
}
impl ExecutionIdentity {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.command_id, "identity.command_id")?;
        identifier(&self.attempt_id, "identity.attempt_id")?;
        digest(&self.fingerprint, "identity.fingerprint")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFence {
    pub channel: ChannelFence,
    pub attempt_id: String,
    pub execution_epoch: u64,
}
impl AttemptFence {
    pub fn validate(&self) -> Result<()> {
        self.channel.validate()?;
        identifier(&self.attempt_id, "attempt_id")?;
        positive(self.execution_epoch, "execution_epoch")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBudgets {
    pub max_seconds: u64,
    pub max_output_bytes: usize,
    pub lease_duration_ms: u64,
}
impl ExecutionBudgets {
    pub fn validate(&self) -> Result<()> {
        if !(1..=1800).contains(&self.max_seconds)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_output_bytes)
        {
            return Err(invalid(
                "budgets",
                "Execution budget exceeds the supported hard limits",
            ));
        }
        duration(self.lease_duration_ms)?;
        if self.lease_duration_ms > self.max_seconds * 1000 {
            return Err(invalid(
                "lease_duration_ms",
                "Initial lease exceeds the execution time budget",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCommand {
    pub channel: ChannelFence,
    pub identity: ExecutionIdentity,
    pub descriptor: ExportDescriptor,
    pub profile: AgentProfileRef,
    pub definition: AgentDefinition,
    pub input: String,
    pub budgets: ExecutionBudgets,
}
impl ExecutionCommand {
    pub fn validate(&self) -> Result<()> {
        self.channel.validate()?;
        self.identity.validate()?;
        self.descriptor.validate()?;
        if self.descriptor.grant.principal.kind != PrincipalKind::Human
            || !self
                .descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Execute)
        {
            return Err(invalid(
                "descriptor.grant",
                "An explicit Human Read and Execute grant is required",
            ));
        }
        identifier(&self.profile.agent_id, "profile.agent_id")?;
        positive(self.profile.version, "profile.version")?;
        digest(&self.profile.digest, "profile.digest")?;
        self.definition
            .validate()
            .map_err(|error| invalid(&format!("definition.{}", error.path), &error.message))?;
        if !self.definition.mcp.is_empty() {
            return Err(invalid(
                "definition.mcp",
                "MCP execution requires a scoped runtime broker",
            ));
        }
        self.budgets.validate()?;
        if self.budgets.max_seconds > self.definition.max_seconds
            || self.budgets.max_output_bytes > self.definition.max_output_bytes
        {
            return Err(invalid(
                "budgets",
                "Requested budget exceeds the pinned agent definition",
            ));
        }
        text(&self.input, "input")?;
        if self.input.trim().is_empty() {
            return Err(invalid("input", "Nonempty execution input is required"));
        }
        frame(self)
    }
}
/// Immutable worker acceptance. Replaying this receipt never renews its lease.
/// The initial duration must exactly match the accepted command, never a clamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub fence: AttemptFence,
    pub identity: ExecutionIdentity,
    pub lease_duration_ms: u64,
}
impl ExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        self.fence.validate()?;
        self.identity.validate()?;
        if self.fence.attempt_id != self.identity.attempt_id {
            return Err(invalid(
                "receipt",
                "Receipt attempt identity differs from its fence",
            ));
        }
        duration(self.lease_duration_ms)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTerminalStatus {
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
impl ExecutionTerminalStatus {
    fn status(self) -> ExecutionStatus {
        match self {
            Self::Completed => ExecutionStatus::Completed,
            Self::Failed => ExecutionStatus::Failed,
            Self::Cancelled => ExecutionStatus::Cancelled,
            Self::Unknown => ExecutionStatus::Unknown,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOutcome {
    pub outcome_id: String,
    pub digest: String,
    pub status: ExecutionTerminalStatus,
    pub text: String,
}
impl ExecutionOutcome {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.outcome_id, "outcome.outcome_id")?;
        digest(&self.digest, "outcome.digest")?;
        text(&self.text, "outcome.text")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecord {
    pub receipt: ExecutionReceipt,
    pub status: ExecutionStatus,
    pub outcome: Option<ExecutionOutcome>,
}
impl ExecutionRecord {
    pub fn validate(&self) -> Result<()> {
        self.receipt.validate()?;
        match (&self.outcome, self.status) {
            (None, ExecutionStatus::Accepted | ExecutionStatus::Running) => Ok(()),
            (Some(outcome), status) if outcome.status.status() == status => outcome.validate(),
            _ => Err(invalid(
                "record",
                "Terminal status requires its matching outcome; live status has none",
            )),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCancelDisposition {
    Requested,
    AlreadyTerminal,
    Unconfirmed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionErrorCode {
    Unauthorized,
    AuthorityRevoked,
    StaleChannel,
    StaleAttempt,
    LeaseExpired,
    InvalidSequence,
    IdentityConflict,
    UnsupportedProfile,
    ProfileMismatch,
    CapacityReached,
    Unavailable,
    InternalError,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionControllerMessage {
    Execute {
        command: Box<ExecutionCommand>,
    },
    /// Executor must also cap this against the immutable task's remaining budget.
    Renew {
        fence: AttemptFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    Cancel {
        fence: AttemptFence,
    },
    /// Read-only lookup works even when the original acceptance/epoch was lost.
    Reconcile {
        channel: ChannelFence,
        identity: ExecutionIdentity,
    },
    /// A fresh channel may acknowledge a historical original-fence outcome.
    /// This never renews, authorizes or starts work on either channel.
    OutcomeAck {
        channel: ChannelFence,
        identity: ExecutionIdentity,
        fence: AttemptFence,
        outcome_id: String,
        outcome_digest: String,
    },
}
impl ExecutionControllerMessage {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Execute { command } => command.validate()?,
            Self::Renew {
                fence,
                sequence,
                lease_duration_ms,
            } => {
                fence.validate()?;
                positive(*sequence, "sequence")?;
                duration(*lease_duration_ms)?;
            }
            Self::Cancel { fence } => fence.validate()?,
            Self::Reconcile { channel, identity } => {
                channel.validate()?;
                identity.validate()?;
            }
            Self::OutcomeAck {
                channel,
                identity,
                fence,
                outcome_id,
                outcome_digest,
            } => {
                channel.validate()?;
                identity.validate()?;
                fence.validate()?;
                if identity.attempt_id != fence.attempt_id {
                    return Err(invalid(
                        "identity",
                        "Acknowledged attempt differs from its original fence",
                    ));
                }
                identifier(outcome_id, "outcome_id")?;
                digest(outcome_digest, "outcome_digest")?;
            }
        }
        frame(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionWorkerMessage {
    Accepted {
        record: ExecutionRecord,
    },
    Renewed {
        fence: AttemptFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    CancelAck {
        fence: AttemptFence,
        disposition: ExecutionCancelDisposition,
    },
    Outcome {
        fence: AttemptFence,
        outcome: ExecutionOutcome,
    },
    Reconciled {
        channel: ChannelFence,
        identity: ExecutionIdentity,
        record: Option<ExecutionRecord>,
    },
    /// Durable negative cleanup receipt, emitted only after exact idempotent
    /// worker SQL deletion. `channel` is current; `fence` remains historical.
    OutcomeAcknowledged {
        channel: ChannelFence,
        identity: ExecutionIdentity,
        fence: AttemptFence,
        outcome_id: String,
        outcome_digest: String,
    },
    OutcomeAckRejected {
        channel: ChannelFence,
        identity: ExecutionIdentity,
        fence: AttemptFence,
        outcome_id: String,
        outcome_digest: String,
        code: ExecutionErrorCode,
    },
    Rejected {
        channel: ChannelFence,
        identity: ExecutionIdentity,
        code: ExecutionErrorCode,
    },
    ControlRejected {
        fence: AttemptFence,
        code: ExecutionErrorCode,
    },
}
impl ExecutionWorkerMessage {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Accepted { record } => record.validate()?,
            Self::Renewed {
                fence,
                sequence,
                lease_duration_ms,
            } => {
                fence.validate()?;
                positive(*sequence, "sequence")?;
                duration(*lease_duration_ms)?;
            }
            Self::CancelAck { fence, .. } | Self::ControlRejected { fence, .. } => {
                fence.validate()?
            }
            Self::Outcome { fence, outcome } => {
                fence.validate()?;
                outcome.validate()?;
            }
            Self::Reconciled {
                channel,
                identity,
                record,
            } => {
                channel.validate()?;
                identity.validate()?;
                if let Some(record) = record {
                    record.validate()?;
                    if record.receipt.identity != *identity {
                        return Err(invalid(
                            "record",
                            "Reconciled receipt differs from the queried identity",
                        ));
                    }
                }
            }
            Self::OutcomeAcknowledged {
                channel,
                identity,
                fence,
                outcome_id,
                outcome_digest,
            }
            | Self::OutcomeAckRejected {
                channel,
                identity,
                fence,
                outcome_id,
                outcome_digest,
                ..
            } => {
                channel.validate()?;
                identity.validate()?;
                fence.validate()?;
                if identity.attempt_id != fence.attempt_id {
                    return Err(invalid(
                        "identity",
                        "Acknowledged attempt differs from its original fence",
                    ));
                }
                identifier(outcome_id, "outcome_id")?;
                digest(outcome_digest, "outcome_digest")?;
            }
            Self::Rejected {
                channel, identity, ..
            } => {
                channel.validate()?;
                identity.validate()?;
            }
        }
        frame(self)
    }
}
pub fn decode_execution_controller_frame(value: &str) -> Result<ExecutionControllerMessage> {
    let message: ExecutionControllerMessage = decode(value)?;
    message.validate()?;
    Ok(message)
}
pub fn decode_execution_worker_frame(value: &str) -> Result<ExecutionWorkerMessage> {
    let message: ExecutionWorkerMessage = decode(value)?;
    message.validate()?;
    Ok(message)
}
#[cfg(test)]
mod tests;
