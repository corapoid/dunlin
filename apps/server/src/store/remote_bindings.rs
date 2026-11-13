//! Explicit controller bindings to opaque worker exports. Observed metadata is
//! not execution authority and never contains a worker filesystem path.
use super::{Result, Store, StoreError, now_secs};
use crate::worker_channel::ObservedExport;
use dunlin_domain::collaboration::{AuthorityStamp, Permission, PolicyError};
use dunlin_relay_protocol::worker::{ExportPermission, ExportPrincipal, PrincipalKind, RepoKind};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
pub(crate) mod resolver;
pub(crate) use resolver::{
    RemoteTargetCandidate, ResolvedRemoteTarget, ResolvedRemoteTargetSummary,
};
pub(crate) const REMOTE_BINDING_SCHEMA:&str="
CREATE TABLE team_remote_bindings (
 id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,project_id TEXT NOT NULL,
 host_id TEXT NOT NULL REFERENCES team_hosts(id),controller_node_id TEXT NOT NULL,host_enrollment_epoch INTEGER NOT NULL,credential_revision INTEGER NOT NULL,
 export_id TEXT NOT NULL,export_revision INTEGER NOT NULL,repo_kind TEXT NOT NULL,checkout_identity_digest TEXT NOT NULL,
 approved_settings_digest TEXT NOT NULL,principal TEXT NOT NULL,grant_revision INTEGER NOT NULL,grant_permissions TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),active INTEGER NOT NULL CHECK(active IN(0,1)),observed_at_ms INTEGER NOT NULL CHECK(observed_at_ms>=0),
 FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id),UNIQUE(workspace_id,project_id,host_id,export_id)
);
CREATE TABLE team_remote_binding_revocations (
 binding_id TEXT PRIMARY KEY REFERENCES team_remote_bindings(id),workspace_id TEXT NOT NULL,project_id TEXT NOT NULL,
 actor_id TEXT NOT NULL REFERENCES users(id),revision INTEGER NOT NULL,created_at INTEGER NOT NULL
);
";
const MAX_REVISION: u64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteBinding {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub host_id: String,
    pub host_enrollment_epoch: u64,
    pub credential_revision: u64,
    pub export_id: String,
    pub export_revision: u64,
    pub repo_kind: RepoKind,
    pub checkout_identity_digest: String,
    pub approved_settings_digest: String,
    pub principal: ExportPrincipal,
    pub grant_revision: u64,
    pub grant_permissions: Vec<ExportPermission>,
    pub revision: u64,
    pub active: bool,
    pub observed_at_ms: i64,
}
fn denied() -> StoreError {
    PolicyError::PermissionDenied.into()
}
fn conflict() -> StoreError {
    StoreError::Conflict("Remote binding changed or is unavailable; reload before editing".into())
}
fn number(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(column)?)
        .ok()
        .filter(|v| *v > 0)
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn next(value: u64, positive: bool) -> Result<u64> {
    if !positive && value >= MAX_REVISION {
        return Ok(MAX_REVISION);
    }
    value
        .checked_add(1)
        .filter(|v| *v <= MAX_REVISION && (!positive || *v < MAX_REVISION))
        .ok_or_else(|| StoreError::Limit("Remote binding revision exhausted".into()))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
impl Store {
    pub(crate) fn remote_binding_transaction<T, E>(
        &self,
        mutation: bool,
        operation: impl FnOnce(&Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<StoreError>,
    {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(StoreError::from)
        .map_err(E::from)?;
        let result = operation(self)?;
        tx.commit().map_err(StoreError::from).map_err(E::from)?;
        if mutation {
            self.catalog_changed();
        }
        Ok(result)
    }
    pub fn capture_remote_binding_authority(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<AuthorityStamp> {
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageHosts)?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)
    }
    fn remote_binding_row(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<RemoteBinding>> {
        let row=self.connection.query_row("SELECT id,workspace_id,project_id,host_id,host_enrollment_epoch,credential_revision,export_id,export_revision,repo_kind,checkout_identity_digest,approved_settings_digest,principal,grant_revision,grant_permissions,revision,active,observed_at_ms FROM team_remote_bindings WHERE workspace_id=?1 AND project_id=?2 AND id=?3",params![workspace,project,id],|r|Ok((RemoteBinding{id:r.get(0)?,workspace_id:r.get(1)?,project_id:r.get(2)?,host_id:r.get(3)?,host_enrollment_epoch:number(r,4)?,credential_revision:number(r,5)?,export_id:r.get(6)?,export_revision:number(r,7)?,repo_kind:RepoKind::Directory,checkout_identity_digest:r.get(9)?,approved_settings_digest:r.get(10)?,principal:ExportPrincipal{kind:PrincipalKind::Human,id:String::new()},grant_revision:number(r,12)?,grant_permissions:vec![],revision:number(r,14)?,active:r.get(15)?,observed_at_ms:r.get(16)?},r.get::<_,String>(8)?,r.get::<_,String>(11)?,r.get::<_,String>(13)?))).optional()?;
        row.map(|(mut binding, kind, principal, permissions)| {
            binding.repo_kind = serde_json::from_str(&kind)?;
            binding.principal = serde_json::from_str(&principal)?;
            binding.grant_permissions = serde_json::from_str(&permissions)?;
            Ok(binding)
        })
        .transpose()
    }
    #[cfg(test)]
    pub fn remote_bindings(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Vec<RemoteBinding>> {
        self.remote_binding_transaction(false, |store| {
            store.remote_bindings_in_transaction(actor, workspace, project)
        })
    }
    pub(crate) fn remote_bindings_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Vec<RemoteBinding>> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "A remote binding authority transaction is required".into(),
            ));
        }
        self.capture_remote_binding_authority(actor, workspace, project)?;
        let mut statement=self.connection.prepare("SELECT id FROM team_remote_bindings WHERE workspace_id=?1 AND project_id=?2 ORDER BY id LIMIT 16")?;
        let ids = statement
            .query_map(params![workspace, project], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let bindings = ids
            .into_iter()
            .map(|id| {
                self.remote_binding_row(workspace, project, &id)?
                    .ok_or_else(conflict)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(bindings)
    }
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_remote_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        binding_id: Option<&str>,
        expected_revision: Option<u64>,
        host_id: &str,
        export_id: &str,
        expected_export_revision: u64,
        proof: &ObservedExport,
        current_controller_boot_id: &str,
        current_controller_node_id: &str,
        now_ms: i64,
        captured: &AuthorityStamp,
    ) -> Result<RemoteBinding> {
        self.remote_binding_transaction(true, |store| {
            store.set_remote_binding_in_transaction(
                actor,
                workspace,
                project,
                binding_id,
                expected_revision,
                host_id,
                export_id,
                expected_export_revision,
                proof,
                current_controller_boot_id,
                current_controller_node_id,
                now_ms,
                captured,
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_remote_binding_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        binding_id: Option<&str>,
        expected_revision: Option<u64>,
        host_id: &str,
        export_id: &str,
        expected_export_revision: u64,
        proof: &ObservedExport,
        current_controller_boot_id: &str,
        current_controller_node_id: &str,
        now_ms: i64,
        captured: &AuthorityStamp,
    ) -> Result<RemoteBinding> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "A remote binding authority transaction is required".into(),
            ));
        }
        if !identifier(host_id)
            || !identifier(export_id)
            || expected_export_revision == 0
            || expected_export_revision > MAX_REVISION
            || now_ms < 0
            || binding_id.is_some() != expected_revision.is_some()
        {
            return Err(StoreError::Limit(
                "Invalid remote binding identity or revision".into(),
            ));
        }
        let current = self.capture_remote_binding_authority(actor, workspace, project)?;
        if &current != captured {
            return Err(conflict());
        }
        let descriptor = proof.descriptor();
        descriptor.validate().map_err(|_| denied())?;
        if !proof.is_fresh()
            || proof.observed_at_ms() < 0
            || proof.observed_at_ms() > now_ms
            || proof.identity().host_id() != host_id
            || proof.identity().workspace_id() != workspace
            || descriptor.scope.controller_node_id != current_controller_node_id
            || descriptor.scope.workspace_id != workspace
            || descriptor.scope.host_id != host_id
            || descriptor.scope.worker_node_id != proof.identity().host().definition.node_id
            || descriptor.scope.enrollment_epoch != proof.identity().host().enrollment_epoch
            || descriptor.scope.credential_revision != proof.identity().host().credential_revision
            || descriptor.export.id != export_id
            || descriptor.export.revision != expected_export_revision
            || !descriptor.export.active
            || descriptor.grant.principal.kind != PrincipalKind::Human
            || descriptor.grant.principal.id != actor
            || !descriptor.grant.active
            || descriptor.grant.export_revision != descriptor.export.revision
            || !descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Read)
        {
            return Err(denied());
        }
        self.validate_worker_channel_in_transaction(
            proof.identity(),
            proof.lease(),
            current_controller_boot_id,
            now_ms,
        )?;
        // A path or arbitrary descriptor never comes from the request body. The
        // proof exists only after current-channel correlation in controller Service.
        let previous = match binding_id {
            Some(id) => Some(
                self.remote_binding_row(workspace, project, id)?
                    .ok_or_else(conflict)?,
            ),
            None => None,
        };
        if previous.as_ref().map(|b| b.revision) != expected_revision {
            return Err(conflict());
        }
        let id = previous
            .as_ref()
            .map(|b| b.id.clone())
            .unwrap_or_else(|| format!("rbd-{}", uuid::Uuid::now_v7()));
        let duplicate:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM team_remote_bindings WHERE workspace_id=?1 AND project_id=?2 AND host_id=?3 AND export_id=?4 AND id<>?5)",params![workspace,project,host_id,export_id,id],|r|r.get(0))?;
        if duplicate {
            return Err(StoreError::Conflict(
                "This host/export already has a binding; update its existing revision".into(),
            ));
        }
        if previous.is_none() {
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM team_remote_bindings WHERE workspace_id=?1 AND project_id=?2",
                params![workspace, project],
                |r| r.get(0),
            )?;
            if count >= 16 {
                return Err(StoreError::Limit(
                    "Project remote binding limit reached".into(),
                ));
            }
        }
        let revision = next(expected_revision.unwrap_or(0), true)?;
        self.team_audit_capacity()?;
        self.connection.execute("INSERT INTO team_remote_bindings(id,workspace_id,project_id,host_id,host_enrollment_epoch,credential_revision,export_id,export_revision,repo_kind,checkout_identity_digest,approved_settings_digest,principal,grant_revision,grant_permissions,revision,active,observed_at_ms,controller_node_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,1,?16,?17) ON CONFLICT(id) DO UPDATE SET controller_node_id=excluded.controller_node_id,host_id=excluded.host_id,host_enrollment_epoch=excluded.host_enrollment_epoch,credential_revision=excluded.credential_revision,export_id=excluded.export_id,export_revision=excluded.export_revision,repo_kind=excluded.repo_kind,checkout_identity_digest=excluded.checkout_identity_digest,approved_settings_digest=excluded.approved_settings_digest,principal=excluded.principal,grant_revision=excluded.grant_revision,grant_permissions=excluded.grant_permissions,revision=excluded.revision,active=1,observed_at_ms=excluded.observed_at_ms",params![id,workspace,project,host_id,descriptor.scope.enrollment_epoch as i64,descriptor.scope.credential_revision as i64,export_id,descriptor.export.revision as i64,serde_json::to_string(&descriptor.export.repo_kind)?,descriptor.export.checkout_identity_digest,descriptor.export.settings_digest,serde_json::to_string(&descriptor.grant.principal)?,descriptor.grant.revision as i64,serde_json::to_string(&descriptor.grant.permissions)?,revision as i64,proof.observed_at_ms(),current_controller_node_id])?;
        self.connection.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,?3,'remote_binding_set',?4,?5,?6)",params![workspace,project,actor,revision as i64,now_secs(),id])?;
        let binding = self
            .remote_binding_row(workspace, project, &id)?
            .ok_or_else(conflict)?;
        Ok(binding)
    }
    #[cfg(test)]
    pub fn revoke_remote_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
    ) -> Result<RemoteBinding> {
        self.remote_binding_transaction(true, |store| {
            store.revoke_remote_binding_in_transaction(
                actor,
                workspace,
                project,
                id,
                expected_revision,
            )
        })
    }
    pub(crate) fn revoke_remote_binding_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
    ) -> Result<RemoteBinding> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "A remote binding authority transaction is required".into(),
            ));
        }
        self.capture_remote_binding_authority(actor, workspace, project)?;
        let mut binding = self
            .remote_binding_row(workspace, project, id)?
            .ok_or_else(conflict)?;
        if binding.revision != expected_revision {
            return Err(conflict());
        }
        if !binding.active {
            return Ok(binding);
        }
        binding.revision = next(binding.revision, false)?;
        binding.active = false;
        self.connection.execute(
            "UPDATE team_remote_bindings SET active=0,revision=?2 WHERE id=?1",
            params![id, binding.revision as i64],
        )?;
        self.connection.execute("INSERT INTO team_remote_binding_revocations VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(binding_id) DO UPDATE SET actor_id=excluded.actor_id,revision=excluded.revision,created_at=excluded.created_at",params![id,workspace,project,actor,binding.revision as i64,now_secs()])?;
        match self.team_audit_capacity() {
            Ok(()) => {
                self.connection.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,?3,'remote_binding_revoked',?4,?5,?6)",params![workspace,project,actor,binding.revision as i64,now_secs(),id])?;
            }
            Err(StoreError::Limit(message))
                if message == "Team audit limit reached; administrator maintenance required" => {}
            Err(error) => return Err(error),
        }
        Ok(binding)
    }
}
#[cfg(test)]
#[path = "remote_bindings_tests.rs"]
mod tests;
