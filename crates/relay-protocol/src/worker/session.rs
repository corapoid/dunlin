//! Unwired v4 persistent ACP session contract. DTO validation and hashes prove
//! shape/integrity only, never current authority, a live lease or process death.
//! V3 terminal execution semantics and all existing handshake versions are intact.
use super::{
    ChannelFence, ExecutionBudgets, ExecutionErrorCode, ExportDescriptor, ExportPermission,
    MAX_EXECUTION_INTEGER, MAX_WORKER_FRAME_BYTES, PrincipalKind, Result, identifier, invalid,
};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
/// Proposed v5 MCP side-channel contract. Existing v4 codecs/admission do not
/// accept these messages or profiles requiring MCP.
pub mod broker;
mod correlation;

pub const WORKER_SESSION_PROTOCOL_VERSION: u16 = 4;
pub const WORKER_SESSION_SUBPROTOCOL: &str = "dunlin.worker.v4";
pub const MAX_SESSION_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_SESSION_CHUNK_BYTES: usize = 4 * 1024;
pub const MAX_SESSION_RETAINED_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SESSION_TURNS: u64 = 128;
pub const MAX_SESSION_SECONDS: u64 = 3600;
pub const MAX_SESSION_REPLAY_EVENTS: u8 = 8;

fn replay_anchor(sequence: u64, digest_value: Option<&str>, limit: u8) -> Result<()> {
    if !(1..=MAX_SESSION_REPLAY_EVENTS).contains(&limit) {
        return Err(invalid("limit", "Replay must request 1–8 events"));
    }
    match (sequence, digest_value) {
        (0, None) => Ok(()),
        (sequence, Some(value)) => {
            positive(sequence, "after_sequence")?;
            digest(value)
        }
        _ => Err(invalid(
            "anchor",
            "Replay requires an exact predecessor digest",
        )),
    }
}

