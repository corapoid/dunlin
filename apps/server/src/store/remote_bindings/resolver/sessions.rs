//! Controller session admission and immutable pins. SQL never reconstructs a
//! live worker lease; caller and v4 transport callbacks are mandatory at cuts.
use super::*;
use crate::store::worker_channels::{ChannelLease, WorkerIdentity};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use dunlin_relay_protocol::worker::{ChannelFence, session as wire};
use dunlin_session_runtime::{AdmissionResult, Lease};
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Remote scoped broker producer integration is in progress"
    )
)]
pub(crate) mod authority;
mod mcp;
mod open;
mod runtime;
pub(crate) use mcp::RemoteMcpAdmission;
use mcp::{McpInput, McpPins, McpPreparation};
pub(crate) use open::RemoteOpen;
pub(crate) use runtime::RemoteStreamCut;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE controller_remote_sessions(
 session TEXT PRIMARY KEY REFERENCES interactive_sessions(id),
 owner TEXT NOT NULL REFERENCES users(id),device TEXT NOT NULL REFERENCES devices(id),
 workspace TEXT NOT NULL,project TEXT NOT NULL,host TEXT NOT NULL REFERENCES team_hosts(id),
 pins TEXT NOT NULL CHECK(length(CAST(pins AS BLOB))<=32768),
 open_command TEXT NOT NULL CHECK(length(CAST(open_command AS BLOB))<=65536),
 intent_id TEXT NOT NULL UNIQUE,
 journal_epoch INTEGER NOT NULL CHECK(journal_epoch BETWEEN 1 AND 9007199254740991),
 journal_instance TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN('pending','opening','active','interrupted','stopped')),
 receipt TEXT CHECK(receipt IS NULL OR length(CAST(receipt AS BLOB))<=8192),
 reservation INTEGER NOT NULL CHECK(reservation IN(0,1)),
 last_sequence INTEGER NOT NULL DEFAULT 0 CHECK(last_sequence BETWEEN 0 AND 9007199254740991),last_digest TEXT,
 ack_sequence INTEGER NOT NULL DEFAULT 0 CHECK(ack_sequence BETWEEN 0 AND last_sequence),ack_digest TEXT,
 confirmed_ack INTEGER NOT NULL DEFAULT 0 CHECK(confirmed_ack BETWEEN 0 AND ack_sequence),
 renew_sequence INTEGER NOT NULL DEFAULT 0 CHECK(renew_sequence BETWEEN 0 AND 9007199254740991),
 closing INTEGER NOT NULL DEFAULT 0 CHECK(closing IN(0,1)),
 CHECK((last_sequence=0 AND last_digest IS NULL) OR (last_sequence>0 AND length(last_digest)=64)),
 CHECK((ack_sequence=0 AND ack_digest IS NULL) OR (ack_sequence>0 AND length(ack_digest)=64)),
 CHECK((phase='stopped' AND reservation=0) OR (phase!='stopped' AND reservation=1)),
 FOREIGN KEY(workspace,project) REFERENCES team_projects(workspace_id,id));
CREATE TABLE controller_remote_session_turns(
 session TEXT NOT NULL REFERENCES controller_remote_sessions(session),
 turn TEXT NOT NULL REFERENCES interactive_turns(id),command_id TEXT NOT NULL UNIQUE,
 turn_index INTEGER NOT NULL CHECK(turn_index BETWEEN 1 AND 128),
 command TEXT NOT NULL CHECK(length(CAST(command AS BLOB))<=65536),
 receipt TEXT CHECK(receipt IS NULL OR length(CAST(receipt AS BLOB))<=8192),
 phase TEXT NOT NULL CHECK(phase IN('pending','sent','accepted','terminal','unknown')),
 terminal_sequence INTEGER,renew_sequence INTEGER NOT NULL DEFAULT 0 CHECK(renew_sequence BETWEEN 0 AND 9007199254740991),
 PRIMARY KEY(session,turn),UNIQUE(session,turn_index));
CREATE TABLE controller_remote_session_stream(
 session TEXT NOT NULL REFERENCES controller_remote_sessions(session),
 sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),
 digest TEXT NOT NULL CHECK(length(digest)=64),
 kind TEXT NOT NULL CHECK(kind IN('text','turn_completed','session_stopped')),
 event TEXT CHECK(event IS NULL OR length(CAST(event AS BLOB))<=65536),
 PRIMARY KEY(session,sequence));
