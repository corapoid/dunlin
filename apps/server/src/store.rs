//! Relay metadata store.
//!
//! Accounts, devices, grants and a session index live here. When
//! `store_transcripts` is enabled, rendered transcript snapshots are also kept
//! for recovery; `store_transcripts = false` keeps metadata only.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::auth;
pub(crate) mod ahp;
pub(crate) mod sharing;
pub(crate) use ahp::AhpAccess;
pub(crate) mod local_metadata;
pub(crate) mod native_checkout;
mod native_grants;
pub use native_grants::NativeGrantStamp;
#[cfg(test)]
mod agent_migration_tests;
pub(crate) mod agents;
mod bindings;
#[cfg(test)]
mod mcp_input_migration_tests;
pub(crate) use bindings::Binding;
mod browser_sessions;
#[cfg(test)]
mod checkout_migration_tests;
mod host_health;
#[cfg(test)]
mod host_migration_tests;
pub(crate) use host_health::HostHealthObservation;
pub(crate) mod hosts;
mod invitations;
pub(crate) mod jobs;
mod linear;
#[cfg(test)]
mod local_metadata_migration_tests;
pub(crate) mod mcp_oauth;
mod oidc;
pub(crate) mod remote_bindings;
pub(crate) mod scoped_secrets;
pub(crate) mod service_principals;
pub(crate) mod team_projects;
pub(crate) mod teams;
pub(crate) mod worker_channels;
pub(crate) mod workflows;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid stored team metadata")]
    Encoding(#[from] serde_json::Error),
    #[error("{0}")]
    Limit(String),
    #[error("{0}")]
    Policy(#[from] dunlin_domain::collaboration::PolicyError),
    #[error("{0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub(crate) const SCHEMA_VERSION: i64 = 49;

const MAX_SESSIONS_PER_USER: i64 = 1000;

fn positive_revision(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(rusqlite::Error::InvalidQuery)
}

pub use dunlin_relay_protocol::{Project, User};

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub last_device_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionOwner {
    pub user_id: String,
    pub project_id: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BillingEvidenceRow {
    pub user_id: String,
    pub user_name: String,
    pub device_id: String,
    pub device_name: String,
    pub payload: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct ArtifactRow {
    pub id: String,
    pub project_id: String,
    pub user_id: String,
    pub kind: String,
    pub title: String,
    pub mime: String,
    pub bytes: i64,
    pub sha256: String,
    pub blob_path: String,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub struct ArtifactShareRow {
    pub artifact_id: String,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

pub struct Store {
    connection: Connection,
    oauth_revocation_owner: std::sync::Arc<std::sync::atomic::AtomicBool>,
    oauth_writer_owner: std::sync::Arc<std::sync::atomic::AtomicBool>,
    catalog_changes: tokio::sync::broadcast::Sender<()>,
    interactive_changes: tokio::sync::broadcast::Sender<()>,
}

impl Store {
    /// Payload-free invalidation; consumers must reread their own authorized scope.
    pub fn subscribe_catalog(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.catalog_changes.subscribe()
    }

    pub(crate) fn subscribe_interactive(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.interactive_changes.subscribe()
    }
    fn interactive_changed(&self) {
        let _ = self.interactive_changes.send(());
    }
    fn catalog_changed(&self) {
        let _ = self.catalog_changes.send(());
        self.interactive_changed();
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let store = Self {
            connection,
            oauth_revocation_owner: Default::default(),
            oauth_writer_owner: Default::default(),
            catalog_changes: tokio::sync::broadcast::channel(64).0,
            interactive_changes: tokio::sync::broadcast::channel(64).0,
        };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            connection,
            oauth_revocation_owner: Default::default(),
            oauth_writer_owner: Default::default(),
            catalog_changes: tokio::sync::broadcast::channel(64).0,
            interactive_changes: tokio::sync::broadcast::channel(64).0,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::Limit(
                "Relay database is newer than this server; refusing to modify it".into(),
            ));
        }
        if version < 1 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS users (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL UNIQUE,
                    role        TEXT NOT NULL CHECK(role IN ('admin','member')),
                    created_at  INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS devices (
                    id            TEXT PRIMARY KEY,
                    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    name          TEXT NOT NULL,
                    token_hash    TEXT NOT NULL UNIQUE,
                    created_at    INTEGER NOT NULL,
                    last_seen_at  INTEGER,
                    revoked_at    INTEGER
                );
                CREATE INDEX IF NOT EXISTS devices_user_idx ON devices(user_id);

                CREATE TABLE IF NOT EXISTS pairing_codes (
                    id          TEXT PRIMARY KEY,
                    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    code_hash   TEXT NOT NULL UNIQUE,
                    expires_at  INTEGER NOT NULL,
                    used_at     INTEGER
                );

                CREATE TABLE IF NOT EXISTS projects (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL UNIQUE,
                    path        TEXT NOT NULL UNIQUE,
                    created_at  INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS project_grants (
                    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    PRIMARY KEY (project_id, user_id)
                );

                CREATE TABLE IF NOT EXISTS sessions (
                    id              TEXT PRIMARY KEY,
                    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    title           TEXT NOT NULL,
                    created_at      INTEGER NOT NULL,
                    updated_at      INTEGER NOT NULL,
                    last_device_id  TEXT
                );
                CREATE INDEX IF NOT EXISTS sessions_user_project_idx
                    ON sessions(user_id, project_id, updated_at DESC);

                PRAGMA user_version = 1;
                "#,
            )?;
        }
        if version < 2 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS transcripts (
                    session_id  TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                    payload     TEXT NOT NULL,
                    updated_at  INTEGER NOT NULL
                );

                PRAGMA user_version = 2;
                "#,
            )?;
        }
        if version < 3 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS project_settings (
                    project_id  TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
                    payload     TEXT NOT NULL,
                    updated_at  INTEGER NOT NULL
                );

                PRAGMA user_version = 3;
                "#,
            )?;
        }
        if version < 4 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS billing_evidence (
                    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    device_id   TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
                    payload     TEXT NOT NULL,
                    updated_at  INTEGER NOT NULL,
                    PRIMARY KEY (user_id, device_id)
                );

                PRAGMA user_version = 4;
                "#,
            )?;
        }
        if version < 5 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS relay_artifacts (
                    id          TEXT PRIMARY KEY,
                    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    kind        TEXT NOT NULL,
                    title       TEXT NOT NULL,
                    mime        TEXT NOT NULL,
                    bytes       INTEGER NOT NULL,
                    sha256      TEXT NOT NULL,
                    blob_path   TEXT NOT NULL,
                    created_at  INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS relay_artifacts_project_idx
                    ON relay_artifacts(project_id, created_at DESC);

                CREATE TABLE IF NOT EXISTS relay_artifact_shares (
                    token_hash  TEXT PRIMARY KEY,
                    artifact_id TEXT NOT NULL REFERENCES relay_artifacts(id) ON DELETE CASCADE,
                    created_at  INTEGER NOT NULL,
                    expires_at  INTEGER NOT NULL,
                    revoked_at  INTEGER
                );
                CREATE INDEX IF NOT EXISTS relay_artifact_shares_artifact_idx
                    ON relay_artifact_shares(artifact_id);

                PRAGMA user_version = 5;
                "#,
            )?;
        }
        if version < 6 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch("ALTER TABLE transcripts ADD COLUMN revision INTEGER NOT NULL DEFAULT 0; PRAGMA user_version = 6;")?;
            transaction.commit()?;
        }
        if version < 7 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                "CREATE TABLE IF NOT EXISTS linear_integrations (
                    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
                    credential_id TEXT NOT NULL UNIQUE,
                    revision TEXT NOT NULL,
                    filter TEXT NOT NULL,
                    inbox TEXT NOT NULL,
                    attempted_at INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS linear_credential_cleanup (credential_id TEXT PRIMARY KEY);
                PRAGMA user_version = 7;"
            )?;
            transaction.commit()?;
        }
        if version < 8 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch("CREATE TABLE IF NOT EXISTS relay_jobs (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL UNIQUE,
                owner TEXT NOT NULL REFERENCES users(id),project TEXT NOT NULL REFERENCES projects(id),
                issue TEXT NOT NULL,idem TEXT NOT NULL,fingerprint TEXT NOT NULL,task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',cancel INTEGER NOT NULL DEFAULT 0,
                report TEXT NOT NULL DEFAULT '',checkout_project TEXT,branch TEXT,
                sync_state TEXT NOT NULL DEFAULT 'none',sync_error TEXT,sync_after INTEGER NOT NULL DEFAULT 0,position INTEGER NOT NULL,
                UNIQUE(owner,idem));
                CREATE UNIQUE INDEX IF NOT EXISTS relay_jobs_one_runner ON relay_jobs((1)) WHERE status IN ('preparing','running');
                CREATE UNIQUE INDEX IF NOT EXISTS relay_jobs_active_issue ON relay_jobs(issue) WHERE status IN ('queued','preparing','running','unknown');
                CREATE TABLE IF NOT EXISTS relay_queue_state (id INTEGER PRIMARY KEY CHECK(id=1),paused INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS relay_job_approvals (owner TEXT NOT NULL,idem TEXT NOT NULL,fingerprint TEXT NOT NULL,job TEXT NOT NULL,PRIMARY KEY(owner,idem));
                INSERT OR IGNORE INTO relay_queue_state VALUES (1,0);
                PRAGMA user_version=8;")?;
            transaction.commit()?;
        }
        if version < 9 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(teams::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 10 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(team_projects::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 11 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(workflows::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 12 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(browser_sessions::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 13 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(invitations::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 14 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(oidc::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 15 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(service_principals::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 16 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(dunlin_session_runtime::SCHEMA)?;
            transaction.execute_batch("PRAGMA user_version=16;")?;
            transaction.commit()?;
        }
        if version < 17 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(bindings::MIGRATION)?;
            transaction.commit()?;
        }
        if version < 18 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(dunlin_session_runtime::STREAM_SCHEMA)?;
            transaction.execute_batch("PRAGMA user_version=18;")?;
            transaction.commit()?;
        }
        if version < 19 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(dunlin_session_runtime::PROJECTION_SCHEMA)?;
            self.interactive_journal()
                .backfill_projection()
                .map_err(|_| StoreError::Limit("AHP projection migration failed".into()))?;
            transaction.execute_batch("PRAGMA user_version=19;")?;
            transaction.commit()?;
        }
        if version < 20 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::CHANNEL_SCHEMA)?;
            self.interactive_journal()
                .backfill_channels()
                .map_err(|_| StoreError::Limit("AHP channel migration failed".into()))?;
            tx.execute_batch("PRAGMA user_version=20;")?;
            tx.commit()?;
        }
        if version < 21 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::DISPATCH_SCHEMA)?;
            self.interactive_journal()
                .backfill_dispatch()
                .map_err(|_| StoreError::Limit("AHP dispatch migration failed".into()))?;
            tx.execute_batch("PRAGMA user_version=21;")?;
            tx.commit()?;
        }
        if version < 22 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::CONTROL_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=22;")?;
            tx.commit()?;
        }
        if version < 23 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::PERMISSION_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=23;")?;
            tx.commit()?;
        }
        if version < 24 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::SHARING_SCHEMA)?;
            tx.execute_batch("ALTER TABLE interactive_ahp_clients ADD COLUMN visibility TEXT NOT NULL DEFAULT '{}'; PRAGMA user_version=24;")?;
            tx.commit()?;
        }
        if version < 25 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(native_grants::MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=25;")?;
            tx.commit()?;
        }
        if version < 26 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(
                "ALTER TABLE relay_jobs ADD COLUMN checkout_grant TEXT; PRAGMA user_version=26;",
            )?;
            tx.commit()?;
        }
        if version < 27 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE interactive_ahp_clients ADD COLUMN views TEXT NOT NULL DEFAULT '{}'; PRAGMA user_version=27;")?;
            tx.commit()?;
        }
        if version < 28 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::ADMISSION_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=28;")?;
            tx.commit()?;
        }
        if version < 29 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE team_audit ADD COLUMN resource_id TEXT;")?;
            tx.execute_batch(agents::AGENT_SCHEMA)?;
            tx.execute_batch(dunlin_session_runtime::ADMISSION_PROFILE_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=29;")?;
            tx.commit()?;
        }
        if version < 30 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(hosts::HOST_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=30;")?;
            tx.commit()?;
        }
        if version < 31 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(worker_channels::WORKER_CHANNEL_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=31;")?;
            tx.commit()?;
        }
        if version < 32 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(remote_bindings::REMOTE_BINDING_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=32;")?;
            tx.commit()?;
        }
        if version < 33 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(local_metadata::SCHEMA)?;
            tx.execute_batch(bindings::REVOCATION_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=33;")?;
            tx.commit()?;
        }
        if version < 34 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE relay_jobs ADD COLUMN checkout_stamp TEXT; ALTER TABLE team_local_bindings ADD COLUMN metadata_approval_revision INTEGER CHECK(metadata_approval_revision IS NULL OR (metadata_approval_revision BETWEEN 1 AND 9007199254740991)); PRAGMA user_version=34;")?;
            tx.commit()?;
        }
        if version < 35 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(remote_bindings::resolver::execution::REMOTE_EXECUTION_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=35;")?;
            tx.commit()?;
        }
        if version < 36 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(remote_bindings::resolver::sessions::SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=36;")?;
            tx.commit()?;
        }
        if version < 37 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(remote_bindings::resolver::sessions::LIFECYCLE_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=37;")?;
            tx.commit()?;
        }
        if version < 38 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=38;")?;
            tx.commit()?;
        }
        if version < 39 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::AUTHORIZATION_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=39;")?;
            tx.commit()?;
        }
        if version < 40 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE mcp_oauth_connections ADD COLUMN display_name TEXT CHECK(display_name IS NULL OR length(display_name) BETWEEN 1 AND 120); PRAGMA user_version=40;")?;
            tx.commit()?;
        }
        if version < 41 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(dunlin_session_runtime::MCP_INPUT_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=41;")?;
            tx.commit()?;
        }
        if version < 42 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::WRITE_METADATA_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=42;")?;
            tx.commit()?;
        }
        if version < 43 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::REVOCATION_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=43;")?;
            tx.commit()?;
        }
        if version < 44 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::CLEANUP_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=44;")?;
            tx.commit()?;
        }
        if version < 45 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE mcp_oauth_secret_references ADD COLUMN writer_protected INTEGER NOT NULL DEFAULT 0 CHECK(writer_protected IN (0,1)); PRAGMA user_version=45;")?;
            tx.commit()?;
        }
        if version < 46 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::WRITE_RECEIPT_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=46;")?;
            tx.commit()?;
        }
        if version < 47 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(mcp_oauth::RETENTION_MIGRATION)?;
            tx.execute_batch("PRAGMA user_version=47;")?;
            tx.commit()?;
        }
        if version < 48 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(scoped_secrets::SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=48;")?;
            tx.commit()?;
        }
        if version < 49 {
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(scoped_secrets::GRANTS_SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=49;")?;
            tx.commit()?;
        }
        Ok(())
    }

    pub(crate) fn interactive_scope(&self, id: &str) -> Result<(String, String)> {
        self.connection
            .query_row(
                "SELECT workspace,project FROM interactive_sessions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| dunlin_domain::collaboration::PolicyError::PermissionDenied.into())
    }

    pub(crate) fn interactive_journal(&self) -> dunlin_session_runtime::Journal<'_> {
        dunlin_session_runtime::Journal::new(&self.connection)
            .notifying(|| self.interactive_changed())
            .validating_shared_owner(|stamp| {
                let owner = self
                    .team_project_context(&stamp.workspace_id, &stamp.project_id, &stamp.user_id)
                    .map_err(|_| {
                        dunlin_session_runtime::Error::Policy(
                            dunlin_domain::collaboration::PolicyError::PermissionDenied,
                        )
                    })?;
                owner
                    .revalidate(
                        stamp,
                        dunlin_domain::collaboration::Permission::ShareOwnSession,
                    )
                    .map(|_| ())
                    .map_err(Into::into)
            })
    }

    /// Called only by the server coordinator, after binding its listener.
    pub fn recover_interactive_sessions(&self) -> Result<usize> {
        dunlin_session_runtime::Journal::new(&self.connection)
            .recover()
            .map_err(|_| StoreError::Limit("Interactive session recovery failed".into()))
    }

    /// Bounded aggregate billing evidence. Raw provider logs and prompts are
    /// never accepted or stored here.
    pub fn save_billing_evidence(
        &self,
        user_id: &str,
        device_id: &str,
        payload: &str,
    ) -> Result<()> {
        if payload.len() > 2 * 1024 * 1024 {
            return Err(StoreError::Limit(
                "Billing evidence exceeds the 2 MiB relay limit".into(),
            ));
        }
        self.connection.execute(
            r#"
            INSERT INTO billing_evidence (user_id, device_id, payload, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(user_id, device_id) DO UPDATE SET
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
            params![user_id, device_id, payload, now_secs()],
        )?;
        Ok(())
    }

    pub fn billing_evidence(&self, user_id: Option<&str>) -> Result<Vec<BillingEvidenceRow>> {
        let sql = "SELECT e.user_id, u.name, e.device_id, d.name, e.payload, e.updated_at \
                   FROM billing_evidence e \
                   JOIN users u ON u.id = e.user_id \
                   JOIN devices d ON d.id = e.device_id";
        let mut statement = self.connection.prepare(sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(BillingEvidenceRow {
                user_id: row.get(0)?,
                user_name: row.get(1)?,
                device_id: row.get(2)?,
                device_name: row.get(3)?,
                payload: row.get(4)?,
                updated_at: row.get(5)?,
            })
        };
        let rows = match user_id {
            Some(user_id) => statement.query_map([user_id], map)?,
            None => statement.query_map([], map)?,
        };
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_user(&self, name: &str, role: &str) -> Result<User> {
        let name = name.trim();
        let user = User {
            id: format!("usr-{}", uuid::Uuid::now_v7()),
            name: name.to_string(),
            role: role.to_string(),
        };
        self.connection.execute(
            "INSERT INTO users (id, name, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![user.id, user.name, user.role, now_secs()],
        )?;
        Ok(user)
    }

    pub fn find_user(&self, id: &str) -> Result<Option<User>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, role FROM users WHERE id = ?1",
                [id],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn find_user_by_name(&self, name: &str) -> Result<Option<User>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, role FROM users WHERE name = ?1",
                [name],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, role FROM users ORDER BY name")?;
        let rows = statement.query_map([], |row| {
            Ok(User {
                id: row.get(0)?,
                name: row.get(1)?,
                role: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        let path = path.trim();
        let path = if path == "/" {
            "/".to_string()
        } else {
            path.trim_end_matches('/').to_string()
        };
        let project = Project {
            id: format!("prj-{}", uuid::Uuid::now_v7()),
            name: name.trim().to_string(),
            path,
        };
        self.connection.execute(
            "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![project.id, project.name, project.path, now_secs()],
        )?;
        self.catalog_changed();
        Ok(project)
    }

    pub fn find_project_by_name(&self, name: &str) -> Result<Option<Project>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, path FROM projects WHERE name = ?1",
                [name],
                |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        path: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, path FROM projects ORDER BY name")?;
        let rows = statement.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn project_settings(&self, project_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT payload FROM project_settings WHERE project_id = ?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn save_project_settings(&self, project_id: &str, payload: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO project_settings (project_id, payload, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![project_id, payload, now_secs()],
        )?;
        Ok(())
    }

    pub fn project_for_user(&self, project_id: &str, user_id: &str) -> Result<Option<Project>> {
        Ok(self
            .connection
            .query_row(
                r#"
                SELECT p.id, p.name, p.path
                FROM projects p
                JOIN project_grants g ON g.project_id = p.id
                WHERE p.id = ?1 AND g.user_id = ?2 AND g.active=1
                "#,
                params![project_id, user_id],
                |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        path: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn projects_for_user(&self, user_id: &str) -> Result<Vec<Project>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT p.id, p.name, p.path
            FROM projects p
            JOIN project_grants g ON g.project_id = p.id
            WHERE g.user_id = ?1 AND g.active=1
            ORDER BY p.name
            "#,
        )?;
        let rows = statement.query_map([user_id], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_pairing_code(&self, user_id: &str, ttl_secs: i64) -> Result<(String, i64)> {
        let code = auth::random_pairing_code();
        let hash = auth::hash_token(&code);
        let expires_at = now_secs() + ttl_secs.clamp(60, 900);
        self.connection.execute(
            "INSERT INTO pairing_codes (id, user_id, code_hash, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                format!("pair-{}", uuid::Uuid::now_v7()),
                user_id,
                hash,
                expires_at
            ],
        )?;
        Ok((code, expires_at))
    }

    pub fn consume_pairing_code(&self, code: &str) -> Result<Option<User>> {
        let hash = auth::hash_token(code.trim());
        let row: Option<(String, String, i64, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT id, user_id, expires_at, used_at FROM pairing_codes WHERE code_hash = ?1",
                [&hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((id, user_id, expires_at, used_at)) = row else {
            return Ok(None);
        };
        if used_at.is_some() || expires_at <= now_secs() {
            return Ok(None);
        }
        let changed = self.connection.execute(
            "UPDATE pairing_codes SET used_at = ?1 WHERE id = ?2 AND used_at IS NULL AND expires_at > ?1",
            params![now_secs(), id],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        self.find_user(&user_id)
    }

    /// Create a device and return its one-time plaintext token.
    pub fn create_device(&self, user_id: &str, name: &str) -> Result<(Device, String)> {
        let token = auth::random_token();
        let device = Device {
            id: format!("dev-{}", uuid::Uuid::now_v7()),
            user_id: user_id.to_string(),
            name: name.trim().to_string(),
            created_at: now_secs(),
            last_seen_at: Some(now_secs()),
            revoked_at: None,
        };
        self.connection.execute(
            r#"
            INSERT INTO devices (id, user_id, name, token_hash, created_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                device.id,
                device.user_id,
                device.name,
                auth::hash_token(&token),
                device.created_at,
                device.last_seen_at
            ],
        )?;
        Ok((device, token))
    }

    pub fn authenticate(&self, token: &str) -> Result<Option<(User, Device)>> {
        let hash = auth::hash_token(token);
        let row = self
            .connection
            .query_row(
                r#"
                SELECT d.id, d.user_id, d.name, d.created_at, d.last_seen_at, d.revoked_at,
                       u.name, u.role
                FROM devices d
                JOIN users u ON u.id = d.user_id
                WHERE d.token_hash = ?1
                "#,
                [&hash],
                |row| {
                    let device = Device {
                        id: row.get(0)?,
                        user_id: row.get(1)?,
                        name: row.get(2)?,
                        created_at: row.get(3)?,
                        last_seen_at: row.get(4)?,
                        revoked_at: row.get(5)?,
                    };
                    let user = User {
                        id: device.user_id.clone(),
                        name: row.get(6)?,
                        role: row.get(7)?,
                    };
                    Ok((user, device))
                },
            )
            .optional()?;
        let Some((user, device)) = row else {
            return Ok(None);
        };
        if device.revoked_at.is_some() {
            return Ok(None);
        }
        self.connection.execute(
            "UPDATE devices SET last_seen_at = ?1 WHERE id = ?2",
            params![now_secs(), device.id],
        )?;
        Ok(Some((user, device)))
    }

    pub fn list_devices(&self, user_id: &str) -> Result<Vec<Device>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT id, user_id, name, created_at, last_seen_at, revoked_at
            FROM devices WHERE user_id = ?1 ORDER BY created_at DESC
            "#,
        )?;
        let rows = statement.query_map([user_id], |row| {
            Ok(Device {
                id: row.get(0)?,
                user_id: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get(3)?,
                last_seen_at: row.get(4)?,
                revoked_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Read current authority without extending last-seen time. Presence must
    /// stop exposing revoked devices, deleted users and stale role information.
    pub fn active_device_user(&self, device_id: &str) -> Result<Option<User>> {
        Ok(self
            .connection
            .query_row(
                "SELECT u.id, u.name, u.role FROM devices d
                 JOIN users u ON u.id = d.user_id
                 WHERE d.id = ?1 AND d.revoked_at IS NULL",
                [device_id],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn revoke_device(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE devices SET revoked_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        self.catalog_changed();
        Ok(())
    }

    pub fn create_session(
        &self,
        id: &str,
        user_id: &str,
        project_id: &str,
        title: &str,
        device_id: &str,
    ) -> Result<()> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(());
        }
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )?;
        if count >= MAX_SESSIONS_PER_USER {
            return Err(StoreError::Limit(
                "Session index limit reached; delete old conversations before creating more".into(),
            ));
        }
        self.connection.execute(
            r#"
            INSERT OR IGNORE INTO sessions
                (id, user_id, project_id, title, created_at, updated_at, last_device_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)
            "#,
            params![id, user_id, project_id, title, now_secs(), device_id],
        )?;
        self.catalog_changed();
        Ok(())
    }

    pub fn session_owner(&self, id: &str) -> Result<Option<SessionOwner>> {
        Ok(self
            .connection
            .query_row(
                "SELECT user_id, project_id, title FROM sessions WHERE id = ?1",
                [id],
                |row| {
                    Ok(SessionOwner {
                        user_id: row.get(0)?,
                        project_id: row.get(1)?,
                        title: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_sessions(&self, user_id: &str, project_id: &str) -> Result<Vec<SessionRow>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT id, title, updated_at, last_device_id
            FROM sessions
            WHERE user_id = ?1 AND project_id = ?2
            ORDER BY updated_at DESC
            "#,
        )?;
        let rows = statement.query_map(params![user_id, project_id], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: row.get(2)?,
                last_device_id: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        self.catalog_changed();
        Ok(())
    }

    pub fn touch_session(&self, id: &str, device_id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET updated_at = ?1, last_device_id = ?2 WHERE id = ?3",
            params![now_secs(), device_id, id],
        )?;
        self.catalog_changed();
        Ok(())
    }

    pub fn rename_session_if_new(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }
        let title: String = title.chars().take(80).collect();
        self.connection.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2 AND (title = 'New session' OR title = '')",
            params![title, id],
        )?;
        self.catalog_changed();
        Ok(())
    }

    /// Legacy clients may update only legacy snapshots. They cannot roll back a
    /// versioned snapshot after a newer client has adopted optimistic concurrency.
    pub fn save_transcript(&self, session_id: &str, payload: &str) -> Result<bool> {
        let changed = self.connection.execute(
            r#"
            INSERT INTO transcripts (session_id, payload, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(session_id) DO UPDATE SET
                payload = excluded.payload,
                updated_at = excluded.updated_at
            WHERE transcripts.revision = 0
            "#,
            params![session_id, payload, now_secs()],
        )?;
        Ok(changed == 1)
    }

    pub fn transcript_revision(&self, session_id: &str) -> Result<i64> {
        Ok(self
            .connection
            .query_row(
                "SELECT revision FROM transcripts WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn save_transcript_if_revision(
        &self,
        session_id: &str,
        payload: &str,
        expected: i64,
    ) -> Result<bool> {
        if expected < 0 || expected == i64::MAX {
            return Err(StoreError::Limit("Invalid transcript revision".into()));
        }
        let changed = self.connection.execute(
            "INSERT INTO transcripts (session_id, payload, updated_at, revision)
             SELECT ?1, ?2, ?3, ?4 + 1
             WHERE ?4 = 0 OR EXISTS (SELECT 1 FROM transcripts WHERE session_id = ?1 AND revision = ?4)
             ON CONFLICT(session_id) DO UPDATE SET payload = excluded.payload,
                 updated_at = excluded.updated_at, revision = excluded.revision
             WHERE transcripts.revision = ?4",
            params![session_id, payload, now_secs(), expected],
        )?;
        Ok(changed == 1)
    }

    pub fn load_transcript(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT payload FROM transcripts WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn insert_artifact(&self, row: &ArtifactRow) -> Result<()> {
        self.connection.execute(
            "INSERT INTO relay_artifacts
             (id, project_id, user_id, kind, title, mime, bytes, sha256, blob_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.id,
                row.project_id,
                row.user_id,
                row.kind,
                row.title,
                row.mime,
                row.bytes,
                row.sha256,
                row.blob_path,
                row.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn artifact(&self, id: &str) -> Result<Option<ArtifactRow>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, project_id, user_id, kind, title, mime, bytes, sha256, blob_path, created_at
                 FROM relay_artifacts WHERE id = ?1",
                [id],
                |row| {
                    Ok(ArtifactRow {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        user_id: row.get(2)?,
                        kind: row.get(3)?,
                        title: row.get(4)?,
                        mime: row.get(5)?,
                        bytes: row.get(6)?,
                        sha256: row.get(7)?,
                        blob_path: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn artifacts_for_project(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<Vec<ArtifactRow>> {
        let mut statement = self.connection.prepare(
        "SELECT id, project_id, user_id, kind, title, mime, bytes, sha256, blob_path, created_at
             FROM relay_artifacts WHERE user_id = ?1 AND project_id = ?2
             ORDER BY created_at DESC, id",
    )?;
        let rows = statement.query_map(params![user_id, project_id], |row| {
            Ok(ArtifactRow {
                id: row.get(0)?,
                project_id: row.get(1)?,
                user_id: row.get(2)?,
                kind: row.get(3)?,
                title: row.get(4)?,
                mime: row.get(5)?,
                bytes: row.get(6)?,
                sha256: row.get(7)?,
                blob_path: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_artifact_share(
        &self,
        artifact_id: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO relay_artifact_shares (token_hash, artifact_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, artifact_id, now_secs(), expires_at],
        )?;
        Ok(())
    }

    pub fn artifact_share_by_token(&self, token_hash: &str) -> Result<Option<ArtifactShareRow>> {
        Ok(self
            .connection
            .query_row(
                "SELECT artifact_id, expires_at, revoked_at
                 FROM relay_artifact_shares WHERE token_hash = ?1",
                [token_hash],
                |row| {
                    Ok(ArtifactShareRow {
                        artifact_id: row.get(0)?,
                        expires_at: row.get(1)?,
                        revoked_at: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn revoke_artifact_shares(&self, artifact_id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE relay_artifact_shares SET revoked_at = ?2
             WHERE artifact_id = ?1 AND revoked_at IS NULL",
            params![artifact_id, now_secs()],
        )?;
        Ok(())
    }

    pub fn delete_artifact(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM relay_artifacts WHERE id = ?1", [id])?;
        Ok(())
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::Store;

    #[test]
    fn pairing_is_limited_to_fifteen_minutes_and_expiry_is_server_enforced() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("admin", "admin").unwrap();
        let before = super::now_secs();
        let (code, expires) = store.create_pairing_code(&user.id, 86400).unwrap();
        assert!((before + 900..=super::now_secs() + 900).contains(&expires));
        store
            .connection
            .execute(
                "UPDATE pairing_codes SET expires_at = ?1",
                [super::now_secs()],
            )
            .unwrap();
        assert!(store.consume_pairing_code(&code).unwrap().is_none());
    }

    #[test]
    fn pairs_a_device_and_authenticates_it() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("marek", "admin").unwrap();
        let (code, _) = store.create_pairing_code(&user.id, 600).unwrap();
        let paired = store.consume_pairing_code(&code).unwrap().unwrap();
        assert_eq!(paired.id, user.id);
        assert!(store.consume_pairing_code(&code).unwrap().is_none());

        let (device, token) = store.create_device(&user.id, "MacBook").unwrap();
        let (auth_user, auth_device) = store.authenticate(&token).unwrap().unwrap();
        assert_eq!(auth_user.id, user.id);
        assert_eq!(auth_device.id, device.id);
        assert!(store.authenticate("wrong").unwrap().is_none());

        store.revoke_device(&device.id).unwrap();
        assert!(store.authenticate(&token).unwrap().is_none());
    }

    #[test]
    fn grants_projects_and_indexes_sessions() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("marek", "member").unwrap();
        let other = store.create_user("anna", "member").unwrap();
        let project = store.create_project("dunlin", "/tmp/dunlin").unwrap();
        store.grant(&project.id, &user.id).unwrap();

        assert!(
            store
                .project_for_user(&project.id, &user.id)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .project_for_user(&project.id, &other.id)
                .unwrap()
                .is_none()
        );

        let (device, _) = store.create_device(&user.id, "Phone").unwrap();
        store
            .create_session("ses-1", &user.id, &project.id, "New session", &device.id)
            .unwrap();
        store
            .rename_session_if_new("ses-1", "Fix the relay")
            .unwrap();
        let sessions = store.list_sessions(&user.id, &project.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Fix the relay");

        store.delete_session("ses-1").unwrap();
        assert!(store.session_owner("ses-1").unwrap().is_none());
    }

    #[test]
    fn artifacts_and_shares_round_trip_and_revoke() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("marek", "member").unwrap();
        let project = store.create_project("dunlin", "/tmp/dunlin").unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let row = super::ArtifactRow {
            id: "art-1".into(),
            project_id: project.id.clone(),
            user_id: user.id.clone(),
            kind: "diff".into(),
            title: "Diff".into(),
            mime: "text/plain".into(),
            bytes: 3,
            sha256: "hash".into(),
            blob_path: "/tmp/art-1".into(),
            created_at: super::now_secs(),
        };
        store.insert_artifact(&row).unwrap();
        assert_eq!(store.artifact("art-1").unwrap().unwrap().title, "Diff");
        assert_eq!(
            store
                .artifacts_for_project(&user.id, &project.id)
                .unwrap()
                .len(),
            1
        );
        store
            .create_artifact_share("art-1", "token-hash", super::now_secs() + 60)
            .unwrap();
        assert_eq!(
            store
                .artifact_share_by_token("token-hash")
                .unwrap()
                .unwrap()
                .artifact_id,
            "art-1"
        );
        store.revoke_artifact_shares("art-1").unwrap();
        assert!(
            store
                .artifact_share_by_token("token-hash")
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        store.delete_artifact("art-1").unwrap();
        assert!(store.artifact("art-1").unwrap().is_none());
    }

    #[test]
    fn stores_and_cascades_transcripts() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("marek", "member").unwrap();
        let project = store.create_project("dunlin", "/tmp/dunlin").unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let (device, _) = store.create_device(&user.id, "Mac").unwrap();
        store
            .create_session("ses-1", &user.id, &project.id, "Test", &device.id)
            .unwrap();
        store
            .save_transcript("ses-1", r#"[{"kind":"smoke"}]"#)
            .unwrap();
        assert_eq!(
            store.load_transcript("ses-1").unwrap().as_deref(),
            Some(r#"[{"kind":"smoke"}]"#)
        );
        store.delete_session("ses-1").unwrap();
        assert!(store.load_transcript("ses-1").unwrap().is_none());
    }

    #[test]
    fn v17_journals_migrate_to_stream_baseline_without_losing_prompts_or_reusing_order() {
        use dunlin_session_runtime::{StreamReplay, Subscription};
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("relay.db");
        let store = Store::open(&database).unwrap();
        let owner = store.create_user("stream-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "stream-team").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "project")
            .unwrap();
        let authority = store
            .team_project_context(&workspace.id, &project.id, &owner.id)
            .unwrap();
        let session = store
            .interactive_journal()
            .create(&authority, &workspace.id, &project.id)
            .unwrap();
        store
            .interactive_journal()
            .enqueue(&authority, &session.id, "one", "v17 retained prompt")
            .unwrap();
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; DROP TABLE interactive_session_share_audit; DROP TABLE interactive_session_readers; DROP TABLE interactive_session_sharing; DROP TABLE interactive_permission_challenges; DROP TABLE interactive_cancellation_intents; DROP TABLE interactive_client_dispatch; DROP TABLE interactive_message_ids; DROP TABLE interactive_ahp_clients; DROP TABLE interactive_ahp_sessions; DROP TABLE interactive_ahp_actions; DROP TABLE interactive_ahp_chats; DROP TABLE interactive_ahp_clock; DROP TRIGGER interactive_event_stream; DROP TABLE interactive_stream_events; DROP TABLE interactive_stream_clock; ALTER TABLE relay_jobs DROP COLUMN checkout_grant; ALTER TABLE project_grants DROP COLUMN active; ALTER TABLE project_grants DROP COLUMN revision; PRAGMA user_version=17;").unwrap();
        drop(store);
        let store = Store::open(&database).unwrap();
        let authority = store
            .team_project_context(&workspace.id, &project.id, &owner.id)
            .unwrap();
        let journal = store.interactive_journal();
        let subscriptions = [Subscription {
            context: &authority,
            session: &session.id,
        }];
        let baseline = journal.snapshot_stream(&subscriptions).unwrap();
        assert_eq!(baseline.cursor.sequence, 2);
        assert_eq!(baseline.sessions[0].1[0].prompt, "v17 retained prompt");
        journal
            .enqueue(&authority, &session.id, "two", "new prompt")
            .unwrap();
        match journal
            .replay_stream(&subscriptions, &baseline.cursor)
            .unwrap()
        {
            StreamReplay::Events { cursor, events } => {
                assert_eq!(cursor.sequence, 3);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event.kind, "turn_queued");
            }
            _ => panic!("Expected migration to preserve the stream baseline"),
        }
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, crate::store::SCHEMA_VERSION);
    }

    #[test]
    fn transcript_revision_migration_preserves_legacy_history() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("relay.db");
        let store = Store::open(&database).unwrap();
        let user = store.create_user("migration", "member").unwrap();
        let project = store
            .create_project("project", directory.path().to_str().unwrap())
            .unwrap();
        let (device, _) = store.create_device(&user.id, "test").unwrap();
        store
            .create_session("session", &user.id, &project.id, "History", &device.id)
            .unwrap();
        assert!(store.save_transcript("session", "legacy history").unwrap());
        store
            .connection
            .execute_batch(
                "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE transcripts DROP COLUMN revision;
                ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; DROP TABLE interactive_session_share_audit; DROP TABLE interactive_session_readers; DROP TABLE interactive_session_sharing; DROP TABLE interactive_permission_challenges; DROP TABLE interactive_cancellation_intents; DROP TABLE interactive_client_dispatch; DROP TABLE interactive_message_ids; DROP TABLE interactive_ahp_clients; DROP TABLE interactive_ahp_sessions; DROP TABLE interactive_ahp_actions; DROP TABLE interactive_ahp_chats; DROP TABLE interactive_ahp_clock; DROP TRIGGER interactive_event_stream; DROP TABLE interactive_stream_events; DROP TABLE interactive_stream_clock; DROP TABLE team_local_bindings; DROP TABLE interactive_events; DROP TABLE interactive_turns; DROP TABLE interactive_sessions; DROP TABLE team_service_principals; DROP TABLE web_oidc_sessions; DROP TABLE team_invitations; DROP TABLE web_browser_sessions;
                DROP TABLE team_workflow_versions; DROP TABLE team_workflow_drafts;
                DROP TABLE team_audit; DROP TABLE team_project_grants; DROP TABLE team_projects;
                DROP TABLE team_memberships; DROP TABLE team_workspaces;
                ALTER TABLE relay_jobs DROP COLUMN checkout_grant; ALTER TABLE project_grants DROP COLUMN active; ALTER TABLE project_grants DROP COLUMN revision; PRAGMA user_version = 5;",
            )
            .unwrap();
        drop(store);
        let store = Store::open(&database).unwrap();
        assert_eq!(store.transcript_revision("session").unwrap(), 0);
        assert_eq!(
            store.load_transcript("session").unwrap().as_deref(),
            Some("legacy history")
        );
        assert!(
            store
                .save_transcript_if_revision("session", "versioned history", 0)
                .unwrap()
        );
        assert!(
            !store
                .save_transcript("session", "late legacy history")
                .unwrap()
        );
        assert!(
            !store
                .save_transcript_if_revision("session", "stale history", 0)
                .unwrap()
        );
        assert_eq!(
            store.load_transcript("session").unwrap().as_deref(),
            Some("versioned history")
        );
    }
}
