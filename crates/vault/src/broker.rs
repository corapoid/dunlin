//! Policy and execution boundary for use-only credentials.

use std::path::PathBuf;
use std::time::Duration;

use dunlin_domain::vault::{
    HttpMethod, Recipe, Secret, SecretAudit, SecretPolicy, SecretSummary, now_secs,
};
use dunlin_storage::Store;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::access::LiveAccess;
use crate::http::{self, HttpExecutor, HttpInput, HttpOutput};
use crate::recipe::{RecipeExecutor, RecipeInput, RecipeOutput, RecipePlan};
use crate::store::SecretStore;

#[derive(Clone, Debug)]
pub struct SessionContext {
    pub project_id: String,
    pub project_path: PathBuf,
    pub session_id: Option<String>,
    pub trusted: bool,
    pub read_only: bool,
    pub allowed: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ApprovalRequest {
    pub alias: String,
    pub label: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub operation: String,
    pub detail: String,
    /// Secret metadata revision; a session-wide allowance is invalidated when
    /// the credential policy is edited.
    pub revision: i64,
}

pub trait Approver: Send + Sync {
    fn approve(&self, request: &ApprovalRequest) -> bool;
}

pub struct DenyAll;
impl Approver for DenyAll {
    fn approve(&self, _request: &ApprovalRequest) -> bool {
        false
    }
}

pub struct AutoApprove;
impl Approver for AutoApprove {
    fn approve(&self, _request: &ApprovalRequest) -> bool {
        true
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Failed(String),
    /// Dispatch may have happened; callers must not automatically retry.
    #[error("{0}")]
    OutcomeUnknown(String),
}

impl BrokerError {
    pub fn message(&self) -> &str {
        match self {
            Self::Denied(message) | Self::Failed(message) | Self::OutcomeUnknown(message) => {
                message
            }
        }
    }
}

pub struct Broker<'a> {
    pub store: &'a Store,
    pub secrets: &'a dyn SecretStore,
    pub http: &'a dyn HttpExecutor,
    pub recipes: &'a dyn RecipeExecutor,
    pub approver: &'a dyn Approver,
    pub session: &'a SessionContext,
}

impl<'a> Broker<'a> {
    pub fn list(&self) -> Result<Vec<SecretSummary>, BrokerError> {
        if !self.session.allowed || !self.session.trusted {
            return Ok(Vec::new());
        }
        let now = now_secs();
        let secrets = self
            .store
            .secrets()
            .map_err(|error| BrokerError::Failed(format!("Could not list credentials: {error}")))?;
        let mut summaries = Vec::new();
        for secret in secrets.into_iter().take(128) {
            let Ok(policy) = self.policy(&secret.id) else {
                continue;
            };
            if !policy.project_ids.contains(&self.session.project_id) {
                continue;
            }
            if policy
                .expires_at
                .is_some_and(|expires_at| expires_at <= now)
            {
                continue;
            }
            summaries.push(policy.summary(secret.alias, secret.label, secret.description, now));
        }
        Ok(summaries)
    }

    pub fn list_guarded(&self, access: &LiveAccess) -> Result<Vec<SecretSummary>, BrokerError> {
        check_live(Some(access), false)?;
        let result = self.list();
        check_live(Some(access), false)?;
        result
    }

    pub fn http(&self, input: HttpInput) -> Result<HttpOutput, BrokerError> {
        self.http_inner(input, None)
    }

    pub fn http_guarded(
        &self,
        input: HttpInput,
        access: &LiveAccess,
    ) -> Result<HttpOutput, BrokerError> {
        self.http_inner(input, Some(access))
    }

