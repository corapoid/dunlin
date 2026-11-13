use super::*;
use std::fs;
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    project: String,
    user: String,
    root: std::path::PathBuf,
    git: std::path::PathBuf,
    common: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        let common = dir.path().join("repo/.git");
        let git = common.join("worktrees/linked");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&git).unwrap();
        fs::create_dir_all(common.join("objects")).unwrap();
        fs::write(root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
        fs::write(git.join("commondir"), "../..\n").unwrap();
        fs::write(
            git.join("gitdir"),
            format!("{}\n", root.join(".git").display()),
        )
        .unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let user = store.create_user("metadata-user", "admin").unwrap();
        let project = store
            .create_project("linked", root.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        Self {
            dir,
            store,
            project: project.id,
            user: user.id,
            root,
            git,
            common,
        }
    }
    fn prepare(&self) -> PreparedMetadata {
        self.store
            .prepare_local_metadata(&self.project)
            .unwrap()
            .capture(&self.git, &self.common)
            .unwrap()
    }
    fn approve(&self) -> MetadataApproval {
        self.store
            .commit_local_metadata(self.prepare(), None)
            .unwrap()
    }
    fn grant(&self, approval: u64, expected: Option<u64>) -> MetadataGrant {
        let native = self
            .store
            .capture_native_grant(&self.project, &self.user)
            .unwrap();
        self.store
            .set_local_metadata_grant(
                &self.project,
                &self.user,
                approval,
                native.revision,
                &[MetadataPermission::Read],
                true,
                expected,
                false,
            )
            .unwrap()
    }
    fn authority(&self) -> MetadataAuthority {
        self.store
            .authorize_local_metadata(
                self.store
                    .local_metadata_candidate(&self.project, &self.user, MetadataPermission::Read)
                    .unwrap()
                    .validate_filesystem()
                    .unwrap(),
            )
            .unwrap()
    }
    fn peer(&self) -> Store {
        Store::open(self.dir.path().join("relay.db")).unwrap()
    }
}
#[test]
fn durable_approval_and_user_grant_never_manufacture_native_or_write_authority() {
    let f = Fixture::new();
    let approval = f.approve();
    assert_eq!(approval.revision, 1);
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    let user = f.store.create_user("ungranted", "member").unwrap();
    assert!(
        f.store
            .set_local_metadata_grant(
                &f.project,
                &user.id,
                1,
                1,
                &[MetadataPermission::Read],
                true,
                None,
                false
            )
            .is_err()
    );
    f.grant(1, None);
    let a = f.authority();
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Write)
            .is_err()
    );
    assert!(
        f.store
            .set_local_metadata_grant(
                &f.project,
                &f.user,
                1,
                1,
                &[MetadataPermission::Read, MetadataPermission::Write],
                true,
                Some(1),
                false
            )
            .is_err()
    );
    let rw = f
        .store
        .set_local_metadata_grant(
            &f.project,
            &f.user,
            1,
            1,
            &[MetadataPermission::Read, MetadataPermission::Write],
            true,
            Some(1),
            true,
        )
        .unwrap();
    assert_eq!(rw.revision, 2);
    assert!(f.store.revalidate_local_metadata(&a).is_err());
    let peer = f.peer();
    assert_eq!(peer.local_metadata(&f.project).unwrap().unwrap(), approval);
    assert_eq!(peer.local_metadata_grants(&f.project).unwrap(), vec![rw]);
    let json = serde_json::to_string(&f.authority().summary()).unwrap();
    assert!(!json.contains(f.dir.path().to_str().unwrap()));
}
#[test]
fn independent_revoke_regrant_native_epoch_and_approval_aba_never_revive_candidates() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    let old = f.authority();
    let validated = f
        .store
        .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let peer = f.peer();
    peer.revoke_local_metadata_grant(&f.project, &f.user, 1)
        .unwrap();
    assert!(f.store.authorize_local_metadata(validated).is_err());
    assert!(f.store.revalidate_local_metadata(&old).is_err());
    f.grant(1, Some(2));
    assert!(f.store.revalidate_local_metadata(&old).is_err());
    let a = f.authority();
    peer.revoke_grant(&f.project, &f.user).unwrap();
    peer.grant(&f.project, &f.user).unwrap();
    assert!(f.store.revalidate_local_metadata(&a).is_err());
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    f.grant(1, Some(3));
    let a = f.authority();
    peer.revoke_local_metadata(&f.project, 1).unwrap();
    f.store.commit_local_metadata(f.prepare(), Some(2)).unwrap();
    assert!(f.store.revalidate_local_metadata(&a).is_err());
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    f.grant(3, Some(4));
    assert_eq!(f.authority().summary().approval.revision, 3);
}
#[test]
fn preparation_pins_path_settings_and_filesystem_identity_without_database_locks() {
    let f = Fixture::new();
    let prepared = f.prepare();
    f.store
        .save_project_settings(&f.project, "{\"changed\":true}")
        .unwrap();
    assert!(f.store.commit_local_metadata(prepared, None).is_err());
    let prepared = f.prepare();
    f.store
        .connection
        .execute(
            "UPDATE projects SET path=?2 WHERE id=?1",
            params![f.project, f.dir.path().join("other").to_str().unwrap()],
        )
        .unwrap();
    assert!(f.store.commit_local_metadata(prepared, None).is_err());
    f.store
        .connection
        .execute(
            "UPDATE projects SET path=?2 WHERE id=?1",
            params![f.project, f.root.to_str().unwrap()],
        )
        .unwrap();
    f.approve();
    f.grant(1, None);
    let a = f.authority();
    fs::rename(f.common.join("objects"), f.common.join("old-objects")).unwrap();
    fs::create_dir(f.common.join("objects")).unwrap();
    assert!(a.validate_filesystem().is_err());
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .unwrap()
            .validate_filesystem()
            .is_err()
    );
}
#[test]
fn audit_full_missing_filesystem_and_terminal_epochs_still_allow_durable_revoke() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    f.store.connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO native_metadata_audit(project_id,subject,action,revision,at) SELECT (SELECT project_id FROM native_metadata_approvals LIMIT 1),'approval','fixture',1,0 FROM n;").unwrap();
    assert!(f.store.commit_local_metadata(f.prepare(), Some(1)).is_err());
    assert_eq!(
        f.store
            .local_metadata(&f.project)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    fs::rename(&f.root, f.dir.path().join("missing-root")).unwrap();
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_grants SET revision=?1",
            [sql(MAX).unwrap()],
        )
        .unwrap();
    let g = f
        .store
        .revoke_local_metadata_grant(&f.project, &f.user, MAX)
        .unwrap();
    assert_eq!(g.revision, MAX);
    assert!(!g.active);
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_approvals SET revision=?1",
            [sql(MAX).unwrap()],
        )
        .unwrap();
    let a = f.store.revoke_local_metadata(&f.project, MAX).unwrap();
    assert_eq!(a.revision, MAX);
    assert!(!a.active);
    let receipts: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM native_metadata_revocations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 2);
    let peer = f.peer();
    assert!(!peer.local_metadata(&f.project).unwrap().unwrap().active);
}
#[test]
fn last_receipt_failure_rolls_back_revoke_and_cas_race_has_one_winner() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    f.store.connection.execute_batch("CREATE TRIGGER deny_receipt BEFORE INSERT ON native_metadata_revocations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f.store.revoke_local_metadata(&f.project, 1).is_err());
    assert!(f.store.local_metadata(&f.project).unwrap().unwrap().active);
    f.store
        .connection
        .execute_batch("DROP TRIGGER deny_receipt;")
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = f.dir.path().join("relay.db");
        let p = f.project.clone();
        let u = f.user.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let s = Store::open(path).unwrap();
            barrier.wait();
            s.revoke_local_metadata_grant(&p, &u, 1)
        }));
    }
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(StoreError::Conflict(_))))
    );
}

