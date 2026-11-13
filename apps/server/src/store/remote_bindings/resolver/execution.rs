//! Passive controller execution ledger. Nothing here sends a frame, spawns a
//! process or turns a v2 observation into v3 authority. Every positive cut has a
//! mandatory trusted producer callback supplied by the internal v3 Service.
//! Public interactive Remote admission uses the separate persistent v4 adapter.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Internal v3 compatibility API is retained and exercised by transport/runtime tests; public interactive sessions use v4"
    )
)]
use super::*;
use crate::store::worker_channels::{ChannelLease, WorkerIdentity};
use crate::worker_execution::{command_fingerprint, outcome_digest, profile_digest};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use dunlin_relay_protocol::worker::{
    ChannelFence, ExecutionBudgets, ExecutionCommand, ExecutionControllerMessage,
    ExecutionIdentity, ExecutionRecord, ExecutionStatus, MAX_EXECUTION_INTEGER,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const REMOTE_EXECUTION_SCHEMA: &str = "
CREATE TABLE controller_remote_executions (
 attempt_id TEXT PRIMARY KEY, command_id TEXT NOT NULL UNIQUE,
 owner TEXT NOT NULL REFERENCES users(id), device TEXT NOT NULL REFERENCES devices(id),
 workspace TEXT NOT NULL, project TEXT NOT NULL, host TEXT NOT NULL REFERENCES team_hosts(id),
 intent_key TEXT NOT NULL, intent_digest TEXT NOT NULL, request TEXT NOT NULL,
 binding TEXT NOT NULL, authority TEXT NOT NULL, command TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN('pending','dispatched','accepted','unknown','terminal')),
 record TEXT, created_at_ms INTEGER NOT NULL CHECK(created_at_ms>=0),
 last_renew_sequence INTEGER NOT NULL DEFAULT 0 CHECK(last_renew_sequence>=0), last_renew TEXT,
 cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN(0,1)),
 UNIQUE(device,intent_key),FOREIGN KEY(workspace,project) REFERENCES team_projects(workspace_id,id)
);
CREATE TABLE controller_remote_execution_outcomes (
 attempt_id TEXT PRIMARY KEY REFERENCES controller_remote_executions(attempt_id),
 outcome_id TEXT NOT NULL,digest TEXT NOT NULL,record TEXT NOT NULL,
 ack_pending INTEGER NOT NULL CHECK(ack_pending IN(0,1))
);
";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteExecutionRequest {
    pub intent_key: String,
    pub binding: BindingRef,
    pub profile: AgentProfileRef,
    pub input: String,
    pub budgets: ExecutionBudgets,
}
/// Captured SQL state only. Network Describe must happen after releasing Store.
/// No peer JSON decoder or public field can manufacture this preparation.
pub(crate) struct PreparedRemoteExecution {
    owner: String,
    device: String,
    workspace: String,
    project: String,
    controller_node_id: String,
    request: RemoteExecutionRequest,
    candidate: RemoteTargetCandidate,
    definition: AgentDefinition,
    intent_digest: String,
}
impl PreparedRemoteExecution {
    pub fn host_id(&self) -> &str {
        self.candidate.host_id()
    }
    pub fn export_id(&self) -> &str {
        self.candidate.export_id()
    }
    pub fn export_revision(&self) -> u64 {
        self.candidate.export_revision()
    }
    pub fn principal(&self) -> ExportPrincipal {
        ExportPrincipal {
            kind: PrincipalKind::Human,
            id: self.owner.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RemoteExecutionPhase {
    Pending,
    Dispatched,
    Accepted,
    Unknown,
    Terminal,
}
impl RemoteExecutionPhase {
    fn text(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Dispatched => "dispatched",
            Self::Accepted => "accepted",
            Self::Unknown => "unknown",
            Self::Terminal => "terminal",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "dispatched" => Ok(Self::Dispatched),
            "accepted" => Ok(Self::Accepted),
            "unknown" => Ok(Self::Unknown),
            "terminal" => Ok(Self::Terminal),
            _ => Err(execution_conflict()),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct RemoteExecutionReservation {
    pub identity: ExecutionIdentity,
    pub phase: RemoteExecutionPhase,
    pub record: Option<ExecutionRecord>,
    pub replayed: bool,
}
/// Read-only daemon material. No Deserialize/Serialize/Debug and no authority
/// constructors: positive work still needs a new opaque preparation/observation.
pub(crate) struct RemoteExecutionSnapshot {
    pub command: ExecutionCommand,
    pub record: Option<ExecutionRecord>,
}

struct StoredExecution {
    owner: String,
    device: String,
    workspace: String,
    project: String,
    request: RemoteExecutionRequest,
    intent_digest: String,
    binding: String,
    authority: AuthorityStamp,
    command: ExecutionCommand,
    phase: RemoteExecutionPhase,
    record: Option<ExecutionRecord>,
    created: i64,
    renew_sequence: u64,
    renewal: Option<ExecutionControllerMessage>,
    cancel_requested: bool,
}
impl StoredExecution {
    fn reservation(&self, replayed: bool) -> RemoteExecutionReservation {
        RemoteExecutionReservation {
            identity: self.command.identity.clone(),
            phase: self.phase,
            record: self.record.clone(),
            replayed,
        }
    }
}
fn execution_conflict() -> StoreError {
    StoreError::Conflict("Remote execution identity, authority or state changed".into())
}
fn protocol<T, E>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|_| execution_conflict())
}
fn clock(now: i64) -> Result<()> {
    if now < 0 || now as u64 > MAX_EXECUTION_INTEGER {
        return Err(execution_conflict());
    }
    Ok(())
}
fn execution_sql_integer(value: u64) -> Result<i64> {
    if value > MAX_EXECUTION_INTEGER {
        return Err(execution_conflict());
    }
    i64::try_from(value).map_err(|_| execution_conflict())
}
fn channel_matches(lease: &ChannelLease, fence: &ChannelFence) -> bool {
    lease.channel_id == fence.channel_id
        && lease.controller_boot_id == fence.controller_boot_id
        && lease.worker_boot_id == fence.worker_boot_id
}
impl Store {
    fn remote_execution_caller(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
    ) -> Result<AuthorityStamp> {
        if !self
            .active_device_user(device)?
            .is_some_and(|user| user.id == owner)
        {
            return Err(denied());
        }
        let context = self.team_project_context(workspace, project, owner)?;
        context.authorize_project(workspace, project, Permission::ViewProject)?;
        Ok(context.authorize_project(workspace, project, Permission::CreateSession)?)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_remote_execution_in_transaction(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        request: RemoteExecutionRequest,
        controller_node_id: &str,
    ) -> Result<PreparedRemoteExecution> {
        if self.connection.is_autocommit() {
            return Err(execution_conflict());
        }
        if !(16..=128).contains(&request.intent_key.len())
            || !identifier(&request.intent_key)
            || request.input.trim().is_empty()
            || request.input.len() > 16 * 1024
            || request
                .input
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(StoreError::Limit(
                "Invalid bounded remote execution intent".into(),
            ));
        }
        protocol(request.budgets.validate())?;
        protocol(request.binding.validate())?;
        let authority = self.remote_execution_caller(owner, device, workspace, project)?;
        let candidate = self.capture_remote_target_in_transaction(
            owner,
            workspace,
            project,
            &request.binding,
            controller_node_id,
        )?;
        if candidate.authority != authority {
            return Err(execution_conflict());
        }
        let published = self
            .agent_version(
                owner,
                workspace,
                project,
                &request.profile.agent_id,
                request.profile.version,
            )?
            .filter(|version| version.digest == request.profile.digest)
            .ok_or_else(denied)?;
        protocol(published.definition.validate())?;
        if !published.definition.mcp.is_empty() {
            return Err(StoreError::Limit(
                "MCP execution requires a scoped runtime broker".into(),
            ));
        }
        if profile_digest(&published.definition)? != request.profile.digest
            || request.budgets.max_seconds > published.definition.max_seconds
            || request.budgets.max_output_bytes > published.definition.max_output_bytes
        {
            return Err(execution_conflict());
        }
        let mut hash = Sha256::new();
        hash.update(b"dunlin.controller.remote_execution.intent.v1\0");
        hash.update(serde_json::to_vec(&(
            owner, device, workspace, project, &request, &authority,
        ))?);
        Ok(PreparedRemoteExecution {
            owner: owner.into(),
            device: device.into(),
            workspace: workspace.into(),
            project: project.into(),
            controller_node_id: controller_node_id.into(),
            request,
            candidate,
            definition: published.definition,
            intent_digest: hex::encode(hash.finalize()),
        })
    }
    /// Historical acceptance lookup before network/capacity. This cannot yield
    /// a command or any execution authority, including after recovery/offline.
    pub(crate) fn lookup_remote_execution_intent_in_transaction<F>(
        &self,
        prepared: &PreparedRemoteExecution,
        authorize_caller: F,
    ) -> Result<Option<RemoteExecutionReservation>>
    where
        F: FnOnce(&Store, &str, &str) -> Result<()>,
    {
        let fresh = self.prepare_remote_execution_in_transaction(
            &prepared.owner,
            &prepared.device,
            &prepared.workspace,
            &prepared.project,
            prepared.request.clone(),
            &prepared.controller_node_id,
        )?;
        if fresh.intent_digest != prepared.intent_digest
            || fresh.candidate.binding != prepared.candidate.binding
        {
            return Err(execution_conflict());
        }
        authorize_caller(self, &prepared.owner, &prepared.device)?;
        let attempt: Option<String> = self.connection.query_row(
            "SELECT attempt_id FROM controller_remote_executions WHERE device=?1 AND intent_key=?2",
            params![prepared.device, prepared.request.intent_key], |row| row.get(0),
        ).optional()?;
        attempt
            .map(|attempt| {
                let row = self
                    .remote_execution_row(&attempt)?
                    .ok_or_else(execution_conflict)?;
                if row.owner != prepared.owner || row.intent_digest != prepared.intent_digest {
                    return Err(execution_conflict());
                }
                Ok(row.reservation(true))
            })
            .transpose()
    }
    fn remote_execution_row(&self, attempt: &str) -> Result<Option<StoredExecution>> {
        let row = self.connection.query_row(
            "SELECT owner,device,workspace,project,request,intent_digest,binding,authority,command,phase,record,created_at_ms,last_renew_sequence,last_renew,cancel_requested FROM controller_remote_executions WHERE attempt_id=?1",[attempt],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,Option<String>>(10)?,r.get::<_,i64>(11)?,r.get::<_,i64>(12)?,r.get::<_,Option<String>>(13)?,r.get::<_,bool>(14)?))
        ).optional()?;
        row.map(
            |(
                owner,
                device,
                workspace,
                project,
                request,
                intent_digest,
                binding,
                authority,
                command,
                phase,
                record,
                created,
                renew_sequence,
                renewal,
                cancel_requested,
            )| {
                let renew_sequence = u64::try_from(renew_sequence)
                    .ok()
                    .filter(|value| *value <= MAX_EXECUTION_INTEGER)
                    .ok_or_else(execution_conflict)?;
                let command: ExecutionCommand = serde_json::from_str(&command)?;
                protocol(command.validate())?;
                if command.identity.attempt_id != attempt
                    || command_fingerprint(&command)? != command.identity.fingerprint
                    || command.descriptor.grant.principal.id != owner
                {
                    return Err(execution_conflict());
                }
                let record: Option<ExecutionRecord> =
                    record.map(|v| serde_json::from_str(&v)).transpose()?;
                if let Some(record) = &record {
                    self.validate_remote_record(&command, record)?;
                }
                Ok(StoredExecution {
                    owner,
                    device,
                    workspace,
                    project,
                    request: serde_json::from_str(&request)?,
                    intent_digest,
                    binding,
                    authority: serde_json::from_str(&authority)?,
                    command,
                    phase: RemoteExecutionPhase::parse(&phase)?,
                    record,
                    created,
                    renew_sequence,
                    renewal: renewal.map(|v| serde_json::from_str(&v)).transpose()?,
                    cancel_requested,
                })
            },
        )
        .transpose()
    }
    /// Mandatory callback validates current caller device/OIDC/host policy AND
    /// an exact live Service-v3 slot. A v2 ObservedExport alone is insufficient.
    fn validate_execution_observation<F>(
        &self,
        prepared: PreparedRemoteExecution,
        proof: ObservedExport,
        channel: &ChannelFence,
        now: i64,
        authorize_v3: F,
    ) -> Result<(
        PreparedRemoteExecution,
        dunlin_relay_protocol::worker::ExportDescriptor,
    )>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        clock(now)?;
        protocol(channel.validate())?;
        let fresh = self.prepare_remote_execution_in_transaction(
            &prepared.owner,
            &prepared.device,
            &prepared.workspace,
            &prepared.project,
            prepared.request.clone(),
            &prepared.controller_node_id,
        )?;
        if fresh.intent_digest != prepared.intent_digest
            || fresh.candidate.binding != prepared.candidate.binding
            || fresh.definition != prepared.definition
            || !channel_matches(proof.lease(), channel)
        {
            return Err(execution_conflict());
        }
        authorize_v3(
            self,
            &prepared.owner,
            &prepared.device,
            proof.identity(),
            proof.lease(),
            channel,
        )?;
        let descriptor = proof.descriptor().clone();
        if !descriptor
            .grant
            .permissions
            .contains(&ExportPermission::Execute)
        {
            return Err(denied());
        }
        // Current host policy narrows admission; reported capabilities grant
        // nothing. Actual worker-local policy and kernel gates remain mandatory.
        let (definition,drain):(String,bool)=self.connection.query_row(
            "SELECT definition,drain FROM team_hosts WHERE id=?1 AND workspace_id=?2 AND status='approved' AND enrollment_epoch=?3 AND credential_revision=?4",
            params![prepared.candidate.binding.host_id,prepared.workspace,execution_sql_integer(prepared.candidate.binding.host_enrollment_epoch)?,execution_sql_integer(prepared.candidate.binding.credential_revision)?],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let host: dunlin_domain::hosts::HostDefinition = serde_json::from_str(&definition)?;
        protocol(host.validate())?;
        if drain
            || !host
                .capability_policy
                .harnesses
                .contains(&prepared.definition.harness)
        {
            return Err(denied());
        }
        // Use the fresh duplicate candidate here; retain the original opaque
        // preparation for comparisons and durable pin capture below.
        self.finish_remote_target_in_transaction(
            &prepared.owner,
            &prepared.workspace,
            &prepared.project,
            fresh.candidate,
            proof,
            &channel.controller_boot_id,
            &prepared.controller_node_id,
            now,
        )?;
        Ok((prepared, descriptor))
    }
    pub(crate) fn admit_remote_execution<F>(
        &self,
        prepared: PreparedRemoteExecution,
        proof: ObservedExport,
        channel: ChannelFence,
        now: i64,
        authorize_v3: F,
    ) -> Result<RemoteExecutionReservation>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            store.admit_remote_execution_in_transaction(prepared, proof, channel, now, authorize_v3)
        })
    }
    /// Shares the caller's SQL cut with session/turn persistence. The caller
    /// must roll back on error and publish notifications only after COMMIT.
    pub(crate) fn admit_remote_execution_in_transaction<F>(
        &self,
        prepared: PreparedRemoteExecution,
        proof: ObservedExport,
        channel: ChannelFence,
        now: i64,
        authorize_v3: F,
    ) -> Result<RemoteExecutionReservation>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        if self.connection.is_autocommit() {
            return Err(execution_conflict());
        }
        let (prepared, descriptor) =
            self.validate_execution_observation(prepared, proof, &channel, now, authorize_v3)?;
        let existing:Option<String>=self.connection.query_row(
                "SELECT attempt_id FROM controller_remote_executions WHERE device=?1 AND intent_key=?2",
                params![prepared.device,prepared.request.intent_key],|r|r.get(0)).optional()?;
        if let Some(attempt) = existing {
            let row = self
                .remote_execution_row(&attempt)?
                .ok_or_else(execution_conflict)?;
            if row.owner != prepared.owner || row.intent_digest != prepared.intent_digest {
                return Err(execution_conflict());
            }
            return Ok(row.reservation(true));
        }
        let total: i64 = self.connection.query_row(
            "SELECT count(*) FROM controller_remote_executions",
            [],
            |r| r.get(0),
        )?;
        let own: i64 = self.connection.query_row(
            "SELECT count(*) FROM controller_remote_executions WHERE owner=?1",
            [&prepared.owner],
            |r| r.get(0),
        )?;
        let parallel: i64 = self.connection.query_row(
            "SELECT (SELECT count(*) FROM controller_remote_executions WHERE host=?1 AND phase<>'terminal')+(SELECT coalesce(sum(reservation),0) FROM controller_remote_sessions WHERE host=?1)",
            [prepared.host_id()],
            |r| r.get(0),
        )?;
        let definition: String = self.connection.query_row(
            "SELECT definition FROM team_hosts WHERE id=?1",
            [prepared.host_id()],
            |r| r.get(0),
        )?;
        let host: dunlin_domain::hosts::HostDefinition = serde_json::from_str(&definition)?;
        if total >= 1024 || own >= 128 || parallel >= i64::from(host.capability_policy.max_parallel)
        {
            return Err(StoreError::Limit(
                "Remote execution reservation capacity reached".into(),
            ));
        }
        let mut command = ExecutionCommand {
            channel,
            identity: ExecutionIdentity {
                command_id: format!("command-{}", uuid::Uuid::now_v7()),
                attempt_id: format!("attempt-{}", uuid::Uuid::now_v7()),
                fingerprint: String::new(),
            },
            descriptor,
            profile: prepared.request.profile.clone(),
            definition: prepared.definition,
            input: prepared.request.input.clone(),
            budgets: prepared.request.budgets.clone(),
        };
        command.identity.fingerprint = command_fingerprint(&command)?;
        protocol(command.validate())?;
        self.connection.execute("INSERT INTO controller_remote_executions(attempt_id,command_id,owner,device,workspace,project,host,intent_key,intent_digest,request,binding,authority,command,phase,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'pending',?14)",
                params![command.identity.attempt_id,command.identity.command_id,prepared.owner,prepared.device,prepared.workspace,prepared.project,
                    command.descriptor.scope.host_id,prepared.request.intent_key,prepared.intent_digest,serde_json::to_string(&prepared.request)?,
                    serde_json::to_string(&prepared.candidate.binding)?,serde_json::to_string(&prepared.candidate.authority)?,serde_json::to_string(&command)?,now])?;
        Ok(RemoteExecutionReservation {
            identity: command.identity,
            phase: RemoteExecutionPhase::Pending,
            record: None,
            replayed: false,
        })
    }
    fn current_remote_execution(
        &self,
        row: &StoredExecution,
        prepared: &PreparedRemoteExecution,
        descriptor: &dunlin_relay_protocol::worker::ExportDescriptor,
        channel: &ChannelFence,
        now: i64,
    ) -> Result<()> {
        if row.owner != prepared.owner
            || row.device != prepared.device
            || row.workspace != prepared.workspace
            || row.project != prepared.project
            || row.intent_digest != prepared.intent_digest
            || row.authority != prepared.candidate.authority
            || row.binding != serde_json::to_string(&prepared.candidate.binding)?
            || row.command.descriptor != *descriptor
            || row.command.channel != *channel
            || row.command.profile != prepared.request.profile
            || row.command.definition != prepared.definition
            || now < row.created
            || (now - row.created) as u64 >= row.command.budgets.max_seconds * 1000
        {
            return Err(execution_conflict());
        }
        Ok(())
    }
    /// Commit dispatched BEFORE sending. None means never send/replay this work.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_remote_execution<F>(
        &self,
        attempt: &str,
        prepared: PreparedRemoteExecution,
        proof: ObservedExport,
        channel: ChannelFence,
        now: i64,
        authorize_v3: F,
    ) -> Result<Option<ExecutionCommand>>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        self.remote_binding_transaction(false,|store| {
            let (prepared,descriptor)=store.validate_execution_observation(prepared,proof,&channel,now,authorize_v3)?;
            let row=store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            store.current_remote_execution(&row,&prepared,&descriptor,&channel,now)?;
            if row.phase!=RemoteExecutionPhase::Pending || row.cancel_requested { return Ok(None) }
            store.connection.execute("UPDATE controller_remote_executions SET phase='dispatched' WHERE attempt_id=?1 AND phase='pending'",[attempt])?;
            Ok(Some(row.command))
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn renew_remote_execution<F>(
        &self,
        attempt: &str,
        prepared: PreparedRemoteExecution,
        proof: ObservedExport,
        channel: ChannelFence,
        now: i64,
        sequence: u64,
        lease_duration_ms: u64,
        authorize_v3: F,
    ) -> Result<ExecutionControllerMessage>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        self.remote_binding_transaction(false,|store| {
            let (prepared,descriptor)=store.validate_execution_observation(prepared,proof,&channel,now,authorize_v3)?;
            let row=store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            store.current_remote_execution(&row,&prepared,&descriptor,&channel,now)?;
            if row.phase!=RemoteExecutionPhase::Accepted || row.cancel_requested { return Err(execution_conflict()) }
            let receipt=&row.record.as_ref().ok_or_else(execution_conflict)?.receipt;
            let command=ExecutionControllerMessage::Renew{fence:receipt.fence.clone(),sequence,lease_duration_ms};
            protocol(command.validate())?;
            if sequence==row.renew_sequence && row.renewal.as_ref()==Some(&command) { return Ok(command) }
            if sequence!=row.renew_sequence.checked_add(1).ok_or_else(execution_conflict)?
                || (now-row.created) as u64+lease_duration_ms>row.command.budgets.max_seconds*1000
            { return Err(execution_conflict()) }
            store.connection.execute("UPDATE controller_remote_executions SET last_renew_sequence=?2,last_renew=?3 WHERE attempt_id=?1",
                params![attempt,execution_sql_integer(sequence)?,serde_json::to_string(&command)?])?;
            Ok(command)
        })
    }
    fn validate_remote_record(
        &self,
        command: &ExecutionCommand,
        record: &ExecutionRecord,
    ) -> Result<()> {
        protocol(record.validate())?;
        if record.receipt.identity != command.identity
            || record.receipt.fence.channel != command.channel
            || record.receipt.lease_duration_ms != command.budgets.lease_duration_ms
        {
            return Err(execution_conflict());
        }
        if let Some(outcome) = &record.outcome
            && outcome_digest(outcome)? != outcome.digest
        {
            return Err(execution_conflict());
        }
        Ok(())
    }
    /// Trusted transport ingestion only. The mandatory producer callback must
    /// correlate the exact original attempt; peer JSON alone cannot call this.
    /// Wire contract for the future real WorkerService: EVERY terminal record
    /// attests its local reservation is false after actual stop confirmation,
    /// including Unknown (uncertain effects). Only that contract permits this
    /// ledger to release a host slot; DTO validity is not OS-stop evidence.
    pub(crate) fn record_remote_execution<F>(
        &self,
        attempt: &str,
        record: ExecutionRecord,
        authorize_producer: F,
    ) -> Result<bool>
    where
        F: FnOnce(&Store, &ExecutionCommand, &ExecutionRecord) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            store.record_remote_execution_in_transaction(attempt, record, authorize_producer)
        })
    }
    /// Shares the caller's SQL cut with session/turn persistence. The caller
    /// must roll back on error and publish notifications only after COMMIT.
    pub(crate) fn record_remote_execution_in_transaction<F>(
        &self,
        attempt: &str,
        record: ExecutionRecord,
        authorize_producer: F,
    ) -> Result<bool>
    where
        F: FnOnce(&Store, &ExecutionCommand, &ExecutionRecord) -> Result<()>,
    {
        if self.connection.is_autocommit() {
            return Err(execution_conflict());
        }
        let row = self.remote_execution_row(attempt)?.ok_or_else(denied)?;
        self.validate_remote_record(&row.command, &record)?;
        authorize_producer(self, &row.command, &record)?;
        if row.phase == RemoteExecutionPhase::Pending
            || (row.phase == RemoteExecutionPhase::Terminal && row.record.is_none())
        {
            return Err(execution_conflict());
        }
        if let Some(previous) = &row.record {
            if previous.receipt != record.receipt {
                return Err(execution_conflict());
            }
            if let Some(outcome) = &previous.outcome {
                if record
                    .outcome
                    .as_ref()
                    .is_some_and(|incoming| incoming != outcome)
                {
                    return Err(execution_conflict());
                }
                return Ok(false);
            }
            if previous == &record
                || (previous.status == ExecutionStatus::Running
                    && record.status == ExecutionStatus::Accepted)
            {
                return Ok(false);
            }
        }
        let phase = if record.outcome.is_some() {
            RemoteExecutionPhase::Terminal
        } else if row.phase == RemoteExecutionPhase::Unknown {
            RemoteExecutionPhase::Unknown
        } else {
            RemoteExecutionPhase::Accepted
        };
        if let Some(outcome) = &record.outcome {
            self.connection.execute("INSERT INTO controller_remote_execution_outcomes(attempt_id,outcome_id,digest,record,ack_pending) VALUES(?1,?2,?3,?4,1)",
                    params![attempt,outcome.outcome_id,outcome.digest,serde_json::to_string(&record)?])?;
        }
        self.connection.execute(
            "UPDATE controller_remote_executions SET phase=?2,record=?3 WHERE attempt_id=?1",
            params![attempt, phase.text(), serde_json::to_string(&record)?],
        )?;
        Ok(true)
    }
    pub(crate) fn remote_execution<F>(
        &self,
        owner: &str,
        device: &str,
        attempt: &str,
        authorize: F,
    ) -> Result<RemoteExecutionReservation>
    where
        F: FnOnce(&Store, &str, &str) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            if row.owner != owner || row.device != device {
                return Err(denied());
            }
            store.remote_execution_caller(owner, device, &row.workspace, &row.project)?;
            authorize(store, owner, device)?;
            Ok(row.reservation(true))
        })
    }
    fn cancel_remote_execution_in_transaction(
        &self,
        attempt: &str,
        row: &StoredExecution,
    ) -> Result<Option<ExecutionControllerMessage>> {
        if self.connection.is_autocommit() {
            return Err(execution_conflict());
        }
        if row.phase == RemoteExecutionPhase::Terminal {
            return Ok(None);
        }
        let phase = match row.phase {
            // The durable dispatch cut never happened. Suppress locally, with
            // no fabricated worker epoch, receipt or terminal worker outcome.
            RemoteExecutionPhase::Pending => RemoteExecutionPhase::Terminal,
            RemoteExecutionPhase::Dispatched if row.record.is_none() => {
                RemoteExecutionPhase::Unknown
            }
            phase => phase,
        };
        self.connection.execute(
            "UPDATE controller_remote_executions SET cancel_requested=1,phase=?2 WHERE attempt_id=?1",
            params![attempt, phase.text()],
        )?;
        if row.phase == RemoteExecutionPhase::Pending {
            return Ok(None);
        }
        Ok(row
            .record
            .as_ref()
            .map(|record| ExecutionControllerMessage::Cancel {
                fence: record.receipt.fence.clone(),
            }))
    }
    pub(crate) fn cancel_remote_execution<F>(
        &self,
        owner: &str,
        device: &str,
        attempt: &str,
        authorize: F,
    ) -> Result<Option<ExecutionControllerMessage>>
    where
        F: FnOnce(&Store, &str, &str) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            if row.owner != owner || row.device != device {
                return Err(denied());
            }
            store.remote_execution_caller(owner, device, &row.workspace, &row.project)?;
            authorize(store, owner, device)?;
            store.cancel_remote_execution_in_transaction(attempt, &row)
        })
    }
    /// Internal decreasing-authority cleanup. The trusted daemon must attest
    /// its exact stored attempt scope; revoked user/device/grants cannot prevent
    /// cancellation. This is not an HTTP authorization adapter.
    pub(crate) fn cancel_remote_execution_from_guard<F>(
        &self,
        attempt: &str,
        authorize_producer: F,
    ) -> Result<Option<ExecutionControllerMessage>>
    where
        F: FnOnce(&Store, &ExecutionCommand) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            authorize_producer(store, &row.command)?;
            store.cancel_remote_execution_in_transaction(attempt, &row)
        })
    }
    /// Reconcile is a read-only identity query on a newly attested channel. It
    /// cannot return Execute/Renew or revive an old pending command.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_remote_execution<F>(
        &self,
        owner: &str,
        device: &str,
        attempt: &str,
        channel: ChannelFence,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        now: i64,
        authorize_v3: F,
    ) -> Result<ExecutionControllerMessage>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            clock(now)?;
            protocol(channel.validate())?;
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            if row.owner != owner
                || row.device != device
                || identity.host_id() != row.command.descriptor.scope.host_id
                || identity.workspace_id() != row.workspace
                || !channel_matches(lease, &channel)
            {
                return Err(denied());
            }
            store.remote_execution_caller(owner, device, &row.workspace, &row.project)?;
            store.validate_worker_channel_in_transaction(
                identity,
                lease,
                &channel.controller_boot_id,
                now,
            )?;
            authorize_v3(store, owner, device, identity, lease, &channel)?;
            Ok(ExecutionControllerMessage::Reconcile {
                channel,
                identity: row.command.identity,
            })
        })
    }
    /// Controller-side SQL pins only, not worker Execute authority. The actor
    /// must still obtain a new Describe+live-v3 attestation for every positive
    /// dispatch/renew and the worker independently guards its local lease.
    pub(crate) fn remote_execution_controller_authority_in_transaction(
        &self,
        attempt: &str,
        owner: &str,
        device: &str,
        controller_node: &str,
    ) -> Result<RemoteExecutionPhase> {
        let row = self.remote_execution_row(attempt)?.ok_or_else(denied)?;
        if row.owner != owner || row.device != device || row.cancel_requested {
            return Err(denied());
        }
        let fresh = self.prepare_remote_execution_in_transaction(
            owner,
            device,
            &row.workspace,
            &row.project,
            row.request.clone(),
            controller_node,
        )?;
        if row.intent_digest != fresh.intent_digest
            || row.authority != fresh.candidate.authority
            || row.binding != serde_json::to_string(&fresh.candidate.binding)?
            || row.command.definition != fresh.definition
        {
            return Err(execution_conflict());
        }
        let (definition,drain):(String,bool)=self.connection.query_row("SELECT definition,drain FROM team_hosts WHERE id=?1 AND workspace_id=?2 AND status='approved'",
            params![row.command.descriptor.scope.host_id,row.workspace],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let definition: dunlin_domain::hosts::HostDefinition = serde_json::from_str(&definition)?;
        protocol(definition.validate())?;
        if drain
            || !definition
                .capability_policy
                .harnesses
                .contains(&row.command.definition.harness)
        {
            return Err(denied());
        }
        Ok(row.phase)
    }
    pub(crate) fn remote_execution_snapshot<F>(
        &self,
        attempt: &str,
        authorize_producer: F,
    ) -> Result<RemoteExecutionSnapshot>
    where
        F: FnOnce(&Store, &ExecutionCommand) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            authorize_producer(store, &row.command)?;
            Ok(RemoteExecutionSnapshot {
                command: row.command,
                record: row.record,
            })
        })
    }
    /// Negative cleanup enumeration only. It does not dequeue or reconstruct a
    /// positive command, and a caller cannot silently skip rejected scopes.
    pub(crate) fn remote_execution_cleanup<F>(
        &self,
        limit: usize,
        after: Option<&str>,
        mut authorize_producer: F,
    ) -> Result<Vec<RemoteExecutionSnapshot>>
    where
        F: FnMut(&Store, &ExecutionCommand) -> Result<()>,
    {
        if !(1..=16).contains(&limit) || after.is_some_and(|id| !identifier(id)) {
            return Err(execution_conflict());
        }
        self.remote_binding_transaction(false,|store| {
            let mut query=store.connection.prepare("SELECT e.attempt_id FROM controller_remote_executions e JOIN controller_remote_execution_outcomes o ON o.attempt_id=e.attempt_id WHERE o.ack_pending=1 AND (?1 IS NULL OR e.attempt_id>?1) ORDER BY e.attempt_id LIMIT ?2")?;
            let ids=query.query_map(params![after,i64::try_from(limit).map_err(|_|execution_conflict())?],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(query);
            ids.into_iter().map(|id|{
                let row=store.remote_execution_row(&id)?.ok_or_else(execution_conflict)?;
                authorize_producer(store,&row.command)?;
                Ok(RemoteExecutionSnapshot{command:row.command,record:row.record})
            }).collect()
        })
    }
    pub(crate) fn mark_remote_execution_uncertain<F>(
        &self,
        attempt: &str,
        authorize_producer: F,
    ) -> Result<()>
    where
        F: FnOnce(&Store, &ExecutionCommand) -> Result<()>,
    {
        self.remote_binding_transaction(false,|store| {
            let row=store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            authorize_producer(store,&row.command)?;
            if row.phase!=RemoteExecutionPhase::Terminal {
                store.connection.execute("UPDATE controller_remote_executions SET phase='unknown',cancel_requested=1,last_renew=NULL WHERE attempt_id=?1",[attempt])?;
            }
            Ok(())
        })
    }
    /// Internal, read-only reconciliation for a daemon-owned stored identity.
    /// This never creates a positive outbox command or renews an old fence.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_remote_execution_from_guard<F>(
        &self,
        attempt: &str,
        channel: ChannelFence,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        now: i64,
        authorize_producer: F,
    ) -> Result<ExecutionControllerMessage>
    where
        F: FnOnce(
            &Store,
            &ExecutionCommand,
            &WorkerIdentity,
            &ChannelLease,
            &ChannelFence,
        ) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            clock(now)?;
            protocol(channel.validate())?;
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            if identity.host_id() != row.command.descriptor.scope.host_id
                || identity.workspace_id() != row.workspace
                || identity.host().definition.node_id != row.command.descriptor.scope.worker_node_id
                || identity.host().enrollment_epoch != row.command.descriptor.scope.enrollment_epoch
                || identity.host().credential_revision
                    != row.command.descriptor.scope.credential_revision
                || !channel_matches(lease, &channel)
            {
                return Err(denied());
            }
            store.validate_worker_channel_in_transaction(
                identity,
                lease,
                &channel.controller_boot_id,
                now,
            )?;
            authorize_producer(store, &row.command, identity, lease, &channel)?;
            Ok(ExecutionControllerMessage::Reconcile {
                channel,
                identity: row.command.identity,
            })
        })
    }
    /// Clear only after a mandatory trusted transport callback attests an exact
    /// post-SQL worker OutcomeAcknowledged receipt. Socket write is insufficient.
    pub(crate) fn confirm_remote_execution_ack<F>(
        &self,
        attempt: &str,
        message: &ExecutionControllerMessage,
        authorize_receipt: F,
    ) -> Result<bool>
    where
        F: FnOnce(&Store, &ExecutionCommand, &ExecutionControllerMessage) -> Result<()>,
    {
        self.remote_binding_transaction(false,|store| {
            protocol(message.validate())?;
            let ExecutionControllerMessage::OutcomeAck{identity,fence,outcome_id,outcome_digest,..}=message else{return Err(execution_conflict())};
            let row=store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            let record=row.record.as_ref().ok_or_else(execution_conflict)?;
            let outcome=record.outcome.as_ref().ok_or_else(execution_conflict)?;
            if identity!=&row.command.identity||fence!=&record.receipt.fence||outcome_id!=&outcome.outcome_id||outcome_digest!=&outcome.digest{return Err(execution_conflict());}
            let inbox:Option<(String,String,String,bool)>=store.connection.query_row(
                "SELECT outcome_id,digest,record,ack_pending FROM controller_remote_execution_outcomes WHERE attempt_id=?1",[attempt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            let inbox=inbox.ok_or_else(execution_conflict)?;
            if inbox.0!=*outcome_id||inbox.1!=*outcome_digest||inbox.2!=serde_json::to_string(record)?{return Err(execution_conflict());}
            authorize_receipt(store,&row.command,message)?;
            if !inbox.3{return Ok(false);}
            store.connection.execute("UPDATE controller_remote_execution_outcomes SET ack_pending=0 WHERE attempt_id=?1",[attempt])?;Ok(true)
        })
    }
    /// Startup never replays uncertain positive work. Unknown remains reserved
    /// until an exact terminal worker outcome is reconciled or handled manually.
    pub(crate) fn recover_remote_executions(&self) -> Result<usize> {
        self.remote_binding_transaction(false,|store| {
            Ok(store.connection.execute("UPDATE controller_remote_executions SET phase='unknown',last_renew=NULL WHERE phase IN('pending','dispatched','accepted')",[])?)
        })
    }
    /// Negative receipt after durable terminal commit, on a current attested
    /// channel. The original fence remains intact; no lease is renewed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remote_execution_outcome_ack<F>(
        &self,
        attempt: &str,
        channel: ChannelFence,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        now: i64,
        authorize_producer: F,
    ) -> Result<ExecutionControllerMessage>
    where
        F: FnOnce(
            &Store,
            &ExecutionCommand,
            &WorkerIdentity,
            &ChannelLease,
            &ChannelFence,
        ) -> Result<()>,
    {
        self.remote_binding_transaction(false, |store| {
            clock(now)?;
            protocol(channel.validate())?;
            let row = store.remote_execution_row(attempt)?.ok_or_else(denied)?;
            if identity.host_id() != row.command.descriptor.scope.host_id
                || identity.workspace_id() != row.workspace || !channel_matches(lease, &channel)
            { return Err(denied()); }
            store.validate_worker_channel_in_transaction(identity, lease, &channel.controller_boot_id, now)?;
            authorize_producer(store, &row.command, identity, lease, &channel)?;
            let record = row.record.ok_or_else(execution_conflict)?;
            let outcome = record.outcome.as_ref().ok_or_else(execution_conflict)?;
            let inbox: Option<(String,String,String)> = store.connection.query_row(
                "SELECT outcome_id,digest,record FROM controller_remote_execution_outcomes WHERE attempt_id=?1",
                [attempt], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            let inbox = inbox.ok_or_else(execution_conflict)?;
            if inbox != (outcome.outcome_id.clone(), outcome.digest.clone(), serde_json::to_string(&record)?) {
                return Err(execution_conflict());
            }
            Ok(ExecutionControllerMessage::OutcomeAck {
                channel, identity: row.command.identity, fence: record.receipt.fence,
                outcome_id: outcome.outcome_id.clone(), outcome_digest: outcome.digest.clone(),
            })
        })
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
impl Store {
    pub(crate) fn remote_execution_lifecycle_sentinel_for_test(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        host: &str,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO controller_remote_executions(attempt_id,command_id,owner,device,workspace,project,host,intent_key,intent_digest,request,binding,authority,command,phase,created_at_ms) VALUES('lifecycle-sentinel','lifecycle-command',?1,?2,?3,?4,?5,'lifecycle-key-0001','sentinel','{}','{}','{}','{}','pending',0)",params![owner,device,workspace,project,host])?;
        Ok(())
    }
    pub(crate) fn connection_for_tests_remote_execution_phase(&self) -> String {
        self.connection.query_row("SELECT phase FROM controller_remote_executions WHERE attempt_id='lifecycle-sentinel'",[],|row|row.get(0)).unwrap()
    }
}
