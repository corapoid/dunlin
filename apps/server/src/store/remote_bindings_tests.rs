use super::*;
use crate::store::worker_channels::{ChannelLease, WorkerIdentity};
use dunlin_domain::collaboration::WorkspaceRole;
use dunlin_relay_protocol::worker::ExportDescriptor;
use serde_json::json;
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    actor: String,
    workspace: String,
    project: String,
    host: String,
    identity: WorkerIdentity,
    lease: ChannelLease,
    descriptor: ExportDescriptor,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let owner = store.create_user("remote-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "Remote").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Project")
            .unwrap();
        let def=serde_json::from_value(json!({"name":"Remote worker","node_id":"worker-node","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["claude"]}})).unwrap();
        let host = store.create_host(&owner.id, &workspace.id, def).unwrap();
        let caps=serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap();
        let cred = store.enroll_host(&host.enrollment.ticket, caps).unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &host.host.id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker(&cred.credential, &host.host.id, 1, 1)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                "controller-boot",
                "worker-boot",
                "channel-fixture",
                clock(),
                15000,
            )
            .unwrap();
        let descriptor=serde_json::from_value(json!({"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node","workspace_id":workspace.id,"host_id":host.host.id,"enrollment_epoch":1,"credential_revision":1},"export":{"id":"export-fixture","name":"Export","repo_kind":"directory","checkout_identity_digest":"b".repeat(64),"settings_digest":"a".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":owner.id},"permissions":["read"],"revision":1,"export_revision":1,"active":true}})).unwrap();
        Self {
            dir,
            store,
            actor: owner.id,
            workspace: workspace.id,
            project: project.id,
            host: host.host.id,
            identity,
            lease,
            descriptor,
        }
    }
    fn authority(&self) -> AuthorityStamp {
        self.store
            .capture_remote_binding_authority(&self.actor, &self.workspace, &self.project)
            .unwrap()
    }
    fn proof(&self, descriptor: ExportDescriptor) -> ObservedExport {
        ObservedExport::for_test(
            self.identity.clone(),
            self.lease.clone(),
            descriptor,
            clock(),
        )
    }
    fn save(
        &self,
        id: Option<&str>,
        expected: Option<u64>,
        descriptor: ExportDescriptor,
    ) -> Result<RemoteBinding> {
        let proof = self.proof(descriptor);
        self.store.set_remote_binding(
            &self.actor,
            &self.workspace,
            &self.project,
            id,
            expected,
            &self.host,
            &proof.descriptor().export.id,
            proof.descriptor().export.revision,
            &proof,
            "controller-boot",
            "controller-node",
            clock(),
            &self.authority(),
        )
    }
}
#[test]
fn current_channel_observations_create_path_free_cas_bindings_with_durable_pins_and_tombstones() {
    let f = Fixture::new();
    let checkout = f.dir.path().join("local-checkout");
    std::fs::create_dir(&checkout).unwrap();
    let native = f
        .store
        .create_project("Existing local", checkout.to_str().unwrap())
        .unwrap();
    f.store.grant(&native.id, &f.actor).unwrap();
    let local = f
        .store
        .set_local_binding(&f.actor, &f.workspace, &f.project, &native.id, None)
        .unwrap();
    let first = f.save(None, None, f.descriptor.clone()).unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.principal.id, f.actor);
    assert_eq!(first.export_revision, 1);
    let raw = serde_json::to_string(&first).unwrap();
    assert!(!raw.contains("/"));
    assert!(!raw.contains("execution_ready"));
    assert!(matches!(
        f.save(None, None, f.descriptor.clone()),
        Err(StoreError::Conflict(_))
    ));
    let mut updated = f.descriptor.clone();
    updated.export.revision = 2;
    updated.grant.export_revision = 2;
    updated.grant.revision = 3;
    let second = f.save(Some(&first.id), Some(1), updated.clone()).unwrap();
    assert_eq!(second.revision, 2);
    assert!(matches!(
        f.save(Some(&first.id), Some(1), updated),
        Err(StoreError::Conflict(_))
    ));
    let pin: String = f
        .store
        .connection
        .query_row(
            "SELECT controller_node_id FROM team_remote_bindings WHERE id=?1",
            [&first.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pin, "controller-node");
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    assert_eq!(
        reopened
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()[0]
            .grant_revision,
        3
    );
    let revoked = f
        .store
        .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &first.id, 2)
        .unwrap();
    assert!(!revoked.active);
    assert_eq!(revoked.revision, 3);
    assert_eq!(
        f.store
            .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &first.id, 3)
            .unwrap()
            .revision,
        3
    );
    assert!(
        f.store
            .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &first.id, 2)
            .is_err()
    );
    let rebound = f
        .save(Some(&first.id), Some(3), f.descriptor.clone())
        .unwrap();
    assert_eq!(rebound.revision, 4);
    assert_eq!(
        f.store
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        serde_json::to_value(
            f.store
                .local_binding_for_user(&f.actor, &f.workspace, &f.project)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(local).unwrap(),
        "Remote mutations never rewrite or select the legacy local binding"
    );
}
#[test]
fn mismatched_expired_or_replaced_channel_and_principal_scope_proofs_cannot_write_bindings() {
    let f = Fixture::new();
    for change in [
        "workspace",
        "worker",
        "controller",
        "host",
        "epoch",
        "credential",
        "service",
        "principal",
        "revision",
        "read",
    ] {
        let mut d = f.descriptor.clone();
        match change {
            "workspace" => d.scope.workspace_id = "other-workspace".into(),
            "worker" => d.scope.worker_node_id = "other-worker".into(),
            "controller" => d.scope.controller_node_id = "other-controller".into(),
            "host" => d.scope.host_id = "other-host".into(),
            "epoch" => d.scope.enrollment_epoch = 2,
            "credential" => d.scope.credential_revision = 2,
            "service" => d.grant.principal.kind = PrincipalKind::Service,
            "principal" => d.grant.principal.id = "other-user".into(),
            "revision" => d.grant.export_revision = 2,
            _ => d.grant.permissions = vec![ExportPermission::Execute],
        };
        assert!(f.save(None, None, d).is_err(), "{change}");
    }
    let stale = ObservedExport::for_test(
        f.identity.clone(),
        f.lease.clone(),
        f.descriptor.clone(),
        clock() - 6000,
    );
    assert!(
        f.store
            .set_remote_binding(
                &f.actor,
                &f.workspace,
                &f.project,
                None,
                None,
                &f.host,
                "export-fixture",
                1,
                &stale,
                "controller-boot",
                "controller-node",
                clock(),
                &f.authority()
            )
            .is_err()
    );
    let proof = f.proof(f.descriptor.clone());
    f.store
        .open_worker_channel(
            &f.identity,
            "controller-boot",
            "replacement-worker",
            "replacement-channel",
            clock(),
            15000,
        )
        .unwrap();
    assert!(
        f.store
            .set_remote_binding(
                &f.actor,
                &f.workspace,
                &f.project,
                None,
                None,
                &f.host,
                "export-fixture",
                1,
                &proof,
                "controller-boot",
                "controller-node",
                clock(),
                &f.authority()
            )
            .is_err()
    );
    assert!(
        f.store
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn current_management_roles_and_exact_prequeue_authority_stamp_fence_revoke_regrant_aba() {
    let f = Fixture::new();
    let viewer = f.store.create_user("viewer", "member").unwrap();
    f.store
        .set_team_member(
            &f.actor,
            &f.workspace,
            &viewer.id,
            WorkspaceRole::Member,
            true,
            None,
        )
        .unwrap();
    f.store
        .set_team_project_grant(
            &f.actor,
            &f.workspace,
            &f.project,
            &viewer.id,
            [Permission::ViewProject, Permission::ManageProject].into(),
            None,
        )
        .unwrap();
    assert!(
        f.store
            .remote_bindings(&viewer.id, &f.workspace, &f.project)
            .is_err()
    );
    let captured = f.authority();
    let proof = f.proof(f.descriptor.clone());
    let peer = Store::open(f.dir.path().join("relay.db")).unwrap();
    let grant = peer
        .team_project_grant(&f.workspace, &f.project, &f.actor)
        .unwrap()
        .unwrap();
    let mut reduced = grant.permissions.clone();
    reduced.remove(&Permission::ManageProject);
    // Another workspace owner performs the revoke/regrant, since the target cannot manage its own project during the gap.
    let admin = peer.create_user("second-owner", "admin").unwrap();
    peer.set_team_member(
        &f.actor,
        &f.workspace,
        &admin.id,
        WorkspaceRole::Admin,
        true,
        None,
    )
    .unwrap();
    peer.set_team_project_grant(
        &f.actor,
        &f.workspace,
        &f.project,
        &admin.id,
        [Permission::ViewProject, Permission::ManageProject].into(),
        None,
    )
    .unwrap();
    peer.set_team_project_grant(
        &admin.id,
        &f.workspace,
        &f.project,
        &f.actor,
        reduced,
        Some(grant.revision),
    )
    .unwrap();
    peer.set_team_project_grant(
        &admin.id,
        &f.workspace,
        &f.project,
        &f.actor,
        grant.permissions,
        Some(grant.revision + 1),
    )
    .unwrap();
    assert!(
        f.store
            .set_remote_binding(
                &f.actor,
                &f.workspace,
                &f.project,
                None,
                None,
                &f.host,
                "export-fixture",
                1,
                &proof,
                "controller-boot",
                "controller-node",
                clock(),
                &captured
            )
            .is_err()
    );
    assert!(
        f.store
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn offline_terminal_revoke_is_audited_at_capacity_and_max_while_positive_writes_are_bounded() {
    let f = Fixture::new();
    let first = f.save(None, None, f.descriptor.clone()).unwrap();
    for index in 1..16 {
        let mut descriptor = f.descriptor.clone();
        descriptor.export.id = format!("export-{index}");
        f.save(None, None, descriptor).unwrap();
    }
    let mut overflow = f.descriptor.clone();
    overflow.export.id = "export-overflow".into();
    assert!(matches!(
        f.save(None, None, overflow),
        Err(StoreError::Limit(_))
    ));
    f.store
        .connection
        .execute(
            "UPDATE team_remote_bindings SET revision=?1 WHERE id=?2",
            params![MAX_REVISION as i64 - 1, first.id],
        )
        .unwrap();
    assert!(
        f.save(
            Some(&first.id),
            Some(MAX_REVISION - 1),
            f.descriptor.clone()
        )
        .is_err()
    );
    let count: i64 = f
        .store
        .connection
        .query_row("SELECT count(*) FROM team_audit", [], |r| r.get(0))
        .unwrap();
    f.store.connection.execute("WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?3) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'quota_fixture',1,0 FROM n",params![f.workspace,f.actor,100000-count]).unwrap();
    f.store.close_worker_channel(&f.identity, &f.lease).unwrap();
    let revoked = f
        .store
        .revoke_remote_binding(
            &f.actor,
            &f.workspace,
            &f.project,
            &first.id,
            MAX_REVISION - 1,
        )
        .unwrap();
    assert_eq!(revoked.revision, MAX_REVISION);
    assert!(!revoked.active);
    assert_eq!(
        f.store
            .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &first.id, MAX_REVISION)
            .unwrap()
            .revision,
        MAX_REVISION
    );
    assert!(
        f.store
            .set_remote_binding(
                &f.actor,
                &f.workspace,
                &f.project,
                Some(&first.id),
                Some(MAX_REVISION),
                &f.host,
                "export-fixture",
                1,
                &f.proof(f.descriptor.clone()),
                "controller-boot",
                "controller-node",
                clock(),
                &f.authority()
            )
            .is_err()
    );
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT count(*) FROM team_remote_binding_revocations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT count(*) FROM team_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        100000
    );
}
#[test]
fn a_failed_terminal_receipt_rolls_back_remote_binding_revocation_atomically() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER fail_remote_receipt BEFORE INSERT ON team_remote_binding_revocations BEGIN SELECT RAISE(ABORT,'receipt unavailable'); END;").unwrap();
    assert!(
        f.store
            .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &binding.id, 1)
            .is_err()
    );
    let current = f
        .store
        .remote_bindings(&f.actor, &f.workspace, &f.project)
        .unwrap();
    assert!(current[0].active);
    assert_eq!(current[0].revision, 1);
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT count(*) FROM team_remote_binding_revocations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn independent_connections_linearize_remote_binding_cas_without_reusing_an_old_revision() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = f.dir.path().join("relay.db");
        let actor = f.actor.clone();
        let workspace = f.workspace.clone();
        let project = f.project.clone();
        let host = f.host.clone();
        let id = binding.id.clone();
        let identity = f.identity.clone();
        let lease = f.lease.clone();
        let descriptor = f.descriptor.clone();
        let captured = f.authority();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            let proof = ObservedExport::for_test(identity, lease, descriptor, clock());
            barrier.wait();
            match store.set_remote_binding(
                &actor,
                &workspace,
                &project,
                Some(&id),
                Some(1),
                &host,
                "export-fixture",
                1,
                &proof,
                "controller-boot",
                "controller-node",
                clock(),
                &captured,
            ) {
                Ok(binding) => {
                    assert_eq!(binding.revision, 2);
                    true
                }
                Err(StoreError::Conflict(_)) => false,
                Err(error) => panic!("Unexpected CAS failure: {error}"),
            }
        }));
    }
    let left = threads.remove(0).join().unwrap();
    let right = threads.remove(0).join().unwrap();
    assert_ne!(left, right);
    assert_eq!(
        f.store
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()[0]
            .revision,
        2
    );
}
#[test]
fn remote_binding_invalidations_follow_commit_and_never_fire_for_reads_or_failed_mutations() {
    let f = Fixture::new();
    let mut changes = f.store.subscribe_catalog();
    f.store
        .remote_bindings(&f.actor, &f.workspace, &f.project)
        .unwrap();
    assert!(matches!(
        changes.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    assert!(changes.try_recv().is_ok());
    assert!(f.save(None, None, f.descriptor.clone()).is_err());
    assert!(matches!(
        changes.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    let result: Result<()> = f.store.remote_binding_transaction(true, |store| {
        store.revoke_remote_binding_in_transaction(
            &f.actor,
            &f.workspace,
            &f.project,
            &binding.id,
            1,
        )?;
        Err(StoreError::Conflict("fixture rollback".into()))
    });
    assert!(result.is_err());
    assert!(
        f.store
            .remote_bindings(&f.actor, &f.workspace, &f.project)
            .unwrap()[0]
            .active
    );
    assert!(matches!(
        changes.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}
#[test]
fn a_reenrolled_host_and_replacement_channel_cannot_revive_the_old_binding_observation() {
    let f = Fixture::new();
    let old = f.proof(f.descriptor.clone());
    let authority = f.authority();
    let host = f
        .store
        .host(&f.actor, &f.workspace, &f.host)
        .unwrap()
        .unwrap();
    let revoked = f
        .store
        .revoke_host(&f.actor, &f.workspace, &f.host, host.revision)
        .unwrap();
    let pending = f
        .store
        .rotate_host_enrollment(&f.actor, &f.workspace, &f.host, revoked.revision)
        .unwrap();
    let credential = f
        .store
        .enroll_host(
            &pending.enrollment.ticket,
            f.identity.host().capabilities.clone().unwrap(),
        )
        .unwrap();
    let claimed = f
        .store
        .host(&f.actor, &f.workspace, &f.host)
        .unwrap()
        .unwrap();
    f.store
        .approve_host(&f.actor, &f.workspace, &f.host, claimed.revision)
        .unwrap();
    let identity = f
        .store
        .authenticate_worker(&credential.credential, &f.host, 3, 3)
        .unwrap();
    let lease = f
        .store
        .open_worker_channel(
            &identity,
            "controller-boot",
            "fresh-worker",
            "fresh-host-channel",
            clock(),
            15000,
        )
        .unwrap();
    assert!(
        f.store
            .set_remote_binding(
                &f.actor,
                &f.workspace,
                &f.project,
                None,
                None,
                &f.host,
                "export-fixture",
                1,
                &old,
                "controller-boot",
                "controller-node",
                clock(),
                &authority
            )
            .is_err()
    );
    let mut descriptor = f.descriptor.clone();
    descriptor.scope.enrollment_epoch = 3;
    descriptor.scope.credential_revision = 3;
    let fresh = ObservedExport::for_test(identity, lease, descriptor, clock());
    let saved = f
        .store
        .set_remote_binding(
            &f.actor,
            &f.workspace,
            &f.project,
            None,
            None,
            &f.host,
            "export-fixture",
            1,
            &fresh,
            "controller-boot",
            "controller-node",
            clock(),
            &authority,
        )
        .unwrap();
    assert_eq!(
        (saved.host_enrollment_epoch, saved.credential_revision),
        (3, 3)
    );
}

#[path = "remote_bindings/resolver_tests.rs"]
mod resolver_tests;
