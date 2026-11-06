//! Explicit local-only reset. Never remove a data directory or repository tree.
use crate::app::DunlinApp;
use dunlin_storage::Store;
use dunlin_ui::*;
use dunlin_vault::store::{KeychainStore, SecretStore};
use std::path::Path;

pub(crate) const TITLE: &str = "Warning: reset Dunlin to factory defaults?";
// shortcut: other instances must be closed; add a shared data-directory lock for multi-instance reset.
pub(crate) const WARNING: &str = "This permanently deletes Dunlin's settings, added projects and groups, conversation history, harness configuration, relay pairings and credentials saved in Dunlin's vault. This cannot be undone.\n\nRepository files, installed agents, external account logins, extensions on disk and diagnostic logs are not deleted. Dunlin will close; open it again to start with default settings.\n\nClose every other Dunlin instance before continuing, or it may save old data again.\n\nDo you want to continue?";
pub(crate) const ANSWERS: &[&str] = &["No", "Yes, reset Dunlin"];

fn confirmed(answer: Option<usize>) -> bool {
    answer == Some(1)
}

fn reset_data(
    store: &Store,
    credentials: &dyn SecretStore,
    hosts: &[String],
    mut delete_token: impl FnMut(&str) -> Result<(), String>,
    window_preferences: &Path,
) -> Result<(), String> {
    // Keep metadata until all owned Keychain entries have been removed successfully.
    for secret in store.secrets().map_err(|e| e.to_string())? {
        credentials.delete(&secret.id)?;
        credentials.delete_policy(&secret.id)?;
    }
    for host in hosts {
        delete_token(host)?;
    }
    match std::fs::remove_file(window_preferences) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not reset window preferences: {error}")),
    }
    store.factory_reset().map_err(|e| e.to_string())
}

impl DunlinApp {
    fn reset_blocked(&self) -> bool {
        self.has_dirty_tabs()
            || self.explorer.busy
            || self.has_running_agents()
            || self.harness.status.is_busy()
            || self.harness.local_session_loading
            || self.branch_ui.running()
            || self.git_history.running
            || self.local_import_running
            || self.project_import.busy
            || self.backup.busy
            || self.backup.cloning
            || self.session_import.loading
            || self.remote_loading
            || self.vault.ui.busy
            || self.security.check_running
            || self.skills_ui.busy
            || !self.comparisons.active.is_empty()
            || !self.ai_commit.jobs.is_empty()
            || self
                .terminal
                .tabs
                .iter()
                .any(|tab| tab.status == crate::terminal::Status::Running)
            || self
                .projects
                .iter()
                .any(|project| self.git_mutation_in_flight(&project.id))
    }

