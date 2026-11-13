//! Logical team project authority is separate from native host/filesystem grants.
//! Nothing in these tables grants access to an existing host checkout or session.
use std::collections::BTreeSet;

use dunlin_domain::collaboration::{AccessContext, AuthorityStamp, Permission, ProjectGrant};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::{Result, Store, StoreError, now_secs};

pub(super) const MIGRATION: &str = "
CREATE TABLE team_projects (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
    name TEXT NOT NULL, created_at INTEGER NOT NULL,
    UNIQUE(workspace_id,id)
);
CREATE TABLE team_project_grants (
    workspace_id TEXT NOT NULL, project_id TEXT NOT NULL, user_id TEXT NOT NULL,
    permissions TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0),
    PRIMARY KEY(workspace_id,project_id,user_id),
    FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id),
    FOREIGN KEY(workspace_id,user_id) REFERENCES team_memberships(workspace_id,user_id)
);
PRAGMA user_version=10;
";

#[derive(Clone, Debug, Serialize)]
pub struct TeamProject {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub created_at: i64,
    pub authority: AuthorityStamp,
}

pub const PROJECT_PERMISSIONS: &[Permission] = &[
    Permission::ViewProject,
    Permission::ViewSharedSession,
    Permission::CreateSession,
    Permission::ShareOwnSession,
    Permission::EditWorkflow,
    Permission::RunWorkflow,
    Permission::Review,
    Permission::Publish,
    Permission::ManageProject,
];

impl Store {
    pub fn team_project_grant(
        &self,
        workspace: &str,
        project: &str,
        user: &str,
    ) -> Result<Option<ProjectGrant>> {
        let row = self.connection.query_row(
            "SELECT permissions,revision FROM team_project_grants WHERE workspace_id=?1 AND project_id=?2 AND user_id=?3",
            params![workspace,project,user], |row| Ok((row.get::<_, String>(0)?, super::positive_revision(row, 1)?)),
        ).optional()?;
        row.map(|(raw, revision)| {
            Ok(ProjectGrant {
                workspace_id: workspace.into(),
                project_id: project.into(),
                user_id: user.into(),
                revision,
                permissions: serde_json::from_str(&raw)?,
            })
        })
        .transpose()
    }

    pub fn team_project_context(
        &self,
        workspace: &str,
        project: &str,
        user: &str,
    ) -> Result<AccessContext> {
        let mut context = self.team_context(workspace, user)?;
        context.project_grant = self.team_project_grant(workspace, project, user)?;
        Ok(context)
    }

    pub fn authorize_team_project(
        &self,
        user: &str,
        workspace: &str,
        project: &str,
        permission: Permission,
    ) -> Result<AuthorityStamp> {
        Ok(self
            .team_project_context(workspace, project, user)?
            .authorize_project(workspace, project, permission)?)
    }

