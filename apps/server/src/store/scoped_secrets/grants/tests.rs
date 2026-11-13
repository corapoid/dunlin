use super::*;
use crate::store::scoped_secrets::tests::{candidate, scope};
use dunlin_domain::collaboration::WorkspaceRole;
use serde_json::json;

fn publish(store: &Store, proof: &Proof, name: &str) -> AgentProfileRef {
    let definition=serde_json::from_value(json!({"schema_version":2,"name":name,"harness":"claude","model":"fixture","mode":"plan","skills":[],"mcp":[{"provider_id":"dunlin-secret-work","name":"http","definition_digest":tool::http().definition_digest().unwrap()}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
    let draft = store
        .create_agent(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            definition,
        )
        .unwrap();
    let version = store
        .publish_agent(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            &draft.id,
            1,
        )
        .unwrap();
    AgentProfileRef {
        agent_id: version.agent_id,
        version: version.version,
        digest: version.digest,
    }
}
fn setup(store: &Store) -> (Proof, GrantRequest) {
    let proof = scope(store);
    let c = candidate(&proof, 0);
    let claim = store
        .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &claim).unwrap();
    store
        .scoped_secret_activate(&proof, claim, &|_| Ok(()))
        .unwrap();
    let profile = publish(store, &proof, "Scoped profile");
    let request = GrantRequest {
        expected_revision: 0,
        credential_revision: 1,
        principal_kind: Principal::Human,
        principal_id: proof.browser.user.clone(),
        profile,
        provider_id: "dunlin-secret-work".into(),
        read_only: true,
    };
    (proof, request)
}
fn available(store: &Store, proof: &Proof, request: &GrantRequest) -> Vec<Grant> {
    store
        .scoped_secret_available(
            &request.principal_id,
            &proof.binding,
            &request.profile,
            &|_| Ok(()),
        )
        .unwrap()
}

#[test]
fn exact_published_profile_grant_survives_reopen_and_revoke_regrant_keeps_revision_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (proof, mut request) = setup(&store);
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_eq!(available(&store, &proof, &request).len(), 1);
    assert!(matches!(
        store.scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(())),
        Err(Error::Conflict)
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(available(&store, &proof, &request)[0].id, grant.id);
    let revoked = store
        .scoped_secret_revoke_grant(&proof, "work-api", &grant.id, 1, &|_| Ok(()))
        .unwrap();
    assert_eq!(revoked.revision, 2);
    assert!(!revoked.active);
    assert!(available(&store, &proof, &request).is_empty());
    request.expected_revision = 2;
    let restored = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_eq!(restored.id, grant.id);
    assert_eq!(restored.revision, 3);
    assert!(matches!(
        store.scoped_secret_revoke_grant(&proof, "work-api", &grant.id, 1, &|_| Ok(())),
        Err(Error::Conflict)
    ));
}

#[test]
fn profile_digest_schema_slot_and_credential_version_cannot_be_substituted() {
    let store = Store::open_in_memory().unwrap();
    let (proof, mut request) = setup(&store);
    let original = request.profile.clone();
    request.profile.digest = "b".repeat(64);
    assert!(matches!(
        store.scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(())),
        Err(Error::Denied)
    ));
    request.profile = original;
    request.provider_id = "external".into();
    assert!(matches!(
        store.scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(())),
        Err(Error::Invalid)
    ));
    request.provider_id = "dunlin-secret-work".into();
    request.credential_revision = 2;
    assert!(matches!(
        store.scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(())),
        Err(Error::Conflict)
    ));
    request.credential_revision = 1;
    request.profile = publish(&store, &proof, "Different profile");
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    let mut different = request.profile.clone();
    different.digest = "a".repeat(64);
    let context = store
        .team_project_context(
            &proof.binding.workspace,
            &proof.binding.project,
            &request.principal_id,
        )
        .unwrap();
    assert!(matches!(
        store.resolve_secret_grant(
            &context,
            &proof.binding,
            &different,
            &grant.id,
            grant.revision
        ),
        Err(Error::Denied)
    ));
}