    pub(crate) fn request_factory_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.factory_resetting {
            return;
        }
        if self.reset_blocked() {
            self.file_notice("Save edited files, close running terminals and wait for file, Git and agent operations before resetting Dunlin", cx);
            return;
        }
        self.factory_resetting = true;
        let answer = window.prompt(PromptLevel::Warning, TITLE, Some(WARNING), ANSWERS, cx);
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let yes = confirmed(answer.await.ok());
            let _ = this.update(cx, |this, cx| {
                this.factory_resetting = false;
                if yes {
                    this.begin_factory_reset(cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn begin_factory_reset(&mut self, cx: &mut Context<Self>) {
        if self.reset_blocked() {
            self.file_notice(
                "Reset cancelled: save edited files and wait for pending operations",
                cx,
            );
            return;
        }
        let Some(path) = self.store.as_ref().and_then(Store::database_path) else {
            self.file_notice("Reset requires an available local database", cx);
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        let preferences = parent.join("window.json");
        let hosts = self
            .remote_hosts
            .iter()
            .map(|host| host.id.clone())
            .collect::<Vec<_>>();
        self.disconnect_harness(cx);
        self.save_and_shutdown_background_agents();
        self.parked_runtimes.clear();
        self.vault.enabled = false;
        let broker = self.vault.service.take();
        let store = self.store.take().unwrap();
        self.factory_resetting = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (store, result) = cx.background_executor().spawn(async move {
                let result = broker.map_or(Ok(()), |broker| broker.shutdown_and_wait())
                    .and_then(|()| reset_data(&store, &KeychainStore, &hosts, crate::remote::delete_token_checked, &preferences));
                (store, result)
            }).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    // Leaving store detached also prevents Drop from saving old history.
                    drop(store);
                    cx.quit();
                }
                Err(error) => {
                    this.store = Some(store);
                    this.factory_resetting = false;
                    this.file_notice(format!("Reset incomplete: {error}. Some credentials may already be removed. Local metadata was retained for retry."), cx);
                }
            });
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_credential(store: &Store, credentials: &dyn SecretStore) {
        let secret = serde_json::from_value(serde_json::json!({
            "id": "reset-test", "namespace": "external", "alias": "test",
            "label": "Reset test", "injection": {"type": "basic", "username": "test"}
        }))
        .unwrap();
        store.upsert_secret(&secret).unwrap();
        credentials.set("reset-test", "test-only-value").unwrap();
        credentials
            .set_policy("reset-test", "test-only-policy")
            .unwrap();
    }
    #[test]
    fn only_an_explicit_yes_can_authorize_reset() {
        assert_eq!(ANSWERS[0], "No");
        assert!(TITLE.starts_with("Warning:"));
        assert!(WARNING.contains("cannot be undone"));
        assert!(WARNING.contains("Repository files"));
        assert!(!confirmed(None));
        assert!(!confirmed(Some(0)));
        assert!(!confirmed(Some(2)));
        assert!(confirmed(Some(1)));
    }
    #[test]
    fn reset_only_removes_app_records_and_window_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repository");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join("keep.txt"), "user content").unwrap();
        let preferences = dir.path().join("window.json");
        std::fs::write(&preferences, "{}").unwrap();
        let store = Store::open(dir.path().join("dunlin.db")).unwrap();
        store
            .save_setting("app", &serde_json::json!({"custom_harness_command":"old"}))
            .unwrap();
        let credentials = dunlin_vault::store::MemoryStore::default();
        add_credential(&store, &credentials);
        let mut deleted = Vec::new();
        reset_data(
            &store,
            &credentials,
            &["paired-host".into()],
            |id| {
                deleted.push(id.to_string());
                Ok(())
            },
            &preferences,
        )
        .unwrap();
        assert_eq!(deleted, ["paired-host"]);
        assert!(
            store
                .load_setting::<serde_json::Value>("app")
                .unwrap()
                .is_none()
        );
        assert!(store.projects().unwrap().is_empty());
        assert!(store.project_groups().unwrap().is_empty());
        assert!(store.secrets().unwrap().is_empty());
        assert!(credentials.get("reset-test").is_err());
        assert!(credentials.get_policy("reset-test").is_err());
        assert!(!preferences.exists());
        assert_eq!(
            std::fs::read_to_string(repo.join("keep.txt")).unwrap(),
            "user content"
        );
        // Retrying after a completed reset is harmless, including a missing window.json.
        reset_data(&store, &credentials, &[], |_| Ok(()), &preferences).unwrap();
    }
    #[test]
    fn keychain_failure_keeps_local_settings_and_pairing_metadata_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        store.save_setting("app", &"keep").unwrap();
        let preferences = dir.path().join("window.json");
        std::fs::write(&preferences, "{}").unwrap();
        let credentials = dunlin_vault::store::MemoryStore::default();
        add_credential(&store, &credentials);
        assert!(
            reset_data(
                &store,
                &credentials,
                &["host".into()],
                |_| Err("Keychain locked".into()),
                &preferences
            )
            .is_err()
        );
        assert_eq!(
            store.load_setting::<String>("app").unwrap().as_deref(),
            Some("keep")
        );
        assert!(preferences.exists());
        assert_eq!(store.secrets().unwrap().len(), 1);
        // Keychain and SQLite cannot share a transaction; a retry tolerates absent values.
        assert!(credentials.get("reset-test").is_err());
        reset_data(
            &store,
            &credentials,
            &["host".into()],
            |_| Ok(()),
            &preferences,
        )
        .unwrap();
        assert!(store.secrets().unwrap().is_empty());
    }

    #[test]
    fn window_preferences_failure_does_not_clear_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        store.save_setting("app", &"keep").unwrap();
        let credentials = dunlin_vault::store::MemoryStore::default();
        assert!(reset_data(&store, &credentials, &[], |_| Ok(()), dir.path()).is_err());
        assert_eq!(
            store.load_setting::<String>("app").unwrap().as_deref(),
            Some("keep")
        );
    }
}
