//! Crash-safe coordination. The journal contains references/digests only; the
//! supplied secret callbacks must use the application's private secret store.
use super::{AuthContext, AuthorizationFlow, CredentialStatus, Error, Secret, TokenSet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

mod cleanup;
mod revocation;
mod shared;
pub use cleanup::{CleanupIntent, CleanupJournal, CleanupSummary, cleanup_credentials};
pub use revocation::{RevocationIntent, RevocationJournal, RevocationSummary, RevocationTarget};

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("OAuth credential access denied")]
    Unauthorized,
    #[error("OAuth credential generation changed or is blocked")]
    Conflict,
    #[error("OAuth credential storage unavailable")]
    Storage,
    #[error("OAuth credential storage limit reached")]
    Limit,
    #[error(transparent)]
    OAuth(#[from] Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialRecord {
    pub generation: u64,
    pub secret_reference: String,
    pub sha256: String,
}

/// A durable reservation, never an authority supplied by an HTTP client. The
/// backend must compare every field against its own row on commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialIntent {
    pub generation: u64,
    pub operation_id: String,
    pub staged_reference: String,
}

/// All writes must be durable before returning. Implementations reauthorize
/// the owner/workspace/revision on positive calls, and serialize reservations with
/// compare-and-swap. No automatic recovery may turn an unfinished intent active.
/// References are unique forever and retained for later negative cleanup,
/// including reservations superseded by disable/reconfiguration.
pub trait CredentialJournal {
    fn active(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialRecord, PersistenceError>;
    fn begin_install(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialIntent, PersistenceError>;
    fn begin_refresh(
        &self,
        context: &AuthContext,
        binding: &str,
        expected: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError>;
    /// Persist the exact envelope identity before any secret-store write. This
    /// must reauthorize the reservation and reject a different digest on retry.
    /// Metadata survives a failed/uncertain write and a later revision fence.
    fn prepare_write(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError>;
    /// One-use durable claim before invoking the private secret backend.
    fn claim_write(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError>;
    /// Negative evidence only, called after one successful backend set. This
    /// must match the original immutable reservation, even if access/revision
    /// was revoked during I/O. It must never activate or republish credentials.
    /// Failure/absence stays unknown; startup must not invent an acknowledgement.
    fn acknowledge_write(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError>;
    fn commit(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError>;
    /// Increments the connection revision and fences every outstanding writer.
    fn disable(&self, context: &AuthContext) -> Result<u64, PersistenceError>;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    user_id: String,
    workspace_id: String,
    connection_id: String,
    connection_revision: u64,
    binding: String,
    generation: u64,
    saved_at_ms: u64,
    expires_at_ms: u64,
    access_token: Secret,
    refresh_token: Option<Secret>,
    scopes: Vec<String>,
}
fn clock_ms() -> Result<u64, PersistenceError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or(PersistenceError::Storage)
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

impl AuthorizationFlow {
    /// Frozen transport/registration configuration, including the client-secret
    /// hash. A configuration change cannot inherit an older credential record.
    pub fn credential_binding(&self) -> String {
        let mut hash = Sha256::new();
        for value in [
            self.endpoint.as_str(),
            self.resource.as_str(),
            self.issuer.as_str(),
            self.authorization_endpoint.as_str(),
            self.token_endpoint.as_str(),
            self.revocation_endpoint.as_ref().map_or("", |u| u.as_str()),
            self.registration.client_id.as_str(),
            self.registration.redirect_uri.as_str(),
            self.registration.binding_method(),
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        match &self.registration.authentication {
            super::Authentication::Basic(secret) | super::Authentication::Post(secret) => {
                hash.update(secret.0.as_bytes());
            }
            super::Authentication::Public | super::Authentication::ClientMetadataDocument => {}
        }
        hex::encode(hash.finalize())
    }

    fn encode(
        &self,
        token: TokenSet,
        generation: u64,
    ) -> Result<Zeroizing<String>, PersistenceError> {
        let saved_at_ms = clock_ms()?;
        let remaining = token
            .expires
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(3600))
            .as_millis() as u64;
        let envelope = Envelope {
            version: 1,
            user_id: token.context.user_id,
            workspace_id: token.context.workspace_id,
            connection_id: token.context.connection_id,
            connection_revision: token.context.connection_revision,
            binding: self.credential_binding(),
            generation,
            saved_at_ms,
            expires_at_ms: saved_at_ms
                .checked_add(remaining)
                .ok_or(PersistenceError::Storage)?,
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            scopes: token.scopes,
        };
        // Tokens are <=8KiB each and scopes <=64*256 ASCII bytes. Reserve
        // the full envelope budget so serialization never reallocates a buffer
        // that already contains credential bytes.
        let mut bytes = Zeroizing::new(Vec::with_capacity(65536));
        serde_json::to_writer(&mut *bytes, &envelope).map_err(|_| PersistenceError::Storage)?;
        if bytes.len() > 65536 {
            return Err(PersistenceError::Limit);
        }
        // Move the allocation into the zeroizing String, without a plaintext copy.
        String::from_utf8(std::mem::take(&mut *bytes))
            .map(Zeroizing::new)
            .map_err(|error| {
                let _bytes = Zeroizing::new(error.into_bytes());
                PersistenceError::Storage
            })
    }

    fn decode(
        &self,
        context: &AuthContext,
        record: &CredentialRecord,
        value: &str,
    ) -> Result<TokenSet, PersistenceError> {
        if value.len() > 65536 || digest(value) != record.sha256 {
            return Err(PersistenceError::Storage);
        }
        let envelope: Envelope =
            serde_json::from_str(value).map_err(|_| PersistenceError::Storage)?;
        let now = clock_ms()?;
        if envelope.version != 1
            || envelope.user_id != context.user_id
            || envelope.workspace_id != context.workspace_id
            || envelope.connection_id != context.connection_id
            || envelope.connection_revision != context.connection_revision
            || envelope.binding != self.credential_binding()
            || envelope.generation != record.generation
            || envelope.saved_at_ms > now
            || envelope.expires_at_ms < envelope.saved_at_ms
            || envelope.expires_at_ms - envelope.saved_at_ms > 3_600_000
        {
            return Err(PersistenceError::Unauthorized);
        }
        super::valid_secret(&envelope.access_token.0)?;
        if let Some(refresh) = &envelope.refresh_token {
            super::valid_secret(&refresh.0)?;
        }
        super::super::scopes(&envelope.scopes)?;
        let token = TokenSet {
            context: context.clone(),
            endpoint: self.endpoint.clone(),
            resource: self.resource.clone(),
            issuer: self.issuer.clone(),
            client_id: self.registration.client_id.clone(),
            access_token: envelope.access_token,
            refresh_token: envelope.refresh_token,
            scopes: envelope.scopes,
            expires: Instant::now()
                + Duration::from_millis(envelope.expires_at_ms.saturating_sub(now)),
            status: CredentialStatus::Active,
        };
        self.bound(context, &token)?;
        Ok(token)
    }

    /// Consumes the callback result. The writer receives a fresh, immutable
    /// reference, never a reference that may contain another generation.
    pub fn install_durable(
        &self,
        context: &AuthContext,
        token: TokenSet,
        journal: &impl CredentialJournal,
        mut put: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<CredentialRecord, PersistenceError> {
        self.bound(context, &token)?;
        if token.status != CredentialStatus::Active {
            return Err(PersistenceError::Conflict);
        }
        let binding = self.credential_binding();
        let intent = journal.begin_install(context, &binding)?;
        self.publish(context, token, journal, &intent, &mut put)
    }

    fn publish(
        &self,
        context: &AuthContext,
        token: TokenSet,
        journal: &impl CredentialJournal,
        intent: &CredentialIntent,
        put: &mut impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<CredentialRecord, PersistenceError> {
        let value = self.encode(token, intent.generation)?;
        let sha256 = digest(&value);
        journal.prepare_write(context, &self.credential_binding(), intent, &sha256)?;
        journal.claim_write(context, &self.credential_binding(), intent, &sha256)?;
        put(&intent.staged_reference, &value)?;
        journal.acknowledge_write(context, &self.credential_binding(), intent, &sha256)?;
        journal.commit(context, &self.credential_binding(), intent, &sha256)?;
        Ok(CredentialRecord {
            generation: intent.generation,
            secret_reference: intent.staged_reference.clone(),
            sha256,
        })
    }

    /// Journal reservation is committed BEFORE the rotating HTTP request. Every
    /// error after reservation leaves the old generation unusable, across restart.
    pub fn refresh_durable(
        &self,
        context: &AuthContext,
        journal: &impl CredentialJournal,
        mut get: impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
        mut put: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<CredentialRecord, PersistenceError> {
        context.validate()?;
        let binding = self.credential_binding();
        let record = journal.active(context, &binding)?;
        let token = self.decode(context, &record, &get(&record.secret_reference)?)?;
        self.refresh_record(context, journal, &record, token, &mut put)
    }

    fn refresh_record(
        &self,
        context: &AuthContext,
        journal: &impl CredentialJournal,
        record: &CredentialRecord,
        mut token: TokenSet,
        put: &mut impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<CredentialRecord, PersistenceError> {
        if !token.has_refresh_token() {
            return Err(Error::Unsupported.into());
        }
        let intent = journal.begin_refresh(context, &self.credential_binding(), record)?;
        self.refresh(context, &mut token)?;
        self.publish(context, token, journal, &intent, put)
    }

    /// Rereads authorization and generation after fetching the secret. Call this
    /// at dispatch, rather than caching bearer tokens between MCP operations.
    pub fn access_token_durable(
        &self,
        context: &AuthContext,
        journal: &impl CredentialJournal,
        mut get: impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
    ) -> Result<Zeroizing<String>, PersistenceError> {
        self.access_record(context, journal, &mut get)
            .map(|(_, bearer)| bearer)
    }

    /// Refresh only a locally expired, valid envelope before the first MCP
    /// dispatch. Never use this to retry an MCP 401 or an uncertain refresh.
    /// The returned record pins the exact generation that owns the bearer.
    pub fn access_token_or_refresh_durable(
        &self,
        context: &AuthContext,
        journal: &impl CredentialJournal,
        mut get: impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
        mut put: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<(CredentialRecord, Zeroizing<String>), PersistenceError> {
        context.validate()?;
        let record = journal.active(context, &self.credential_binding())?;
        let token = self.decode(context, &record, &get(&record.secret_reference)?)?;
        if token.expires <= Instant::now() {
            self.refresh_record(context, journal, &record, token, &mut put)?;
        }
        self.access_record(context, journal, &mut get)
    }

    fn access_record(
        &self,
        context: &AuthContext,
        journal: &impl CredentialJournal,
        get: &mut impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
    ) -> Result<(CredentialRecord, Zeroizing<String>), PersistenceError> {
        context.validate()?;
        let binding = self.credential_binding();
        let record = journal.active(context, &binding)?;
        let token = self.decode(context, &record, &get(&record.secret_reference)?)?;
        let bearer = Zeroizing::new(token.access_token(context, &self.endpoint)?.to_owned());
        if journal.active(context, &binding)? != record {
            return Err(PersistenceError::Conflict);
        }
        Ok((record, bearer))
    }
}
