//! Use-only credential vault: metadata UI, broker service and harness wiring.

pub(crate) mod panel;
pub(crate) mod service;
pub(crate) mod settings;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use dunlin_agent::McpServerConfig;
use dunlin_domain::vault::{
    GrantPolicy, HttpMethod, Injection, Recipe, Secret, SecretAudit, SecretGrant, SecretPolicy,
    normalize_ttl, now_secs,
};
use dunlin_domain::{PermissionMode, Project};
use dunlin_storage::Store;
use dunlin_ui::{Context, PromptButton, PromptLevel, Window};
use dunlin_vault::store::{KeychainStore, SecretStore};

use crate::app::DunlinApp;

pub(crate) use service::SessionHandle;

#[derive(Default)]
pub(crate) struct Ui {
    pub draft: Option<settings::Draft>,
    pub recipe_draft: Option<settings::RecipeDraft>,
    pub operation_draft: Option<settings::OperationDraft>,
    pub message: Option<String>,
    pub busy: bool,
    pub panel: panel::State,
}

pub(crate) struct State {
    pub service: Option<service::ServiceHandle>,
    pub error: Option<String>,
    pub enabled: bool,
    /// Persisted preference. It can be true while `enabled` is false because
    /// storage was unavailable at launch.
    pub requested_enabled: bool,
    pub secrets: Vec<Secret>,
    pub grants: Vec<SecretGrant>,
    pub recipes: HashMap<String, Vec<Recipe>>,
    pub operations: HashMap<String, Vec<dunlin_domain::vault::Operation>>,
    pub audit: Vec<SecretAudit>,
    pub approval_tx: async_channel::Sender<service::ApprovalJob>,
    pub approval_rx: Option<async_channel::Receiver<service::ApprovalJob>>,
    pub refresh_generation: u64,
    pub ui: Ui,
}

impl State {
    pub(crate) fn start(enabled: bool) -> Self {
        let (approval_tx, approval_rx) = async_channel::unbounded();
        let (service, error) = if enabled {
            let approver = Arc::new(service::UiApprover::new(approval_tx.clone()));
            match service::ServiceHandle::start_with_approver(approver) {
                Ok(service) => (Some(service), None),
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, None)
        };
        let service_ok = service.is_some();
        Self {
            service,
            error,
            enabled: enabled && service_ok,
            requested_enabled: enabled,
            secrets: Vec::new(),
            grants: Vec::new(),
            recipes: HashMap::new(),
            operations: HashMap::new(),
            audit: Vec::new(),
            approval_tx,
            approval_rx: Some(approval_rx),
            refresh_generation: 0,
            ui: Ui::default(),
        }
    }

    pub(crate) fn status(&self) -> &'static str {
        if !self.requested_enabled {
            "disabled"
        } else if self.enabled && self.service.is_some() {
            "ready"
        } else {
            "unavailable"
        }
    }
}

struct VaultSnapshot {
    secrets: Vec<Secret>,
    grants: Vec<SecretGrant>,
    recipes: HashMap<String, Vec<Recipe>>,
    operations: HashMap<String, Vec<dunlin_domain::vault::Operation>>,
    audit: Vec<SecretAudit>,
    error: Option<String>,
}

