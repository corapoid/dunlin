//! Explicit local checkout bindings. Logical grants never manufacture native
//! filesystem grants. Remote hosts and linked worktrees need separate adapters.
use super::{Project, Result, Store, StoreError, now_secs};
use dunlin_domain::collaboration::Permission;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(super) const MIGRATION: &str = "
CREATE TABLE team_local_bindings (
 workspace_id TEXT NOT NULL, project_id TEXT NOT NULL,
 native_project_id TEXT NOT NULL REFERENCES projects(id),
 root TEXT NOT NULL, identity TEXT NOT NULL, repo_kind TEXT NOT NULL,
 settings_digest TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision>0), active INTEGER NOT NULL CHECK(active IN (0,1)),
 PRIMARY KEY(workspace_id,project_id),
 FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id)
);
PRAGMA user_version=17;
";
/// One latest terminal receipt per local binding. Positive writes still require
/// ordinary audit capacity, so revocation can remain available at that limit.
pub(crate) const REVOCATION_SCHEMA:&str="
CREATE TABLE team_local_binding_revocations (
 workspace_id TEXT NOT NULL,project_id TEXT NOT NULL,native_project_id TEXT NOT NULL REFERENCES projects(id),
 actor_id TEXT NOT NULL REFERENCES users(id),revision INTEGER NOT NULL CHECK(revision>0),created_at INTEGER NOT NULL,
 PRIMARY KEY(workspace_id,project_id),
 FOREIGN KEY(workspace_id,project_id) REFERENCES team_local_bindings(workspace_id,project_id)
);
";
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Binding {
    pub workspace_id: String,
    pub project_id: String,
    pub native_project_id: String,
    pub host_id: &'static str,
    pub repo_kind: String,
    pub settings_digest: String,
    pub revision: u64,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_approval_revision: Option<u64>,
    #[serde(skip_serializing)]
    root: String,
    #[serde(skip_serializing)]
    identity: String,
}
impl Binding {
    /// Captured native authority is independent of the logical binding. The
    /// caller must perform its fresh SQL cut after this filesystem validation.
    pub(crate) fn validate_native_checkout(
        &self,
        native: &Project,
        checkout: &super::native_checkout::NativeCheckoutAuthority,
    ) -> Result<String> {
        checkout.validate_filesystem()?;
        let stamp = checkout.stamp();
        if native.id != self.native_project_id
            || stamp.native_grant.project_id != native.id
            || stamp.settings_digest != self.settings_digest
        {
            return Err(StoreError::Conflict(
                "Checkout binding authority changed".into(),
            ));
        }
        if let Some(revision) = self.metadata_approval_revision {
            let metadata = stamp.metadata.as_ref().ok_or_else(denied)?;
            if stamp.repo_kind != crate::checkout_identity::RepoKind::LinkedWorktree
                || metadata.approval.revision != revision
                || metadata.approval.project_id != self.native_project_id
                || stamp.checkout_identity_digest != self.identity
                || self.repo_kind != "linked_worktree"
                || checkout.root() != self.root
            {
                return Err(StoreError::Conflict(
                    "Linked checkout binding changed; rebind explicitly".into(),
                ));
            }
            Ok(checkout.root().into())
        } else {
            if stamp.repo_kind == crate::checkout_identity::RepoKind::LinkedWorktree {
                return Err(denied());
            }
            self.validate_checkout(native)
        }
    }
    /// Filesystem validation may block; invoke outside Store's database lock.
    pub(crate) fn validate_checkout(&self, native: &Project) -> Result<String> {
        if self.metadata_approval_revision.is_some() {
            return Err(denied());
        }
        let (root, identity, kind) = capture(&native.path)?;
        if root != self.root || identity != self.identity || kind != self.repo_kind {
            return Err(StoreError::Conflict(
                "Checkout identity changed; rebind explicitly".into(),
            ));
        }
        Ok(root)
    }
}
fn denied() -> StoreError {
    dunlin_domain::collaboration::PolicyError::PermissionDenied.into()
}
fn capture(path: &str) -> Result<(String, String, String)> {
    let root = std::fs::canonicalize(path)
        .map_err(|_| StoreError::Limit("Checkout root is unavailable".into()))?;
    if !root.is_dir() {
        return Err(StoreError::Limit(
            "Checkout root must be a directory".into(),
        ));
    }
    let root_text = root
        .to_str()
        .ok_or_else(|| StoreError::Limit("Checkout path must be UTF-8".into()))?
        .to_string();
    let marker = root.join(".git");
    let git = match std::fs::symlink_metadata(&marker) {
        Ok(_) => {
            let git = std::fs::canonicalize(marker)
                .map_err(|_| StoreError::Limit("Git metadata is unavailable".into()))?;
            if !git.starts_with(&root) || !git.is_dir() {
                return Err(StoreError::Limit("External/linked-worktree Git metadata needs a separate explicit binding adapter".into()));
            }
            if git.join("commondir").symlink_metadata().is_ok() {
                return Err(StoreError::Limit(
                    "Shared Git metadata needs a separate explicit binding adapter".into(),
                ));
            }
            Some(git)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            for parent in root.ancestors().skip(1) {
                match parent.join(".git").symlink_metadata() {
                    Ok(_) => {
                        return Err(StoreError::Limit(
                            "Bind the repository root, not a nested directory".into(),
                        ));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(StoreError::Limit(
                            "Parent repository metadata is unavailable".into(),
                        ));
                    }
                }
            }
            None
        }
        Err(_) => return Err(StoreError::Limit("Git metadata is unavailable".into())),
    };
    let mut digest = Sha256::new();
    pin_directory(&mut digest, &root)?;
    if let Some(ref git) = git {
        pin_directory(&mut digest, git)?;
    }
    let kind = if git.is_some() { "git" } else { "directory" }.to_string();
    digest.update(kind.as_bytes());
    Ok((root_text, hex::encode(digest.finalize()), kind))
}
#[cfg(unix)]
fn pin_directory(digest: &mut Sha256, path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path
        .metadata()
        .map_err(|_| StoreError::Limit("Checkout metadata is unavailable".into()))?;
    digest.update((path.as_os_str().as_encoded_bytes().len() as u64).to_le_bytes());
    digest.update(path.as_os_str().as_encoded_bytes());
    digest.update(metadata.dev().to_le_bytes());
    digest.update(metadata.ino().to_le_bytes());
    Ok(())
}
#[cfg(not(unix))]
fn pin_directory(_: &mut Sha256, _: &std::path::Path) -> Result<()> {
    Err(StoreError::Limit(
        "Local execution bindings require macOS or Linux".into(),
    ))
}
pub(crate) struct BindingPreparation {
    actor: String,
    workspace: String,
    project: String,
    native: Project,
    previous: Option<Binding>,
    expected: Option<u64>,
    settings: String,
    authority: dunlin_domain::collaboration::AuthorityStamp,
    grant: super::NativeGrantStamp,
    metadata_revision: Option<u64>,
    linked: Option<super::native_checkout::NativeCheckoutCandidate>,
}
pub(crate) struct PreparedLocalBinding {
    preparation: BindingPreparation,
    ordinary: Option<(String, String, String)>,
    linked: Option<super::native_checkout::ValidatedNativeCheckout>,
}
impl BindingPreparation {
    pub(crate) fn validate_filesystem(mut self) -> Result<PreparedLocalBinding> {
        let linked = self
            .linked
            .take()
            .map(|c| c.validate_filesystem())
            .transpose()?;
        let ordinary = if linked.is_none() {
            Some(capture(&self.native.path)?)
        } else {
            None
        };
        Ok(PreparedLocalBinding {
            preparation: self,
            ordinary,
            linked,
        })
    }
}
impl Store {
    fn local_binding_row(&self, workspace: &str, project: &str) -> Result<Option<Binding>> {
        Ok(self.connection.query_row("SELECT native_project_id,root,identity,repo_kind,settings_digest,revision,active,metadata_approval_revision FROM team_local_bindings WHERE workspace_id=?1 AND project_id=?2",params![workspace,project],|r|Ok(Binding{workspace_id:workspace.into(),project_id:project.into(),native_project_id:r.get(0)?,host_id:"local",root:r.get(1)?,identity:r.get(2)?,repo_kind:r.get(3)?,settings_digest:r.get(4)?,revision:super::positive_revision(r,5)?,active:r.get(6)?,metadata_approval_revision:r.get::<_,Option<i64>>(7)?.map(|v|u64::try_from(v).map_err(|_|rusqlite::Error::InvalidQuery)).transpose()?})).optional()?)
    }
    fn binding_admin(&self, actor: &str, workspace: &str, project: &str) -> Result<()> {
        if !self.find_user(actor)?.is_some_and(|u| u.role == "admin") {
            return Err(denied());
        }
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageHosts)?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        Ok(())
    }
    fn binding_settings_digest(&self, native: &str) -> Result<String> {
        Ok(super::auth::hash_token(
            &self
                .project_settings(native)?
                .unwrap_or_else(|| "{}".into()),
        ))
    }
    #[cfg(test)]
    pub fn set_local_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        native: &str,
        expected_revision: Option<u64>,
    ) -> Result<Binding> {
        let prepared = self
            .prepare_local_binding(actor, workspace, project, native, expected_revision, None)?
            .validate_filesystem()?;
        self.commit_local_binding_authorized(prepared, |_, _| Ok::<_, StoreError>(()))
    }
    #[cfg(test)]
    pub(crate) fn prepare_local_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        native: &str,
        expected_revision: Option<u64>,
        metadata_revision: Option<u64>,
    ) -> Result<BindingPreparation> {
        let tx = self.connection.unchecked_transaction()?;
        let result = self.prepare_local_binding_in_transaction(
            actor,
            workspace,
            project,
            native,
            expected_revision,
            metadata_revision,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub(crate) fn prepare_local_binding_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        native: &str,
        expected_revision: Option<u64>,
        metadata_revision: Option<u64>,
    ) -> Result<BindingPreparation> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict("Binding transaction required".into()));
        }
        self.binding_admin(actor, workspace, project)?;
        let authority =
            self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        let previous = self.local_binding_row(workspace, project)?;
        if previous.as_ref().map(|b| b.revision) != expected_revision {
            return Err(StoreError::Conflict(
                "Checkout binding changed; reload before rebinding".into(),
            ));
        }
        let native = self.project_for_user(native, actor)?.ok_or_else(denied)?;
        let grant = self.capture_native_grant(&native.id, actor)?;
        let settings = self.binding_settings_digest(&native.id)?;
        let linked = if let Some(revision) = metadata_revision {
            if revision == 0 || revision > 9_007_199_254_740_991 {
                return Err(StoreError::Limit(
                    "Invalid metadata approval revision".into(),
                ));
            }
            self.local_metadata(&native.id)?
                .filter(|a| a.active && a.revision == revision)
                .ok_or_else(denied)?;
            Some(self.native_checkout_candidate_in_transaction(
                &native.id,
                actor,
                super::local_metadata::MetadataPermission::Read,
            )?)
        } else {
            None
        };
        Ok(BindingPreparation {
            actor: actor.into(),
            workspace: workspace.into(),
            project: project.into(),
            native,
            previous,
            expected: expected_revision,
            settings,
            authority,
            grant,
            metadata_revision,
            linked,
        })
    }
    pub(crate) fn commit_local_binding_authorized<E>(
        &self,
        prepared: PreparedLocalBinding,
        authorize: impl FnOnce(&Self, &str) -> std::result::Result<(), E>,
    ) -> std::result::Result<Binding, E>
    where
        E: From<StoreError>,
    {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(StoreError::from)?;
        let p = prepared.preparation;
        authorize(self, &p.actor)?;
        self.binding_admin(&p.actor, &p.workspace, &p.project)?;
        if self.authorize_team_project(
            &p.actor,
            &p.workspace,
            &p.project,
            Permission::ManageProject,
        )? != p.authority
            || self.local_binding_row(&p.workspace, &p.project)? != p.previous
            || !self
                .project_for_user(&p.native.id, &p.actor)?
                .is_some_and(|n| n.path == p.native.path)
            || self.binding_settings_digest(&p.native.id)? != p.settings
        {
            return Err(
                StoreError::Conflict("Binding preparation authority changed".into()).into(),
            );
        }
        self.revalidate_native_grant(&p.grant)?;
        let (root, identity, kind) = if let Some(linked) = prepared.linked {
            let a = self.authorize_native_checkout_in_transaction(linked)?;
            let stamp = a.stamp();
            if stamp.native_grant != p.grant
                || stamp.metadata.as_ref().map(|m| m.approval.revision) != p.metadata_revision
                || stamp.repo_kind != crate::checkout_identity::RepoKind::LinkedWorktree
            {
                return Err(denied().into());
            }
            (
                a.root().to_owned(),
                stamp.checkout_identity_digest,
                "linked_worktree".to_owned(),
            )
        } else {
            prepared.ordinary.ok_or_else(denied)?
        };
        let revision = p
            .expected
            .unwrap_or(0)
            .checked_add(1)
            .filter(|v| *v < i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Binding revision limit reached".into()))?;
        self.team_audit_capacity()?;
        self.connection.execute("INSERT INTO team_local_bindings(workspace_id,project_id,native_project_id,root,identity,repo_kind,settings_digest,revision,active,metadata_approval_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9) ON CONFLICT(workspace_id,project_id) DO UPDATE SET native_project_id=excluded.native_project_id,root=excluded.root,identity=excluded.identity,repo_kind=excluded.repo_kind,settings_digest=excluded.settings_digest,revision=excluded.revision,active=1,metadata_approval_revision=excluded.metadata_approval_revision",params![p.workspace,p.project,p.native.id,root,identity,kind,p.settings,revision as i64,p.metadata_revision.map(|v|v as i64)]).map_err(StoreError::from)?;
        self.connection.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,?3,'local_binding_set',?4,?5)",params![p.workspace,p.project,p.actor,revision as i64,now_secs()]).map_err(StoreError::from)?;
        let result = self
            .local_binding_row(&p.workspace, &p.project)?
            .ok_or_else(denied)?;
        tx.commit().map_err(StoreError::from)?;
        self.catalog_changed();
        Ok(result)
    }
    pub fn local_binding_for_user(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Option<Binding>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        let binding = self.local_binding_row(workspace, project)?;
        if let Some(ref binding) = binding {
            self.project_for_user(&binding.native_project_id, actor)?
                .ok_or_else(denied)?;
        }
        Ok(binding)
    }
    /// Caller must hold Store serialization and check device/host-account policy.
    /// This does not start a process or grant additional sandbox roots.
    pub fn local_binding_candidate(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        permission: Permission,
    ) -> Result<Option<(Binding, Project)>> {
        self.authorize_team_project(actor, workspace, project, permission)?;
        let Some(binding) = self.local_binding_row(workspace, project)? else {
            return Ok(None);
        };
        if !binding.active {
            return Err(StoreError::Conflict("Checkout binding was revoked".into()));
        }
        let native = self
            .project_for_user(&binding.native_project_id, actor)?
            .ok_or_else(denied)?;
        if let Some(revision) = binding.metadata_approval_revision {
            self.local_metadata(&native.id)?
                .filter(|a| {
                    a.active
                        && a.revision == revision
                        && a.checkout_identity_digest == binding.identity
                        && a.settings_digest == binding.settings_digest
                })
                .ok_or_else(denied)?;
        }
        if self.binding_settings_digest(&native.id)? != binding.settings_digest {
            return Err(StoreError::Conflict(
                "Checkout settings changed; rebind explicitly".into(),
            ));
        }
        Ok(Some((binding, native)))
    }
    #[cfg(test)]
    pub fn resolve_local_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        permission: Permission,
    ) -> Result<Option<(Binding, Project)>> {
        let Some((binding, mut native)) =
            self.local_binding_candidate(actor, workspace, project, permission)?
        else {
            return Ok(None);
        };
        native.path = if binding.metadata_approval_revision.is_some() {
            let validated = self
                .native_checkout_candidate(
                    &native.id,
                    actor,
                    super::local_metadata::MetadataPermission::Read,
                )?
                .validate_filesystem()?;
            let authority = self.authorize_native_checkout(validated)?;
            binding.validate_native_checkout(&native, &authority)?
        } else {
            binding.validate_checkout(&native)?
        };
        Ok(Some((binding, native)))
    }
    #[cfg(test)]
    pub fn revoke_local_binding(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        expected_revision: u64,
    ) -> Result<Binding> {
        self.revoke_local_binding_authorized(actor, workspace, project, expected_revision, |_| {
            Ok::<_, StoreError>(())
        })
    }
    pub(crate) fn revoke_local_binding_authorized<E>(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        expected_revision: u64,
        authorize: impl FnOnce(&Self) -> std::result::Result<(), E>,
    ) -> std::result::Result<Binding, E>
    where
        E: From<StoreError>,
    {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(StoreError::from)?;
        authorize(self)?;
        self.binding_admin(actor, workspace, project)?;
        let mut binding = self
            .local_binding_row(workspace, project)?
            .ok_or_else(denied)?;
        if binding.revision != expected_revision {
            return Err(StoreError::Conflict(
                "Checkout binding changed; reload before revoking".into(),
            )
            .into());
        }
        if !binding.active {
            tx.commit().map_err(StoreError::from)?;
            return Ok(binding);
        }
        // Even a legacy active record already at MAX must lose authority.
        binding.revision = expected_revision.saturating_add(1).min(i64::MAX as u64);
        binding.active = false;
        tx.execute("UPDATE team_local_bindings SET active=0,revision=?3 WHERE workspace_id=?1 AND project_id=?2",params![workspace,project,binding.revision as i64]).map_err(StoreError::from)?;
        tx.execute("INSERT INTO team_local_binding_revocations(workspace_id,project_id,native_project_id,actor_id,revision,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,project_id) DO UPDATE SET native_project_id=excluded.native_project_id,actor_id=excluded.actor_id,revision=excluded.revision,created_at=excluded.created_at",params![workspace,project,binding.native_project_id,actor,binding.revision as i64,now_secs()]).map_err(StoreError::from)?;
        match self.team_audit_capacity() {
            Ok(()) => {
                tx.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,?3,'local_binding_revoked',?4,?5)",params![workspace,project,actor,binding.revision as i64,now_secs()]).map_err(StoreError::from)?;
            }
            Err(StoreError::Limit(message))
                if message == "Team audit limit reached; administrator maintenance required" => {}
            Err(error) => return Err(error.into()),
        }
        tx.commit().map_err(StoreError::from)?;
        self.catalog_changed();
        Ok(binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::collaboration::WorkspaceRole;
    fn member(store: &Store, owner: &str, workspace: &str, project: &str) -> String {
        let user = store.create_user("member", "member").unwrap();
        store
            .set_team_member(
                owner,
                workspace,
                &user.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                owner,
                workspace,
                project,
                &user.id,
                [Permission::ViewProject, Permission::CreateSession]
                    .into_iter()
                    .collect(),
                None,
            )
            .unwrap();
        user.id
    }
    #[test]
    fn logical_membership_binding_and_native_grant_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "logical")
            .unwrap();
        let native = store
            .create_project("checkout", dir.path().to_str().unwrap())
            .unwrap();
        let user = member(&store, &owner.id, &w.id, &p.id);
        assert!(
            store
                .set_local_binding(&owner.id, &w.id, &p.id, &native.id, None)
                .is_err()
        );
        store.grant(&native.id, &owner.id).unwrap();
        let binding = store
            .set_local_binding(&owner.id, &w.id, &p.id, &native.id, None)
            .unwrap();
        assert_eq!(binding.revision, 1);
        assert!(store.project_for_user(&native.id, &user).unwrap().is_none());
        assert!(
            store
                .resolve_local_binding(&user, &w.id, &p.id, Permission::CreateSession)
                .is_err()
        );
        store.grant(&native.id, &user).unwrap();
        assert!(
            store
                .resolve_local_binding(&user, &w.id, &p.id, Permission::CreateSession)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .set_local_binding(&user, &w.id, &p.id, &native.id, Some(1))
                .is_err()
        );
        store.revoke_grant(&native.id, &user).unwrap();
        assert!(
            store
                .resolve_local_binding(&user, &w.id, &p.id, Permission::CreateSession)
                .is_err()
        );
        store.grant(&native.id, &user).unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &user,
                WorkspaceRole::Member,
                false,
                Some(1),
            )
            .unwrap();
        assert!(store.local_binding_for_user(&user, &w.id, &p.id).is_err());
    }
    #[test]
    fn root_settings_revision_and_revocation_are_checked_again_before_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let path = dir.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "team").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "logical")
            .unwrap();
        let native = store
            .create_project("checkout", root.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &owner.id).unwrap();
        store
            .set_local_binding(&owner.id, &w.id, &p.id, &native.id, None)
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(
            store
                .resolve_local_binding(&owner.id, &w.id, &p.id, Permission::CreateSession)
                .unwrap()
                .is_some()
        );
        std::fs::rename(&root, dir.path().join("old")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(
            store
                .resolve_local_binding(&owner.id, &w.id, &p.id, Permission::CreateSession)
                .is_err()
        );
        assert!(matches!(
            store.set_local_binding(&owner.id, &w.id, &p.id, &native.id, None),
            Err(StoreError::Conflict(_))
        ));
        store
            .set_local_binding(&owner.id, &w.id, &p.id, &native.id, Some(1))
            .unwrap();
        store
            .save_project_settings(&native.id, "{\"test\":true}")
            .unwrap();
        assert!(
            store
                .resolve_local_binding(&owner.id, &w.id, &p.id, Permission::CreateSession)
                .is_err()
        );
        store
            .set_local_binding(&owner.id, &w.id, &p.id, &native.id, Some(2))
            .unwrap();
        let revoked = store
            .revoke_local_binding(&owner.id, &w.id, &p.id, 3)
            .unwrap();
        assert!(!revoked.active);
        assert_eq!(revoked.revision, 4);
        assert!(
            store
                .resolve_local_binding(&owner.id, &w.id, &p.id, Permission::CreateSession)
                .is_err()
        );
    }
    #[test]
    fn git_marker_and_external_metadata_changes_do_not_silently_retarget_a_binding() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let (_, first, kind) = capture(root.to_str().unwrap()).unwrap();
        assert_eq!(kind, "directory");
        std::fs::create_dir(root.join(".git")).unwrap();
        let (_, git, kind) = capture(root.to_str().unwrap()).unwrap();
        assert_eq!(kind, "git");
        assert_ne!(first, git);
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(capture(root.to_str().unwrap()).unwrap().1, git);
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(capture(nested.to_str().unwrap()).is_err());
        std::fs::write(root.join(".git/commondir"), "../../outside").unwrap();
        assert!(capture(root.to_str().unwrap()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_retarget_changes_the_pinned_checkout_identity() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one");
        let two = dir.path().join("two");
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&two).unwrap();
        let link = dir.path().join("checkout");
        symlink(&one, &link).unwrap();
        let first = capture(link.to_str().unwrap()).unwrap();
        std::fs::remove_file(&link).unwrap();
        symlink(&two, &link).unwrap();
        let second = capture(link.to_str().unwrap()).unwrap();
        assert_ne!(first.0, second.0);
        assert_ne!(first.1, second.1);
    }
}

#[cfg(test)]
#[path = "bindings_revocation_tests.rs"]
mod revocation_tests;

#[cfg(test)]
#[path = "bindings_linked_tests.rs"]
mod linked_tests;
