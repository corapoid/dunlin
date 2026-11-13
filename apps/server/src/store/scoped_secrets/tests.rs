use super::*;
use crate::auth;
use crate::store::team_projects::PROJECT_PERMISSIONS;
use dunlin_domain::hosts::{HostCapabilities, HostDefinition};
use std::collections::BTreeSet;

pub(super) fn scope(store: &Store) -> Proof {
    let user = store.create_user("scoped-owner", "admin").unwrap();
    let workspace = store.create_workspace(&user, "Scoped").unwrap();
    let project = store
        .create_team_project(&user.id, &workspace.id, "Secrets")
        .unwrap();
    let definition:HostDefinition=serde_json::from_value(serde_json::json!({"name":"Worker","node_id":"scoped-worker","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["claude"]}})).unwrap();
    let host = store
        .create_host(&user.id, &workspace.id, definition)
        .unwrap();
    let capabilities:HostCapabilities=serde_json::from_value(serde_json::json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap();
    store
        .enroll_host(&host.enrollment.ticket, capabilities)
        .unwrap();
    store
        .approve_host(&user.id, &workspace.id, &host.host.id, 2)
        .unwrap();
    let (device, _) = store.create_device(&user.id, "browser").unwrap();
    let cookie = store.create_browser_session(&device.id, 600).unwrap();
    store
        .scoped_secret_proof(
            BrowserProof {
                user: user.id,
                device: device.id,
                session_hash: auth::hash_token(&cookie),
            },
            Binding {
                workspace: workspace.id,
                project: project.id,
                host: host.host.id,
            },
        )
        .unwrap()
}
pub(super) fn candidate(proof: &Proof, expected: u64) -> Candidate {
    Candidate {
        credential: "work-api".into(),
        binding: proof.binding.clone(),
        expected_revision: expected,
        reference: format!("scoped-secret:{}", dunlin_vault::protocol::new_token()),
        sha256: "a".repeat(64),
    }
}
fn count(store: &Store) -> (i64, i64, i64) {
    store.connection.query_row("SELECT count(*),coalesce(sum(write_ack),0),coalesce(sum(activated),0) FROM scoped_secret_references",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}

#[test]
fn reservation_ack_activation_and_unknown_references_survive_reopen_and_fence_second_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let proof = scope(&store);
    let c = candidate(&proof, 0);
    let claim = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    assert_eq!(count(&store), (1, 0, 0));
    assert!(matches!(
        store.scoped_secret_activate(&proof, claim, &|_| Ok(())),
        Err(Error::Conflict)
    ));
    let second = Store::open(&path).unwrap();
    assert!(matches!(
        second.scoped_secret_reserve(&proof, &candidate(&proof, 0), &|_| Ok(())),
        Err(Error::Conflict)
    ));
    drop(second);
    drop(store);
    let store = Store::open(&path).unwrap();
    let status = store.scoped_secret_list(&proof, &|_| Ok(())).unwrap();
    assert_eq!(status[0].state, "installing");
    assert_eq!(status[0].revision, 1);
    // An explicit new generation leaves the uncertain original intact.
    let newer = candidate(&proof, 1);
    let claim = store
        .scoped_secret_reserve(&proof, &newer, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &claim).unwrap();
    let version = store
        .scoped_secret_activate(&proof, claim, &|_| Ok(()))
        .unwrap();
    assert_eq!(version.revision, 2);
    assert_eq!(version.reference, newer.reference);
    assert_eq!(count(&store), (2, 1, 1));
    assert_eq!(
        store.scoped_secret_list(&proof, &|_| Ok(())).unwrap()[0].state,
        "active"
    );
}

#[test]
fn membership_and_project_grant_aba_block_activation_but_original_write_ack_is_retained() {
    for membership in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let proof = scope(&store);
        let c = candidate(&proof, 0);
        let claim = store
            .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
            .unwrap();
        if membership {
            store.connection.execute("UPDATE team_memberships SET revision=revision+2 WHERE workspace_id=?1 AND user_id=?2",params![proof.binding.workspace,proof.browser.user]).unwrap();
        } else {
            let permissions: BTreeSet<_> = PROJECT_PERMISSIONS.iter().copied().collect();
            store
                .set_team_project_grant(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &proof.browser.user,
                    BTreeSet::from([Permission::ViewProject, Permission::ManageProject]),
                    Some(proof.stamp.grant_revision),
                )
                .unwrap();
            store
                .set_team_project_grant(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &proof.browser.user,
                    permissions,
                    Some(proof.stamp.grant_revision + 1),
                )
                .unwrap();
        }
        store.scoped_secret_acknowledge(&proof, &claim).unwrap();
        assert!(matches!(
            store.scoped_secret_activate(&proof, claim, &|_| Ok(())),
            Err(Error::Denied)
        ));
        assert_eq!(count(&store), (1, 1, 0));
    }
}

#[test]
fn required_project_permission_revocation_keeps_ack_but_cannot_activate() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let c = candidate(&proof, 0);
    let claim = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    store
        .set_team_project_grant(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            &proof.browser.user,
            BTreeSet::new(),
            Some(proof.stamp.grant_revision),
        )
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &claim).unwrap();
    assert!(matches!(
        store.scoped_secret_activate(&proof, claim, &|_| Ok(())),
        Err(Error::Denied)
    ));
    assert_eq!(count(&store), (1, 1, 0));
}

#[test]
fn forged_writer_proof_cannot_ack_and_late_ack_cannot_reactivate_disabled_or_replaced_slot() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let first = candidate(&proof, 0);
    let old = store
        .scoped_secret_reserve(&proof, &first, &|_| Ok(()))
        .unwrap();
    let mut forged = proof.clone();
    forged.browser.session_hash = "different".into();
    assert!(matches!(
        store.scoped_secret_acknowledge(&forged, &old),
        Err(Error::Conflict)
    ));
    let status = store
        .scoped_secret_disable(&proof, &first.credential, 1, &|_| Ok(()))
        .unwrap();
    assert_eq!(status.revision, 2);
    let second = candidate(&proof, 2);
    let newer = store
        .scoped_secret_reserve(&proof, &second, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &newer).unwrap();
    store
        .scoped_secret_activate(&proof, newer, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &old).unwrap();
    assert!(matches!(
        store.scoped_secret_activate(&proof, old, &|_| Ok(())),
        Err(Error::Conflict)
    ));
    assert_eq!(count(&store), (2, 2, 1));
    let reference: String = store
        .connection
        .query_row("SELECT active_reference FROM scoped_credentials", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(reference, second.reference);
}

#[test]
fn browser_check_runs_inside_transaction_and_denial_or_audit_failure_rolls_back() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let c = candidate(&proof, 0);
    let denied = |store: &Store| {
        assert!(!store.connection.is_autocommit());
        Err(Error::Denied)
    };
    assert!(matches!(
        store.scoped_secret_reserve(&proof, &c, &denied),
        Err(Error::Denied)
    ));
    assert_eq!(count(&store), (0, 0, 0));
    store.connection.execute_batch("CREATE TRIGGER reject_scoped_audit BEFORE INSERT ON team_audit WHEN NEW.action='scoped_secret_reserved' BEGIN SELECT RAISE(ABORT,'fixture audit failure'); END;").unwrap();
    assert!(matches!(
        store.scoped_secret_reserve(&proof, &c, &|_| Ok(())),
        Err(Error::Storage)
    ));
    assert_eq!(count(&store), (0, 0, 0));
    assert!(
        store
            .scoped_secret_list(&proof, &|_| Ok(()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn capacity_refuses_new_write_without_pruning_uncertain_records_and_disable_still_works() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    for revision in 0..256 {
        store
            .scoped_secret_reserve(&proof, &candidate(&proof, revision), &|_| Ok(()))
            .unwrap();
    }
    assert!(matches!(
        store.scoped_secret_reserve(&proof, &candidate(&proof, 256), &|_| Ok(())),
        Err(Error::Limit)
    ));
    assert_eq!(count(&store), (256, 0, 0));
    assert_eq!(
        store
            .scoped_secret_disable(&proof, "work-api", 256, &|_| Ok(()))
            .unwrap()
            .state,
        "disabled"
    );
    assert_eq!(count(&store), (256, 0, 0));
}

#[test]
fn audit_exhaustion_or_rejected_event_cannot_block_disable_receipt() {
    for full in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let proof = scope(&store);
        store
            .scoped_secret_reserve(&proof, &candidate(&proof, 0), &|_| Ok(()))
            .unwrap();
        if full {
            store.connection.execute("WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100000) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'fixture_capacity',1,0 FROM rows LIMIT (100000-(SELECT count(*) FROM team_audit))",params![proof.binding.workspace,proof.browser.user]).unwrap();
        } else {
            store.connection.execute_batch("CREATE TRIGGER reject_disable_event BEFORE INSERT ON team_audit WHEN NEW.action='scoped_secret_disabled' BEGIN SELECT RAISE(ABORT,'fixture audit event failure'); END;").unwrap();
        }
        assert_eq!(
            store
                .scoped_secret_disable(&proof, "work-api", 1, &|_| Ok(()))
                .unwrap()
                .state,
            "disabled"
        );
        let receipt: String = store
            .connection
            .query_row("SELECT last_disable FROM scoped_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
        assert_eq!(receipt["actor"], proof.browser.user);
        assert_eq!(receipt["revision"], 2);
    }
}

#[test]
fn host_revision_and_status_are_pinned_and_no_value_or_policy_column_exists() {
    let store = Store::open_in_memory().unwrap();
    let proof = scope(&store);
    let c = candidate(&proof, 0);
    let claim = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE team_hosts SET revision=revision+1,status='revoked' WHERE id=?1",
            [&proof.binding.host],
        )
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &claim).unwrap();
    assert!(matches!(
        store.scoped_secret_activate(&proof, claim, &|_| Ok(())),
        Err(Error::Denied)
    ));
    for table in ["scoped_credentials", "scoped_secret_references"] {
        let mut stmt = store
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let names = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(!names.iter().any(|n| n == "value" || n == "policy"));
    }
}

#[test]
fn migration_47_preserves_existing_data_and_collision_rolls_back_new_schema_only() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let proof = scope(&store);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE scoped_secret_references; DROP TABLE scoped_credentials; PRAGMA user_version=47;").unwrap();
        if collision {
            store.connection.execute_batch("CREATE TABLE scoped_secret_references(marker TEXT);INSERT INTO scoped_secret_references VALUES('preserve me');").unwrap();
        }
        drop(store);
        let reopened = Store::open(&path);
        if collision {
            assert!(reopened.is_err());
            let raw = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                47
            );
            assert_eq!(
                raw.query_row("SELECT marker FROM scoped_secret_references", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "preserve me"
            );
            assert_eq!(
                raw.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='scoped_credentials'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        } else {
            let reopened = reopened.unwrap();
            assert!(reopened.find_user(&proof.browser.user).unwrap().is_some());
            assert_eq!(
                reopened
                    .connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                crate::store::SCHEMA_VERSION
            );
            assert!(
                reopened
                    .scoped_secret_list(&proof, &|_| Ok(()))
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
