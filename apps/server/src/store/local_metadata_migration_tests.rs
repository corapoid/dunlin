use super::*;

const DOWNGRADE: &str = "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials;
DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations;
DROP TABLE native_metadata_grants;
DROP TABLE native_metadata_audit;
DROP TABLE native_metadata_revocations;
DROP TABLE native_metadata_approvals;
PRAGMA user_version=32;
";

#[test]
fn metadata_migration_preserves_native_grants_without_manufacturing_metadata_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let user = store.create_user("existing-member", "member").unwrap();
    let project = store
        .create_project("existing", "/unavailable/checkout")
        .unwrap();
    store.grant(&project.id, &user.id).unwrap();
    let stamp = store.capture_native_grant(&project.id, &user.id).unwrap();
    store.connection.execute_batch(DOWNGRADE).unwrap();
    drop(store);

    let store = Store::open(&path).unwrap();
    store.revalidate_native_grant(&stamp).unwrap();
    assert_eq!(
        store.capture_native_grant(&project.id, &user.id).unwrap(),
        stamp
    );
    for table in [
        "native_metadata_approvals",
        "native_metadata_grants",
        "native_metadata_audit",
        "native_metadata_revocations",
        "team_local_binding_revocations",
    ] {
        let count: i64 = store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "migration must not create authority or receipts in {table}"
        );
    }
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    drop(store);
    Store::open(&path)
        .unwrap()
        .revalidate_native_grant(&stamp)
        .unwrap();
}

#[test]
fn failed_metadata_migration_rolls_back_all_new_tables_and_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    store.connection.execute_batch(DOWNGRADE).unwrap();
    // The last schema in this migration collides, after metadata DDL ran.
    store
        .connection
        .execute_batch("CREATE TABLE team_local_binding_revocations(sentinel TEXT);")
        .unwrap();
    drop(store);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        32
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'native_metadata_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TABLE team_local_binding_revocations;")
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}
