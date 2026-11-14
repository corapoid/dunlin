//! v50 separates the controller-local target from enrolled workers. No host row
//! or filesystem capability is manufactured. Local use still needs the actual
//! native checkout authority held by the managed runtime.
use super::*;
const CREDENTIALS_V50: &str = "
CREATE TABLE scoped_credentials (
 workspace TEXT NOT NULL, project TEXT NOT NULL, host TEXT NOT NULL,
 credential TEXT NOT NULL CHECK(length(credential) BETWEEN 1 AND 128),
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 state TEXT NOT NULL CHECK(state IN ('installing','active','disabled')),
 active_reference TEXT, staged_reference TEXT, updated_at INTEGER NOT NULL,
 last_disable TEXT CHECK(last_disable IS NULL OR length(last_disable)<=4096),
 remote_host TEXT REFERENCES team_hosts(id), local_workspace TEXT,local_project TEXT,
 PRIMARY KEY(workspace,project,host,credential),
 FOREIGN KEY(workspace,project) REFERENCES team_projects(workspace_id,id),
 FOREIGN KEY(local_workspace,local_project) REFERENCES team_local_bindings(workspace_id,project_id),
 CHECK((host='local' AND remote_host IS NULL AND local_workspace=workspace AND local_project=project)
    OR (host!='local' AND remote_host=host AND remote_host IS NOT NULL AND local_workspace IS NULL AND local_project IS NULL)),
 CHECK((host!='local') OR (local_workspace IS NOT NULL AND local_project IS NOT NULL)),
 CHECK((state='installing' AND staged_reference IS NOT NULL)
    OR (state='active' AND active_reference IS NOT NULL AND staged_reference IS NULL)
    OR (state='disabled' AND staged_reference IS NULL))
);
";
fn columns(tx: &Transaction<'_>, table: &str, expected: &[&str]) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare(&format!("PRAGMA main.table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if names != expected {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}
pub(crate) fn migrate(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    columns(
        tx,
        "scoped_credentials",
        &[
            "workspace",
            "project",
            "host",
            "credential",
            "revision",
            "state",
            "active_reference",
            "staged_reference",
            "updated_at",
            "last_disable",
        ],
    )?;
    columns(
        tx,
        "scoped_secret_references",
        &[
            "reference",
            "workspace",
            "project",
            "host",
            "credential",
            "revision",
            "sha256",
            "writer_user",
            "writer_device",
            "browser_hash",
            "membership_revision",
            "grant_revision",
            "host_revision",
            "write_ack",
            "activated",
            "created_at",
        ],
    )?;
    columns(
        tx,
        "scoped_secret_grants",
        &[
            "id",
            "workspace",
            "project",
            "host",
            "credential",
            "principal_kind",
            "principal_id",
            "agent_id",
            "agent_version",
            "profile_digest",
            "provider_id",
            "revision",
            "active",
            "credential_revision",
            "membership_revision",
            "project_revision",
            "host_revision",
            "read_only",
            "updated_by",
            "updated_at",
            "last_revoke",
        ],
    )?;
    tx.execute_batch("CREATE TEMP TABLE _scoped_credentials_v49 AS SELECT * FROM main.scoped_credentials;
CREATE TEMP TABLE _scoped_references_v49 AS SELECT * FROM main.scoped_secret_references;
CREATE TEMP TABLE _scoped_grants_v49 AS SELECT * FROM main.scoped_secret_grants;
DROP TABLE main.scoped_secret_grants;DROP TABLE main.scoped_secret_references;DROP TABLE main.scoped_credentials;")?;
    tx.execute_batch(CREDENTIALS_V50)?;
    tx.execute_batch(REFERENCES_SCHEMA)?;
    tx.execute_batch(grants::SCHEMA)?;
    tx.execute_batch("INSERT INTO scoped_credentials SELECT c.*,c.host,NULL,NULL FROM temp._scoped_credentials_v49 c;
INSERT INTO scoped_secret_references SELECT * FROM temp._scoped_references_v49;
INSERT INTO scoped_secret_grants SELECT * FROM temp._scoped_grants_v49;
DROP TABLE temp._scoped_grants_v49;DROP TABLE temp._scoped_references_v49;DROP TABLE temp._scoped_credentials_v49;")?;
    for table in [
        "scoped_credentials",
        "scoped_secret_references",
        "scoped_secret_grants",
    ] {
        let bad: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check('{table}'))"),
            [],
            |r| r.get(0),
        )?;
        if bad {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn fixture_v49(store: &Store) -> rusqlite::Result<()> {
    let tx = store.connection.unchecked_transaction()?;
    tx.execute_batch("CREATE TEMP TABLE _c50 AS SELECT workspace,project,host,credential,revision,state,active_reference,staged_reference,updated_at,last_disable FROM scoped_credentials;
CREATE TEMP TABLE _r50 AS SELECT * FROM scoped_secret_references;CREATE TEMP TABLE _g50 AS SELECT * FROM scoped_secret_grants;
DROP TABLE scoped_secret_grants;DROP TABLE scoped_secret_references;DROP TABLE scoped_credentials;")?;
    create_v48(&tx)?;
    tx.execute_batch(grants::SCHEMA)?;
    tx.execute_batch("INSERT INTO scoped_credentials SELECT * FROM temp._c50;INSERT INTO scoped_secret_references SELECT * FROM temp._r50;INSERT INTO scoped_secret_grants SELECT * FROM temp._g50;DROP TABLE temp._g50;DROP TABLE temp._r50;DROP TABLE temp._c50;PRAGMA user_version=49;")?;
    tx.commit()
}
#[cfg(test)]
mod tests;
