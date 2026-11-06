//! Use-only credentials for agent sessions.
//!
//! A secret value lives in the macOS Keychain. Only its metadata is persisted in
//! SQLite. Agents learn a public alias and ask the local broker to perform an
//! HTTP request or a fixed CLI recipe; the value never enters the model context,
//! harness environment, command line or project files.

use serde::{Deserialize, Serialize};

/// Internal namespace marker for secrets created by Dunlin. The literal prefix
/// is never shown in the UI or to the model.
pub const SECRET_NAMESPACE: &str = "dunlin";
pub const DEFAULT_SECRET_TTL_SECS: i64 = 60 * 60;
pub const MAX_SECRET_TTL_SECS: i64 = 24 * 60 * 60;

/// How the stored value is attached to an outbound HTTP request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Injection {
    Header {
        name: String,
        prefix: String,
    },
    Basic {
        username: String,
    },
    Query {
        name: String,
    },
    /// Exchanges the stored refresh token at `token_url` and injects the
    /// resulting short-lived access token as a Bearer header.
    OAuth2 {
        token_url: String,
        client_id: String,
    },
    /// Signs the request with AWS Signature Version 4. The stored value is the
    /// secret access key; the access key ID, region and service are policy.
    AwsSigV4 {
        access_key_id: String,
        region: String,
        service: String,
    },
}

impl Injection {
    pub fn label(&self) -> String {
        match self {
            Self::Header { name, .. } => format!("{name} header"),
            Self::Basic { .. } => "HTTP Basic auth".into(),
            Self::Query { name } => format!("{name} query parameter"),
            Self::OAuth2 { token_url, .. } => format!("OAuth 2.0 refresh via {token_url}"),
            Self::AwsSigV4 {
                access_key_id,
                region,
                service,
            } => format!("AWS SigV4 · {access_key_id} · {region}/{service}"),
        }
    }
}

/// Security-critical metadata stored in the macOS Keychain, next to the value.
/// SQLite only caches these fields for display; a harness that edits the cache
/// cannot widen the policy the broker actually enforces.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SecretPolicy {
    #[serde(default = "default_policy_version")]
    pub version: u32,
    pub managed: bool,
    pub injection: Injection,
    pub allowed_hosts: Vec<String>,
    pub path_prefix: String,
    pub allowed_methods: Vec<HttpMethod>,
    pub allowed_headers: Vec<String>,
    pub auto_approve: bool,
    pub expires_at: Option<i64>,
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub recipes: Vec<Recipe>,
    #[serde(default)]
    pub operations: Vec<Operation>,
}

/// Named, pre-approved HTTP operation. The model supplies declared path
/// parameters only; the host, method and path template remain policy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub method: HttpMethod,
    pub host: String,
    pub path: String,
    #[serde(default)]
    pub parameters: Vec<String>,
}