    pub fn create_team_project(
        &self,
        actor: &str,
        workspace: &str,
        name: &str,
    ) -> Result<TeamProject> {
        let transaction = self.connection.unchecked_transaction()?;
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageProjects)?;
        self.team_audit_capacity()?;
        let name = name.trim();
        if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
            return Err(StoreError::Limit(
                "Project name must contain 1–120 bytes without control characters".into(),
            ));
        }
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM team_projects WHERE workspace_id=?1",
            [workspace],
            |row| row.get(0),
        )?;
        if count >= 256 {
            return Err(StoreError::Limit("Team project limit reached".into()));
        }
        let id = format!("tpr-{}", uuid::Uuid::now_v7());
        let created_at = now_secs();
        let permissions: BTreeSet<_> = PROJECT_PERMISSIONS.iter().copied().collect();
        transaction.execute(
            "INSERT INTO team_projects(id,workspace_id,name,created_at) VALUES(?1,?2,?3,?4)",
            params![id, workspace, name, created_at],
        )?;
        // Explicit creator grant, not implicit workspace-owner omniscience.
        transaction.execute("INSERT INTO team_project_grants(workspace_id,project_id,user_id,permissions,revision) VALUES(?1,?2,?3,?4,1)", params![workspace,id,actor,serde_json::to_string(&permissions)?])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?2,'team_project_created',1,?3)", params![workspace,actor,created_at])?;
        let authority =
            self.authorize_team_project(actor, workspace, &id, Permission::ViewProject)?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(TeamProject {
            id,
            workspace_id: workspace.into(),
            name: name.into(),
            created_at,
            authority,
        })
    }

    pub fn team_projects_for_user(&self, user: &str, workspace: &str) -> Result<Vec<TeamProject>> {
        let context = self.team_context(workspace, user)?;
        // Validate even an empty roster, including inactive/invalid membership.
        if !context.membership.active {
            return Err(dunlin_domain::collaboration::PolicyError::InactiveMembership.into());
        }
        let mut statement = self.connection.prepare("SELECT p.id,p.name,p.created_at FROM team_projects p JOIN team_project_grants g ON p.id=g.project_id AND p.workspace_id=g.workspace_id WHERE p.workspace_id=?1 AND g.user_id=?2 ORDER BY p.created_at,p.id")?;
        let rows = statement.query_map(params![workspace, user], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut projects = Vec::new();
        for row in rows {
            let (id, name, created_at) = row?;
            match self.authorize_team_project(user, workspace, &id, Permission::ViewProject) {
                Ok(authority) => projects.push(TeamProject {
                    id,
                    workspace_id: workspace.into(),
                    name,
                    created_at,
                    authority,
                }),
                Err(StoreError::Policy(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(projects)
    }

    /// Empty permissions revoke; never delete a revision tombstone (ABA safety).
    pub fn set_team_project_grant(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        user: &str,
        permissions: BTreeSet<Permission>,
        expected_revision: Option<u64>,
    ) -> Result<ProjectGrant> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        if permissions
            .iter()
            .any(|permission| permission.is_workspace_scoped())
        {
            return Err(StoreError::Limit(
                "Project grants cannot contain workspace permissions".into(),
            ));
        }
        if !self
            .team_membership(workspace, user)?
            .is_some_and(|member| member.active)
        {
            return Err(dunlin_domain::collaboration::PolicyError::InactiveMembership.into());
        }
        let previous = self.team_project_grant(workspace, project, user)?;
        if previous.as_ref().map(|grant| grant.revision) != expected_revision {
            return Err(StoreError::Conflict(
                "Project grant changed; reload before editing it".into(),
            ));
        }
        self.team_audit_capacity()?;
        let revision = expected_revision
            .unwrap_or(0)
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Project grant revision limit reached".into()))?;
        transaction.execute("INSERT INTO team_project_grants(workspace_id,project_id,user_id,permissions,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,project_id,user_id) DO UPDATE SET permissions=excluded.permissions,revision=excluded.revision",
            params![workspace,project,user,serde_json::to_string(&permissions)?,revision as i64])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,'project_grant_changed',?4,?5)", params![workspace,actor,user,revision as i64,now_secs()])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(ProjectGrant {
            workspace_id: workspace.into(),
            project_id: project.into(),
            user_id: user.into(),
            permissions,
            revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::collaboration::WorkspaceRole;

    #[test]
    fn revocation_changes_live_fingerprint_even_when_visible_event_count_is_unchanged() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "logical")
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &member.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                [Permission::ViewProject].into(),
                None,
            )
            .unwrap();
        store
            .create_workflow(
                &owner.id,
                &workspace.id,
                &project.id,
                serde_json::from_str(include_str!(
                    "../../../../tests/fixtures/workflows/basic.json"
                ))
                .unwrap(),
            )
            .unwrap();
        let before = store.team_catalog_revision(&member.id).unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                BTreeSet::new(),
                Some(1),
            )
            .unwrap();
        assert_ne!(before, store.team_catalog_revision(&member.id).unwrap());
        let stable = store.team_catalog_revision(&member.id).unwrap();
        let outsider = store.create_user("outsider", "admin").unwrap();
        store.create_workspace(&outsider, "unrelated").unwrap();
        assert_eq!(stable, store.team_catalog_revision(&member.id).unwrap());
    }

    #[test]
    fn project_grants_are_explicit_scoped_and_do_not_grant_host_access() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let viewer = store.create_user("viewer", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &viewer.id,
                WorkspaceRole::Viewer,
                true,
                None,
            )
            .unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "logical")
            .unwrap();
        assert!(
            store
                .team_projects_for_user(&viewer.id, &workspace.id)
                .unwrap()
                .is_empty()
        );
        let grant = store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &viewer.id,
                PROJECT_PERMISSIONS.iter().copied().collect(),
                None,
            )
            .unwrap();
        assert_eq!(grant.revision, 1);
        assert_eq!(
            store
                .team_projects_for_user(&viewer.id, &workspace.id)
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .authorize_team_project(
                    &viewer.id,
                    &workspace.id,
                    &project.id,
                    Permission::RunWorkflow
                )
                .is_err()
        );
        assert!(store.projects_for_user(&viewer.id).unwrap().is_empty());
        assert!(
            store
                .authorize_team_project(&viewer.id, "other", &project.id, Permission::ViewProject)
                .is_err()
        );
        assert!(
            store
                .set_team_project_grant(
                    &viewer.id,
                    &workspace.id,
                    &project.id,
                    &viewer.id,
                    [Permission::RunWorkflow].into(),
                    Some(1)
                )
                .is_err()
        );
    }

    #[test]
    fn revocations_and_regrants_invalidate_authority_without_revision_reuse() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &member.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "logical")
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                [Permission::ViewProject].into(),
                None,
            )
            .unwrap();
        let stamp = store
            .authorize_team_project(
                &member.id,
                &workspace.id,
                &project.id,
                Permission::ViewProject,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                BTreeSet::new(),
                Some(1),
            )
            .unwrap();
        assert!(
            store
                .authorize_team_project(
                    &member.id,
                    &workspace.id,
                    &project.id,
                    Permission::ViewProject
                )
                .is_err()
        );
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                [Permission::ViewProject].into(),
                Some(2),
            )
            .unwrap();
        assert!(
            store
                .team_project_context(&workspace.id, &project.id, &member.id)
                .unwrap()
                .revalidate(&stamp, Permission::ViewProject)
                .is_err()
        );
        assert!(matches!(
            store.set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &member.id,
                BTreeSet::new(),
                Some(1)
            ),
            Err(StoreError::Conflict(_))
        ));
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &member.id,
                WorkspaceRole::Member,
                false,
                Some(1),
            )
            .unwrap();
        assert!(
            store
                .authorize_team_project(
                    &member.id,
                    &workspace.id,
                    &project.id,
                    Permission::ViewProject
                )
                .is_err()
        );
    }
}
