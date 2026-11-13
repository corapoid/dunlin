//! Opt-in team metadata. Existing projects, native roles and private sessions
//! are not silently reassigned or exposed by this migration.
use dunlin_domain::collaboration::{
    AccessContext, Membership, Permission, PrincipalKind, WorkspaceRole,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Result, Store, StoreError, User, now_secs};

pub(super) const MIGRATION: &str = "
CREATE TABLE team_workspaces (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE team_memberships (
    workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK(role IN ('owner','admin','member','reviewer','viewer')),
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    PRIMARY KEY(workspace_id,user_id)
);
CREATE INDEX team_memberships_user ON team_memberships(user_id,active);
CREATE TABLE team_audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
    actor_id TEXT NOT NULL REFERENCES users(id),
    target_id TEXT NOT NULL REFERENCES users(id),
    action TEXT NOT NULL,
    revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
PRAGMA user_version = 9;
";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub membership: Membership,
}

#[derive(Clone, Debug, Serialize)]
pub struct TeamAudit {
    pub sequence: i64,
    pub actor_id: String,
    pub target_id: String,
    pub action: String,
    pub revision: i64,
    pub created_at: i64,
    pub project_id: Option<String>,
    pub resource_id: Option<String>,
}

fn role_name(role: WorkspaceRole) -> &'static str {
    match role {
        WorkspaceRole::Owner => "owner",
        WorkspaceRole::Admin => "admin",
        WorkspaceRole::Member => "member",
        WorkspaceRole::Reviewer => "reviewer",
        WorkspaceRole::Viewer => "viewer",
    }
}

