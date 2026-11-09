//! Selection requires a successful ACP session, not merely an executable on PATH.
//! Checks are in-memory and scoped to the project, command, environment and model.
use super::*;
use std::hash::{Hash, Hasher};

pub(super) fn config_key(name: &str, config: &HarnessConfig) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hash);
    config.description().hash(&mut hash);
    config.cwd.hash(&mut hash);
    config.additional_directories.hash(&mut hash);
    config.default_model.hash(&mut hash);
    let environment: std::collections::BTreeMap<_, _> = config.process_env.iter().collect();
    environment.hash(&mut hash);
    hash.finish()
}

impl HarnessState {
    pub(super) fn clear_connection_operations(&mut self) {
        self.local_session_loading = false;
        self.session_metadata_pending = false;
        self.model_changing = false;
        self.mode_changing = false;
    }
}

fn unavailable_reason(status: Option<&HarnessStatus>) -> Option<String> {
    match status {
        Some(HarnessStatus::Ready { .. }) => None,
        Some(HarnessStatus::Starting) => Some("Connection in progress…".into()),
        Some(HarnessStatus::Failed(message) | HarnessStatus::Exited(message)) => Some(format!(
            "Connection failed: {message}. Retry in Settings → Agents."
        )),
        _ => Some("Test the connection in Settings → Agents first.".into()),
    }
}

impl DunlinApp {
    pub(super) fn local_harness_config(&self, name: &str) -> Result<HarnessConfig, String> {
        let project = self.project();
        if project.is_remote() || !project.trusted {
            return Err("A trusted local project is required to connect a local harness.".into());
        }
        let cwd = PathBuf::from(&project.path);
        if !cwd.is_dir() {
            return Err(format!("Folder is not available: {}", project.path));
        }
        let command = std::env::var("DUNLIN_HARNESS_COMMAND")
            .ok()
            .filter(|command| !command.trim().is_empty())
            .unwrap_or_else(|| harness_command(name, &self.custom_harness_command));
        let command = crate::harness_discovery::resolved_command(&command)
            .ok_or_else(|| format!("Install or configure the ACP adapter first: {command}"))?;
        let settings = self.resolved_project_settings(project)?;
        let mut config = HarnessConfig::new(command, cwd)
            .with_additional_directories(self.session_additional_directories());
        if let Some(path) = crate::harness_discovery::child_path() {
            config.process_env.insert("PATH".into(), path);
        }
        config.process_env.extend(settings.process_environment()?);
        for key in [
            "DUNLIN_REMOTE_TOKEN",
            "DUNLIN_AUTOPROMPT",
            "DUNLIN_HARNESS_COMMAND",
            "DUNLIN_DATA_DIR",
            "DUNLIN_LOG_FILE",
        ] {
            config.process_env.insert(key.into(), String::new());
        }
        config.default_model = settings.default_model(name).map(str::to_owned);
        Ok(config)
    }

    pub(crate) fn harness_check_status(&self, name: &str) -> Option<&HarnessStatus> {
        let config = self.local_harness_config(name).ok()?;
        self.harness_checks.get(&config_key(name, &config))
    }

    pub(crate) fn harness_unavailable_reason(&self, name: &str) -> Option<String> {
        let config = match self.local_harness_config(name) {
            Ok(config) => config,
            Err(error) => return Some(error),
        };
        unavailable_reason(self.harness_checks.get(&config_key(name, &config)))
    }

    pub(super) fn record_harness_connection(&mut self, event: &HarnessEvent) {
        let Some(key) = self.harness.selection_key else {
            return;
        };
        let status = match event {
            HarnessEvent::SessionCreated { .. } | HarnessEvent::SessionLoaded { .. } => {
                self.harness.status.clone()
            }
            HarnessEvent::Error { message }
                if self.harness.status.is_busy() || self.harness.local_session_loading =>
            {
                HarnessStatus::Failed(message.clone())
            }
            HarnessEvent::SessionLoadFailed { message, .. } => {
                HarnessStatus::Failed(message.clone())
            }
            HarnessEvent::Exited { reason } => HarnessStatus::Exited(reason.clone()),
            _ => return,
        };
        self.harness_checks.insert(key, status);
    }