    fn http_inner(
        &self,
        input: HttpInput,
        access: Option<&LiveAccess>,
    ) -> Result<HttpOutput, BrokerError> {
        check_live(access, false)?;
        let now = now_secs();
        let detail = http_detail(&input);
        let (secret, policy) = self.lookup(&input.secret)?;
        self.authorize(&secret, &policy, "http", &detail, now)?;
        if !self.session.allowed {
            return Err(self.deny(&secret, "http", &detail, "session is stopped"));
        }
        if !self.session.trusted {
            return Err(self.deny(&secret, "http", &detail, "project is not trusted"));
        }
        let validated = match http::validate_input(&policy, &input) {
            Ok(validated) => validated,
            Err(error) => return Err(self.deny(&secret, "http", &detail, &error)),
        };
        if self.session.read_only && validated.method.is_write() {
            return Err(self.deny(
                &secret,
                "http",
                &detail,
                "read-only mode allows only GET and HEAD",
            ));
        }
        self.require_approval(&secret, &policy, "http", &detail)?;
        self.check_use(access, &secret, &policy, false)?;
        let value = match self.secrets.get(&secret.id) {
            Ok(value) => value,
            Err(error) => {
                self.record(&secret, "http", &detail, "error", None, Some(error.clone()));
                return Err(BrokerError::Failed(error));
            }
        };
        self.check_use(access, &secret, &policy, false)?;
        let mut dispatched = false;
        let injection_value: Zeroizing<String> = match &policy.injection {
            dunlin_domain::vault::Injection::OAuth2 {
                token_url,
                client_id,
            } => {
                self.check_use(access, &secret, &policy, false)?;
                dispatched = true;
                let result = http::refresh_oauth(self.http, token_url, client_id, value.as_str());
                self.check_use(access, &secret, &policy, true)?;
                match result {
                    Ok(token) => token,
                    Err(error) => {
                        let error = http::redact(&error, value.as_str(), None);
                        self.record(&secret, "http", &detail, "error", None, Some(error.clone()));
                        return Err(BrokerError::Failed(error));
                    }
                }
            }
            _ => Zeroizing::new(value.as_str().to_string()),
        };
        let mut plan = http::apply_injection(&policy, validated, injection_value.as_str());
        if matches!(
            &policy.injection,
            dunlin_domain::vault::Injection::AwsSigV4 { .. }
        ) && let Err(error) = http::sign_aws(&policy, &mut plan, value.as_str())
        {
            let error = http::redact(&error, value.as_str(), None);
            self.record(&secret, "http", &detail, "error", None, Some(error.clone()));
            return Err(BrokerError::Failed(error));
        }
        let basic_pair = match &policy.injection {
            dunlin_domain::vault::Injection::Basic { username } => {
                Some(format!("{username}:{}", value.as_str()))
            }
            _ => None,
        };
        let extra_secret = matches!(
            &policy.injection,
            dunlin_domain::vault::Injection::OAuth2 { .. }
        )
        .then_some(injection_value.as_str());
        self.check_use(access, &secret, &policy, dispatched)?;
        let result = self.http.execute(&plan);
        self.check_use(access, &secret, &policy, true)?;
        let mut output = match result {
            Ok(output) => output,
            Err(error) => {
                let error = http::redact(&error, value.as_str(), basic_pair.as_deref());
                let error = http::redact(&error, extra_secret.unwrap_or(""), None);
                self.record(&secret, "http", &detail, "error", None, Some(error.clone()));
                return Err(BrokerError::Failed(error));
            }
        };
        output.body = http::redact(&output.body, value.as_str(), basic_pair.as_deref());
        output.body = http::redact(&output.body, extra_secret.unwrap_or(""), None);
        if let Some(content_type) = &mut output.content_type {
            *content_type = http::redact(content_type, value.as_str(), basic_pair.as_deref());
            *content_type = http::redact(content_type, extra_secret.unwrap_or(""), None);
        }
        if output.truncated {
            output.body.push_str("\n[response truncated by Dunlin]");
        }
        self.record(
            &secret,
            "http",
            &detail,
            "allowed",
            Some(i64::from(output.status)),
            None,
        );
        let _ = self.store.touch_secret(&secret.id, now);
        self.check_use(access, &secret, &policy, true)?;
        Ok(output)
    }

    pub fn run(&self, input: RecipeInput) -> Result<RecipeOutput, BrokerError> {
        self.run_inner(input, None)
    }

    pub fn run_guarded(
        &self,
        input: RecipeInput,
        access: &LiveAccess,
    ) -> Result<RecipeOutput, BrokerError> {
        self.run_inner(input, Some(access))
    }

