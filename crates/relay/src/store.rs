//! Relay metadata store.
//!
//! Only accounts, devices, grants and a session index live here. Prompt text,
//! tool output and transcripts are deliberately not stored by the relay.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::auth;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: String,
}

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
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
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

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
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
        Ok(())
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
        let project = Project {
            id: format!("prj-{}", uuid::Uuid::now_v7()),
            name: name.trim().to_string(),
            path: path.trim_end_matches('/').to_string(),
        };
        self.connection.execute(
            "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![project.id, project.name, project.path, now_secs()],
        )?;
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

    pub fn grant(&self, project_id: &str, user_id: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO project_grants (project_id, user_id) VALUES (?1, ?2)",
            params![project_id, user_id],
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
                WHERE p.id = ?1 AND g.user_id = ?2
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
            WHERE g.user_id = ?1
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
        let expires_at = now_secs() + ttl_secs.max(60);
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
        self.connection.execute(
            "UPDATE pairing_codes SET used_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
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

    pub fn revoke_device(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE devices SET revoked_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
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
        self.connection.execute(
            r#"
            INSERT OR IGNORE INTO sessions
                (id, user_id, project_id, title, created_at, updated_at, last_device_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)
            "#,
            params![id, user_id, project_id, title, now_secs(), device_id],
        )?;
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
        Ok(())
    }

    pub fn touch_session(&self, id: &str, device_id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET updated_at = ?1, last_device_id = ?2 WHERE id = ?3",
            params![now_secs(), device_id, id],
        )?;
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
        Ok(())
    }

    pub fn save_transcript(&self, session_id: &str, payload: &str) -> Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO transcripts (session_id, payload, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(session_id) DO UPDATE SET
                payload = excluded.payload,
                updated_at = excluded.updated_at
            "#,
            params![session_id, payload, now_secs()],
        )?;
        Ok(())
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
        let project = store.create_project("kestrel", "/tmp/kestrel").unwrap();
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
    fn stores_and_cascades_transcripts() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("marek", "member").unwrap();
        let project = store.create_project("kestrel", "/tmp/kestrel").unwrap();
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
}
