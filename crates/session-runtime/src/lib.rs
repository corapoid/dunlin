//! Durable interactive-session journal, independent of a transport or harness.
//! Adapters must load current authority for every operation. No prompt dispatch
//! happens here; dispatching is persisted BEFORE a worker may write to ACP.
use dunlin_domain::collaboration::{
    AccessContext, AuthorityStamp, Permission, PolicyError, PrincipalKind,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod admissions;
pub use admissions::{ADMISSION_PROFILE_SCHEMA, ADMISSION_SCHEMA, AdmissionResult};
mod sharing;
pub use sharing::{SHARING_SCHEMA, ShareAudit, ShareState, SharedReader};
mod permissions;
pub use permissions::{
    MCP_INPUT_SCHEMA, McpApproval, McpCompletion, McpDispatchPermit, McpInput, McpInputChallenge,
    McpInputDecision, McpInputScope, McpInvocation, McpPermissionChallenge, PERMISSION_SCHEMA,
    PermissionChallenge, PermissionDecision, PermissionStatus,
};
mod controls;
pub use controls::{CONTROL_SCHEMA, DispatchAdmission};
mod dispatch;
pub use dispatch::DISPATCH_SCHEMA;
mod channels;
pub use channels::{
    CHANNEL_SCHEMA, ChannelCut, ChannelHistory, ChannelPageCut, ChannelReplay, ChannelSubscription,
    FrozenChatHistory, HISTORY_BYTES, HISTORY_TURNS, HistoryPage,
};
mod projection;
pub use projection::PROJECTION_SCHEMA;
mod stream;
pub use stream::{
    STREAM_SCHEMA, StreamCursor, StreamEvent, StreamReplay, StreamSnapshot, Subscription,
};

pub const SCHEMA: &str = "
CREATE TABLE interactive_sessions (
 id TEXT PRIMARY KEY, workspace TEXT NOT NULL, project TEXT NOT NULL,
 owner TEXT NOT NULL REFERENCES users(id), phase TEXT NOT NULL CHECK(phase IN ('pending','active','interrupted')),
 epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0), runtime TEXT, authority TEXT,
 seq INTEGER NOT NULL DEFAULT 0 CHECK(seq>=0), bytes INTEGER NOT NULL DEFAULT 0 CHECK(bytes>=0),
 FOREIGN KEY(workspace,project) REFERENCES team_projects(workspace_id,id)
);
CREATE TABLE interactive_turns (
 id TEXT PRIMARY KEY, session TEXT NOT NULL REFERENCES interactive_sessions(id),
 idem TEXT NOT NULL, fingerprint TEXT NOT NULL, prompt TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('queued','dispatching','running','completed','failed','cancelled','unknown')), position INTEGER NOT NULL CHECK(position>0),
 UNIQUE(session,idem)
);
CREATE TABLE interactive_events (
 session TEXT NOT NULL REFERENCES interactive_sessions(id),
 seq INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL,
 PRIMARY KEY(session,seq)
);
";
const MAX_EVENTS: i64 = 1024;
const MAX_BYTES: i64 = 8 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Session authority denied: {0}")]
    Policy(#[from] PolicyError),
    #[error("Session persistence unavailable")]
    Sql(#[from] rusqlite::Error),
    #[error("Invalid session record")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Limit(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Pending,
    Active,
    Interrupted,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Queued,
    Dispatching,
    Running,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub id: String,
    pub workspace: String,
    pub project: String,
    pub owner: String,
    pub phase: SessionPhase,
    pub epoch: u64,
    pub sequence: u64,
    #[serde(skip_serializing)]
    runtime: Option<String>,
    #[serde(skip_serializing)]
    authority: Option<AuthorityStamp>,
}
/// Server-only dispatch fencing; deliberately not deserializable from a client.
#[derive(Clone, Debug)]
pub struct Lease {
    pub epoch: u64,
    pub instance: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Turn {
    pub id: String,
    pub prompt: String,
    pub phase: TurnPhase,
}
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub sequence: u64,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Verified backend metadata, not a deserializable request or authority source.
#[derive(Clone, Debug, Serialize)]
pub struct HarnessInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<dunlin_domain::agent::AgentProfileRef>,
    pub remote_session_id: String,
    pub binding_revision: u64,
    pub model: String,
    pub mode: String,
    pub max_seconds: u64,
    pub max_output_bytes: usize,
}

type SharedOwnerValidator<'a> = dyn Fn(&AuthorityStamp) -> Result<()> + 'a;

pub struct Journal<'a> {
    connection: &'a Connection,
    invalidator: Option<Box<dyn Fn() + 'a>>,
    shared_owner_validator: Option<Box<SharedOwnerValidator<'a>>>,
}
fn nonnegative(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn sql_integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Limit("Sequence or epoch limit reached"))
}
fn phase<T: serde::de::DeserializeOwned>(raw: String) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(raw))?)
}
fn phase_name<T: Serialize>(value: T) -> Result<String> {
    Ok(serde_json::to_value(value)?
        .as_str()
        .ok_or(Error::Limit("Invalid phase"))?
        .into())
}
impl<'a> Journal<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            invalidator: None,
            shared_owner_validator: None,
        }
    }
    pub fn notifying(mut self, invalidator: impl Fn() + 'a) -> Self {
        self.invalidator = Some(Box::new(invalidator));
        self
    }
    fn changed(&self) {
        if let Some(invalidate) = &self.invalidator {
            invalidate();
        }
    }
    fn session(&self, id: &str) -> Result<Session> {
        let (workspace,project,owner,status,epoch,runtime,authority,seq)=self.connection.query_row("SELECT workspace,project,owner,phase,epoch,runtime,authority,seq FROM interactive_sessions WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,nonnegative(r,4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,nonnegative(r,7)?))).optional()?.ok_or(PolicyError::PermissionDenied)?;
        Ok(Session {
            id: id.into(),
            workspace,
            project,
            owner,
            phase: phase(status)?,
            epoch,
            runtime,
            authority: authority
                .map(|raw| serde_json::from_str(&raw))
                .transpose()?,
            sequence: seq,
        })
    }
    fn authorized(
        &self,
        context: &AccessContext,
        id: &str,
        permission: Permission,
    ) -> Result<Session> {
        let s = self.session(id)?;
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        if context.membership.user_id != s.owner {
            if permission != Permission::ViewProject {
                return Err(PolicyError::PermissionDenied.into());
            }
            self.authorize_shared_reader(context, &s)?;
        }
        context.authorize_project(&s.workspace, &s.project, permission)?;
        Ok(s)
    }
    fn leased(&self, context: &AccessContext, id: &str, lease: &Lease) -> Result<Session> {
        let s = self.authorized(context, id, Permission::CreateSession)?;
        self.check_lease(&s, lease)?;
        context.revalidate(
            s.authority.as_ref().ok_or(PolicyError::StaleAuthority)?,
            Permission::CreateSession,
        )?;
        Ok(s)
    }
    fn check_lease(&self, s: &Session, lease: &Lease) -> Result<()> {
        if s.phase != SessionPhase::Active
            || s.epoch != lease.epoch
            || s.runtime.as_deref() != Some(lease.instance.as_str())
        {
            return Err(Error::Conflict("Runtime lease was fenced"));
        }
        Ok(())
    }
    fn event(
        &self,
        id: &str,
        kind: &str,
        mut payload: serde_json::Value,
        recovery: bool,
    ) -> Result<()> {
        let at: String =
            self.connection
                .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                    r.get(0)
                })?;
        payload["_recorded_at"] = serde_json::json!(at);
        let raw = serde_json::to_string(&payload)?;
        let (seq, bytes): (i64, i64) = self.connection.query_row(
            "SELECT seq,bytes FROM interactive_sessions WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        // Reserve room for a terminal recovery record even after normal quota fills.
        let (event_limit, byte_limit) = if recovery {
            (MAX_EVENTS, MAX_BYTES)
        } else {
            (MAX_EVENTS - 16, MAX_BYTES - 8192)
        };
        if raw.len() > 65536 || seq >= event_limit || bytes + raw.len() as i64 > byte_limit {
            return Err(Error::Limit("Session journal limit reached"));
        }
        self.connection.execute(
            "INSERT INTO interactive_events VALUES(?1,?2,?3,?4)",
            params![id, seq + 1, kind, raw],
        )?;
        self.connection.execute(
            "UPDATE interactive_sessions SET seq=seq+1,bytes=bytes+?2 WHERE id=?1",
            params![id, raw.len() as i64],
        )?;
        self.project_event(
            id,
            u64::try_from(seq + 1).map_err(|_| Error::Limit("Invalid source sequence"))?,
            kind,
            &payload,
        )?;
        Ok(())
    }
    pub fn create(
        &self,
        context: &AccessContext,
        workspace: &str,
        project: &str,
    ) -> Result<Session> {
        let tx = self.connection.unchecked_transaction()?;
        let session = self.create_inner(context, workspace, project)?;
        tx.commit()?;
        self.changed();
        Ok(session)
    }
    fn create_inner(
        &self,
        context: &AccessContext,
        workspace: &str,
        project: &str,
    ) -> Result<Session> {
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        context.authorize_project(workspace, project, Permission::CreateSession)?;
        let total: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM interactive_sessions", [], |r| {
                    r.get(0)
                })?;
        let own: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM interactive_sessions WHERE owner=?1",
            [&context.membership.user_id],
            |r| r.get(0),
        )?;
        if total >= 256 || own >= 64 {
            return Err(Error::Limit("Interactive session limit reached"));
        }
        let id = format!("ses-{}", uuid::Uuid::now_v7());
        self.connection.execute("INSERT INTO interactive_sessions(id,workspace,project,owner,phase) VALUES(?1,?2,?3,?4,'pending')",params![id,workspace,project,context.membership.user_id])?;
        self.event(&id, "session_created", serde_json::json!({}), false)?;
        self.session(&id)
    }
    pub fn sequence(&self, context: &AccessContext, id: &str) -> Result<u64> {
        Ok(self
            .authorized(context, id, Permission::ViewProject)?
            .sequence)
    }
    pub fn snapshot(&self, context: &AccessContext, id: &str) -> Result<(Session, Vec<Turn>)> {
        let tx = self.connection.unchecked_transaction()?;
        let snapshot = self.snapshot_rows(context, id)?;
        tx.commit()?;
        Ok(snapshot)
    }
    fn snapshot_rows(&self, context: &AccessContext, id: &str) -> Result<(Session, Vec<Turn>)> {
        let s = self.authorized(context, id, Permission::ViewProject)?;
        let mut query = self.connection.prepare(
            "SELECT id,prompt,phase FROM interactive_turns WHERE session=?1 ORDER BY position",
        )?;
        let turns = query
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .map(|r| {
                let (id, prompt, status) = r?;
                Ok(Turn {
                    id,
                    prompt,
                    phase: phase(status)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((s, turns))
    }
    pub fn enqueue(
        &self,
        context: &AccessContext,
        id: &str,
        key: &str,
        prompt: &str,
    ) -> Result<String> {
        if key.is_empty() || key.len() > 128 || prompt.trim().is_empty() || prompt.len() > 32768 {
            return Err(Error::Limit("Invalid prompt or idempotency key"));
        }
        let tx = self.connection.unchecked_transaction()?;
        self.authorized(context, id, Permission::CreateSession)?;
        let fingerprint = hex::encode(Sha256::digest(prompt.as_bytes()));
        if let Some((turn, previous)) = tx
            .query_row(
                "SELECT id,fingerprint FROM interactive_turns WHERE session=?1 AND idem=?2",
                params![id, key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != fingerprint {
                return Err(Error::Conflict(
                    "Idempotency key was used with another prompt",
                ));
            }
            return Ok(turn);
        }
        let (total,queued):(i64,i64)=tx.query_row("SELECT COUNT(*),coalesce(sum(phase='queued'),0) FROM interactive_turns WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if total >= 256 || queued >= 32 {
            return Err(Error::Limit("Session turn queue limit reached"));
        }
        let turn = format!("turn-{}", uuid::Uuid::now_v7());
        let position: u64 = tx.query_row(
            "SELECT coalesce(max(position),0)+1 FROM interactive_turns WHERE session=?1",
            [id],
            |r| nonnegative(r, 0),
        )?;
        tx.execute(
            "INSERT INTO interactive_turns VALUES(?1,?2,?3,?4,?5,'queued',?6)",
            params![turn, id, key, fingerprint, prompt, sql_integer(position)?],
        )?;
        self.event(
            id,
            "turn_queued",
            serde_json::json!({"turn_id":turn}),
            false,
        )?;
        tx.commit()?;
        self.changed();
        Ok(turn)
    }
    pub fn acquire(
        &self,
        context: &AccessContext,
        id: &str,
        expected_epoch: u64,
        instance: &str,
    ) -> Result<Lease> {
        let tx = self.connection.unchecked_transaction()?;
        let lease = self.acquire_inner(context, id, expected_epoch, instance)?;
        tx.commit()?;
        self.changed();
        Ok(lease)
    }
    fn acquire_inner(
        &self,
        context: &AccessContext,
        id: &str,
        expected_epoch: u64,
        instance: &str,
    ) -> Result<Lease> {
        if instance.is_empty() || instance.len() > 128 {
            return Err(Error::Limit("Invalid runtime instance"));
        }
        let s = self.authorized(context, id, Permission::CreateSession)?;
        if s.phase == SessionPhase::Active || s.epoch != expected_epoch {
            return Err(Error::Conflict("Session runtime changed"));
        }
        let epoch = expected_epoch
            .checked_add(1)
            .filter(|e| *e <= i64::MAX as u64)
            .ok_or(Error::Limit("Runtime epoch limit reached"))?;
        let stamp =
            context.authorize_project(&s.workspace, &s.project, Permission::CreateSession)?;
        self.connection.execute("UPDATE interactive_sessions SET phase='active',epoch=?2,runtime=?3,authority=?4 WHERE id=?1",params![id,sql_integer(epoch)?,instance,serde_json::to_string(&stamp)?])?;
        self.event(
            id,
            "runtime_acquired",
            serde_json::json!({"epoch":epoch}),
            false,
        )?;
        Ok(Lease {
            epoch,
            instance: instance.into(),
        })
    }
    pub fn claim_next(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
    ) -> Result<Option<Turn>> {
        let tx = self.connection.unchecked_transaction()?;
        let claimed = self.claim_next_in_transaction(context, id, lease)?;
        tx.commit()?;
        if claimed.is_some() {
            self.notify_committed_claim()?;
        }
        Ok(claimed)
    }
    /// Claim and persist dispatch intent inside the caller's SQL transaction.
    /// The caller must roll back on any later failure, and notify only after a
    /// successful outer commit. This does not authorize external dispatch.
    pub fn claim_next_in_transaction(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
    ) -> Result<Option<Turn>> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Turn claim requires an existing SQL transaction",
            ));
        }
        self.leased(context, id, lease)?;
        let busy:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND phase IN ('dispatching','running'))",[id],|r|r.get(0))?;
        if busy {
            return Err(Error::Conflict("A turn is already active"));
        }
        let next=self.connection.query_row("SELECT id,prompt FROM interactive_turns WHERE session=?1 AND phase='queued' ORDER BY position LIMIT 1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        let Some((turn, prompt)) = next else {
            return Ok(None);
        };
        self.connection.execute(
            "UPDATE interactive_turns SET phase='dispatching' WHERE id=?1",
            [&turn],
        )?;
        self.event(
            id,
            "turn_dispatching",
            serde_json::json!({"turn_id":turn}),
            false,
        )?;
        Ok(Some(Turn {
            id: turn,
            prompt,
            phase: TurnPhase::Dispatching,
        }))
    }
    /// The outer transaction owner calls this exactly once after committing a
    /// nonempty claim, never after rollback or a claim returning None.
    pub fn notify_committed_claim(&self) -> Result<()> {
        if !self.connection.is_autocommit() {
            return Err(Error::Conflict("Claim notification requires committed SQL"));
        }
        self.changed();
        Ok(())
    }
    pub fn transition(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        target: TurnPhase,
    ) -> Result<()> {
        self.transition_timed(context, id, lease, turn, target, None)
    }
    /// Duration comes from the worker's monotonic clock. None represents an
    /// unknown duration (e.g. recovery), not a timestamp subtraction.
    pub fn transition_timed(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        target: TurnPhase,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.transition_timed_in_transaction(context, id, lease, turn, target, duration_ms)?;
        tx.commit()?;
        self.notify_committed_update()
    }
    /// Persist the turn state and its AHP projection in the caller's transaction.
    /// The caller must commit its inbox/dedupe/ACK watermark in that same cut,
    /// roll back every error, then notify only after the outer commit.
    #[allow(clippy::too_many_arguments)]
    pub fn transition_timed_in_transaction(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        target: TurnPhase,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        self.require_transaction()?;
        if duration_ms.is_some_and(|d| d > i64::MAX as u64) {
            return Err(Error::Limit("Invalid turn duration"));
        }
        self.leased(context, id, lease)?;
        if matches!(
            target,
            TurnPhase::Completed | TurnPhase::Failed | TurnPhase::Cancelled
        ) && self.cancellation_pending(id, turn)?
        {
            self.settle_cancellation_inner(
                id,
                lease,
                turn,
                &format!("agent_response:{}", phase_name(target)?),
                duration_ms.unwrap_or(0),
            )?;
            return Ok(());
        }
        let previous: String = self
            .connection
            .query_row(
                "SELECT phase FROM interactive_turns WHERE session=?1 AND id=?2",
                params![id, turn],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::Conflict("Turn is unavailable"))?;
        let previous: TurnPhase = phase(previous)?;
        if !matches!(
            (previous, target),
            (
                TurnPhase::Dispatching,
                TurnPhase::Running | TurnPhase::Failed | TurnPhase::Unknown
            ) | (
                TurnPhase::Running,
                TurnPhase::Completed
                    | TurnPhase::Failed
                    | TurnPhase::Cancelled
                    | TurnPhase::Unknown
            )
        ) {
            return Err(Error::Conflict("Invalid turn transition"));
        }
        if target != TurnPhase::Running {
            self.invalidate_permissions_inner(id, None, Some(turn), "turn_ended")?;
        }
        self.connection.execute(
            "UPDATE interactive_turns SET phase=?2 WHERE id=?1",
            params![turn, phase_name(target)?],
        )?;
        self.event(
            id,
            "turn_phase",
            serde_json::json!({"turn_id":turn,"phase":target,"duration_ms":duration_ms}),
            false,
        )?;
        Ok(())
    }
    pub fn output(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        text: &str,
    ) -> Result<()> {
        if text.len() > 32768 {
            return Err(Error::Limit("Output chunk limit reached"));
        }
        let tx = self.connection.unchecked_transaction()?;
        self.leased(context, id, lease)?;
        let running:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='running')",params![id,turn],|r|r.get(0))?;
        if !running && !self.cancellation_known(id, turn)? {
            return Err(Error::Conflict("Output has no active turn"));
        }
        self.event(
            id,
            "agent_output",
            serde_json::json!({"turn_id":turn,"text":text}),
            false,
        )?;
        tx.commit()?;
        self.changed();
        Ok(())
    }
    pub fn replay(&self, context: &AccessContext, id: &str, after: u64) -> Result<Vec<Event>> {
        let tx = self.connection.unchecked_transaction()?;
        let s = self.authorized(context, id, Permission::ViewProject)?;
        if after > s.sequence {
            return Err(Error::Conflict("Cursor requires a snapshot"));
        }
        let mut query=self.connection.prepare("SELECT seq,kind,payload FROM interactive_events WHERE session=?1 AND seq>?2 ORDER BY seq LIMIT 128")?;
        let events = query
            .query_map(params![id, sql_integer(after)?], |r| {
                Ok((
                    nonnegative(r, 0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .map(|r| {
                let (sequence, kind, payload) = r?;
                Ok(Event {
                    sequence,
                    kind,
                    payload: serde_json::from_str(&payload)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit()?;
        Ok(events)
    }
    /// Server-owned worker only; must still work after user authority is revoked.
    pub fn interrupt(&self, id: &str, lease: &Lease, reason: &'static str) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.interrupt_in_transaction(id, lease, reason)?;
        tx.commit()?;
        self.notify_committed_update()
    }
    /// Trusted negative cleanup remains usable after user revocation. A journal
    /// interruption is not OS StopProof and never releases a worker reservation.
    pub fn interrupt_in_transaction(
        &self,
        id: &str,
        lease: &Lease,
        reason: &'static str,
    ) -> Result<()> {
        self.require_transaction()?;
        self.check_lease(&self.session(id)?, lease)?;
        self.invalidate_permissions_inner(id, None, None, "runtime_interrupted")?;
        self.unconfirm_cancellations(id)?;
        self.connection.execute("UPDATE interactive_turns SET phase='unknown' WHERE session=?1 AND phase IN ('dispatching','running')", [id])?;
        self.connection.execute("UPDATE interactive_sessions SET phase='interrupted',runtime=NULL,authority=NULL WHERE id=?1", [id])?;
        self.event(
            id,
            "runtime_interrupted",
            serde_json::json!({"reason":reason}),
            true,
        )?;
        Ok(())
    }
    pub fn harness_ready(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        info: HarnessInfo,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.harness_ready_in_transaction(context, id, lease, info)?;
        tx.commit()?;
        self.notify_committed_update()
    }
    pub fn harness_ready_in_transaction(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        info: HarnessInfo,
    ) -> Result<()> {
        self.require_transaction()?;
        if info.remote_session_id.is_empty()
            || info.remote_session_id.len() > 256
            || info.remote_session_id.chars().any(char::is_control)
        {
            return Err(Error::Limit("Invalid ACP session identity"));
        }
        self.leased(context, id, lease)?;
        self.event(
            id,
            "harness_ready",
            serde_json::json!({"backend":info,"epoch":lease.epoch}),
            false,
        )?;
        Ok(())
    }
    pub fn agent_update(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        update: serde_json::Value,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.agent_update_in_transaction(context, id, lease, turn, update)?;
        tx.commit()?;
        self.notify_committed_update()
    }
    pub fn agent_update_in_transaction(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        update: serde_json::Value,
    ) -> Result<()> {
        self.require_transaction()?;
        self.leased(context, id, lease)?;
        let running: bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='running')",params![id,turn],|r|r.get(0))?;
        if !running && !self.cancellation_known(id, turn)? {
            return Err(Error::Conflict("Update has no active turn"));
        }
        self.event(
            id,
            "agent_update",
            serde_json::json!({"turn_id":turn,"update":update}),
            false,
        )?;
        Ok(())
    }
    fn require_transaction(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Journal update requires an existing SQL transaction",
            ));
        }
        Ok(())
    }
    pub fn notify_committed_update(&self) -> Result<()> {
        if !self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Journal notification requires committed SQL",
            ));
        }
        self.changed();
        Ok(())
    }
    /// Coordinator startup only, after acquiring its exclusive listener. Never on
    /// a read-only database open: CLI readers must not interrupt the live runtime.
    pub fn recover(&self) -> Result<usize> {
        let tx = self.connection.unchecked_transaction()?;
        let ids = {
            let mut q = tx.prepare("SELECT id FROM interactive_sessions WHERE phase='active'")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for id in &ids {
            self.invalidate_permissions_inner(id, None, None, "recovery_required")?;
            self.unconfirm_cancellations(id)?;
            tx.execute("UPDATE interactive_turns SET phase='unknown' WHERE session=?1 AND phase IN ('dispatching','running')",[id])?;
            tx.execute("UPDATE interactive_sessions SET phase='interrupted',runtime=NULL,authority=NULL WHERE id=?1",[id])?;
            self.event(id, "recovery_required", serde_json::json!({}), true)?;
        }
        tx.commit()?;
        if !ids.is_empty() {
            self.changed();
        }
        Ok(ids.len())
    }
}

#[cfg(test)]
mod claim_tests;
#[cfg(test)]
mod update_transaction_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::collaboration::{Membership, ProjectGrant, WorkspaceRole};
    use std::collections::BTreeSet;
    pub(super) fn context() -> AccessContext {
        AccessContext {
            principal: PrincipalKind::Human,
            membership: Membership {
                workspace_id: "workspace".into(),
                user_id: "owner".into(),
                role: WorkspaceRole::Member,
                revision: 1,
                active: true,
            },
            project_grant: Some(ProjectGrant {
                workspace_id: "workspace".into(),
                project_id: "project".into(),
                user_id: "owner".into(),
                permissions: BTreeSet::from([Permission::ViewProject, Permission::CreateSession]),
                revision: 1,
            }),
        }
    }
    pub(super) fn setup(connection: &Connection) {
        connection.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE users(id TEXT PRIMARY KEY); CREATE TABLE devices(id TEXT PRIMARY KEY); INSERT INTO users VALUES('owner'); CREATE TABLE team_projects(workspace_id TEXT,id TEXT,UNIQUE(workspace_id,id)); INSERT INTO team_projects VALUES('workspace','project');").unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection.execute_batch(STREAM_SCHEMA).unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        tx.execute_batch(PROJECTION_SCHEMA).unwrap();
        tx.execute_batch(CHANNEL_SCHEMA).unwrap();
        tx.execute_batch(DISPATCH_SCHEMA).unwrap();
        tx.execute_batch(CONTROL_SCHEMA).unwrap();
        tx.execute_batch(PERMISSION_SCHEMA).unwrap();
        tx.execute_batch(MCP_INPUT_SCHEMA).unwrap();
        tx.execute_batch(SHARING_SCHEMA).unwrap();
        tx.execute_batch(ADMISSION_SCHEMA).unwrap();
        Journal::new(connection).backfill_projection().unwrap();
        Journal::new(connection).backfill_channels().unwrap();
        Journal::new(connection).backfill_dispatch().unwrap();
        tx.commit().unwrap();
    }
    #[test]
    fn queue_is_idempotent_serial_and_persists_output_before_completion() {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let j = Journal::new(&c);
        let a = context();
        let s = j.create(&a, "workspace", "project").unwrap();
        let first = j.enqueue(&a, &s.id, "key", "first").unwrap();
        assert_eq!(j.enqueue(&a, &s.id, "key", "first").unwrap(), first);
        assert!(j.enqueue(&a, &s.id, "key", "changed").is_err());
        let second = j.enqueue(&a, &s.id, "second", "second").unwrap();
        let lease = j.acquire(&a, &s.id, 0, "runtime-a").unwrap();
        assert!(j.acquire(&a, &s.id, 1, "runtime-b").is_err());
        assert_eq!(j.claim_next(&a, &s.id, &lease).unwrap().unwrap().id, first);
        assert!(j.claim_next(&a, &s.id, &lease).is_err());
        assert!(
            j.transition(&a, &s.id, &lease, &first, TurnPhase::Completed)
                .is_err()
        );
        j.transition(&a, &s.id, &lease, &first, TurnPhase::Running)
            .unwrap();
        j.output(&a, &s.id, &lease, &first, "streamed text")
            .unwrap();
        j.transition(&a, &s.id, &lease, &first, TurnPhase::Completed)
            .unwrap();
        assert_eq!(j.claim_next(&a, &s.id, &lease).unwrap().unwrap().id, second);
        let events = j.replay(&a, &s.id, 0).unwrap();
        assert!(
            events
                .iter()
                .enumerate()
                .all(|(i, e)| e.sequence == i as u64 + 1)
        );
        assert!(
            events
                .iter()
                .any(|e| e.payload.get("text") == Some(&serde_json::json!("streamed text")))
        );
    }
    #[test]
    fn reopen_and_recovery_never_replay_ambiguous_dispatch_and_fence_old_workers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.db");
        let c = Connection::open(&path).unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let s = j.create(&a, "workspace", "project").unwrap();
        let ambiguous = j.enqueue(&a, &s.id, "uncertain", "do a mutation").unwrap();
        let queued = j.enqueue(&a, &s.id, "queued", "later").unwrap();
        let lease = j.acquire(&a, &s.id, 0, "old-worker").unwrap();
        j.claim_next(&a, &s.id, &lease).unwrap();
        drop(j);
        drop(c);
        let c = Connection::open(&path).unwrap();
        let j = Journal::new(&c);
        assert_eq!(
            j.snapshot(&a, &s.id).unwrap().1[0].phase,
            TurnPhase::Dispatching,
            "database readers do not recover a live actor"
        );
        assert_eq!(j.recover().unwrap(), 1);
        assert_eq!(j.recover().unwrap(), 0);
        let snapshot = j.snapshot(&a, &s.id).unwrap();
        assert_eq!(snapshot.0.phase, SessionPhase::Interrupted);
        assert_eq!(snapshot.1[0].phase, TurnPhase::Unknown);
        assert_eq!(snapshot.1[1].phase, TurnPhase::Queued);
        assert_eq!(
            j.enqueue(&a, &s.id, "uncertain", "do a mutation").unwrap(),
            ambiguous
        );
        assert!(j.claim_next(&a, &s.id, &lease).is_err());
        let new = j.acquire(&a, &s.id, 1, "new-worker").unwrap();
        assert_eq!(new.epoch, 2);
        assert!(
            j.transition(&a, &s.id, &lease, &ambiguous, TurnPhase::Running)
                .is_err()
        );
        assert_eq!(j.claim_next(&a, &s.id, &new).unwrap().unwrap().id, queued);
    }
    #[test]
    fn current_authority_and_private_ownership_are_required() {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let j = Journal::new(&c);
        let a = context();
        let s = j.create(&a, "workspace", "project").unwrap();
        j.enqueue(&a, &s.id, "key", "prompt").unwrap();
        let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
        let mut changed = a.clone();
        changed.membership.revision = 2;
        assert!(j.claim_next(&changed, &s.id, &lease).is_err());
        let mut stranger = a.clone();
        stranger.membership.user_id = "admin".into();
        stranger.membership.role = WorkspaceRole::Owner;
        stranger.project_grant.as_mut().unwrap().user_id = "admin".into();
        assert!(j.snapshot(&stranger, &s.id).is_err());
        let mut revoked = a.clone();
        revoked.membership.active = false;
        assert!(j.replay(&revoked, &s.id, 0).is_err());
        assert!(j.replay(&a, &s.id, 999).is_err());
    }
    #[test]
    fn queue_and_journal_limits_preserve_space_for_recovery() {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let j = Journal::new(&c);
        let a = context();
        let s = j.create(&a, "workspace", "project").unwrap();
        for i in 0..32 {
            j.enqueue(&a, &s.id, &format!("key-{i}"), "prompt").unwrap();
        }
        assert!(j.enqueue(&a, &s.id, "overflow", "prompt").is_err());
        let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
        let turn = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
        j.transition(&a, &s.id, &lease, &turn.id, TurnPhase::Running)
            .unwrap();
        let mut chunks = 0;
        loop {
            match j.output(&a, &s.id, &lease, &turn.id, "chunk") {
                Ok(()) => chunks += 1,
                Err(Error::Limit(_)) => break,
                Err(error) => panic!("Unexpected journal error: {error}"),
            }
        }
        assert!(chunks > 900);
        assert_eq!(j.recover().unwrap(), 1);
        let snapshot = j.snapshot(&a, &s.id).unwrap();
        assert_eq!(snapshot.1[0].phase, TurnPhase::Unknown);
    }
}