impl Operation {
    pub fn validate(&self, policy: &SecretPolicy) -> Result<(), String> {
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err("An HTTP operation needs a lowercase name".into());
        }
        if self.description.len() > 1024 {
            return Err("An HTTP operation description is too large".into());
        }
        if !policy
            .allowed_hosts
            .iter()
            .any(|host| host.eq_ignore_ascii_case(&self.host))
        {
            return Err("An HTTP operation host must be in the credential allowlist".into());
        }
        if !policy.allowed_methods.contains(&self.method) {
            return Err("An HTTP operation method must be allowed by the credential".into());
        }
        if !self.path.starts_with('/')
            || self.path.len() > 2048
            || self.path.contains(['?', '#', '\\'])
            || self
                .path
                .chars()
                .any(|c| c == '\r' || c == '\n' || c == '\0')
        {
            return Err("An HTTP operation path is invalid".into());
        }
        if policy.path_prefix != "/" {
            let prefix = policy.path_prefix.trim_end_matches('/');
            if self.path != prefix && !self.path.starts_with(&format!("{prefix}/")) {
                return Err("An HTTP operation path is outside the credential prefix".into());
            }
        }
        if self
            .path
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
        {
            return Err("An HTTP operation path contains traversal".into());
        }
        if self.parameters.len() > 8 {
            return Err("An HTTP operation may declare at most 8 parameters".into());
        }
        let mut names = std::collections::HashSet::new();
        for parameter in &self.parameters {
            if parameter.is_empty()
                || parameter.len() > 48
                || !parameter
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || !names.insert(parameter.as_str())
            {
                return Err("An HTTP operation has an invalid parameter name".into());
            }
        }
        let mut rest = self.path.as_str();
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else {
                return Err("An HTTP operation has an unclosed path parameter".into());
            };
            if !names.contains(&after[..end]) {
                return Err("An HTTP operation uses an undeclared path parameter".into());
            }
            rest = &after[end + 1..];
        }
        if rest.contains('}') {
            return Err("An HTTP operation has an unmatched '}'".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OperationSummary {
    pub name: String,
    pub description: String,
    pub method: String,
    pub host: String,
    pub parameters: Vec<String>,
}

fn default_policy_version() -> u32 {
    1
}

impl SecretPolicy {
    /// A stored key has no destinations or grants until the user configures access.
    pub fn is_storage_only(&self) -> bool {
        self.allowed_hosts.is_empty()
            && self.allowed_methods.is_empty()
            && self.project_ids.is_empty()
            && self.recipes.is_empty()
            && self.operations.is_empty()
            && !self.auto_approve
    }

    #[allow(clippy::collapsible_match)]
    pub fn validate(&self) -> Result<(), String> {
        if self.version != default_policy_version() {
            return Err("Unsupported credential policy version".into());
        }
        let storage_only = self.is_storage_only();
        if (!storage_only && self.allowed_hosts.is_empty()) || self.allowed_hosts.len() > 16 {
            return Err("A credential policy needs 1–16 hostnames".into());
        }
        for host in &self.allowed_hosts {
            let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
            if host.is_empty()
                || host.len() > 253
                || host.contains(['/', ':', ' ', '*', '?', '#', '@'])
                || host.parse::<std::net::IpAddr>().is_ok()
                || host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".localdomain")
                || host.ends_with(".internal")
            {
                return Err("The credential policy contains an unsafe hostname".into());
            }
        }
        if !self.path_prefix.starts_with('/')
            || self.path_prefix.contains(['?', '#', '\\'])
            || self.path_prefix.contains(['\r', '\n', '\0'])
        {
            return Err("The credential policy has an unsafe path prefix".into());
        }
        if !storage_only && self.allowed_methods.is_empty() {
            return Err("A credential policy needs at least one HTTP method".into());
        }
        for header in &self.allowed_headers {
            if header.is_empty()
                || header.len() > 128
                || !header.bytes().all(|byte| {
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
                return Err("The credential policy has an unsafe allowed header".into());
            }
        }
        for operation in &self.operations {
            operation.validate(self)?;
        }
        match &self.injection {
            Injection::OAuth2 {
                token_url,
                client_id,
            } => {
                if !token_url.starts_with("https://")
                    || token_url.contains([' ', '\r', '\n', '\0'])
                    || token_url.len() > 2048
                    || client_id.trim().is_empty()
                    || client_id.len() > 512
                {
                    return Err("The OAuth 2.0 token URL/client ID is invalid".into());
                }
            }
            Injection::AwsSigV4 {
                access_key_id,
                region,
                service,
            } => {
                if access_key_id.len() < 8
                    || access_key_id.len() > 128
                    || !access_key_id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric())
                    || region.is_empty()
                    || !region
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    || service.is_empty()
                    || !service.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                {
                    return Err("The AWS SigV4 policy is invalid".into());
                }
            }
            _ => {}
        }
        if let Injection::Header { name, prefix } = &self.injection
            && (name.is_empty()
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
                || prefix.contains(['\r', '\n', '\0']))
        {
            return Err("The credential policy has an unsafe header injection".into());
        }
        if let Injection::Query { name } = &self.injection
            && (name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
        {
            return Err("The credential policy has an unsafe query injection".into());
        }
        for recipe in &self.recipes {
            recipe.validate()?;
        }
        Ok(())
    }

    pub fn summary(
        &self,
        alias: String,
        label: String,
        description: String,
        now: i64,
    ) -> SecretSummary {
        SecretSummary {
            alias,
            label,
            description,
            hosts: self.allowed_hosts.clone(),
            methods: self
                .allowed_methods
                .iter()
                .map(|method| method.as_str().to_string())
                .collect(),
            recipes: self
                .recipes
                .iter()
                .map(|recipe| RecipeSummary {
                    name: recipe.name.clone(),
                    description: recipe.description.clone(),
                    parameters: recipe
                        .parameters
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                })
                .collect(),
            expires_at: self.expires_at,
            expires_in_secs: self.expires_at.map(|expires_at| (expires_at - now).max(0)),
            expired: self.expires_at.is_some_and(|expires_at| expires_at <= now),
            auto_approve: self.managed && self.auto_approve,
            operations: self
                .operations
                .iter()
                .map(|operation| OperationSummary {
                    name: operation.name.clone(),
                    description: operation.description.clone(),
                    method: operation.method.as_str().to_string(),
                    host: operation.host.clone(),
                    parameters: operation.parameters.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub const ALL: [Self; 6] = [
        Self::Get,
        Self::Head,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
    ];

    pub const READ: [Self; 2] = [Self::Get, Self::Head];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    pub fn is_write(self) -> bool {
        !matches!(self, Self::Get | Self::Head)
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|method| method.as_str().eq_ignore_ascii_case(value))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Secret {
    /// Internal id. Contains the hidden `dunlin/` namespace for managed keys.
    pub id: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Public alias shown to the user and the agent.
    pub alias: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub injection: Injection,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default = "default_path_prefix")]
    pub path_prefix: String,
    #[serde(default)]
    pub allowed_methods: Vec<HttpMethod>,
    #[serde(default)]
    pub allowed_headers: Vec<String>,
    #[serde(default = "default_true")]
    pub auto_approve: bool,
    /// Unix seconds. `None` only for explicitly non-expiring keys.
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub last_used_at: Option<i64>,
}

impl Secret {
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }

    pub fn is_managed(&self) -> bool {
        self.namespace == SECRET_NAMESPACE
    }

    pub fn summary(&self, recipes: &[Recipe], now: i64) -> SecretSummary {
        SecretSummary {
            alias: self.alias.clone(),
            label: self.label.clone(),
            description: self.description.clone(),
            hosts: self.allowed_hosts.clone(),
            methods: self
                .allowed_methods
                .iter()
                .map(|method| method.as_str().to_string())
                .collect(),
            recipes: recipes
                .iter()
                .map(|recipe| RecipeSummary {
                    name: recipe.name.clone(),
                    description: recipe.description.clone(),
                    parameters: recipe
                        .parameters
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                })
                .collect(),
            expires_at: self.expires_at,
            expires_in_secs: self.expires_at.map(|expires_at| (expires_at - now).max(0)),
            expired: self.is_expired(now),
            auto_approve: self.auto_approve && self.is_managed(),
            operations: Vec::new(),
        }
    }

    pub fn policy(
        &self,
        project_ids: Vec<String>,
        recipes: Vec<Recipe>,
        operations: Vec<Operation>,
    ) -> SecretPolicy {
        SecretPolicy {
            version: 1,
            managed: self.is_managed(),
            injection: self.injection.clone(),
            allowed_hosts: self.allowed_hosts.clone(),
            path_prefix: self.path_prefix.clone(),
            allowed_methods: self.allowed_methods.clone(),
            allowed_headers: self.allowed_headers.clone(),
            auto_approve: self.auto_approve,
            expires_at: self.expires_at,
            project_ids,
            recipes,
            operations,
        }
    }

    /// Replace display metadata from the Keychain-authoritative policy.
    pub fn apply_policy(&mut self, policy: &SecretPolicy) {
        self.injection = policy.injection.clone();
        self.allowed_hosts = policy.allowed_hosts.clone();
        self.path_prefix = policy.path_prefix.clone();
        self.allowed_methods = policy.allowed_methods.clone();
        self.allowed_headers = policy.allowed_headers.clone();
        self.auto_approve = policy.auto_approve;
        self.expires_at = policy.expires_at;
    }
}

fn default_namespace() -> String {
    SECRET_NAMESPACE.into()
}

fn default_path_prefix() -> String {
    "/".into()
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantPolicy {
    /// Follow the secret's `auto_approve` flag (managed keys default to yes).
    Auto,
    /// Always ask the user before this project may use the secret.
    Ask,
    /// The project may not use the secret.
    Deny,
}

impl GrantPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim_matches('"') {
            "auto" => Self::Auto,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SecretGrant {
    pub secret_id: String,
    pub project_id: String,
    pub policy: GrantPolicy,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecipeParameter {
    pub name: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_parameter_max_len")]
    pub max_len: usize,
    #[serde(default)]
    pub allow_leading_dash: bool,
}

fn default_parameter_max_len() -> usize {
    512
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
    pub id: String,
    pub secret_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Canonical absolute path. Never resolved through the harness PATH.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_env_var")]
    pub env_var: String,
    #[serde(default)]
    pub parameters: Vec<RecipeParameter>,
    #[serde(default = "default_recipe_timeout")]
    pub timeout_secs: u64,
    /// Optional SHA-256 pin for scripts the agent could otherwise replace.
    #[serde(default)]
    pub program_hash: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

fn default_env_var() -> String {
    "DUNLIN_SECRET".into()
}

fn default_recipe_timeout() -> u64 {
    30
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SecretAudit {
    pub id: i64,
    pub secret_id: Option<String>,
    pub alias: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub operation: String,
    /// Method/host/path or recipe name/program. Never a query string, body or value.
    pub detail: String,
    pub decision: String,
    pub status: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
}

/// Metadata safe to expose to the model. Never contains the value or internal id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SecretSummary {
    pub alias: String,
    pub label: String,
    pub description: String,
    pub hosts: Vec<String>,
    pub methods: Vec<String>,
    pub recipes: Vec<RecipeSummary>,
    pub expires_at: Option<i64>,
    pub expires_in_secs: Option<i64>,
    pub expired: bool,
    pub auto_approve: bool,
    #[serde(default)]
    pub operations: Vec<OperationSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecipeSummary {
    pub name: String,
    pub description: String,
    pub parameters: Vec<String>,
}

impl Recipe {
    /// True when the argument template only references declared parameters.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 64 {
            return Err("Recipe name must be 1–64 characters".into());
        }
        if !self
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
        {
            return Err("Recipe name may contain lowercase letters, digits, '-' and '_'".into());
        }
        if !self.program.starts_with('/') {
            return Err("Recipe program must be an absolute path".into());
        }
        let basename = std::path::Path::new(&self.program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            basename.as_str(),
            "sh" | "bash"
                | "zsh"
                | "dash"
                | "ksh"
                | "fish"
                | "csh"
                | "tcsh"
                | "python"
                | "python3"
                | "node"
                | "ruby"
                | "perl"
                | "php"
                | "osascript"
                | "env"
                | "xargs"
        ) {
            return Err(
                "Shells and general-purpose interpreters cannot be recipe programs; use a fixed script or binary"
                    .into(),
            );
        }
        if self.env_var.is_empty()
            || !self.env_var.bytes().enumerate().all(|(index, b)| {
                (b.is_ascii_alphabetic() || b == b'_')
                    || (index > 0 && (b.is_ascii_alphanumeric() || b == b'_'))
            })
        {
            return Err("Recipe environment variable must be a valid name".into());
        }
        if self.timeout_secs == 0 || self.timeout_secs > 120 {
            return Err("Recipe timeout must be between 1 and 120 seconds".into());
        }
        if self.args.len() > 32 || self.args.iter().any(|arg| arg.len() > 4096) {
            return Err("Recipe arguments exceed the supported size".into());
        }
        let names = self
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<std::collections::HashSet<_>>();
        if names.len() != self.parameters.len() {
            return Err("Recipe parameters need unique names".into());
        }
        for parameter in &self.parameters {
            if parameter.name.is_empty()
                || parameter.name.len() > 48
                || !parameter
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err("Recipe parameter names may contain letters, digits and '_'".into());
            }
            if parameter.max_len == 0 || parameter.max_len > 4096 {
                return Err("Recipe parameter length is out of range".into());
            }
        }
        for arg in &self.args {
            let mut rest = arg.as_str();
            while let Some(start) = rest.find('{') {
                let after = &rest[start + 1..];
                let Some(end) = after.find('}') else {
                    return Err("Recipe argument has an unclosed parameter".into());
                };
                let name = &after[..end];
                if !names.contains(name) {
                    return Err(format!(
                        "Recipe argument uses undeclared parameter '{name}'"
                    ));
                }
                rest = &after[end + 1..];
            }
            if rest.contains('}') {
                return Err("Recipe argument has an unmatched '}'".into());
            }
        }
        Ok(())
    }

    /// Substitute declared parameters into the fixed argument template.
    pub fn render_args(
        &self,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<String>, String> {
        self.validate()?;
        for parameter in &self.parameters {
            let value = values.get(&parameter.name);
            if value.is_none() && parameter.required {
                return Err(format!("Missing recipe parameter '{}'", parameter.name));
            }
            if let Some(value) = value {
                if value.len() > parameter.max_len {
                    return Err(format!("Recipe parameter '{}' is too long", parameter.name));
                }
                if value.contains(['\0', '\r', '\n']) {
                    return Err(format!(
                        "Recipe parameter '{}' contains control characters",
                        parameter.name
                    ));
                }
                if value.starts_with('-') && !parameter.allow_leading_dash {
                    return Err(format!(
                        "Recipe parameter '{}' may not start with '-'",
                        parameter.name
                    ));
                }
            }
        }
        if values
            .keys()
            .any(|name| !self.parameters.iter().any(|p| &p.name == name))
        {
            return Err("Recipe received an undeclared parameter".into());
        }
        let mut args = Vec::with_capacity(self.args.len());
        for template in &self.args {
            let mut rendered = String::with_capacity(template.len());
            let mut rest = template.as_str();
            while let Some(start) = rest.find('{') {
                rendered.push_str(&rest[..start]);
                let after = &rest[start + 1..];
                let end = after
                    .find('}')
                    .ok_or_else(|| "Recipe argument has an unclosed parameter".to_string())?;
                let name = &after[..end];
                let value = values
                    .get(name)
                    .ok_or_else(|| format!("Missing recipe parameter '{name}'"))?;
                rendered.push_str(value);
                rest = &after[end + 1..];
            }
            rendered.push_str(rest);
            args.push(rendered);
        }
        Ok(args)
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn normalize_ttl(ttl_secs: i64) -> Result<i64, String> {
    if ttl_secs <= 0 || ttl_secs > MAX_SECRET_TTL_SECS {
        return Err(format!(
            "A quick key must expire within {} hours",
            MAX_SECRET_TTL_SECS / 3600
        ));
    }
    Ok(ttl_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn recipe() -> Recipe {
        Recipe {
            id: "r1".into(),
            secret_id: "s1".into(),
            name: "gh-api".into(),
            description: "Call the GitHub API with the work token".into(),
            program: "/opt/homebrew/bin/gh".into(),
            args: vec!["api".into(), "{path}".into()],
            env_var: "GH_TOKEN".into(),
            parameters: vec![RecipeParameter {
                name: "path".into(),
                required: true,
                max_len: 200,
                allow_leading_dash: false,
            }],
            timeout_secs: 30,
            program_hash: None,
            created_at: 0,
        }
    }

    #[test]
    fn recipe_templates_are_closed_and_validate_parameters() {
        let recipe = recipe();
        recipe.validate().unwrap();
        let mut values = BTreeMap::new();
        values.insert("path".into(), "/repos/acme/app".into());
        assert_eq!(
            recipe.render_args(&values).unwrap(),
            vec!["api", "/repos/acme/app"]
        );
        values.insert("path".into(), "--host=evil".into());
        assert!(recipe.render_args(&values).is_err());
        let mut bad = recipe.clone();
        bad.args = vec!["{other}".into()];
        assert!(bad.validate().is_err());
        let mut bad = recipe.clone();
        bad.args = vec!["{path".into()];
        assert!(bad.validate().is_err());
    }

    #[test]
    fn ttl_is_bounded_and_managed_summaries_hide_values() {
        assert!(normalize_ttl(DEFAULT_SECRET_TTL_SECS).is_ok());
        assert!(normalize_ttl(0).is_err());
        assert!(normalize_ttl(MAX_SECRET_TTL_SECS + 1).is_err());
        let secret = Secret {
            id: "dunlin/one".into(),
            namespace: SECRET_NAMESPACE.into(),
            alias: "github".into(),
            label: "GitHub".into(),
            description: "Use for api.github.com".into(),
            injection: Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            },
            allowed_hosts: vec!["api.github.com".into()],
            path_prefix: "/".into(),
            allowed_methods: vec![HttpMethod::Get],
            allowed_headers: vec![],
            auto_approve: true,
            expires_at: Some(now_secs() + 60),
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        };
        let summary = secret.summary(&[recipe()], now_secs());
        assert!(!summary.expired);
        assert!(summary.auto_approve);
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("token-value"));
        assert!(!json.contains("dunlin/one"));
    }
}
