//! Local OpenCode provider discovery. Authentication stays in OpenCode's own
//! interactive client; no keys, tokens or credential-list responses enter Dunlin.
use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use dunlin_agent::ModelChoice;
use dunlin_ui::Context;
use serde::{Deserialize, Serialize};

use crate::app::DunlinApp;

pub(crate) const GO: &str = "opencode-go";

#[derive(Clone)]
pub(crate) struct Provider {
    pub id: String,
    pub name: String,
    pub integration_id: String,
    pub accounts: usize,
}

#[derive(Default)]
pub(crate) struct Catalog {
    pub directory: String,
    pub loading: bool,
    pub error: Option<String>,
    pub providers: Vec<Provider>,
    pub models: HashSet<String>,
    generation: u64,
}

impl Catalog {
    fn selectable(
        &self,
        directory: &str,
        provider: Option<&str>,
        choices: &[ModelChoice],
    ) -> Vec<ModelChoice> {
        if self.directory != directory || self.loading || self.error.is_some() {
            return Vec::new();
        }
        choices
            .iter()
            .filter(|choice| {
                provider.is_some()
                    && provider_id(&choice.id) == provider
                    && self.models.contains(&choice.id)
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RecentModel {
    pub scope: String,
    pub id: String,
}

pub(crate) fn provider_id(model_id: &str) -> Option<&str> {
    model_id.split_once('/').map(|(provider, _)| provider)
}

/// Only successfully dispatched models enter the MRU. Keep three per provider
/// and environment/harness, using wire IDs rather than ambiguous display names.
pub(crate) fn remember(recent: &mut Vec<RecentModel>, scope: &str, id: &str) {
    recent.retain(|entry| entry.scope != scope || entry.id != id);
    recent.insert(
        0,
        RecentModel {
            scope: scope.into(),
            id: id.into(),
        },
    );
    let mut count = 0;
    recent.retain(|entry| {
        if entry.scope == scope && provider_id(&entry.id) == provider_id(id) {
            count += 1;
            count <= 3
        } else {
            true
        }
    });
}

pub(crate) fn sections<'a>(
    choices: &'a [ModelChoice],
    recent: &[RecentModel],
    scope: &str,
) -> (Vec<&'a ModelChoice>, Vec<&'a ModelChoice>) {
    let mut top = Vec::new();
    for entry in recent.iter().filter(|entry| entry.scope == scope) {
        if let Some(choice) = choices.iter().find(|choice| choice.id == entry.id)
            && !top
                .iter()
                .any(|existing: &&ModelChoice| existing.id == choice.id)
        {
            top.push(choice);
            if top.len() == 3 {
                break;
            }
        }
    }
    let rest = choices
        .iter()
        .filter(|choice| !top.iter().any(|top| top.id == choice.id))
        .collect();
    (top, rest)
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Use the CLI's discovery and authentication, not an unauthenticated HTTP
/// connection to a guessed server. Never surface raw API bodies or stderr.
fn api(directory: &str, resource: &str) -> Result<serde_json::Value, String> {
    let path = format!(
        "/api/{resource}?location%5Bdirectory%5D={}",
        encode(directory)
    );
    let output = command_output(
        Command::new("opencode")
            .args(["api", "get", &path])
            .current_dir(directory),
        Duration::from_secs(30),
    )
    .map_err(|_| "Could not run OpenCode provider discovery".to_string())?;
    if !output.status.success() {
        return Err("OpenCode could not load providers. Check its connection and retry.".into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "OpenCode returned an unsupported provider response".into())
}

/// Drain pipes while waiting so a large catalog cannot deadlock the process;
/// terminate only our CLI child on timeout, never the shared OpenCode service.
fn command_output(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (sender, reader) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "OpenCode command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = reader
        .recv_timeout(timeout.saturating_sub(start.elapsed()))
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "OpenCode output timed out")
        })??;
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

fn parse_catalog(
    providers: serde_json::Value,
    models: serde_json::Value,
    integrations: serde_json::Value,
) -> Result<(Vec<Provider>, HashSet<String>), String> {
    #[derive(Deserialize)]
    struct Envelope<T> {
        data: Vec<T>,
    }
    #[derive(Deserialize)]
    struct WireProvider {
        id: String,
        name: String,
        activation: String,
        #[serde(rename = "integrationID")]
        integration_id: Option<String>,
    }
    #[derive(Deserialize)]
    struct WireModel {
        id: String,
        #[serde(rename = "providerID")]
        provider_id: String,
        enabled: bool,
    }
    #[derive(Deserialize)]
    struct Integration {
        id: String,
        connections: Vec<serde_json::Value>,
    }
    let providers: Envelope<WireProvider> = serde_json::from_value(providers)
        .map_err(|_| "Unsupported OpenCode provider catalog".to_string())?;
    let models: Envelope<WireModel> = serde_json::from_value(models)
        .map_err(|_| "Unsupported OpenCode model catalog".to_string())?;
    let integrations: Envelope<Integration> = serde_json::from_value(integrations)
        .map_err(|_| "Unsupported OpenCode integration catalog".to_string())?;
    let mut providers: Vec<_> = providers
        .data
        .into_iter()
        .filter(|provider| provider.activation != "disabled")
        .map(|provider| {
            let integration_id = provider
                .integration_id
                .unwrap_or_else(|| provider.id.clone());
            let accounts = integrations
                .data
                .iter()
                .find(|integration| integration.id == integration_id)
                .map(|integration| integration.connections.len())
                .unwrap_or(0);
            Provider {
                integration_id,
                id: provider.id,
                name: provider.name,
                accounts,
            }
        })
        .collect();
    providers.sort_by_key(|provider| (provider.id != GO, provider.name.to_lowercase()));
    let models = models
        .data
        .into_iter()
        .filter(|model| {
            model.enabled
                && providers
                    .iter()
                    .any(|provider| provider.id == model.provider_id)
        })
        .map(|model| format!("{}/{}", model.provider_id, model.id))
        .collect();
    Ok((providers, models))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn auth_command(executable: &str, directory: &str, integration: &str) -> String {
    format!(
        "cd {} && {} auth login {}",
        shell_quote(directory),
        shell_quote(executable),
        shell_quote(integration)
    )
}

fn launch_auth(directory: &str, integration: &str) -> Result<(), String> {
    let executable = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join("opencode"))
        .find(|path| path.is_file())
        .ok_or("OpenCode is not installed")?;
    let command = auth_command(&executable.to_string_lossy(), directory, integration);
    // Launch Services can open a .command without Apple Events/Automation
    // permission. The private, disposable script contains no credentials and
    // removes itself after sign-in. OpenCode owns the interactive secret prompt.
    let path = std::env::temp_dir().join(format!(
        "dunlin-opencode-auth-{}.command",
        uuid::Uuid::now_v7()
    ));
    let launch = || -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)?;
        writeln!(file, "#!/bin/sh\ntrap 'rm -f -- \"$0\"' EXIT\n{command}")?;
        drop(file);
        let output = command_output(
            Command::new("/usr/bin/open")
                .args(["-a", "Terminal"])
                .arg(&path),
            Duration::from_secs(15),
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("Terminal launch failed"))
        }
    };
    launch().map_err(|_| {
        let _ = std::fs::remove_file(&path);
        "Could not open sign-in. Run ‘opencode auth login’ in Terminal, then refresh providers."
            .into()
    })
}