    fn run_inner(
        &self,
        input: RecipeInput,
        access: Option<&LiveAccess>,
    ) -> Result<RecipeOutput, BrokerError> {
        check_live(access, false)?;
        let now = now_secs();
        let (secret, policy) = self.lookup(&input.secret)?;
        let Some(recipe) = policy
            .recipes
            .iter()
            .find(|recipe| recipe.name == input.recipe)
            .cloned()
        else {
            return Err(self.deny(&secret, "recipe", &input.recipe, "recipe is not registered"));
        };
        let detail = recipe_detail(&recipe);
        self.authorize(&secret, &policy, "recipe", &detail, now)?;
        if !self.session.allowed {
            return Err(self.deny(&secret, "recipe", &detail, "session is stopped"));
        }
        if !self.session.trusted {
            return Err(self.deny(&secret, "recipe", &detail, "project is not trusted"));
        }
        if self.session.read_only {
            return Err(self.deny(
                &secret,
                "recipe",
                &detail,
                "read-only mode does not run CLI recipes",
            ));
        }
        if let Err(error) = recipe.validate() {
            return Err(self.deny(&secret, "recipe", &detail, &error));
        }
        let args = match recipe.render_args(&input.params) {
            Ok(args) => args,
            Err(error) => return Err(self.deny(&secret, "recipe", &detail, &error)),
        };
        self.require_approval(&secret, &policy, "recipe", &detail)?;
        self.check_use(access, &secret, &policy, false)?;
        let value = match self.secrets.get(&secret.id) {
            Ok(value) => value,
            Err(error) => {
                self.record(
                    &secret,
                    "recipe",
                    &detail,
                    "error",
                    None,
                    Some(error.clone()),
                );
                return Err(BrokerError::Failed(error));
            }
        };
        self.check_use(access, &secret, &policy, false)?;
        let plan = RecipePlan {
            program: recipe.program.clone(),
            args,
            env_var: recipe.env_var.clone(),
            secret: Zeroizing::new(value.as_str().to_string()),
            cwd: self.session.project_path.clone(),
            timeout: Duration::from_secs(recipe.timeout_secs),
            program_hash: recipe.program_hash.clone(),
        };
        self.check_use(access, &secret, &policy, false)?;
        let result = self.recipes.execute_guarded(&plan, access.cloned());
        self.check_use(access, &secret, &policy, true)?;
        let mut output = match result {
            Ok(output) => output,
            Err(error) => {
                let error = http::redact(&error, value.as_str(), None);
                self.record(
                    &secret,
                    "recipe",
                    &detail,
                    "error",
                    None,
                    Some(error.clone()),
                );
                return Err(BrokerError::Failed(error));
            }
        };
        output.stdout = http::redact(&output.stdout, value.as_str(), None);
        output.stderr = http::redact(&output.stderr, value.as_str(), None);
        if output.truncated {
            output.stdout.push_str("\n[output truncated by Dunlin]");
            output.stderr.push_str("\n[output truncated by Dunlin]");
        }
        let status = output.exit_code.map(i64::from);
        let decision = if output.timed_out {
            "timeout"
        } else if output.exit_code == Some(0) {
            "allowed"
        } else {
            "failed"
        };
        self.record(
            &secret,
            "recipe",
            &detail,
            decision,
            status,
            (!output.timed_out && output.exit_code.is_none())
                .then(|| "The recipe ended without an exit code".to_string()),
        );
        let _ = self.store.touch_secret(&secret.id, now);
        self.check_use(access, &secret, &policy, true)?;
        Ok(output)
    }

    /// Re-read protected policy after blocking boundaries. The SQL cache cannot
    /// widen it; a changed or missing credential invalidates this request.
    fn check_use(
        &self,
        access: Option<&LiveAccess>,
        secret: &Secret,
        policy: &SecretPolicy,
        dispatched: bool,
    ) -> Result<(), BrokerError> {
        let result = (|| {
            check_live(access, dispatched)?;
            let Some(access) = access else {
                return Ok(());
            };
            let current = self.store.secret_by_alias(&secret.alias).ok().flatten();
            let current_policy = self.policy(&secret.id).ok();
            let valid = current.is_some_and(|current| {
                current.id == secret.id && current.updated_at == secret.updated_at
            }) && current_policy.as_ref() == Some(policy)
                && policy.expires_at.is_none_or(|expiry| expiry > now_secs());
            if !valid {
                access.revoke();
            }
            check_live(Some(access), dispatched)
        })();
        if let Err(error) = &result {
            self.record(
                secret,
                "authority",
                "credential-use",
                if dispatched { "unknown" } else { "denied" },
                None,
                Some(error.message().into()),
            );
        }
        result
    }

    fn lookup(&self, alias: &str) -> Result<(Secret, SecretPolicy), BrokerError> {
        let secret = match self.store.secret_by_alias(alias) {
            Ok(Some(secret)) => secret,
            Ok(None) => {
                return Err(BrokerError::Denied(
                    "That credential is not available in this project".into(),
                ));
            }
            Err(error) => {
                return Err(BrokerError::Failed(format!(
                    "Could not load the credential: {error}"
                )));
            }
        };
        let policy = self.policy(&secret.id)?;
        Ok((secret, policy))
    }

    fn policy(&self, id: &str) -> Result<SecretPolicy, BrokerError> {
        let raw = self.secrets.get_policy(id).map_err(|error| {
            BrokerError::Denied(format!("The credential policy is unavailable: {error}"))
        })?;
        let policy: SecretPolicy = serde_json::from_str(&raw)
            .map_err(|_| BrokerError::Denied("The credential policy is unreadable".into()))?;
        policy.validate().map_err(|error| {
            BrokerError::Denied(format!("The credential policy is invalid: {error}"))
        })?;
        Ok(policy)
    }

