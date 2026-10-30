//! Local OpenCode provider discovery. Account secrets are never persisted here.
use std::collections::HashSet;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use dunlin_agent::ModelChoice;
use dunlin_ui::Context;
use serde::{Deserialize, Serialize};

use crate::app::DunlinApp;

pub(crate) const GO: &str = "opencode-go";
/// UI-only filter. Never send this identifier to ACP or an auth endpoint.
pub(crate) const FREE: &str = "dunlin:opencode-free";
pub(crate) const CONSOLE: &str = "opencode";

pub(crate) fn account_provider(id: &str) -> &str {
    if id == FREE { CONSOLE } else { id }
}

pub(crate) fn provider_name(id: &str) -> &str {
    match id {
        FREE => "OpenCode Free",
        GO => "OpenCode Go",
        CONSOLE => "OpenCode Console",
        _ => id,
    }
}

pub(crate) fn uses_provider_accounts(harness: &str, remote: bool) -> bool {
    harness == "OpenCode" && !remote
}

#[derive(Clone)]
pub(crate) struct Provider {
    pub id: String,
    pub name: String,
    pub integration_id: String,
    pub accounts: usize,
    pub connected: bool,
}

#[derive(Default)]
pub(crate) struct Catalog {
    pub directory: String,
    pub loading: bool,
    pub error: Option<String>,
    pub providers: Vec<Provider>,
    pub models: HashSet<String>,
    pub free_models: HashSet<String>,
    generation: u64,
}

impl Catalog {
    fn is_connected(&self, directory: &str, provider: Option<&str>) -> bool {
        self.directory == directory
            && self.providers.iter().any(|entry| {
                Some(entry.id.as_str()) == provider.map(account_provider) && entry.connected
            })
    }

    pub(crate) fn confirm_connection(
        &mut self,
        directory: &str,
        id: &str,
        name: &str,
        integration: &str,
    ) {
        if self.directory != directory {
            self.directory = directory.into();
            self.providers.clear();
            self.models.clear();
            self.free_models.clear();
        }
        if !self.providers.iter().any(|provider| provider.id == id) {
            self.providers.push(Provider {
                id: id.into(),
                name: name.into(),
                integration_id: integration.into(),
                accounts: 0,
                connected: false,
            });
        }
        for provider in &mut self.providers {
            if provider.integration_id == integration {
                provider.connected = true;
                provider.accounts = provider.accounts.max(1);
            }
        }
    }

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
                    && (if provider == Some(FREE) {
                        self.free_models.contains(&choice.id)
                    } else {
                        provider_id(&choice.id) == provider
                    })
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

pub(crate) fn model_name<'a>(name: &'a str, provider: Option<&str>) -> &'a str {
    provider
        .map(account_provider)
        .and_then(|provider| name.strip_prefix(provider)?.strip_prefix('/'))
        .unwrap_or(name)
}