fn load_vault_snapshot() -> VaultSnapshot {
    let Some(store) = crate::app::open_store().ok() else {
        return VaultSnapshot {
            secrets: Vec::new(),
            grants: Vec::new(),
            recipes: HashMap::new(),
            operations: HashMap::new(),
            audit: Vec::new(),
            error: Some("Local storage is unavailable".into()),
        };
    };
    let secrets = match store.secrets() {
        Ok(secrets) => secrets,
        Err(error) => {
            return VaultSnapshot {
                secrets: Vec::new(),
                grants: Vec::new(),
                recipes: HashMap::new(),
                operations: HashMap::new(),
                audit: Vec::new(),
                error: Some(format!("Could not load credentials: {error}")),
            };
        }
    };
    let mut snapshot = VaultSnapshot {
        secrets: Vec::new(),
        grants: Vec::new(),
        recipes: HashMap::new(),
        operations: HashMap::new(),
        audit: store.secret_audit(None, 100).unwrap_or_default(),
        error: None,
    };
    let mut unavailable = 0;
    for mut secret in secrets.into_iter().take(256) {
        match KeychainStore
            .get_policy(&secret.id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .filter(|policy| policy.validate().is_ok())
        {
            Some(policy) => {
                secret.apply_policy(&policy);
                for project in &policy.project_ids {
                    snapshot.grants.push(SecretGrant {
                        secret_id: secret.id.clone(),
                        project_id: project.clone(),
                        policy: GrantPolicy::Auto,
                        created_at: 0,
                    });
                }
                snapshot
                    .recipes
                    .insert(secret.id.clone(), policy.recipes.clone());
                snapshot
                    .operations
                    .insert(secret.id.clone(), policy.operations.clone());
            }
            None => {
                unavailable += 1;
                secret.auto_approve = false;
                secret.allowed_hosts.clear();
                secret.allowed_methods.clear();
                secret.expires_at = None;
            }
        }
        snapshot.secrets.push(secret);
    }
    snapshot.error = (unavailable > 0).then(|| {
        format!(
            "{unavailable} credential(s) have an unavailable or invalid policy and are inert. Replace or delete them."
        )
    });
    snapshot
}

/// Values entered in the settings or bottom-panel form. The value is moved into
/// the Keychain immediately and is never retained in app state.
pub(crate) struct NewSecret {
    pub storage_only: bool,
    pub label: String,
    pub description: String,
    pub value: String,
    pub hosts: Vec<String>,
    pub path_prefix: String,
    pub injection: Injection,
    pub allowed_headers: Vec<String>,
    pub ttl_secs: i64,
    pub allow_writes: bool,
    pub auto_approve: bool,
    pub project_ids: Vec<String>,
}

pub(crate) fn slug(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "key".into() } else { out }
}

fn validate_hosts(hosts: &[String]) -> Result<Vec<String>, String> {
    if hosts.is_empty() {
        return Err("Add at least one API hostname, for example api.github.com".into());
    }
    if hosts.len() > 16 {
        return Err("A credential may target at most 16 hostnames".into());
    }
    let mut normalized = Vec::new();
    for host in hosts {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty()
            || host.len() > 253
            || host.contains(['/', ':', ' ', '*', '?', '#', '@'])
            || host.parse::<IpAddr>().is_ok()
            || host == "localhost"
            || host.ends_with(".localhost")
            || host.ends_with(".local")
            || host.ends_with(".localdomain")
            || host.ends_with(".internal")
        {
            return Err(format!("'{host}' is not a public API hostname"));
        }
        if !normalized.contains(&host) {
            normalized.push(host);
        }
    }
    Ok(normalized)
}

fn validate_header(name: &str, prefix: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name.len() > 128
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
    {
        return Err("The credential header name is invalid".into());
    }
    if name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("upgrade")
    {
        return Err("That header is managed by Dunlin".into());
    }
    if prefix.contains(['\r', '\n', '\0']) {
        return Err("The credential header prefix contains control characters".into());
    }
    Ok(())
}

fn normalized_headers(headers: &[String]) -> Vec<String> {
    let mut headers = headers
        .iter()
        .map(|header| header.trim().to_string())
        .filter(|header| !header.is_empty())
        .collect::<Vec<_>>();
    for default in ["Accept", "Content-Type"] {
        if !headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case(default))
        {
            headers.push(default.to_string());
        }
    }
    headers
}

fn validate_headers(headers: &[String]) -> Result<(), String> {
    for header in headers {
        let header = header.trim();
        if header.is_empty() {
            continue;
        }
        if header.eq_ignore_ascii_case("authorization")
            || header.eq_ignore_ascii_case("proxy-authorization")
        {
            return Err("Authorization headers are managed by the credential injection".into());
        }
        validate_header(header, "")?;
    }
    Ok(())
}

