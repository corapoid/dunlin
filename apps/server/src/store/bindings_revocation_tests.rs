use super::*;
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    owner: String,
    workspace: String,
    project: String,
    native: String,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let owner = store.create_user("binding-owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "Bindings").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "Project")
            .unwrap();
        let native = store
            .create_project("Checkout", root.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &owner.id).unwrap();
        store
            .set_local_binding(&owner.id, &w.id, &p.id, &native.id, None)
            .unwrap();
        Self {
            dir,
            store,
            owner: owner.id,
            workspace: w.id,
            project: p.id,
            native: native.id,
            root,
        }
    }
    fn revoke(&self, expected: u64) -> Result<Binding> {
        self.store
            .revoke_local_binding(&self.owner, &self.workspace, &self.project, expected)
    }
    fn audit_count(&self) -> i64 {
        self.store
            .connection
            .query_row("SELECT count(*) FROM team_audit", [], |row| row.get(0))
            .unwrap()
    }
}
#[test]
fn ordinary_binding_revoke_remains_audited_without_checkout_or_ordinary_audit_capacity() {
    let f = Fixture::new();
    let count = f.audit_count();
    f.store.connection.execute("WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?3) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'binding_quota_fixture',1,0 FROM n",params![f.workspace,f.owner,100000-count]).unwrap();
    std::fs::remove_dir(&f.root).unwrap();
    let mut changed = f.store.subscribe_catalog();
    let revoked = f.revoke(1).unwrap();
    assert!(!revoked.active);
    assert_eq!(revoked.revision, 2);
    assert!(changed.try_recv().is_ok());
    assert_eq!(f.audit_count(), 100000);
    let receipt:(String,String,i64)=f.store.connection.query_row("SELECT native_project_id,actor_id,revision FROM team_local_binding_revocations WHERE workspace_id=?1 AND project_id=?2",params![f.workspace,f.project],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(receipt, (f.native.clone(), f.owner.clone(), 2));
    assert_eq!(f.revoke(2).unwrap().revision, 2);
    assert!(matches!(
        changed.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(f.audit_count(), 100000);
    assert!(matches!(f.revoke(1), Err(StoreError::Conflict(_))));
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    assert!(
        !reopened
            .local_binding_for_user(&f.owner, &f.workspace, &f.project)
            .unwrap()
            .unwrap()
            .active
    );
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT count(*) FROM team_local_binding_revocations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
#[test]
fn final_local_revision_is_reserved_for_denial_and_imported_max_active_binding_still_revokes() {
    for previous in [i64::MAX - 1, i64::MAX] {
        let f = Fixture::new();
        f.store.connection.execute("UPDATE team_local_bindings SET revision=?1 WHERE workspace_id=?2 AND project_id=?3",params![previous,f.workspace,f.project]).unwrap();
        assert!(matches!(
            f.store.set_local_binding(
                &f.owner,
                &f.workspace,
                &f.project,
                &f.native,
                Some(previous as u64)
            ),
            Err(StoreError::Limit(_))
        ));
        let revoked = f.revoke(previous as u64).unwrap();
        assert_eq!(revoked.revision, i64::MAX as u64);
        assert!(!revoked.active);
        assert_eq!(f.revoke(i64::MAX as u64).unwrap().revision, i64::MAX as u64);
        assert!(
            f.store
                .set_local_binding(
                    &f.owner,
                    &f.workspace,
                    &f.project,
                    &f.native,
                    Some(i64::MAX as u64)
                )
                .is_err()
        );
    }
}
#[test]
fn receipt_or_ordinary_audit_sql_failure_rolls_back_local_revocation_without_notification() {
    for table in ["team_local_binding_revocations", "team_audit"] {
        let f = Fixture::new();
        let before = f.audit_count();
        let condition = if table == "team_audit" {
            "WHEN NEW.action='local_binding_revoked'"
        } else {
            ""
        };
        f.store.connection.execute_batch(&format!("CREATE TRIGGER fail_binding_audit BEFORE INSERT ON {table} {condition} BEGIN SELECT RAISE(ABORT,'binding receipt unavailable'); END;")).unwrap();
        let mut changes = f.store.subscribe_catalog();
        assert!(matches!(f.revoke(1), Err(StoreError::Sqlite(_))));
        let current = f
            .store
            .local_binding_for_user(&f.owner, &f.workspace, &f.project)
            .unwrap()
            .unwrap();
        assert!(current.active);
        assert_eq!(current.revision, 1);
        assert_eq!(f.audit_count(), before);
        assert_eq!(
            f.store
                .connection
                .query_row(
                    "SELECT count(*) FROM team_local_binding_revocations",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert!(matches!(
            changes.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}
#[test]
fn independent_connections_revoke_once_and_old_cas_cannot_revoke_a_rebound_target() {
    let f = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let path = f.dir.path().join("relay.db");
        let owner = f.owner.clone();
        let workspace = f.workspace.clone();
        let project = f.project.clone();
        let barrier = barrier.clone();
        jobs.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            barrier.wait();
            match store.revoke_local_binding(&owner, &workspace, &project, 1) {
                Ok(binding) => {
                    assert_eq!(binding.revision, 2);
                    true
                }
                Err(StoreError::Conflict(_)) => false,
                Err(error) => panic!("Unexpected revocation failure: {error}"),
            }
        }));
    }
    let left = jobs.remove(0).join().unwrap();
    let right = jobs.remove(0).join().unwrap();
    assert_ne!(left, right);
    let rebound = f
        .store
        .set_local_binding(&f.owner, &f.workspace, &f.project, &f.native, Some(2))
        .unwrap();
    assert_eq!(rebound.revision, 3);
    assert!(rebound.active);
    assert!(matches!(f.revoke(1), Err(StoreError::Conflict(_))));
    assert!(matches!(f.revoke(2), Err(StoreError::Conflict(_))));
    assert_eq!(
        f.store
            .local_binding_for_user(&f.owner, &f.workspace, &f.project)
            .unwrap()
            .unwrap()
            .revision,
        3
    );
}
