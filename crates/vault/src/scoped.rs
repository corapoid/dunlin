//! Immutable scoped credentials. The host journal owns grants and publication;
//! keyring records bind policy and value in one envelope. No API returns values
//! to callers, and no transport message can deserialize a permit.
use crate::access::LiveAccess;
use crate::http::{self, HttpExecutor, HttpInput, HttpOutput};
use crate::store::{SecretStore, ServiceKeychainStore};
use dunlin_domain::collaboration::PrincipalKind;
use dunlin_domain::vault::{Injection, SecretPolicy, now_secs};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;
use zeroize::{Zeroize, Zeroizing};

const MAX_RECORD: usize = 65536;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Invalid scoped credential or operation")]
    Invalid,
    #[error("Scoped credential authority is no longer valid")]
    Denied,
    #[error("Scoped credential journal conflict")]
    Conflict,
    #[error("Scoped credential storage is unavailable")]
    Storage,
    #[error("Scoped credential journal capacity reached")]
    Limit,
    #[error("Credential write outcome unknown; reserved reference must be retained")]
    WriteUnknown,
    #[error("Credential operation outcome unknown; do not retry automatically")]
    OutcomeUnknown,
    #[error("Credential operation failed")]
    Failed,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub workspace: String,
    pub project: String,
    pub host: String,
}
impl Binding {
    fn validate(&self) -> Result<()> {
        if [&self.workspace, &self.project, &self.host]
            .into_iter()
            .all(|v| token(v))
        {
            Ok(())
        } else {
            Err(Error::Invalid)
        }
    }
}
fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

/// No secrets. The journal must reserve this exact candidate before keyring I/O.
#[derive(Clone)]
pub struct Candidate {
    pub credential: String,
    pub binding: Binding,
    pub expected_revision: u64,
    pub reference: String,
    pub sha256: String,
}
/// Host-only, non-cloneable claim for exactly one attempted keyring write.
pub struct WriteClaim {
    candidate: Candidate,
    revision: u64,
}
impl WriteClaim {
    /// Called by a journal after its durable reservation/one-use claim succeeds.
    pub fn reserved(candidate: Candidate, revision: u64) -> Result<Self> {
        if revision
            != candidate
                .expected_revision
                .checked_add(1)
                .ok_or(Error::Invalid)?
            || revision > 9_007_199_254_740_991
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            candidate,
            revision,
        })
    }
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Only call after durable write acknowledgement and fresh activation CAS.
    pub fn activated(self) -> Version {
        Version {
            credential: self.candidate.credential,
            binding: self.candidate.binding,
            revision: self.revision,
            reference: self.candidate.reference,
            sha256: self.candidate.sha256,
        }
    }
}
/// Resolved from trusted host storage, never accepted as a runner authorization.
#[derive(Clone, PartialEq, Eq)]
pub struct Version {
    pub credential: String,
    pub binding: Binding,
    pub revision: u64,
    pub reference: String,
    pub sha256: String,
}

pub trait Journal: Send + Sync {
    /// Atomic expected-revision CAS and unique-reference reservation, with a
    /// durable one-use writer claim. Keep pending/unknown references forever
    /// until a separate proven-safe cleanup resolves them. Never reuse a slot.
    fn reserve(&self, candidate: &Candidate) -> Result<WriteClaim>;
    /// A successful keyring return only. Must preserve the exact claim/digest.
    fn acknowledge(&self, claim: &WriteClaim) -> Result<()>;
    /// Fresh grant check and CAS; an unacknowledged/late writer cannot activate.
    fn activate(&self, claim: WriteClaim) -> Result<Version>;
}

