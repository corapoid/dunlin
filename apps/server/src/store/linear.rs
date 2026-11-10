//! Per-user Linear metadata and bounded cached issues, never credential values.
use super::*;
use dunlin_mcp_client::linear::{Filter, Inbox};

pub(crate) struct LinearConnection {
    pub credential_id: String,
    pub revision: String,
    pub filter: Filter,
    pub inbox: Inbox,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_cache_survives_reopen_and_old_revisions_cannot_overwrite_new_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("member", "member").unwrap();
        let inbox = Inbox {
            filter: Some(Filter {
                label: "dunlin".into(),
                team: None,
            }),
            ..Inbox::default()
        };
        store
            .linear_save(&user.id, "linear-first", "first", &inbox)
            .unwrap();
        store
            .linear_save(&user.id, "linear-second", "second", &inbox)
            .unwrap();
        assert!(
            !store
                .linear_update(&user.id, "first", &Inbox::default())
                .unwrap()
        );
        assert_eq!(store.linear_cleanup().unwrap(), vec!["linear-first"]);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .linear_connection(&user.id)
                .unwrap()
                .unwrap()
                .filter
                .label,
            "dunlin"
        );
        assert!(store.linear_due().unwrap().is_empty());
        store
            .connection
            .execute("UPDATE linear_integrations SET attempted_at=0", [])
            .unwrap();
        assert_eq!(store.linear_due().unwrap(), vec![user.id.clone()]);
        store.linear_remove(&user.id).unwrap();
        assert!(store.linear_connection(&user.id).unwrap().is_none());
        assert_eq!(store.linear_cleanup().unwrap().len(), 2);
    }
}

const MAX_INBOX: usize = 8 * 1024 * 1024;

impl Store {
    pub(crate) fn linear_revision(&self, user: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT revision FROM linear_integrations WHERE user_id=?1",
                [user],
                |row| row.get(0),
            )
            .optional()?)
    }
    pub(crate) fn linear_connection(&self, user: &str) -> Result<Option<LinearConnection>> {
        let row = self.connection.query_row(
            "SELECT credential_id, revision, filter, inbox FROM linear_integrations WHERE user_id = ?1",
            [user], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?)),
        ).optional()?;
        row.map(|(credential_id, revision, filter, inbox)| {
            if inbox.len() > MAX_INBOX || filter.len() > 2048 {
                return Err(StoreError::Limit("Invalid Linear metadata".into()));
            }
            let connection = LinearConnection {
                credential_id,
                revision,
                filter: serde_json::from_str(&filter)
                    .map_err(|_| StoreError::Limit("Invalid Linear filter".into()))?,
                inbox: serde_json::from_str(&inbox)
                    .map_err(|_| StoreError::Limit("Invalid Linear cache".into()))?,
            };
            connection
                .inbox
                .validate()
                .map_err(|_| StoreError::Limit("Invalid Linear cache".into()))?;
            if connection.inbox.filter.as_ref() != Some(&connection.filter) {
                return Err(StoreError::Limit("Mismatched Linear filter".into()));
            }
            Ok(connection)
        })
        .transpose()
    }

    pub(crate) fn linear_save(
        &self,
        user: &str,
        credential: &str,
        revision: &str,
        inbox: &Inbox,
    ) -> Result<()> {
        inbox
            .validate()
            .map_err(|_| StoreError::Limit("Invalid Linear inbox".into()))?;
        let filter = inbox
            .filter
            .as_ref()
            .ok_or_else(|| StoreError::Limit("Missing Linear filter".into()))?;
        let filter = serde_json::to_string(filter)
            .map_err(|_| StoreError::Limit("Invalid Linear filter".into()))?;
        let payload = serde_json::to_string(inbox)
            .map_err(|_| StoreError::Limit("Invalid Linear cache".into()))?;
        if payload.len() > MAX_INBOX {
            return Err(StoreError::Limit(
                "Linear inbox exceeds 8 MiB; narrow the team/label filter".into(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM linear_integrations", [], |row| {
                row.get(0)
            })?;
        let previous: Option<String> = transaction
            .query_row(
                "SELECT credential_id FROM linear_integrations WHERE user_id = ?1",
                [user],
                |row| row.get(0),
            )
            .optional()?;
        if count >= 128 && previous.is_none() {
            return Err(StoreError::Limit(
                "Too many Linear integrations on this relay".into(),
            ));
        }
        if let Some(previous) = previous.filter(|previous| previous != credential) {
            transaction.execute(
                "INSERT OR IGNORE INTO linear_credential_cleanup VALUES (?1)",
                [&previous],
            )?;
        }
        transaction.execute(
            "INSERT INTO linear_integrations(user_id,credential_id,revision,filter,inbox,attempted_at) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(user_id) DO UPDATE SET credential_id=excluded.credential_id,revision=excluded.revision,filter=excluded.filter,inbox=excluded.inbox,attempted_at=excluded.attempted_at",
            params![user,credential,revision,filter,payload,now_secs()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn linear_update(&self, user: &str, revision: &str, inbox: &Inbox) -> Result<bool> {
        inbox
            .validate()
            .map_err(|_| StoreError::Limit("Invalid Linear inbox".into()))?;
        let payload = serde_json::to_string(inbox)
            .map_err(|_| StoreError::Limit("Invalid Linear cache".into()))?;
        if payload.len() > MAX_INBOX {
            return Err(StoreError::Limit(
                "Linear inbox exceeds 8 MiB; narrow the team/label filter".into(),
            ));
        }
        Ok(self.connection.execute("UPDATE linear_integrations SET inbox=?1,attempted_at=?2 WHERE user_id=?3 AND revision=?4", params![payload,now_secs(),user,revision])? == 1)
    }

    pub(crate) fn linear_remove(&self, user: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("INSERT OR IGNORE INTO linear_credential_cleanup SELECT credential_id FROM linear_integrations WHERE user_id=?1", [user])?;
        transaction.execute("DELETE FROM linear_integrations WHERE user_id=?1", [user])?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn linear_due(&self) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare("SELECT user_id FROM linear_integrations WHERE attempted_at <= ?1 ORDER BY attempted_at LIMIT 128")?;
        Ok(statement
            .query_map([now_secs() - 60], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn linear_cleanup(&self) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT credential_id FROM linear_credential_cleanup LIMIT 128")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn linear_cleaned(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM linear_credential_cleanup WHERE credential_id=?1",
            [id],
        )?;
        Ok(())
    }

    pub(crate) fn linear_retire(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO linear_credential_cleanup VALUES (?1)",
            [id],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn linear_make_due(&self) {
        self.connection
            .execute("UPDATE linear_integrations SET attempted_at=0", [])
            .unwrap();
    }
}