fn positive(value: u64, path: &str) -> Result<()> {
    if !(1..=MAX_EXECUTION_INTEGER).contains(&value) {
        return Err(invalid(path, "A positive safe integer is required"));
    }
    Ok(())
}
fn digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("digest", "A lowercase SHA-256 digest is required"));
    }
    Ok(())
}
fn lease(value: u64) -> Result<()> {
    if !(1000..=10_000).contains(&value) {
        return Err(invalid("lease_duration_ms", "Lease must be 1–10 seconds"));
    }
    Ok(())
}
fn text(value: &str, max: usize) -> Result<()> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            "text",
            "Bounded plaintext without other controls is required",
        ));
    }
    Ok(())
}
fn frame<T: Serialize>(value: &T) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|_| invalid("frame", "Invalid session JSON"))?
        .len()
        > MAX_WORKER_FRAME_BYTES
    {
        return Err(invalid("frame", "Worker frame exceeds 64 KiB"));
    }
    Ok(())
}
fn equal_session(left: &SessionFence, right: &SessionFence) -> Result<()> {
    if left != right {
        return Err(invalid("session", "Session fence differs"));
    }
    Ok(())
}
fn current(channel: &ChannelFence, session: &SessionFence) -> Result<()> {
    if channel != &session.channel {
        return Err(invalid(
            "channel",
            "Positive control requires its original current channel",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    pub intent_id: String,
    pub fingerprint: String,
}
impl SessionIdentity {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.intent_id, "session.intent_id")?;
        digest(&self.fingerprint)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBudgets {
    pub max_seconds: u64,
    pub max_output_bytes: usize,
    pub idle_timeout_ms: u64,
    pub lease_duration_ms: u64,
    pub max_turns: u64,
    pub retained_bytes: usize,
}
impl SessionBudgets {
    pub fn validate(&self) -> Result<()> {
        if !(1..=MAX_SESSION_SECONDS).contains(&self.max_seconds)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_output_bytes)
            || !(1..=MAX_SESSION_TURNS).contains(&self.max_turns)
            || !(1..=MAX_SESSION_RETAINED_BYTES).contains(&self.retained_bytes)
            || !(1..=30_000).contains(&self.idle_timeout_ms)
            || self.idle_timeout_ms > self.max_seconds * 1000
        {
            return Err(invalid("budgets", "Session budget exceeds hard limits"));
        }
        lease(self.lease_duration_ms)?;
        if self.lease_duration_ms > self.max_seconds * 1000 {
            return Err(invalid("budgets", "Initial lease exceeds session lifetime"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOpenCommand {
    pub channel: ChannelFence,
    pub controller_session_id: String,
    pub identity: SessionIdentity,
    pub descriptor: ExportDescriptor,
    pub profile: AgentProfileRef,
    pub definition: AgentDefinition,
    pub budgets: SessionBudgets,
}
impl SessionOpenCommand {
    pub fn validate(&self) -> Result<()> {
        self.validate_mode(false)
    }
    fn validate_mode(&self, broker: bool) -> Result<()> {
        self.channel.validate()?;
        identifier(&self.controller_session_id, "controller_session_id")?;
        self.identity.validate()?;
        self.descriptor.validate()?;
        if self.descriptor.grant.principal.kind != PrincipalKind::Human
            || !self
                .descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Read)
            || !self
                .descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Execute)
        {
            return Err(invalid(
                "descriptor",
                "Own Human Read and Execute are required",
            ));
        }
        identifier(&self.profile.agent_id, "profile.agent_id")?;
        positive(self.profile.version, "profile.version")?;
        digest(&self.profile.digest)?;
        self.definition
            .validate()
            .map_err(|e| invalid(&format!("definition.{}", e.path), &e.message))?;
        if !broker && !self.definition.mcp.is_empty() {
            return Err(invalid(
                "definition.mcp",
                "MCP execution requires a scoped runtime broker",
            ));
        }
        self.budgets.validate()?;
        if self.budgets.max_seconds > self.definition.max_seconds
            || self.budgets.max_output_bytes > self.definition.max_output_bytes
        {
            return Err(invalid("budgets", "Session exceeds pinned profile budgets"));
        }
        frame(self)
    }
}
/// Worker-issued process/session identity. Deserialize never grants runtime ownership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFence {
    pub channel: ChannelFence,
    pub controller_session_id: String,
    pub worker_session_id: String,
    pub session_epoch: u64,
}
impl SessionFence {
    pub fn validate(&self) -> Result<()> {
        self.channel.validate()?;
        identifier(&self.controller_session_id, "controller_session_id")?;
        identifier(&self.worker_session_id, "worker_session_id")?;
        positive(self.session_epoch, "session_epoch")
    }
}
/// Immutable acceptance is historical; replaying it never renews a lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReceipt {
    pub identity: SessionIdentity,
    pub fence: SessionFence,
    pub budgets: SessionBudgets,
}
impl SessionReceipt {
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        self.fence.validate()?;
        self.budgets.validate()
    }
    pub fn validate_for_open(&self, command: &SessionOpenCommand) -> Result<()> {
        self.validate_for_open_mode(command, false)
    }
    fn validate_for_open_mode(&self, command: &SessionOpenCommand, broker: bool) -> Result<()> {
        self.validate()?;
        command.validate_mode(broker)?;
        if self.identity != command.identity
            || self.fence.channel != command.channel
            || self.fence.controller_session_id != command.controller_session_id
            || self.budgets != command.budgets
        {
            return Err(invalid(
                "receipt",
                "Session receipt differs from immutable open intent",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnIdentity {
    pub command_id: String,
    pub turn_id: String,
    pub fingerprint: String,
}
impl TurnIdentity {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.command_id, "command_id")?;
        identifier(&self.turn_id, "turn_id")?;
        digest(&self.fingerprint)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCommand {
    pub session: SessionFence,
    pub identity: TurnIdentity,
    pub turn_index: u64,
    pub input: String,
    pub budgets: ExecutionBudgets,
}
impl TurnCommand {
    pub fn validate(&self) -> Result<()> {
        self.session.validate()?;
        self.identity.validate()?;
        if !(1..=MAX_SESSION_TURNS).contains(&self.turn_index) {
            return Err(invalid(
                "turn_index",
                "Turn index exceeds session hard limit",
            ));
        }
        text(&self.input, MAX_SESSION_INPUT_BYTES)?;
        if self.input.trim().is_empty() {
            return Err(invalid("input", "A nonempty prompt is required"));
        }
        self.budgets.validate()?;
        frame(self)
    }
    pub fn validate_for_session(&self, session: &SessionReceipt) -> Result<()> {
        self.validate()?;
        session.validate()?;
        equal_session(&self.session, &session.fence)?;
        turn_budgets(&self.budgets, self.turn_index, &session.budgets)
    }
}
fn turn_budgets(turn: &ExecutionBudgets, index: u64, session: &SessionBudgets) -> Result<()> {
    if index > session.max_turns
        || turn.max_seconds > session.max_seconds
        || turn.max_output_bytes > session.max_output_bytes
    {
        return Err(invalid("turn", "Turn exceeds immutable session budgets"));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnFence {
    pub session: SessionFence,
    pub turn_id: String,
    pub turn_index: u64,
    pub turn_epoch: u64,
}
impl TurnFence {
    pub fn validate(&self) -> Result<()> {
        self.session.validate()?;
        identifier(&self.turn_id, "turn_id")?;
        positive(self.turn_epoch, "turn_epoch")?;
        if !(1..=MAX_SESSION_TURNS).contains(&self.turn_index) {
            return Err(invalid("turn_index", "Invalid turn index"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnReceipt {
    pub identity: TurnIdentity,
    pub fence: TurnFence,
    pub budgets: ExecutionBudgets,
}
impl TurnReceipt {
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        self.fence.validate()?;
        self.budgets.validate()?;
        if self.identity.turn_id != self.fence.turn_id {
            return Err(invalid("turn", "Receipt identity and fence differ"));
        }
        Ok(())
    }
    pub fn validate_for_command(&self, command: &TurnCommand) -> Result<()> {
        self.validate()?;
        command.validate()?;
        if self.identity != command.identity
            || self.fence.session != command.session
            || self.fence.turn_index != command.turn_index
            || self.budgets != command.budgets
        {
            return Err(invalid(
                "receipt",
                "Turn receipt differs from immutable command",
            ));
        }
        Ok(())
    }
    pub fn validate_for_session(&self, session: &SessionReceipt) -> Result<()> {
        self.validate()?;
        session.validate()?;
        equal_session(&self.fence.session, &session.fence)?;
        turn_budgets(&self.budgets, self.fence.turn_index, &session.budgets)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalStatus {
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
impl TurnTerminalStatus {
    fn status(self) -> TurnStatus {
        match self {
            Self::Completed => TurnStatus::Completed,
            Self::Failed => TurnStatus::Failed,
            Self::Cancelled => TurnStatus::Cancelled,
            Self::Unknown => TurnStatus::Unknown,
        }
    }
}
/// Last event is the matching TurnCompleted event, not a process stop proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCompletion {
    pub status: TurnTerminalStatus,
    pub final_sequence: u64,
    pub event_digest: String,
}
impl TurnCompletion {
    fn validate(&self) -> Result<()> {
        positive(self.final_sequence, "final_sequence")?;
        digest(&self.event_digest)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRecord {
    pub receipt: TurnReceipt,
    pub status: TurnStatus,
    pub completion: Option<TurnCompletion>,
}
impl TurnRecord {
    /// Reconciliation's terminal reference must name this exact stored event.
    /// Receivers still enforce contiguous delivery and independently verify its hash.
    pub fn validate_for_event(&self, event: &SessionEvent) -> Result<()> {
        self.validate()?;
        event.validate()?;
        let completion = self
            .completion
            .as_ref()
            .ok_or_else(|| invalid("completion", "A terminal turn is required"))?;
        equal_session(&self.receipt.fence.session, &event.session)?;
        if completion.final_sequence != event.sequence || completion.event_digest != event.digest {
            return Err(invalid(
                "completion",
                "Terminal reference differs from stored event",
            ));
        }
        match &event.event {
            SessionEventKind::TurnCompleted { receipt, status }
                if receipt == &self.receipt && *status == completion.status =>
            {
                Ok(())
            }
            _ => Err(invalid(
                "completion",
                "Event does not complete this exact turn",
            )),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.receipt.validate()?;
        match (&self.completion, self.status) {
            (None, TurnStatus::Accepted | TurnStatus::Running) => Ok(()),
            (Some(completion), status) if completion.status.status() == status => {
                completion.validate()
            }
            _ => Err(invalid(
                "turn",
                "Terminal turn requires matching completion; live turn has none",
            )),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Accepted,
    Idle,
    Busy,
    Unknown,
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopReason {
    Closed,
    Cancelled,
    LeaseExpired,
    AuthorityRevoked,
    BudgetExceeded,
    HarnessFailed,
    Unknown,
}
/// Only an actual runtime stop producer may emit this. JSON is not StopProof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStop {
    pub reason: SessionStopReason,
    pub final_sequence: u64,
    pub event_digest: String,
}
impl SessionStop {
    fn validate(&self) -> Result<()> {
        positive(self.final_sequence, "final_sequence")?;
        digest(&self.event_digest)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub receipt: SessionReceipt,
    pub phase: SessionPhase,
    pub active_turn: Option<TurnReceipt>,
    pub stop: Option<SessionStop>,
}
impl SessionRecord {
    pub fn validate_for_stop_event(&self, event: &SessionEvent) -> Result<()> {
        self.validate()?;
        event.validate()?;
        let stop = self
            .stop
            .as_ref()
            .ok_or_else(|| invalid("stop", "A stopped session is required"))?;
        equal_session(&self.receipt.fence, &event.session)?;
        if stop.final_sequence != event.sequence || stop.event_digest != event.digest {
            return Err(invalid("stop", "Stop reference differs from stored event"));
        }
        match &event.event {
            SessionEventKind::SessionStopped { reason } if *reason == stop.reason => Ok(()),
            _ => Err(invalid("stop", "Event does not stop this exact session")),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.receipt.validate()?;
        if let Some(turn) = &self.active_turn {
            turn.validate_for_session(&self.receipt)?;
        }
        match (self.phase, &self.active_turn, &self.stop) {
            (SessionPhase::Accepted | SessionPhase::Idle, None, None)
            | (SessionPhase::Busy, Some(_), None)
            | (SessionPhase::Unknown, _, None) => Ok(()),
            (SessionPhase::Stopped, None, Some(stop)) => stop.validate(),
            _ => Err(invalid(
                "session",
                "Phase, active turn and stop record disagree",
            )),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Text,
    Reasoning,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionEventKind {
    Text {
        turn: TurnFence,
        kind: TextKind,
        text: String,
    },
    /// ACP response only; the session/process reservation remains held.
    TurnCompleted {
        receipt: TurnReceipt,
        status: TurnTerminalStatus,
    },
    /// Runtime guarantees process stop before this event becomes deliverable.
    SessionStopped { reason: SessionStopReason },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEvent {
    pub session: SessionFence,
    pub sequence: u64,
    pub previous: Option<StreamPosition>,
    pub digest: String,
    pub event: SessionEventKind,
}
/// Exact previous durable event. A receiver must also verify the stored digest,
/// not merely these structural chain bounds, before projecting a terminal event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamPosition {
    pub session: SessionFence,
    pub sequence: u64,
    pub digest: String,
}
impl SessionEvent {
    /// Verify an original stored event against the preceding authorized durable
    /// position. The envelope's current transport never rewrites its hash pins.
    pub fn validate_after(
        &self,
        session: &SessionFence,
        sequence: u64,
        predecessor_digest: Option<&str>,
    ) -> Result<()> {
        replay_anchor(sequence, predecessor_digest, 1)?;
        self.validate()?;
        session.validate()?;
        equal_session(session, &self.session)?;
        if sequence.checked_add(1) != Some(self.sequence)
            || self.previous.as_ref().map(|p| p.digest.as_str()) != predecessor_digest
            || session_event_digest(self)? != self.digest
        {
            return Err(invalid(
                "event",
                "Replay event differs from its exact hashed predecessor",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.session.validate()?;
        positive(self.sequence, "sequence")?;
        digest(&self.digest)?;
        match (&self.previous, self.sequence) {
            (None, 1) => {}
            (Some(previous), sequence) if sequence > 1 => {
                previous.session.validate()?;
                equal_session(&self.session, &previous.session)?;
                positive(previous.sequence, "previous.sequence")?;
                digest(&previous.digest)?;
                if previous.sequence != sequence - 1 {
                    return Err(invalid(
                        "previous",
                        "Event must reference its exact preceding sequence",
                    ));
                }
            }
            _ => return Err(invalid("previous", "Event chain is incomplete")),
        }
        match &self.event {
            SessionEventKind::Text {
                turn,
                text: content,
                ..
            } => {
                turn.validate()?;
                equal_session(&self.session, &turn.session)?;
                text(content, MAX_SESSION_CHUNK_BYTES)?;
                if content.is_empty() {
                    return Err(invalid("text", "Empty output chunks are unsupported"));
                }
            }
            SessionEventKind::TurnCompleted { receipt, .. } => {
                receipt.validate()?;
                equal_session(&self.session, &receipt.fence.session)?;
            }
            SessionEventKind::SessionStopped { .. } => {}
        }
        frame(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionControllerMessage {
    OpenSession {
        command: Box<SessionOpenCommand>,
    },
    SubmitTurn {
        command: Box<TurnCommand>,
    },
    RenewSession {
        session: SessionFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    RenewTurn {
        turn: TurnFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    CancelTurn {
        turn: TurnFence,
    },
    CloseSession {
        session: SessionFence,
    },
    ReconcileSession {
        channel: ChannelFence,
        identity: SessionIdentity,
    },
    ReconcileTurn {
        channel: ChannelFence,
        session: SessionFence,
        identity: TurnIdentity,
    },
    /// Read-only replay on a current channel of an original immutable stream.
    ReplayStream {
        channel: ChannelFence,
        session: SessionFence,
        after_sequence: u64,
        after_digest: Option<String>,
        limit: u8,
    },
    /// Current channel, historical original session; negative cleanup only.
    AckStream {
        channel: ChannelFence,
        session: SessionFence,
        through_sequence: u64,
        event_digest: String,
    },
}
impl SessionControllerMessage {
    /// Contextual fence check; current transport remains an opaque runtime input.
    /// Only explicit read/reconcile/ACK variants permit historical session fences.
    pub fn validate_for_channel(&self, channel: &ChannelFence) -> Result<()> {
        self.validate()?;
        channel.validate()?;
        let actual = match self {
            Self::OpenSession { command } => &command.channel,
            Self::SubmitTurn { command } => &command.session.channel,
            Self::RenewSession { session, .. } | Self::CloseSession { session } => &session.channel,
            Self::RenewTurn { turn, .. } | Self::CancelTurn { turn } => &turn.session.channel,
            Self::ReconcileSession { channel, .. }
            | Self::ReconcileTurn { channel, .. }
            | Self::ReplayStream { channel, .. }
            | Self::AckStream { channel, .. } => channel,
        };
        if actual != channel {
            return Err(invalid(
                "channel",
                "Message belongs to another channel/boot",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::OpenSession { command } => command.validate()?,
            Self::SubmitTurn { command } => command.validate()?,
            Self::RenewSession {
                session,
                sequence,
                lease_duration_ms,
            } => {
                session.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
            }
            Self::RenewTurn {
                turn,
                sequence,
                lease_duration_ms,
            } => {
                turn.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
            }
            Self::CancelTurn { turn } => turn.validate()?,
            Self::CloseSession { session } => session.validate()?,
            Self::ReconcileSession { channel, identity } => {
                channel.validate()?;
                identity.validate()?;
            }
            Self::ReconcileTurn {
                channel,
                session,
                identity,
            } => {
                channel.validate()?;
                session.validate()?;
                identity.validate()?;
            }
            Self::AckStream {
                channel,
                session,
                through_sequence,
                event_digest,
            } => {
                channel.validate()?;
                session.validate()?;
                positive(*through_sequence, "through_sequence")?;
                digest(event_digest)?;
            }
            Self::ReplayStream {
                channel,
                session,
                after_sequence,
                after_digest,
                limit,
            } => {
                channel.validate()?;
                session.validate()?;
                replay_anchor(*after_sequence, after_digest.as_deref(), *limit)?;
            }
        }
        frame(self)
    }
}
/// Compact exact correlation reference; never echoes prompt/profile body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionRequestRef {
    Open {
        identity: SessionIdentity,
    },
    Submit {
        session: SessionFence,
        identity: TurnIdentity,
    },
    RenewSession {
        session: SessionFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    RenewTurn {
        turn: TurnFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    Cancel {
        turn: TurnFence,
    },
    Close {
        session: SessionFence,
    },
    QuerySession {
        identity: SessionIdentity,
    },
    QueryTurn {
        session: SessionFence,
        identity: TurnIdentity,
    },
    Replay {
        session: SessionFence,
        after_sequence: u64,
        after_digest: Option<String>,
        limit: u8,
    },
    Ack {
        session: SessionFence,
        through_sequence: u64,
        event_digest: String,
    },
}
impl SessionRequestRef {
    fn validate_at(&self, channel: &ChannelFence) -> Result<()> {
        match self {
            Self::Open { identity } | Self::QuerySession { identity } => identity.validate(),
            Self::Submit { session, identity } => {
                session.validate()?;
                identity.validate()?;
                current(channel, session)
            }
            Self::RenewSession {
                session,
                sequence,
                lease_duration_ms,
            } => {
                session.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
                current(channel, session)
            }
            Self::RenewTurn {
                turn,
                sequence,
                lease_duration_ms,
            } => {
                turn.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
                current(channel, &turn.session)
            }
            Self::Cancel { turn } => {
                turn.validate()?;
                current(channel, &turn.session)
            }
            Self::Close { session } => {
                session.validate()?;
                current(channel, session)
            }
            Self::QueryTurn { session, identity } => {
                session.validate()?;
                identity.validate()
            }
            Self::Ack {
                session,
                through_sequence,
                event_digest,
            } => {
                session.validate()?;
                positive(*through_sequence, "through_sequence")?;
                digest(event_digest)
            }
            Self::Replay {
                session,
                after_sequence,
                after_digest,
                limit,
            } => {
                session.validate()?;
                replay_anchor(*after_sequence, after_digest.as_deref(), *limit)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionWorkerMessage {
    SessionAccepted {
        receipt: SessionReceipt,
    },
    /// Emitted only after actual ACP initialize/new/config negotiation.
    SessionReady {
        receipt: SessionReceipt,
    },
    TurnAccepted {
        receipt: TurnReceipt,
    },
    Event {
        event: SessionEvent,
    },
    SessionRenewed {
        session: SessionFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    TurnRenewed {
        turn: TurnFence,
        sequence: u64,
        lease_duration_ms: u64,
    },
    CancelRequested {
        turn: TurnFence,
    },
    CloseRequested {
        session: SessionFence,
    },
    SessionReconciled {
        channel: ChannelFence,
        identity: SessionIdentity,
        record: Option<SessionRecord>,
    },
    TurnReconciled {
        channel: ChannelFence,
        session: SessionFence,
        identity: TurnIdentity,
        record: Option<TurnRecord>,
    },
    StreamAcknowledged {
        channel: ChannelFence,
        session: SessionFence,
        through_sequence: u64,
        event_digest: String,
    },
    /// A fixed authorized durable stream cut. Missing/pruned history is an
    /// explicit Rejected response, never a fabricated empty/end-of-stream page.
    StreamReplayed {
        channel: ChannelFence,
        session: SessionFence,
        after_sequence: u64,
        after_digest: Option<String>,
        limit: u8,
        through_sequence: u64,
        events: Vec<SessionEvent>,
        more: bool,
    },
    Rejected {
        channel: ChannelFence,
        request: SessionRequestRef,
        code: ExecutionErrorCode,
    },
}
impl SessionWorkerMessage {
    /// Exact RPC correlation. Unsolicited readiness/events never acknowledge
    /// another request, and a rejection must name the complete request identity.
    pub fn validate_for_request(&self, request: &SessionControllerMessage) -> Result<()> {
        self.validate()?;
        request.validate()?;
        match (self, request) {
            (
                Self::SessionAccepted { receipt },
                SessionControllerMessage::OpenSession { command },
            ) => receipt.validate_for_open(command),
            (Self::TurnAccepted { receipt }, SessionControllerMessage::SubmitTurn { command }) => {
                receipt.validate_for_command(command)
            }
            (
                Self::SessionRenewed {
                    session,
                    sequence,
                    lease_duration_ms,
                },
                SessionControllerMessage::RenewSession {
                    session: expected,
                    sequence: expected_sequence,
                    lease_duration_ms: expected_duration,
                },
            ) if session == expected
                && sequence == expected_sequence
                && lease_duration_ms == expected_duration =>
            {
                Ok(())
            }
            (
                Self::TurnRenewed {
                    turn,
                    sequence,
                    lease_duration_ms,
                },
                SessionControllerMessage::RenewTurn {
                    turn: expected,
                    sequence: expected_sequence,
                    lease_duration_ms: expected_duration,
                },
            ) if turn == expected
                && sequence == expected_sequence
                && lease_duration_ms == expected_duration =>
            {
                Ok(())
            }
            (
                Self::CancelRequested { turn },
                SessionControllerMessage::CancelTurn { turn: expected },
            ) if turn == expected => Ok(()),
            (
                Self::CloseRequested { session },
                SessionControllerMessage::CloseSession { session: expected },
            ) if session == expected => Ok(()),
            (
                Self::SessionReconciled {
                    channel, identity, ..
                },
                SessionControllerMessage::ReconcileSession {
                    channel: expected,
                    identity: expected_identity,
                },
            ) if channel == expected && identity == expected_identity => Ok(()),
            (
                Self::TurnReconciled {
                    channel,
                    session,
                    identity,
                    ..
                },
                SessionControllerMessage::ReconcileTurn {
                    channel: expected,
                    session: expected_session,
                    identity: expected_identity,
                },
            ) if channel == expected
                && session == expected_session
                && identity == expected_identity =>
            {
                Ok(())
            }
            (
                Self::StreamAcknowledged {
                    channel,
                    session,
                    through_sequence,
                    event_digest,
                },
                SessionControllerMessage::AckStream {
                    channel: expected,
                    session: expected_session,
                    through_sequence: expected_sequence,
                    event_digest: expected_digest,
                },
            ) if channel == expected
                && session == expected_session
                && through_sequence == expected_sequence
                && event_digest == expected_digest =>
            {
                Ok(())
            }
            (
                Self::Rejected {
                    channel,
                    request: reference,
                    ..
                },
                expected,
            ) if channel == expected.channel() && *reference == expected.reference() => Ok(()),
            (
                Self::StreamReplayed {
                    channel,
                    session,
                    after_sequence,
                    after_digest,
                    limit,
                    ..
                },
                SessionControllerMessage::ReplayStream {
                    channel: expected_channel,
                    session: expected_session,
                    after_sequence: expected_sequence,
                    after_digest: expected_digest,
                    limit: expected_limit,
                },
            ) if channel == expected_channel
                && session == expected_session
                && after_sequence == expected_sequence
                && after_digest == expected_digest
                && limit == expected_limit =>
            {
                Ok(())
            }
            _ => Err(invalid(
                "correlation",
                "Session reply differs from the exact request",
            )),
        }
    }
    pub fn validate_for_channel(&self, channel: &ChannelFence) -> Result<()> {
        self.validate()?;
        channel.validate()?;
        let actual = match self {
            Self::SessionAccepted { receipt } | Self::SessionReady { receipt } => {
                &receipt.fence.channel
            }
            Self::TurnAccepted { receipt } => &receipt.fence.session.channel,
            Self::Event { event } => &event.session.channel,
            Self::SessionRenewed { session, .. } | Self::CloseRequested { session } => {
                &session.channel
            }
            Self::TurnRenewed { turn, .. } | Self::CancelRequested { turn } => {
                &turn.session.channel
            }
            Self::SessionReconciled { channel, .. }
            | Self::TurnReconciled { channel, .. }
            | Self::StreamAcknowledged { channel, .. }
            | Self::StreamReplayed { channel, .. }
            | Self::Rejected { channel, .. } => channel,
        };
        if actual != channel {
            return Err(invalid(
                "channel",
                "Message belongs to another channel/boot",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::SessionAccepted { receipt } | Self::SessionReady { receipt } => {
                receipt.validate()?
            }
            Self::TurnAccepted { receipt } => receipt.validate()?,
            Self::Event { event } => event.validate()?,
            Self::SessionRenewed {
                session,
                sequence,
                lease_duration_ms,
            } => {
                session.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
            }
            Self::TurnRenewed {
                turn,
                sequence,
                lease_duration_ms,
            } => {
                turn.validate()?;
                positive(*sequence, "sequence")?;
                lease(*lease_duration_ms)?;
            }
            Self::CancelRequested { turn } => turn.validate()?,
            Self::CloseRequested { session } => session.validate()?,
            Self::SessionReconciled {
                channel,
                identity,
                record,
            } => {
                channel.validate()?;
                identity.validate()?;
                if let Some(record) = record {
                    record.validate()?;
                    if record.receipt.identity != *identity {
                        return Err(invalid("record", "Queried session identity differs"));
                    }
                }
            }
            Self::TurnReconciled {
                channel,
                session,
                identity,
                record,
            } => {
                channel.validate()?;
                session.validate()?;
                identity.validate()?;
                if let Some(record) = record {
                    record.validate()?;
                    equal_session(session, &record.receipt.fence.session)?;
                    if record.receipt.identity != *identity {
                        return Err(invalid("record", "Queried turn identity differs"));
                    }
                }
            }
            Self::StreamAcknowledged {
                channel,
                session,
                through_sequence,
                event_digest,
            } => {
                channel.validate()?;
                session.validate()?;
                positive(*through_sequence, "through_sequence")?;
                digest(event_digest)?;
            }
            Self::Rejected {
                channel, request, ..
            } => {
                channel.validate()?;
                request.validate_at(channel)?;
            }
            Self::StreamReplayed {
                channel,
                session,
                after_sequence,
                after_digest,
                limit,
                through_sequence,
                events,
                more,
            } => {
                channel.validate()?;
                session.validate()?;
                replay_anchor(*after_sequence, after_digest.as_deref(), *limit)?;
                if *through_sequence > MAX_EXECUTION_INTEGER
                    || after_sequence > through_sequence
                    || events.len() > usize::from(*limit)
                {
                    return Err(invalid(
                        "replay",
                        "Replay cut or page exceeds requested bounds",
                    ));
                }
                let mut sequence = *after_sequence;
                let mut previous = after_digest.as_deref();
                for event in events {
                    event.validate_after(session, sequence, previous)?;
                    sequence = event.sequence;
                    previous = Some(&event.digest);
                }
                if sequence > *through_sequence
                    || *more != (sequence < *through_sequence)
                    || (events.is_empty() && *more)
                {
                    return Err(invalid(
                        "replay",
                        "Replay page is missing contiguous retained history",
                    ));
                }
            }
        }
        frame(self)
    }
}
pub fn decode_session_controller_frame(text: &str) -> Result<SessionControllerMessage> {
    let value: SessionControllerMessage = super::decode(text)?;
    value.validate()?;
    Ok(value)
}
pub fn decode_session_worker_frame(text: &str) -> Result<SessionWorkerMessage> {
    let value: SessionWorkerMessage = super::decode(text)?;
    value.validate()?;
    Ok(value)
}
fn hash<T: Serialize>(domain: &[u8], value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("hash", "Session codec failed"))?;
    let mut state = Sha256::new();
    state.update(domain);
    state.update(bytes);
    Ok(hex::encode(state.finalize()))
}
/// Validate all pins first, then clear ONLY this identity's self-fingerprint.
pub fn session_open_fingerprint(command: &SessionOpenCommand) -> Result<String> {
    command.validate()?;
    let mut command = command.clone();
    command.identity.fingerprint.clear();
    hash(b"dunlin.worker.session.v4.open\0", &command)
}
pub fn session_turn_fingerprint(command: &TurnCommand) -> Result<String> {
    command.validate()?;
    let mut command = command.clone();
    command.identity.fingerprint.clear();
    hash(b"dunlin.worker.session.v4.turn\0", &command)
}
pub fn session_event_digest(event: &SessionEvent) -> Result<String> {
    event.validate()?;
    let mut event = event.clone();
    event.digest.clear();
    hash(b"dunlin.worker.session.v4.event\0", &event)
}

#[cfg(test)]
mod tests;