fn validate_path_prefix(prefix: &str) -> Result<String, String> {
    let prefix = prefix.trim();
    let prefix = if prefix.is_empty() { "/" } else { prefix };
    if !prefix.starts_with('/') || prefix.contains(['?', '#', '\\']) {
        return Err("The path prefix must start with '/' and contain no query or fragment".into());
    }
    Ok(prefix.trim_end_matches('/').to_string()).map(|prefix| {
        if prefix.is_empty() {
            "/".into()
        } else {
            prefix
        }
    })
}

fn existing_aliases(store: &Store) -> Vec<String> {
    store
        .secrets()
        .map(|secrets| secrets.into_iter().map(|secret| secret.alias).collect())
        .unwrap_or_default()
}

fn unique_alias(store: &Store, label: &str, current: Option<&str>) -> String {
    let mut base = slug(label);
    if base.is_empty() {
        base = "key".into();
    }
    let aliases = existing_aliases(store);
    let mut candidate = base.clone();
    let mut suffix = 2;
    while aliases
        .iter()
        .any(|alias| alias == &candidate && Some(alias.as_str()) != current)
    {
        candidate = format!("{}-{suffix}", base.trim_end_matches('-'));
        suffix += 1;
    }
    candidate
}

impl DunlinApp {
    pub(crate) fn refresh_vault(&mut self, cx: &mut Context<Self>) {
        self.vault.refresh_generation = self.vault.refresh_generation.wrapping_add(1);
        let generation = self.vault.refresh_generation;
        self.vault.ui.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { load_vault_snapshot() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.vault.refresh_generation != generation {
                    return;
                }
                this.vault.secrets = snapshot.secrets;
                this.vault.grants = snapshot.grants;
                this.vault.recipes = snapshot.recipes;
                this.vault.operations = snapshot.operations;
                this.vault.audit = snapshot.audit;
                this.vault.error = snapshot.error;
                this.vault.ui.busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn vault_granted_projects(&self, secret_id: &str) -> Vec<&SecretGrant> {
        self.vault
            .grants
            .iter()
            .filter(|grant| grant.secret_id == secret_id && grant.policy != GrantPolicy::Deny)
            .collect()
    }

    pub(crate) fn vault_audit_for(&self, secret_id: &str) -> Vec<&SecretAudit> {
        self.vault
            .audit
            .iter()
            .filter(|audit| audit.secret_id.as_deref() == Some(secret_id))
            .take(20)
            .collect()
    }

    pub(crate) fn vault_secrets_for_project(&self, project_id: &str) -> Vec<&Secret> {
        let now = now_secs();
        self.vault
            .secrets
            .iter()
            .filter(|secret| !secret.is_expired(now))
            .filter(|secret| {
                self.vault.grants.iter().any(|grant| {
                    grant.secret_id == secret.id
                        && grant.project_id == project_id
                        && grant.policy != GrantPolicy::Deny
                })
            })
            .collect()
    }

    pub(crate) fn create_vault_secret(
        &mut self,
        input: NewSecret,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let label = input.label.trim().to_string();
        if label.is_empty() || label.len() > 120 {
            return Err("Name the credential with 1–120 characters".into());
        }
        if input.description.len() > 1024 {
            return Err("The agent description must fit in 1,024 bytes".into());
        }
        if input.value.trim().is_empty() {
            return Err("Paste the credential value".into());
        }
        if input.value.len() > 8192 {
            return Err("The credential value is too large".into());
        }
        let hosts = if input.storage_only {
            Vec::new()
        } else {
            validate_hosts(&input.hosts)?
        };
        let path_prefix = validate_path_prefix(&input.path_prefix)?;
        validate_headers(&input.allowed_headers)?;
        let ttl = normalize_ttl(input.ttl_secs)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        if store
            .secrets()
            .map(|secrets| secrets.len())
            .unwrap_or_default()
            >= 128
        {
            return Err("The credential vault is limited to 128 entries".into());
        }
        let alias = unique_alias(store, &label, None);
        let id = format!("dunlin/{}", uuid::Uuid::now_v7().simple());
        let now = now_secs();
        let mut methods = if input.storage_only {
            Vec::new()
        } else {
            vec![HttpMethod::Get, HttpMethod::Head]
        };
        if !input.storage_only && input.allow_writes {
            methods.extend([
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
            ]);
        }
        let secret = Secret {
            id: id.clone(),
            namespace: "dunlin".into(),
            alias: alias.clone(),
            label,
            description: input.description.trim().to_string(),
            injection: input.injection,
            allowed_hosts: hosts,
            path_prefix,
            allowed_methods: methods,
            allowed_headers: normalized_headers(&input.allowed_headers),
            auto_approve: !input.storage_only && input.auto_approve,
            expires_at: (!input.storage_only).then_some(now + ttl),
            created_at: now,
            updated_at: now,
            last_used_at: None,
        };
        let policy = secret.policy(
            if input.storage_only {
                Vec::new()
            } else {
                input.project_ids.clone()
            },
            Vec::new(),
            Vec::new(),
        );
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        if let Err(error) = store.upsert_secret(&secret) {
            return Err(format!("Could not save credential metadata: {error}"));
        }
        if let Err(error) = KeychainStore.set_policy(&id, &policy_json) {
            let _ = store.delete_secret(&id);
            return Err(format!("Could not save the credential policy: {error}"));
        }
        if let Err(error) = KeychainStore.set(&id, input.value.trim()) {
            let _ = KeychainStore.delete_policy(&id);
            let _ = store.delete_secret(&id);
            return Err(format!("Could not save the credential: {error}"));
        }
        self.refresh_vault(cx);
        self.vault.ui.message = Some(format!(
            "Created '{alias}'. {}",
            if input.storage_only {
                "Stored in Keychain; agent access is not configured.".into()
            } else {
                format!("It expires {}.", expiry_label(ttl))
            }
        ));
        Ok(alias)
    }

    pub(crate) fn update_vault_secret(
        &mut self,
        id: &str,
        input: NewSecret,
        new_value: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let label = input.label.trim().to_string();
        if label.is_empty() || label.len() > 120 {
            return Err("Name the credential with 1–120 characters".into());
        }
        if input.description.len() > 1024 {
            return Err("The agent description must fit in 1,024 bytes".into());
        }
        let hosts = if input.storage_only {
            Vec::new()
        } else {
            validate_hosts(&input.hosts)?
        };
        let path_prefix = validate_path_prefix(&input.path_prefix)?;
        validate_headers(&input.allowed_headers)?;
        let ttl = normalize_ttl(input.ttl_secs)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        let existing = store
            .secret_by_id(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "That credential no longer exists".to_string())?;
        let existing_policy = KeychainStore
            .get_policy(id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .filter(|policy| policy.validate().is_ok());
        let recipes = existing_policy
            .as_ref()
            .map(|policy| policy.recipes.clone())
            .unwrap_or_default();
        let operations = existing_policy
            .as_ref()
            .map(|policy| policy.operations.clone())
            .unwrap_or_default();
        let managed = existing_policy
            .as_ref()
            .map(|policy| policy.managed)
            .unwrap_or(true);
        let now = now_secs();
        let mut methods = if input.storage_only {
            Vec::new()
        } else {
            vec![HttpMethod::Get, HttpMethod::Head]
        };
        if !input.storage_only && input.allow_writes {
            methods.extend([
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
            ]);
        }
        let alias = unique_alias(store, &label, Some(&existing.alias));
        let secret = Secret {
            id: id.to_string(),
            namespace: if managed {
                "dunlin".into()
            } else {
                "external".into()
            },
            alias,
            label: label.clone(),
            description: input.description.trim().to_string(),
            injection: input.injection,
            allowed_hosts: hosts,
            path_prefix,
            allowed_methods: methods,
            allowed_headers: normalized_headers(&input.allowed_headers),
            auto_approve: !input.storage_only && input.auto_approve,
            expires_at: (!input.storage_only).then_some(now + ttl),
            created_at: existing.created_at,
            updated_at: now,
            last_used_at: existing.last_used_at,
        };
        let policy = if input.storage_only {
            secret.policy(Vec::new(), Vec::new(), Vec::new())
        } else {
            secret.policy(input.project_ids.clone(), recipes, operations)
        };
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        KeychainStore
            .set_policy(id, &policy_json)
            .map_err(|error| format!("Could not save the credential policy: {error}"))?;
        if let Some(value) = new_value.filter(|value| !value.trim().is_empty()) {
            if value.len() > 8192 {
                return Err("The credential value is too large".into());
            }
            KeychainStore
                .set(id, value.trim())
                .map_err(|error| format!("Could not replace the credential: {error}"))?;
        }
        store
            .upsert_secret(&secret)
            .map_err(|error| format!("Could not save credential metadata: {error}"))?;
        self.refresh_vault(cx);
        self.vault.ui.message = Some(format!("Updated '{label}'."));
        Ok(())
    }

    pub(crate) fn revoke_all_vault_secrets(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        let secrets = store
            .secrets()
            .map_err(|error| format!("Could not list credentials: {error}"))?;
        let mut failed = 0;
        for secret in secrets {
            if KeychainStore.delete(&secret.id).is_err() {
                failed += 1;
                continue;
            }
            let _ = KeychainStore.delete_policy(&secret.id);
            let _ = store.delete_secret(&secret.id);
        }
        if let Some(session) = self.harness.vault_session.take() {
            session.revoke();
        }
        for runtime in self.parked_runtimes.values_mut() {
            if let Some(session) = runtime.state.vault_session.take() {
                session.revoke();
            }
        }
        self.vault.ui.draft = None;
        self.vault.ui.recipe_draft = None;
        self.vault.ui.operation_draft = None;
        self.vault.ui.panel.value = None;
        self.refresh_vault(cx);
        if failed > 0 {
            Err(format!(
                "Revoked the remaining quick keys, but {failed} value(s) could not be removed from the Keychain; their metadata was retained. Retry after unlocking the Keychain."
            ))
        } else {
            self.vault.ui.message = Some(
                "Revoked all quick keys. Restart active or parked harnesses to use new credentials."
                    .into(),
            );
            Ok(())
        }
    }

    pub(crate) fn delete_vault_secret(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        let alias = store
            .secret_by_id(id)
            .ok()
            .flatten()
            .map(|secret| secret.alias)
            .unwrap_or_else(|| "credential".into());
        KeychainStore
            .delete(id)
            .map_err(|error| format!("Could not remove the credential value: {error}"))?;
        let _ = KeychainStore.delete_policy(id);
        store
            .delete_secret(id)
            .map_err(|error| format!("Could not remove the credential metadata: {error}"))?;
        self.refresh_vault(cx);
        self.vault.ui.message = Some(format!("Deleted '{alias}'."));
        Ok(())
    }

    pub(crate) fn add_vault_operation(
        &mut self,
        secret_id: &str,
        operation: dunlin_domain::vault::Operation,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut policy = KeychainStore
            .get_policy(secret_id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .ok_or_else(|| {
                "The credential policy is unavailable; re-save the credential first".to_string()
            })?;
        policy
            .operations
            .retain(|existing| existing.name != operation.name);
        policy.operations.push(operation.clone());
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        KeychainStore
            .set_policy(secret_id, &policy_json)
            .map_err(|error| format!("Could not save the credential policy: {error}"))?;
        self.refresh_vault(cx);
        self.vault.ui.message = Some(format!("Saved operation '{}'.", operation.name));
        Ok(())
    }

    pub(crate) fn remove_vault_operation(
        &mut self,
        secret_id: &str,
        operation_name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut policy = KeychainStore
            .get_policy(secret_id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .ok_or_else(|| {
                "The credential policy is unavailable; re-save the credential first".to_string()
            })?;
        policy
            .operations
            .retain(|operation| operation.name != operation_name);
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        KeychainStore
            .set_policy(secret_id, &policy_json)
            .map_err(|error| format!("Could not save the credential policy: {error}"))?;
        self.refresh_vault(cx);
        self.vault.ui.message = Some("Removed the HTTP operation.".into());
        Ok(())
    }

    pub(crate) fn add_vault_recipe(
        &mut self,
        secret_id: &str,
        mut recipe: Recipe,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        recipe.secret_id = secret_id.to_string();
        recipe.validate()?;
        if recipe.program_hash.is_none() {
            recipe.program_hash = Some(
                dunlin_vault::recipe::program_hash(std::path::Path::new(&recipe.program))
                    .map_err(|error| format!("Could not pin the recipe program: {error}"))?,
            );
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        let mut policy = KeychainStore
            .get_policy(secret_id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .ok_or_else(|| {
                "The credential policy is unavailable; re-save the credential first".to_string()
            })?;
        policy
            .recipes
            .retain(|existing| existing.id != recipe.id && existing.name != recipe.name);
        policy.recipes.push(recipe.clone());
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        KeychainStore
            .set_policy(secret_id, &policy_json)
            .map_err(|error| format!("Could not save the credential policy: {error}"))?;
        let _ = store.upsert_recipe(&recipe);
        self.refresh_vault(cx);
        self.vault.ui.message = Some(format!("Saved recipe '{}'.", recipe.name));
        Ok(())
    }

    pub(crate) fn remove_vault_recipe(
        &mut self,
        secret_id: &str,
        recipe_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "Local storage is unavailable".to_string())?;
        let mut policy = KeychainStore
            .get_policy(secret_id)
            .ok()
            .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok())
            .ok_or_else(|| {
                "The credential policy is unavailable; re-save the credential first".to_string()
            })?;
        policy.recipes.retain(|recipe| recipe.id != recipe_id);
        policy.validate()?;
        let policy_json = policy_json(&policy)?;
        KeychainStore
            .set_policy(secret_id, &policy_json)
            .map_err(|error| format!("Could not save the credential policy: {error}"))?;
        let _ = store.delete_recipe(recipe_id);
        self.refresh_vault(cx);
        self.vault.ui.message = Some("Removed the CLI recipe.".into());
        Ok(())
    }

    pub(crate) fn set_vault_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.vault.enabled == enabled && self.vault.requested_enabled == enabled {
            return;
        }
        self.vault.requested_enabled = enabled;
        if enabled && self.store.is_none() {
            self.vault.error = Some("Local storage is unavailable".into());
            cx.notify();
            return;
        }
        if !enabled {
            if let Some(session) = self.harness.vault_session.take() {
                session.revoke();
            }
            for runtime in self.parked_runtimes.values_mut() {
                if let Some(session) = runtime.state.vault_session.take() {
                    session.revoke();
                }
            }
            self.vault.ui.draft = None;
            self.vault.ui.recipe_draft = None;
            self.vault.ui.operation_draft = None;
            self.vault.ui.panel.value = None;
            self.vault.service.take();
            self.vault.enabled = false;
            self.vault.ui.message = Some("Credential broker disabled.".into());
        } else {
            if self.vault.service.is_some() {
                self.vault.enabled = true;
                self.vault.error = None;
                self.persist_settings(cx);
                cx.notify();
                return;
            }
            let approver = Arc::new(service::UiApprover::new(self.vault.approval_tx.clone()));
            match service::ServiceHandle::start_with_approver(approver) {
                Ok(service) => {
                    self.vault.service = Some(service);
                    self.vault.error = None;
                    self.vault.enabled = true;
                    self.vault.ui.message = Some(
                        "Credential broker enabled. Restart the current harness to attach the vault MCP server."
                            .into(),
                    );
                }
                Err(error) => {
                    self.vault.error = Some(error);
                    self.vault.enabled = false;
                }
            }
        }
        self.persist_settings(cx);
        cx.notify();
    }

    /// Create a session token and attach the MCP stdio proxy to a local harness.
    pub(crate) fn attach_vault(
        &mut self,
        config: &mut dunlin_agent::HarnessConfig,
        project: &Project,
        read_only: bool,
    ) -> Option<SessionHandle> {
        if self.store.is_none() || !self.vault.enabled || project.is_remote() || !project.trusted {
            return None;
        }
        let service = self.vault.service.as_ref()?;
        let session = service.new_session(
            project.id.clone(),
            project.path.clone(),
            project.trusted,
            read_only,
        );
        let Ok(executable) = std::env::current_exe() else {
            session.revoke();
            return None;
        };
        let socket = service.socket_path().to_string_lossy().to_string();
        config.mcp_servers.push(McpServerConfig::stdio(
            "dunlin-secrets",
            executable,
            vec!["--mcp-vault".into()],
            vec![
                ("DUNLIN_VAULT_SOCKET".into(), socket),
                ("DUNLIN_VAULT_TOKEN".into(), session.token.clone()),
            ],
        ));
        Some(session)
    }

    pub(crate) fn vault_read_only(&self) -> bool {
        self.vault_permission_mode(&self.session().id) == PermissionMode::ReadOnly
    }

    /// Resolve the effective mode for any session, including a parked runtime
    /// that is not the currently selected session. This mirrors the harness
    /// permission resolution without depending on UI-only state.
    fn vault_permission_mode(&self, session_id: &str) -> PermissionMode {
        if let Some(context) = self.group_contexts.get(session_id) {
            return context.permission_mode(
                &self.projects,
                &self.project_groups,
                self.permission_mode,
            );
        }
        let project = self
            .all_sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| {
                self.projects
                    .iter()
                    .find(|project| project.id == session.project_id)
            })
            .unwrap_or_else(|| self.project());
        self.resolved_project_settings(project)
            .map(|settings| settings.permission_mode.value)
            .unwrap_or(PermissionMode::ReadOnly)
    }

    /// Keep the live broker capability aligned with the effective project mode.
    pub(crate) fn spawn_vault_approvals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(receiver) = self.vault.approval_rx.take() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(job) = receiver.recv().await {
                let service::ApprovalJob { request, reply } = job;
                let (done_tx, done_rx) = async_channel::bounded(1);
                let _ = this.update_in(cx, |this, window, cx| {
                    let project = this
                        .projects
                        .iter()
                        .find(|project| project.id == request.project_id)
                        .map(|project| project.name.clone())
                        .unwrap_or_else(|| request.project_id.clone());
                    let session = request
                        .session_id
                        .as_deref()
                        .and_then(|id| this.all_sessions.iter().find(|session| session.id == id))
                        .map(|session| session.title.clone())
                        .unwrap_or_else(|| "not bound yet".into());
                    let text = format!(
                        "Project: {project}\nSession: {session}\n{} {}\n\nCredential: {} ({})\n\nThe agent will receive only the operation result, never the credential value.",
                        request.operation,
                        request.detail,
                        request.label,
                        request.alias
                    );
                    let prompt = window.prompt(
                        PromptLevel::Warning,
                        "Agent wants to use a credential",
                        Some(text.as_str()),
                        &[PromptButton::cancel("Deny"), PromptButton::new("Allow once"), PromptButton::new("Allow for this session")],
                        cx,
                    );
                    let prompt = this.overlay_backdrop.track_native(prompt, cx);
                    cx.spawn_in(window, async move |_, _cx| {
                        let choice = prompt.await.unwrap_or(0);
                        let decision = match choice {
                            1 => service::ApprovalDecision::Once,
                            2 => service::ApprovalDecision::Session,
                            _ => service::ApprovalDecision::Deny,
                        };
                        let _ = reply.send(decision);
                        let _ = done_tx.send(()).await;
                    })
                    .detach();
                });
                let _ = done_rx.recv().await;
            }
        })
        .detach();
    }

