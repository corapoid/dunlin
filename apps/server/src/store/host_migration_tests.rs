use super::*;
use dunlin_domain::agent::AgentDefinition;
#[test]
fn version_29_upgrade_preserves_published_agents_and_atomic_profile_admission_without_inventing_hosts()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host-upgrade.db");
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("host-upgrade-owner", "admin").unwrap();
    let device = store.create_device(&owner.id, "browser").unwrap().0;
    let workspace = store.create_workspace(&owner, "kept-workspace").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "kept-project")
        .unwrap();
    let definition:AgentDefinition=serde_json::from_value(serde_json::json!({"schema_version":1,"name":"Kept published agent","harness":"opencode","model":"fixture-model","mode":"plan","skills":[],"max_seconds":30,"max_output_bytes":32768})).unwrap();
    let draft = store
        .create_agent(&owner.id, &workspace.id, &project.id, definition)
        .unwrap();
    let version = store
        .publish_agent(&owner.id, &workspace.id, &project.id, &draft.id, 1)
        .unwrap();
    let pin = dunlin_domain::agent::AgentProfileRef {
        agent_id: version.agent_id.clone(),
        version: version.version,
        digest: version.digest.clone(),
    };
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let accepted = store
        .interactive_journal()
        .create_admitted_profile(
            &context,
            &workspace.id,
            &project.id,
            &device.id,
            "kept-profile-admission-0001",
            &"a".repeat(64),
            "actor",
            Some(&pin),
        )
        .unwrap();
    let original = serde_json::to_value(&accepted.session).unwrap();
    store
        .connection
        .execute_batch(
            "DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; PRAGMA user_version=29;",
        )
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.hosts(&owner.id, &workspace.id).unwrap().is_empty());
    let kept = store
        .agent_version(&owner.id, &workspace.id, &project.id, &version.agent_id, 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(kept).unwrap(),
        serde_json::to_value(version).unwrap()
    );
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    assert_eq!(
        store
            .interactive_journal()
            .session_profile(&context, &accepted.session.id)
            .unwrap(),
        Some(pin)
    );
    let replay = store
        .interactive_journal()
        .admission_lookup(
            &context,
            &device.id,
            "kept-profile-admission-0001",
            &"a".repeat(64),
        )
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), original);
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}

#[test]
fn version_30_upgrade_preserves_enrolled_worker_and_starts_without_synthetic_channel_authority() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("channels.db");
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("channel-upgrade-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Kept hosts").unwrap();
    let definition=serde_json::from_value(serde_json::json!({"name":"Kept worker","node_id":"n-kept","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap();
    let pending = store
        .create_host(&owner.id, &workspace.id, definition)
        .unwrap();
    let capabilities=serde_json::from_value(serde_json::json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"none","max_parallel":1})).unwrap();
    let credential = store
        .enroll_host(&pending.enrollment.ticket, capabilities)
        .unwrap();
    let approved = store
        .approve_host(&owner.id, &workspace.id, &credential.host_id, 2)
        .unwrap();
    let before = serde_json::to_value(approved).unwrap();
    store
        .connection
        .execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; PRAGMA user_version=30;")
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(
            store
                .host(&owner.id, &workspace.id, &credential.host_id)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        before
    );
    let identity = store
        .authenticate_worker_token(&credential.credential)
        .unwrap();
    assert_eq!(identity.host_id(), credential.host_id);
    assert!(
        store
            .worker_channel_diagnostic(&owner.id, &workspace.id, &credential.host_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}

#[test]
fn version_31_upgrade_preserves_local_targets_and_worker_fences_without_inventing_remote_bindings()
{
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("remote-upgrade.db");
    let checkout = directory.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("remote-upgrade", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Kept workspace").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Kept project")
        .unwrap();
    let native = store
        .create_project("Kept local", checkout.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &owner.id).unwrap();
    let binding = store
        .set_local_binding(&owner.id, &workspace.id, &project.id, &native.id, None)
        .unwrap();
    let host=store.create_host(&owner.id,&workspace.id,serde_json::from_value(serde_json::json!({"name":"Kept worker","node_id":"n-kept","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":[]}})).unwrap()).unwrap();
    let credential=store.enroll_host(&host.enrollment.ticket,serde_json::from_value(serde_json::json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":[],"sandbox":"none","max_parallel":1})).unwrap()).unwrap();
    store
        .approve_host(&owner.id, &workspace.id, &credential.host_id, 2)
        .unwrap();
    let identity = store
        .authenticate_worker_token(&credential.credential)
        .unwrap();
    let lease = store
        .open_worker_channel(
            &identity,
            "old-controller",
            "worker-boot",
            "kept-channel",
            1000,
            10000,
        )
        .unwrap();
    store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; PRAGMA user_version=31;").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let kept = store
        .local_binding_for_user(&owner.id, &workspace.id, &project.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(kept).unwrap(),
        serde_json::to_value(binding).unwrap()
    );
    assert!(
        store
            .remote_bindings(&owner.id, &workspace.id, &project.id)
            .unwrap()
            .is_empty()
    );
    let identity = store
        .authenticate_worker_token(&credential.credential)
        .unwrap();
    assert!(
        store
            .validate_worker_channel(&identity, &lease, "new-controller", 1001)
            .is_err()
    );
    let diagnostic = store
        .worker_channel_diagnostic(&owner.id, &workspace.id, &credential.host_id)
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.lease.channel_id, lease.channel_id);
    assert_eq!(diagnostic.lease.epoch, lease.epoch);
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}
