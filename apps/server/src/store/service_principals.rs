//! Automation credentials are not users, native devices or browser sessions.
use super::{Result, Store, StoreError, auth, now_secs};
use dunlin_domain::collaboration::{
    AccessContext, Membership, Permission, PrincipalKind, ProjectGrant, WorkspaceRole,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::collections::BTreeSet;

pub(super) const MIGRATION: &str = "
CREATE TABLE team_service_principals (
 id TEXT PRIMARY KEY,
 workspace_id TEXT NOT NULL,
 project_id TEXT NOT NULL,
 name TEXT NOT NULL,
 permissions TEXT NOT NULL,
 token_hash TEXT NOT NULL UNIQUE,
 expires_at INTEGER NOT NULL,
 revision INTEGER NOT NULL CHECK(revision>0),
 revoked INTEGER NOT NULL CHECK(revoked IN (0,1)),
 FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id)
);
PRAGMA user_version=15;
";
#[derive(Debug, Clone, Serialize)]
pub struct ServicePrincipal {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
    pub expires_at: i64,
    pub revision: u64,
    pub revoked: bool,
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(ServicePrincipal, String)> {
    Ok((
        ServicePrincipal {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            project_id: r.get(2)?,
            name: r.get(3)?,
            permissions: BTreeSet::new(),
            expires_at: r.get(5)?,
            revision: super::positive_revision(r, 6)?,
            revoked: r.get(7)?,
        },
        r.get(4)?,
    ))
}
fn decode((mut principal, permissions): (ServicePrincipal, String)) -> Result<ServicePrincipal> {
    principal.permissions = serde_json::from_str(&permissions)?;
    Ok(principal)
}
impl Store {
    pub fn service_workflow_version(
        &self,
        token: &str,
        workspace: &str,
        project: &str,
        id: &str,
        version: u64,
    ) -> Result<Option<super::workflows::WorkflowVersion>> {
        self.service_authority(token, workspace, project, Permission::ViewProject)?;
        self.load_workflow_version(workspace, project, id, version)
    }