#[test]
fn credential_rotation_and_host_change_invalidate_old_grants_without_rewriting_them() {
    for host_change in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let (proof, request) = setup(&store);
        let grant = store
            .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
            .unwrap();
        if host_change {
            store
                .connection
                .execute(
                    "UPDATE team_hosts SET revision=revision+1 WHERE id=?1",
                    [&proof.binding.host],
                )
                .unwrap();
        } else {
            let c = candidate(&proof, 1);
            let claim = store
                .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
                .unwrap();
            store.scoped_secret_acknowledge(&proof, &claim).unwrap();
            store
                .scoped_secret_activate(&proof, claim, &|_| Ok(()))
                .unwrap();
        }
        assert!(available(&store, &proof, &request).is_empty());
        assert_eq!(
            store
                .secret_grant_row(&proof.binding, &grant.id)
                .unwrap()
                .revision,
            1
        );
    }
}

#[test]
fn recipient_membership_and_project_grant_aba_require_explicit_secret_regrant() {
    for membership in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let (proof, mut request) = setup(&store);
        let actor = store.create_user("recipient", "member").unwrap();
        store
            .set_team_member(
                &proof.browser.user,
                &proof.binding.workspace,
                &actor.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        let permissions = BTreeSet::from([Permission::ViewProject, Permission::CreateSession]);
        store
            .set_team_project_grant(
                &proof.browser.user,
                &proof.binding.workspace,
                &proof.binding.project,
                &actor.id,
                permissions.clone(),
                None,
            )
            .unwrap();
        request.principal_id = actor.id.clone();
        store
            .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
            .unwrap();
        assert_eq!(available(&store, &proof, &request).len(), 1);
        if membership {
            store
                .set_team_member(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &actor.id,
                    WorkspaceRole::Member,
                    false,
                    Some(1),
                )
                .unwrap();
            store
                .set_team_member(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &actor.id,
                    WorkspaceRole::Member,
                    true,
                    Some(2),
                )
                .unwrap();
        } else {
            store
                .set_team_project_grant(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &actor.id,
                    BTreeSet::new(),
                    Some(1),
                )
                .unwrap();
            store
                .set_team_project_grant(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &actor.id,
                    permissions,
                    Some(2),
                )
                .unwrap();
        }
        assert!(available(&store, &proof, &request).is_empty());
        request.expected_revision = 1;
        store
            .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
            .unwrap();
        assert_eq!(available(&store, &proof, &request).len(), 1);
    }
}

#[test]
fn service_grants_require_live_scoped_principal_and_never_authorize_a_human_with_the_same_id() {
    let store = Store::open_in_memory().unwrap();
    let (proof, mut request) = setup(&store);
    let (service, token) = store
        .create_service_principal(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            "Automation",
            BTreeSet::from([Permission::ViewProject, Permission::RunWorkflow]),
            600,
        )
        .unwrap();
    request.principal_kind = Principal::Service;
    request.principal_id = service.id.clone();
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    let context = store
        .service_authority(
            &token,
            &proof.binding.workspace,
            &proof.binding.project,
            Permission::RunWorkflow,
        )
        .unwrap();
    assert!(
        store
            .resolve_secret_grant(
                &context,
                &proof.binding,
                &request.profile,
                &grant.id,
                grant.revision
            )
            .is_ok()
    );
    let mut human = context.clone();
    human.principal = PrincipalKind::Human;
    assert!(
        store
            .resolve_secret_grant(
                &human,
                &proof.binding,
                &request.profile,
                &grant.id,
                grant.revision
            )
            .is_err()
    );
    store
        .revoke_service_principal(
            &proof.browser.user,
            &proof.binding.workspace,
            &service.id,
            service.revision,
        )
        .unwrap();
    assert!(
        store
            .resolve_secret_grant(
                &context,
                &proof.binding,
                &request.profile,
                &grant.id,
                grant.revision
            )
            .is_err()
    );
}

#[test]
fn identical_definition_digests_do_not_merge_distinct_published_profile_references() {
    let store = Store::open_in_memory().unwrap();
    let (proof, mut request) = setup(&store);
    let first = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    request.profile = publish(&store, &proof, "Scoped profile");
    assert_eq!(first.profile.digest, request.profile.digest);
    let second = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_ne!(first.id, second.id);
    let list = available(&store, &proof, &request);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, second.id);
}