impl DunlinApp {
    pub(crate) fn local_opencode(&self) -> bool {
        !self.project().is_remote() && self.session_harness() == "OpenCode"
    }

    pub(crate) fn model_scope(&self) -> String {
        let environment = self
            .project()
            .remote
            .as_ref()
            .map(|project| format!("remote:{}:{}", project.host_id, self.project().id))
            .unwrap_or_else(|| "local".into());
        format!("{environment}:{}", self.session_harness())
    }

    pub(crate) fn selected_provider(&self) -> Option<&str> {
        if !self.local_opencode() {
            return None;
        }
        self.opencode_provider.as_deref().or_else(|| {
            self.harness
                .models
                .as_ref()
                .and_then(|models| provider_id(&models.current))
        })
    }

    pub(crate) fn visible_models(&self) -> Vec<ModelChoice> {
        let Some(models) = &self.harness.models else {
            return Vec::new();
        };
        if !self.local_opencode() {
            return models.choices.clone();
        }
        self.provider_catalog.selectable(
            &self.project().path,
            self.selected_provider(),
            &models.choices,
        )
    }

    /// Do not silently send through the old billing provider when the UI is
    /// filtering another one. Leave composer contents intact on rejection.
    pub(crate) fn check_provider_model(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.local_opencode() {
            return true;
        }
        let current = self
            .harness
            .models
            .as_ref()
            .map(|models| models.current.as_str());
        if self
            .visible_models()
            .iter()
            .any(|choice| Some(choice.id.as_str()) == current)
        {
            return true;
        }
        self.set_notice("Select an available model from the chosen provider before sending. Connect your account or refresh providers if the list is empty.", cx);
        false
    }