    fn authorize(
        &self,
        secret: &Secret,
        policy: &SecretPolicy,
        operation: &str,
        detail: &str,
        now: i64,
    ) -> Result<(), BrokerError> {
        if policy
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            return Err(self.deny(secret, operation, detail, "credential has expired"));
        }
        if !policy.project_ids.contains(&self.session.project_id) {
            return Err(self.deny(
                secret,
                operation,
                detail,
                "credential is not granted to this project",
            ));
        }
        Ok(())
    }

    fn require_approval(
        &self,
        secret: &Secret,
        policy: &SecretPolicy,
        operation: &str,
        detail: &str,
    ) -> Result<(), BrokerError> {
        if !needs_approval(policy) {
            return Ok(());
        }
        let request = ApprovalRequest {
            alias: secret.alias.clone(),
            label: secret.label.clone(),
            project_id: self.session.project_id.clone(),
            session_id: self.session.session_id.clone(),
            operation: operation.into(),
            detail: detail.into(),
            revision: secret.updated_at,
        };
        if self.approver.approve(&request) {
            Ok(())
        } else {
            self.record(
                secret,
                operation,
                detail,
                "denied",
                None,
                Some("The user denied the credential use".into()),
            );
            Err(BrokerError::Denied(
                "The user denied this credential use".into(),
            ))
        }
    }

    fn deny(&self, secret: &Secret, operation: &str, detail: &str, reason: &str) -> BrokerError {
        self.record(
            secret,
            operation,
            detail,
            "denied",
            None,
            Some(reason.into()),
        );
        BrokerError::Denied(reason.into())
    }

    fn record(
        &self,
        secret: &Secret,
        operation: &str,
        detail: &str,
        decision: &str,
        status: Option<i64>,
        error: Option<String>,
    ) {
        let _ = self.store.record_secret_audit(&SecretAudit {
            id: 0,
            secret_id: Some(secret.id.clone()),
            alias: secret.alias.clone(),
            project_id: Some(self.session.project_id.clone()),
            session_id: self.session.session_id.clone(),
            operation: operation.into(),
            detail: detail.into(),
            decision: decision.into(),
            status,
            error,
            created_at: now_secs(),
        });
    }
}

pub fn needs_approval(policy: &SecretPolicy) -> bool {
    !policy.managed || !policy.auto_approve
}

fn check_live(access: Option<&LiveAccess>, dispatched: bool) -> Result<(), BrokerError> {
    if access.is_none_or(LiveAccess::allowed) {
        return Ok(());
    }
    if dispatched {
        Err(BrokerError::OutcomeUnknown("Credential authority was revoked after dispatch; outcome unknown, do not retry automatically".into()))
    } else {
        Err(BrokerError::Denied(
            "Credential authority is no longer valid".into(),
        ))
    }
}

fn http_detail(input: &HttpInput) -> String {
    if let Some(operation) = &input.operation {
        return format!("operation {operation}");
    }
    let method = HttpMethod::parse(&input.method)
        .map(HttpMethod::as_str)
        .unwrap_or("HTTP");
    match url::Url::parse(&input.url) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("invalid-host");
            format!("{method} {host}{}", url.path())
        }
        Err(_) => format!("{method} invalid-url"),
    }
}

