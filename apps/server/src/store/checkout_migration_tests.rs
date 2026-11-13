//! Schema 34 preserves prior observations; migration cannot approve a checkout.
use super::*;
use serde_json::json;
const DOWNGRADE: &str = "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; PRAGMA user_version=33;";
fn column(connection: &Connection, table: &str, name: &str) -> bool {
    let mut query = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    query
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .any(|r| r.unwrap() == name)
}
#[test]
fn checkout_migration_preserves_ordinary_binding_and_legacy_job_without_manufacturing_stamps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let user = store.create_user("existing-owner", "admin").unwrap();
    let (device, _) = store.create_device(&user.id, "old-browser").unwrap();
    let workspace = store.create_workspace(&user, "existing").unwrap();
    let logical = store
        .create_team_project(&user.id, &workspace.id, "logical")
        .unwrap();
    let native = store
        .create_project("ordinary", root.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &user.id).unwrap();
    let native_grant = store.capture_native_grant(&native.id, &user.id).unwrap();
    let binding = store
        .set_local_binding(&user.id, &workspace.id, &logical.id, &native.id, None)
        .unwrap();
    assert!(binding.metadata_approval_revision.is_none());
    // This is the historical payload: source checkout authority did not exist.
    let task:jobs::Task=serde_json::from_value(json!({
        "request":{"issue_id":"old-issue","expected_issue_digest":"a".repeat(64),"project_id":native.id,"idempotency_key":"old-job-approval","approved":true,"profile":{"harness":"opencode","model":"fixture","mode":"build","timeout_secs":30,"max_tool_requests":1,"allow_tools":true,"review_state":"In Review"}},
        "issue":{"id":"old-issue","identifier":"OLD-1","title":"Retained","description":"historical","url":"https://linear.app/fixture","status":"Todo","labels":[]},
        "root":native.path,"head":"b".repeat(40),"settings_digest":auth::hash_token("{}"),"device":device.id,"linear_revision":"historical-revision","native_grant":native_grant
    })).unwrap();
    assert!(task.source_checkout_stamp.is_none());
    let job = store.enqueue(&user.id, &task).unwrap();
    let before: (String, String) = store
        .connection
        .query_row(
            "SELECT task,fingerprint FROM relay_jobs WHERE id=?1",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!before.0.contains("source_checkout_stamp"));
    store.connection.execute_batch(DOWNGRADE).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(column(&store.connection, "relay_jobs", "checkout_stamp"));
    assert!(column(
        &store.connection,
        "team_local_bindings",
        "metadata_approval_revision"
    ));
    assert_eq!(
        store.capture_native_grant(&native.id, &user.id).unwrap(),
        native_grant
    );
    let binding = store
        .local_binding_for_user(&user.id, &workspace.id, &logical.id)
        .unwrap()
        .unwrap();
    assert!(binding.active);
    assert_eq!(binding.revision, 1);
    assert!(binding.metadata_approval_revision.is_none());
    let resolved = store
        .resolve_local_binding(
            &user.id,
            &workspace.id,
            &logical.id,
            dunlin_domain::collaboration::Permission::CreateSession,
        )
        .unwrap()
        .unwrap();
    assert_eq!(resolved.1.id, native.id);
    let record = store.job(&job.id).unwrap().unwrap();
    assert!(record.task.source_checkout_stamp.is_none());
    assert!(record.checkout_stamp.is_none());
    assert!(record.checkout_grant.is_none());
    let after: (String, String) = store
        .connection
        .query_row(
            "SELECT task,fingerprint FROM relay_jobs WHERE id=?1",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    for table in [
        "native_metadata_approvals",
        "native_metadata_grants",
        "native_metadata_audit",
        "native_metadata_revocations",
    ] {
        assert_eq!(
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    let record = store.job(&job.id).unwrap().unwrap();
    let state = crate::api::AppState::new(
        store,
        crate::config::Config {
            db: path,
            ..Default::default()
        },
    );
    assert!(
        crate::jobs::authorized(&state, &record)
            .unwrap_err()
            .contains("Legacy task")
    );
}
#[test]
fn checkout_migration_second_alter_collision_rolls_back_first_alter_and_recovers_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    store.connection.execute_batch(DOWNGRADE).unwrap();
    // The first ALTER adds checkout_stamp; force only the second to fail.
    store.connection.execute_batch("ALTER TABLE team_local_bindings ADD COLUMN metadata_approval_revision TEXT; CREATE TABLE migration34_sentinel(value TEXT); INSERT INTO migration34_sentinel VALUES('retained');").unwrap();
    drop(store);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert!(
        !column(&connection, "relay_jobs", "checkout_stamp"),
        "First ALTER must roll back with second ALTER failure"
    );
    assert!(column(
        &connection,
        "team_local_bindings",
        "metadata_approval_revision"
    ));
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        33
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM migration34_sentinel", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "retained"
    );
    // Remove the actual collision only; checkout_stamp was already rolled back.
    connection
        .execute_batch("ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision;")
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert!(column(&store.connection, "relay_jobs", "checkout_stamp"));
    assert!(column(
        &store.connection,
        "team_local_bindings",
        "metadata_approval_revision"
    ));
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    drop(store);
    Store::open(&path).unwrap();
}