fn connection_ready(connection: &serde_json::Value) -> bool {
    matches!(
        connection.get("type").and_then(serde_json::Value::as_str),
        Some("credential" | "env")
    ) && connection
        .get("status")
        .is_none_or(serde_json::Value::is_null)
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

pub(crate) fn encode(value: &str) -> String {
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
pub(crate) fn api(directory: &str, resource: &str) -> Result<serde_json::Value, String> {
    let path = format!(
        "/api/{resource}?location%5Bdirectory%5D={}",
        encode(directory)
    );
    let output = command_output(
        crate::harness_discovery::local_command("opencode")
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
pub(crate) fn command_output(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
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

/// Read metadata only; never fetch credentials or send a model prompt.
pub(crate) fn authenticated_model(
    directory: &str,
    provider: Option<&str>,
    model: &str,
) -> Result<bool, String> {
    let discovered = discover_catalog(directory)?;
    let catalog = Catalog {
        directory: directory.into(),
        providers: discovered.providers,
        models: discovered.models,
        free_models: discovered.free_models,
        ..Default::default()
    };
    let choice = ModelChoice {
        id: model.into(),
        name: model.into(),
        description: None,
    };
    Ok(catalog.is_connected(directory, provider)
        && !catalog
            .selectable(directory, provider, &[choice])
            .is_empty())
}

struct Discovered {
    providers: Vec<Provider>,
    models: HashSet<String>,
    free_models: HashSet<String>,
}

/// Missing/unknown prices, paid cache operations and paid higher-context tiers
/// are not free. Go's promotional/subscription models are intentionally separate.
fn free_cost(cost: &serde_json::Value) -> bool {
    cost.as_array().is_some_and(|tiers| {
        !tiers.is_empty()
            && tiers.iter().all(|tier| {
                ["/input", "/output", "/cache/read", "/cache/write"]
                    .iter()
                    .all(|path| tier.pointer(path).and_then(serde_json::Value::as_f64) == Some(0.0))
            })
    })
}

fn parse_catalog(
    providers: serde_json::Value,
    models: serde_json::Value,
    integrations: serde_json::Value,
) -> Result<Discovered, String> {
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
        #[serde(default)]
        cost: serde_json::Value,
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
            let connections = integrations
                .data
                .iter()
                .find(|integration| integration.id == integration_id)
                .map(|integration| integration.connections.as_slice())
                .unwrap_or(&[]);
            Provider {
                integration_id,
                id: provider.id,
                name: provider.name,
                accounts: connections.len(),
                connected: connections.iter().any(connection_ready),
            }
        })
        .collect();
    providers.sort_by_key(|provider| (provider.id != GO, provider.name.to_lowercase()));
    let enabled: Vec<_> = models
        .data
        .into_iter()
        .filter(|model| {
            model.enabled
                && providers
                    .iter()
                    .any(|provider| provider.id == model.provider_id)
        })
        .collect();
    let free_models = enabled
        .iter()
        .filter(|model| model.provider_id == CONSOLE && free_cost(&model.cost))
        .map(|model| format!("{}/{}", model.provider_id, model.id))
        .collect();
    let models = enabled
        .iter()
        .map(|model| format!("{}/{}", model.provider_id, model.id))
        .collect();
    Ok(Discovered {
        providers,
        models,
        free_models,
    })
}

fn discover_catalog(directory: &str) -> Result<Discovered, String> {
    discover_with_retry(
        || {
            parse_catalog(
                api(directory, "provider")?,
                api(directory, "model")?,
                api(directory, "integration")?,
            )
        },
        std::thread::sleep,
    )
}

fn discover_with_retry(
    mut load: impl FnMut() -> Result<Discovered, String>,
    mut wait: impl FnMut(Duration),
) -> Result<Discovered, String> {
    // ACP initialize can finish before location/provider plugins are ready.
    // Retry only empty startup snapshots, with a fixed bound, not indefinitely.
    for attempt in 0..3 {
        let catalog = load()?;
        if !catalog.providers.is_empty() && (!catalog.models.is_empty() || attempt == 2) {
            return Ok(catalog);
        }
        if attempt < 2 {
            wait(Duration::from_millis(300 * (attempt + 1)));
        }
    }
    Err("OpenCode has not advertised any providers for this project. Refresh providers / models to retry.".into())
}

impl DunlinApp {
    pub(crate) fn local_opencode(&self) -> bool {
        uses_provider_accounts(&self.session_harness(), self.project().is_remote())
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

    pub(crate) fn provider_connected(&self) -> bool {
        self.local_opencode()
            && self
                .provider_catalog
                .is_connected(&self.project().path, self.selected_provider())
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
        if !matches!(id, GO | FREE)
            && !self
                .provider_catalog
                .providers
                .iter()
                .any(|provider| provider.id == id)
        {
            return;
        }
        self.opencode_provider = Some(id.into());
        self.ensure_usage(cx);
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
        // Keep the known account state while refreshing this same location, so
        // a successful sign-in does not flash back into an unsigned state.
        let providers = if self.provider_catalog.directory == directory {
            std::mem::take(&mut self.provider_catalog.providers)
        } else {
            Vec::new()
        };
        self.provider_catalog = Catalog {
            directory: directory.clone(),
            loading: true,
            generation,
            providers,
            ..Default::default()
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let requested_directory = directory.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    discover_catalog(&directory)
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
                    Ok(catalog) => {
                        crate::logging::info(&format!("OpenCode provider catalog refreshed · {} providers · {} enabled models · {} free models", catalog.providers.len(), catalog.models.len(), catalog.free_models.len()));
                        this.provider_catalog.providers = catalog.providers;
                        this.provider_catalog.models = catalog.models;
                        this.provider_catalog.free_models = catalog.free_models;
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

    fn fixture(models: serde_json::Value) -> Discovered {
        parse_catalog(
            json!({"data":[
                {"id":CONSOLE,"name":"Console","activation":"auto","integrationID":CONSOLE},
                {"id":GO,"name":"Go","activation":"auto","integrationID":CONSOLE}
            ]}),
            models,
            json!({"data":[]}),
        )
        .unwrap()
    }

    #[test]
    fn free_filter_and_send_allowlist_exclude_paid_unknown_disabled_and_go_models() {
        let zero = json!({"input":0,"output":0,"cache":{"read":0,"write":0}});
        let paid = json!({"input":1,"output":2,"cache":{"read":0,"write":0}});
        let discovered = fixture(json!({"data":[
            {"id":"free","providerID":CONSOLE,"enabled":true,"cost":[zero]},
            {"id":"big-pickle","providerID":CONSOLE,"enabled":true,"cost":[zero]},
            {"id":"paid-free","providerID":CONSOLE,"enabled":true,"cost":[paid]},
            {"id":"tiered","providerID":CONSOLE,"enabled":true,"cost":[zero,paid]},
            {"id":"cache","providerID":CONSOLE,"enabled":true,"cost":[{"input":0,"output":0,"cache":{"read":1,"write":0}}]},
            {"id":"unknown","providerID":CONSOLE,"enabled":true},
            {"id":"empty","providerID":CONSOLE,"enabled":true,"cost":[]},
            {"id":"partial","providerID":CONSOLE,"enabled":true,"cost":[{"input":0}]},
            {"id":"disabled","providerID":CONSOLE,"enabled":false,"cost":[zero]},
            {"id":"free","providerID":GO,"enabled":true,"cost":[zero]}
        ]}));
        assert_eq!(discovered.free_models.len(), 2);
        let mut catalog = Catalog {
            directory: "/project".into(),
            providers: discovered.providers,
            models: discovered.models,
            free_models: discovered.free_models,
            ..Default::default()
        };
        let choices: Vec<_> = catalog.models.iter().map(|id| choice(id)).collect();
        let visible = catalog.selectable("/project", Some(FREE), &choices);
        assert_eq!(visible.len(), 2);
        assert!(
            visible
                .iter()
                .all(|model| catalog.free_models.contains(&model.id))
        );
        assert!(
            visible
                .iter()
                .all(|model| provider_id(&model.id) == Some(CONSOLE))
        );
        let mut recent = Vec::new();
        for id in ["opencode/free", "opencode/paid-free", "opencode-go/free"] {
            remember(&mut recent, "local:OpenCode", id);
        }
        let (top, _) = sections(&visible, &recent, "local:OpenCode");
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "opencode/free");
        // Neither the filter nor its wire IDs change when authorization completes.
        catalog.confirm_connection("/project", CONSOLE, "Console", CONSOLE);
        assert!(catalog.is_connected("/project", Some(FREE)));
        assert!(!catalog.providers.iter().any(|provider| provider.id == FREE));
        assert!(
            Catalog::default()
                .selectable("/project", Some(FREE), &choices)
                .is_empty()
        );
        catalog.free_models.clear();
        assert!(
            catalog
                .selectable("/project", Some(FREE), &choices)
                .is_empty()
        );
    }

    #[test]
    fn empty_startup_catalog_is_retried_but_never_accepted_or_retried_forever() {
        let empty = || parse_catalog(json!({"data":[]}), json!({"data":[]}), json!({"data":[]}));
        let mut calls = 0;
        let mut waits = 0;
        let result = discover_with_retry(
            || {
                calls += 1;
                empty()
            },
            |_| waits += 1,
        );
        assert!(result.is_err());
        assert_eq!((calls, waits), (3, 2));
        calls = 0;
        let result = discover_with_retry(
            || {
                calls += 1;
                if calls == 1 {
                    empty()
                } else {
                    Ok(fixture(
                        json!({"data":[{"id":"model","providerID":CONSOLE,"enabled":true}]}),
                    ))
                }
            },
            |_| {},
        );
        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    #[ignore = "requires installed OpenCode and its current public model catalog"]
    fn local_discovery_finds_free_models_without_changing_provider_or_model() {
        let directory = std::env::current_dir().unwrap();
        let discovered = discover_catalog(directory.to_str().unwrap()).unwrap();
        assert!(!discovered.providers.is_empty());
        assert!(!discovered.free_models.is_empty());
        assert!(
            discovered
                .free_models
                .iter()
                .all(|id| { provider_id(id) == Some(CONSOLE) && discovered.models.contains(id) })
        );
        assert!(
            discovered
                .providers
                .iter()
                .any(|provider| provider.id == GO && provider.integration_id == CONSOLE)
        );
    }

    #[test]
    fn provider_accounts_belong_only_to_the_local_opencode_harness() {
        assert!(uses_provider_accounts("OpenCode", false));
        assert!(!uses_provider_accounts("OpenCode", true));
        for harness in ["Claude Code", "OpenAI Codex", "Custom ACP", "Other"] {
            assert!(!uses_provider_accounts(harness, false));
            assert!(!uses_provider_accounts(harness, true));
        }
    }

    #[test]
    fn account_state_follows_the_selected_provider_and_location() {
        let mut catalog = Catalog::default();
        assert!(!catalog.is_connected("/project", Some(GO)));
        catalog.confirm_connection("/project", GO, "Go", "console");
        assert!(catalog.is_connected("/project", Some(GO)));
        assert!(!catalog.is_connected("/other-project", Some(GO)));
        assert!(!catalog.is_connected("/project", Some("another-provider")));
        assert!(!catalog.is_connected("/project", None));
        // Success is reflected immediately; the directory's next catalog
        // refresh can carry this public state without showing Sign in again.
        let providers = std::mem::take(&mut catalog.providers);
        let catalog = Catalog {
            directory: "/project".into(),
            loading: true,
            providers,
            ..Default::default()
        };
        assert!(catalog.is_connected("/project", Some(GO)));
    }

    #[test]
    fn public_connection_state_excludes_accounts_needing_reauthorization() {
        assert!(connection_ready(
            &json!({"type":"credential","method":"key"})
        ));
        assert!(connection_ready(&json!({"type":"env","name":"API_KEY"})));
        assert!(!connection_ready(
            &json!({"type":"credential","status":{"status":"needs_auth","message":"expired"}})
        ));
        assert!(!connection_ready(&json!({"type":"unknown"})));
        let Discovered { providers, .. } = parse_catalog(
            json!({"data":[{"id":GO,"name":"Go","activation":"auto","integrationID":"console"}]}),
            json!({"data":[]}),
            json!({"data":[{"id":"console","connections":[{"type":"credential","status":{"status":"needs_auth","message":"expired"}}]}]}),
        ).unwrap();
        assert_eq!(providers[0].accounts, 1);
        assert!(!providers[0].connected);
    }

    #[test]
    fn model_display_omits_only_the_selected_provider_prefix() {
        assert_eq!(
            model_name("opencode-go/DeepSeek V4 Pro", Some(GO)),
            "DeepSeek V4 Pro"
        );
        assert_eq!(
            model_name("opencode-go/DeepSeek", Some("opencode")),
            "opencode-go/DeepSeek"
        );
        assert_eq!(
            model_name("openrouter/org/model", Some("openrouter")),
            "org/model"
        );
        assert_eq!(model_name("Claude Sonnet", None), "Claude Sonnet");
        assert_eq!(model_name("opencode/Big Pickle", Some(FREE)), "Big Pickle");
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
        let Discovered { providers, models, .. } = parse_catalog(json!({"data":[
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
        assert!(providers[0].connected);
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
    fn encodes_project_paths() {
        assert_eq!(encode("/a?b&c ż"), "%2Fa%3Fb%26c%20%C5%BC");
    }
}
