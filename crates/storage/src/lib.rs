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
    /// Open (or create) a database, enabling WAL and foreign keys.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
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
        self.connection.execute(
            r#"
            INSERT INTO projects (id, name, path, branch, tone, pinned, trusted, remote, group_id, settings)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                path = excluded.path,
                branch = excluded.branch,
                tone = excluded.tone,
                pinned = excluded.pinned,
                trusted = excluded.trusted,
                remote = excluded.remote,
                group_id = excluded.group_id,
                settings = excluded.settings
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
            ],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, path, branch, tone, pinned, trusted, remote, group_id, settings FROM projects ORDER BY created_at",
        )?;
        let rows = statement.query_map([], |row| {
            let tone: String = row.get(4)?;
            let remote: Option<String> = row.get(7)?;
            let settings: Option<String> = row.get(9)?;
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                branch: row.get(3)?,
                tone: serde_json::from_str(&tone).unwrap_or(dunlin_domain::ProjectTone::Blue),
                pinned: row.get::<_, i64>(5)? != 0,
                trusted: row.get::<_, i64>(6)? != 0,
                running_sessions: 0,
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
                updated_at, cost_usd, duration_secs, tokens, summary, origin, acp_session_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
                acp_session_id = excluded.acp_session_id
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
            ],
        )?;
        Ok(())
    }

    /// Insert-only, atomic history import. Existing conversations/transcripts
    /// are never updated, even when a preview is stale or imported twice.
    pub fn import_sessions<T: Serialize>(
        &self,
        rows: &[(SessionSummary, T)],
    ) -> Result<Vec<String>> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut inserted = Vec::new();
        for (session, transcript) in rows {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
                [&session.id],
                |r| r.get(0),
            )?;
            if exists {
                continue;
            }
            self.upsert_session(session)?;
            self.save_transcript(&session.id, transcript)?;
            inserted.push(session.id.clone());
        }
        transaction.commit()?;
        Ok(inserted)
    }

    pub fn sessions(&self, project_id: Option<&str>) -> Result<Vec<SessionSummary>> {
        let sql = match project_id {
            Some(_) => {
                "SELECT id, project_id, title, status, harness, model, branch, updated_at, \
                 cost_usd, duration_secs, tokens, summary, origin, acp_session_id FROM sessions \
                 WHERE project_id = ?1 ORDER BY created_at DESC"
            }
            None => {
                "SELECT id, project_id, title, status, harness, model, branch, updated_at, \
                 cost_usd, duration_secs, tokens, summary, origin, acp_session_id FROM sessions \
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
        self.connection
            .execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::{PermissionMode, ProjectTone, RemoteProject, SessionOrigin, SessionStatus};

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
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    fn session() -> SessionSummary {
        SessionSummary {
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
            9
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
        assert_eq!(store.schema_version().unwrap(), 9);
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

}