    pub fn create_service_principal(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        name: &str,
        permissions: BTreeSet<Permission>,
        lifetime: u64,
    ) -> Result<(ServicePrincipal, String)> {
        let tx = self.connection.unchecked_transaction()?;
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        let name = name.trim();
        if name.is_empty()
            || name.len() > 120
            || name.chars().any(char::is_control)
            || lifetime == 0
            || lifetime > 30 * 86400
            || !permissions.contains(&Permission::ViewProject)
            || permissions
                .iter()
                .any(|p| !matches!(p, Permission::ViewProject | Permission::RunWorkflow))
        {
            return Err(StoreError::Limit(
                "Invalid automation name, lifetime or permissions (view_project/run_workflow only)"
                    .into(),
            ));
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM team_projects WHERE workspace_id=?1 AND id=?2)",
            params![workspace, project],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(dunlin_domain::collaboration::PolicyError::PermissionDenied.into());
        }
        self.team_audit_capacity()?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM team_service_principals WHERE workspace_id=?1",
            [workspace],
            |r| r.get(0),
        )?;
        if count >= 256 {
            return Err(StoreError::Limit("Service principal limit reached".into()));
        }
        let token = auth::random_token().replacen("kst_", "dsp_", 1);
        let principal = ServicePrincipal {
            id: format!("spn-{}", uuid::Uuid::now_v7()),
            workspace_id: workspace.into(),
            project_id: project.into(),
            name: name.into(),
            permissions,
            expires_at: now_secs() + lifetime as i64,
            revision: 1,
            revoked: false,
        };
        tx.execute(
            "INSERT INTO team_service_principals VALUES(?1,?2,?3,?4,?5,?6,?7,1,0)",
            params![
                principal.id,
                workspace,
                project,
                name,
                serde_json::to_string(&principal.permissions)?,
                auth::hash_token(&token),
                principal.expires_at
            ],
        )?;
        tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,project_id) VALUES(?1,?2,?2,?3,1,?4,?5)",params![workspace,actor,format!("service_created:{}",principal.id),now_secs(),project])?;
        tx.commit()?;
        self.catalog_changed();
        Ok((principal, token))
    }
    pub fn service_principals(
        &self,
        actor: &str,
        workspace: &str,
    ) -> Result<Vec<ServicePrincipal>> {
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        let mut q=self.connection.prepare("SELECT id,workspace_id,project_id,name,permissions,expires_at,revision,revoked FROM team_service_principals WHERE workspace_id=?1 ORDER BY id")?;
        q.query_map([workspace], row)?.map(|r| decode(r?)).collect()
    }
    pub fn revoke_service_principal(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        revision: u64,
    ) -> Result<ServicePrincipal> {
        let tx = self.connection.unchecked_transaction()?;
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        let principal=tx.query_row("SELECT id,workspace_id,project_id,name,permissions,expires_at,revision,revoked FROM team_service_principals WHERE workspace_id=?1 AND id=?2",params![workspace,id],row).optional()?.ok_or(dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        let mut principal = decode(principal)?;
        if principal.revision != revision {
            return Err(StoreError::Conflict(
                "Service principal changed; reload before revoking".into(),
            ));
        }
        if principal.revoked {
            return Ok(principal);
        }
        self.team_audit_capacity()?;
        principal.revision = revision
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Revision limit reached".into()))?;
        principal.revoked = true;
        tx.execute("UPDATE team_service_principals SET revoked=1,revision=?3 WHERE workspace_id=?1 AND id=?2",params![workspace,id,principal.revision as i64])?;
        tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,project_id) VALUES(?1,?2,?2,?3,?4,?5,?6)",params![workspace,actor,format!("service_revoked:{id}"),principal.revision as i64,now_secs(),principal.project_id])?;
        tx.commit()?;
        self.catalog_changed();
        Ok(principal)
    }
    pub fn authenticate_service(&self, token: &str) -> Result<Option<ServicePrincipal>> {
        if token.len() != 47
            || !token.starts_with("dsp_")
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Ok(None);
        }
        self.connection.query_row("SELECT id,workspace_id,project_id,name,permissions,expires_at,revision,revoked FROM team_service_principals WHERE token_hash=?1 AND revoked=0 AND expires_at>?2",params![auth::hash_token(token),now_secs()],row).optional()?.map(decode).transpose()
    }
    pub fn service_authority(
        &self,
        token: &str,
        workspace: &str,
        project: &str,
        permission: Permission,
    ) -> Result<AccessContext> {
        let principal = self
            .authenticate_service(token)?
            .ok_or(dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        let context = AccessContext {
            principal: PrincipalKind::Service,
            membership: Membership {
                workspace_id: principal.workspace_id.clone(),
                user_id: principal.id.clone(),
                role: WorkspaceRole::Member,
                revision: principal.revision,
                active: true,
            },
            project_grant: Some(ProjectGrant {
                workspace_id: principal.workspace_id,
                project_id: principal.project_id,
                user_id: principal.id,
                permissions: principal.permissions,
                revision: principal.revision,
            }),
        };
        context.authorize_project(workspace, project, permission)?;
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automation_is_scoped_not_a_user_and_survives_restart_with_hash_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("services.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "project")
            .unwrap();
        assert!(
            store
                .create_service_principal(
                    &member.id,
                    &w.id,
                    &p.id,
                    "denied",
                    BTreeSet::from([Permission::ViewProject]),
                    600
                )
                .is_err()
        );
        assert!(
            store
                .create_service_principal(
                    &owner.id,
                    &w.id,
                    &p.id,
                    "denied",
                    BTreeSet::from([Permission::ViewProject, Permission::Review]),
                    600
                )
                .is_err()
        );
        let (principal, token) = store
            .create_service_principal(
                &owner.id,
                &w.id,
                &p.id,
                "automation",
                BTreeSet::from([Permission::ViewProject, Permission::RunWorkflow]),
                600,
            )
            .unwrap();
        assert!(store.authenticate(&token).unwrap().is_none());
        assert!(store.browser_session_device(&token).unwrap().is_none());
        assert!(store.find_user(&principal.id).unwrap().is_none());
        let hash: String = store
            .connection
            .query_row(
                "SELECT token_hash FROM team_service_principals WHERE id=?1",
                [&principal.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hash, auth::hash_token(&token));
        assert_ne!(hash, token);
        let context = store
            .service_authority(&token, &w.id, &p.id, Permission::RunWorkflow)
            .unwrap();
        assert_eq!(context.principal, PrincipalKind::Service);
        assert!(
            context
                .authorize_approval(&w.id, &p.id, &owner.id, false)
                .is_err()
        );
        assert!(
            store
                .service_authority(&token, &w.id, "other", Permission::ViewProject)
                .is_err()
        );
        assert!(
            store
                .service_authority(&token, &w.id, &p.id, Permission::Publish)
                .is_err()
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.authenticate_service(&token).unwrap().is_some());
        assert!(matches!(
            store.revoke_service_principal(&owner.id, &w.id, &principal.id, 2),
            Err(StoreError::Conflict(_))
        ));
        let revoked = store
            .revoke_service_principal(&owner.id, &w.id, &principal.id, 1)
            .unwrap();
        assert_eq!(revoked.revision, 2);
        assert!(store.authenticate_service(&token).unwrap().is_none());
        assert!(
            !serde_json::to_string(&store.service_principals(&owner.id, &w.id).unwrap())
                .unwrap()
                .contains(&token)
        );
    }
    #[test]
    fn expiration_and_workspace_boundaries_are_authoritative() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        let other = store.create_workspace(&owner, "other").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "project")
            .unwrap();
        assert!(
            store
                .create_service_principal(
                    &owner.id,
                    &other.id,
                    &p.id,
                    "cross scope",
                    BTreeSet::from([Permission::ViewProject]),
                    600
                )
                .is_err()
        );
        let (principal, token) = store
            .create_service_principal(
                &owner.id,
                &w.id,
                &p.id,
                "expiry",
                BTreeSet::from([Permission::ViewProject]),
                600,
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE team_service_principals SET expires_at=0 WHERE id=?1",
                [principal.id],
            )
            .unwrap();
        assert!(store.authenticate_service(&token).unwrap().is_none());
    }
}