#[test]
fn full_or_rejected_global_audit_cannot_block_revocation_receipt() {
    for full in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let (proof, request) = setup(&store);
        let grant = store
            .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
            .unwrap();
        if full {
            store.connection.execute("WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100000) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'fixture_capacity',1,0 FROM rows LIMIT (100000-(SELECT count(*) FROM team_audit))",params![proof.binding.workspace,proof.browser.user]).unwrap();
        } else {
            store.connection.execute_batch("CREATE TRIGGER reject_grant_revoke BEFORE INSERT ON team_audit WHEN NEW.action='scoped_secret_grant_revoked' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        }
        assert!(
            !store
                .scoped_secret_revoke_grant(&proof, "work-api", &grant.id, 1, &|_| Ok(()))
                .unwrap()
                .active
        );
        let receipt: String = store
            .connection
            .query_row(
                "SELECT last_revoke FROM scoped_secret_grants WHERE id=?1",
                [&grant.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(receipt.contains(&proof.browser.user));
    }
}

#[test]
fn changing_grant_permissions_never_refreshes_an_old_selected_revision() {
    let store = Store::open_in_memory().unwrap();
    let (proof, mut request) = setup(&store);
    let first = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    let context = store
        .team_project_context(
            &proof.binding.workspace,
            &proof.binding.project,
            &proof.browser.user,
        )
        .unwrap();
    request.expected_revision = 1;
    request.read_only = false;
    let current = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_eq!(first.id, current.id);
    assert_eq!(current.revision, 2);
    assert!(matches!(
        store.resolve_secret_grant(
            &context,
            &proof.binding,
            &request.profile,
            &first.id,
            first.revision
        ),
        Err(Error::Denied)
    ));
    assert!(
        !store
            .resolve_secret_grant(
                &context,
                &proof.binding,
                &request.profile,
                &current.id,
                current.revision
            )
            .unwrap()
            .0
            .read_only
    );
}

#[test]
fn revision_ceiling_still_allows_terminal_credential_and_grant_revocation() {
    let max = 9_007_199_254_740_991u64;
    for credential in [false, true] {
        let store = Store::open_in_memory().unwrap();
        let (proof, mut request) = setup(&store);
        let grant = store
            .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
            .unwrap();
        if credential {
            store
                .connection
                .execute("UPDATE scoped_credentials SET revision=?1", [max as i64])
                .unwrap();
            store
                .connection
                .execute(
                    "UPDATE scoped_secret_references SET revision=?1",
                    [max as i64],
                )
                .unwrap();
            let status = store
                .scoped_secret_disable(&proof, "work-api", max, &|_| Ok(()))
                .unwrap();
            assert_eq!(status.revision, max);
            assert_eq!(status.state, "disabled");
            assert!(
                store
                    .scoped_secret_reserve(&proof, &candidate(&proof, max), &|_| Ok(()))
                    .is_err()
            );
        } else {
            store
                .connection
                .execute(
                    "UPDATE scoped_secret_grants SET revision=?1 WHERE id=?2",
                    params![max as i64, grant.id],
                )
                .unwrap();
            let revoked = store
                .scoped_secret_revoke_grant(&proof, "work-api", &grant.id, max, &|_| Ok(()))
                .unwrap();
            assert_eq!(revoked.revision, max);
            assert!(!revoked.active);
            request.expected_revision = max;
            assert!(
                store
                    .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
                    .is_err()
            );
        }
        assert!(available(&store, &proof, &request).is_empty());
    }
}

#[test]
fn schema_48_upgrade_preserves_credentials_and_rejects_a_foreign_grant_table() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let (proof, _) = setup(&store);
        store
            .connection
            .execute_batch("DROP TABLE scoped_secret_grants; PRAGMA user_version=48;")
            .unwrap();
        if collision {
            store.connection.execute_batch("CREATE TABLE scoped_secret_grants(marker TEXT);INSERT INTO scoped_secret_grants VALUES('preserve me');").unwrap();
        }
        drop(store);
        let reopened = Store::open(&path);
        if collision {
            assert!(reopened.is_err());
            let raw = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                48
            );
            assert_eq!(
                raw.query_row("SELECT marker FROM scoped_secret_grants", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "preserve me"
            );
        } else {
            let store = reopened.unwrap();
            assert!(
                store
                    .secret_active_version(&proof.binding, "work-api")
                    .is_ok()
            );
            assert!(
                store
                    .scoped_secret_grants(&proof, "work-api", &|_| Ok(()))
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