    pub(crate) fn vault_needs_harness_restart(&self) -> bool {
        self.vault.enabled
            && self.vault.service.is_some()
            && !self.project().is_remote()
            && self.harness.harness.is_some()
            && self.harness.vault_session.is_none()
    }

    pub(crate) fn sync_vault_read_only(&self) {
        if let Some(session) = &self.harness.vault_session {
            session.set_read_only(
                self.vault_permission_mode(&self.session().id) == PermissionMode::ReadOnly,
            );
        }
        for (session_id, runtime) in &self.parked_runtimes {
            if let Some(session) = &runtime.state.vault_session {
                session.set_read_only(
                    self.vault_permission_mode(session_id) == PermissionMode::ReadOnly,
                );
            }
        }
    }
}

fn policy_json(policy: &SecretPolicy) -> Result<String, String> {
    let json = serde_json::to_string(policy)
        .map_err(|error| format!("Could not encode the credential policy: {error}"))?;
    if json.len() > 64 * 1024 {
        return Err("The credential policy is too large (64 KiB maximum)".into());
    }
    Ok(json)
}

fn expiry_label(ttl_secs: i64) -> String {
    if ttl_secs >= 3600 {
        let hours = ttl_secs / 3600;
        if ttl_secs % 3600 == 0 {
            format!("in {hours} h")
        } else {
            format!("in {} min", ttl_secs / 60)
        }
    } else {
        format!("in {} min", (ttl_secs / 60).max(1))
    }
}