/// Complete live identity supplied by the host runtime. No Deserialize/Serialize.
#[derive(Clone, PartialEq, Eq)]
pub struct UseScope {
    pub principal_kind: PrincipalKind,
    pub actor: String,
    pub binding: Binding,
    pub session: String,
    pub turn: String,
    pub request: String,
    pub profile_digest: String,
    pub lease_instance: String,
    pub lease_epoch: u64,
    pub grant_revision: u64,
    pub read_only: bool,
}
impl UseScope {
    fn validate(&self) -> Result<()> {
        self.binding.validate()?;
        if ![
            &self.actor,
            &self.session,
            &self.turn,
            &self.request,
            &self.lease_instance,
        ]
        .into_iter()
        .all(|v| token(v))
            || self.lease_epoch == 0
            || self.grant_revision == 0
            || self.profile_digest.len() != 64
            || !self
                .profile_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Complete,
    Failed,
    Unknown,
}

pub trait Authority: Send + Sync {
    /// Must re-read active credential generation, principal/membership/grant,
    /// host, profile, turn and live lease. Reject replacement/ABA revisions.
    fn check(&self, scope: &UseScope, version: &Version) -> Result<()>;
    /// Atomically consume a durable approval bound to this scope and full input
    /// digest. Reused request IDs must stay denied after completion or restart.
    fn consume(&self, scope: &UseScope, version: &Version, input_sha256: &str) -> Result<()>;
    /// Durable outcome for the same consumed request; failed/lost completion is
    /// unknown. This cannot reinstall authority or enable automatic replay.
    fn complete(&self, scope: &UseScope, version: &Version, outcome: Completion) -> Result<()>;
}

/// A permit cannot be constructed by decoding a runner message.
/// ```compile_fail
/// use dunlin_vault::scoped::Permit;
/// let _: Permit = serde_json::from_str("{}").unwrap();
/// ```
/// Nor can a consumed permit be duplicated.
/// ```compile_fail
/// use dunlin_vault::scoped::Permit;
/// fn duplicate(permit: Permit) -> (Permit, Permit) {
///     (permit.clone(), permit)
/// }
/// ```
pub struct Permit {
    scope: UseScope,
    version: Version,
    input_sha256: String,
    access: LiveAccess,
    authority: Arc<dyn Authority>,
    expiry: Arc<AtomicI64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    reference: String,
    credential: String,
    binding: Binding,
    policy: SecretPolicy,
    value: String,
}
impl Drop for Record {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
fn validate_policy(binding: &Binding, policy: &SecretPolicy) -> Result<()> {
    binding.validate()?;
    policy.validate().map_err(|_| Error::Invalid)?;
    if policy.project_ids.as_slice() != std::slice::from_ref(&binding.project)
        || matches!(policy.injection, Injection::OAuth2 { .. })
    {
        // OAuth token rotation uses the separate durable OAuth engine.
        return Err(Error::Invalid);
    }
    Ok(())
}

pub struct Credentials<J> {
    journal: Arc<J>,
    secrets: Arc<dyn SecretStore>,
}
impl<J: Journal> Credentials<J> {
    /// Separate keyring namespace: a legacy raw-value broker cannot read a
    /// scoped JSON envelope as an injectable credential.
    pub fn relay(journal: Arc<J>) -> Self {
        Self::new(journal, Arc::new(ServiceKeychainStore::scoped_relay()))
    }
    pub fn new(journal: Arc<J>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { journal, secrets }
    }