"#;
/// Additive upgrade from the already tested schema36. Interrupted historical
/// rows conservatively crossed the send cut; only pending/no-receipt is proven
/// never dispatched. New admissions retain the default zero marker.
pub(crate) const LIFECYCLE_SCHEMA: &str = r#"
ALTER TABLE controller_remote_sessions ADD COLUMN dispatched INTEGER NOT NULL DEFAULT 0 CHECK(dispatched IN(0,1));
ALTER TABLE controller_remote_sessions ADD COLUMN stop_record TEXT CHECK(stop_record IS NULL OR length(CAST(stop_record AS BLOB))<=8192);
UPDATE controller_remote_sessions SET dispatched=1 WHERE phase!='pending' OR receipt IS NOT NULL;
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteSessionRequest {
    pub binding: BindingRef,
    pub profile: AgentProfileRef,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    owner: String,
    device: String,
    workspace: String,
    project: String,
    controller_node_id: String,
    request: RemoteSessionRequest,
    binding: RemoteBinding,
    authority: AuthorityStamp,
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp: Option<McpPins>,
}
pub(crate) struct PreparedRemoteSession {
    pins: Pins,
    candidate: RemoteTargetCandidate,
    definition: AgentDefinition,
    budgets: wire::SessionBudgets,
    mcp: Option<McpPreparation>,
}
impl PreparedRemoteSession {
    pub fn export_id(&self) -> &str {
        self.candidate.export_id()
    }
    pub fn export_revision(&self) -> u64 {
        self.candidate.export_revision()
    }
    pub fn principal(&self) -> ExportPrincipal {
        ExportPrincipal {
            kind: PrincipalKind::Human,
            id: self.pins.owner.clone(),
        }
    }
}
pub(crate) struct RemoteSessionAdmission {
    pub admission: AdmissionResult,
    /// Only a fresh admission can yield this intent; replay never yields a
    /// positive dispatch handle, even when the original worker is offline.
    pub command: Option<RemoteOpen>,
}
pub(crate) struct RemoteSessionSnapshot {
    pub session: String,
    pub owner: String,
    pub device: String,
    pub workspace: String,
    pub project: String,
    pub lease: Lease,
    pub open: RemoteOpen,
    pub mcp_binding: Option<String>,
    pub receipt: Option<wire::SessionReceipt>,
    pub phase: String,
    pub dispatched: bool,
    pub closing: bool,
    pub sequence: u64,
    pub digest: Option<String>,
    pub ack_sequence: u64,
    pub ack_digest: Option<String>,
    pub confirmed_ack: u64,
}
pub(crate) struct RemoteSessionActivity {
    pub queued: bool,
    pub active_turn: Option<String>,
    pub cancellation: bool,
}
fn session_conflict() -> StoreError {
    StoreError::Conflict("Remote session identity, authority or state changed".into())
}
fn valid<T, E>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|_| session_conflict())
}
fn integer(value: u64) -> Result<i64> {
    if value > dunlin_relay_protocol::worker::MAX_EXECUTION_INTEGER {
        return Err(session_conflict());
    }
    i64::try_from(value).map_err(|_| session_conflict())
}
fn journal_error(error: dunlin_session_runtime::Error) -> StoreError {
    match error {
        dunlin_session_runtime::Error::Policy(error) => error.into(),
        dunlin_session_runtime::Error::Limit(message) => StoreError::Limit(message.into()),
        _ => session_conflict(),
    }
}
fn session_budgets(definition: &AgentDefinition) -> Result<wire::SessionBudgets> {
    let budgets = wire::SessionBudgets {
        max_seconds: definition.max_seconds,
        max_output_bytes: definition.max_output_bytes,
        idle_timeout_ms: (definition.max_seconds * 1000).min(30_000),
        lease_duration_ms: (definition.max_seconds * 1000).min(10_000),
        max_turns: wire::MAX_SESSION_TURNS,
        retained_bytes: wire::MAX_SESSION_RETAINED_BYTES,
    };
    valid(budgets.validate())?;
    Ok(budgets)
}
fn session_fingerprint(pins: &Pins, budgets: &wire::SessionBudgets) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(if pins.mcp.is_some() {
        b"dunlin.controller.remote_session.intent.v5\0"
    } else {
        b"dunlin.controller.remote_session.intent.v4\0"
    });
    hash.update(serde_json::to_vec(&(
        &pins.owner,
        &pins.device,
        &pins.workspace,
        &pins.project,
        &pins.request,
        &pins.binding,
        &pins.authority,
        budgets,
    ))?);
    if let Some(mcp) = &pins.mcp {
        hash.update(serde_json::to_vec(mcp)?);
    }
    Ok(hex::encode(hash.finalize()))
}
impl Store {
    fn session_transaction_required(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(session_conflict());
        }
        Ok(())
    }
    pub(crate) fn prepare_remote_session_in_transaction(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        request: RemoteSessionRequest,
        controller_node_id: &str,
    ) -> Result<PreparedRemoteSession> {
        self.prepare_remote_session_mode(
            owner,
            device,
            workspace,
            project,
            request,
            controller_node_id,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_remote_mcp_session_in_transaction(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        request: RemoteSessionRequest,
        controller_node_id: &str,
        mcp: RemoteMcpAdmission<'_>,
    ) -> Result<PreparedRemoteSession> {
        self.prepare_remote_session_mode(
            owner,
            device,
            workspace,
            project,
            request,
            controller_node_id,
            Some(McpInput::Fresh(mcp)),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_remote_session_mode(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        request: RemoteSessionRequest,
        controller_node_id: &str,
        mcp: Option<McpInput<'_>>,
    ) -> Result<PreparedRemoteSession> {
        self.session_transaction_required()?;
        valid(request.binding.validate())?;
        if !self
            .active_device_user(device)?
            .is_some_and(|user| user.id == owner)
        {
            return Err(denied());
        }
        let context = self.team_project_context(workspace, project, owner)?;
        context.authorize_project(workspace, project, Permission::ViewProject)?;
        let authority = context.authorize_project(workspace, project, Permission::CreateSession)?;
        let candidate = self.capture_remote_target_in_transaction(
            owner,
            workspace,
            project,
            &request.binding,
            controller_node_id,
        )?;
        if candidate.authority != authority {
            return Err(session_conflict());
        }
        let profile = self
            .agent_version(
                owner,
                workspace,
                project,
                &request.profile.agent_id,
                request.profile.version,
            )?
            .filter(|profile| profile.digest == request.profile.digest)
            .ok_or_else(denied)?;
        valid(profile.definition.validate())?;
        if !profile.definition.mcp.is_empty() && mcp.is_none() {
            return Err(StoreError::Limit(
                "MCP execution requires a scoped runtime broker".into(),
            ));
        }
        let mcp = mcp
            .map(|input| input.prepare(self, owner, workspace, &profile.definition, &candidate))
            .transpose()?;
        if crate::worker_execution::profile_digest(&profile.definition)? != request.profile.digest {
            return Err(session_conflict());
        }
        let budgets = session_budgets(&profile.definition)?;
        let mut pins = Pins {
            owner: owner.into(),
            device: device.into(),
            workspace: workspace.into(),
            project: project.into(),
            controller_node_id: controller_node_id.into(),
            request,
            binding: candidate.binding.clone(),
            authority,
            fingerprint: String::new(),
            mcp: mcp.as_ref().map(McpPreparation::pins),
        };
        pins.fingerprint = session_fingerprint(&pins, &budgets)?;
        Ok(PreparedRemoteSession {
            pins,
            candidate,
            definition: profile.definition,
            budgets,
            mcp,
        })
    }
    fn revalidate_session_preparation(
        &self,
        prepared: &PreparedRemoteSession,
    ) -> Result<PreparedRemoteSession> {
        let pins = &prepared.pins;
        let fresh = self.prepare_remote_session_mode(
            &pins.owner,
            &pins.device,
            &pins.workspace,
            &pins.project,
            pins.request.clone(),
            &pins.controller_node_id,
            prepared.mcp.as_ref().map(McpInput::Pinned),
        )?;
        if fresh.pins.fingerprint != pins.fingerprint
            || fresh.pins.binding != pins.binding
            || fresh.definition != prepared.definition
            || fresh.budgets != prepared.budgets
        {
            return Err(session_conflict());
        }
        Ok(fresh)
    }
    /// Historical receipt only: no transport, lease capture or dispatch. The
    /// saved broker limits participate in intent matching, never in authority.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lookup_remote_session_request_in_transaction(
        &self,
        owner: &str,
        device: &str,
        workspace: &str,
        project: &str,
        request: &RemoteSessionRequest,
        controller: &str,
        config: &crate::config::Config,
        selections: &[crate::interactive::mcp::Selection],
        key: &str,
        caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<Option<dunlin_session_runtime::Session>> {
        self.session_transaction_required()?;
        valid(request.binding.validate())?;
        caller(self, owner, device)?;
        if !self
            .active_device_user(device)?
            .is_some_and(|user| user.id == owner)
        {
            return Err(denied());
        }
        let context = self.team_project_context(workspace, project, owner)?;
        self.interactive_journal()
            .admission_lookup_checked(&context, device, key, |id, fingerprint| {
                let check = || -> Result<()> {
                    let mut pins = self.session_pins(id)?;
                    if pins.owner != owner
                        || pins.device != device
                        || pins.workspace != workspace
                        || pins.project != project
                        || pins.request != *request
                        || pins.controller_node_id != controller
                        || pins.fingerprint != fingerprint
                    {
                        return Err(session_conflict());
                    }
                    let candidate = self.capture_remote_target_in_transaction(
                        owner,
                        workspace,
                        project,
                        &request.binding,
                        controller,
                    )?;
                    let authority =
                        context.authorize_project(workspace, project, Permission::CreateSession)?;
                    if candidate.authority != authority {
                        return Err(session_conflict());
                    }
                    let profile = self
                        .agent_version(
                            owner,
                            workspace,
                            project,
                            &request.profile.agent_id,
                            request.profile.version,
                        )?
                        .filter(|profile| profile.digest == request.profile.digest)
                        .ok_or_else(denied)?;
                    valid(profile.definition.validate())?;
                    if crate::worker_execution::profile_digest(&profile.definition)?
                        != request.profile.digest
                    {
                        return Err(session_conflict());
                    }
                    match &mut pins.mcp {
                        Some(mcp) if !profile.definition.mcp.is_empty() => {
                            valid(mcp.limits.validate())?;
                            mcp.snapshot = crate::interactive::mcp::BoundSession::capture(
                                config,
                                self,
                                owner,
                                workspace,
                                &profile.definition,
                                selections,
                            )
                            .map_err(|_| denied())?
                            .encode()
                            .map_err(|_| denied())?;
                        }
                        None if selections.is_empty() && profile.definition.mcp.is_empty() => {}
                        _ => return Err(session_conflict()),
                    }
                    pins.binding = candidate.binding;
                    pins.authority = authority;
                    if session_fingerprint(&pins, &session_budgets(&profile.definition)?)?
                        != fingerprint
                    {
                        return Err(session_conflict());
                    }
                    Ok(())
                };
                check().map_err(|_| {
                    dunlin_session_runtime::Error::Conflict(
                        "Remote admission identity or authority changed",
                    )
                })
            })
            .map_err(journal_error)
    }
    #[cfg(test)]
    pub(crate) fn lookup_remote_session_admission_in_transaction(
        &self,
        prepared: &PreparedRemoteSession,
        key: &str,
        authorize_caller: impl FnOnce(&Store, &str, &str) -> Result<()>,
    ) -> Result<Option<dunlin_session_runtime::Session>> {
        let fresh = self.revalidate_session_preparation(prepared)?;
        let pins = fresh.pins;
        authorize_caller(self, &pins.owner, &pins.device)?;
        let context = self.team_project_context(&pins.workspace, &pins.project, &pins.owner)?;
        let receipt = self
            .interactive_journal()
            .admission_lookup(&context, &pins.device, key, &pins.fingerprint)
            .map_err(journal_error)?;
        if let Some(session) = &receipt {
            let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM controller_remote_sessions WHERE session=?1 AND owner=?2 AND device=?3)", params![session.id,pins.owner,pins.device], |row|row.get(0))?;
            if !exists {
                return Err(session_conflict());
            }
        }
        Ok(receipt)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_remote_session_in_transaction<F>(
        &self,
        prepared: PreparedRemoteSession,
        proof: ObservedExport,
        channel: ChannelFence,
        key: &str,
        instance: &str,
        now: i64,
        authorize_v4: F,
    ) -> Result<RemoteSessionAdmission>
    where
        F: FnOnce(&Store, &str, &str, &WorkerIdentity, &ChannelLease, &ChannelFence) -> Result<()>,
    {
        let fresh = self.revalidate_session_preparation(&prepared)?;
        let pins = &prepared.pins;
        valid(channel.validate())?;
        if now < 0
            || proof.lease().channel_id != channel.channel_id
            || proof.lease().controller_boot_id != channel.controller_boot_id
            || proof.lease().worker_boot_id != channel.worker_boot_id
        {
            return Err(session_conflict());
        }
        authorize_v4(
            self,
            &pins.owner,
            &pins.device,
            proof.identity(),
            proof.lease(),
            &channel,
        )?;
        if let Some(mcp) = &fresh.mcp {
            mcp.transport
                .attest_in_transaction(self, proof.identity(), proof.lease(), &channel)?;
        }
        let descriptor = proof.descriptor().clone();
        if !descriptor
            .grant
            .permissions
            .contains(&ExportPermission::Execute)
        {
            return Err(denied());
        }
        let (raw,drain):(String,bool)=self.connection.query_row("SELECT definition,drain FROM team_hosts WHERE id=?1 AND workspace_id=?2 AND status='approved' AND enrollment_epoch=?3 AND credential_revision=?4",params![pins.binding.host_id,pins.workspace,integer(pins.binding.host_enrollment_epoch)?,integer(pins.binding.credential_revision)?],|row|Ok((row.get(0)?,row.get(1)?)))?;
        let host: dunlin_domain::hosts::HostDefinition = serde_json::from_str(&raw)?;
        valid(host.validate())?;
        if drain
            || !host
                .capability_policy
                .harnesses
                .contains(&prepared.definition.harness)
        {
            return Err(denied());
        }
        self.finish_remote_target_in_transaction(
            &pins.owner,
            &pins.workspace,
            &pins.project,
            fresh.candidate,
            proof,
            &channel.controller_boot_id,
            &pins.controller_node_id,
            now,
        )?;
        let context = self.team_project_context(&pins.workspace, &pins.project, &pins.owner)?;
        if let Some(session) = self
            .interactive_journal()
            .admission_lookup(&context, &pins.device, key, &pins.fingerprint)
            .map_err(journal_error)?
        {
            self.remote_session_snapshot(&session.id)?;
            return Ok(RemoteSessionAdmission {
                admission: AdmissionResult {
                    session,
                    lease: None,
                },
                command: None,
            });
        }
        let (total,own,live):(i64,i64,i64)=self.connection.query_row("SELECT count(*),coalesce(sum(owner=?1),0),coalesce(sum(host=?2 AND reservation=1),0) FROM controller_remote_sessions",params![pins.owner,pins.binding.host_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        let legacy: i64 = self.connection.query_row(
            "SELECT count(*) FROM controller_remote_executions WHERE host=?1 AND phase!='terminal'",
            [&pins.binding.host_id],
            |row| row.get(0),
        )?;
        if total >= 1024
            || own >= 128
            || live + legacy >= i64::from(host.capability_policy.max_parallel)
        {
            return Err(StoreError::Limit(
                "Remote session reservation capacity reached".into(),
            ));
        }
        let admission = self
            .interactive_journal()
            .create_admitted_profile_in_transaction(
                &context,
                &pins.workspace,
                &pins.project,
                &pins.device,
                key,
                &pins.fingerprint,
                instance,
                Some(&pins.request.profile),
            )
            .map_err(journal_error)?;
        let lease = admission.lease.as_ref().ok_or_else(session_conflict)?;
        let base = wire::SessionOpenCommand {
            channel,
            identity: wire::SessionIdentity {
                intent_id: format!("open-{}", uuid::Uuid::now_v7()),
                fingerprint: "0".repeat(64),
            },
            controller_session_id: admission.session.id.clone(),
            descriptor,
            profile: pins.request.profile.clone(),
            definition: prepared.definition,
            budgets: prepared.budgets,
        };
        let mut command = match &pins.mcp {
            Some(mcp) => RemoteOpen::Broker(Box::new(wire::broker::OpenBrokerSessionCommand {
                protocol_version: wire::broker::WORKER_BROKER_PROTOCOL_VERSION,
                open: base,
                broker_limits: mcp.limits.clone(),
            })),
            None => RemoteOpen::Legacy(Box::new(base)),
        };
        command.set_fingerprint()?;
        command.validate()?;
        match &command {
            RemoteOpen::Legacy(open) => valid(
                wire::SessionControllerMessage::OpenSession {
                    command: open.clone(),
                }
                .validate(),
            )?,
            RemoteOpen::Broker(open) => {
                let transport = &prepared
                    .mcp
                    .as_ref()
                    .ok_or_else(session_conflict)?
                    .transport;
                valid(
                    wire::broker::BrokerSessionControllerMessage::OpenBrokerSession {
                        command: open.clone(),
                    }
                    .validate_for_channel(
                        transport.fence(),
                        transport.broker_offer().ok_or_else(session_conflict)?,
                    ),
                )?;
            }
        }
        self.connection.execute("INSERT INTO controller_remote_sessions(session,owner,device,workspace,project,host,pins,open_command,intent_id,journal_epoch,journal_instance,phase,reservation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'pending',1)",params![admission.session.id,pins.owner,pins.device,pins.workspace,pins.project,pins.binding.host_id,serde_json::to_string(pins)?,serde_json::to_string(&command)?,command.identity.intent_id,integer(lease.epoch)?,lease.instance])?;
        Ok(RemoteSessionAdmission {
            admission,
            command: Some(command),
        })
    }
    pub(crate) fn remote_session_snapshot(&self, id: &str) -> Result<RemoteSessionSnapshot> {
        type StoredSessionRow = (
            String,
            String,
            i64,
            String,
            String,
            Option<String>,
            bool,
            i64,
            Option<String>,
            i64,
            Option<String>,
            i64,
            bool,
        );
        let (raw,open,epoch,instance,phase,receipt,closing,sequence,digest,ack_sequence,ack_digest,confirmed_ack,dispatched):StoredSessionRow=self.connection.query_row("SELECT pins,open_command,journal_epoch,journal_instance,phase,receipt,closing,last_sequence,last_digest,ack_sequence,ack_digest,confirmed_ack,dispatched FROM controller_remote_sessions WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?)))?;
        if raw.len() > 32768 || open.len() > 65536 {
            return Err(session_conflict());
        }
        let pins: Pins = serde_json::from_str(&raw)?;
        let open: RemoteOpen = serde_json::from_str(&open)?;
        open.validate()?;
        if open.controller_session_id != id
            || open.fingerprint()? != open.identity.fingerprint
            || open.descriptor.grant.principal.id != pins.owner
            || open.profile != pins.request.profile
            || open.limits() != pins.mcp.as_ref().map(|mcp| &mcp.limits)
        {
            return Err(session_conflict());
        }
        if let Some(mcp) = &pins.mcp {
            crate::interactive::mcp::BoundSession::validate_snapshot(
                &pins.owner,
                &pins.workspace,
                &open.definition,
                &mcp.snapshot,
            )
            .map_err(|_| session_conflict())?;
        }
        let receipt: Option<wire::SessionReceipt> =
            receipt.map(|raw| serde_json::from_str(&raw)).transpose()?;
        if let Some(receipt) = &receipt {
            open.validate_receipt(receipt)?;
        }
        let number = |value: i64| {
            u64::try_from(value)
                .ok()
                .filter(|value| *value <= dunlin_relay_protocol::worker::MAX_EXECUTION_INTEGER)
                .ok_or_else(session_conflict)
        };
        Ok(RemoteSessionSnapshot {
            session: id.into(),
            owner: pins.owner,
            device: pins.device,
            workspace: pins.workspace,
            project: pins.project,
            lease: Lease {
                epoch: number(epoch)?,
                instance,
            },
            open,
            mcp_binding: pins.mcp.map(|mcp| mcp.snapshot),
            receipt,
            phase,
            dispatched,
            closing,
            sequence: number(sequence)?,
            digest,
            ack_sequence: number(ack_sequence)?,
            ack_digest,
            confirmed_ack: number(confirmed_ack)?,
        })
    }
}
#[cfg(test)]
mod tests;