#[test]
fn corrupt_capture_never_becomes_authority_and_cannot_block_revocation() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_approvals SET identity=?1",
            ["b".repeat(64)],
        )
        .unwrap();
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_approvals SET checkout='not JSON'",
            [],
        )
        .unwrap();
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    assert!(
        !f.store
            .revoke_local_metadata_grant(&f.project, &f.user, 1)
            .unwrap()
            .active
    );
    assert!(!f.store.revoke_local_metadata(&f.project, 1).unwrap().active);
    assert!(!f.store.revoke_local_metadata(&f.project, 2).unwrap().active);
}
#[test]
fn terminal_epoch_is_reserved_and_cannot_recreate_positive_authority() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_grants SET revision=?1",
            [sql(MAX - 1).unwrap()],
        )
        .unwrap();
    assert!(
        f.store
            .set_local_metadata_grant(
                &f.project,
                &f.user,
                1,
                1,
                &[MetadataPermission::Read, MetadataPermission::Write],
                true,
                Some(MAX - 1),
                true
            )
            .is_err()
    );
    assert_eq!(
        f.store
            .revoke_local_metadata_grant(&f.project, &f.user, MAX - 1)
            .unwrap()
            .revision,
        MAX
    );
    assert!(
        f.store
            .set_local_metadata_grant(
                &f.project,
                &f.user,
                1,
                1,
                &[MetadataPermission::Read],
                true,
                Some(MAX),
                false
            )
            .is_err()
    );
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_approvals SET revision=?1",
            [sql(MAX - 1).unwrap()],
        )
        .unwrap();
    assert!(
        f.store
            .commit_local_metadata(f.prepare(), Some(MAX - 1))
            .is_err()
    );
    assert_eq!(
        f.store
            .revoke_local_metadata(&f.project, MAX - 1)
            .unwrap()
            .revision,
        MAX
    );
    assert!(
        f.store
            .commit_local_metadata(f.prepare(), Some(MAX))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn native_project_symlink_retarget_never_reuses_old_canonical_checkout_approval() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let alias = f.dir.path().join("alias");
    symlink(&f.root, &alias).unwrap();
    f.store
        .connection
        .execute(
            "UPDATE projects SET path=?2 WHERE id=?1",
            params![f.project, alias.to_str().unwrap()],
        )
        .unwrap();
    f.approve();
    f.grant(1, None);
    let authority = f.authority();
    let candidate = f
        .store
        .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
        .unwrap();
    let other = f.dir.path().join("different");
    fs::create_dir(&other).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&other, &alias).unwrap();
    assert!(candidate.validate_filesystem().is_err());
    assert!(authority.validate_filesystem().is_err());
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .unwrap()
            .validate_filesystem()
            .is_err()
    );
    // The formerly approved canonical checkout is still intact; validating only
    // that path would incorrectly accept the alias now naming another checkout.
    assert!(
        CapturedCheckout::capture(
            &f.root,
            &authority.summary().approval.settings_digest,
            Some(&LinkedRoots {
                git_dir: f.git.clone(),
                common_dir: f.common.clone()
            })
        )
        .unwrap()
        .revalidate()
        .is_ok()
    );
}
#[test]
fn ordinary_identity_revalidation_stays_ordinary_and_rejects_a_different_original_path() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    fs::create_dir(&one).unwrap();
    fs::create_dir(&two).unwrap();
    let captured = CapturedCheckout::capture(&one, &"a".repeat(64), None).unwrap();
    captured.revalidate().unwrap();
    captured.revalidate_at(&one).unwrap();
    assert!(captured.revalidate_at(&two).is_err());
}

#[test]
fn malformed_grant_permissions_fail_closed_but_ungrant_needs_no_permission_decoder() {
    let f = Fixture::new();
    f.approve();
    f.grant(1, None);
    f.store
        .connection
        .execute(
            "UPDATE native_metadata_grants SET permissions='not JSON'",
            [],
        )
        .unwrap();
    assert!(
        f.store
            .local_metadata_candidate(&f.project, &f.user, MetadataPermission::Read)
            .is_err()
    );
    let revoked = f
        .store
        .revoke_local_metadata_grant(&f.project, &f.user, 1)
        .unwrap();
    assert!(!revoked.active);
    assert!(revoked.permissions.is_empty());
    assert_eq!(revoked.revision, 2);
    let before: i64 = f
        .store
        .connection
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let noop = f
        .store
        .revoke_local_metadata_grant(&f.project, &f.user, 2)
        .unwrap();
    assert_eq!(noop, revoked);
    assert_eq!(
        before,
        f.store
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        f.store.local_metadata_grants(&f.project).unwrap(),
        vec![revoked]
    );
    let receipt: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM native_metadata_revocations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipt, 1);
}
