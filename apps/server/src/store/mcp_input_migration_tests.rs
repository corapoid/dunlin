use super::*;

#[test]
fn version_40_upgrade_preserves_sessions_and_creates_empty_mcp_input_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.db");
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("input-upgrade-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "kept-workspace").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "kept-project")
        .unwrap();
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let session = store
        .interactive_journal()
        .create(&context, &workspace.id, &project.id)
        .unwrap();
    let expected = serde_json::to_value(&session).unwrap();
    store
        .connection
        .execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup; ALTER TABLE mcp_oauth_connections DROP COLUMN revocation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN credential_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN binding; ALTER TABLE mcp_oauth_secret_references DROP COLUMN generation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN sha256; DROP TABLE interactive_mcp_inputs; PRAGMA user_version=40;")
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let context = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let (actual, turns) = store
        .interactive_journal()
        .snapshot(&context, &session.id)
        .unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    assert!(turns.is_empty());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM interactive_mcp_inputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}

#[test]
fn version_41_collision_rolls_back_without_accepting_or_replacing_foreign_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collision.db");
    let store = Store::open(&path).unwrap();
    store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup; ALTER TABLE mcp_oauth_connections DROP COLUMN revocation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN credential_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN binding; ALTER TABLE mcp_oauth_secret_references DROP COLUMN generation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN sha256; DROP TABLE interactive_mcp_inputs; CREATE TABLE interactive_mcp_inputs(collision_marker TEXT); INSERT INTO interactive_mcp_inputs VALUES('preserve me'); PRAGMA user_version=40;").unwrap();
    drop(store);
    assert!(Store::open(&path).is_err());
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        40
    );
    assert_eq!(
        c.query_row(
            "SELECT collision_marker FROM interactive_mcp_inputs",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preserve me"
    );
    assert_eq!(c.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='interactive_mcp_input_parent'", [], |r|r.get::<_,i64>(0)).unwrap(),0);
}