pub(crate) const TTL_OPTIONS: [(i64, &str); 3] =
    [(15 * 60, "15 min"), (60 * 60, "1 h"), (8 * 60 * 60, "8 h")];

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::vault::MAX_SECRET_TTL_SECS;

    #[test]
    fn slugs_are_public_safe_and_bounded() {
        assert_eq!(slug("GitHub (work)"), "github-work");
        assert_eq!(slug("  !!!  "), "key");
        assert_eq!(slug(&"a".repeat(120)).len(), 48);
    }

    #[test]
    fn hosts_reject_schemes_ports_private_names_and_wildcards() {
        assert_eq!(
            validate_hosts(&["API.GitHub.com.".into()]).unwrap(),
            vec!["api.github.com"]
        );
        for host in [
            "https://api.github.com",
            "api.github.com:443",
            "*.github.com",
            "127.0.0.1",
            "localhost",
            "api.local",
        ] {
            assert!(validate_hosts(&[host.into()]).is_err(), "{host}");
        }
    }

    #[test]
    fn path_prefix_and_header_are_bounded() {
        assert_eq!(validate_path_prefix("").unwrap(), "/");
        assert_eq!(validate_path_prefix("/repos/acme/").unwrap(), "/repos/acme");
        assert!(validate_path_prefix("repos").is_err());
        assert!(validate_path_prefix("/repos?x=1").is_err());
        assert!(validate_header("Authorization", "Bearer ").is_ok());
        assert!(validate_header("Host", "").is_err());
        assert!(validate_header("X-Test", "bad\r\nInjected").is_err());
    }

    #[test]
    fn ttl_options_are_within_the_bound() {
        assert_eq!(TTL_OPTIONS.map(|(ttl, _)| ttl), [900, 3600, 28800]);
        for (ttl, _) in TTL_OPTIONS {
            assert!(normalize_ttl(ttl).is_ok());
        }
        assert!(normalize_ttl(MAX_SECRET_TTL_SECS + 1).is_err());
    }
}
