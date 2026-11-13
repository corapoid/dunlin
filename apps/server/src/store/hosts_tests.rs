use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use serde_json::json;
fn definition(index: usize) -> HostDefinition {
    serde_json::from_value(json!({"name":format!("Worker {index}"),"node_id":format!("node-{index}"),"serve_origin":format!("https://worker{index}.tail123.ts.net"),"capability_policy":{"max_parallel":2,"harnesses":["claude"]}})).unwrap()
}
fn capabilities() -> HostCapabilities {
    serde_json::from_value(json!({"protocol_version":1,"version":"fixture-v1","os":"linux","architecture":"aarch64","harnesses":["claude","codex"],"sandbox":"linux_bubblewrap","max_parallel":8})).unwrap()
}
fn scope(store: &Store) -> (String, String) {
    let owner = store.create_user("host-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Workers").unwrap();
    (owner.id, workspace.id)
}
#[test]
fn tickets_and_credentials_are_distinct_hashed_single_use_and_survive_reopen_without_execution_authority()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w) = scope(&store);
    let created = store.create_host(&owner, &w, definition(1)).unwrap();
    assert_eq!(created.host.status, HostStatus::Pending);
    assert_eq!(created.host.capability_revision, 0);
    assert!(!created.host.execution_ready);
    let hash: String = store
        .connection
        .query_row(
            "SELECT ticket_hash FROM team_hosts WHERE id=?1",
            [&created.host.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash, auth::hash_token(&created.enrollment.ticket));
    assert_ne!(hash, created.enrollment.ticket);
    assert!(store.approve_host(&owner, &w, &created.host.id, 1).is_err());
    let credential = store
        .enroll_host(&created.enrollment.ticket, capabilities())
        .unwrap();
    assert!(credential.credential.starts_with("dhw_"));
    assert!(
        store
            .enroll_host(&created.enrollment.ticket, capabilities())
            .is_err()
    );
    assert!(
        store
            .report_host(
                &created.enrollment.ticket,
                &credential.host_id,
                1,
                1,
                capabilities()
            )
            .is_err()
    );
    let claimed = store
        .host(&owner, &w, &credential.host_id)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.status, HostStatus::Claimed);
    assert_eq!(claimed.revision, 2);
    assert_eq!(claimed.capability_revision, 1);
    let approved = store
        .approve_host(&owner, &w, &credential.host_id, 2)
        .unwrap();
    assert_eq!(approved.status, HostStatus::Approved);
    assert_eq!(approved.network_verification, "unverified");
    assert!(!approved.execution_ready);
    let report = store
        .report_host(
            &credential.credential,
            &credential.host_id,
            1,
            1,
            capabilities(),
        )
        .unwrap();
    assert_eq!(
        report.revision, approved.revision,
        "A heartbeat cannot invalidate administrative CAS"
    );
    assert_eq!(report.capability_revision, 2);
    assert_eq!(
        report.capabilities.unwrap().max_parallel,
        8,
        "Reported capabilities are untrusted metadata, not policy grants"
    );
    assert_eq!(
        store
            .drain_host(&owner, &w, &credential.host_id, approved.revision, true)
            .unwrap()
            .revision,
        4
    );
    assert!(
        store
            .drain_host(&owner, &w, &credential.host_id, approved.revision, false)
            .is_err()
    );
    let public = serde_json::to_string(&store.hosts(&owner, &w).unwrap()).unwrap();
    let audit = serde_json::to_string(&store.team_audit(&owner, &w).unwrap()).unwrap();
    for secret in [&credential.credential, &created.enrollment.ticket, &hash] {
        assert!(!public.contains(secret));
        assert!(!audit.contains(secret));
    }
    for event in store
        .team_audit(&owner, &w)
        .unwrap()
        .iter()
        .filter(|event| event.action.starts_with("host_"))
    {
        assert_eq!(
            event.resource_id.as_deref(),
            Some(credential.host_id.as_str())
        );
        assert_eq!(event.project_id, None);
    }
    drop(store);
    let store = Store::open(path).unwrap();
    let report = store
        .report_host(
            &credential.credential,
            &credential.host_id,
            1,
            1,
            capabilities(),
        )
        .unwrap();
    assert_eq!(report.capability_revision, 3);
    assert!(!report.execution_ready);
}
#[test]
fn host_revoke_reenroll_fences_old_ticket_credential_and_epoch_aba() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let initial = store.create_host(&owner, &w, definition(1)).unwrap();
    let old = store
        .enroll_host(&initial.enrollment.ticket, capabilities())
        .unwrap();
    let revoked = store.revoke_host(&owner, &w, &old.host_id, 2).unwrap();
    assert_eq!(revoked.enrollment_epoch, 2);
    assert_eq!(revoked.credential_revision, 2);
    assert!(revoked.drain);
    assert!(
        store
            .report_host(&old.credential, &old.host_id, 1, 1, capabilities())
            .is_err()
    );
    let rotated = store
        .rotate_host_enrollment(&owner, &w, &old.host_id, revoked.revision)
        .unwrap();
    assert_eq!(rotated.host.enrollment_epoch, 3);
    assert_eq!(rotated.host.definition, initial.host.definition);
    assert!(
        store
            .enroll_host(&initial.enrollment.ticket, capabilities())
            .is_err()
    );
    let fresh = store
        .enroll_host(&rotated.enrollment.ticket, capabilities())
        .unwrap();
    assert_ne!(old.credential, fresh.credential);
    assert!(
        store
            .report_host(&old.credential, &old.host_id, 3, 3, capabilities())
            .is_err()
    );
    assert!(
        store
            .report_host(&fresh.credential, &fresh.host_id, 1, 1, capabilities())
            .is_err()
    );
    assert!(
        store
            .report_host(&fresh.credential, &fresh.host_id, 3, 3, capabilities())
            .is_ok()
    );
    let pending = store.create_host(&owner, &w, definition(2)).unwrap();
    store
        .connection
        .execute(
            "UPDATE team_hosts SET ticket_expires=0 WHERE id=?1",
            [&pending.host.id],
        )
        .unwrap();
    assert!(
        store
            .enroll_host(&pending.enrollment.ticket, capabilities())
            .is_err()
    );
    assert_eq!(
        store
            .host(&owner, &w, &pending.host.id)
            .unwrap()
            .unwrap()
            .status,
        HostStatus::Pending
    );
}
#[test]
fn host_management_requires_current_workspace_role_and_counts_revoked_tombstones_toward_quota() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let member = store.create_user("host-member", "member").unwrap();
    let outsider = store.create_user("unrelated-admin", "admin").unwrap();
    store
        .set_team_member(&owner, &w, &member.id, WorkspaceRole::Member, true, None)
        .unwrap();
    assert!(store.hosts(&member.id, &w).is_err());
    assert!(store.hosts(&outsider.id, &w).is_err());
    assert!(store.create_host(&member.id, &w, definition(0)).is_err());
    store
        .set_team_member(&owner, &w, &member.id, WorkspaceRole::Admin, true, Some(1))
        .unwrap();
    let first = store.create_host(&member.id, &w, definition(0)).unwrap();
    assert!(store.create_host(&owner, &w, definition(0)).is_err());
    store.revoke_host(&owner, &w, &first.host.id, 1).unwrap();
    store
        .set_team_member(&owner, &w, &member.id, WorkspaceRole::Member, true, Some(2))
        .unwrap();
    assert!(store.host(&member.id, &w, &first.host.id).is_err());
    assert!(
        store
            .rotate_host_enrollment(&member.id, &w, &first.host.id, 2)
            .is_err()
    );
    for index in 1..64 {
        store.create_host(&owner, &w, definition(index)).unwrap();
    }
    assert_eq!(store.hosts(&owner, &w).unwrap().len(), 64);
    assert!(matches!(
        store.create_host(&owner, &w, definition(64)),
        Err(StoreError::Limit(_))
    ));
}
#[test]
fn simultaneous_ticket_claims_on_independent_database_connections_issue_one_credential() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w) = scope(&store);
    let created = store.create_host(&owner, &w, definition(1)).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let ticket = created.enrollment.ticket.clone();
        let barrier = barrier.clone();
        jobs.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            barrier.wait();
            store.enroll_host(&ticket, capabilities()).is_ok()
        }));
    }
    assert_eq!(
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .filter(|accepted| *accepted)
            .count(),
        1
    );
    let host = store.host(&owner, &w, &created.host.id).unwrap().unwrap();
    assert_eq!(host.status, HostStatus::Claimed);
    assert_eq!(host.revision, 2);
}
#[test]
fn host_report_catalog_invalidation_is_visible_only_to_current_workspace_managers() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let member = store.create_user("host-viewer", "member").unwrap();
    let outsider = store.create_user("outside-admin", "admin").unwrap();
    store
        .set_team_member(&owner, &w, &member.id, WorkspaceRole::Member, true, None)
        .unwrap();
    let created = store.create_host(&owner, &w, definition(1)).unwrap();
    let credential = store
        .enroll_host(&created.enrollment.ticket, capabilities())
        .unwrap();
    let manager_before = store.team_catalog_revision(&owner).unwrap();
    let member_before = store.team_catalog_revision(&member.id).unwrap();
    let outsider_before = store.team_catalog_revision(&outsider.id).unwrap();
    store
        .report_host(
            &credential.credential,
            &credential.host_id,
            1,
            1,
            capabilities(),
        )
        .unwrap();
    assert_ne!(manager_before, store.team_catalog_revision(&owner).unwrap());
    assert_eq!(
        member_before,
        store.team_catalog_revision(&member.id).unwrap()
    );
    assert_eq!(
        outsider_before,
        store.team_catalog_revision(&outsider.id).unwrap()
    );
}
#[test]
fn full_ordinary_audit_never_prevents_a_bounded_durable_host_revocation_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w) = scope(&store);
    let created = store.create_host(&owner, &w, definition(1)).unwrap();
    let pending = store.create_host(&owner, &w, definition(2)).unwrap();
    let credential = store
        .enroll_host(&created.enrollment.ticket, capabilities())
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_revocation BEFORE INSERT ON team_host_revocations BEGIN SELECT RAISE(ABORT,'receipt unavailable'); END;").unwrap();
    assert!(
        store
            .revoke_host(&owner, &w, &credential.host_id, 2)
            .is_err()
    );
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                1,
                1,
                capabilities()
            )
            .is_ok(),
        "Failed receipt publication atomically rolls back invalidation"
    );
    assert!(
        store
            .host_revocation(&owner, &w, &credential.host_id)
            .unwrap()
            .is_none()
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_revocation;")
        .unwrap();
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM team_audit", [], |r| r.get(0))
        .unwrap();
    store.connection.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?3) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'quota_fixture',1,0 FROM n",params![w,owner,100000-count]).unwrap();
    assert!(store.team_audit_capacity().is_err());
    let revoked = store
        .revoke_host(&owner, &w, &credential.host_id, 2)
        .unwrap();
    assert_eq!(revoked.status, HostStatus::Revoked);
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                1,
                1,
                capabilities()
            )
            .is_err()
    );
    let again = store
        .revoke_host(&owner, &w, &credential.host_id, revoked.revision)
        .unwrap();
    assert_eq!(again.revision, revoked.revision);
    assert_eq!(again.enrollment_epoch, revoked.enrollment_epoch);
    assert!(
        store
            .revoke_host(&owner, &w, &credential.host_id, 2)
            .is_err()
    );
    store.revoke_host(&owner, &w, &pending.host.id, 1).unwrap();
    assert!(
        store
            .enroll_host(&pending.enrollment.ticket, capabilities())
            .is_err()
    );
    assert!(
        store
            .rotate_host_enrollment(&owner, &w, &credential.host_id, revoked.revision)
            .is_err(),
        "No positive enrollment cycle can overwrite receipts at exhausted ordinary audit capacity"
    );
    let receipt = store
        .host_revocation(&owner, &w, &credential.host_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.actor_id, owner);
    assert_eq!(receipt.enrollment_epoch, 2);
    assert_eq!(receipt.credential_revision, 2);
    assert_eq!(receipt.revision, revoked.revision);
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM team_host_revocations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM team_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        100000
    );
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(
        store
            .host_revocation(&owner, &w, &credential.host_id)
            .unwrap()
            .unwrap()
            .revision,
        revoked.revision
    );
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                1,
                1,
                capabilities()
            )
            .is_err()
    );
}
#[test]
fn final_counter_values_are_reserved_for_terminal_revocation_not_positive_authority() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let created = store.create_host(&owner, &w, definition(1)).unwrap();
    let credential = store
        .enroll_host(&created.enrollment.ticket, capabilities())
        .unwrap();
    let penultimate = i64::MAX - 1;
    let terminal = i64::MAX as u64;
    store.connection.execute("UPDATE team_hosts SET revision=?1,enrollment_epoch=?1,credential_revision=?1 WHERE id=?2",params![penultimate,credential.host_id]).unwrap();
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                penultimate as u64,
                penultimate as u64,
                capabilities()
            )
            .is_ok()
    );
    for result in [
        store.approve_host(&owner, &w, &credential.host_id, penultimate as u64),
        store.drain_host(&owner, &w, &credential.host_id, penultimate as u64, true),
    ] {
        assert!(matches!(result, Err(StoreError::Limit(_))));
    }
    assert!(matches!(
        store.rotate_host_enrollment(&owner, &w, &credential.host_id, penultimate as u64),
        Err(StoreError::Limit(_))
    ));
    let revoked = store
        .revoke_host(&owner, &w, &credential.host_id, penultimate as u64)
        .unwrap();
    assert_eq!(
        (
            revoked.revision,
            revoked.enrollment_epoch,
            revoked.credential_revision
        ),
        (terminal, terminal, terminal)
    );
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                terminal,
                terminal,
                capabilities()
            )
            .is_err()
    );
    assert!(
        store
            .report_host(
                &credential.credential,
                &credential.host_id,
                penultimate as u64,
                penultimate as u64,
                capabilities()
            )
            .is_err()
    );
    let repeated = store
        .revoke_host(&owner, &w, &credential.host_id, terminal)
        .unwrap();
    assert_eq!(repeated.revision, terminal);
    assert_eq!(repeated.enrollment_epoch, terminal);
    assert!(
        store
            .rotate_host_enrollment(&owner, &w, &credential.host_id, terminal)
            .is_err()
    );
    let pending = store.create_host(&owner, &w, definition(2)).unwrap();
    store
        .connection
        .execute(
            "UPDATE team_hosts SET revision=?1 WHERE id=?2",
            params![penultimate, pending.host.id],
        )
        .unwrap();
    assert!(
        store
            .enroll_host(&pending.enrollment.ticket, capabilities())
            .is_err()
    );
    assert_eq!(
        store
            .host(&owner, &w, &pending.host.id)
            .unwrap()
            .unwrap()
            .status,
        HostStatus::Pending
    );
    assert_eq!(
        store
            .revoke_host(&owner, &w, &pending.host.id, penultimate as u64)
            .unwrap()
            .revision,
        terminal
    );
    // A nonterminal legacy row already at MAX must still lose its credential.
    let legacy = store.create_host(&owner, &w, definition(3)).unwrap();
    let legacy = store
        .enroll_host(&legacy.enrollment.ticket, capabilities())
        .unwrap();
    store.connection.execute("UPDATE team_hosts SET revision=?1,enrollment_epoch=?1,credential_revision=?1 WHERE id=?2",params![i64::MAX,legacy.host_id]).unwrap();
    assert_eq!(
        store
            .revoke_host(&owner, &w, &legacy.host_id, terminal)
            .unwrap()
            .revision,
        terminal
    );
    assert!(
        store
            .report_host(
                &legacy.credential,
                &legacy.host_id,
                terminal,
                terminal,
                capabilities()
            )
            .is_err()
    );
}