fn role_value(role: String) -> rusqlite::Result<WorkspaceRole> {
    match role.as_str() {
        "owner" => Ok(WorkspaceRole::Owner),
        "admin" => Ok(WorkspaceRole::Admin),
        "member" => Ok(WorkspaceRole::Member),
        "reviewer" => Ok(WorkspaceRole::Reviewer),
        "viewer" => Ok(WorkspaceRole::Viewer),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn membership(row: &rusqlite::Row<'_>) -> rusqlite::Result<Membership> {
    let revision: i64 = row.get(4)?;
    if revision <= 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Membership {
        workspace_id: row.get(0)?,
        user_id: row.get(1)?,
        role: role_value(row.get(2)?)?,
        active: row.get(3)?,
        revision: revision as u64,
    })
}

impl Store {
    /// Only events visible to this user affect their live workspace revision.
    /// Viewers do not learn about mutations in unrelated workspaces or rosters.
    pub fn team_catalog_revision(&self, user: &str) -> Result<u64> {
        let mut statement = self.connection.prepare(
            "SELECT a.actor_id,a.target_id,a.action,a.revision,a.created_at,a.project_id,a.resource_id FROM team_audit a
             JOIN team_memberships m ON m.workspace_id=a.workspace_id
             WHERE m.user_id=?1 AND (a.target_id=?1 OR (m.active=1 AND (
                 m.role IN ('owner','admin') OR EXISTS (
                     SELECT 1 FROM team_project_grants g, json_each(g.permissions) p
                     WHERE g.workspace_id=a.workspace_id AND g.project_id=a.project_id
                         AND g.user_id=?1 AND p.value='view_project'
                 )
             ))) ORDER BY a.seq",
        )?;
        let rows = statement.query_map([user], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut hash = Sha256::new();
        let mut seen = false;
        for row in rows {
            hash.update(serde_json::to_vec(&row?)?);
            seen = true;
        }
        // Worker heartbeats never consume audit capacity or invalidate control
        // CAS. Only workspace host managers observe their metadata fingerprint.
        let mut hosts = self.connection.prepare(
            "SELECT h.id,h.revision,h.capability_revision,h.last_seen FROM team_hosts h
             JOIN team_memberships m ON m.workspace_id=h.workspace_id
             WHERE m.user_id=?1 AND m.active=1 AND m.role IN ('owner','admin') ORDER BY h.id",
        )?;
        let reports = hosts.query_map([user], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for report in reports {
            hash.update(serde_json::to_vec(&report?)?);
            seen = true;
        }
        if !seen {
            return Ok(0);
        }
        // Change fingerprint, not a monotonic/global audit sequence. Counts can
        // stay equal when revocation hides an event and adds another. Hash only
        // visible payloads, never global sequence IDs; 48 bits are JS-safe.
        let digest = hash.finalize();
        Ok(u64::from_be_bytes([
            0, 0, digest[0], digest[1], digest[2], digest[3], digest[4], digest[5],
        ]))
    }

    pub fn team_membership(&self, workspace: &str, user: &str) -> Result<Option<Membership>> {
        Ok(self.connection.query_row("SELECT workspace_id,user_id,role,active,revision FROM team_memberships WHERE workspace_id=?1 AND user_id=?2",
            params![workspace,user], membership).optional()?)
    }

    pub(super) fn team_context(&self, workspace: &str, user: &str) -> Result<AccessContext> {
        let membership = self
            .team_membership(workspace, user)?
            .ok_or(dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        Ok(AccessContext {
            principal: PrincipalKind::Human,
            membership,
            project_grant: None,
        })
    }

    pub(super) fn team_audit_capacity(&self) -> Result<()> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM team_audit", [], |row| row.get(0))?;
        if count >= 100_000 {
            return Err(StoreError::Limit(
                "Team audit limit reached; administrator maintenance required".into(),
            ));
        }
        Ok(())
    }

    pub fn create_workspace(&self, actor: &User, name: &str) -> Result<Workspace> {
        // Bootstrap is a relay-admin operation, not arbitrary tenant creation.
        // Reread the role rather than trust a stale browser-provided object.
        let current = self
            .find_user(&actor.id)?
            .filter(|user| user.role == "admin")
            .ok_or(dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        let name = name.trim();
        if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
            return Err(StoreError::Limit(
                "Workspace name must contain 1–120 bytes without control characters".into(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        self.team_audit_capacity()?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM team_workspaces", [], |row| row.get(0))?;
        if count >= 128 {
            return Err(StoreError::Limit("Workspace limit reached".into()));
        }
        let id = format!("wsp-{}", uuid::Uuid::now_v7());
        let created_at = now_secs();
        transaction.execute(
            "INSERT INTO team_workspaces(id,name,created_at) VALUES(?1,?2,?3)",
            params![id, name, created_at],
        )?;
        transaction.execute("INSERT INTO team_memberships(workspace_id,user_id,role,active,revision) VALUES(?1,?2,'owner',1,1)", params![id,current.id])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?2,'workspace_created',1,?3)", params![id,current.id,created_at])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(Workspace {
            id: id.clone(),
            name: name.into(),
            created_at,
            membership: Membership {
                workspace_id: id,
                user_id: current.id,
                role: WorkspaceRole::Owner,
                revision: 1,
                active: true,
            },
        })
    }

    pub fn workspaces_for_user(&self, user: &str) -> Result<Vec<Workspace>> {
        let mut statement = self.connection.prepare("SELECT m.workspace_id,m.user_id,m.role,m.active,m.revision,w.name,w.created_at FROM team_memberships m JOIN team_workspaces w ON w.id=m.workspace_id WHERE m.user_id=?1 AND m.active=1 ORDER BY w.created_at,w.id")?;
        let rows = statement.query_map([user], |row| {
            let member = membership(row)?;
            Ok(Workspace {
                id: member.workspace_id.clone(),
                name: row.get(5)?,
                created_at: row.get(6)?,
                membership: member,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn team_members(&self, actor: &str, workspace: &str) -> Result<Vec<Membership>> {
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        let mut statement = self.connection.prepare("SELECT workspace_id,user_id,role,active,revision FROM team_memberships WHERE workspace_id=?1 ORDER BY user_id")?;
        Ok(statement
            .query_map([workspace], membership)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_team_member(
        &self,
        actor: &str,
        workspace: &str,
        user: &str,
        role: WorkspaceRole,
        active: bool,
        expected_revision: Option<u64>,
    ) -> Result<Membership> {
        let transaction = self.connection.unchecked_transaction()?;
        let member = self.set_team_member_in_transaction(
            actor,
            workspace,
            user,
            role,
            active,
            expected_revision,
        )?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(member)
    }

    pub(super) fn set_team_member_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        user: &str,
        role: WorkspaceRole,
        active: bool,
        expected_revision: Option<u64>,
    ) -> Result<Membership> {
        let context = self.team_context(workspace, actor)?;
        context.authorize_workspace(workspace, Permission::ManageMembers)?;
        let previous = self.team_membership(workspace, user)?;
        if previous.as_ref().map(|member| member.revision) != expected_revision {
            return Err(StoreError::Conflict(
                "Membership changed; reload before editing it".into(),
            ));
        }
        if role == WorkspaceRole::Owner
            || previous
                .as_ref()
                .is_some_and(|member| member.role == WorkspaceRole::Owner)
        {
            context.authorize_workspace(workspace, Permission::TransferOwnership)?;
        }
        if previous
            .as_ref()
            .is_some_and(|member| member.active && member.role == WorkspaceRole::Owner)
            && (!active || role != WorkspaceRole::Owner)
        {
            let owners: i64 = self.connection.query_row("SELECT COUNT(*) FROM team_memberships WHERE workspace_id=?1 AND role='owner' AND active=1", [workspace], |row| row.get(0))?;
            if owners <= 1 {
                return Err(StoreError::Conflict(
                    "A workspace must retain an active owner".into(),
                ));
            }
        }
        self.team_audit_capacity()?;
        if self.find_user(user)?.is_none() {
            return Err(StoreError::Limit(
                "Recipient must be an existing relay user".into(),
            ));
        }
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM team_memberships WHERE workspace_id=?1",
            [workspace],
            |row| row.get(0),
        )?;
        if previous.is_none() && count >= 256 {
            return Err(StoreError::Limit(
                "Workspace membership limit reached".into(),
            ));
        }
        let revision = expected_revision
            .unwrap_or(0)
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Membership revision limit reached".into()))?;
        self.connection.execute("INSERT INTO team_memberships(workspace_id,user_id,role,active,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,user_id) DO UPDATE SET role=excluded.role,active=excluded.active,revision=excluded.revision",
            params![workspace,user,role_name(role),active,revision as i64])?;
        self.connection.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,'membership_changed',?4,?5)", params![workspace,actor,user,revision as i64,now_secs()])?;
        Ok(Membership {
            workspace_id: workspace.into(),
            user_id: user.into(),
            role,
            active,
            revision,
        })
    }

    pub fn team_audit(&self, actor: &str, workspace: &str) -> Result<Vec<TeamAudit>> {
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        let mut statement = self.connection.prepare("SELECT seq,actor_id,target_id,action,revision,created_at,project_id,resource_id FROM team_audit WHERE workspace_id=?1 ORDER BY seq DESC LIMIT 100")?;
        Ok(statement
            .query_map([workspace], |row| {
                Ok(TeamAudit {
                    sequence: row.get(0)?,
                    actor_id: row.get(1)?,
                    target_id: row.get(2)?,
                    action: row.get(3)?,
                    revision: row.get(4)?,
                    created_at: row.get(5)?,
                    project_id: row.get(6)?,
                    resource_id: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_creation_is_opt_in_and_preserves_private_legacy_data() {
        let store = Store::open_in_memory().unwrap();
        let admin = store.create_user("admin", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        assert!(store.workspaces_for_user(&admin.id).unwrap().is_empty());
        assert!(store.create_workspace(&member, "forbidden").is_err());
        let workspace = store.create_workspace(&admin, "team").unwrap();
        assert_eq!(store.workspaces_for_user(&admin.id).unwrap().len(), 1);
        assert!(store.workspaces_for_user(&member.id).unwrap().is_empty());
        assert_eq!(store.team_audit(&admin.id, &workspace.id).unwrap().len(), 1);
    }

    #[test]
    fn stale_membership_writes_cannot_overwrite_newer_roles() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("admin", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let first = store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &member.id,
                WorkspaceRole::Reviewer,
                true,
                None,
            )
            .unwrap();
        assert_eq!(first.revision, 1);
        assert!(matches!(
            store.set_team_member(
                &owner.id,
                &workspace.id,
                &member.id,
                WorkspaceRole::Admin,
                true,
                None
            ),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(
            store
                .team_membership(&workspace.id, &member.id)
                .unwrap()
                .unwrap()
                .role,
            WorkspaceRole::Reviewer
        );
    }

    #[test]
    fn last_owner_and_owner_permissions_are_preserved() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let admin = store.create_user("other", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &admin.id,
                WorkspaceRole::Admin,
                true,
                None,
            )
            .unwrap();
        assert!(
            store
                .set_team_member(
                    &owner.id,
                    &workspace.id,
                    &owner.id,
                    WorkspaceRole::Member,
                    true,
                    Some(1)
                )
                .is_err()
        );
        assert!(
            store
                .set_team_member(
                    &admin.id,
                    &workspace.id,
                    &admin.id,
                    WorkspaceRole::Owner,
                    true,
                    Some(1)
                )
                .is_err()
        );
        assert_eq!(
            store
                .team_membership(&workspace.id, &owner.id)
                .unwrap()
                .unwrap()
                .role,
            WorkspaceRole::Owner
        );
    }

    #[test]
    fn unrelated_workspace_and_revoked_members_cannot_manage_memberships() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let member = store.create_user("member", "member").unwrap();
        let a = store.create_workspace(&owner, "a").unwrap();
        let b = store.create_workspace(&owner, "b").unwrap();
        store
            .set_team_member(
                &owner.id,
                &a.id,
                &member.id,
                WorkspaceRole::Admin,
                true,
                None,
            )
            .unwrap();
        assert!(store.team_members(&member.id, &b.id).is_err());
        store
            .set_team_member(
                &owner.id,
                &a.id,
                &member.id,
                WorkspaceRole::Admin,
                false,
                Some(1),
            )
            .unwrap();
        assert!(store.team_members(&member.id, &a.id).is_err());
        assert!(store.workspaces_for_user(&member.id).unwrap().is_empty());
    }

    #[test]
    fn newer_database_version_is_refused_without_overwriting_existing_teams() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        store.create_workspace(&owner, "team").unwrap();
        let future_version = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap()
            + 1;
        store
            .connection
            .pragma_update(None, "user_version", future_version)
            .unwrap();
        assert!(store.migrate().is_err());
        assert_eq!(store.workspaces_for_user(&owner.id).unwrap().len(), 1);
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            future_version
        );
    }

    #[test]
    fn private_workspace_changes_do_not_change_other_users_live_revisions() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let outsider = store.create_user("outsider", "member").unwrap();
        assert_eq!(store.team_catalog_revision(&outsider.id).unwrap(), 0);
        store.create_workspace(&owner, "private metadata").unwrap();
        assert_eq!(store.team_catalog_revision(&outsider.id).unwrap(), 0);
        assert_ne!(store.team_catalog_revision(&owner.id).unwrap(), 0);
    }
}
