//! Separate hashed browser credentials. Native bearer authentication never reads
//! this table. Absolute expiry and device revocation survive a relay restart.
use rusqlite::{OptionalExtension, params};

use super::{Device, Result, Store, StoreError, auth, now_secs};

pub(super) const MIGRATION: &str = "
CREATE TABLE web_browser_sessions (
    token_hash TEXT PRIMARY KEY,
    device_id TEXT NOT NULL UNIQUE REFERENCES devices(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE INDEX web_browser_session_expiry ON web_browser_sessions(expires_at);
PRAGMA user_version=12;
";

impl Store {
    pub fn create_browser_session(&self, device: &str, lifetime_seconds: u64) -> Result<String> {
        let transaction = self.connection.unchecked_transaction()?;
        let token = self.create_browser_session_in_transaction(device, lifetime_seconds)?;
        transaction.commit()?;
        Ok(token)
    }

    pub(super) fn create_browser_session_in_transaction(
        &self,
        device: &str,
        lifetime_seconds: u64,
    ) -> Result<String> {
        if lifetime_seconds == 0 || lifetime_seconds > 8 * 60 * 60 {
            return Err(StoreError::Limit(
                "Browser session lifetime must be 1 second to 8 hours".into(),
            ));
        }
        let now = now_secs();
        self.connection.execute("DELETE FROM web_browser_sessions WHERE expires_at<=?1 OR device_id IN (SELECT id FROM devices WHERE revoked_at IS NOT NULL)", [now])?;
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM web_browser_sessions", [], |row| {
                    row.get(0)
                })?;
        if count >= 1024 {
            return Err(StoreError::Limit("Browser session limit reached".into()));
        }
        let token = auth::random_token();
        let expires = now
            .checked_add(lifetime_seconds as i64)
            .ok_or_else(|| StoreError::Limit("Invalid server clock".into()))?;
        let inserted = self.connection.execute("INSERT INTO web_browser_sessions(token_hash,device_id,expires_at) SELECT ?1,id,?2 FROM devices WHERE id=?3 AND revoked_at IS NULL", params![auth::hash_token(&token),expires,device])?;
        if inserted != 1 {
            return Err(dunlin_domain::collaboration::PolicyError::PermissionDenied.into());
        }
        Ok(token)
    }

    pub fn browser_session_device(&self, token: &str) -> Result<Option<Device>> {
        self.browser_session_device_hash(&auth::hash_token(token))
    }

    /// Internal proof captured at OAuth start, never accepted from a wire request.
    pub(crate) fn browser_session_device_hash(&self, token_hash: &str) -> Result<Option<Device>> {
        Ok(self.connection.query_row(
            "SELECT d.id,d.user_id,d.name,d.created_at,d.last_seen_at,d.revoked_at FROM web_browser_sessions b JOIN devices d ON d.id=b.device_id WHERE b.token_hash=?1 AND b.expires_at>?2 AND d.revoked_at IS NULL",
            params![token_hash,now_secs()], |row| Ok(Device {
                id: row.get(0)?, user_id: row.get(1)?, name: row.get(2)?, created_at: row.get(3)?, last_seen_at: row.get(4)?, revoked_at: row.get(5)?,
            }),
        ).optional()?)
    }

    pub fn remove_browser_session(&self, token: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM web_browser_sessions WHERE token_hash=?1",
            [auth::hash_token(token)],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn expire_browser_session(&self, token: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE web_browser_sessions SET expires_at=0 WHERE token_hash=?1",
            [auth::hash_token(token)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_credentials_survive_restart_without_becoming_native_bearer_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("member", "member").unwrap();
        let (device, native_token) = store.create_device(&user.id, "browser").unwrap();
        let browser_token = store.create_browser_session(&device.id, 8 * 3600).unwrap();
        assert!(store.authenticate(&browser_token).unwrap().is_none());
        assert!(
            store
                .browser_session_device(&native_token)
                .unwrap()
                .is_none()
        );
        let stored: String = store
            .connection
            .query_row("SELECT token_hash FROM web_browser_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_ne!(stored, browser_token);
        assert_eq!(stored, auth::hash_token(&browser_token));
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .browser_session_device(&browser_token)
                .unwrap()
                .unwrap()
                .id,
            device.id
        );
        store.revoke_device(&device.id).unwrap();
        assert!(
            store
                .browser_session_device(&browser_token)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn expiration_and_logout_are_authoritative_and_do_not_remove_other_browsers() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("member", "member").unwrap();
        let first = store.create_device(&user.id, "first").unwrap().0;
        let second = store.create_device(&user.id, "second").unwrap().0;
        assert!(store.create_browser_session(&first.id, 0).is_err());
        assert!(
            store
                .create_browser_session(&first.id, 8 * 3600 + 1)
                .is_err()
        );
        let one = store.create_browser_session(&first.id, 3600).unwrap();
        let two = store.create_browser_session(&second.id, 3600).unwrap();
        store.expire_browser_session(&one).unwrap();
        assert!(store.browser_session_device(&one).unwrap().is_none());
        assert!(store.browser_session_device(&two).unwrap().is_some());
        store.remove_browser_session(&two).unwrap();
        assert!(store.browser_session_device(&two).unwrap().is_none());
    }

    #[test]
    fn browser_registry_is_bounded_and_expired_rows_release_capacity() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("member", "member").unwrap();
        let mut first = None;
        for index in 0..1024 {
            let device = store
                .create_device(&user.id, &format!("browser-{index}"))
                .unwrap()
                .0;
            let token = store.create_browser_session(&device.id, 3600).unwrap();
            if first.is_none() {
                first = Some(token);
            }
        }
        let next = store.create_device(&user.id, "next").unwrap().0;
        assert!(matches!(
            store.create_browser_session(&next.id, 3600),
            Err(StoreError::Limit(_))
        ));
        store
            .expire_browser_session(first.as_deref().unwrap())
            .unwrap();
        assert!(store.create_browser_session(&next.id, 3600).is_ok());
        let count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM web_browser_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1024);
    }
}
