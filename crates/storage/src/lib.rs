//! Local persistence for Dunlin.
//!
//! SQLite in WAL mode keeps writes cheap and crash-safe. Projects, sessions and
//! the rendered transcript are persisted; graph and trace projections will be
//! derived from the same session log in a later migration.

use std::path::Path;

use dunlin_domain::{Project, ProjectGroup, SessionSummary};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    InvalidGroup(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A deliberate human decision bound to the exact bytes of one artifact.
/// Files remain canonical; this row is local evidence, not a replacement.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArtifactApproval {
    pub project_id: String,
    pub path: String,
    pub revision: String,
    pub status: String,
    pub actor: String,
    pub note: String,
    /// HEAD commit observed at decision time; empty when Git was unavailable.
    #[serde(default)]
    pub commit: String,
    pub approved_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskEvidence {
    pub project_id: String,
    pub task_id: String,
    pub kind: String,
    pub reference: String,
    pub summary: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub intent_id: Option<String>,
    pub task_id: Option<String>,
    pub handoff_id: Option<String>,
    pub harness: String,
    pub model: String,
    pub permission_mode: String,
    pub worktree: Option<String>,
    pub budget_id: Option<String>,
    pub envelope_hash: Option<String>,
    pub status: String,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub result: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtifactRecord {
    pub id: String,
    pub project_id: String,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub intent_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub mime: String,
    pub bytes: i64,
    pub sha256: String,
    pub blob_path: String,
    pub created_ms: i64,
    pub visibility: String,
    pub expires_ms: Option<i64>,
    pub revoked_ms: Option<i64>,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClaimRecord {
    pub id: String,
    pub project_id: String,
    pub intent_id: Option<String>,
    pub task_id: Option<String>,
    pub statement: String,
    pub status: String,
    pub confidence: String,
    pub scope: String,
    pub last_verified_ms: Option<i64>,
    pub stale_reason: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub artifact_id: Option<String>,
    pub run_id: Option<String>,
    pub source: String,
    pub revision: String,
    pub hash: String,
    pub observed_ms: i64,
    pub freshness: String,
    pub summary: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PluginRecord {
    pub id: String,
    pub version: String,
    pub manifest_hash: String,
    pub wasm_hash: Option<String>,
    pub directory: String,
    pub enabled: bool,
    pub installed_ms: i64,
    pub payload: serde_json::Value,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    /// Separate read-only handle for background statistics; never migrate or
    /// create a database during a read.
    pub fn read_only(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            connection: Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?,
        })
    }

    pub fn database_path(&self) -> Option<std::path::PathBuf> {
        self.connection
            .path()
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn quick_check(&self) -> Result<String> {
        Ok(self
            .connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?)
    }

    /// Clear local application records atomically, without touching project files.
    pub fn factory_reset(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute_batch(
            "DELETE FROM evidence_claims;
             DELETE FROM evidence;
             DELETE FROM claims;
             DELETE FROM artifacts;
             DELETE FROM runs;
             DELETE FROM knowledge_edges;
             DELETE FROM knowledge_objects;
             DELETE FROM plugin_registry;
             DELETE FROM ssh_hosts;
             DELETE FROM task_evidence;
             DELETE FROM artifact_index;
             DELETE FROM artifact_approvals;
             DELETE FROM secret_audit;
             DELETE FROM secret_grants;
             DELETE FROM secret_recipes;
             DELETE FROM secrets;
             DELETE FROM transcripts;
             DELETE FROM sessions;
             DELETE FROM projects;
             DELETE FROM project_groups;
             DELETE FROM settings;",
        )?;
        transaction.commit()?;
        Ok(())
    }
    /// Open (or create) a database, enabling WAL and foreign keys.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 1 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS projects (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    path        TEXT NOT NULL UNIQUE,
                    branch      TEXT,
                    tone        TEXT NOT NULL,
                    pinned      INTEGER NOT NULL DEFAULT 0,
                    trusted     INTEGER NOT NULL DEFAULT 1,
                    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );

                CREATE TABLE IF NOT EXISTS sessions (
                    id            TEXT PRIMARY KEY,
                    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    title         TEXT NOT NULL,
                    status        TEXT NOT NULL,
                    harness       TEXT NOT NULL,
                    model         TEXT NOT NULL,
                    branch        TEXT,
                    updated_at    TEXT NOT NULL,
                    cost_usd      REAL NOT NULL DEFAULT 0,
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    tokens        INTEGER NOT NULL DEFAULT 0,
                    summary       TEXT NOT NULL DEFAULT '',
                    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
                );
                CREATE INDEX IF NOT EXISTS sessions_project_idx
                    ON sessions(project_id, created_at DESC);

                CREATE TABLE IF NOT EXISTS transcripts (
                    session_id  TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                    payload     TEXT NOT NULL,
                    updated_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );

                PRAGMA user_version = 1;
                "#,
            )?;
        }
        if version < 2 {
            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS settings (
                    key         TEXT PRIMARY KEY,
                    value       TEXT NOT NULL,
                    updated_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );

                PRAGMA user_version = 2;
                "#,
            )?;
        }
        if version < 3 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                ALTER TABLE projects ADD COLUMN remote TEXT;
                ALTER TABLE sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'local';
                ALTER TABLE sessions ADD COLUMN acp_session_id TEXT;

                PRAGMA user_version = 3;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 4 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS project_groups (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    tone        TEXT NOT NULL,
                    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );

                ALTER TABLE projects ADD COLUMN group_id TEXT
                    REFERENCES project_groups(id) ON DELETE SET NULL;

                PRAGMA user_version = 4;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 5 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                ALTER TABLE projects ADD COLUMN settings TEXT;

                PRAGMA user_version = 5;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 6 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch("ALTER TABLE project_groups ADD COLUMN parent_id TEXT REFERENCES project_groups(id) ON DELETE SET NULL; PRAGMA user_version = 6;")?;
            transaction.commit()?;
        }
        if version < 7 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch("ALTER TABLE project_groups ADD COLUMN settings TEXT NOT NULL DEFAULT '{}'; PRAGMA user_version = 7;")?;
            transaction.commit()?;
        }
        if version < 8 {
            let transaction = self.connection.unchecked_transaction()?;
            let has_kind: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'kind'",
                [],
                |row| row.get(0),
            )?;
            if has_kind == 0 {
                transaction.execute_batch(
                    "ALTER TABLE projects ADD COLUMN kind TEXT NOT NULL DEFAULT 'repo';",
                )?;
            }
            transaction.execute_batch("PRAGMA user_version = 8;")?;
            transaction.commit()?;
        }
        if version < 9 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS secrets (
                    id              TEXT PRIMARY KEY,
                    namespace       TEXT NOT NULL,
                    alias           TEXT NOT NULL UNIQUE,
                    label           TEXT NOT NULL,
                    description     TEXT NOT NULL DEFAULT '',
                    injection       TEXT NOT NULL,
                    allowed_hosts   TEXT NOT NULL DEFAULT '[]',
                    path_prefix     TEXT NOT NULL DEFAULT '/',
                    allowed_methods TEXT NOT NULL DEFAULT '["GET","HEAD"]',
                    allowed_headers TEXT NOT NULL DEFAULT '[]',
                    auto_approve    INTEGER NOT NULL DEFAULT 1,
                    expires_at      INTEGER,
                    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
                    updated_at      INTEGER NOT NULL DEFAULT (unixepoch()),
                    last_used_at    INTEGER
                );

                CREATE TABLE IF NOT EXISTS secret_grants (
                    secret_id   TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
                    project_id  TEXT NOT NULL,
                    policy      TEXT NOT NULL DEFAULT 'auto',
                    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                    PRIMARY KEY (secret_id, project_id)
                );

                CREATE TABLE IF NOT EXISTS secret_recipes (
                    id           TEXT PRIMARY KEY,
                    secret_id    TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
                    name         TEXT NOT NULL,
                    description  TEXT NOT NULL DEFAULT '',
                    program      TEXT NOT NULL,
                    args         TEXT NOT NULL DEFAULT '[]',
                    env_var      TEXT NOT NULL DEFAULT 'DUNLIN_SECRET',
                    parameters   TEXT NOT NULL DEFAULT '[]',
                    timeout_secs INTEGER NOT NULL DEFAULT 30,
                    program_hash TEXT,
                    created_at   INTEGER NOT NULL DEFAULT (unixepoch()),
                    UNIQUE (secret_id, name)
                );

                CREATE TABLE IF NOT EXISTS secret_audit (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    secret_id   TEXT,
                    alias       TEXT NOT NULL,
                    project_id  TEXT,
                    session_id  TEXT,
                    operation   TEXT NOT NULL,
                    detail      TEXT NOT NULL,
                    decision    TEXT NOT NULL,
                    status      INTEGER,
                    error       TEXT,
                    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );
                CREATE INDEX IF NOT EXISTS secret_audit_secret_idx
                    ON secret_audit(secret_id, created_at DESC);
                CREATE INDEX IF NOT EXISTS secret_audit_created_idx
                    ON secret_audit(created_at DESC);

                PRAGMA user_version = 9;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 10 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS artifact_approvals (
                    project_id  TEXT NOT NULL,
                    path        TEXT NOT NULL,
                    revision    TEXT NOT NULL,
                    status      TEXT NOT NULL,
                    actor       TEXT NOT NULL,
                    note        TEXT NOT NULL DEFAULT '',
                    approved_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    PRIMARY KEY (project_id, path)
                );
                CREATE INDEX IF NOT EXISTS artifact_approvals_project_idx
                    ON artifact_approvals(project_id, approved_at DESC);

                PRAGMA user_version = 10;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 11 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS artifact_index (
                    project_id TEXT NOT NULL,
                    path       TEXT NOT NULL,
                    kind       TEXT NOT NULL,
                    status     TEXT NOT NULL,
                    title      TEXT NOT NULL,
                    revision   TEXT NOT NULL,
                    modified_ms INTEGER NOT NULL,
                    payload    TEXT NOT NULL,
                    indexed_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    PRIMARY KEY (project_id, path)
                );
                CREATE INDEX IF NOT EXISTS artifact_index_project_idx
                    ON artifact_index(project_id, kind);

                PRAGMA user_version = 11;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 12 {
            let transaction = self.connection.unchecked_transaction()?;
            let has_commit: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('artifact_approvals') WHERE name = 'head_commit'",
                [],
                |row| row.get(0),
            )?;
            if has_commit == 0 {
                transaction.execute_batch(
                    "ALTER TABLE artifact_approvals ADD COLUMN head_commit TEXT NOT NULL DEFAULT '';",
                )?;
            }
            transaction.execute_batch("PRAGMA user_version = 12;")?;
            transaction.commit()?;
        }
        if version < 13 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS task_evidence (
                    project_id TEXT NOT NULL,
                    task_id    TEXT NOT NULL,
                    kind       TEXT NOT NULL,
                    reference  TEXT NOT NULL,
                    summary    TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    PRIMARY KEY (project_id, task_id, kind, reference)
                );
                CREATE INDEX IF NOT EXISTS task_evidence_project_idx
                    ON task_evidence(project_id, task_id, created_at DESC);

                PRAGMA user_version = 13;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 14 {
            let transaction = self.connection.unchecked_transaction()?;
            let exists: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'workspace'",
                [],
                |row| row.get(0),
            )?;
            if exists == 0 {
                transaction.execute_batch("ALTER TABLE sessions ADD COLUMN workspace TEXT;")?;
            }
            // Legacy display labels were not timestamps; use the last persisted transcript instead.
            transaction.execute_batch("UPDATE sessions SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', COALESCE((SELECT updated_at FROM transcripts WHERE session_id = sessions.id), created_at), 'unixepoch') WHERE updated_at = 'now' OR updated_at GLOB '[0-9]*m ago' OR updated_at GLOB '[0-9]*h ago' OR updated_at GLOB '[0-9]*d ago'; PRAGMA user_version = 14;")?;
            transaction.commit()?;
        }
        if version < 15 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS knowledge_objects (
                    project_id TEXT NOT NULL,
                    id         TEXT NOT NULL,
                    kind       TEXT NOT NULL,
                    title      TEXT NOT NULL,
                    status     TEXT NOT NULL,
                    revision   TEXT NOT NULL,
                    path       TEXT,
                    payload    TEXT NOT NULL,
                    updated_ms INTEGER NOT NULL,
                    PRIMARY KEY (project_id, id)
                );
                CREATE INDEX IF NOT EXISTS knowledge_objects_project_kind_idx
                    ON knowledge_objects(project_id, kind, status);

                CREATE TABLE IF NOT EXISTS knowledge_edges (
                    project_id TEXT NOT NULL,
                    from_id    TEXT NOT NULL,
                    to_id      TEXT NOT NULL,
                    relation   TEXT NOT NULL,
                    metadata   TEXT NOT NULL DEFAULT '{}',
                    PRIMARY KEY (project_id, from_id, to_id, relation)
                );
                CREATE INDEX IF NOT EXISTS knowledge_edges_from_idx
                    ON knowledge_edges(project_id, from_id, relation);
                CREATE INDEX IF NOT EXISTS knowledge_edges_to_idx
                    ON knowledge_edges(project_id, to_id, relation);

                PRAGMA user_version = 15;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 16 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS runs (
                    id              TEXT PRIMARY KEY,
                    project_id      TEXT NOT NULL,
                    session_id      TEXT,
                    intent_id       TEXT,
                    task_id         TEXT,
                    handoff_id      TEXT,
                    harness         TEXT NOT NULL,
                    model           TEXT NOT NULL,
                    permission_mode TEXT NOT NULL,
                    worktree        TEXT,
                    budget_id       TEXT,
                    envelope_hash   TEXT,
                    status          TEXT NOT NULL,
                    started_ms      INTEGER,
                    ended_ms        INTEGER,
                    result          TEXT NOT NULL DEFAULT '',
                    payload         TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS runs_project_idx ON runs(project_id, started_ms DESC);

                CREATE TABLE IF NOT EXISTS artifacts (
                    id          TEXT PRIMARY KEY,
                    project_id  TEXT NOT NULL,
                    run_id      TEXT,
                    session_id  TEXT,
                    intent_id   TEXT,
                    task_id     TEXT,
                    kind        TEXT NOT NULL,
                    title       TEXT NOT NULL,
                    mime        TEXT NOT NULL,
                    bytes       INTEGER NOT NULL,
                    sha256      TEXT NOT NULL,
                    blob_path   TEXT NOT NULL,
                    created_ms  INTEGER NOT NULL,
                    visibility  TEXT NOT NULL DEFAULT 'local',
                    expires_ms  INTEGER,
                    revoked_ms  INTEGER,
                    payload     TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS artifacts_project_idx
                    ON artifacts(project_id, created_ms DESC);

                CREATE TABLE IF NOT EXISTS claims (
                    id               TEXT PRIMARY KEY,
                    project_id       TEXT NOT NULL,
                    intent_id        TEXT,
                    task_id          TEXT,
                    statement        TEXT NOT NULL,
                    status           TEXT NOT NULL,
                    confidence       TEXT NOT NULL,
                    scope            TEXT NOT NULL DEFAULT '',
                    last_verified_ms INTEGER,
                    stale_reason     TEXT,
                    payload          TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS claims_project_idx
                    ON claims(project_id, status);

                CREATE TABLE IF NOT EXISTS evidence (
                    id          TEXT PRIMARY KEY,
                    project_id  TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    artifact_id TEXT,
                    run_id      TEXT,
                    source      TEXT NOT NULL,
                    revision    TEXT NOT NULL,
                    hash        TEXT NOT NULL,
                    observed_ms INTEGER NOT NULL,
                    freshness   TEXT NOT NULL,
                    summary     TEXT NOT NULL DEFAULT '',
                    payload     TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS evidence_project_idx
                    ON evidence(project_id, observed_ms DESC);

                CREATE TABLE IF NOT EXISTS evidence_claims (
                    evidence_id TEXT NOT NULL,
                    claim_id    TEXT NOT NULL,
                    relation    TEXT NOT NULL,
                    PRIMARY KEY (evidence_id, claim_id, relation)
                );
                CREATE INDEX IF NOT EXISTS evidence_claims_claim_idx
                    ON evidence_claims(claim_id, relation);

                PRAGMA user_version = 16;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 17 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS plugin_registry (
                    id            TEXT PRIMARY KEY,
                    version       TEXT NOT NULL,
                    manifest_hash TEXT NOT NULL,
                    wasm_hash     TEXT,
                    directory     TEXT NOT NULL,
                    enabled       INTEGER NOT NULL DEFAULT 0,
                    installed_ms  INTEGER NOT NULL DEFAULT (unixepoch()),
                    payload       TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS plugin_registry_enabled_idx
                    ON plugin_registry(enabled, id);

                PRAGMA user_version = 17;
                "#,
            )?;
            transaction.commit()?;
        }
        if version < 18 {
            let transaction = self.connection.unchecked_transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS ssh_hosts (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    target      TEXT NOT NULL,
                    user        TEXT NOT NULL,
                    port        INTEGER NOT NULL,
                    identity    TEXT,
                    relay_port  INTEGER NOT NULL DEFAULT 8787,
                    local_port  INTEGER NOT NULL DEFAULT 0,
                    host_key    TEXT NOT NULL DEFAULT '',
                    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
                );

                PRAGMA user_version = 18;
                "#,
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// Commit an additive catalog import and preferences together, or nothing.
    pub fn import_catalog(
        &self,
        groups: &[ProjectGroup],
        projects: &[Project],
        settings: Option<&serde_json::Value>,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut all = self.project_groups()?;
        let mut ids: std::collections::HashSet<_> =
            all.iter().map(|group| group.id.clone()).collect();
        all.extend_from_slice(groups);
        for group in groups {
            if !ids.insert(group.id.clone()) {
                return Err(StoreError::InvalidGroup(
                    "Imported group already exists".into(),
                ));
            }
            group
                .validate_parent(&all)
                .map_err(StoreError::InvalidGroup)?;
        }
        // Input is parent-first; existing catalog entries are never overwritten.
        for group in groups {
            self.write_project_group(group)?;
        }
        for project in projects {
            let exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
                [&project.id],
                |r| r.get(0),
            )?;
            if exists {
                return Err(StoreError::InvalidGroup(
                    "Imported project already exists".into(),
                ));
            }
            self.upsert_project(project)?;
        }
        if let Some(settings) = settings {
            self.save_setting("app", settings)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_project(&self, project: &Project) -> Result<()> {
        let tone = serde_json::to_string(&project.tone)?;
        let remote = project
            .remote
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let settings = serde_json::to_string(&project.settings)?;
        let kind = serde_json::to_string(&project.kind)?;
        self.connection.execute(
            r#"
            INSERT INTO projects (id, name, path, branch, tone, pinned, trusted, remote, group_id, settings, kind)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                path = excluded.path,
                branch = excluded.branch,
                tone = excluded.tone,
                pinned = excluded.pinned,
                trusted = excluded.trusted,
                remote = excluded.remote,
                group_id = excluded.group_id,
                settings = excluded.settings,
                kind = excluded.kind
            "#,
            params![
                project.id,
                project.name,
                project.path,
                project.branch,
                tone,
                project.pinned as i64,
                project.trusted as i64,
                remote,
                project.group_id,
                settings,
                kind,
            ],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, path, branch, tone, pinned, trusted, remote, group_id, settings, kind FROM projects ORDER BY created_at",
        )?;
        let rows = statement.query_map([], |row| {
            let tone: String = row.get(4)?;
            let remote: Option<String> = row.get(7)?;
            let settings: Option<String> = row.get(9)?;
            let kind: String = row.get(10)?;
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                branch: row.get(3)?,
                tone: serde_json::from_str(&tone).unwrap_or(dunlin_domain::ProjectTone::Blue),
                pinned: row.get::<_, i64>(5)? != 0,
                trusted: row.get::<_, i64>(6)? != 0,
                running_sessions: 0,
                kind: serde_json::from_str(&kind).unwrap_or_default(),
                remote: remote
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                group_id: row.get(8)?,
                settings: settings
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn project_groups(&self) -> Result<Vec<ProjectGroup>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, tone, parent_id, settings FROM project_groups ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([], |row| {
            let tone: String = row.get(2)?;
            Ok(ProjectGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                tone: serde_json::from_str(&tone).unwrap_or(dunlin_domain::ProjectTone::Blue),
                parent_id: row.get(3)?,
                settings: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_project_group(&self, group: &ProjectGroup) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        group
            .validate_parent(&self.project_groups()?)
            .map_err(StoreError::InvalidGroup)?;
        self.write_project_group(group)?;
        transaction.commit()?;
        Ok(())
    }

    fn write_project_group(&self, group: &ProjectGroup) -> Result<()> {
        let tone = serde_json::to_string(&group.tone)?;
        let settings = serde_json::to_string(&group.settings)?;
        self.connection.execute(
            r#"
            INSERT INTO project_groups (id, name, tone, parent_id, settings)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                tone = excluded.tone,
                parent_id = excluded.parent_id,
                settings = excluded.settings
            "#,
            params![group.id, group.name, tone, group.parent_id, settings],
        )?;
        Ok(())
    }

    pub fn delete_project_group(&self, id: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("UPDATE projects SET group_id = (SELECT parent_id FROM project_groups WHERE id = ?1) WHERE group_id = ?1", [id])?;
        transaction.execute("UPDATE project_groups SET parent_id = (SELECT parent_id FROM project_groups WHERE id = ?1) WHERE parent_id = ?1", [id])?;
        transaction.execute("DELETE FROM project_groups WHERE id = ?1", [id])?;
        transaction.commit()?;
        Ok(())
    }

    /// Undo only the original direct memberships, never histories, settings,
    /// files or later user moves. The complete restoration is atomic.
    pub fn restore_project_group(
        &self,
        group: &ProjectGroup,
        promoted_parent: Option<&str>,
        project_ids: &[String],
        child_ids: &[String],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut groups = self.project_groups()?;
        if groups.iter().any(|g| g.id == group.id) {
            return Err(StoreError::InvalidGroup("The group already exists".into()));
        }
        group
            .validate_parent(&groups)
            .map_err(StoreError::InvalidGroup)?;
        self.write_project_group(group)?;
        groups.push(group.clone());
        for id in project_ids {
            transaction.execute(
                "UPDATE projects SET group_id = ?1 WHERE id = ?2 AND group_id IS ?3",
                params![group.id, id, promoted_parent],
            )?;
        }
        for id in child_ids {
            if let Some(child) = groups
                .iter()
                .find(|g| &g.id == id && g.parent_id.as_deref() == promoted_parent)
            {
                let mut child = child.clone();
                child.parent_id = Some(group.id.clone());
                child
                    .validate_parent(&groups)
                    .map_err(StoreError::InvalidGroup)?;
                transaction.execute(
                    "UPDATE project_groups SET parent_id = ?1 WHERE id = ?2 AND parent_id IS ?3",
                    params![group.id, id, promoted_parent],
                )?;
                if let Some(stored) = groups.iter_mut().find(|g| &g.id == id) {
                    *stored = child;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_session(&self, session: &SessionSummary) -> Result<()> {
        let status = serde_json::to_string(&session.status)?;
        let origin = serde_json::to_string(&session.origin)?;
        self.connection.execute(
            r#"
            INSERT INTO sessions (
                id, project_id, title, status, harness, model, branch,
                updated_at, cost_usd, duration_secs, tokens, summary, origin, acp_session_id, workspace
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                status = excluded.status,
                harness = excluded.harness,
                model = excluded.model,
                branch = excluded.branch,
                updated_at = excluded.updated_at,
                cost_usd = excluded.cost_usd,
                duration_secs = excluded.duration_secs,
                tokens = excluded.tokens,
                summary = excluded.summary,
                origin = excluded.origin,
                acp_session_id = excluded.acp_session_id,
                workspace = excluded.workspace
            "#,
            params![
                session.id,
                session.project_id,
                session.title,
                status,
                session.harness,
                session.model,
                session.branch,
                session.updated_at,
                session.cost_usd,
                session.duration_secs as i64,
                session.tokens as i64,
                session.summary,
                origin,
                session.acp_session_id,
                session.workspace,
            ],
        )?;
        Ok(())
    }

    /// Update only the recorded location, preserving conversation metadata and turns.
    pub fn update_session_location(
        &self,
        id: &str,
        project_id: &str,
        workspace: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET project_id = ?2, workspace = COALESCE(?3, workspace) WHERE id = ?1",
            params![id, project_id, workspace],
        )?;
        Ok(())
    }

    /// Insert-only, atomic history import. Existing conversations/transcripts
    /// are never updated, even when a preview is stale or imported twice.
    pub fn import_sessions<T: Serialize>(
        &self,
        rows: &[(SessionSummary, T)],
    ) -> Result<Vec<String>> {
        self.import_session_stream(
            rows.iter()
                .map(|(session, transcript)| Ok((session.clone(), transcript))),
            |_| {},
        )
    }

    /// Stream histories through one transaction, retaining only one transcript at a time.
    pub fn import_session_stream<T: Serialize>(
        &self,
        rows: impl IntoIterator<Item = Result<(SessionSummary, T)>>,
        mut progress: impl FnMut(usize),
    ) -> Result<Vec<String>> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut inserted = Vec::new();
        for (index, row) in rows.into_iter().enumerate() {
            let (session, transcript) = row?;
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
                [&session.id],
                |r| r.get(0),
            )?;
            if !exists {
                self.upsert_session(&session)?;
                self.save_transcript(&session.id, &transcript)?;
                inserted.push(session.id.clone());
            }
            progress(index + 1);
        }
        transaction.commit()?;
        Ok(inserted)
    }

    pub fn sessions(&self, project_id: Option<&str>) -> Result<Vec<SessionSummary>> {
        let sql = match project_id {
            Some(_) => {
                "SELECT id, project_id, title, status, harness, model, branch, updated_at, \
                 cost_usd, duration_secs, tokens, summary, origin, acp_session_id, workspace FROM sessions \
                 WHERE project_id = ?1 ORDER BY created_at DESC"
            }
            None => {
                "SELECT id, project_id, title, status, harness, model, branch, updated_at, \
                 cost_usd, duration_secs, tokens, summary, origin, acp_session_id, workspace FROM sessions \
                 ORDER BY created_at DESC"
            }
        };
        let mut statement = self.connection.prepare(sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            let status: String = row.get(3)?;
            let origin: String = row.get(12)?;
            Ok(SessionSummary {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                status: serde_json::from_str(&status).unwrap_or(dunlin_domain::SessionStatus::Done),
                harness: row.get(4)?,
                model: row.get(5)?,
                branch: row.get(6)?,
                updated_at: row.get(7)?,
                cost_usd: row.get(8)?,
                duration_secs: row.get::<_, i64>(9)? as u64,
                tokens: row.get::<_, i64>(10)? as u64,
                summary: row.get(11)?,
                origin: serde_json::from_str(&origin)
                    .unwrap_or(dunlin_domain::SessionOrigin::Local),
                acp_session_id: row.get(13)?,
                workspace: row.get(14)?,
            })
        };
        let rows = match project_id {
            Some(project_id) => statement.query_map([project_id], map)?,
            None => statement.query_map([], map)?,
        };
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Replace the rendered transcript for a session.
    pub fn save_transcript<T: Serialize>(&self, session_id: &str, transcript: &T) -> Result<()> {
        let payload = serde_json::to_string(transcript)?;
        self.connection.execute(
            r#"
            INSERT INTO transcripts (session_id, payload, updated_at)
            VALUES (?1, ?2, unixepoch())
            ON CONFLICT(session_id) DO UPDATE SET
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
            params![session_id, payload],
        )?;
        Ok(())
    }

    pub fn load_transcript<T: DeserializeOwned>(&self, session_id: &str) -> Result<Option<T>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM transcripts WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        match payload {
            Some(payload) => Ok(Some(serde_json::from_str(&payload)?)),
            None => Ok(None),
        }
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
        Ok(())
    }

    /// Remove every conversation. Projects and settings stay.
    pub fn clear_sessions(&self) -> Result<()> {
        self.connection.execute("DELETE FROM sessions", [])?;
        Ok(())
    }

    /// Remove a project. Sessions and transcripts cascade with foreign keys.
    pub fn delete_project(&self, project_id: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM artifact_approvals WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM artifact_index WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM task_evidence WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
        transaction.commit()?;
        Ok(())
    }

    /// Record one explicit approve/reject decision for a project-relative path.
    /// A later decision for the same path replaces the earlier row; callers
    /// compare `revision` with the current file hash to detect stale approvals.
    #[allow(clippy::too_many_arguments)]
    pub fn set_artifact_approval(
        &self,
        project_id: &str,
        path: &str,
        revision: &str,
        status: &str,
        actor: &str,
        note: &str,
        commit: &str,
    ) -> Result<()> {
        if project_id.is_empty()
            || path.is_empty()
            || revision.is_empty()
            || !matches!(status, "accepted" | "rejected")
        {
            return Err(StoreError::InvalidGroup("Invalid artifact approval".into()));
        }
        self.connection.execute(
            r#"
            INSERT INTO artifact_approvals
                (project_id, path, revision, status, actor, note, head_commit, approved_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())
            ON CONFLICT(project_id, path) DO UPDATE SET
                revision = excluded.revision,
                status = excluded.status,
                actor = excluded.actor,
                note = excluded.note,
                head_commit = excluded.head_commit,
                approved_at = excluded.approved_at
            "#,
            params![project_id, path, revision, status, actor, note, commit],
        )?;
        Ok(())
    }

    pub fn artifact_approvals(&self, project_id: &str) -> Result<Vec<ArtifactApproval>> {
        let mut statement = self.connection.prepare(
            "SELECT project_id, path, revision, status, actor, note, head_commit, approved_at \
             FROM artifact_approvals WHERE project_id = ?1 ORDER BY approved_at DESC, path",
        )?;
        let rows = statement.query_map([project_id], |row| {
            Ok(ArtifactApproval {
                project_id: row.get(0)?,
                path: row.get(1)?,
                revision: row.get(2)?,
                status: row.get(3)?,
                actor: row.get(4)?,
                note: row.get(5)?,
                commit: row.get(6)?,
                approved_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_artifact_approval(&self, project_id: &str, path: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM artifact_approvals WHERE project_id = ?1 AND path = ?2",
            params![project_id, path],
        )?;
        Ok(())
    }

    /// Replace the derived artifact index for one project. Files remain the
    /// source of truth; this table is a rebuildable cache for metrics, audits
    /// and fast lookups, never an authority for approval or content.
    pub fn replace_artifact_index(
        &self,
        project_id: &str,
        artifacts: &[dunlin_artifacts::Artifact],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM artifact_index WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM task_evidence WHERE project_id = ?1",
            [project_id],
        )?;
        for artifact in artifacts {
            let kind = serde_json::to_string(&artifact.kind)?;
            let status = serde_json::to_string(&artifact.status)?;
            let payload = serde_json::to_string(artifact)?;
            transaction.execute(
                r#"
                INSERT INTO artifact_index
                    (project_id, path, kind, status, title, revision, modified_ms, payload, indexed_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch())
                ON CONFLICT(project_id, path) DO UPDATE SET
                    kind = excluded.kind,
                    status = excluded.status,
                    title = excluded.title,
                    revision = excluded.revision,
                    modified_ms = excluded.modified_ms,
                    payload = excluded.payload,
                    indexed_at = excluded.indexed_at
                "#,
                params![
                    project_id,
                    artifact.path,
                    kind,
                    status,
                    artifact.title,
                    artifact.revision,
                    artifact.modified_ms,
                    payload,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn artifact_index(&self, project_id: &str) -> Result<Vec<dunlin_artifacts::Artifact>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM artifact_index WHERE project_id = ?1 ORDER BY path")?;
        let rows = statement.query_map([project_id], |row| row.get::<_, String>(0))?;
        let mut artifacts = Vec::new();
        for payload in rows {
            let payload = payload?;
            match serde_json::from_str(&payload) {
                Ok(artifact) => artifacts.push(artifact),
                Err(error) => {
                    return Err(StoreError::Json(error));
                }
            }
        }
        Ok(artifacts)
    }

    pub fn clear_artifact_index(&self, project_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM artifact_index WHERE project_id = ?1",
            [project_id],
        )?;
        Ok(())
    }

    pub fn set_task_evidence(
        &self,
        project_id: &str,
        task_id: &str,
        kind: &str,
        reference: &str,
        summary: &str,
    ) -> Result<()> {
        if project_id.is_empty()
            || task_id.is_empty()
            || kind.is_empty()
            || reference.is_empty()
            || task_id.len() > 64
            || kind.len() > 32
            || reference.len() > 512
            || summary.len() > 2048
        {
            return Err(StoreError::InvalidGroup("Invalid task evidence".into()));
        }
        self.connection.execute(
            r#"
            INSERT INTO task_evidence (project_id, task_id, kind, reference, summary, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
            ON CONFLICT(project_id, task_id, kind, reference) DO UPDATE SET
                summary = excluded.summary,
                created_at = excluded.created_at
            "#,
            params![project_id, task_id, kind, reference, summary],
        )?;
        Ok(())
    }

    pub fn task_evidence(&self, project_id: &str) -> Result<Vec<TaskEvidence>> {
        let mut statement = self.connection.prepare(
            "SELECT project_id, task_id, kind, reference, summary, created_at \
             FROM task_evidence WHERE project_id = ?1 ORDER BY task_id, created_at DESC",
        )?;
        let rows = statement.query_map([project_id], |row| {
            Ok(TaskEvidence {
                project_id: row.get(0)?,
                task_id: row.get(1)?,
                kind: row.get(2)?,
                reference: row.get(3)?,
                summary: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_task_evidence(&self, project_id: &str, task_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM task_evidence WHERE project_id = ?1 AND task_id = ?2",
            params![project_id, task_id],
        )?;
        Ok(())
    }

    fn secret_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<dunlin_domain::vault::Secret> {
        fn parse<T: DeserializeOwned>(value: &str, column: usize) -> rusqlite::Result<T> {
            serde_json::from_str(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    column,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        }
        Ok(dunlin_domain::vault::Secret {
            id: row.get(0)?,
            namespace: row.get(1)?,
            alias: row.get(2)?,
            label: row.get(3)?,
            description: row.get(4)?,
            injection: parse(&row.get::<_, String>(5)?, 5)?,
            allowed_hosts: parse(&row.get::<_, String>(6)?, 6)?,
            path_prefix: row.get(7)?,
            allowed_methods: parse(&row.get::<_, String>(8)?, 8)?,
            allowed_headers: parse(&row.get::<_, String>(9)?, 9)?,
            auto_approve: row.get::<_, i64>(10)? != 0,
            expires_at: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
            last_used_at: row.get(14)?,
        })
    }

    fn recipe_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<dunlin_domain::vault::Recipe> {
        fn parse<T: DeserializeOwned>(value: &str, column: usize) -> rusqlite::Result<T> {
            serde_json::from_str(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    column,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        }
        Ok(dunlin_domain::vault::Recipe {
            id: row.get(0)?,
            secret_id: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            program: row.get(4)?,
            args: parse(&row.get::<_, String>(5)?, 5)?,
            env_var: row.get(6)?,
            parameters: parse(&row.get::<_, String>(7)?, 7)?,
            timeout_secs: row.get::<_, i64>(8)? as u64,
            program_hash: row.get(9)?,
            created_at: row.get(10)?,
        })
    }

    fn audit_from_row(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<dunlin_domain::vault::SecretAudit> {
        Ok(dunlin_domain::vault::SecretAudit {
            id: row.get(0)?,
            secret_id: row.get(1)?,
            alias: row.get(2)?,
            project_id: row.get(3)?,
            session_id: row.get(4)?,
            operation: row.get(5)?,
            detail: row.get(6)?,
            decision: row.get(7)?,
            status: row.get(8)?,
            error: row.get(9)?,
            created_at: row.get(10)?,
        })
    }

    pub fn upsert_secret(&self, secret: &dunlin_domain::vault::Secret) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO secrets (
                id, namespace, alias, label, description, injection, allowed_hosts,
                path_prefix, allowed_methods, allowed_headers, auto_approve, expires_at,
                created_at, updated_at, last_used_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace,
                alias = excluded.alias,
                label = excluded.label,
                description = excluded.description,
                injection = excluded.injection,
                allowed_hosts = excluded.allowed_hosts,
                path_prefix = excluded.path_prefix,
                allowed_methods = excluded.allowed_methods,
                allowed_headers = excluded.allowed_headers,
                auto_approve = excluded.auto_approve,
                expires_at = excluded.expires_at,
                updated_at = excluded.updated_at,
                last_used_at = excluded.last_used_at
            "#,
            params![
                secret.id,
                secret.namespace,
                secret.alias,
                secret.label,
                secret.description,
                serde_json::to_string(&secret.injection)?,
                serde_json::to_string(&secret.allowed_hosts)?,
                secret.path_prefix,
                serde_json::to_string(&secret.allowed_methods)?,
                serde_json::to_string(&secret.allowed_headers)?,
                secret.auto_approve as i64,
                secret.expires_at,
                secret.created_at,
                secret.updated_at,
                secret.last_used_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_secret(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM secrets WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn secrets(&self) -> Result<Vec<dunlin_domain::vault::Secret>> {
        let mut statement = self.connection.prepare(
            "SELECT id, namespace, alias, label, description, injection, allowed_hosts, \
             path_prefix, allowed_methods, allowed_headers, auto_approve, expires_at, \
             created_at, updated_at, last_used_at FROM secrets ORDER BY created_at, alias",
        )?;
        let rows = statement.query_map([], Self::secret_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn secret_by_id(&self, id: &str) -> Result<Option<dunlin_domain::vault::Secret>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, namespace, alias, label, description, injection, allowed_hosts, \
                 path_prefix, allowed_methods, allowed_headers, auto_approve, expires_at, \
                 created_at, updated_at, last_used_at FROM secrets WHERE id = ?1",
                [id],
                Self::secret_from_row,
            )
            .optional()?)
    }

    pub fn secret_by_alias(&self, alias: &str) -> Result<Option<dunlin_domain::vault::Secret>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, namespace, alias, label, description, injection, allowed_hosts, \
                 path_prefix, allowed_methods, allowed_headers, auto_approve, expires_at, \
                 created_at, updated_at, last_used_at FROM secrets WHERE alias = ?1",
                [alias],
                Self::secret_from_row,
            )
            .optional()?)
    }

    pub fn secrets_for_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<dunlin_domain::vault::Secret>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, s.namespace, s.alias, s.label, s.description, s.injection, \
             s.allowed_hosts, s.path_prefix, s.allowed_methods, s.allowed_headers, \
             s.auto_approve, s.expires_at, s.created_at, s.updated_at, s.last_used_at \
             FROM secrets s JOIN secret_grants g ON g.secret_id = s.id \
             WHERE g.project_id = ?1 AND g.policy != 'deny' ORDER BY s.created_at, s.alias",
        )?;
        let rows = statement.query_map([project_id], Self::secret_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_secret_grant(&self, grant: &dunlin_domain::vault::SecretGrant) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO secret_grants (secret_id, project_id, policy, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(secret_id, project_id) DO UPDATE SET
                policy = excluded.policy,
                created_at = excluded.created_at
            "#,
            params![
                grant.secret_id,
                grant.project_id,
                grant.policy.as_str(),
                grant.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn remove_secret_grant(&self, secret_id: &str, project_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM secret_grants WHERE secret_id = ?1 AND project_id = ?2",
            params![secret_id, project_id],
        )?;
        Ok(())
    }

    pub fn secret_grant(
        &self,
        secret_id: &str,
        project_id: &str,
    ) -> Result<Option<dunlin_domain::vault::SecretGrant>> {
        Ok(self
            .connection
            .query_row(
                "SELECT secret_id, project_id, policy, created_at FROM secret_grants \
                 WHERE secret_id = ?1 AND project_id = ?2",
                params![secret_id, project_id],
                |row| {
                    let policy: String = row.get(2)?;
                    Ok(dunlin_domain::vault::SecretGrant {
                        secret_id: row.get(0)?,
                        project_id: row.get(1)?,
                        policy: dunlin_domain::vault::GrantPolicy::parse(&policy),
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn secret_grants(&self, secret_id: &str) -> Result<Vec<dunlin_domain::vault::SecretGrant>> {
        let mut statement = self.connection.prepare(
            "SELECT secret_id, project_id, policy, created_at FROM secret_grants \
             WHERE secret_id = ?1 ORDER BY project_id",
        )?;
        let rows = statement.query_map([secret_id], |row| {
            let policy: String = row.get(2)?;
            Ok(dunlin_domain::vault::SecretGrant {
                secret_id: row.get(0)?,
                project_id: row.get(1)?,
                policy: dunlin_domain::vault::GrantPolicy::parse(&policy),
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_recipe(&self, recipe: &dunlin_domain::vault::Recipe) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO secret_recipes (
                id, secret_id, name, description, program, args, env_var, parameters,
                timeout_secs, program_hash, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                program = excluded.program,
                args = excluded.args,
                env_var = excluded.env_var,
                parameters = excluded.parameters,
                timeout_secs = excluded.timeout_secs,
                program_hash = excluded.program_hash
            "#,
            params![
                recipe.id,
                recipe.secret_id,
                recipe.name,
                recipe.description,
                recipe.program,
                serde_json::to_string(&recipe.args)?,
                recipe.env_var,
                serde_json::to_string(&recipe.parameters)?,
                recipe.timeout_secs as i64,
                recipe.program_hash,
                recipe.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_recipe(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM secret_recipes WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn recipes_for_secret(&self, secret_id: &str) -> Result<Vec<dunlin_domain::vault::Recipe>> {
        let mut statement = self.connection.prepare(
            "SELECT id, secret_id, name, description, program, args, env_var, parameters, timeout_secs, \
             program_hash, created_at FROM secret_recipes WHERE secret_id = ?1 ORDER BY name",
        )?;
        let rows = statement.query_map([secret_id], Self::recipe_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn recipe_by_name(
        &self,
        secret_id: &str,
        name: &str,
    ) -> Result<Option<dunlin_domain::vault::Recipe>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, secret_id, name, description, program, args, env_var, parameters, timeout_secs, \
                 program_hash, created_at FROM secret_recipes WHERE secret_id = ?1 AND name = ?2",
                params![secret_id, name],
                Self::recipe_from_row,
            )
            .optional()?)
    }

    pub fn record_secret_audit(&self, audit: &dunlin_domain::vault::SecretAudit) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO secret_audit (
                secret_id, alias, project_id, session_id, operation, detail,
                decision, status, error, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                audit.secret_id,
                audit.alias,
                audit.project_id,
                audit.session_id,
                audit.operation,
                audit.detail,
                audit.decision,
                audit.status,
                audit.error,
                audit.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn secret_audit(
        &self,
        secret_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<dunlin_domain::vault::SecretAudit>> {
        let limit = limit.clamp(1, 200) as i64;
        match secret_id {
            Some(secret_id) => {
                let mut statement = self.connection.prepare(
                    "SELECT id, secret_id, alias, project_id, session_id, operation, detail, \
                     decision, status, error, created_at FROM secret_audit \
                     WHERE secret_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
                )?;
                let rows = statement.query_map(params![secret_id, limit], Self::audit_from_row)?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            }
            None => {
                let mut statement = self.connection.prepare(
                    "SELECT id, secret_id, alias, project_id, session_id, operation, detail, \
                     decision, status, error, created_at FROM secret_audit \
                     ORDER BY created_at DESC, id DESC LIMIT ?1",
                )?;
                let rows = statement.query_map([limit], Self::audit_from_row)?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            }
        }
    }

    pub fn expired_secrets(&self, now: i64) -> Result<Vec<dunlin_domain::vault::Secret>> {
        let mut statement = self.connection.prepare(
            "SELECT id, namespace, alias, label, description, injection, allowed_hosts, \
             path_prefix, allowed_methods, allowed_headers, auto_approve, expires_at, \
             created_at, updated_at, last_used_at FROM secrets \
             WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        )?;
        let rows = statement.query_map([now], Self::secret_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn touch_secret(&self, id: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE secrets SET last_used_at = ?2, updated_at = updated_at WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    /// Store an application setting as JSON under a stable key.
    pub fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let value = serde_json::to_string(value)?;
        self.connection.execute(
            r#"
            INSERT INTO settings (key, value, updated_at)
            VALUES (?1, ?2, unixepoch())
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            "#,
            params![key, value],
        )?;
        Ok(())
    }

    pub fn load_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    /// Replace the derived knowledge index for one project. Files remain the
    /// canonical source; this method never touches the project directory.
    pub fn replace_knowledge(
        &self,
        project_id: &str,
        objects: &[dunlin_knowledge::KnowledgeObject],
        edges: &[dunlin_knowledge::KnowledgeEdge],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM knowledge_objects WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM knowledge_edges WHERE project_id = ?1",
            [project_id],
        )?;
        for object in objects {
            transaction.execute(
                "INSERT INTO knowledge_objects
                 (project_id, id, kind, title, status, revision, path, payload, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    project_id,
                    object.id,
                    object.kind,
                    object.title,
                    object.status,
                    object.revision,
                    object.path,
                    serde_json::to_string(object)?,
                    object.updated_ms,
                ],
            )?;
        }
        for edge in edges {
            transaction.execute(
                "INSERT INTO knowledge_edges (project_id, from_id, to_id, relation, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project_id,
                    edge.from_id,
                    edge.to_id,
                    edge.relation,
                    serde_json::to_string(&edge.metadata)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn knowledge_objects(
        &self,
        project_id: &str,
    ) -> Result<Vec<dunlin_knowledge::KnowledgeObject>> {
        let mut statement = self.connection.prepare(
            "SELECT payload FROM knowledge_objects WHERE project_id = ?1 ORDER BY kind, id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let payload: String = row.get(0)?;
            Ok(serde_json::from_str(&payload))
        })?;
        let mut objects = Vec::new();
        for row in rows {
            if let Ok(object) = row? {
                objects.push(object);
            }
        }
        Ok(objects)
    }

    pub fn knowledge_edges(
        &self,
        project_id: &str,
    ) -> Result<Vec<dunlin_knowledge::KnowledgeEdge>> {
        let mut statement = self.connection.prepare(
            "SELECT from_id, to_id, relation, metadata FROM knowledge_edges
             WHERE project_id = ?1 ORDER BY from_id, relation, to_id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let metadata: String = row.get(3)?;
            Ok(dunlin_knowledge::KnowledgeEdge {
                from_id: row.get(0)?,
                to_id: row.get(1)?,
                relation: row.get(2)?,
                metadata: serde_json::from_str(&metadata).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_run(&self, run: &RunRecord) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO runs (
                id, project_id, session_id, intent_id, task_id, handoff_id,
                harness, model, permission_mode, worktree, budget_id,
                envelope_hash, status, started_ms, ended_ms, result, payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                ended_ms = excluded.ended_ms,
                result = excluded.result,
                payload = excluded.payload
            "#,
            params![
                run.id,
                run.project_id,
                run.session_id,
                run.intent_id,
                run.task_id,
                run.handoff_id,
                run.harness,
                run.model,
                run.permission_mode,
                run.worktree,
                run.budget_id,
                run.envelope_hash,
                run.status,
                run.started_ms,
                run.ended_ms,
                run.result,
                serde_json::to_string(&run.payload)?,
            ],
        )?;
        Ok(())
    }

    pub fn runs(&self, project_id: &str) -> Result<Vec<RunRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, session_id, intent_id, task_id, handoff_id, harness, model,
                    permission_mode, worktree, budget_id, envelope_hash, status, started_ms,
                    ended_ms, result, payload
             FROM runs WHERE project_id = ?1 ORDER BY started_ms DESC, id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let payload: String = row.get(16)?;
            Ok(RunRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                session_id: row.get(2)?,
                intent_id: row.get(3)?,
                task_id: row.get(4)?,
                handoff_id: row.get(5)?,
                harness: row.get(6)?,
                model: row.get(7)?,
                permission_mode: row.get(8)?,
                worktree: row.get(9)?,
                budget_id: row.get(10)?,
                envelope_hash: row.get(11)?,
                status: row.get(12)?,
                started_ms: row.get(13)?,
                ended_ms: row.get(14)?,
                result: row.get(15)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_artifact(&self, artifact: &ArtifactRecord) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO artifacts (
                id, project_id, run_id, session_id, intent_id, task_id, kind, title,
                mime, bytes, sha256, blob_path, created_ms, visibility, expires_ms,
                revoked_ms, payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                revoked_ms = excluded.revoked_ms,
                visibility = excluded.visibility,
                payload = excluded.payload
            "#,
            params![
                artifact.id,
                artifact.project_id,
                artifact.run_id,
                artifact.session_id,
                artifact.intent_id,
                artifact.task_id,
                artifact.kind,
                artifact.title,
                artifact.mime,
                artifact.bytes,
                artifact.sha256,
                artifact.blob_path,
                artifact.created_ms,
                artifact.visibility,
                artifact.expires_ms,
                artifact.revoked_ms,
                serde_json::to_string(&artifact.payload)?,
            ],
        )?;
        Ok(())
    }

    pub fn delete_artifact(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM artifacts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn artifacts(&self, project_id: &str) -> Result<Vec<ArtifactRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, run_id, session_id, intent_id, task_id, kind, title, mime,
                    bytes, sha256, blob_path, created_ms, visibility, expires_ms, revoked_ms, payload
             FROM artifacts WHERE project_id = ?1 ORDER BY created_ms DESC, id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let payload: String = row.get(16)?;
            Ok(ArtifactRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                run_id: row.get(2)?,
                session_id: row.get(3)?,
                intent_id: row.get(4)?,
                task_id: row.get(5)?,
                kind: row.get(6)?,
                title: row.get(7)?,
                mime: row.get(8)?,
                bytes: row.get(9)?,
                sha256: row.get(10)?,
                blob_path: row.get(11)?,
                created_ms: row.get(12)?,
                visibility: row.get(13)?,
                expires_ms: row.get(14)?,
                revoked_ms: row.get(15)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_claim(&self, claim: &ClaimRecord) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO claims (
                id, project_id, intent_id, task_id, statement, status, confidence,
                scope, last_verified_ms, stale_reason, payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                statement = excluded.statement,
                status = excluded.status,
                confidence = excluded.confidence,
                scope = excluded.scope,
                last_verified_ms = excluded.last_verified_ms,
                stale_reason = excluded.stale_reason,
                payload = excluded.payload
            "#,
            params![
                claim.id,
                claim.project_id,
                claim.intent_id,
                claim.task_id,
                claim.statement,
                claim.status,
                claim.confidence,
                claim.scope,
                claim.last_verified_ms,
                claim.stale_reason,
                serde_json::to_string(&claim.payload)?,
            ],
        )?;
        Ok(())
    }

    pub fn claims(&self, project_id: &str) -> Result<Vec<ClaimRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, intent_id, task_id, statement, status, confidence, scope,
                    last_verified_ms, stale_reason, payload
             FROM claims WHERE project_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let payload: String = row.get(10)?;
            Ok(ClaimRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                intent_id: row.get(2)?,
                task_id: row.get(3)?,
                statement: row.get(4)?,
                status: row.get(5)?,
                confidence: row.get(6)?,
                scope: row.get(7)?,
                last_verified_ms: row.get(8)?,
                stale_reason: row.get(9)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_evidence(&self, evidence: &EvidenceRecord) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO evidence (
                id, project_id, kind, artifact_id, run_id, source, revision, hash,
                observed_ms, freshness, summary, payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(id) DO UPDATE SET
                freshness = excluded.freshness,
                summary = excluded.summary,
                payload = excluded.payload
            "#,
            params![
                evidence.id,
                evidence.project_id,
                evidence.kind,
                evidence.artifact_id,
                evidence.run_id,
                evidence.source,
                evidence.revision,
                evidence.hash,
                evidence.observed_ms,
                evidence.freshness,
                evidence.summary,
                serde_json::to_string(&evidence.payload)?,
            ],
        )?;
        Ok(())
    }

    pub fn evidence(&self, project_id: &str) -> Result<Vec<EvidenceRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, kind, artifact_id, run_id, source, revision, hash,
                    observed_ms, freshness, summary, payload
             FROM evidence WHERE project_id = ?1 ORDER BY observed_ms DESC, id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            let payload: String = row.get(11)?;
            Ok(EvidenceRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                kind: row.get(2)?,
                artifact_id: row.get(3)?,
                run_id: row.get(4)?,
                source: row.get(5)?,
                revision: row.get(6)?,
                hash: row.get(7)?,
                observed_ms: row.get(8)?,
                freshness: row.get(9)?,
                summary: row.get(10)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn link_evidence(&self, evidence_id: &str, claim_id: &str, relation: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO evidence_claims (evidence_id, claim_id, relation)
             VALUES (?1, ?2, ?3)",
            params![evidence_id, claim_id, relation],
        )?;
        Ok(())
    }

    pub fn evidence_links(&self, claim_id: &str) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT evidence_id, relation FROM evidence_claims WHERE claim_id = ?1
             ORDER BY evidence_id, relation",
        )?;
        let rows = statement.query_map([claim_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_plugin(&self, plugin: &PluginRecord) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO plugin_registry (
                id, version, manifest_hash, wasm_hash, directory, enabled, installed_ms, payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                version = excluded.version,
                manifest_hash = excluded.manifest_hash,
                wasm_hash = excluded.wasm_hash,
                directory = excluded.directory,
                enabled = excluded.enabled,
                payload = excluded.payload
            "#,
            params![
                plugin.id,
                plugin.version,
                plugin.manifest_hash,
                plugin.wasm_hash,
                plugin.directory,
                plugin.enabled as i64,
                plugin.installed_ms,
                serde_json::to_string(&plugin.payload)?,
            ],
        )?;
        Ok(())
    }

    pub fn plugins(&self) -> Result<Vec<PluginRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, version, manifest_hash, wasm_hash, directory, enabled, installed_ms, payload
             FROM plugin_registry ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            let payload: String = row.get(7)?;
            Ok(PluginRecord {
                id: row.get(0)?,
                version: row.get(1)?,
                manifest_hash: row.get(2)?,
                wasm_hash: row.get(3)?,
                directory: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                installed_ms: row.get(6)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_ssh_host(&self, host: &dunlin_domain::SshHost) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO ssh_hosts
                (id, name, target, user, port, identity, relay_port, local_port, host_key, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                target = excluded.target,
                user = excluded.user,
                port = excluded.port,
                identity = excluded.identity,
                relay_port = excluded.relay_port,
                local_port = excluded.local_port,
                host_key = excluded.host_key
            "#,
            params![
                host.id,
                host.name,
                host.target,
                host.user,
                host.port as i64,
                host.identity,
                host.relay_port as i64,
                host.local_port as i64,
                host.host_key,
                host.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn ssh_hosts(&self) -> Result<Vec<dunlin_domain::SshHost>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, target, user, port, identity, relay_port, local_port, host_key, created_at
             FROM ssh_hosts ORDER BY name, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(dunlin_domain::SshHost {
                id: row.get(0)?,
                name: row.get(1)?,
                target: row.get(2)?,
                user: row.get(3)?,
                port: row.get::<_, i64>(4)? as u16,
                identity: row.get(5)?,
                relay_port: row.get::<_, i64>(6)? as u16,
                local_port: row.get::<_, i64>(7)? as u16,
                host_key: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_ssh_host(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM ssh_hosts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_plugin_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE plugin_registry SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::{
        PermissionMode, ProjectKind, ProjectTone, RemoteProject, SessionOrigin, SessionStatus,
    };

    fn project() -> Project {
        Project {
            id: "dunlin".into(),
            name: "Dunlin".into(),
            path: "/tmp/dunlin".into(),
            branch: Some("main".into()),
            tone: ProjectTone::Teal,
            pinned: true,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    fn session() -> SessionSummary {
        SessionSummary {
            workspace: None,
            id: "ses-1".into(),
            project_id: "dunlin".into(),
            title: "Verify permissions".into(),
            status: SessionStatus::Waiting,
            harness: "OpenCode".into(),
            model: "Auto".into(),
            branch: Some("main".into()),
            updated_at: "now".into(),
            cost_usd: 0.12,
            duration_secs: 42,
            tokens: 1_024,
            summary: "Round-trip a permission request.".into(),
            origin: SessionOrigin::Local,
            acp_session_id: None,
        }
    }

    #[test]
    fn catalog_import_is_atomic_additive_and_preserves_deferred_checkout_after_reopen() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dunlin-catalog-import-{unique}.db"));
        let store = Store::open(&path).unwrap();
        let existing = project();
        store.upsert_project(&existing).unwrap();
        store.upsert_session(&session()).unwrap();
        store
            .save_setting("app", &serde_json::json!({"chat_logo":true}))
            .unwrap();
        let root = ProjectGroup {
            id: "root".into(),
            name: "Root".into(),
            parent_id: None,
            tone: ProjectTone::Teal,
            settings: Default::default(),
        };
        let child = ProjectGroup {
            id: "child".into(),
            name: "Child".into(),
            parent_id: Some(root.id.clone()),
            tone: ProjectTone::Blue,
            settings: Default::default(),
        };
        let mut pending = existing.clone();
        pending.id = "imported".into();
        pending.trusted = false;
        pending.path = "/chosen/github.com/o/r".into();
        pending.group_id = Some(child.id.clone());
        pending.settings.pending_clone = Some(dunlin_domain::PendingClone {
            url: "https://github.com/o/r".into(),
            destination: "/chosen".into(),
        });
        let settings = serde_json::json!({"chat_logo":false});
        assert!(
            store
                .import_catalog(
                    &[root.clone(), child.clone()],
                    &[pending.clone(), existing.clone()],
                    Some(&settings)
                )
                .is_err()
        );
        assert!(store.project_groups().unwrap().is_empty());
        assert_eq!(store.projects().unwrap().len(), 1);
        assert_eq!(
            store
                .load_setting::<serde_json::Value>("app")
                .unwrap()
                .unwrap()["chat_logo"],
            true
        );
        store
            .import_catalog(&[root, child], &[pending.clone()], Some(&settings))
            .unwrap();
        assert_eq!(store.sessions(None).unwrap().len(), 1);
        drop(store);
        let store = Store::open(&path).unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(projects.len(), 2);
        let loaded = projects.iter().find(|p| p.id == pending.id).unwrap();
        assert!(loaded.is_not_downloaded() && !loaded.trusted);
        assert_eq!(
            loaded.settings.pending_clone,
            pending.settings.pending_clone
        );
        assert_eq!(store.project_groups().unwrap().len(), 2);
        assert_eq!(
            store
                .load_setting::<serde_json::Value>("app")
                .unwrap()
                .unwrap(),
            settings
        );
        let old = projects.iter().find(|p| p.id == existing.id).unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(existing).unwrap()
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn correcting_session_location_preserves_history_and_other_metadata() {
        let store = Store::open_in_memory().unwrap();
        let source = project();
        let mut target = source.clone();
        target.id = "other-repo".into();
        target.path = "/work/other-repo".into();
        store.upsert_project(&source).unwrap();
        store.upsert_project(&target).unwrap();
        let mut imported = session();
        imported.id = "import-history".into();
        let transcript = vec!["imported conversation", "local follow-up"];
        store
            .import_sessions(&[(imported.clone(), transcript.clone())])
            .unwrap();

        store
            .update_session_location(&imported.id, &target.id, Some("/work/other-repo/src"))
            .unwrap();
        assert!(store.sessions(Some(&source.id)).unwrap().is_empty());
        let moved = store.sessions(Some(&target.id)).unwrap().pop().unwrap();
        imported.project_id = target.id.clone();
        imported.workspace = Some("/work/other-repo/src".into());
        assert_eq!(
            serde_json::to_value(&moved).unwrap(),
            serde_json::to_value(&imported).unwrap()
        );
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&imported.id)
                .unwrap()
                .unwrap(),
            transcript
        );

        store
            .update_session_location(&imported.id, &target.id, None)
            .unwrap();
        assert_eq!(
            store.sessions(Some(&target.id)).unwrap()[0].workspace,
            imported.workspace
        );
        assert!(
            store
                .update_session_location(&imported.id, "missing-project", Some("/wrong"))
                .is_err()
        );
        let unchanged = store.sessions(Some(&target.id)).unwrap().pop().unwrap();
        assert_eq!(
            serde_json::to_value(unchanged).unwrap(),
            serde_json::to_value(imported).unwrap()
        );
    }

    fn seed_factory_reset(store: &Store) {
        store
            .upsert_project_group(&group("reset-group", None))
            .unwrap();
        let mut project = project();
        project.group_id = Some("reset-group".into());
        project.settings.harness = Some("Custom ACP".into());
        store.upsert_project(&project).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_transcript("ses-1", &vec!["history"]).unwrap();
        store.save_setting("app", &"custom settings").unwrap();
        store
            .replace_artifact_index("dunlin", &[indexed_artifact("spec.md")])
            .unwrap();
        store.connection.execute_batch(
            "INSERT INTO secrets (id, namespace, alias, label, injection)
                VALUES ('secret', 'external', 'test', 'Test', '{\"type\":\"basic\",\"username\":\"test\"}');
             INSERT INTO secret_grants (secret_id, project_id) VALUES ('secret', 'dunlin');
             INSERT INTO secret_recipes (id, secret_id, name, program)
                VALUES ('recipe', 'secret', 'Test', '/usr/bin/true');
             INSERT INTO secret_audit (alias, operation, detail, decision)
                VALUES ('test', 'request', 'test', 'deny');
             INSERT INTO artifact_approvals (project_id, path, revision, status, actor)
                VALUES ('dunlin', 'spec.md', 'hash', 'approved', 'user');
             INSERT INTO task_evidence (project_id, task_id, kind, reference)
                VALUES ('dunlin', 'T001', 'commit', 'hash');
             INSERT INTO knowledge_objects (project_id, id, kind, title, status, revision, payload, updated_ms)
                 VALUES ('dunlin', 'INT-1', 'intent', 'Intent', 'accepted', 'hash', '{}', 1);
             INSERT INTO knowledge_edges (project_id, from_id, to_id, relation)
                 VALUES ('dunlin', 'INT-1', 'CON-1', 'has_concept');
             INSERT INTO runs (id, project_id, harness, model, permission_mode, status)
                 VALUES ('run-1', 'dunlin', 'OpenCode', 'model', 'ask', 'done');
             INSERT INTO artifacts (id, project_id, kind, title, mime, bytes, sha256, blob_path, created_ms)
                 VALUES ('art-1', 'dunlin', 'diff', 'Diff', 'text/plain', 1, 'hash', 'blob', 1);
             INSERT INTO claims (id, project_id, statement, status, confidence)
                 VALUES ('claim-1', 'dunlin', 'Claim', 'unverified', 'low');
             INSERT INTO evidence (id, project_id, kind, source, revision, hash, observed_ms, freshness)
                 VALUES ('ev-1', 'dunlin', 'check', 'check', 'hash', 'hash', 1, 'fresh');
             INSERT INTO evidence_claims (evidence_id, claim_id, relation)
                 VALUES ('ev-1', 'claim-1', 'supports');
             INSERT INTO plugin_registry (id, version, manifest_hash, directory)
                 VALUES ('plugin-1', '1.0.0', 'hash', '/tmp/plugin');
             INSERT INTO ssh_hosts (id, name, target, user, port)
                 VALUES ('ssh-1', 'Host', 'host', 'user', 22);"
        ).unwrap();
    }

    #[test]
    fn factory_reset_clears_every_app_table_and_remains_usable_after_reopening() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dunlin-factory-reset-{unique}.db"));
        let store = Store::open(&path).unwrap();
        seed_factory_reset(&store);
        let tables = store
            .connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            tables.len(),
            21,
            "include new app tables in reset and this fixture"
        );
        for table in &tables {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "fixture must populate {table}");
        }
        store.factory_reset().unwrap();
        store.factory_reset().unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        for table in tables {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "reset left records in {table}");
        }
        assert_eq!(store.quick_check().unwrap(), "ok");
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_setting("app", &"new defaults").unwrap();
        assert_eq!(store.projects().unwrap().len(), 1);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn factory_reset_rolls_back_all_deletions_if_any_table_fails() {
        let store = Store::open_in_memory().unwrap();
        seed_factory_reset(&store);
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_reset BEFORE DELETE ON settings
             BEGIN SELECT RAISE(ABORT, 'injected reset failure'); END;",
            )
            .unwrap();
        assert!(store.factory_reset().is_err());
        assert_eq!(store.projects().unwrap().len(), 1);
        assert_eq!(store.sessions(None).unwrap().len(), 1);
        assert_eq!(store.project_groups().unwrap().len(), 1);
        assert_eq!(store.secrets().unwrap().len(), 1);
        assert_eq!(
            store
                .load_transcript::<Vec<String>>("ses-1")
                .unwrap()
                .unwrap(),
            ["history"]
        );
        assert_eq!(
            store.load_setting::<String>("app").unwrap().as_deref(),
            Some("custom settings")
        );
        assert_eq!(store.task_evidence("dunlin").unwrap().len(), 1);
        assert_eq!(store.artifact_index("dunlin").unwrap().len(), 1);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_reset")
            .unwrap();
        store.factory_reset().unwrap();
        assert!(store.projects().unwrap().is_empty());
    }

    #[test]
    fn history_import_is_insert_only_deduplicated_and_atomic() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        let original = session();
        store.upsert_session(&original).unwrap();
        store
            .save_transcript(&original.id, &vec!["original"])
            .unwrap();
        let mut collision = original.clone();
        collision.title = "must not overwrite".into();
        let mut new = original.clone();
        new.id = "import-new".into();
        let rows = vec![
            (collision, vec!["replacement"]),
            (new.clone(), vec!["imported"]),
            (new.clone(), vec!["duplicate"]),
        ];
        assert_eq!(store.import_sessions(&rows).unwrap(), vec![new.id.clone()]);
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&original.id)
                .unwrap()
                .unwrap(),
            vec!["original"]
        );
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&new.id)
                .unwrap()
                .unwrap(),
            vec!["imported"]
        );
        assert!(store.import_sessions(&rows).unwrap().is_empty());
        assert_eq!(
            store
                .sessions(None)
                .unwrap()
                .iter()
                .find(|s| s.id == original.id)
                .unwrap()
                .title,
            original.title
        );
        let mut valid = original.clone();
        valid.id = "rollback-valid".into();
        let mut invalid = original;
        invalid.id = "rollback-invalid".into();
        invalid.project_id = "missing-project".into();
        assert!(
            store
                .import_sessions(&[(valid.clone(), vec!["one"]), (invalid, vec!["two"])])
                .is_err()
        );
        assert!(
            store
                .load_transcript::<Vec<String>>(&valid.id)
                .unwrap()
                .is_none()
        );
        assert_eq!(store.sessions(None).unwrap().len(), 2);
    }

    #[test]
    fn streamed_import_reports_duplicates_and_rolls_back_on_a_source_read_failure() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        let original = session();
        store.upsert_session(&original).unwrap();
        store
            .save_transcript(&original.id, &vec!["original"])
            .unwrap();
        let mut new = original.clone();
        new.id = "streamed-new".into();
        let mut progress = Vec::new();
        let rows = vec![
            Ok((original.clone(), vec!["must not overwrite"])),
            Ok((new.clone(), vec!["new transcript"])),
            Err(StoreError::Io(std::io::Error::other(
                "import cache read failed",
            ))),
        ];
        assert!(
            store
                .import_session_stream(rows, |done| progress.push(done))
                .is_err()
        );
        assert_eq!(progress, [1, 2]);
        assert_eq!(store.sessions(None).unwrap().len(), 1);
        assert!(
            store
                .load_transcript::<Vec<String>>(&new.id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&original.id)
                .unwrap()
                .unwrap(),
            ["original"]
        );
        progress.clear();
        let rows = vec![
            Ok((original, vec!["unchanged"])),
            Ok((new.clone(), vec!["new transcript"])),
        ];
        assert_eq!(
            store
                .import_session_stream(rows, |done| progress.push(done))
                .unwrap(),
            [new.id]
        );
        assert_eq!(progress, [1, 2]);
    }

    #[test]
    fn round_trips_projects_and_sessions() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Dunlin");
        assert_eq!(projects[0].tone, ProjectTone::Teal);

        let sessions = store.sessions(Some("dunlin")).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Waiting);
        assert_eq!(sessions[0].tokens, 1_024);

        assert!(store.sessions(Some("missing")).unwrap().is_empty());
    }

    #[test]
    fn changing_project_defaults_preserves_existing_conversation_harnesses() {
        let store = Store::open_in_memory().unwrap();
        let mut project = project();
        store.upsert_project(&project).unwrap();
        let mut old = session();
        old.harness = "Claude Code".into();
        old.acp_session_id = Some("claude-session-1".into());
        store.upsert_session(&old).unwrap();
        project.settings.harness = Some("OpenAI Codex".into());
        store.upsert_project(&project).unwrap();
        let saved = &store.sessions(None).unwrap()[0];
        assert_eq!(saved.harness, "Claude Code");
        assert_eq!(saved.acp_session_id.as_deref(), Some("claude-session-1"));
        assert_eq!(
            store.projects().unwrap()[0].settings.harness.as_deref(),
            Some("OpenAI Codex")
        );
    }

    #[test]
    fn removing_the_last_project_keeps_its_files_and_app_settings() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-remove-project-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("keep.txt");
        std::fs::write(&file, "user work").unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut project = project();
        project.path = root.to_string_lossy().to_string();
        store.upsert_project(&project).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_setting("app", &"preferences").unwrap();
        store.delete_project(&project.id).unwrap();
        assert!(store.projects().unwrap().is_empty());
        assert!(store.sessions(None).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "user work");
        assert_eq!(
            store.load_setting::<String>("app").unwrap().as_deref(),
            Some("preferences")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_approvals_are_hash_bound_and_replaceable() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store
            .set_artifact_approval(
                "dunlin",
                "intent/a.md",
                "rev-1",
                "accepted",
                "reviewer",
                "ok",
                "abc",
            )
            .unwrap();
        let rows = store.artifact_approvals("dunlin").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].revision, "rev-1");
        assert_eq!(rows[0].status, "accepted");
        store
            .set_artifact_approval(
                "dunlin",
                "intent/a.md",
                "rev-2",
                "rejected",
                "reviewer",
                "changed",
                "def",
            )
            .unwrap();
        let rows = store.artifact_approvals("dunlin").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].revision, "rev-2");
        assert_eq!(rows[0].status, "rejected");
        assert_eq!(rows[0].commit, "def");
        assert!(
            store
                .set_artifact_approval(
                    "dunlin",
                    "intent/a.md",
                    "rev-3",
                    "approved",
                    "reviewer",
                    "",
                    ""
                )
                .is_err()
        );
        store
            .clear_artifact_approval("dunlin", "intent/a.md")
            .unwrap();
        assert!(store.artifact_approvals("dunlin").unwrap().is_empty());
    }

    #[test]
    fn deleting_a_project_also_removes_its_artifact_approvals() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store
            .set_artifact_approval(
                "dunlin",
                "intent/a.md",
                "rev-1",
                "accepted",
                "reviewer",
                "",
                "",
            )
            .unwrap();
        store.delete_project("dunlin").unwrap();
        assert!(store.artifact_approvals("dunlin").unwrap().is_empty());
    }

    fn indexed_artifact(path: &str) -> dunlin_artifacts::Artifact {
        dunlin_artifacts::Artifact {
            id: path.into(),
            kind: dunlin_artifacts::Kind::Spec,
            status: dunlin_artifacts::Status::Draft,
            title: "Spec".into(),
            path: path.into(),
            absolute: None,
            revision: "rev-1".into(),
            bytes: 10,
            modified_ms: 1,
            author: None,
            open_questions: Vec::new(),
            requirements: Vec::new(),
            tasks: Vec::new(),
            frontmatter: Default::default(),
            frontmatter_raw: String::new(),
            body: "# Spec
"
            .into(),
        }
    }

    #[test]
    fn artifact_index_is_replaceable_and_deleted_with_the_project() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store
            .replace_artifact_index(
                "dunlin",
                &[
                    indexed_artifact("specs/001/spec.md"),
                    indexed_artifact("specs/001/plan.md"),
                ],
            )
            .unwrap();
        assert_eq!(store.artifact_index("dunlin").unwrap().len(), 2);
        store
            .replace_artifact_index("dunlin", &[indexed_artifact("specs/002/spec.md")])
            .unwrap();
        let rows = store.artifact_index("dunlin").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "specs/002/spec.md");
        store.delete_project("dunlin").unwrap();
        assert!(store.artifact_index("dunlin").unwrap().is_empty());
    }

    #[test]
    fn task_evidence_is_upserted_cleared_and_deleted_with_the_project() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store
            .set_task_evidence("dunlin", "T001", "commit", "abc123", "worktree branch")
            .unwrap();
        store
            .set_task_evidence("dunlin", "T001", "commit", "abc123", "updated summary")
            .unwrap();
        store
            .set_task_evidence("dunlin", "T001", "worktree", "dunlin/subtask/t001", "")
            .unwrap();
        let rows = store.task_evidence("dunlin").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.summary == "updated summary"));
        store.clear_task_evidence("dunlin", "T001").unwrap();
        assert!(store.task_evidence("dunlin").unwrap().is_empty());
        store
            .set_task_evidence("dunlin", "T002", "commit", "def456", "")
            .unwrap();
        store.delete_project("dunlin").unwrap();
        assert!(store.task_evidence("dunlin").unwrap().is_empty());
    }

    #[test]
    fn read_only_database_reports_failed_session_and_transcript_writes() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store
            .connection
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(store.upsert_session(&session()).is_err());
        assert!(
            store
                .save_transcript(&session().id, &vec!["unsaved"])
                .is_err()
        );
        assert!(store.delete_project(&project().id).is_err());
        assert_eq!(store.projects().unwrap().len(), 1);
    }

    #[test]
    fn chat_spaces_roundtrip_with_isolated_history_and_old_projects_stay_repos() {
        let store = Store::open_in_memory().unwrap();
        let mut repo = project();
        store.upsert_project(&repo).unwrap();
        let chat = Project {
            id: "chat-1".into(),
            name: "Chat".into(),
            path: "/tmp/dunlin-chat".into(),
            branch: None,
            tone: ProjectTone::Teal,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: ProjectKind::ChatSpace,
            remote: None,
            group_id: None,
            settings: Default::default(),
        };
        store.upsert_project(&chat).unwrap();
        let mut chat_session = session();
        chat_session.id = "chat-session".into();
        chat_session.project_id = chat.id.clone();
        store.upsert_session(&chat_session).unwrap();
        store
            .save_transcript("chat-session", &vec!["scratch history"])
            .unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(
            projects
                .iter()
                .find(|project| project.id == "chat-1")
                .unwrap()
                .kind,
            ProjectKind::ChatSpace
        );
        assert_eq!(
            projects
                .iter()
                .find(|project| project.id == repo.id)
                .unwrap()
                .kind,
            ProjectKind::Repo
        );
        assert_eq!(
            store.sessions(Some("chat-1")).unwrap().first().unwrap().id,
            "chat-session"
        );
        assert_eq!(
            store
                .load_transcript::<Vec<String>>("chat-session")
                .unwrap()
                .unwrap(),
            vec!["scratch history"]
        );
        repo.settings.harness = Some("OpenCode".into());
        store.upsert_project(&repo).unwrap();
        assert!(
            store
                .load_transcript::<Vec<String>>(&session().id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn round_trips_remote_projects_and_sessions() {
        let store = Store::open_in_memory().unwrap();
        let mut project = project();
        project.remote = Some(RemoteProject {
            host_id: "host-1".into(),
            project_id: "prj-1".into(),
            path: "/srv/repo".into(),
        });
        store.upsert_project(&project).unwrap();

        let mut session = session();
        session.origin = SessionOrigin::Remote {
            host_id: "host-1".into(),
        };
        session.acp_session_id = Some("acp-1".into());
        store.upsert_session(&session).unwrap();

        let projects = store.projects().unwrap();
        assert_eq!(projects[0].remote.as_ref().unwrap().project_id, "prj-1");
        let sessions = store.sessions(None).unwrap();
        assert!(sessions[0].origin.is_remote());
        assert_eq!(sessions[0].acp_session_id.as_deref(), Some("acp-1"));
    }

    #[test]
    fn groups_projects_and_ungroups_on_delete() {
        let store = Store::open_in_memory().unwrap();
        let group = ProjectGroup {
            id: "group-work".into(),
            name: "Work".into(),
            tone: ProjectTone::Violet,
            parent_id: None,
            settings: Default::default(),
        };
        store.upsert_project_group(&group).unwrap();
        let mut project = project();
        project.group_id = Some(group.id.clone());
        store.upsert_project(&project).unwrap();

        let groups = store.project_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Work");
        assert_eq!(
            store.projects().unwrap()[0].group_id.as_deref(),
            Some("group-work")
        );

        store.delete_project_group("group-work").unwrap();
        assert!(store.project_groups().unwrap().is_empty());
        assert!(store.projects().unwrap()[0].group_id.is_none());
    }

    fn group(id: &str, parent: Option<&str>) -> ProjectGroup {
        ProjectGroup {
            id: id.into(),
            name: id.into(),
            tone: ProjectTone::Blue,
            parent_id: parent.map(str::to_string),
            settings: Default::default(),
        }
    }

    #[test]
    fn v5_migration_preserves_groups_project_settings_histories_and_transcripts() {
        let store = Store::open_in_memory().unwrap();
        store
            .connection
            .execute_batch(
                "ALTER TABLE project_groups DROP COLUMN parent_id; ALTER TABLE project_groups DROP COLUMN settings; PRAGMA user_version = 5;",
            )
            .unwrap();
        store.connection.execute("INSERT INTO project_groups (id, name, tone) VALUES ('legacy', 'Private', '\"Teal\"')", []).unwrap();
        let mut project = project();
        project.group_id = Some("legacy".into());
        project.settings.harness = Some("Claude Code".into());
        store.upsert_project(&project).unwrap();
        store.upsert_session(&session()).unwrap();
        store
            .save_transcript("ses-1", &vec!["user history", "assistant answer"])
            .unwrap();
        let settings =
            serde_json::json!({"dark": false, "ui_font_family": "Custom", "sidebar_width": 333});
        store.save_setting("app", &settings).unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let groups = store.project_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].parent_id.is_none());
        assert_eq!(groups[0].name, "Private");
        assert_eq!(groups[0].settings, dunlin_domain::GroupSettings::default());
        let project = &store.projects().unwrap()[0];
        assert_eq!(project.group_id.as_deref(), Some("legacy"));
        assert_eq!(project.settings.harness.as_deref(), Some("Claude Code"));
        assert_eq!(store.sessions(None).unwrap()[0].tokens, session().tokens);
        assert_eq!(
            store
                .load_transcript::<Vec<String>>("ses-1")
                .unwrap()
                .unwrap(),
            ["user history", "assistant answer"]
        );
        assert_eq!(
            store.load_setting::<serde_json::Value>("app").unwrap(),
            Some(settings)
        );
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            18
        );
    }

    #[test]
    fn v14_session_workspace_and_activity_migration_preserves_unknown_history() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        let mut row = session();
        row.workspace = Some("/recorded/worktree".into());
        store.upsert_session(&row).unwrap();
        assert_eq!(store.sessions(None).unwrap()[0].workspace, row.workspace);
        store.save_transcript(&row.id, &vec!["history"]).unwrap();
        store.connection.execute_batch("ALTER TABLE sessions DROP COLUMN workspace; UPDATE sessions SET updated_at = 'now'; UPDATE transcripts SET updated_at = 1762344000; PRAGMA user_version = 13;").unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let migrated = &store.sessions(None).unwrap()[0];
        assert!(migrated.workspace.is_none());
        assert_eq!(migrated.updated_at, "2025-11-05T12:00:00Z");
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&row.id)
                .unwrap()
                .unwrap(),
            ["history"]
        );
        row.updated_at = "Imported".into();
        store.upsert_session(&row).unwrap();
        store
            .connection
            .execute_batch("PRAGMA user_version = 13;")
            .unwrap();
        store.migrate().unwrap();
        assert_eq!(store.sessions(None).unwrap()[0].updated_at, "Imported");
    }

    #[test]
    fn group_color_and_icon_roundtrip_without_changing_runtime_defaults() {
        let store = Store::open_in_memory().unwrap();
        let mut row = group("visual-group", None);
        row.tone = dunlin_domain::ProjectTone::Rose;
        row.settings.icon = Some("aws-lambda".into());
        row.settings.harness = Some("Custom ACP".into());
        store.upsert_project_group(&row).unwrap();
        let saved = store.project_groups().unwrap().pop().unwrap();
        assert_eq!(saved.tone, row.tone);
        assert_eq!(saved.settings, row.settings);
        row.settings.icon = Some("png:embedded-fixture".into());
        store.upsert_project_group(&row).unwrap();
        assert_eq!(
            store.project_groups().unwrap()[0].settings.icon,
            row.settings.icon
        );
    }

    #[test]
    fn v6_migration_adds_empty_group_defaults_without_rewriting_project_data() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project_group(&group("root", None)).unwrap();
        store
            .upsert_project_group(&group("child", Some("root")))
            .unwrap();
        let mut project = project();
        project.group_id = Some("child".into());
        project.settings.harness = Some("Custom ACP".into());
        project.settings.environment =
            dunlin_domain::parse_project_environment("LANG=pl_PL.UTF-8").unwrap();
        store.upsert_project(&project).unwrap();
        store.upsert_session(&session()).unwrap();
        store
            .save_transcript(&session().id, &vec!["keep history"])
            .unwrap();
        let old_payload: String = store
            .connection
            .query_row("SELECT settings FROM projects", [], |row| row.get(0))
            .unwrap();
        store
            .connection
            .execute_batch(
                "ALTER TABLE project_groups DROP COLUMN settings; PRAGMA user_version = 6;",
            )
            .unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let groups = store.project_groups().unwrap();
        assert!(
            groups
                .iter()
                .all(|g| g.settings == dunlin_domain::GroupSettings::default())
        );
        assert_eq!(
            groups
                .iter()
                .find(|group| group.id == "child")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("root")
        );
        let payload: String = store
            .connection
            .query_row("SELECT settings FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(payload, old_payload);
        assert_eq!(
            store
                .load_transcript::<Vec<String>>(&session().id)
                .unwrap()
                .unwrap(),
            vec!["keep history"]
        );
        assert_eq!(store.sessions(None).unwrap()[0].harness, session().harness);
        let legacy: ProjectGroup =
            serde_json::from_str(r#"{"id":"old","name":"Old","tone":"Blue"}"#).unwrap();
        assert_eq!(legacy.settings, dunlin_domain::GroupSettings::default());
    }

    #[test]
    fn group_defaults_roundtrip_reparent_promote_and_undo_without_copying_to_projects() {
        use dunlin_domain::resolve_project_settings;
        let store = Store::open_in_memory().unwrap();
        let mut a = group("a", None);
        a.settings.permission_mode = Some(PermissionMode::ReadOnly);
        a.settings.environment = dunlin_domain::parse_project_environment("TEAM=a").unwrap();
        let mut b = group("b", None);
        b.settings.harness = Some("Claude Code".into());
        b.settings.permission_mode = Some(PermissionMode::AutoApprove);
        b.settings
            .set_default_model("Claude Code", "team-model")
            .unwrap();
        b.settings.environment = dunlin_domain::parse_project_environment("TEAM=b").unwrap();
        let mut child = group("child", Some("a"));
        for group in [&a, &b, &child] {
            store.upsert_project_group(group).unwrap();
        }
        let mut project = project();
        project.group_id = Some("child".into());
        project.settings.custom_icon = Some("png:keep".into());
        store.upsert_project(&project).unwrap();
        store.upsert_session(&session()).unwrap();
        let resolve = || {
            resolve_project_settings(
                &store.projects().unwrap()[0],
                &store.project_groups().unwrap(),
                "OpenCode",
                PermissionMode::Ask,
            )
            .unwrap()
        };
        assert_eq!(resolve().permission_mode.value, PermissionMode::ReadOnly);
        child.parent_id = Some("b".into());
        store.upsert_project_group(&child).unwrap();
        assert_eq!(resolve().permission_mode.value, PermissionMode::AutoApprove);
        assert_eq!(resolve().default_model("Claude Code"), Some("team-model"));
        assert_eq!(resolve().process_environment().unwrap()["TEAM"], "b");
        store.delete_project_group("b").unwrap();
        assert_eq!(resolve().permission_mode.value, PermissionMode::Ask);
        assert_eq!(resolve().harness.value, "OpenCode");
        assert!(resolve().environment.is_empty());
        let mut promoted = store
            .project_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.id == "child")
            .unwrap();
        promoted.settings.environment =
            dunlin_domain::parse_project_environment("CHILD=later").unwrap();
        store.upsert_project_group(&promoted).unwrap();
        store
            .restore_project_group(&b, None, &[], &["child".into()])
            .unwrap();
        assert_eq!(resolve().harness.value, "Claude Code");
        assert_eq!(resolve().process_environment().unwrap()["TEAM"], "b");
        assert_eq!(resolve().process_environment().unwrap()["CHILD"], "later");
        assert_eq!(
            store
                .project_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == "b")
                .unwrap()
                .settings,
            b.settings
        );
        let stored = &store.projects().unwrap()[0].settings;
        assert!(stored.harness.is_none());
        assert!(stored.environment.is_empty());
        assert!(stored.default_models.is_empty());
        assert_eq!(stored.custom_icon, project.settings.custom_icon);
        assert_eq!(store.sessions(None).unwrap()[0].harness, session().harness);
    }

    #[test]
    fn corrupt_group_settings_are_reported_not_silently_replaced_by_broader_defaults() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project_group(&group("root", None)).unwrap();
        store
            .connection
            .execute("UPDATE project_groups SET settings = 'invalid'", [])
            .unwrap();
        assert!(store.project_groups().is_err());
    }

    #[test]
    fn deletion_promotes_only_direct_members_and_undo_preserves_later_moves() {
        let store = Store::open_in_memory().unwrap();
        for group in [
            group("root", None),
            group("section", Some("root")),
            group("child", Some("section")),
            group("leaf", Some("child")),
            group("other", None),
        ] {
            store.upsert_project_group(&group).unwrap();
        }
        let mut direct = project();
        direct.group_id = Some("section".into());
        store.upsert_project(&direct).unwrap();
        let mut nested = direct.clone();
        nested.id = "nested".into();
        nested.path = "/unused/nested".into();
        nested.group_id = Some("leaf".into());
        store.upsert_project(&nested).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_transcript("ses-1", &vec!["history"]).unwrap();
        store.delete_project_group("section").unwrap();
        assert_eq!(
            store
                .projects()
                .unwrap()
                .iter()
                .find(|p| p.id == direct.id)
                .unwrap()
                .group_id
                .as_deref(),
            Some("root")
        );
        assert_eq!(
            store
                .project_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == "child")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("root")
        );
        assert_eq!(
            store
                .project_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == "leaf")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("child")
        );
        direct.group_id = Some("other".into());
        store.upsert_project(&direct).unwrap();
        store
            .upsert_project_group(&group("child", Some("other")))
            .unwrap();
        store
            .restore_project_group(
                &group("section", Some("root")),
                Some("root"),
                &[direct.id.clone()],
                &["child".into()],
            )
            .unwrap();
        assert_eq!(
            store
                .projects()
                .unwrap()
                .iter()
                .find(|p| p.id == direct.id)
                .unwrap()
                .group_id
                .as_deref(),
            Some("other")
        );
        assert_eq!(
            store
                .projects()
                .unwrap()
                .iter()
                .find(|p| p.id == nested.id)
                .unwrap()
                .group_id
                .as_deref(),
            Some("leaf")
        );
        assert_eq!(
            store
                .project_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == "child")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("other")
        );
        assert_eq!(
            store
                .load_transcript::<Vec<String>>("ses-1")
                .unwrap()
                .unwrap(),
            ["history"]
        );
        assert_eq!(store.sessions(None).unwrap().len(), 1);
    }

    #[test]
    fn hierarchy_updates_and_restore_are_cycle_safe_and_atomic() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project_group(&group("root", None)).unwrap();
        store
            .upsert_project_group(&group("child", Some("root")))
            .unwrap();
        assert!(
            store
                .upsert_project_group(&group("root", Some("child")))
                .is_err()
        );
        assert!(
            store
                .upsert_project_group(&group("root", Some("root")))
                .is_err()
        );
        assert!(
            store
                .upsert_project_group(&group("missing", Some("not-there")))
                .is_err()
        );
        let mut project = project();
        project.group_id = Some("root".into());
        store.upsert_project(&project).unwrap();
        // Restoring under an original child must not reattach that child and
        // form a cycle, nor leave the preceding project update half-written.
        assert!(
            store
                .restore_project_group(
                    &group("restored", Some("child")),
                    Some("root"),
                    &[project.id.clone()],
                    &["child".into()]
                )
                .is_err()
        );
        assert!(
            !store
                .project_groups()
                .unwrap()
                .iter()
                .any(|g| g.id == "restored")
        );
        assert_eq!(
            store.projects().unwrap()[0].group_id.as_deref(),
            Some("root")
        );
        store.delete_project_group("root").unwrap();
        store
            .restore_project_group(&group("root", None), None, &[project.id], &["child".into()])
            .unwrap();
        assert_eq!(
            store.projects().unwrap()[0].group_id.as_deref(),
            Some("root")
        );
        assert_eq!(
            store
                .project_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == "child")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("root")
        );
    }

    #[test]
    fn round_trips_project_overrides() {
        let store = Store::open_in_memory().unwrap();
        let mut project = project();
        project.settings.harness = Some("Claude Code".into());
        project.settings.permission_mode = Some(PermissionMode::ReadOnly);
        project
            .settings
            .set_default_model("Claude Code", "model-id")
            .unwrap();
        project.settings.environment =
            dunlin_domain::parse_project_environment("LANG=pl_PL.UTF-8\nEMPTY=").unwrap();
        store.upsert_project(&project).unwrap();

        let stored = &store.projects().unwrap()[0].settings;
        assert_eq!(stored.harness.as_deref(), Some("Claude Code"));
        assert_eq!(stored.permission_mode, Some(PermissionMode::ReadOnly));
        assert_eq!(stored.default_model("Claude Code"), Some("model-id"));
        assert_eq!(stored.environment, project.settings.environment);
    }

    #[test]
    fn old_project_settings_load_without_resetting_harness_permissions_or_history() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store
            .save_transcript(&session().id, &vec!["preserved"])
            .unwrap();
        for old in [
            None,
            Some("{}"),
            Some(
                r#"{"harness":"Claude Code","permission_mode":"ReadOnly","custom_icon":"png:old"}"#,
            ),
        ] {
            store
                .connection
                .execute("UPDATE projects SET settings = ?1", [old])
                .unwrap();
            let restored = store.projects().unwrap().remove(0);
            assert!(restored.settings.default_models.is_empty());
            assert!(restored.settings.environment.is_empty());
            if old.is_some_and(|s| s.contains("Claude Code")) {
                assert_eq!(restored.settings.harness.as_deref(), Some("Claude Code"));
                assert_eq!(
                    restored.settings.permission_mode,
                    Some(PermissionMode::ReadOnly)
                );
                assert_eq!(restored.settings.custom_icon.as_deref(), Some("png:old"));
            }
            store.upsert_project(&restored).unwrap();
            assert_eq!(store.sessions(None).unwrap().len(), 1);
            assert_eq!(
                store
                    .load_transcript::<Vec<String>>(&session().id)
                    .unwrap()
                    .unwrap(),
                vec!["preserved"]
            );
        }
    }

    #[test]
    fn upserts_keep_one_row() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();

        let mut updated = session();
        updated.status = SessionStatus::Done;
        updated.tokens = 2_048;
        store.upsert_session(&updated).unwrap();

        let sessions = store.sessions(None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Done);
        assert_eq!(sessions[0].tokens, 2_048);
    }

    #[test]
    fn round_trips_transcripts() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();

        let transcript = vec!["hello", "world"];
        store.save_transcript("ses-1", &transcript).unwrap();
        let loaded: Option<Vec<String>> = store.load_transcript("ses-1").unwrap();
        assert_eq!(loaded, Some(vec!["hello".into(), "world".into()]));

        let missing: Option<Vec<String>> = store.load_transcript("missing").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn deleting_a_session_cascades_to_transcripts() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_transcript("ses-1", &vec!["x"]).unwrap();

        store.delete_session("ses-1").unwrap();
        assert!(store.sessions(None).unwrap().is_empty());
        let transcript: Option<Vec<String>> = store.load_transcript("ses-1").unwrap();
        assert_eq!(transcript, None);
    }

    #[test]
    fn clearing_sessions_keeps_projects_and_settings() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_transcript("ses-1", &vec!["x"]).unwrap();
        store.save_setting("app", &"kept").unwrap();

        store.clear_sessions().unwrap();
        assert_eq!(store.projects().unwrap().len(), 1);
        assert!(store.sessions(None).unwrap().is_empty());
        let transcript: Option<Vec<String>> = store.load_transcript("ses-1").unwrap();
        assert_eq!(transcript, None);
        let setting: Option<String> = store.load_setting("app").unwrap();
        assert_eq!(setting.as_deref(), Some("kept"));
    }

    #[test]
    fn deleting_a_project_cascades_to_sessions_and_transcripts() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        store.upsert_session(&session()).unwrap();
        store.save_transcript("ses-1", &vec!["x"]).unwrap();

        store.delete_project("dunlin").unwrap();
        assert!(store.projects().unwrap().is_empty());
        assert!(store.sessions(None).unwrap().is_empty());
        let transcript: Option<Vec<String>> = store.load_transcript("ses-1").unwrap();
        assert_eq!(transcript, None);
    }

    #[test]
    fn round_trips_settings_and_migrates_existing_databases() {
        let store = Store::open_in_memory().unwrap();
        let settings = serde_json::json!({
            "default_harness": "Claude Code",
            "dark": true,
            "auto_connect": false,
        });
        store.save_setting("app", &settings).unwrap();
        let loaded: Option<serde_json::Value> = store.load_setting("app").unwrap();
        assert_eq!(loaded, Some(settings));

        let updated = serde_json::json!({ "default_harness": "OpenCode" });
        store.save_setting("app", &updated).unwrap();
        let loaded: Option<serde_json::Value> = store.load_setting("app").unwrap();
        assert_eq!(loaded, Some(updated));

        let missing: Option<serde_json::Value> = store.load_setting("missing").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn reports_schema_version_and_quick_check_for_release_qa() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 18);
        assert_eq!(store.quick_check().unwrap(), "ok");
    }

    #[test]
    fn vault_metadata_grants_recipes_and_audit_round_trip_without_values() {
        use dunlin_domain::vault::{
            GrantPolicy, HttpMethod, Injection, Recipe, RecipeParameter, Secret, SecretAudit,
            SecretGrant,
        };
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();
        let secret = Secret {
            id: "dunlin/one".into(),
            namespace: "dunlin".into(),
            alias: "github-work".into(),
            label: "GitHub (work)".into(),
            description: "Use for api.github.com".into(),
            injection: Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            },
            allowed_hosts: vec!["api.github.com".into()],
            path_prefix: "/".into(),
            allowed_methods: vec![HttpMethod::Get, HttpMethod::Post],
            allowed_headers: vec!["Accept".into()],
            auto_approve: true,
            expires_at: Some(1_900_000_000),
            created_at: 1,
            updated_at: 2,
            last_used_at: None,
        };
        store.upsert_secret(&secret).unwrap();
        assert_eq!(
            store.secret_by_alias("github-work").unwrap(),
            Some(secret.clone())
        );
        assert_eq!(store.secrets().unwrap().len(), 1);

        store
            .set_secret_grant(&SecretGrant {
                secret_id: secret.id.clone(),
                project_id: "dunlin".into(),
                policy: GrantPolicy::Auto,
                created_at: 3,
            })
            .unwrap();
        assert_eq!(
            store.secrets_for_project("dunlin").unwrap(),
            vec![secret.clone()]
        );
        assert_eq!(
            store
                .secret_grant(&secret.id, "dunlin")
                .unwrap()
                .unwrap()
                .policy,
            GrantPolicy::Auto
        );

        let recipe = Recipe {
            id: "recipe-1".into(),
            secret_id: secret.id.clone(),
            name: "gh-api".into(),
            description: "Call the GitHub API".into(),
            program: "/opt/homebrew/bin/gh".into(),
            args: vec!["api".into(), "{path}".into()],
            env_var: "GH_TOKEN".into(),
            parameters: vec![RecipeParameter {
                name: "path".into(),
                required: true,
                max_len: 200,
                allow_leading_dash: false,
            }],
            timeout_secs: 30,
            program_hash: None,
            created_at: 4,
        };
        store.upsert_recipe(&recipe).unwrap();
        assert_eq!(
            store.recipe_by_name(&secret.id, "gh-api").unwrap(),
            Some(recipe.clone())
        );

        store
            .record_secret_audit(&SecretAudit {
                id: 0,
                secret_id: Some(secret.id.clone()),
                alias: secret.alias.clone(),
                project_id: Some("dunlin".into()),
                session_id: Some("ses-1".into()),
                operation: "http".into(),
                detail: "GET api.github.com/repos".into(),
                decision: "auto".into(),
                status: Some(200),
                error: None,
                created_at: 5,
            })
            .unwrap();
        let audit = store.secret_audit(Some(&secret.id), 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].status, Some(200));
        assert!(!serde_json::to_string(&audit).unwrap().contains("value"));

        store.delete_secret(&secret.id).unwrap();
        assert!(store.secrets().unwrap().is_empty());
        assert!(store.recipes_for_secret(&secret.id).unwrap().is_empty());
        assert!(store.secret_grants(&secret.id).unwrap().is_empty());
        // Audit history is intentionally retained after secret deletion.
        assert_eq!(store.secret_audit(None, 10).unwrap().len(), 1);
    }

    #[test]
    fn ssh_hosts_round_trip_and_delete() {
        let store = Store::open_in_memory().unwrap();
        let host = dunlin_domain::SshHost {
            id: "ssh:host".into(),
            name: "Host".into(),
            target: "host".into(),
            user: "user".into(),
            port: 2222,
            identity: Some("~/.ssh/id_ed25519".into()),
            relay_port: 8787,
            local_port: 0,
            host_key: String::new(),
            created_at: 1,
        };
        store.upsert_ssh_host(&host).unwrap();
        assert_eq!(store.ssh_hosts().unwrap(), vec![host]);
        store.delete_ssh_host("ssh:host").unwrap();
        assert!(store.ssh_hosts().unwrap().is_empty());
    }

    #[test]
    fn knowledge_runs_artifacts_claims_evidence_and_plugins_round_trip() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_project(&project()).unwrap();

        let object = dunlin_knowledge::KnowledgeObject {
            id: "INT-001".into(),
            project_id: "dunlin".into(),
            kind: "intent".into(),
            title: "Intent".into(),
            status: "accepted".into(),
            path: Some("intent/INT-001.md".into()),
            revision: "rev".into(),
            updated_ms: 1,
            fields: std::collections::BTreeMap::new(),
            body: "Why".into(),
            plugin_id: Some("dunlin-governance".into()),
            schema_version: 1,
        };
        let edge = dunlin_knowledge::KnowledgeEdge {
            from_id: "INT-001".into(),
            to_id: "CON-001".into(),
            relation: "has_concept".into(),
            metadata: std::collections::BTreeMap::new(),
        };
        store
            .replace_knowledge(
                "dunlin",
                std::slice::from_ref(&object),
                std::slice::from_ref(&edge),
            )
            .unwrap();
        assert_eq!(store.knowledge_objects("dunlin").unwrap(), vec![object]);
        assert_eq!(store.knowledge_edges("dunlin").unwrap(), vec![edge]);

        let run = RunRecord {
            id: "run-1".into(),
            project_id: "dunlin".into(),
            session_id: Some("ses-1".into()),
            intent_id: Some("INT-001".into()),
            task_id: None,
            handoff_id: None,
            harness: "OpenCode".into(),
            model: "model".into(),
            permission_mode: "ask".into(),
            worktree: None,
            budget_id: None,
            envelope_hash: Some("hash".into()),
            status: "done".into(),
            started_ms: Some(1),
            ended_ms: Some(2),
            result: "ok".into(),
            payload: serde_json::json!({"k": "v"}),
        };
        store.upsert_run(&run).unwrap();
        assert_eq!(store.runs("dunlin").unwrap(), vec![run]);

        let artifact = ArtifactRecord {
            id: "art-1".into(),
            project_id: "dunlin".into(),
            run_id: Some("run-1".into()),
            session_id: Some("ses-1".into()),
            intent_id: Some("INT-001".into()),
            task_id: None,
            kind: "diff".into(),
            title: "Diff".into(),
            mime: "text/plain".into(),
            bytes: 1,
            sha256: "hash".into(),
            blob_path: "blob".into(),
            created_ms: 1,
            visibility: "local".into(),
            expires_ms: None,
            revoked_ms: None,
            payload: serde_json::json!({}),
        };
        store.upsert_artifact(&artifact).unwrap();
        assert_eq!(store.artifacts("dunlin").unwrap(), vec![artifact]);

        let claim = ClaimRecord {
            id: "claim-1".into(),
            project_id: "dunlin".into(),
            intent_id: Some("INT-001".into()),
            task_id: None,
            statement: "It works".into(),
            status: "unverified".into(),
            confidence: "low".into(),
            scope: "commit".into(),
            last_verified_ms: None,
            stale_reason: None,
            payload: serde_json::json!({}),
        };
        store.upsert_claim(&claim).unwrap();
        assert_eq!(store.claims("dunlin").unwrap(), vec![claim]);

        let evidence = EvidenceRecord {
            id: "ev-1".into(),
            project_id: "dunlin".into(),
            kind: "check".into(),
            artifact_id: Some("art-1".into()),
            run_id: Some("run-1".into()),
            source: "check".into(),
            revision: "rev".into(),
            hash: "hash".into(),
            observed_ms: 1,
            freshness: "fresh".into(),
            summary: "exit 0".into(),
            payload: serde_json::json!({}),
        };
        store.upsert_evidence(&evidence).unwrap();
        assert_eq!(store.evidence("dunlin").unwrap(), vec![evidence]);
        store.link_evidence("ev-1", "claim-1", "supports").unwrap();
        assert_eq!(
            store.evidence_links("claim-1").unwrap(),
            vec![("ev-1".into(), "supports".into())]
        );

        let plugin = PluginRecord {
            id: "dunlin-governance".into(),
            version: "1.0.0".into(),
            manifest_hash: "manifest".into(),
            wasm_hash: Some("wasm".into()),
            directory: "/tmp/governance".into(),
            enabled: false,
            installed_ms: 1,
            payload: serde_json::json!({}),
        };
        store.upsert_plugin(&plugin).unwrap();
        assert_eq!(store.plugins().unwrap(), vec![plugin]);
        store.set_plugin_enabled("dunlin-governance", true).unwrap();
        assert!(store.plugins().unwrap()[0].enabled);
    }
}
