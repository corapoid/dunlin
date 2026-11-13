use super::*;

#[test]
fn version_28_upgrade_preserves_existing_admission_and_audit_and_adds_empty_profile_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.db");
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("upgrade-owner", "admin").unwrap();
    let device = store.create_device(&owner.id, "browser").unwrap().0;
    let workspace = store.create_workspace(&owner, "kept-workspace").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "kept-project")
        .unwrap();
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let receipt = store
        .interactive_journal()
        .create_admitted(
            &context,
            &workspace.id,
            &project.id,
            &device.id,
            "upgrade-admission-0001",
            &"a".repeat(64),
            "actor",
        )
        .unwrap();
    let expected = serde_json::to_value(&receipt.session).unwrap();
    let before: i64 = store
        .connection
        .query_row("SELECT count(*) FROM team_audit", [], |row| row.get(0))
        .unwrap();
    store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; ALTER TABLE team_audit DROP COLUMN resource_id; PRAGMA user_version=28;").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let replay = store
        .interactive_journal()
        .admission_lookup(
            &context,
            &device.id,
            "upgrade-admission-0001",
            &"a".repeat(64),
        )
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), expected);
    assert_eq!(
        store
            .interactive_journal()
            .session_profile(&context, &receipt.session.id)
            .unwrap(),
        None
    );
    assert!(
        store
            .agent_drafts(&owner.id, &workspace.id, &project.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .agent_profiles(&owner.id, &workspace.id, &project.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT count(*) FROM team_audit WHERE resource_id IS NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}