    pub(crate) fn reload_provider_models(&mut self, cx: &mut Context<Self>) {
        if !self.local_opencode() || self.chat_controls_busy() {
            return;
        }
        self.refresh_providers(cx);
        if self.harness.status.is_ready() {
            self.activate_local_session(cx);
        }
    }

    pub(crate) fn select_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.local_opencode() || self.chat_controls_busy() {
            return;
        }
        if id != GO
            && !self
                .provider_catalog
                .providers
                .iter()
                .any(|provider| provider.id == id)
        {
            return;
        }
        self.opencode_provider = Some(id.into());
        self.persist_settings(cx);
        self.set_notice("Choose a model from this provider before sending. Existing session context is preserved.", cx);
    }

    pub(crate) fn refresh_providers(&mut self, cx: &mut Context<Self>) {
        if !self.local_opencode() || self.projects.is_empty() {
            return;
        }
        let directory = self.project().path.clone();
        if self.provider_catalog.loading && self.provider_catalog.directory == directory {
            return;
        }
        let generation = self.provider_catalog.generation.wrapping_add(1);
        self.provider_catalog = Catalog {
            directory: directory.clone(),
            loading: true,
            generation,
            ..Default::default()
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let requested_directory = directory.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    parse_catalog(
                        api(&directory, "provider")?,
                        api(&directory, "model")?,
                        api(&directory, "integration")?,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.provider_catalog.generation != generation {
                    return;
                }
                this.provider_catalog.loading = false;
                if !this.local_opencode() || this.project().path != requested_directory {
                    return;
                }
                match result {
                    Ok((providers, models)) => {
                        crate::logging::info(&format!("OpenCode provider catalog refreshed · {} providers · {} enabled models", providers.len(), models.len()));
                        this.provider_catalog.providers = providers;
                        this.provider_catalog.models = models;
                    }
                    Err(error) => {
                        crate::logging::warn(&error);
                        this.provider_catalog.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn connect_provider(&mut self, cx: &mut Context<Self>) {
        if !self.local_opencode() || self.projects.is_empty() || self.chat_controls_busy() {
            return;
        }
        let Some(id) = self.selected_provider() else {
            return;
        };
        let integration = self
            .provider_catalog
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .map(|provider| provider.integration_id.clone())
            .unwrap_or_else(|| id.into());
        let directory = self.project().path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { launch_auth(&directory, &integration) }).await;
            let _ = this.update(cx, |this, cx| {
                this.set_notice(match result {
                    Ok(()) => "Complete OpenCode sign-in in Terminal, then choose Refresh providers / models.".into(),
                    Err(error) => error,
                }, cx);
            });
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn choice(id: &str) -> ModelChoice {
        ModelChoice {
            id: id.into(),
            name: id.into(),
            description: None,
        }
    }

    #[test]
    fn distinguishes_go_from_regular_opencode_and_preserves_model_paths() {
        assert_eq!(provider_id("opencode-go/deepseek"), Some(GO));
        assert_eq!(provider_id("opencode/deepseek"), Some("opencode"));
        assert_eq!(provider_id("openrouter/org/model"), Some("openrouter"));
        assert_eq!(provider_id("auto"), None);
    }

    #[test]
    fn only_selected_provider_models_in_both_catalogs_can_be_selected_or_sent() {
        let mut catalog = Catalog {
            directory: "/project".into(),
            models: [
                "opencode-go/same",
                "opencode/same",
                "opencode-go/not-in-acp",
            ]
            .map(String::from)
            .into(),
            ..Default::default()
        };
        let choices = vec![
            choice("opencode/same"),
            choice("opencode-go/same"),
            choice("opencode-go/disabled"),
        ];
        let visible = catalog.selectable("/project", Some(GO), &choices);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "opencode-go/same");
        // This is also the send guard: a previously active ordinary OpenCode
        // model cannot be used while the composer displays Go as its provider.
        assert!(!visible.iter().any(|choice| choice.id == "opencode/same"));
        assert!(
            catalog
                .selectable("/another-project", Some(GO), &choices)
                .is_empty()
        );
        assert!(catalog.selectable("/project", None, &choices).is_empty());
        catalog.loading = true;
        assert!(
            catalog
                .selectable("/project", Some(GO), &choices)
                .is_empty()
        );
        catalog.loading = false;
        catalog.error = Some("unavailable".into());
        assert!(
            catalog
                .selectable("/project", Some(GO), &choices)
                .is_empty()
        );
    }

    #[test]
    fn command_output_drains_stdout_discards_stderr_and_times_out() {
        let output = command_output(
            Command::new("/bin/sh").args(["-c", "printf catalog; printf hidden >&2"]),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"catalog");
        assert!(output.stderr.is_empty());
        let result = command_output(
            Command::new("/bin/sleep").arg("5"),
            Duration::from_millis(50),
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn catalog_excludes_disabled_entries_and_resolves_integration_not_provider() {
        let (providers, models) = parse_catalog(json!({"data":[
            {"id":"opencode", "name":"OpenCode", "activation":"auto"},
            {"id":GO, "name":"Go", "activation":"auto", "integrationID":"opencode"},
            {"id":"off", "name":"Off", "activation":"disabled"}
        ]}), json!({"data":[
            {"id":"same", "providerID":GO, "enabled":true},
            {"id":"same", "providerID":"opencode", "enabled":true},
            {"id":"disabled", "providerID":GO, "enabled":false},
            {"id":"model", "providerID":"off", "enabled":true}
        ]}), json!({"data":[{"id":"opencode", "connections":[{"type":"credential", "id":"cred_test", "method":"oauth"}]}]})).unwrap();
        assert_eq!(providers[0].id, GO);
        assert_eq!(providers[0].integration_id, "opencode");
        assert_eq!(providers[0].accounts, 1);
        assert_eq!(models.len(), 2);
        assert!(models.contains("opencode-go/same"));
        assert!(parse_catalog(json!({}), json!({}), json!({})).is_err());
    }

    #[test]
    fn recent_models_are_unique_bounded_persistent_and_scoped() {
        let mut recent = Vec::new();
        for id in ["go/a", "go/b", "go/c", "go/d", "go/b", "other/a"] {
            remember(&mut recent, "local:OpenCode", id);
        }
        remember(&mut recent, "remote:host:OpenCode", "go/a");
        let recent: Vec<RecentModel> =
            serde_json::from_value(serde_json::to_value(&recent).unwrap()).unwrap();
        let choices = vec![
            choice("go/a"),
            choice("go/b"),
            choice("go/c"),
            choice("go/d"),
        ];
        let (top, rest) = sections(&choices, &recent, "local:OpenCode");
        assert_eq!(
            top.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["go/b", "go/d", "go/c"]
        );
        assert_eq!(rest[0].id, "go/a");
        assert_eq!(
            sections(&choices, &recent, "remote:host:OpenCode").0[0].id,
            "go/a"
        );
        assert!(
            sections(&choices, &recent, "local:Claude Code")
                .0
                .is_empty()
        );
        assert_eq!(
            sections(&[choice("go/d")], &recent, "local:OpenCode")
                .0
                .len(),
            1
        );
    }

    #[test]
    fn auth_quotes_paths_and_integration_ids_without_embedding_secrets() {
        assert_eq!(
            auth_command(
                "/path with spaces/opencode",
                "/a/b's project",
                "opencode-go"
            ),
            "cd '/a/b'\\''s project' && '/path with spaces/opencode' auth login 'opencode-go'"
        );
        assert_eq!(encode("/a?b&c ż"), "%2Fa%3Fb%26c%20%C5%BC");
    }
}
