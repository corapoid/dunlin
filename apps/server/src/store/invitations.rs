//! Recipient-bound invitations for existing relay identities. Tokens are returned
//! once; only hashes persist. OIDC provisioning is a separate boundary.
use super::{Result, Store, StoreError, auth, now_secs};
use dunlin_domain::collaboration::{Membership, Permission, PolicyError, WorkspaceRole};
use rusqlite::{OptionalExtension, params};

pub(super) const MIGRATION: &str = "
CREATE TABLE team_invitations (
 token_hash TEXT PRIMARY KEY,
 workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
 issuer_id TEXT NOT NULL REFERENCES users(id),
 recipient_id TEXT NOT NULL REFERENCES users(id),
 issuer_revision INTEGER NOT NULL,
 recipient_revision INTEGER,
 role TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
PRAGMA user_version=13;
";

impl Store {
    pub fn invite_team_member(
        &self,
        actor: &str,
        workspace: &str,
        recipient: &str,
        role: WorkspaceRole,
    ) -> Result<String> {
        let transaction = self.connection.unchecked_transaction()?;
        let context = self.team_context(workspace, actor)?;
        context.authorize_workspace(workspace, Permission::ManageMembers)?;
        // Ownership transfers require the explicit CAS membership operation.
        if role == WorkspaceRole::Owner || self.find_user(recipient)?.is_none() {
            return Err(PolicyError::PermissionDenied.into());
        }
        let previous = self.team_membership(workspace, recipient)?;
        if previous.as_ref().is_some_and(|m| m.active) {
            return Err(StoreError::Conflict("Recipient is already a member".into()));
        }
        self.team_audit_capacity()?;
        transaction.execute(
            "DELETE FROM team_invitations WHERE expires_at<=?1",
            [now_secs()],
        )?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM team_invitations", [], |r| r.get(0))?;
        if count >= 1024 {
            return Err(StoreError::Limit("Invitation limit reached".into()));
        }
        let token = auth::random_token();
        transaction.execute(
            "INSERT INTO team_invitations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                auth::hash_token(&token),
                workspace,
                actor,
                recipient,
                context.membership.revision as i64,
                previous.map(|m| m.revision as i64),
                serde_json::to_string(&role)?,
                now_secs() + 86400
            ],
        )?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,'invitation_created',1,?4)",params![workspace,actor,recipient,now_secs()])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(token)
    }

    pub fn accept_team_invitation(&self, recipient: &str, token: &str) -> Result<Membership> {
        if token.len() > 256 {
            return Err(PolicyError::PermissionDenied.into());
        }
        let transaction = self.connection.unchecked_transaction()?;
        let invitation = transaction.query_row("SELECT workspace_id,issuer_id,issuer_revision,recipient_revision,role FROM team_invitations WHERE token_hash=?1 AND recipient_id=?2 AND expires_at>?3",params![auth::hash_token(token),recipient,now_secs()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,String>(4)?))).optional()?.ok_or(PolicyError::PermissionDenied)?;
        let (workspace, issuer, revision, previous, role) = invitation;
        let context = self.team_context(&workspace, &issuer)?;
        if context.membership.revision != revision as u64 {
            return Err(PolicyError::PermissionDenied.into());
        }
        let member = self.set_team_member_in_transaction(
            &issuer,
            &workspace,
            recipient,
            serde_json::from_str(&role)?,
            true,
            previous.map(|r| r as u64),
        )?;
        self.team_audit_capacity()?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?2,'invitation_accepted',?3,?4)",params![workspace,recipient,member.revision as i64,now_secs()])?;
        transaction.execute(
            "DELETE FROM team_invitations WHERE token_hash=?1",
            [auth::hash_token(token)],
        )?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(member)
    }

    pub fn revoke_team_invitations(
        &self,
        actor: &str,
        workspace: &str,
        recipient: &str,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageMembers)?;
        self.team_audit_capacity()?;
        transaction.execute(
            "DELETE FROM team_invitations WHERE workspace_id=?1 AND recipient_id=?2",
            params![workspace, recipient],
        )?;
        transaction.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,id,'invitations_revoked',1,?4 FROM users WHERE id=?3",params![workspace,actor,recipient,now_secs()])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitation_hash_and_authority_survive_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invitations.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let recipient = store.create_user("recipient", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let token = store
            .invite_team_member(
                &owner.id,
                &workspace.id,
                &recipient.id,
                WorkspaceRole::Member,
            )
            .unwrap();
        let hash: String = store
            .connection
            .query_row("SELECT token_hash FROM team_invitations", [], |r| r.get(0))
            .unwrap();
        assert_ne!(hash, token);
        assert_eq!(hash, auth::hash_token(&token));
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(
            store
                .accept_team_invitation(&recipient.id, &token)
                .unwrap()
                .active
        );
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
    }

    #[test]
    fn recipient_expiry_revocation_and_replay_are_enforced() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let recipient = store.create_user("recipient", "member").unwrap();
        let other = store.create_user("other", "member").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        let token = store
            .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Member)
            .unwrap();
        assert!(store.accept_team_invitation(&other.id, &token).is_err());
        assert!(
            store
                .team_membership(&w.id, &recipient.id)
                .unwrap()
                .is_none()
        );
        store
            .connection
            .execute("UPDATE team_invitations SET expires_at=0", [])
            .unwrap();
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
        let token = store
            .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Reviewer)
            .unwrap();
        store
            .revoke_team_invitations(&owner.id, &w.id, &recipient.id)
            .unwrap();
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
        let token = store
            .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Reviewer)
            .unwrap();
        assert_eq!(
            store
                .accept_team_invitation(&recipient.id, &token)
                .unwrap()
                .role,
            WorkspaceRole::Reviewer
        );
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
    }
    #[test]
    fn changed_authority_or_recipient_revision_invalidates_invitation() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let recipient = store.create_user("recipient", "member").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        assert!(
            store
                .invite_team_member(&recipient.id, &w.id, &owner.id, WorkspaceRole::Member)
                .is_err()
        );
        assert!(
            store
                .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Owner)
                .is_err()
        );
        let token = store
            .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Member)
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &recipient.id,
                WorkspaceRole::Viewer,
                false,
                None,
            )
            .unwrap();
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
        let token = store
            .invite_team_member(&owner.id, &w.id, &recipient.id, WorkspaceRole::Member)
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &owner.id,
                WorkspaceRole::Owner,
                true,
                Some(1),
            )
            .unwrap();
        assert!(store.accept_team_invitation(&recipient.id, &token).is_err());
    }
}
