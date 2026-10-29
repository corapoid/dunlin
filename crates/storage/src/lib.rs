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
            self.connection.execute_batch(
                r#"
                ALTER TABLE projects ADD COLUMN remote TEXT;
                ALTER TABLE sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'local';
                ALTER TABLE sessions ADD COLUMN acp_session_id TEXT;

                PRAGMA user_version = 3;
                "#,
            )?;
        }
        if version < 4 {
            self.connection.execute_batch(
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
        }
        if version < 5 {
            self.connection.execute_batch(
                r#"
                ALTER TABLE projects ADD COLUMN settings TEXT;

                PRAGMA user_version = 5;
                "#,
            )?;
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
        let mut statement = self
            .connection
            .prepare("SELECT id, name, tone FROM project_groups ORDER BY created_at, id")?;
        let rows = statement.query_map([], |row| {
            let tone: String = row.get(2)?;
            Ok(ProjectGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                tone: serde_json::from_str(&tone).unwrap_or(dunlin_domain::ProjectTone::Blue),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_project_group(&self, group: &ProjectGroup) -> Result<()> {
        let tone = serde_json::to_string(&group.tone)?;
        self.connection.execute(
            r#"
            INSERT INTO project_groups (id, name, tone)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                tone = excluded.tone
            "#,
            params![group.id, group.name, tone],
        )?;
        Ok(())
    }

    pub fn delete_project_group(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM project_groups WHERE id = ?1", [id])?;
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

    #[test]
    fn round_trips_project_overrides() {
        let store = Store::open_in_memory().unwrap();
        let mut project = project();
        project.settings.harness = Some("Claude Code".into());
        project.settings.permission_mode = Some(PermissionMode::ReadOnly);
        store.upsert_project(&project).unwrap();

        let stored = &store.projects().unwrap()[0].settings;
        assert_eq!(stored.harness.as_deref(), Some("Claude Code"));
        assert_eq!(stored.permission_mode, Some(PermissionMode::ReadOnly));
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
}
