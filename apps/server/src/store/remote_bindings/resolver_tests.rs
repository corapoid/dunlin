use super::*;
use dunlin_domain::binding::BindingRef;
fn reader(f: &Fixture) -> String {
    let reader = f.store.create_user("target-reader", "member").unwrap();
    f.store
        .set_team_member(
            &f.actor,
            &f.workspace,
            &reader.id,
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
            &reader.id,
            [Permission::ViewProject, Permission::CreateSession].into(),
            None,
        )
        .unwrap();
    reader.id
}
fn candidate(f: &Fixture, actor: &str, reference: &BindingRef) -> Result<RemoteTargetCandidate> {
    f.store.remote_binding_transaction(false, |store| {
        store.capture_remote_target_in_transaction(
            actor,
            &f.workspace,
            &f.project,
            reference,
            "controller-node",
        )
    })
}
fn finish(
    f: &Fixture,
    actor: &str,
    candidate: RemoteTargetCandidate,
    descriptor: ExportDescriptor,
) -> Result<ResolvedRemoteTarget> {
    let proof = f.proof(descriptor);
    f.store.remote_binding_transaction(false, |store| {
        store.finish_remote_target_in_transaction(
            actor,
            &f.workspace,
            &f.project,
            candidate,
            proof,
            "controller-boot",
            "controller-node",
            clock(),
        )
    })
}
fn descriptor(f: &Fixture, actor: &str) -> ExportDescriptor {
    let mut descriptor = f.descriptor.clone();
    descriptor.grant.principal.id = actor.into();
    descriptor.grant.revision = 17;
    descriptor.grant.permissions = vec![ExportPermission::Read];
    descriptor
}
#[test]
fn create_only_actual_caller_resolves_own_read_grant_without_inheriting_admin_or_execution_authority()
 {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let reference = BindingRef {
        id: binding.id,
        revision: 1,
    };
    let reader = reader(&f);
    let mut events = f.store.subscribe_catalog();
    let before: i64 = f
        .store
        .connection
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let resolved = finish(
        &f,
        &reader,
        candidate(&f, &reader, &reference).unwrap(),
        descriptor(&f, &reader),
    )
    .unwrap();
    let summary = resolved.summary();
    assert_eq!(summary.binding, reference);
    assert_eq!(summary.principal.id, reader);
    assert_ne!(summary.principal.id, f.actor);
    assert_eq!(summary.grant_revision, 17);
    assert_eq!(summary.grant_permissions, vec![ExportPermission::Read]);
    assert_eq!(summary.required_worker_permission, "read");
    assert!(summary.observed_state_only);
    assert!(!summary.execution_ready);
    let json = serde_json::to_string(summary).unwrap();
    assert!(!json.contains(&f.actor));
    assert!(!json.contains(f.dir.path().to_str().unwrap()));
    assert_eq!(
        before,
        f.store
            .connection
            .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    // Diagnostic Read remains possible under drain; no execution permission is claimed.
    let host = f
        .store
        .host(&f.actor, &f.workspace, &f.host)
        .unwrap()
        .unwrap();
    f.store
        .drain_host(&f.actor, &f.workspace, &f.host, host.revision, true)
        .unwrap();
    assert!(
        finish(
            &f,
            &reader,
            candidate(&f, &reader, &reference).unwrap(),
            descriptor(&f, &reader)
        )
        .is_ok()
    );
}
#[test]
fn wrong_actual_principal_scope_export_configuration_or_read_permission_cannot_resolve() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let reference = BindingRef {
        id: binding.id,
        revision: 1,
    };
    let reader = reader(&f);
    for change in [
        "admin-principal",
        "service",
        "export",
        "export-revision",
        "checkout",
        "settings",
        "repo-kind",
        "permission",
        "scope",
    ] {
        let mut d = descriptor(&f, &reader);
        match change {
            "admin-principal" => d.grant.principal.id = f.actor.clone(),
            "service" => d.grant.principal.kind = PrincipalKind::Service,
            "export" => d.export.id = "another-export".into(),
            "export-revision" => {
                d.export.revision = 2;
                d.grant.export_revision = 2;
            }
            "checkout" => d.export.checkout_identity_digest = "c".repeat(64),
            "settings" => d.export.settings_digest = "d".repeat(64),
            "repo-kind" => d.export.repo_kind = RepoKind::Git,
            "permission" => d.grant.permissions = vec![ExportPermission::Execute],
            _ => d.scope.workspace_id = "another-workspace".into(),
        };
        assert!(
            finish(&f, &reader, candidate(&f, &reader, &reference).unwrap(), d).is_err(),
            "{change}"
        );
    }
}
#[test]
fn exact_reference_revoke_rebind_aba_and_current_controller_configuration_are_fenced() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let reference = BindingRef {
        id: binding.id.clone(),
        revision: 1,
    };
    let reader = reader(&f);
    let old = candidate(&f, &reader, &reference).unwrap();
    let revoked = f
        .store
        .revoke_remote_binding(&f.actor, &f.workspace, &f.project, &binding.id, 1)
        .unwrap();
    let rebound = f
        .save(
            Some(&binding.id),
            Some(revoked.revision),
            f.descriptor.clone(),
        )
        .unwrap();
    assert_eq!(rebound.revision, 3);
    assert!(candidate(&f, &reader, &reference).is_err());
    assert!(finish(&f, &reader, old, descriptor(&f, &reader)).is_err());
    let fresh = BindingRef {
        id: binding.id,
        revision: 3,
    };
    assert!(
        finish(
            &f,
            &reader,
            candidate(&f, &reader, &fresh).unwrap(),
            descriptor(&f, &reader)
        )
        .is_ok()
    );
    let captured = candidate(&f, &reader, &fresh).unwrap();
    let proof = f.proof(descriptor(&f, &reader));
    assert!(
        f.store
            .remote_binding_transaction(false, |store| store.finish_remote_target_in_transaction(
                &reader,
                &f.workspace,
                &f.project,
                captured,
                proof,
                "controller-boot",
                "changed-controller",
                clock()
            ))
            .is_err()
    );
    let captured = candidate(&f, &reader, &fresh).unwrap();
    let proof = f.proof(descriptor(&f, &reader));
    assert!(
        f.store
            .remote_binding_transaction(false, |store| store.finish_remote_target_in_transaction(
                &reader,
                &f.workspace,
                &f.project,
                captured,
                proof,
                "new-controller-boot",
                "controller-node",
                clock()
            ))
            .is_err()
    );
}
#[test]
fn current_create_authority_and_post_rpc_grant_aba_are_required_on_independent_connections() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let reference = BindingRef {
        id: binding.id,
        revision: 1,
    };
    let reader = reader(&f);
    let captured = candidate(&f, &reader, &reference).unwrap();
    let peer = Store::open(f.dir.path().join("relay.db")).unwrap();
    peer.set_team_project_grant(
        &f.actor,
        &f.workspace,
        &f.project,
        &reader,
        [Permission::ViewProject].into(),
        Some(1),
    )
    .unwrap();
    assert!(candidate(&f, &reader, &reference).is_err());
    peer.set_team_project_grant(
        &f.actor,
        &f.workspace,
        &f.project,
        &reader,
        [Permission::ViewProject, Permission::CreateSession].into(),
        Some(2),
    )
    .unwrap();
    assert!(finish(&f, &reader, captured, descriptor(&f, &reader)).is_err());
    assert!(
        finish(
            &f,
            &reader,
            candidate(&f, &reader, &reference).unwrap(),
            descriptor(&f, &reader)
        )
        .is_ok()
    );
}
#[test]
fn stale_observation_expired_channel_and_foreign_project_never_fall_back_to_a_local_target() {
    let f = Fixture::new();
    let binding = f.save(None, None, f.descriptor.clone()).unwrap();
    let reference = BindingRef {
        id: binding.id,
        revision: 1,
    };
    let reader = reader(&f);
    let captured = candidate(&f, &reader, &reference).unwrap();
    let proof = ObservedExport::for_test(
        f.identity.clone(),
        f.lease.clone(),
        descriptor(&f, &reader),
        clock() - 6000,
    );
    assert!(
        f.store
            .remote_binding_transaction(false, |store| store.finish_remote_target_in_transaction(
                &reader,
                &f.workspace,
                &f.project,
                captured,
                proof,
                "controller-boot",
                "controller-node",
                clock()
            ))
            .is_err()
    );
    let captured = candidate(&f, &reader, &reference).unwrap();
    let proof = f.proof(descriptor(&f, &reader));
    assert!(
        f.store
            .remote_binding_transaction(false, |store| store.finish_remote_target_in_transaction(
                &reader,
                &f.workspace,
                &f.project,
                captured,
                proof,
                "controller-boot",
                "controller-node",
                f.lease.expires_at_ms
            ))
            .is_err()
    );
    let other = f
        .store
        .create_team_project(&f.actor, &f.workspace, "Other")
        .unwrap();
    assert!(
        f.store
            .remote_binding_transaction(false, |store| store.capture_remote_target_in_transaction(
                &f.actor,
                &f.workspace,
                &other.id,
                &reference,
                "controller-node"
            ))
            .is_err()
    );
    let unknown = BindingRef {
        id: "unknown-binding".into(),
        revision: 1,
    };
    assert!(candidate(&f, &reader, &unknown).is_err());
}