fn recipe_detail(recipe: &Recipe) -> String {
    let program = std::path::Path::new(&recipe.program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("program");
    format!("{} {program}", recipe.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use dunlin_domain::vault::{Injection, RecipeParameter, now_secs};
    use dunlin_storage::Store;

    use crate::http::{HttpOutput, HttpPlan};
    use crate::recipe::{RecipeOutput, RecipePlan};
    use crate::store::MemoryStore;

    struct CountingStore {
        inner: MemoryStore,
        value_reads: AtomicUsize,
        revoke_on_get: Option<Arc<AtomicBool>>,
    }

    impl CountingStore {
        fn new() -> Self {
            Self {
                inner: MemoryStore::new(),
                value_reads: AtomicUsize::new(0),
                revoke_on_get: None,
            }
        }
        fn reads(&self) -> usize {
            self.value_reads.load(Ordering::Relaxed)
        }
        fn install(&self, secret: &Secret, policy: &SecretPolicy, value: &str) {
            self.set(&secret.id, value).unwrap();
            self.set_policy(&secret.id, &serde_json::to_string(policy).unwrap())
                .unwrap();
        }
    }

    impl SecretStore for CountingStore {
        fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
            self.value_reads.fetch_add(1, Ordering::Relaxed);
            if let Some(granted) = &self.revoke_on_get {
                granted.store(false, Ordering::Release);
            }
            self.inner.get(id)
        }
        fn set(&self, id: &str, value: &str) -> Result<(), String> {
            self.inner.set(id, value)
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.inner.delete(id)
        }
        fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
            self.inner.get_policy(id)
        }
        fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
            self.inner.set_policy(id, value)
        }
        fn delete_policy(&self, id: &str) -> Result<(), String> {
            self.inner.delete_policy(id)
        }
    }

    struct FakeHttp {
        plan: Mutex<Option<HttpPlan>>,
        body: String,
    }

    impl FakeHttp {
        fn new(body: impl Into<String>) -> Self {
            Self {
                plan: Mutex::new(None),
                body: body.into(),
            }
        }
    }

    impl HttpExecutor for FakeHttp {
        fn execute(&self, plan: &HttpPlan) -> Result<HttpOutput, String> {
            *self.plan.lock().unwrap() = Some(plan.clone());
            Ok(HttpOutput {
                status: 200,
                content_type: Some("application/json".into()),
                body: self.body.clone(),
                truncated: false,
            })
        }
    }

    struct FakeRecipe;

    impl RecipeExecutor for FakeRecipe {
        fn execute(&self, plan: &RecipePlan) -> Result<RecipeOutput, String> {
            Ok(RecipeOutput {
                exit_code: Some(0),
                signal: None,
                stdout: format!("value={}", plan.secret.as_str()),
                stderr: String::new(),
                truncated: false,
                timed_out: false,
            })
        }
    }

    struct AlwaysFailHttp;
    impl HttpExecutor for AlwaysFailHttp {
        fn execute(&self, _plan: &HttpPlan) -> Result<HttpOutput, String> {
            Err("fixture failed".into())
        }
    }

    fn secret(expires_at: Option<i64>) -> Secret {
        Secret {
            id: "dunlin/one".into(),
            namespace: "dunlin".into(),
            alias: "github-work".into(),
            label: "GitHub (work)".into(),
            description: "Use for api.github.com".into(),
            injection: Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            },
            allowed_hosts: vec!["api.github.com".into()],
            path_prefix: "/".into(),
            allowed_methods: vec![HttpMethod::Get, HttpMethod::Post],
            allowed_headers: vec!["Accept".into()],
            auto_approve: true,
            expires_at,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        }
    }

    fn context() -> SessionContext {
        SessionContext {
            project_id: "project".into(),
            project_path: std::env::temp_dir(),
            session_id: Some("session".into()),
            trusted: true,
            read_only: false,
            allowed: true,
        }
    }

    fn prepared() -> (Store, Secret, SecretPolicy) {
        let store = Store::open_in_memory().unwrap();
        let secret = secret(Some(now_secs() + 600));
        let policy = secret.policy(vec!["project".into()], Vec::new(), Vec::new());
        store.upsert_secret(&secret).unwrap();
        (store, secret, policy)
    }

    fn http_input() -> HttpInput {
        HttpInput {
            secret: "github-work".into(),
            method: "GET".into(),
            url: "https://api.github.com/user".into(),
            headers: BTreeMap::new(),
            body: None,
            operation: None,
            params: BTreeMap::new(),
        }
    }

    fn recipe() -> Recipe {
        Recipe {
            id: "r1".into(),
            secret_id: "dunlin/one".into(),
            name: "gh".into(),
            description: "GitHub CLI".into(),
            program: "/opt/homebrew/bin/gh".into(),
            args: vec!["api".into(), "{path}".into()],
            env_var: "GH_TOKEN".into(),
            parameters: vec![RecipeParameter {
                name: "path".into(),
                required: true,
                max_len: 100,
                allow_leading_dash: false,
            }],
            timeout_secs: 30,
            program_hash: None,
            created_at: 0,
        }
    }

    struct FnApprover<F>(F);
    impl<F: Fn(&ApprovalRequest) -> bool + Send + Sync> Approver for FnApprover<F> {
        fn approve(&self, request: &ApprovalRequest) -> bool {
            (self.0)(request)
        }
    }

    fn live(granted: &Arc<AtomicBool>) -> LiveAccess {
        let granted = granted.clone();
        LiveAccess::new(Instant::now() + Duration::from_secs(30), move || {
            granted.load(Ordering::Acquire)
        })
    }

    #[test]
    fn guarded_http_rechecks_after_approval_before_reading_value() {
        let (store, secret, mut policy) = prepared();
        policy.auto_approve = false;
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        let granted = Arc::new(AtomicBool::new(true));
        let access = live(&granted);
        let approver = FnApprover(|_: &ApprovalRequest| {
            granted.store(false, Ordering::Release);
            true
        });
        let http = FakeHttp::new("must not run");
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &approver,
            session: &context,
        };
        assert!(matches!(
            broker.http_guarded(http_input(), &access),
            Err(BrokerError::Denied(_))
        ));
        assert_eq!(secrets.reads(), 0);
        assert!(http.plan.lock().unwrap().is_none());
        granted.store(true, Ordering::Release);
        assert!(broker.list_guarded(&access).is_err());
    }

    #[test]
    fn guarded_http_rejects_policy_change_during_approval() {
        let (store, secret, mut policy) = prepared();
        policy.auto_approve = false;
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        let mut changed = policy.clone();
        changed.allowed_methods = vec![HttpMethod::Get];
        let approver = FnApprover(|_: &ApprovalRequest| {
            secrets
                .set_policy(&secret.id, &serde_json::to_string(&changed).unwrap())
                .unwrap();
            true
        });
        let http = FakeHttp::new("");
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &approver,
            session: &context,
        };
        let access = live(&Arc::new(AtomicBool::new(true)));
        assert!(matches!(
            broker.http_guarded(http_input(), &access),
            Err(BrokerError::Denied(_))
        ));
        assert_eq!(secrets.reads(), 0);
        assert!(http.plan.lock().unwrap().is_none());
    }

    #[test]
    fn guarded_http_rechecks_after_secret_store_before_dispatch() {
        let (store, secret, policy) = prepared();
        let granted = Arc::new(AtomicBool::new(true));
        let mut secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        secrets.revoke_on_get = Some(granted.clone());
        let http = FakeHttp::new("");
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(matches!(
            broker.http_guarded(http_input(), &live(&granted)),
            Err(BrokerError::Denied(_))
        ));
        assert_eq!(secrets.reads(), 1);
        assert!(http.plan.lock().unwrap().is_none());
    }

    struct RevokeHttp {
        granted: Arc<AtomicBool>,
        calls: AtomicUsize,
        fail: bool,
    }
    impl HttpExecutor for RevokeHttp {
        fn execute(&self, _: &HttpPlan) -> Result<HttpOutput, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.granted.store(false, Ordering::Release);
            if self.fail {
                return Err("private-token failure".into());
            }
            Ok(HttpOutput {
                status: 200,
                content_type: None,
                body: "private-token response".into(),
                truncated: false,
            })
        }
    }

    #[test]
    fn guarded_http_discards_success_and_error_after_revocation_without_retry() {
        for fail in [false, true] {
            let (store, secret, policy) = prepared();
            let secrets = CountingStore::new();
            secrets.install(&secret, &policy, "private-token");
            let granted = Arc::new(AtomicBool::new(true));
            let http = RevokeHttp {
                granted: granted.clone(),
                calls: AtomicUsize::new(0),
                fail,
            };
            let context = context();
            let broker = Broker {
                store: &store,
                secrets: &secrets,
                http: &http,
                recipes: &FakeRecipe,
                approver: &AutoApprove,
                session: &context,
            };
            let error = broker
                .http_guarded(http_input(), &live(&granted))
                .unwrap_err();
            assert!(matches!(error, BrokerError::OutcomeUnknown(_)));
            assert!(!error.message().contains("private-token"));
            assert_eq!(http.calls.load(Ordering::Relaxed), 1);
            let audit = store.secret_audit(Some(&secret.id), 10).unwrap();
            assert_eq!(audit.len(), 1);
            assert_eq!(audit[0].decision, "unknown");
            assert!(
                !serde_json::to_string(&audit)
                    .unwrap()
                    .contains("private-token")
            );
        }
    }

    #[test]
    fn guarded_oauth_stops_after_refresh_if_authority_is_revoked() {
        let (store, secret, mut policy) = prepared();
        policy.injection = Injection::OAuth2 {
            token_url: "https://api.github.com/token".into(),
            client_id: "fixture".into(),
        };
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-refresh-token");
        let granted = Arc::new(AtomicBool::new(true));
        let http = RevokeHttp {
            granted: granted.clone(),
            calls: AtomicUsize::new(0),
            fail: true,
        };
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(matches!(
            broker.http_guarded(http_input(), &live(&granted)),
            Err(BrokerError::OutcomeUnknown(_))
        ));
        assert_eq!(http.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn guarded_recipe_rechecks_approval_before_reading_secret() {
        let (store, secret, mut policy) = prepared();
        policy.recipes = vec![recipe()];
        policy.auto_approve = false;
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        let granted = Arc::new(AtomicBool::new(true));
        let approver = FnApprover(|_: &ApprovalRequest| {
            granted.store(false, Ordering::Release);
            true
        });
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &AlwaysFailHttp,
            recipes: &FakeRecipe,
            approver: &approver,
            session: &context,
        };
        let input = RecipeInput {
            secret: secret.alias,
            recipe: "gh".into(),
            params: BTreeMap::from([("path".into(), "user".into())]),
        };
        assert!(matches!(
            broker.run_guarded(input, &live(&granted)),
            Err(BrokerError::Denied(_))
        ));
        assert_eq!(secrets.reads(), 0);
    }

    struct PolicyChangingHttp<'a>(&'a CountingStore, &'a Secret, &'a SecretPolicy);
    impl HttpExecutor for PolicyChangingHttp<'_> {
        fn execute(&self, _: &HttpPlan) -> Result<HttpOutput, String> {
            let mut policy = self.2.clone();
            policy.project_ids.clear();
            self.0
                .set_policy(&self.1.id, &serde_json::to_string(&policy).unwrap())
                .unwrap();
            Ok(HttpOutput {
                status: 200,
                content_type: None,
                body: "must not be returned".into(),
                truncated: false,
            })
        }
    }

    #[test]
    fn guarded_http_discards_response_when_protected_policy_is_revoked() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        let http = PolicyChangingHttp(&secrets, &secret, &policy);
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(matches!(
            broker.http_guarded(http_input(), &live(&Arc::new(AtomicBool::new(true)))),
            Err(BrokerError::OutcomeUnknown(_))
        ));
        assert_eq!(
            store.secret_audit(Some(&secret.id), 10).unwrap()[0].decision,
            "unknown"
        );
    }

    #[test]
    fn guarded_http_success_still_redacts_credentials() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "private-token");
        let http = FakeHttp::new("private-token response");
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &FakeRecipe,
            approver: &AutoApprove,
            session: &context,
        };
        let output = broker
            .http_guarded(http_input(), &live(&Arc::new(AtomicBool::new(true))))
            .unwrap();
        assert!(!output.body.contains("private-token"));
        assert!(http.plan.lock().unwrap().is_some());
    }

    #[test]
    fn list_uses_the_keychain_policy_and_hides_values_and_ids() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "tok_value-123");
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        let summaries = broker.list().unwrap();
        assert_eq!(summaries.len(), 1);
        let json = serde_json::to_string(&summaries).unwrap();
        assert!(!json.contains("tok_value-123"));
        assert!(!json.contains("dunlin/one"));
        assert!(!json.contains("Authorization"));

        let mut expired = policy.clone();
        expired.expires_at = Some(now_secs() - 1);
        secrets
            .set_policy(&secret.id, &serde_json::to_string(&expired).unwrap())
            .unwrap();
        assert!(broker.list().unwrap().is_empty());

        let mut ungranted = policy;
        ungranted.project_ids.clear();
        secrets
            .set_policy(&secret.id, &serde_json::to_string(&ungranted).unwrap())
            .unwrap();
        assert!(broker.list().unwrap().is_empty());
    }

    #[test]
    fn stored_only_keys_are_valid_but_unlisted_and_unusable_without_reading_the_value() {
        let (store, secret, mut policy) = prepared();
        policy.allowed_hosts.clear();
        policy.allowed_methods.clear();
        policy.project_ids.clear();
        policy.auto_approve = false;
        policy.expires_at = None;
        assert!(policy.is_storage_only());
        assert!(policy.validate().is_ok());
        let json = serde_json::to_string(&policy).unwrap();
        assert!(
            serde_json::from_str::<SecretPolicy>(&json)
                .unwrap()
                .is_storage_only()
        );
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "fixture-key");
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(broker.list().unwrap().is_empty());
        assert!(matches!(
            broker.http(http_input()),
            Err(BrokerError::Denied(_))
        ));
        assert_eq!(secrets.reads(), 0);
        assert!(http.plan.lock().unwrap().is_none());
        policy.project_ids.push("project".into());
        assert!(
            policy.validate().is_err(),
            "stored-only keys cannot acquire grants without a valid destination policy"
        );
        policy.project_ids.clear();
        policy.auto_approve = true;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn managed_auto_approve_injects_and_redacts_the_value() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "tok_value-123");
        let http = FakeHttp::new(r#"{"echo":"tok_value-123"}"#);
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &DenyAll,
            session: &context,
        };
        let output = broker.http(http_input()).unwrap();
        assert!(output.body.contains("[REDACTED]"));
        assert!(!output.body.contains("tok_value-123"));
        let plan = http.plan.lock().unwrap().clone().unwrap();
        assert!(
            plan.headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "Bearer tok_value-123")
        );
        assert_eq!(secrets.reads(), 1);
        assert_eq!(store.secret_audit(Some(&secret.id), 10).unwrap().len(), 1);
    }

    #[test]
    fn denied_untrusted_expired_read_only_and_ask_never_read_the_key() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "tok_value-123");
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;

        let mut bad_context = context();
        bad_context.trusted = false;
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &bad_context,
        };
        assert!(broker.http(http_input()).is_err());
        assert_eq!(secrets.reads(), 0);

        let mut ask_policy = policy.clone();
        ask_policy.auto_approve = false;
        secrets
            .set_policy(&secret.id, &serde_json::to_string(&ask_policy).unwrap())
            .unwrap();
        let ask_context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &DenyAll,
            session: &ask_context,
        };
        assert!(broker.http(http_input()).is_err());
        assert_eq!(secrets.reads(), 0);

        let mut auto_policy = policy.clone();
        auto_policy.auto_approve = true;
        secrets
            .set_policy(&secret.id, &serde_json::to_string(&auto_policy).unwrap())
            .unwrap();
        let mut missing = http_input();
        missing.method = "POST".into();
        missing.body = Some("{}".into());
        let mut read_only = context();
        read_only.read_only = true;
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &read_only,
        };
        assert!(broker.http(missing).is_err());
        assert_eq!(secrets.reads(), 0);

        let mut expired = policy;
        expired.expires_at = Some(now_secs() - 1);
        secrets
            .set_policy(&secret.id, &serde_json::to_string(&expired).unwrap())
            .unwrap();
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(broker.http(http_input()).is_err());
        assert_eq!(secrets.reads(), 0);
    }

    #[test]
    fn sqlite_tampering_cannot_widen_the_keychain_policy() {
        let (store, secret, policy) = prepared();
        let secrets = CountingStore::new();
        let mut narrowed = policy;
        narrowed.expires_at = Some(now_secs() - 1);
        narrowed.project_ids = vec!["project".into()];
        secrets.install(&secret, &narrowed, "tok_value-123");
        // Simulate a harness editing the display cache to add a hostile host,
        // remove expiry and enable auto-approval.
        let mut tampered = secret;
        tampered.allowed_hosts = vec!["evil.test".into()];
        tampered.expires_at = None;
        tampered.auto_approve = true;
        store.upsert_secret(&tampered).unwrap();
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        let mut hostile = http_input();
        hostile.url = "https://evil.test/collect".into();
        assert!(broker.http(hostile).is_err());
        assert!(broker.http(http_input()).is_err());
        assert_eq!(secrets.reads(), 0);
    }

    #[test]
    fn missing_policy_denies_without_reading_the_value() {
        let (store, secret, _policy) = prepared();
        let secrets = CountingStore::new();
        secrets.set(&secret.id, "tok_value-123").unwrap();
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(broker.http(http_input()).is_err());
        assert_eq!(secrets.reads(), 0);
    }

    #[test]
    fn http_failures_and_recipe_output_are_redacted() {
        let (store, secret, mut policy) = prepared();
        policy.recipes.push(recipe());
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "tok_value-123");
        let http = AlwaysFailHttp;
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        assert!(broker.http(http_input()).is_err());
        assert!(
            !serde_json::to_string(&store.secret_audit(Some(&secret.id), 10).unwrap())
                .unwrap()
                .contains("tok_value-123")
        );
        let output = broker
            .run(RecipeInput {
                secret: "github-work".into(),
                recipe: "gh".into(),
                params: BTreeMap::from([("path".into(), "user".into())]),
            })
            .unwrap();
        assert!(!output.stdout.contains("tok_value-123"));
        assert!(output.stdout.contains("[REDACTED]"));
    }

    #[test]
    fn recipe_requires_a_keychain_recipe_and_declared_parameters() {
        let (store, secret, mut policy) = prepared();
        policy.recipes.push(recipe());
        let secrets = CountingStore::new();
        secrets.install(&secret, &policy, "tok_value-123");
        let http = FakeHttp::new("");
        let recipes = FakeRecipe;
        let context = context();
        let broker = Broker {
            store: &store,
            secrets: &secrets,
            http: &http,
            recipes: &recipes,
            approver: &AutoApprove,
            session: &context,
        };
        let missing_param = broker.run(RecipeInput {
            secret: "github-work".into(),
            recipe: "gh".into(),
            params: BTreeMap::new(),
        });
        assert!(missing_param.is_err());
        assert_eq!(secrets.reads(), 0);
        let missing_recipe = broker.run(RecipeInput {
            secret: "github-work".into(),
            recipe: "missing".into(),
            params: BTreeMap::new(),
        });
        assert!(missing_recipe.is_err());
        assert_eq!(secrets.reads(), 0);
        let output = broker
            .run(RecipeInput {
                secret: "github-work".into(),
                recipe: "gh".into(),
                params: BTreeMap::from([("path".into(), "user".into())]),
            })
            .unwrap();
        assert!(output.stdout.contains("[REDACTED]"));
        assert_eq!(secrets.reads(), 1);
    }
}