    /// Explicitly starts an adapter and an empty ACP session, never a prompt or login.
    /// A successful test does not alter the current conversation or default harness.
    pub(crate) fn test_harness_connection(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut config = match self.local_harness_config(name) {
            Ok(config) => config,
            Err(error) => {
                logging::error(&format!("{name} connection test · {error}"));
                self.set_notice(error, cx);
                return;
            }
        };
        let key = config_key(name, &config);
        if self
            .harness_checks
            .get(&key)
            .is_some_and(HarnessStatus::is_busy)
        {
            return;
        }
        logging::register_secrets(config.process_env.clone());
        config.diagnostics = Some(logging::diagnostic_sink(format!(
            "{name} · project {} · connection test",
            self.project().id
        )));
        self.harness_checks.insert(key, HarnessStatus::Starting);
        let name = name.to_owned();
        logging::info(&format!("{name} · connection test started"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let adapter = AcpHarness::spawn(config);
            let mut ready = HarnessStatus::Offline;
            let check = async {
                while let Ok(event) = adapter.events().recv().await {
                    logging::harness_event(&format!("{name} · connection test"), &event);
                    match event {
                        HarnessEvent::Ready { agent, version } => {
                            ready = HarnessStatus::Ready { agent, version }
                        }
                        HarnessEvent::SessionCreated { .. } => return ready,
                        HarnessEvent::Error { message }
                        | HarnessEvent::SessionLoadFailed { message, .. } => {
                            return HarnessStatus::Failed(message);
                        }
                        HarnessEvent::Exited { reason } => return HarnessStatus::Failed(reason),
                        HarnessEvent::PermissionRequested { request_id, .. } => {
                            adapter.respond_permission(request_id, None);
                        }
                        _ => {}
                    }
                }
                HarnessStatus::Failed("The harness closed before a session was created".into())
            };
            let check = std::pin::pin!(check);
            let timeout = std::pin::pin!(cx.background_executor().timer(Duration::from_secs(35)));
            let result = match futures_util::future::select(check, timeout).await {
                futures_util::future::Either::Left((status, _)) => status,
                futures_util::future::Either::Right(_) => {
                    HarnessStatus::Failed("Connection test timed out".into())
                }
            };
            adapter.shutdown();
            logging::info(&format!(
                "{name} · connection test · {} · {}",
                result.label(),
                result.detail().unwrap_or_default()
            ));
            let _ = this.update(cx, |this, cx| {
                let message = if result.is_ready() {
                    format!("{name} · connection verified; available for new conversations")
                } else {
                    format!(
                        "{name} · {}",
                        result.detail().unwrap_or_else(|| result.label())
                    )
                };
                this.harness_checks.insert(key, result);
                this.harness_setup_notice = Some(message.clone());
                this.set_notice(message, cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verified_connections_are_selectable_and_retry_can_restore_selection() {
        assert!(unavailable_reason(None).is_some());
        for status in [
            HarnessStatus::Offline,
            HarnessStatus::Starting,
            HarnessStatus::Failed("missing API key".into()),
            HarnessStatus::Exited("disconnected".into()),
        ] {
            assert!(unavailable_reason(Some(&status)).is_some());
        }
        let ready = HarnessStatus::Ready {
            agent: "OpenCode".into(),
            version: "1".into(),
        };
        assert!(unavailable_reason(Some(&ready)).is_none());
    }

    #[test]
    fn failed_connection_clears_all_busy_operations() {
        let mut state = HarnessState {
            local_session_loading: true,
            session_metadata_pending: true,
            model_changing: true,
            mode_changing: true,
            ..Default::default()
        };
        state.clear_connection_operations();
        assert!(!state.local_session_loading && !state.session_metadata_pending);
        assert!(!state.model_changing && !state.mode_changing);
    }

    #[test]
    fn verification_is_scoped_to_configuration_not_resume_session() {
        let mut config = HarnessConfig::new("agent --acp", "/project");
        let key = config_key("Agent", &config);
        config.resume_session_id = Some("saved".into());
        assert_eq!(key, config_key("Agent", &config));
        config
            .process_env
            .insert("API_KEY".into(), "changed".into());
        assert_ne!(key, config_key("Agent", &config));
        assert_ne!(
            key,
            config_key(
                "Other agent",
                &HarnessConfig::new("agent --acp", "/project")
            )
        );
        assert_ne!(
            key,
            config_key("Agent", &HarnessConfig::new("agent --acp", "/other"))
        );
        assert_ne!(
            key,
            config_key(
                "Agent",
                &HarnessConfig::new("different-agent --acp", "/project")
            )
        );
        let mut changed_model = HarnessConfig::new("agent --acp", "/project");
        changed_model.default_model = Some("different-model".into());
        assert_ne!(key, config_key("Agent", &changed_model));
    }
}