    pub fn install(
        &self,
        credential: &str,
        binding: Binding,
        expected_revision: u64,
        policy: SecretPolicy,
        value: Zeroizing<String>,
    ) -> Result<Version> {
        validate_policy(&binding, &policy)?;
        if !token(credential) || value.is_empty() || value.len() > 16384 {
            return Err(Error::Invalid);
        }
        let reference = format!("scoped-secret:{}", crate::protocol::new_token());
        let record = Record {
            schema: 1,
            reference: reference.clone(),
            credential: credential.into(),
            binding: binding.clone(),
            policy,
            value: value.to_string(),
        };
        let raw = Zeroizing::new(serde_json::to_string(&record).map_err(|_| Error::Invalid)?);
        if raw.len() > MAX_RECORD {
            return Err(Error::Invalid);
        }
        let candidate = Candidate {
            credential: credential.into(),
            binding,
            expected_revision,
            reference,
            sha256: digest(raw.as_bytes()),
        };
        let claim = self.journal.reserve(&candidate)?;
        if claim.candidate.reference != candidate.reference
            || claim.candidate.sha256 != candidate.sha256
            || claim.candidate.credential != candidate.credential
            || claim.candidate.binding != candidate.binding
            || claim.candidate.expected_revision != expected_revision
        {
            return Err(Error::Conflict);
        }
        self.secrets
            .set(&candidate.reference, &raw)
            .map_err(|_| Error::WriteUnknown)?;
        self.journal
            .acknowledge(&claim)
            .map_err(|_| Error::WriteUnknown)?;
        let version = self.journal.activate(claim).map_err(|error| match error {
            Error::Denied | Error::Conflict | Error::Invalid => error,
            _ => Error::WriteUnknown,
        })?;
        if version.reference != candidate.reference
            || version.sha256 != candidate.sha256
            || version.binding != candidate.binding
            || version.credential != candidate.credential
            || version.revision != expected_revision + 1
        {
            return Err(Error::Conflict);
        }
        Ok(version)
    }

    pub fn permit(
        &self,
        scope: UseScope,
        version: Version,
        input: &HttpInput,
        deadline: Instant,
        authority: Arc<dyn Authority>,
    ) -> Result<Permit> {
        scope.validate()?;
        let reference = version
            .reference
            .strip_prefix("scoped-secret:")
            .ok_or(Error::Denied)?;
        if reference.len() != 64
            || !reference
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || version.sha256.len() != 64
            || !version
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || version.revision == 0
            || version.revision > 9_007_199_254_740_991
            || !token(&version.credential)
            || scope.binding != version.binding
            || input.secret != version.credential
            || deadline <= Instant::now()
        {
            return Err(Error::Denied);
        }
        let input_sha256 = input_digest(input)?;
        authority.check(&scope, &version)?;
        authority.consume(&scope, &version, &input_sha256)?;
        let checked_scope = scope.clone();
        let checked_version = version.clone();
        let checked_authority = authority.clone();
        let expiry = Arc::new(AtomicI64::new(i64::MAX));
        let checked_expiry = expiry.clone();
        let access = LiveAccess::new(deadline, move || {
            now_secs() < checked_expiry.load(Ordering::Acquire)
                && checked_authority
                    .check(&checked_scope, &checked_version)
                    .is_ok()
        });
        if !access.allowed() {
            return Err(Error::Denied);
        }
        Ok(Permit {
            scope,
            version,
            input_sha256,
            access,
            authority,
            expiry,
        })
    }

