use super::super::tests::{candidate, scope};
use super::*;
fn install(store: &Store, proof: &Proof, expected: u64) -> Version {
    let c = candidate(proof, expected);
    let claim = store.scoped_secret_reserve(proof, &c, &|_| Ok(())).unwrap();
    store.scoped_secret_acknowledge(proof, &claim).unwrap();
    store
        .scoped_secret_activate(proof, claim, &|_| Ok(()))
        .unwrap()
}
fn clean(store: &Store, proof: &Proof, expected: u64) -> usize {
    let claims = store
        .scoped_secret_cleanup_claims(proof, "work-api", expected, &|_| Ok(()))
        .unwrap();
    let n = claims.len();
    for claim in claims {
        store
            .scoped_secret_cleanup_check(proof, &claim, &|_| Ok(()))
            .unwrap();
        store.scoped_secret_cleanup_ack(proof, &claim).unwrap();
    }
    n
}
#[test]
fn cleanup_releases_quota_beyond_256_rotations_and_bounds_history_without_forgetting_unknown_writer()
 {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let c = candidate(&proof, 0);
    let unknown = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    for expected in 1..620 {
        install(&store, &proof, expected);
        if expected % 16 == 0 {
            clean(&store, &proof, expected + 1);
        }
    }
    while clean(&store, &proof, 620) > 0 {}
    let status = store
        .scoped_secret_retention(&proof, "work-api", &|_| Ok(()))
        .unwrap();
    assert_eq!(status.pending_writes, 1);
    assert_eq!(status.retained, 2);
    assert_eq!(status.deleted_history, 512);
    assert!(
        store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scoped_secret_references WHERE reference=?1)",
                [&unknown.candidate().reference],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    store.scoped_secret_acknowledge(&proof, &unknown).unwrap();
    assert_eq!(clean(&store, &proof, 620), 1);
    assert_eq!(
        store
            .scoped_secret_retention(&proof, "work-api", &|_| Ok(()))
            .unwrap()
            .retained,
        1
    );
    assert!(
        store
            .scoped_secret_activate(&proof, unknown, &|_| Ok(()))
            .is_err()
    );
}
#[test]
fn current_and_staged_are_protected_until_disable_and_old_activation_cannot_revive_cleaned_slot() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let active = install(&store, &proof, 0);
    let c = candidate(&proof, 1);
    let staged = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &staged).unwrap();
    assert_eq!(clean(&store, &proof, 2), 0);
    store
        .scoped_secret_disable(&proof, "work-api", 2, &|_| Ok(()))
        .unwrap();
    assert_eq!(clean(&store, &proof, 3), 2);
    assert!(
        store
            .scoped_secret_activate(&proof, staged, &|_| Ok(()))
            .is_err()
    );
    assert_eq!(store.connection.query_row("SELECT count(*) FROM scoped_secret_references WHERE cleanup_state='deleted' AND cleanup_receipt IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(
        store
            .connection
            .query_row(
                "SELECT active_reference IS NULL FROM scoped_credentials",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    let newer = install(&store, &proof, 3);
    assert_ne!(active.reference, newer.reference);
}
#[test]
fn cleanup_intent_and_fair_cursor_survive_restart_without_advancing_unconfirmed_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let proof = scope(&store);
    for rev in 0..36 {
        install(&store, &proof, rev);
    }
    let first = store
        .scoped_secret_cleanup_claims(&proof, "work-api", 36, &|_| Ok(()))
        .unwrap();
    assert_eq!(first.len(), 16);
    let reference = first[0].version().reference.clone();
    let nonce = first[0].nonce().to_string();
    drop(store);
    let store = Store::open(&path).unwrap();
    let second = store
        .scoped_secret_cleanup_claims(&proof, "work-api", 36, &|_| Ok(()))
        .unwrap();
    assert_eq!(second.len(), 16);
    assert!(second.iter().all(|c| {
        !first
            .iter()
            .any(|f| f.version().reference == c.version().reference)
    }));
    assert_eq!(
        store
            .scoped_secret_retention(&proof, "work-api", &|_| Ok(()))
            .unwrap()
            .retained,
        36
    );
    let third = store
        .scoped_secret_cleanup_claims(&proof, "work-api", 36, &|_| Ok(()))
        .unwrap();
    let retry = third
        .iter()
        .find(|c| c.version().reference == reference)
        .unwrap();
    assert_eq!(retry.nonce(), nonce);
    assert_eq!(
        store
            .scoped_secret_retention(&proof, "work-api", &|_| Ok(()))
            .unwrap()
            .deleted_history,
        0
    );
}
#[test]
fn revoked_manager_cannot_start_delete_but_factual_ack_survives_revocation_and_full_audit() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    install(&store, &proof, 0);
    install(&store, &proof, 1);
    store.connection.execute("WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100000) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'fixture_capacity',1,0 FROM rows LIMIT (100000-(SELECT count(*) FROM team_audit))",params![proof.binding.workspace,proof.browser.user]).unwrap();
    assert!(matches!(
        store.team_audit_capacity(),
        Err(crate::store::StoreError::Limit(_))
    ));
    let claim = store
        .scoped_secret_cleanup_claims(&proof, "work-api", 2, &|_| Ok(()))
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(
        store.scoped_secret_cleanup_claims(&proof, "work-api", 1, &|_| Ok(())),
        Err(Error::Conflict)
    ));
    assert!(
        store
            .scoped_secret_cleanup_check(&proof, &claim, &|_| Err(Error::Denied))
            .is_err()
    );
    store
        .connection
        .execute(
            "UPDATE team_memberships SET revision=revision+1 WHERE workspace_id=?1 AND user_id=?2",
            params![proof.binding.workspace, proof.browser.user],
        )
        .unwrap();
    assert!(
        store
            .scoped_secret_cleanup_check(&proof, &claim, &|_| Ok(()))
            .is_err()
    );
    store.scoped_secret_cleanup_ack(&proof, &claim).unwrap();
    assert!(store.scoped_secret_cleanup_ack(&proof, &claim).is_err());
}
#[test]
fn migration50_preserves_write_facts_and_rolls_back_late_column_collision() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let proof = scope(&store);
        let v = install(&store, &proof, 0);
        super::super::local::fixture_v49(&store).unwrap();
        let tx = store.connection.unchecked_transaction().unwrap();
        super::super::local::migrate(&tx).unwrap();
        tx.execute_batch("PRAGMA user_version=50;").unwrap();
        tx.commit().unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE scoped_secret_references ADD COLUMN cleanup_receipt TEXT;UPDATE scoped_secret_references SET cleanup_receipt='foreign-marker';").unwrap();
        }
        drop(store);
        let result = Store::open(&path);
        if collision {
            assert!(result.is_err());
            let db = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                50
            );
            assert_eq!(
                db.query_row(
                    "SELECT cleanup_receipt FROM scoped_secret_references",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "foreign-marker"
            );
            assert!(
                db.prepare("SELECT cleanup_cursor_time FROM scoped_credentials")
                    .is_err()
            );
        } else {
            let store = result.unwrap();
            let status = store
                .scoped_secret_retention(&proof, "work-api", &|_| Ok(()))
                .unwrap();
            assert_eq!(status.retained, 1);
            assert_eq!(status.eligible, 0);
            assert_eq!(store.connection.query_row("SELECT reference FROM scoped_secret_references WHERE write_ack=1 AND activated=1",[],|r|r.get::<_,String>(0)).unwrap(),v.reference);
        }
    }
}
