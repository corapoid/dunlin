//! Durable OIDC provenance; configuration revocation applies to existing cookies.
use super::{Result, Store};
use rusqlite::{OptionalExtension, params};
pub(super) const MIGRATION: &str = "
CREATE TABLE web_oidc_sessions (
 device_id TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
 subject TEXT NOT NULL,
 config_digest TEXT NOT NULL
);
PRAGMA user_version=14;
";
impl Store {
    pub fn create_oidc_browser_session(
        &self,
        user: &str,
        subject: &str,
        digest: &str,
        lifetime: u64,
    ) -> Result<String> {
        let transaction = self.connection.unchecked_transaction()?;
        let (device, _) = self.create_device(user, "OIDC browser")?;
        self.mark_oidc_device(&device.id, subject, digest)?;
        let token = self.create_browser_session_in_transaction(&device.id, lifetime)?;
        transaction.commit()?;
        Ok(token)
    }

    fn mark_oidc_device(&self, device: &str, subject: &str, digest: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO web_oidc_sessions VALUES(?1,?2,?3)",
            params![device, subject, digest],
        )?;
        Ok(())
    }
    pub fn oidc_device_allowed(
        &self,
        device: &str,
        config: Option<&crate::oidc::Config>,
    ) -> Result<bool> {
        let binding=self.connection.query_row("SELECT o.subject,o.config_digest,d.user_id FROM web_oidc_sessions o JOIN devices d ON d.id=o.device_id WHERE o.device_id=?1",[device],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        Ok(match binding {
            None => true,
            Some((subject, digest, user)) => config
                .is_some_and(|c| c.digest() == digest && c.bindings.get(&subject) == Some(&user)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provenance_survives_restart_and_changed_bindings_revoke_only_oidc_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oidc.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("owner", "admin").unwrap();
        let config = crate::oidc::Config {
            issuer: "https://issuer.example".into(),
            client_id: "client".into(),
            redirect_uri: "http://127.0.0.1:8787/v1/web/oidc/callback".into(),
            client_secret_env: None,
            allowed_origins: vec![],
            bindings: std::collections::BTreeMap::from([("subject".into(), user.id.clone())]),
        };
        let token = store
            .create_oidc_browser_session(&user.id, "subject", &config.digest(), 600)
            .unwrap();
        let device = store.browser_session_device(&token).unwrap().unwrap();
        let (native, _) = store.create_device(&user.id, "native").unwrap();
        assert!(
            store
                .oidc_device_allowed(&device.id, Some(&config))
                .unwrap()
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(
            store
                .oidc_device_allowed(&device.id, Some(&config))
                .unwrap()
        );
        assert!(!store.oidc_device_allowed(&device.id, None).unwrap());
        let mut changed = config.clone();
        changed.bindings.clear();
        assert!(
            !store
                .oidc_device_allowed(&device.id, Some(&changed))
                .unwrap()
        );
        assert!(store.oidc_device_allowed(&native.id, None).unwrap());
        let before = store.list_devices(&user.id).unwrap().len();
        assert!(
            store
                .create_oidc_browser_session(&user.id, "subject", &config.digest(), 0)
                .is_err()
        );
        assert_eq!(
            store.list_devices(&user.id).unwrap().len(),
            before,
            "failed session creation must roll back device and provenance"
        );
    }
}
