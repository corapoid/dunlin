use super::*;
use std::{fs, path::PathBuf};
const SETTINGS: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn scope() -> ExportScope {
    ExportScope {
        controller_node_id: "controller-one".into(),
        worker_node_id: "worker-one".into(),
        workspace_id: "workspace-one".into(),
        host_id: "host-one".into(),
        enrollment_epoch: 1,
        credential_revision: 1,
    }
}
fn principal() -> ExportPrincipal {
    ExportPrincipal {
        kind: PrincipalKind::Human,
        id: "controller-user-one".into(),
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    state: PathBuf,
    scope: ExportScope,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        fs::create_dir(&root).unwrap();
        let state = dir.path().join("state");
        Self {
            dir,
            root,
            state,
            scope: scope(),
        }
    }
    fn store(&self) -> WorkerStore {
        WorkerStore::open(&self.state, self.scope.clone()).unwrap()
    }
    fn capture(&self) -> CapturedCheckout {
        CapturedCheckout::capture(&self.root, SETTINGS, None).unwrap()
    }
}
fn authorize(
    store: &WorkerStore,
    scope: &ExportScope,
    id: &str,
    p: &ExportPrincipal,
) -> Result<ExportAuthority> {
    store.authorize_validated(
        scope,
        store
            .candidate(scope, id, p, ExportPermission::Read)?
            .validate_filesystem()?,
    )
}
fn git(path: &Path, args: &[&str]) {
    let result = std::process::Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "user.name=Local fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(result.status.success(), "Local Git fixture failed");
}
#[test]
fn durable_scope_and_explicit_principal_kind_survive_reopen_without_implicit_os_authority() {
    let f = Fixture::new();
    let store = f.store();
    let export = store
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    assert!(
        store
            .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
            .is_err()
    );
    let granted = store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    assert_eq!(granted.revision, 1);
    let authority = authorize(&store, &f.scope, &export.id, &p).unwrap();
    let service = ExportPrincipal {
        kind: PrincipalKind::Service,
        id: p.id.clone(),
    };
    assert!(
        store
            .candidate(&f.scope, &export.id, &service, ExportPermission::Read)
            .is_err()
    );
    let public = serde_json::to_string(&authority.export_stamp()).unwrap();
    assert!(!public.contains(f.root.to_str().unwrap()));
    assert!(!public.contains("git_dir"));
    drop(store);
    let store = f.store();
    store.revalidate_authority(&f.scope, &authority).unwrap();
    authority.validate_filesystem().unwrap();
    for changed in [
        ExportScope {
            enrollment_epoch: 2,
            ..f.scope.clone()
        },
        ExportScope {
            credential_revision: 2,
            ..f.scope.clone()
        },
        ExportScope {
            controller_node_id: "other-controller".into(),
            ..f.scope.clone()
        },
        ExportScope {
            worker_node_id: "other-worker".into(),
            ..f.scope.clone()
        },
        ExportScope {
            workspace_id: "other-workspace".into(),
            ..f.scope.clone()
        },
    ] {
        assert!(WorkerStore::open(&f.state, changed.clone()).is_err());
        assert!(store.list(&changed).is_err());
        assert!(store.revalidate_authority(&changed, &authority).is_err());
    }
    let mut same_node = f.scope.clone();
    same_node.worker_node_id = same_node.controller_node_id.clone();
    assert!(same_node.validate().is_err());
}
#[test]
fn grant_aba_and_rebind_invalidate_prior_captures_and_require_explicit_regrant() {
    let f = Fixture::new();
    let store = f.store();
    let export = store
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    let grant = store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    let authority = authorize(&store, &f.scope, &export.id, &p).unwrap();
    let validated = store
        .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let revoked = store
        .set_grant(&f.scope, &export.id, &p, &[], false, Some(grant.revision))
        .unwrap();
    let regranted = store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            Some(revoked.revision),
        )
        .unwrap();
    assert!(regranted.revision > grant.revision);
    assert!(store.revalidate_authority(&f.scope, &authority).is_err());
    assert!(store.authorize_validated(&f.scope, validated).is_err());
    let old = authorize(&store, &f.scope, &export.id, &p).unwrap();
    let changed = store
        .rebind_captured(
            &f.scope,
            &export.id,
            export.revision,
            "Renamed",
            &f.capture(),
        )
        .unwrap();
    assert!(store.revalidate_authority(&f.scope, &old).is_err());
    assert!(
        store
            .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
            .is_err()
    );
    let tombstone = store.grants(&f.scope, &export.id).unwrap().remove(0);
    assert!(!tombstone.active);
    store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            Some(tombstone.revision),
        )
        .unwrap();
    let current = authorize(&store, &f.scope, &export.id, &p).unwrap();
    assert_eq!(current.export_stamp().export.revision, changed.revision);
    assert!(
        store
            .set_grant(
                &f.scope,
                &export.id,
                &p,
                &[ExportPermission::Read],
                true,
                Some(grant.revision)
            )
            .is_err()
    );
}
#[test]
fn fresh_filesystem_validation_detects_replacement_and_revoke_needs_neither_filesystem_nor_audit_room()
 {
    let f = Fixture::new();
    let store = f.store();
    let export = store
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    let authority = authorize(&store, &f.scope, &export.id, &p).unwrap();
    fs::rename(&f.root, f.dir.path().join("old-checkout")).unwrap();
    fs::create_dir(&f.root).unwrap();
    assert!(authority.validate_filesystem().is_err());
    assert!(
        store
            .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
            .unwrap()
            .validate_filesystem()
            .is_err()
    );
    fs::remove_dir(&f.root).unwrap();
    store.connection.execute_batch("WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO export_audit(export_id,principal_kind,principal_id,action,revision,at) SELECT 'fixture','export','','quota',1,0 FROM n;").unwrap();
    let revoked = store
        .revoke_export(&f.scope, &export.id, export.revision)
        .unwrap();
    assert!(!revoked.active);
    let again = store
        .revoke_export(&f.scope, &export.id, revoked.revision)
        .unwrap();
    assert_eq!(again, revoked);
    assert!(store.revalidate_authority(&f.scope, &authority).is_err());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM export_revocations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        store
            .rebind_captured(
                &f.scope,
                &export.id,
                revoked.revision,
                "New",
                &f.capture_for_new_root()
            )
            .is_err()
    );
}
impl Fixture {
    fn capture_for_new_root(&self) -> CapturedCheckout {
        fs::create_dir_all(&self.root).unwrap();
        self.capture()
    }
}
#[test]
fn late_receipt_failure_rolls_back_export_and_grant_revocation_atomically() {
    let f = Fixture::new();
    let store = f.store();
    let export = store
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    let authority = authorize(&store, &f.scope, &export.id, &p).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON export_revocations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        store
            .revoke_export(&f.scope, &export.id, export.revision)
            .is_err()
    );
    store.revalidate_authority(&f.scope, &authority).unwrap();
    assert_eq!(store.list(&f.scope).unwrap()[0], export);
    assert!(store.grants(&f.scope, &export.id).unwrap()[0].active);
}
#[test]
fn final_revision_is_reserved_for_terminal_revocation_even_for_imported_max_active_records() {
    let f = Fixture::new();
    let store = f.store();
    let export = store
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE worker_exports SET revision=?1 WHERE id=?2",
            params![sql_revision(MAX_REVISION - 1).unwrap(), export.id],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE export_grants SET revision=?1,export_revision=?2 WHERE export_id=?3",
            params![
                sql_revision(MAX_REVISION - 1).unwrap(),
                sql_revision(MAX_REVISION - 1).unwrap(),
                export.id
            ],
        )
        .unwrap();
    let revoked = store
        .revoke_export(&f.scope, &export.id, MAX_REVISION - 1)
        .unwrap();
    assert_eq!(revoked.revision, MAX_REVISION);
    assert_eq!(
        store.grants(&f.scope, &export.id).unwrap()[0].revision,
        MAX_REVISION
    );
    assert!(
        store
            .rebind_captured(&f.scope, &export.id, MAX_REVISION, "new", &f.capture())
            .is_err()
    );
    store
        .connection
        .execute(
            "UPDATE worker_exports SET revision=?1,active=1 WHERE id=?2",
            params![sql_revision(MAX_REVISION).unwrap(), export.id],
        )
        .unwrap();
    assert!(
        !store
            .revoke_export(&f.scope, &export.id, MAX_REVISION)
            .unwrap()
            .active
    );
    let other = store
        .register_captured(&f.scope, "Other", &f.capture())
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &other.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE export_grants SET revision=?1 WHERE export_id=?2",
            params![sql_revision(MAX_REVISION).unwrap(), other.id],
        )
        .unwrap();
    assert!(
        !store
            .set_grant(&f.scope, &other.id, &p, &[], false, Some(MAX_REVISION))
            .unwrap()
            .active
    );
    assert!(
        store
            .set_grant(
                &f.scope,
                &other.id,
                &p,
                &[ExportPermission::Read],
                true,
                Some(MAX_REVISION)
            )
            .is_err()
    );
}
#[test]
fn normal_git_objects_and_explicit_linked_worktree_roots_are_pinned_without_hidden_expansion() {
    let f = Fixture::new();
    git(&f.root, &["init", "-q"]);
    git(&f.root, &["commit", "--allow-empty", "-qm", "fixture"]);
    let ordinary = f.capture();
    assert_eq!(ordinary.repo_kind(), RepoKind::Git);
    let linked = f.dir.path().join("linked");
    git(
        &f.root,
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    assert!(CapturedCheckout::capture(&linked, SETTINGS, None).is_err());
    let common = f.root.join(".git");
    let pointer = fs::read_to_string(linked.join(".git")).unwrap();
    let git_dir = PathBuf::from(pointer.trim().strip_prefix("gitdir: ").unwrap());
    let roots = LinkedRoots {
        git_dir: git_dir.clone(),
        common_dir: common.clone(),
    };
    let capture = CapturedCheckout::capture(&linked, SETTINGS, Some(&roots)).unwrap();
    assert_eq!(capture.repo_kind(), RepoKind::LinkedWorktree);
    capture.revalidate().unwrap();
    let wrong = LinkedRoots {
        git_dir: git_dir.clone(),
        common_dir: f.root.clone(),
    };
    assert!(CapturedCheckout::capture(&linked, SETTINGS, Some(&wrong)).is_err());
    fs::write(
        git_dir.join("gitdir"),
        f.root.join(".git").to_str().unwrap(),
    )
    .unwrap();
    assert!(capture.revalidate().is_err());
    fs::rename(common.join("objects"), common.join("old-objects")).unwrap();
    fs::create_dir(common.join("objects")).unwrap();
    assert!(ordinary.revalidate().is_err());
    #[cfg(unix)]
    {
        fs::remove_dir(common.join("objects")).unwrap();
        std::os::unix::fs::symlink(common.join("old-objects"), common.join("objects")).unwrap();
        assert!(CapturedCheckout::capture(&f.root, SETTINGS, None).is_err());
    }
}
#[cfg(unix)]
#[test]
fn private_state_permissions_and_symlink_or_fifo_metadata_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let store = f.store();
    drop(store);
    assert_eq!(
        fs::metadata(&f.state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(f.state.join("exports.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let link = f.dir.path().join("state-link");
    symlink(&f.state, &link).unwrap();
    assert!(WorkerStore::open(&link, f.scope.clone()).is_err());
    let public = f.dir.path().join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(WorkerStore::open(&public, f.scope.clone()).is_err());
    assert_eq!(
        fs::metadata(&public).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let marker = f.root.join(".git");
    let c = std::ffi::CString::new(marker.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(CapturedCheckout::capture(&f.root, SETTINGS, None).is_err());
}
#[tokio::test]
async fn cli_rejects_unknown_or_duplicate_flags_without_network_or_config_fallback() {
    for arguments in [
        vec!["list", "--unknown", "value"],
        vec!["list", "--config", "a", "--config", "b"],
        vec!["register", "--config"],
        vec!["unknown"],
    ] {
        assert!(
            command(
                &arguments
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            )
            .await
            .is_err()
        );
    }
}

#[test]
fn two_connection_grant_cas_race_and_cli_aba_cannot_revive_daemon_captures() {
    let f = Fixture::new();
    let daemon = f.store();
    let export = daemon
        .register_captured(&f.scope, "Fixture", &f.capture())
        .unwrap();
    let p = principal();
    daemon
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    let old = authorize(&daemon, &f.scope, &export.id, &p).unwrap();
    let validated = daemon
        .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = f.state.clone();
        let scope = f.scope.clone();
        let id = export.id.clone();
        let p = p.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let cli = WorkerStore::open(&path, scope.clone()).unwrap();
            barrier.wait();
            match cli.set_grant(
                &scope,
                &id,
                &p,
                &[ExportPermission::Read, ExportPermission::Execute],
                true,
                Some(1),
            ) {
                Ok(grant) => (true, grant.revision),
                Err(Error::Conflict(_)) => (false, 0),
                Err(_) => panic!("Unexpected concurrent grant failure"),
            }
        }));
    }
    let first = threads.remove(0).join().unwrap();
    let second = threads.remove(0).join().unwrap();
    assert_ne!(first.0, second.0);
    assert_eq!(first.1.max(second.1), 2);
    assert!(daemon.revalidate_authority(&f.scope, &old).is_err());
    assert!(daemon.authorize_validated(&f.scope, validated).is_err());
    let current = authorize(&daemon, &f.scope, &export.id, &p).unwrap();
    let validated = daemon
        .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let cli = f.store();
    let revoked = cli
        .set_grant(&f.scope, &export.id, &p, &[], false, Some(2))
        .unwrap();
    let regranted = cli
        .set_grant(
            &f.scope,
            &export.id,
            &p,
            &[ExportPermission::Read],
            true,
            Some(revoked.revision),
        )
        .unwrap();
    assert_eq!(regranted.revision, 4);
    assert!(daemon.revalidate_authority(&f.scope, &current).is_err());
    assert!(daemon.authorize_validated(&f.scope, validated).is_err());
    let current = authorize(&daemon, &f.scope, &export.id, &p).unwrap();
    let validated = daemon
        .candidate(&f.scope, &export.id, &p, ExportPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let changed = CapturedCheckout::capture(&f.root, &"a".repeat(64), None).unwrap();
    cli.rebind_captured(
        &f.scope,
        &export.id,
        export.revision,
        "New approved policy",
        &changed,
    )
    .unwrap();
    assert!(daemon.revalidate_authority(&f.scope, &current).is_err());
    assert!(daemon.authorize_validated(&f.scope, validated).is_err());
    assert!(!daemon.grants(&f.scope, &export.id).unwrap()[0].active);
}

#[cfg(unix)]
#[test]
fn metadata_open_never_creates_or_initializes_an_operator_catalog() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    assert!(WorkerStore::open_existing(&f.state, f.scope.clone()).is_err());
    assert!(!f.state.exists());
    fs::create_dir(&f.state).unwrap();
    fs::set_permissions(&f.state, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(WorkerStore::open_existing(&f.state, f.scope.clone()).is_err());
    let database = f.state.join("exports.sqlite3");
    assert!(!database.exists());
    fs::write(&database, []).unwrap();
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(WorkerStore::open_existing(&f.state, f.scope.clone()).is_err());
    assert_eq!(fs::metadata(&database).unwrap().len(), 0);
    let db = Connection::open(&database).unwrap();
    let tables: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    drop(db);
    let store = f.store();
    assert!(store.list(&f.scope).unwrap().is_empty());
    drop(store);
    assert!(WorkerStore::open_existing(&f.state, f.scope.clone()).is_ok());
}
