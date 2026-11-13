use super::*;
use dunlin_domain::hosts::{HostCapabilities, HostDefinition};
use serde_json::json;
fn capabilities() -> HostCapabilities {
    serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":2})).unwrap()
}
fn scope(store: &Store) -> (String, String) {
    let owner = store.create_user("worker-channel-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Workers").unwrap();
    (owner.id, workspace.id)
}
fn definition(index: usize) -> HostDefinition {
    serde_json::from_value(json!({"name":format!("Worker {index}"),"node_id":format!("node-{index}"),"serve_origin":format!("https://worker{index}.tail123.ts.net"),"capability_policy":{"max_parallel":2,"harnesses":["claude"]}})).unwrap()
}
fn worker(
    store: &Store,
    owner: &str,
    workspace: &str,
    index: usize,
) -> (Host, String, WorkerIdentity) {
    let pending = store
        .create_host(owner, workspace, definition(index))
        .unwrap();
    let credential = store
        .enroll_host(&pending.enrollment.ticket, capabilities())
        .unwrap();
    assert!(
        store
            .authenticate_worker(&credential.credential, &credential.host_id, 1, 1)
            .is_err(),
        "Claimed metadata is not administrator approval"
    );
    let host = store
        .approve_host(owner, workspace, &credential.host_id, 2)
        .unwrap();
    let identity = store
        .authenticate_worker(&credential.credential, &host.id, 1, 1)
        .unwrap();
    (host, credential.credential, identity)
}
fn open(store: &Store, identity: &WorkerIdentity, id: &str, now: i64) -> ChannelLease {
    store
        .open_worker_channel(identity, "controller-1", "worker-1", id, now, 10000)
        .unwrap()
}
#[test]
fn worker_identity_is_approved_exact_epoch_and_worker_prefix_only_while_drain_remains_diagnostic() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let (host, token, identity) = worker(&store, &owner, &w, 1);
    let token_identity = store.authenticate_worker_token(&token).unwrap();
    assert_eq!(token_identity.host_id(), host.id);
    assert_eq!(token_identity.workspace_id(), w);
    let native = store.create_device(&owner, "native").unwrap().1;
    for wrong in [
        native.as_str(),
        "dhe_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "dhw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(store.authenticate_worker_token(wrong).is_err());
        assert!(store.authenticate_worker(wrong, &host.id, 1, 1).is_err());
    }
    assert!(store.authenticate_worker(&token, &host.id, 2, 1).is_err());
    assert!(store.authenticate_worker(&token, &host.id, 1, 2).is_err());
    assert_eq!(identity.host().id, host.id);
    assert_eq!(identity.workspace_id(), w);
    store
        .drain_host(&owner, &w, &host.id, host.revision, true)
        .unwrap();
    assert!(store.validate_worker_identity(&identity).unwrap().drain);
    let lease = open(&store, &identity, "drained-diagnostic", 1000);
    assert_eq!(lease.epoch, 1);
    assert!(
        !store
            .validate_worker_identity(&identity)
            .unwrap()
            .execution_ready
    );
    let current = store.host(&owner, &w, &host.id).unwrap().unwrap();
    store
        .revoke_host(&owner, &w, &host.id, current.revision)
        .unwrap();
    assert!(store.validate_worker_identity(&identity).is_err());
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "controller-1", 1001)
            .is_err()
    );
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 1, 1001, 10000)
            .is_err()
    );
    assert!(store.authenticate_worker(&token, &host.id, 1, 1).is_err());
    store.close_worker_channel(&identity, &lease).unwrap();
    store.close_worker_channel(&identity, &lease).unwrap();
    let phase: String = store
        .connection
        .query_row(
            "SELECT phase FROM team_worker_channels WHERE host_id=?1",
            [&host.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        phase, "offline",
        "Exact captured lease cleanup remains possible after credential revocation"
    );
}
#[test]
fn worker_heartbeat_sequences_duplicate_receipts_expiration_and_clock_bounds_are_exact() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let (_, _, identity) = worker(&store, &owner, &w, 1);
    let lease = open(&store, &identity, "channel-1", 1000);
    assert_eq!(lease.expires_at_ms, 11000);
    assert_eq!(lease.last_sequence, 0);
    let renewed = store
        .renew_worker_channel(&identity, &lease, 1, 2000, 10000)
        .unwrap();
    assert_eq!(renewed.expires_at_ms, 12000);
    assert_eq!(renewed.last_sequence, 1);
    let duplicate = store
        .renew_worker_channel(&identity, &lease, 1, 3000, 15000)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&duplicate).unwrap(),
        serde_json::to_value(&renewed).unwrap(),
        "Exact duplicate cannot prolong a lease"
    );
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 0, 3001, 10000)
            .is_err()
    );
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 3, 3001, 10000)
            .is_err()
    );
    let next = store
        .renew_worker_channel(&identity, &lease, 2, 3000, 10000)
        .unwrap();
    assert_eq!(next.expires_at_ms, 13000);
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 1, 3001, 10000)
            .is_err()
    );
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "controller-2", 3001)
            .is_err()
    );
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "controller-1", 13000)
            .is_err()
    );
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 3, 13000, 10000)
            .is_err(),
        "An expired lease cannot be revived by a late heartbeat"
    );
    for (now, ttl) in [(-1, 10000), (0, 4999), (0, 15001), (i64::MAX, 5000)] {
        assert!(
            store
                .open_worker_channel(
                    &identity,
                    "controller-1",
                    "worker-1",
                    "invalid-clock",
                    now,
                    ttl
                )
                .is_err()
        );
    }
    store.close_worker_channel(&identity, &lease).unwrap();
    store.close_worker_channel(&identity, &lease).unwrap();
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "controller-1", 3001)
            .is_err()
    );
    assert!(
        store
            .renew_worker_channel(&identity, &lease, 3, 3001, 10000)
            .is_err()
    );
}
#[test]
fn channel_epochs_survive_reopen_controller_restart_and_host_enrollment_aba_without_resurrection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w) = scope(&store);
    let (host, old_token, identity) = worker(&store, &owner, &w, 1);
    let first = open(&store, &identity, "first-channel", 1000);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(
        store
            .validate_worker_channel(&identity, &first, "new-controller", 1001)
            .is_err()
    );
    let replacement = store
        .open_worker_channel(
            &identity,
            "new-controller",
            "new-worker",
            "second-channel",
            1001,
            10000,
        )
        .unwrap();
    assert_eq!(replacement.epoch, 2);
    assert!(store.close_worker_channel(&identity, &first).is_err());
    assert!(
        store
            .renew_worker_channel(&identity, &first, 1, 1002, 10000)
            .is_err()
    );
    assert!(
        store
            .validate_worker_channel(&identity, &replacement, "new-controller", 1002)
            .is_ok()
    );
    let current = store.host(&owner, &w, &host.id).unwrap().unwrap();
    let revoked = store
        .revoke_host(&owner, &w, &host.id, current.revision)
        .unwrap();
    let pending = store
        .rotate_host_enrollment(&owner, &w, &host.id, revoked.revision)
        .unwrap();
    let credential = store
        .enroll_host(&pending.enrollment.ticket, capabilities())
        .unwrap();
    let claimed = store.host(&owner, &w, &host.id).unwrap().unwrap();
    store
        .approve_host(&owner, &w, &host.id, claimed.revision)
        .unwrap();
    let fresh = store
        .authenticate_worker(&credential.credential, &host.id, 3, 3)
        .unwrap();
    let third = store
        .open_worker_channel(
            &fresh,
            "new-controller",
            "new-worker",
            "third-channel",
            1003,
            10000,
        )
        .unwrap();
    assert_eq!(third.epoch, 3);
    assert!(
        store
            .authenticate_worker(&old_token, &host.id, 3, 3)
            .is_err()
    );
    assert!(store.close_worker_channel(&identity, &replacement).is_err());
    assert!(
        store
            .validate_worker_channel(&fresh, &third, "new-controller", 1004)
            .is_ok()
    );
    store
        .connection
        .execute(
            "UPDATE team_worker_channels SET epoch=?1 WHERE host_id=?2",
            params![i64::MAX, host.id],
        )
        .unwrap();
    let mut maximum = third;
    maximum.epoch = i64::MAX as u64;
    store.close_worker_channel(&fresh, &maximum).unwrap();
    assert!(
        store
            .open_worker_channel(
                &fresh,
                "new-controller",
                "new-worker",
                "exhausted",
                1005,
                10000
            )
            .is_err()
    );
}
#[test]
fn independent_database_opens_linearize_one_current_epoch_and_stale_close_cannot_touch_replacement()
{
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w) = scope(&store);
    let (_, _, identity) = worker(&store, &owner, &w, 1);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for n in 0..2 {
        let path = path.clone();
        let identity = identity.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            barrier.wait();
            store
                .open_worker_channel(
                    &identity,
                    "controller-1",
                    "worker-1",
                    &format!("racing-channel-{n}"),
                    1000,
                    10000,
                )
                .unwrap()
        }));
    }
    let mut leases = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    leases.sort_by_key(|lease| lease.epoch);
    assert_eq!(
        leases.iter().map(|lease| lease.epoch).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(store.close_worker_channel(&identity, &leases[0]).is_err());
    assert!(
        store
            .validate_worker_channel(&identity, &leases[1], "controller-1", 1001)
            .is_ok()
    );
}
#[test]
fn global_active_capacity_bounds_new_hosts_but_allows_same_host_replacement_and_expired_reuse() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let mut identities = Vec::new();
    for index in 0..64 {
        let (_, _, identity) = worker(&store, &owner, &w, index);
        open(
            &store,
            &identity,
            &format!("capacity-channel-{index}"),
            1000,
        );
        identities.push(identity);
    }
    let user = store
        .find_user_by_name("worker-channel-owner")
        .unwrap()
        .unwrap();
    let second_workspace = store.create_workspace(&user, "Overflow").unwrap();
    let (_, _, overflow) = worker(&store, &owner, &second_workspace.id, 64);
    assert!(matches!(
        store.open_worker_channel(
            &overflow,
            "controller-1",
            "worker-1",
            "capacity-overflow",
            1001,
            10000
        ),
        Err(StoreError::Limit(_))
    ));
    assert_eq!(
        open(&store, &identities[0], "same-host-replacement", 1001).epoch,
        2
    );
    assert!(
        store
            .open_worker_channel(
                &overflow,
                "controller-1",
                "worker-1",
                "capacity-overflow",
                11001,
                10000
            )
            .is_ok(),
        "Expired transport rows release capacity but remain as epoch tombstones"
    );
}
#[test]
fn diagnostic_read_requires_current_host_management_scope_and_is_not_active_lease_proof() {
    use dunlin_domain::collaboration::WorkspaceRole;
    let store = Store::open_in_memory().unwrap();
    let (owner, w) = scope(&store);
    let (host, _, identity) = worker(&store, &owner, &w, 1);
    let member = store.create_user("channel-viewer", "member").unwrap();
    let outsider = store.create_user("unrelated-admin", "admin").unwrap();
    store
        .set_team_member(&owner, &w, &member.id, WorkspaceRole::Member, true, None)
        .unwrap();
    assert!(
        store
            .worker_channel_diagnostic(&owner, &w, &host.id)
            .unwrap()
            .is_none()
    );
    let lease = open(&store, &identity, "diagnostic-channel", 1000);
    let diagnostic = store
        .worker_channel_diagnostic(&owner, &w, &host.id)
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.lease.epoch, lease.epoch);
    assert_eq!(diagnostic.phase, "active");
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "controller-1", 11000)
            .is_err()
    );
    assert_eq!(
        store
            .worker_channel_diagnostic(&owner, &w, &host.id)
            .unwrap()
            .unwrap()
            .phase,
        "active",
        "Persisted active metadata alone proves no current health"
    );
    assert!(
        store
            .worker_channel_diagnostic(&member.id, &w, &host.id)
            .is_err()
    );
    assert!(
        store
            .worker_channel_diagnostic(&outsider.id, &w, &host.id)
            .is_err()
    );
    let user = store
        .find_user_by_name("worker-channel-owner")
        .unwrap()
        .unwrap();
    let other = store.create_workspace(&user, "Other scope").unwrap();
    assert!(
        store
            .worker_channel_diagnostic(&owner, &other.id, &host.id)
            .is_err()
    );
    store.close_worker_channel(&identity, &lease).unwrap();
    assert_eq!(
        store
            .worker_channel_diagnostic(&owner, &w, &host.id)
            .unwrap()
            .unwrap()
            .phase,
        "offline"
    );
}
