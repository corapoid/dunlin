use super::*;
use dunlin_domain::agent::AgentProfileRef;
use serde_json::json;
fn setup(store: &Store, root: &std::path::Path) -> (Proof, GrantRequest, String) {
    let owner = store.create_user("local-secret-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Local secret").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Local")
        .unwrap();
    let native = store
        .create_project("native", root.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &owner.id).unwrap();
    store
        .set_local_binding(&owner.id, &workspace.id, &project.id, &native.id, None)
        .unwrap();
    let (device, _) = store.create_device(&owner.id, "browser").unwrap();
    let cookie = store.create_browser_session(&device.id, 600).unwrap();
    let proof = store
        .scoped_secret_proof(
            BrowserProof {
                user: owner.id.clone(),
                device: device.id,
                session_hash: crate::auth::hash_token(&cookie),
            },
            Binding {
                workspace: workspace.id,
                project: project.id,
                host: "local".into(),
            },
        )
        .unwrap();
    let candidate = super::super::tests::candidate(&proof, 0);
    let claim = store
        .scoped_secret_reserve(&proof, &candidate, &|_| Ok(()))
        .unwrap();
    store.scoped_secret_acknowledge(&proof, &claim).unwrap();
    store
        .scoped_secret_activate(&proof, claim, &|_| Ok(()))
        .unwrap();
    let definition=serde_json::from_value(json!({"schema_version":2,"name":"Local secret consumer","harness":"opencode","model":"fixture-model","mode":"plan","skills":[],"mcp":[{"provider_id":"dunlin-secret-local","name":"http","definition_digest":dunlin_vault::scoped::tool::http().definition_digest().unwrap()}],"max_seconds":30,"max_output_bytes":65536})).unwrap();
    let draft = store
        .create_agent(
            &owner.id,
            &proof.binding.workspace,
            &proof.binding.project,
            definition,
        )
        .unwrap();
    let p = store
        .publish_agent(
            &owner.id,
            &proof.binding.workspace,
            &proof.binding.project,
            &draft.id,
            1,
        )
        .unwrap();
    let request = GrantRequest {
        expected_revision: 0,
        credential_revision: 1,
        principal_kind: Principal::Human,
        principal_id: owner.id,
        profile: AgentProfileRef {
            agent_id: p.agent_id,
            version: p.version,
            digest: p.digest,
        },
        provider_id: "dunlin-secret-local".into(),
        read_only: true,
    };
    (proof, request, native.id)
}
#[test]
fn local_scoped_credential_has_native_foreign_key_without_synthetic_worker_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (proof, request, _) = setup(&store, &root);
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM team_hosts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let target: (Option<String>, String, String) = store
        .connection
        .query_row(
            "SELECT remote_host,local_workspace,local_project FROM scoped_credentials",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        target,
        (
            None,
            proof.binding.workspace.clone(),
            proof.binding.project.clone()
        )
    );
    drop(store);
    let store = Store::open(path).unwrap();
    let available = store
        .scoped_secret_available(
            &request.principal_id,
            &proof.binding,
            &request.profile,
            &|_| Ok(()),
        )
        .unwrap();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].id, grant.id);
    assert!(
        store
            .connection
            .execute(
                "UPDATE scoped_credentials SET remote_host='invented-worker'",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "UPDATE scoped_credentials SET local_project='other-project'",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute("DELETE FROM team_local_bindings", [])
            .is_err()
    );
}
#[test]
fn local_binding_revoke_regrant_requires_explicit_secret_regrant_and_negative_cut_remains_available()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let (proof, mut request, native) = setup(&store, dir.path());
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    let b = store
        .revoke_local_binding(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            1,
        )
        .unwrap();
    assert!(!b.active);
    assert!(
        store
            .scoped_secret_available(
                &request.principal_id,
                &proof.binding,
                &request.profile,
                &|_| Ok(())
            )
            .unwrap()
            .is_empty()
    );
    let fresh = store
        .scoped_secret_proof(proof.browser.clone(), proof.binding.clone())
        .unwrap();
    assert!(
        store
            .scoped_secret_reserve(&fresh, &super::super::tests::candidate(&proof, 1), &|_| Ok(
                ()
            ))
            .is_err()
    );
    let revoked = store
        .scoped_secret_revoke_grant(&fresh, "work-api", &grant.id, 1, &|_| Ok(()))
        .unwrap();
    assert!(!revoked.active);
    store
        .set_local_binding(
            &proof.browser.user,
            &proof.binding.workspace,
            &proof.binding.project,
            &native,
            Some(b.revision),
        )
        .unwrap();
    assert!(
        store
            .scoped_secret_available(
                &request.principal_id,
                &proof.binding,
                &request.profile,
                &|_| Ok(())
            )
            .unwrap()
            .is_empty()
    );
    request.expected_revision = revoked.revision;
    let fresh = store
        .scoped_secret_proof(proof.browser.clone(), proof.binding.clone())
        .unwrap();
    store
        .scoped_secret_set_grant(&fresh, "work-api", &request, &|_| Ok(()))
        .unwrap();
    assert_eq!(
        store
            .scoped_secret_available(
                &request.principal_id,
                &proof.binding,
                &request.profile,
                &|_| Ok(())
            )
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn revoked_native_grant_denies_local_catalog_but_cannot_block_manager_disabling_credential() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let (proof, request, native) = setup(&store, dir.path());
    store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    store.revoke_grant(&native, &proof.browser.user).unwrap();
    assert!(
        store
            .scoped_secret_available(
                &request.principal_id,
                &proof.binding,
                &request.profile,
                &|_| Ok(())
            )
            .unwrap()
            .is_empty()
    );
    let fresh = store
        .scoped_secret_proof(proof.browser.clone(), proof.binding.clone())
        .unwrap();
    let disabled = store
        .scoped_secret_disable(&fresh, "work-api", 1, &|_| Ok(()))
        .unwrap();
    assert_eq!(disabled.state, "disabled");
}
#[test]
fn local_grant_cannot_be_resolved_for_a_remote_target_or_another_project() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let (proof, request, _) = setup(&store, dir.path());
    let grant = store
        .scoped_secret_set_grant(&proof, "work-api", &request, &|_| Ok(()))
        .unwrap();
    let context = store
        .team_project_context(
            &proof.binding.workspace,
            &proof.binding.project,
            &request.principal_id,
        )
        .unwrap();
    for change in ["host", "project"] {
        let mut b = proof.binding.clone();
        if change == "host" {
            b.host = "worker".into();
        } else {
            b.project = "other".into();
        }
        assert!(
            store
                .resolve_secret_grant(&context, &b, &request.profile, &grant.id, 1)
                .is_err()
        );
    }
}
#[test]
fn migration49_preserves_remote_ledger_receipts_and_rejects_unknown_shape_without_deletion() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let proof = super::super::tests::scope(&store);
        let c = super::super::tests::candidate(&proof, 0);
        let claim = store
            .scoped_secret_reserve(&proof, &c, &|_| Ok(()))
            .unwrap();
        store.scoped_secret_acknowledge(&proof, &claim).unwrap();
        store
            .scoped_secret_activate(&proof, claim, &|_| Ok(()))
            .unwrap();
        store
            .scoped_secret_disable(&proof, "work-api", 1, &|_| Ok(()))
            .unwrap();
        let before: String = store
            .connection
            .query_row("SELECT last_disable FROM scoped_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        fixture_v49(&store).unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE scoped_credentials ADD COLUMN foreign_marker TEXT;UPDATE scoped_credentials SET foreign_marker='preserve';").unwrap();
        }
        drop(store);
        let reopened = Store::open(&path);
        if collision {
            assert!(reopened.is_err());
            let raw = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                49
            );
            assert_eq!(
                raw.query_row("SELECT foreign_marker FROM scoped_credentials", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "preserve"
            );
        } else {
            let store = reopened.unwrap();
            assert_eq!(
                store
                    .connection
                    .query_row("SELECT last_disable FROM scoped_credentials", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap(),
                before
            );
            assert_eq!(store.connection.query_row("SELECT count(*) FROM scoped_secret_references WHERE write_ack=1 AND activated=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            let remote: String = store
                .connection
                .query_row("SELECT remote_host FROM scoped_credentials", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(remote, proof.binding.host);
            assert!(
                store
                    .connection
                    .execute("UPDATE scoped_credentials SET remote_host='unknown'", [])
                    .is_err()
            );
        }
    }
}
