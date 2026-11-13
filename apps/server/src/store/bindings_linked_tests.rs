//! Real local Git worktrees; no daemon, model account, network or worker proof.
use super::*;
use crate::store::local_metadata::MetadataPermission;
use dunlin_domain::collaboration::WorkspaceRole;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    owner: String,
    workspace: String,
    project: String,
    native: String,
    root: PathBuf,
    git: PathBuf,
    common: PathBuf,
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-C",
        ])
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repository");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init"]);
        git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
        let root = dir.path().join("checkout");
        git(
            &repo,
            &["worktree", "add", "-b", "linked", root.to_str().unwrap()],
        );
        let git_dir = PathBuf::from(git(&root, &["rev-parse", "--absolute-git-dir"]));
        let common = fs::canonicalize(repo.join(".git")).unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let owner = store.create_user("linked-binding-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "Linked bindings").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Logical")
            .unwrap();
        let native = store
            .create_project("Linked", root.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &owner.id).unwrap();
        let f = Self {
            dir,
            store,
            owner: owner.id,
            workspace: workspace.id,
            project: project.id,
            native: native.id,
            root,
            git: git_dir,
            common,
        };
        assert_eq!(f.approve(None), 1);
        f
    }
    fn approve(&self, expected: Option<u64>) -> u64 {
        let prepared = self
            .store
            .prepare_local_metadata(&self.native)
            .unwrap()
            .capture(&self.git, &self.common)
            .unwrap();
        self.store
            .commit_local_metadata(prepared, expected)
            .unwrap()
            .revision
    }
    fn read_grant(&self, user: &str, approval: u64, expected: Option<u64>) {
        let native = self.store.capture_native_grant(&self.native, user).unwrap();
        self.store
            .set_local_metadata_grant(
                &self.native,
                user,
                approval,
                native.revision,
                &[MetadataPermission::Read],
                true,
                expected,
                false,
            )
            .unwrap();
    }
    fn preparation(&self, expected: Option<u64>, approval: u64) -> Result<BindingPreparation> {
        self.store.prepare_local_binding(
            &self.owner,
            &self.workspace,
            &self.project,
            &self.native,
            expected,
            Some(approval),
        )
    }
    fn commit(&self, prepared: PreparedLocalBinding) -> Result<Binding> {
        self.store
            .commit_local_binding_authorized(prepared, |_, _| Ok::<_, StoreError>(()))
    }
    fn bind(&self, expected: Option<u64>, approval: u64) -> Binding {
        self.commit(
            self.preparation(expected, approval)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap()
    }
    fn member(&self) -> String {
        let user = self
            .store
            .create_user("linked-binding-member", "member")
            .unwrap();
        self.store
            .set_team_member(
                &self.owner,
                &self.workspace,
                &user.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        self.store
            .set_team_project_grant(
                &self.owner,
                &self.workspace,
                &self.project,
                &user.id,
                [Permission::ViewProject, Permission::CreateSession]
                    .into_iter()
                    .collect(),
                None,
            )
            .unwrap();
        self.store.grant(&self.native, &user.id).unwrap();
        user.id
    }
    fn resolve(&self, user: &str) -> Result<Option<(Binding, Project)>> {
        self.store.resolve_local_binding(
            user,
            &self.workspace,
            &self.project,
            Permission::CreateSession,
        )
    }
    fn audit_count(&self) -> i64 {
        self.store
            .connection
            .query_row("SELECT count(*) FROM team_audit", [], |row| row.get(0))
            .unwrap()
    }
    fn unchanged_failed_commit(&self, prepared: PreparedLocalBinding, before: &Binding) {
        let count = self.audit_count();
        let mut changes = self.store.subscribe_catalog();
        assert!(self.commit(prepared).is_err());
        assert_eq!(
            self.store
                .local_binding_row(&self.workspace, &self.project)
                .unwrap()
                .as_ref(),
            Some(before)
        );
        assert_eq!(self.audit_count(), count);
        assert!(matches!(
            changes.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}

#[test]
fn linked_binding_requires_explicit_approval_revision_and_administrators_own_metadata_read() {
    let f = Fixture::new();
    assert!(f.preparation(None, 1).is_err());
    f.read_grant(&f.owner, 1, None);
    assert!(
        f.store
            .set_local_binding(&f.owner, &f.workspace, &f.project, &f.native, None)
            .is_err()
    );
    assert!(f.preparation(None, 2).is_err());
    let binding = f.bind(None, 1);
    assert_eq!(binding.metadata_approval_revision, Some(1));
    assert_eq!(binding.repo_kind, "linked_worktree");
    let public = serde_json::to_value(&binding).unwrap();
    assert_eq!(public["metadata_approval_revision"], 1);
    assert!(public.get("root").is_none());
    assert!(public.get("identity").is_none());
    assert!(
        !serde_json::to_string(&binding)
            .unwrap()
            .contains(f.dir.path().to_str().unwrap())
    );
    let resolved = f.resolve(&f.owner).unwrap().unwrap();
    assert_eq!(
        resolved.1.path,
        fs::canonicalize(&f.root).unwrap().to_str().unwrap()
    );
}

#[test]
fn logical_member_must_supply_own_metadata_read_independently_of_binding_administrator() {
    let f = Fixture::new();
    f.read_grant(&f.owner, 1, None);
    f.bind(None, 1);
    let member = f.member();
    assert!(f.resolve(&member).is_err());
    f.read_grant(&member, 1, None);
    assert!(f.resolve(&member).unwrap().is_some());
    assert!(
        f.store
            .native_checkout_candidate(&f.native, &member, MetadataPermission::Write)
            .is_err()
    );
    f.store
        .revoke_local_metadata_grant(&f.native, &f.owner, 1)
        .unwrap();
    assert!(f.resolve(&f.owner).is_err());
    assert!(f.resolve(&member).unwrap().is_some());
}

#[test]
fn linked_binding_preparation_cannot_refresh_native_metadata_or_approval_epochs_at_commit() {
    for change in ["native", "metadata", "approval"] {
        let f = Fixture::new();
        f.read_grant(&f.owner, 1, None);
        let original = f.bind(None, 1);
        let prepared = f
            .preparation(Some(1), 1)
            .unwrap()
            .validate_filesystem()
            .unwrap();
        match change {
            "native" => {
                f.store.revoke_grant(&f.native, &f.owner).unwrap();
                f.store.grant(&f.native, &f.owner).unwrap();
                f.read_grant(&f.owner, 1, Some(1));
            }
            "metadata" => {
                f.store
                    .revoke_local_metadata_grant(&f.native, &f.owner, 1)
                    .unwrap();
                f.read_grant(&f.owner, 1, Some(2));
            }
            "approval" => {
                f.store.revoke_local_metadata(&f.native, 1).unwrap();
                assert_eq!(f.approve(Some(2)), 3);
                f.read_grant(&f.owner, 3, Some(1));
            }
            _ => unreachable!(),
        }
        f.unchanged_failed_commit(prepared, &original);
    }
}

#[test]
fn linked_preparation_rejects_checkout_inode_swap_during_filesystem_validation() {
    let f = Fixture::new();
    f.read_grant(&f.owner, 1, None);
    let preparation = f.preparation(None, 1).unwrap();
    fs::rename(&f.root, f.root.with_extension("retired")).unwrap();
    fs::create_dir(&f.root).unwrap();
    fs::write(
        f.root.join(".git"),
        format!("gitdir: {}\n", f.git.display()),
    )
    .unwrap();
    assert!(preparation.validate_filesystem().is_err());
    assert!(
        f.store
            .local_binding_row(&f.workspace, &f.project)
            .unwrap()
            .is_none()
    );
}

#[test]
fn late_identity_callback_rejection_rolls_back_binding_audit_and_its_own_sql_without_catalog_change()
 {
    let f = Fixture::new();
    f.read_grant(&f.owner, 1, None);
    let prepared = f
        .preparation(None, 1)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let count = f.audit_count();
    let mut changes = f.store.subscribe_catalog();
    let result = f
        .store
        .commit_local_binding_authorized(prepared, |store, _| {
            store.connection.execute(
                "UPDATE projects SET name='must roll back' WHERE id=?1",
                [&f.native],
            )?;
            Err::<(), StoreError>(StoreError::Conflict(
                "Late device revocation fixture".into(),
            ))
        });
    assert!(result.is_err());
    assert!(
        f.store
            .local_binding_row(&f.workspace, &f.project)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.audit_count(), count);
    assert_eq!(
        f.store
            .project_for_user(&f.native, &f.owner)
            .unwrap()
            .unwrap()
            .name,
        "Linked"
    );
    assert!(matches!(
        changes.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[test]
fn new_metadata_approval_requires_explicit_binding_cas_and_each_users_fresh_grant() {
    let f = Fixture::new();
    f.read_grant(&f.owner, 1, None);
    f.bind(None, 1);
    let member = f.member();
    f.read_grant(&member, 1, None);
    f.store.revoke_local_metadata(&f.native, 1).unwrap();
    assert_eq!(f.approve(Some(2)), 3);
    assert!(f.resolve(&f.owner).is_err());
    assert!(f.resolve(&member).is_err());
    f.read_grant(&f.owner, 3, Some(1));
    assert!(f.resolve(&f.owner).is_err());
    assert!(f.preparation(None, 3).is_err());
    assert_eq!(f.bind(Some(1), 3).revision, 2);
    assert!(f.resolve(&f.owner).unwrap().is_some());
    assert!(f.resolve(&member).is_err());
    f.read_grant(&member, 3, Some(1));
    assert!(f.resolve(&member).unwrap().is_some());
}

#[test]
fn ordinary_binding_without_metadata_pin_keeps_the_legacy_public_json_shape() {
    let f = Fixture::new();
    let root = f.dir.path().join("ordinary");
    fs::create_dir(&root).unwrap();
    let native = f
        .store
        .create_project("ordinary", root.to_str().unwrap())
        .unwrap();
    f.store.grant(&native.id, &f.owner).unwrap();
    let binding = f
        .store
        .set_local_binding(&f.owner, &f.workspace, &f.project, &native.id, None)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&binding).unwrap(),
        serde_json::json!({
            "workspace_id":f.workspace,"project_id":f.project,"native_project_id":native.id,
            "host_id":"local","repo_kind":"directory","settings_digest":f.store.binding_settings_digest(&native.id).unwrap(),
            "revision":1,"active":true,
        })
    );
}