    pub fn http(
        &self,
        permit: Permit,
        input: HttpInput,
        executor: &dyn HttpExecutor,
    ) -> Result<HttpOutput> {
        let mut dispatched = false;
        let result = self.http_inner(&permit, input, executor, &mut dispatched);
        let outcome = match &result {
            Ok(_) => Completion::Complete,
            Err(Error::OutcomeUnknown) => Completion::Unknown,
            Err(_) => Completion::Failed,
        };
        if permit
            .authority
            .complete(&permit.scope, &permit.version, outcome)
            .is_err()
        {
            return Err(Error::OutcomeUnknown);
        }
        if !permit.access.allowed() {
            return Err(if dispatched {
                Error::OutcomeUnknown
            } else {
                Error::Denied
            });
        }
        result
    }
    fn http_inner(
        &self,
        permit: &Permit,
        input: HttpInput,
        executor: &dyn HttpExecutor,
        dispatched: &mut bool,
    ) -> Result<HttpOutput> {
        if input_digest(&input)? != permit.input_sha256 || !permit.access.allowed() {
            return Err(Error::Denied);
        }
        let raw = self
            .secrets
            .get(&permit.version.reference)
            .map_err(|_| Error::Storage)?;
        if !permit.access.allowed() {
            return Err(Error::Denied);
        }
        if raw.len() > MAX_RECORD || digest(raw.as_bytes()) != permit.version.sha256 {
            return Err(Error::Denied);
        }
        let record: Record = serde_json::from_str(&raw).map_err(|_| Error::Denied)?;
        if record.schema != 1
            || record.reference != permit.version.reference
            || record.credential != permit.version.credential
            || record.binding != permit.scope.binding
        {
            return Err(Error::Denied);
        }
        validate_policy(&record.binding, &record.policy)?;
        permit.expiry.store(
            record.policy.expires_at.unwrap_or(i64::MAX),
            Ordering::Release,
        );
        if record
            .policy
            .expires_at
            .is_some_and(|expiry| expiry <= now_secs())
        {
            return Err(Error::Denied);
        }
        let validated = http::validate_input(&record.policy, &input).map_err(|_| Error::Denied)?;
        if permit.scope.read_only && validated.method.is_write() {
            return Err(Error::Denied);
        }
        let mut plan = http::apply_injection(&record.policy, validated, &record.value);
        if matches!(record.policy.injection, Injection::AwsSigV4 { .. }) {
            http::sign_aws(&record.policy, &mut plan, &record.value).map_err(|_| Error::Failed)?;
        }
        if !permit.access.allowed() {
            return Err(Error::Denied);
        }
        *dispatched = true;
        let response = executor.execute(&plan);
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        let mut output = response.map_err(|_| Error::OutcomeUnknown)?;
        let basic = match &record.policy.injection {
            Injection::Basic { username } => {
                Some(Zeroizing::new(format!("{username}:{}", record.value)))
            }
            _ => None,
        };
        let basic = basic.as_ref().map(|s| s.as_str());
        output.body = http::redact(&output.body, &record.value, basic);
        if let Some(content_type) = &mut output.content_type {
            *content_type = http::redact(content_type, &record.value, basic);
        }
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        Ok(output)
    }
}
fn input_digest(input: &HttpInput) -> Result<String> {
    let raw = Zeroizing::new(serde_json::to_vec(input).map_err(|_| Error::Invalid)?);
    if raw.len() > http::MAX_REQUEST_BODY + 16384 {
        return Err(Error::Invalid);
    }
    Ok(digest(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpPlan;
    use crate::store::MemoryStore;
    use dunlin_domain::vault::HttpMethod;
    use std::collections::{BTreeMap, HashSet};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct State {
        revision: u64,
        references: Vec<(Candidate, bool)>,
        active: Option<Version>,
    }
    #[derive(Default)]
    struct TestJournal {
        state: Mutex<State>,
        fail_ack: AtomicBool,
        deny_activation: AtomicBool,
        fail_activation_ack: AtomicBool,
    }
    impl Journal for TestJournal {
        fn reserve(&self, candidate: &Candidate) -> Result<WriteClaim> {
            let mut state = self.state.lock().unwrap();
            if state.revision != candidate.expected_revision
                || state
                    .references
                    .iter()
                    .any(|(old, _)| old.reference == candidate.reference)
            {
                return Err(Error::Conflict);
            }
            let claim = WriteClaim::reserved(candidate.clone(), state.revision + 1)?;
            state.revision += 1;
            state.references.push((candidate.clone(), false));
            Ok(claim)
        }
        fn acknowledge(&self, claim: &WriteClaim) -> Result<()> {
            if self.fail_ack.load(Ordering::Acquire) {
                return Err(Error::Storage);
            }
            let mut state = self.state.lock().unwrap();
            let pending = state
                .references
                .iter_mut()
                .find(|(candidate, _)| candidate.reference == claim.candidate.reference)
                .ok_or(Error::Conflict)?;
            assert_eq!(pending.0.sha256, claim.candidate.sha256);
            pending.1 = true;
            Ok(())
        }
        fn activate(&self, claim: WriteClaim) -> Result<Version> {
            if self.deny_activation.load(Ordering::Acquire) {
                return Err(Error::Denied);
            }
            let mut state = self.state.lock().unwrap();
            if state.revision != claim.revision
                || !state.references.iter().any(|(candidate, ack)| {
                    candidate.reference == claim.candidate.reference && *ack
                })
            {
                return Err(Error::Conflict);
            }
            let version = claim.activated();
            state.active = Some(version.clone());
            if self.fail_activation_ack.load(Ordering::Acquire) {
                return Err(Error::Storage);
            }
            Ok(version)
        }
    }
    struct TestAuthority {
        scope: UseScope,
        version: Version,
        allowed: AtomicBool,
        requests: Mutex<HashSet<String>>,
        outcomes: Mutex<Vec<Completion>>,
        fail_completion: AtomicBool,
    }
    impl TestAuthority {
        fn new(scope: UseScope, version: Version) -> Arc<Self> {
            Arc::new(Self {
                scope,
                version,
                allowed: AtomicBool::new(true),
                requests: Mutex::default(),
                outcomes: Mutex::default(),
                fail_completion: AtomicBool::new(false),
            })
        }
    }
    impl Authority for TestAuthority {
        fn check(&self, scope: &UseScope, version: &Version) -> Result<()> {
            if self.allowed.load(Ordering::Acquire)
                && scope == &self.scope
                && version == &self.version
            {
                Ok(())
            } else {
                Err(Error::Denied)
            }
        }
        fn consume(&self, scope: &UseScope, version: &Version, _: &str) -> Result<()> {
            self.check(scope, version)?;
            if self.requests.lock().unwrap().insert(scope.request.clone()) {
                Ok(())
            } else {
                Err(Error::Conflict)
            }
        }
        fn complete(&self, _: &UseScope, _: &Version, outcome: Completion) -> Result<()> {
            self.outcomes.lock().unwrap().push(outcome);
            if self.fail_completion.load(Ordering::Acquire) {
                Err(Error::Storage)
            } else {
                Ok(())
            }
        }
    }
    struct TestHttp {
        calls: AtomicUsize,
        revoke: Option<Arc<TestAuthority>>,
        fail: bool,
    }
    impl HttpExecutor for TestHttp {
        fn execute(&self, plan: &HttpPlan) -> std::result::Result<HttpOutput, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(
                plan.headers
                    .iter()
                    .find(|(name, _)| name == "Authorization")
                    .map(|(_, value)| value.as_str())
                    .unwrap(),
                "Bearer fixture-private-token"
            );
            if let Some(authority) = &self.revoke {
                authority.allowed.store(false, Ordering::Release);
            }
            if self.fail {
                return Err("fixture-private-token error".into());
            }
            Ok(HttpOutput {
                status: 200,
                content_type: Some("fixture-private-token".into()),
                body: "fixture-private-token output".into(),
                truncated: false,
            })
        }
    }
    fn http() -> TestHttp {
        TestHttp {
            calls: AtomicUsize::new(0),
            revoke: None,
            fail: false,
        }
    }
    fn binding() -> Binding {
        Binding {
            workspace: "workspace".into(),
            project: "project".into(),
            host: "host".into(),
        }
    }
    fn policy() -> SecretPolicy {
        SecretPolicy {
            version: 1,
            managed: true,
            injection: Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            },
            allowed_hosts: vec!["api.example.com".into()],
            path_prefix: "/".into(),
            allowed_methods: vec![HttpMethod::Get, HttpMethod::Post],
            allowed_headers: vec![],
            auto_approve: false,
            expires_at: Some(now_secs() + 600),
            project_ids: vec!["project".into()],
            recipes: vec![],
            operations: vec![],
        }
    }
    fn scope() -> UseScope {
        UseScope {
            principal_kind: PrincipalKind::Human,
            actor: "actor".into(),
            binding: binding(),
            session: "session".into(),
            turn: "turn".into(),
            request: "request".into(),
            profile_digest: "a".repeat(64),
            lease_instance: "instance".into(),
            lease_epoch: 1,
            grant_revision: 1,
            read_only: false,
        }
    }
    fn input() -> HttpInput {
        HttpInput {
            secret: "github-work".into(),
            method: "GET".into(),
            url: "https://api.example.com/item".into(),
            headers: BTreeMap::new(),
            body: None,
            operation: None,
            params: BTreeMap::new(),
        }
    }
    fn installed() -> (Credentials<TestJournal>, Arc<MemoryStore>, Version) {
        let journal = Arc::new(TestJournal::default());
        let secrets = Arc::new(MemoryStore::new());
        let credentials = Credentials::new(journal, secrets.clone());
        let version = credentials
            .install(
                "github-work",
                binding(),
                0,
                policy(),
                Zeroizing::new("fixture-private-token".into()),
            )
            .unwrap();
        (credentials, secrets, version)
    }
    fn permit(credentials: &Credentials<TestJournal>, authority: Arc<TestAuthority>) -> Permit {
        credentials
            .permit(
                authority.scope.clone(),
                authority.version.clone(),
                &input(),
                Instant::now() + Duration::from_secs(30),
                authority,
            )
            .unwrap()
    }

    #[test]
    fn immutable_envelope_redacts_http_and_consumes_approval_once() {
        let (credentials, secrets, version) = installed();
        let authority = TestAuthority::new(scope(), version.clone());
        let permit = permit(&credentials, authority.clone());
        let executor = http();
        let output = credentials.http(permit, input(), &executor).unwrap();
        assert!(!output.body.contains("fixture-private-token"));
        assert!(
            !output
                .content_type
                .unwrap()
                .contains("fixture-private-token")
        );
        assert_eq!(executor.calls.load(Ordering::Relaxed), 1);
        assert!(
            credentials
                .permit(
                    scope(),
                    version.clone(),
                    &input(),
                    Instant::now() + Duration::from_secs(30),
                    authority.clone()
                )
                .is_err()
        );
        assert_eq!(
            *authority.outcomes.lock().unwrap(),
            vec![Completion::Complete]
        );
        assert!(secrets.get_policy(&version.reference).is_err());
        let next = credentials
            .install(
                "github-work",
                binding(),
                1,
                policy(),
                Zeroizing::new("new-token".into()),
            )
            .unwrap();
        assert_ne!(next.reference, version.reference);
        assert_ne!(next.sha256, version.sha256);
        assert_eq!(next.revision, 2);
        assert!(
            secrets
                .get(&version.reference)
                .unwrap()
                .contains("fixture-private-token")
        );
    }

    #[test]
    fn record_tampering_and_cross_binding_cannot_dispatch() {
        for kind in 0..3 {
            let (credentials, secrets, version) = installed();
            let authority = TestAuthority::new(scope(), version.clone());
            let permit = permit(&credentials, authority);
            let raw = secrets.get(&version.reference).unwrap();
            let changed = match kind {
                0 => raw.replace("fixture-private-token", "replaced-token"),
                1 => raw.replace("api.example.com", "evil.example.com"),
                _ => raw.replace("\"host\"", "\"other-host\""),
            };
            secrets.set(&version.reference, &changed).unwrap();
            let executor = http();
            assert!(matches!(
                credentials.http(permit, input(), &executor),
                Err(Error::Denied)
            ));
            assert_eq!(executor.calls.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn input_change_and_read_only_write_are_denied_without_dispatch() {
        for read_only in [false, true] {
            let (credentials, _, version) = installed();
            let mut scope = scope();
            scope.read_only = read_only;
            let authority = TestAuthority::new(scope.clone(), version.clone());
            let mut request = input();
            if read_only {
                request.method = "POST".into();
            }
            let permit = credentials
                .permit(
                    scope,
                    version,
                    &request,
                    Instant::now() + Duration::from_secs(30),
                    authority,
                )
                .unwrap();
            if !read_only {
                request.url.push_str("/changed");
            }
            let executor = http();
            assert!(matches!(
                credentials.http(permit, request, &executor),
                Err(Error::Denied)
            ));
            assert_eq!(executor.calls.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn revoke_or_transport_error_after_dispatch_stays_unknown_without_provider_data() {
        for fail in [false, true] {
            let (credentials, _, version) = installed();
            let authority = TestAuthority::new(scope(), version);
            let permit = permit(&credentials, authority.clone());
            let executor = TestHttp {
                calls: AtomicUsize::new(0),
                revoke: if fail { None } else { Some(authority.clone()) },
                fail,
            };
            let error = credentials.http(permit, input(), &executor).unwrap_err();
            assert_eq!(error, Error::OutcomeUnknown);
            assert!(!error.to_string().contains("fixture-private-token"));
            assert_eq!(executor.calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                *authority.outcomes.lock().unwrap(),
                vec![Completion::Unknown]
            );
        }
    }

    struct ExpireHttp(Arc<AtomicI64>);
    impl HttpExecutor for ExpireHttp {
        fn execute(&self, _: &HttpPlan) -> std::result::Result<HttpOutput, String> {
            let expiry = self.0.load(Ordering::Acquire);
            assert!(
                expiry > now_secs() && expiry < i64::MAX,
                "record expiry must be installed before dispatch"
            );
            self.0.store(0, Ordering::Release);
            Ok(HttpOutput {
                status: 200,
                content_type: None,
                body: "must not be returned".into(),
                truncated: false,
            })
        }
    }
    #[test]
    fn policy_expiry_after_dispatch_discards_response() {
        let (credentials, _, version) = installed();
        let authority = TestAuthority::new(scope(), version);
        let permit = permit(&credentials, authority.clone());
        let executor = ExpireHttp(permit.expiry.clone());
        assert!(matches!(
            credentials.http(permit, input(), &executor),
            Err(Error::OutcomeUnknown)
        ));
        assert_eq!(
            *authority.outcomes.lock().unwrap(),
            vec![Completion::Unknown]
        );
    }

    struct RevokeOnRead {
        inner: Arc<MemoryStore>,
        authority: Arc<TestAuthority>,
    }
    impl SecretStore for RevokeOnRead {
        fn get(&self, id: &str) -> std::result::Result<Zeroizing<String>, String> {
            self.authority.allowed.store(false, Ordering::Release);
            self.inner.get(id)
        }
        fn set(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            panic!("read-only fixture")
        }
        fn delete(&self, _: &str) -> std::result::Result<(), String> {
            panic!("read-only fixture")
        }
        fn get_policy(&self, _: &str) -> std::result::Result<Zeroizing<String>, String> {
            panic!("no split policy")
        }
        fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            panic!("no split policy")
        }
        fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
            panic!("no split policy")
        }
    }
    #[test]
    fn revocation_while_reading_keyring_prevents_http_dispatch() {
        let (mut credentials, secrets, version) = installed();
        let authority = TestAuthority::new(scope(), version);
        let permit = permit(&credentials, authority.clone());
        credentials.secrets = Arc::new(RevokeOnRead {
            inner: secrets,
            authority: authority.clone(),
        });
        let executor = http();
        assert!(matches!(
            credentials.http(permit, input(), &executor),
            Err(Error::Denied)
        ));
        assert_eq!(executor.calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            *authority.outcomes.lock().unwrap(),
            vec![Completion::Failed]
        );
    }

    #[test]
    fn lost_completion_ack_discards_success() {
        let (credentials, _, version) = installed();
        let authority = TestAuthority::new(scope(), version);
        let permit = permit(&credentials, authority.clone());
        authority.fail_completion.store(true, Ordering::Release);
        assert!(matches!(
            credentials.http(permit, input(), &http()),
            Err(Error::OutcomeUnknown)
        ));
    }

    struct AmbiguousStore(MemoryStore);
    impl SecretStore for AmbiguousStore {
        fn get(&self, id: &str) -> std::result::Result<Zeroizing<String>, String> {
            self.0.get(id)
        }
        fn set(&self, id: &str, value: &str) -> std::result::Result<(), String> {
            self.0.set(id, value)?;
            Err("lost write acknowledgement".into())
        }
        fn delete(&self, _: &str) -> std::result::Result<(), String> {
            panic!("never clean an uncertain write")
        }
        fn get_policy(&self, _: &str) -> std::result::Result<Zeroizing<String>, String> {
            panic!("no split policy")
        }
        fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            panic!("no split policy")
        }
        fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
            panic!("no split policy")
        }
    }
    #[test]
    fn ambiguous_write_and_lost_ack_never_activate_or_erase_pending_reference() {
        for fail_ack in [false, true] {
            let journal = Arc::new(TestJournal::default());
            journal.fail_ack.store(fail_ack, Ordering::Release);
            let secrets: Arc<dyn SecretStore> = if fail_ack {
                Arc::new(MemoryStore::new())
            } else {
                Arc::new(AmbiguousStore(MemoryStore::new()))
            };
            let credentials = Credentials::new(journal.clone(), secrets.clone());
            assert!(matches!(
                credentials.install(
                    "github-work",
                    binding(),
                    0,
                    policy(),
                    Zeroizing::new("fixture-private-token".into())
                ),
                Err(Error::WriteUnknown)
            ));
            let state = journal.state.lock().unwrap();
            assert!(state.active.is_none());
            assert_eq!(state.references.len(), 1);
            assert!(!state.references[0].1);
            assert!(secrets.get(&state.references[0].0.reference).is_ok());
        }
    }

    #[test]
    fn revoked_activation_keeps_acknowledged_record_and_stale_install_is_fenced() {
        let journal = Arc::new(TestJournal::default());
        journal.deny_activation.store(true, Ordering::Release);
        let secrets = Arc::new(MemoryStore::new());
        let credentials = Credentials::new(journal.clone(), secrets.clone());
        assert!(matches!(
            credentials.install(
                "github-work",
                binding(),
                0,
                policy(),
                Zeroizing::new("fixture-private-token".into())
            ),
            Err(Error::Denied)
        ));
        assert!(matches!(
            credentials.install(
                "github-work",
                binding(),
                0,
                policy(),
                Zeroizing::new("fixture-private-token".into())
            ),
            Err(Error::Conflict)
        ));
        let state = journal.state.lock().unwrap();
        assert!(state.active.is_none());
        assert_eq!(state.references.len(), 1);
        assert!(state.references[0].1);
        assert!(secrets.get(&state.references[0].0.reference).is_ok());
    }

    #[test]
    fn lost_activation_commit_ack_is_reported_unknown_without_erasing_record() {
        let journal = Arc::new(TestJournal::default());
        journal.fail_activation_ack.store(true, Ordering::Release);
        let secrets = Arc::new(MemoryStore::new());
        let credentials = Credentials::new(journal.clone(), secrets.clone());
        assert!(matches!(
            credentials.install(
                "github-work",
                binding(),
                0,
                policy(),
                Zeroizing::new("fixture-private-token".into())
            ),
            Err(Error::WriteUnknown)
        ));
        let state = journal.state.lock().unwrap();
        let active = state.active.as_ref().unwrap();
        assert!(secrets.get(&active.reference).is_ok());
        assert_eq!(state.references.len(), 1);
        assert!(state.references[0].1);
    }

    #[test]
    fn host_authority_receives_all_scope_pins_before_value_read() {
        let (credentials, _, version) = installed();
        let authority = TestAuthority::new(scope(), version.clone());
        for i in 0..13 {
            let mut scope = scope();
            let mut version = version.clone();
            match i {
                0 => scope.principal_kind = PrincipalKind::Service,
                1 => scope.actor = "other".into(),
                2 => scope.binding.workspace = "other".into(),
                3 => scope.binding.project = "other".into(),
                4 => scope.binding.host = "other".into(),
                5 => scope.session = "other".into(),
                6 => scope.turn = "other".into(),
                7 => scope.profile_digest = "b".repeat(64),
                8 => scope.lease_instance = "other".into(),
                9 => scope.lease_epoch += 1,
                10 => scope.grant_revision += 1,
                11 => version.revision += 1,
                _ => version.reference = "mcp-oauth:other".into(),
            }
            assert!(
                credentials
                    .permit(
                        scope,
                        version,
                        &input(),
                        Instant::now() + Duration::from_secs(30),
                        authority.clone()
                    )
                    .is_err()
            );
        }
        assert!(authority.requests.lock().unwrap().is_empty());
    }
}
