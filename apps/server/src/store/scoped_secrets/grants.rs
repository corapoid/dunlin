//! Credential-use policy is separate from provisioning, MCP profile intent and
//! per-call consent. These rows grant no runtime/transport authority themselves.
use super::*;
use dunlin_domain::{
    agent::AgentProfileRef,
    collaboration::{AccessContext, PrincipalKind},
};
use dunlin_vault::scoped::tool;
use serde::Deserialize;
use std::collections::BTreeSet;

pub(crate) const SCHEMA:&str="
CREATE TABLE scoped_secret_grants (
 id TEXT PRIMARY KEY,
 workspace TEXT NOT NULL,project TEXT NOT NULL,host TEXT NOT NULL,credential TEXT NOT NULL,
 principal_kind TEXT NOT NULL CHECK(principal_kind IN ('human','service')),
 principal_id TEXT NOT NULL,
 agent_id TEXT NOT NULL,agent_version INTEGER NOT NULL CHECK(agent_version>0),
 profile_digest TEXT NOT NULL CHECK(length(profile_digest)=64),provider_id TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 active INTEGER NOT NULL CHECK(active IN (0,1)),
 credential_revision INTEGER NOT NULL CHECK(credential_revision>0),
 membership_revision INTEGER NOT NULL CHECK(membership_revision>0),
 project_revision INTEGER NOT NULL CHECK(project_revision>0),host_revision INTEGER NOT NULL CHECK(host_revision>0),
 read_only INTEGER NOT NULL CHECK(read_only IN (0,1)),
 updated_by TEXT NOT NULL REFERENCES users(id),updated_at INTEGER NOT NULL,
 last_revoke TEXT CHECK(last_revoke IS NULL OR length(last_revoke)<=4096),
 FOREIGN KEY(workspace,project,host,credential) REFERENCES scoped_credentials(workspace,project,host,credential),
 FOREIGN KEY(agent_id,agent_version) REFERENCES team_agent_versions(agent_id,version),
 UNIQUE(workspace,project,host,credential,principal_kind,principal_id,agent_id,agent_version,profile_digest,provider_id)
);
CREATE INDEX scoped_grants_recipient ON scoped_secret_grants(workspace,project,host,principal_kind,principal_id,profile_digest);
";
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Principal {
    Human,
    Service,
}
impl Principal {
    fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Service => "service",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrantRequest {
    pub expected_revision: u64,
    pub credential_revision: u64,
    pub principal_kind: Principal,
    pub principal_id: String,
    pub profile: AgentProfileRef,
    pub provider_id: String,
    pub read_only: bool,
}
#[derive(Clone, Serialize)]
pub(crate) struct Grant {
    pub id: String,
    pub credential: String,
    pub principal_kind: Principal,
    pub principal_id: String,
    pub profile: AgentProfileRef,
    pub provider_id: String,
    pub revision: u64,
    pub active: bool,
    pub credential_revision: u64,
    pub read_only: bool,
    #[serde(skip)]
    membership_revision: u64,
    #[serde(skip)]
    project_revision: u64,
    #[serde(skip)]
    host_revision: u64,
}
const COLUMNS: &str = "id,credential,principal_kind,principal_id,agent_id,agent_version,profile_digest,provider_id,revision,active,credential_revision,read_only,membership_revision,project_revision,host_revision";
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Grant> {
    let kind: String = r.get(2)?;
    Ok(Grant {
        id: r.get(0)?,
        credential: r.get(1)?,
        principal_kind: match kind.as_str() {
            "human" => Principal::Human,
            "service" => Principal::Service,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        principal_id: r.get(3)?,
        profile: AgentProfileRef {
            agent_id: r.get(4)?,
            version: super::super::positive_revision(r, 5)?,
            digest: r.get(6)?,
        },
        provider_id: r.get(7)?,
        revision: super::super::positive_revision(r, 8)?,
        active: r.get(9)?,
        credential_revision: super::super::positive_revision(r, 10)?,
        read_only: r.get(11)?,
        membership_revision: super::super::positive_revision(r, 12)?,
        project_revision: super::super::positive_revision(r, 13)?,
        host_revision: super::super::positive_revision(r, 14)?,
    })
}
impl Store {
    fn secret_recipient_stamp(
        &self,
        binding: &Binding,
        kind: Principal,
        id: &str,
    ) -> Result<AuthorityStamp> {
        if !token(id) {
            return Err(Error::Invalid);
        }
        match kind {
            Principal::Human => {
                let context = self
                    .team_project_context(&binding.workspace, &binding.project, id)
                    .map_err(error)?;
                context
                    .authorize_project(
                        &binding.workspace,
                        &binding.project,
                        Permission::ViewProject,
                    )
                    .map_err(|_| Error::Denied)?;
                context
                    .authorize_project(
                        &binding.workspace,
                        &binding.project,
                        Permission::CreateSession,
                    )
                    .map_err(|_| Error::Denied)
            }
            Principal::Service => {
                let raw:Option<(i64,String)>=self.connection.query_row("SELECT revision,permissions FROM team_service_principals WHERE id=?1 AND workspace_id=?2 AND project_id=?3 AND revoked=0 AND expires_at>?4",params![id,binding.workspace,binding.project,super::super::now_secs()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
                let (revision, permissions) = raw.ok_or(Error::Denied)?;
                let permissions: BTreeSet<Permission> =
                    serde_json::from_str(&permissions).map_err(|_| Error::Storage)?;
                if revision <= 0
                    || !permissions.contains(&Permission::ViewProject)
                    || !permissions.contains(&Permission::RunWorkflow)
                {
                    return Err(Error::Denied);
                }
                Ok(AuthorityStamp {
                    workspace_id: binding.workspace.clone(),
                    project_id: binding.project.clone(),
                    user_id: id.into(),
                    membership_revision: revision as u64,
                    grant_revision: revision as u64,
                })
            }
        }
    }
    fn secret_profile(
        &self,
        binding: &Binding,
        profile: &AgentProfileRef,
        provider: &str,
        read_only: bool,
    ) -> Result<()> {
        if !token(&profile.agent_id)
            || profile.version == 0
            || profile.version > 9_007_199_254_740_991
            || !hex(&profile.digest)
            || !tool::is_provider(provider)
        {
            return Err(Error::Invalid);
        }
        let published = self
            .load_agent_version(
                &binding.workspace,
                &binding.project,
                &profile.agent_id,
                profile.version,
            )
            .map_err(error)?
            .ok_or(Error::Denied)?;
        let computed = crate::worker_execution::profile_digest(&published.definition)
            .map_err(|_| Error::Storage)?;
        let pins: Vec<_> = published
            .definition
            .mcp
            .iter()
            .filter(|pin| pin.provider_id == provider)
            .collect();
        if published.digest != profile.digest
            || computed != profile.digest
            || pins.is_empty()
            || pins.len() > 2
            || pins
                .iter()
                .map(|pin| &pin.name)
                .collect::<BTreeSet<_>>()
                .len()
                != pins.len()
            || pins.iter().any(|pin| {
                let tool = match pin.name.as_str() {
                    "http" => tool::http(),
                    "recipe" if !read_only => tool::recipe(),
                    _ => return true,
                };
                tool.definition_digest()
                    .map_or(true, |digest| digest != pin.definition_digest)
            })
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    fn secret_active_version(&self, binding: &Binding, credential: &str) -> Result<Version> {
        self.connection.query_row("SELECT c.revision,r.reference,r.sha256 FROM scoped_credentials c JOIN scoped_secret_references r ON r.reference=c.active_reference AND r.workspace=c.workspace AND r.project=c.project AND r.host=c.host AND r.credential=c.credential AND r.revision=c.revision WHERE c.workspace=?1 AND c.project=?2 AND c.host=?3 AND c.credential=?4 AND c.state='active' AND r.write_ack=1 AND r.activated=1",params![binding.workspace,binding.project,binding.host,credential],|r|Ok(Version {credential:credential.into(),binding:binding.clone(),revision:super::super::positive_revision(r,0)?,reference:r.get(1)?,sha256:r.get(2)?})).optional().map_err(storage)?.ok_or(Error::Denied)
    }
    fn secret_grant_row(&self, binding: &Binding, id: &str) -> Result<Grant> {
        self.connection.query_row(&format!("SELECT {COLUMNS} FROM scoped_secret_grants WHERE workspace=?1 AND project=?2 AND host=?3 AND id=?4"),params![binding.workspace,binding.project,binding.host,id],row).optional().map_err(storage)?.ok_or(Error::Denied)
    }
    /// Resolve an explicitly selected private grant. No keyring or dispatch authority.
    pub(crate) fn scoped_secret_selection(
        &self,
        owner: &str,
        workspace: &str,
        id: &str,
        revision: u64,
    ) -> Result<(Grant, Version)> {
        let binding = self.connection.query_row(
            "SELECT workspace,project,host FROM scoped_secret_grants WHERE id=?1 AND workspace=?2 AND principal_kind='human' AND principal_id=?3",
            params![id, workspace, owner], |r| Ok(Binding {workspace:r.get(0)?,project:r.get(1)?,host:r.get(2)?}),
        ).optional().map_err(storage)?.ok_or(Error::Denied)?;
        let context = self
            .team_project_context(workspace, &binding.project, owner)
            .map_err(error)?;
        let profile = self.secret_grant_row(&binding, id)?.profile;
        self.resolve_secret_grant(&context, &binding, &profile, id, revision)
    }
    pub(crate) fn scoped_secret_set_grant(
        &self,
        proof: &Proof,
        credential: &str,
        request: &GrantRequest,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Grant> {
        if !token(credential) || request.expected_revision >= 9_007_199_254_740_991 {
            return Err(Error::Invalid);
        }
        let tx = self.scoped_secret_cut(proof, true, identity)?;
        let b = &proof.binding;
        self.secret_profile(b, &request.profile, &request.provider_id, request.read_only)?;
        let recipient =
            self.secret_recipient_stamp(b, request.principal_kind, &request.principal_id)?;
        let version = self.secret_active_version(b, credential)?;
        if version.revision != request.credential_revision {
            return Err(Error::Conflict);
        }
        let previous:Option<(String,u64)>=tx.query_row("SELECT id,revision FROM scoped_secret_grants WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4 AND principal_kind=?5 AND principal_id=?6 AND profile_digest=?7 AND provider_id=?8 AND agent_id=?9 AND agent_version=?10",params![b.workspace,b.project,b.host,credential,request.principal_kind.name(),request.principal_id,request.profile.digest,request.provider_id,request.profile.agent_id,request.profile.version as i64],|r|Ok((r.get(0)?,super::super::positive_revision(r,1)?))).optional().map_err(storage)?;
        if previous.as_ref().map_or(0, |p| p.1) != request.expected_revision {
            return Err(Error::Conflict);
        }
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM scoped_secret_grants WHERE workspace=?1",
                [&b.workspace],
                |r| r.get(0),
            )
            .map_err(storage)?;
        let own:i64=tx.query_row("SELECT count(*) FROM scoped_secret_grants WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4",params![b.workspace,b.project,b.host,credential],|r|r.get(0)).map_err(storage)?;
        if previous.is_none() && (count >= 4096 || own >= 256) {
            return Err(Error::Limit);
        }
        let id = previous.map_or_else(|| format!("sgt-{}", uuid::Uuid::now_v7()), |p| p.0);
        let revision = request.expected_revision + 1;
        tx.execute("INSERT INTO scoped_secret_grants(id,workspace,project,host,credential,principal_kind,principal_id,agent_id,agent_version,profile_digest,provider_id,revision,active,credential_revision,membership_revision,project_revision,host_revision,read_only,updated_by,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?14,?15,?16,?17,?18,?19) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,active=1,credential_revision=excluded.credential_revision,membership_revision=excluded.membership_revision,project_revision=excluded.project_revision,host_revision=excluded.host_revision,read_only=excluded.read_only,updated_by=excluded.updated_by,updated_at=excluded.updated_at",params![id,b.workspace,b.project,b.host,credential,request.principal_kind.name(),request.principal_id,request.profile.agent_id,request.profile.version as i64,request.profile.digest,request.provider_id,revision as i64,version.revision as i64,recipient.membership_revision as i64,recipient.grant_revision as i64,proof.host_revision as i64,request.read_only,proof.browser.user,super::super::now_secs()]).map_err(storage)?;
        self.scoped_secret_audit(proof, &id, "scoped_secret_grant_set", revision)?;
        let grant = self.secret_grant_row(b, &id)?;
        tx.commit().map_err(storage)?;
        Ok(grant)
    }
    pub(crate) fn scoped_secret_grants(
        &self,
        proof: &Proof,
        credential: &str,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Vec<Grant>> {
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let b = &proof.binding;
        let grants = {
            let mut stmt=tx.prepare(&format!("SELECT {COLUMNS} FROM scoped_secret_grants WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4 ORDER BY id LIMIT 256")).map_err(storage)?;
            stmt.query_map(params![b.workspace, b.project, b.host, credential], row)
                .map_err(storage)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage)?
        };
        tx.commit().map_err(storage)?;
        Ok(grants)
    }
    pub(crate) fn scoped_secret_revoke_grant(
        &self,
        proof: &Proof,
        credential: &str,
        id: &str,
        expected: u64,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Grant> {
        if expected == 0 || expected > 9_007_199_254_740_991 {
            return Err(Error::Invalid);
        }
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let grant = self.secret_grant_row(&proof.binding, id)?;
        if grant.credential != credential || grant.revision != expected {
            return Err(Error::Conflict);
        }
        let revision = (expected + 1).min(9_007_199_254_740_991);
        let receipt=serde_json::json!({"actor":proof.browser.user,"device":proof.browser.device,"membership_revision":proof.stamp.membership_revision,"project_revision":proof.stamp.grant_revision,"host_revision":proof.host_revision,"revision":revision,"terminal":revision==expected,"at":super::super::now_secs()}).to_string();
        tx.execute("UPDATE scoped_secret_grants SET revision=?5,active=0,updated_by=?1,updated_at=?2,last_revoke=?3 WHERE id=?4",params![proof.browser.user,super::super::now_secs(),receipt,id,revision as i64]).map_err(storage)?;
        match self.team_audit_capacity() {
            Ok(()) => {
                let _ =
                    self.scoped_secret_audit(proof, id, "scoped_secret_grant_revoked", revision);
            }
            Err(super::super::StoreError::Limit(_)) => {}
            Err(other) => return Err(error(other)),
        }
        let grant = self.secret_grant_row(&proof.binding, id)?;
        tx.commit().map_err(storage)?;
        Ok(grant)
    }
    /// Metadata resolution only. A host must additionally attest its live
    /// session/turn/lease and consume per-call approval before reading keyring.
    pub(crate) fn resolve_secret_grant(
        &self,
        context: &AccessContext,
        binding: &Binding,
        profile: &AgentProfileRef,
        id: &str,
        expected_revision: u64,
    ) -> Result<(Grant, Version)> {
        let grant = self.secret_grant_row(binding, id)?;
        let kind = match context.principal {
            PrincipalKind::Human => Principal::Human,
            PrincipalKind::Service => Principal::Service,
        };
        let permission = if kind == Principal::Human {
            Permission::CreateSession
        } else {
            Permission::RunWorkflow
        };
        let supplied = context
            .authorize_project(&binding.workspace, &binding.project, permission)
            .map_err(|_| Error::Denied)?;
        let current = self.secret_recipient_stamp(binding, kind, &context.membership.user_id)?;
        if grant.revision != expected_revision
            || !grant.active
            || grant.principal_kind != kind
            || grant.principal_id != context.membership.user_id
            || grant.profile != *profile
            || current != supplied
            || grant.membership_revision != current.membership_revision
            || grant.project_revision != current.grant_revision
        {
            return Err(Error::Denied);
        }
        self.secret_profile(binding, profile, &grant.provider_id, grant.read_only)?;
        let host = if binding.host == "local" {
            self.local_binding_candidate(
                &context.membership.user_id,
                &binding.workspace,
                &binding.project,
                Permission::CreateSession,
            )
            .map_err(error)?
            .is_some_and(|(current, _)| current.revision == grant.host_revision)
        } else {
            self.connection.query_row("SELECT EXISTS(SELECT 1 FROM team_hosts WHERE workspace_id=?1 AND id=?2 AND status='approved' AND revision=?3)",params![binding.workspace,binding.host,grant.host_revision as i64],|r|r.get(0)).map_err(storage)?
        };
        let version = self.secret_active_version(binding, &grant.credential)?;
        if !host || version.revision != grant.credential_revision {
            return Err(Error::Denied);
        }
        Ok((grant, version))
    }
    pub(crate) fn scoped_secret_available(
        &self,
        actor: &str,
        binding: &Binding,
        profile: &AgentProfileRef,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Vec<Grant>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        identity(self)?;
        let context = self
            .team_project_context(&binding.workspace, &binding.project, actor)
            .map_err(error)?;
        let actor_stamp = context
            .authorize_project(
                &binding.workspace,
                &binding.project,
                Permission::CreateSession,
            )
            .map_err(|_| Error::Denied)?;
        context
            .authorize_project(
                &binding.workspace,
                &binding.project,
                Permission::ViewProject,
            )
            .map_err(|_| Error::Denied)?;
        if !token(&profile.agent_id)
            || profile.version == 0
            || profile.version > 9_007_199_254_740_991
            || !hex(&profile.digest)
        {
            return Err(Error::Invalid);
        }
        let published = self
            .load_agent_version(
                &binding.workspace,
                &binding.project,
                &profile.agent_id,
                profile.version,
            )
            .map_err(error)?
            .ok_or(Error::Denied)?;
        if published.digest != profile.digest
            || crate::worker_execution::profile_digest(&published.definition)
                .map_err(|_| Error::Storage)?
                != profile.digest
        {
            return Err(Error::Denied);
        }
        let ids = {
            let mut stmt=tx.prepare("SELECT g.id,g.revision FROM scoped_secret_grants g JOIN scoped_credentials c ON c.workspace=g.workspace AND c.project=g.project AND c.host=g.host AND c.credential=g.credential AND c.state='active' AND c.revision=g.credential_revision WHERE g.workspace=?1 AND g.project=?2 AND g.host=?3 AND g.principal_kind='human' AND g.principal_id=?4 AND g.profile_digest=?5 AND g.agent_id=?6 AND g.agent_version=?7 AND g.membership_revision=?8 AND g.project_revision=?9 AND g.active=1 AND ((g.host='local' AND EXISTS(SELECT 1 FROM team_local_bindings b WHERE b.workspace_id=g.workspace AND b.project_id=g.project AND b.active=1 AND b.revision=g.host_revision)) OR (g.host!='local' AND EXISTS(SELECT 1 FROM team_hosts h WHERE h.id=g.host AND h.workspace_id=g.workspace AND h.status='approved' AND h.revision=g.host_revision))) ORDER BY g.id LIMIT 1025").map_err(storage)?;
            stmt.query_map(
                params![
                    binding.workspace,
                    binding.project,
                    binding.host,
                    actor,
                    profile.digest,
                    profile.agent_id,
                    profile.version as i64,
                    actor_stamp.membership_revision as i64,
                    actor_stamp.grant_revision as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        super::super::positive_revision(r, 1)?,
                    ))
                },
            )
            .map_err(storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
        };
        if ids.len() > 1024 {
            return Err(Error::Limit);
        }
        let mut available = Vec::new();
        for (id, revision) in ids {
            match self.resolve_secret_grant(&context, binding, profile, &id, revision) {
                Ok((grant, _)) => available.push(grant),
                Err(Error::Denied) => {}
                Err(other) => return Err(other),
            }
        }
        tx.commit().map_err(storage)?;
        Ok(available)
    }
}
#[cfg(test)]
mod tests;
