//! Single-use, user-bound authorization records. Credentials and callback codes
//! have no Debug/Serialize and are kept in zeroizing buffers owned by this layer.
use super::{
    AuthorizationMetadata, Discovery, Error, MAX_METADATA, ResourceMetadata, scopes, secure_url,
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use dunlin_http::{Agent, AgentBuilder};
use rand::RngExt;
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    sync::Mutex,
    time::{Duration, Instant},
};
use url::Url;
use zeroize::Zeroizing;

const MAX_PENDING: usize = 32;
const STATE_SECONDS: u64 = 300;
mod lifecycle;
pub mod persistence;

#[derive(Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub user_id: String,
    pub workspace_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
}
impl AuthContext {
    /// Checks shape only; the server must independently authenticate and authorize the caller.
    pub fn validate(&self) -> Result<(), Error> {
        if self.connection_revision == 0 || self.connection_revision > 9_007_199_254_740_991 {
            return Err(Error::Protocol);
        }
        for value in [&self.user_id, &self.workspace_id, &self.connection_id] {
            if value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
            {
                return Err(Error::Protocol);
            }
        }
        Ok(())
    }
}

struct Secret(Zeroizing<String>);
impl serde::Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Zeroizing::new(s)))
    }
}
enum Authentication {
    Public,
    ClientMetadataDocument,
    Basic(Secret),
    Post(Secret),
}
pub struct Registration {
    issuer: String,
    client_id: String,
    redirect_uri: String,
    authentication: Authentication,
}
impl Registration {
    pub fn public(issuer: &str, client_id: &str, redirect_uri: &str) -> Result<Self, Error> {
        Self::build(issuer, client_id, redirect_uri, Authentication::Public)
    }
    /// Public CIMD client. The operator must host the matching document at
    /// `client_id`; fetching/validating that document belongs to the AS. This
    /// constructor does no network I/O and never falls back to preregistration.
    pub fn client_metadata_document(
        issuer: &str,
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<Self, Error> {
        super::client_metadata::identifier(client_id)?;
        Self::build(
            issuer,
            client_id,
            redirect_uri,
            Authentication::ClientMetadataDocument,
        )
    }
    pub fn confidential_basic(
        issuer: &str,
        client_id: &str,
        redirect_uri: &str,
        secret: String,
    ) -> Result<Self, Error> {
        Self::build(
            issuer,
            client_id,
            redirect_uri,
            Authentication::Basic(Secret(Zeroizing::new(secret))),
        )
    }
    pub fn confidential_post(
        issuer: &str,
        client_id: &str,
        redirect_uri: &str,
        secret: String,
    ) -> Result<Self, Error> {
        Self::build(
            issuer,
            client_id,
            redirect_uri,
            Authentication::Post(Secret(Zeroizing::new(secret))),
        )
    }
    fn build(
        issuer: &str,
        client_id: &str,
        redirect_uri: &str,
        authentication: Authentication,
    ) -> Result<Self, Error> {
        secure_url(issuer, false)?;
        secure_url(redirect_uri, false)?;
        if client_id.is_empty()
            || client_id.len() > 256
            || !client_id.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(Error::Credential);
        }
        match &authentication {
            Authentication::Basic(secret) | Authentication::Post(secret) => {
                valid_secret(&secret.0)?
            }
            Authentication::Public | Authentication::ClientMetadataDocument => {}
        }
        Ok(Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            authentication,
        })
    }
    fn method(&self) -> &'static str {
        match self.authentication {
            Authentication::Public | Authentication::ClientMetadataDocument => "none",
            Authentication::Basic(_) => "client_secret_basic",
            Authentication::Post(_) => "client_secret_post",
        }
    }
    fn binding_method(&self) -> &'static str {
        if matches!(self.authentication, Authentication::ClientMetadataDocument) {
            "client_metadata_document:none"
        } else {
            self.method()
        }
    }
}
fn valid_secret(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > 8192 || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(Error::Credential);
    }
    Ok(())
}
fn random() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand::rng().fill(&mut *bytes);
    Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_slice()))
}
fn hash(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
fn component(value: &str) -> Zeroizing<String> {
    let mut encoded = Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("", value)
            .finish(),
    );
    encoded.remove(0);
    encoded
}

struct Pending {
    context: AuthContext,
    verifier: Zeroizing<String>,
    scopes: Vec<String>,
    created: Instant,
}
pub struct AuthorizationRequest {
    pub url: String,
}
pub struct AuthorizationFlow {
    endpoint: String,
    resource: String,
    issuer: String,
    authorization_endpoint: Url,
    token_endpoint: Url,
    revocation_endpoint: Option<Url>,
    issuer_required: bool,
    registration: Registration,
    agent: Agent,
    pending: Mutex<HashMap<[u8; 32], Pending>>,
}
impl Discovery {
    pub fn authorization_flow(
        &self,
        resource: &ResourceMetadata,
        metadata: AuthorizationMetadata,
        registration: Registration,
    ) -> Result<AuthorizationFlow, Error> {
        resource.validate(&self.endpoint)?;
        if !resource.authorization_servers.contains(&metadata.issuer)
            || metadata.issuer != registration.issuer
        {
            return Err(Error::Protocol);
        }
        if matches!(
            registration.authentication,
            Authentication::ClientMetadataDocument
        ) && !metadata.client_id_metadata_document_supported
        {
            return Err(Error::Unsupported);
        }
        self.allowed_url(&metadata.issuer, false)?;
        let authorization_endpoint = self.allowed_url(&metadata.authorization_endpoint, true)?;
        let token_endpoint = self.allowed_url(&metadata.token_endpoint, true)?;
        let revoke = metadata
            .revocation_endpoint
            .as_ref()
            .map(|url| self.allowed_url(url, true))
            .transpose()?;
        if !metadata
            .response_types_supported
            .iter()
            .any(|v| v == "code")
            || !metadata
                .code_challenge_methods_supported
                .iter()
                .any(|v| v == "S256")
        {
            return Err(Error::Unsupported);
        }
        let default = vec!["client_secret_basic".into()];
        if !metadata
            .token_endpoint_auth_methods_supported
            .as_ref()
            .unwrap_or(&default)
            .iter()
            .any(|m| m == registration.method())
        {
            return Err(Error::Unsupported);
        }
        for endpoint in [&authorization_endpoint, &token_endpoint]
            .into_iter()
            .chain(revoke.iter())
        {
            if endpoint.query_pairs().any(|(name, _)| {
                matches!(
                    &*name,
                    "client_id"
                        | "client_secret"
                        | "redirect_uri"
                        | "response_type"
                        | "state"
                        | "scope"
                        | "resource"
                        | "code"
                        | "code_challenge"
                        | "code_challenge_method"
                        | "code_verifier"
                        | "grant_type"
                        | "refresh_token"
                        | "token"
                        | "token_type_hint"
                        | "access_token"
                )
            }) {
                return Err(Error::Protocol);
            }
        }
        Ok(AuthorizationFlow {
            endpoint: self.endpoint.to_string(),
            resource: secure_url(&resource.resource, false)?.to_string(),
            issuer: metadata.issuer,
            authorization_endpoint,
            token_endpoint,
            revocation_endpoint: if metadata
                .revocation_endpoint_auth_methods_supported
                .as_ref()
                .unwrap_or(&default)
                .iter()
                .any(|m| m == registration.method())
            {
                revoke
            } else {
                None
            },
            issuer_required: metadata.authorization_response_iss_parameter_supported,
            registration,
            agent: AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0)
                .build(),
            pending: Mutex::new(HashMap::new()),
        })
    }
}
impl AuthorizationFlow {
    /// context must come from a freshly authenticated Dunlin caller, never the
    /// callback's query. Return URL is reviewed/opened by the application UI.
    pub fn begin(
        &self,
        context: AuthContext,
        requested_scopes: Vec<String>,
    ) -> Result<AuthorizationRequest, Error> {
        context.validate()?;
        scopes(&requested_scopes)?;
        let mut pending = self.pending.lock().map_err(|_| Error::Protocol)?;
        pending.retain(|_, p| p.created.elapsed() < Duration::from_secs(STATE_SECONDS));
        if pending.len() >= MAX_PENDING {
            return Err(Error::Limit);
        }
        let state = random();
        let verifier = random();
        let challenge = URL_SAFE_NO_PAD.encode(hash(&verifier));
        let mut url = self.authorization_endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.registration.client_id)
                .append_pair("redirect_uri", &self.registration.redirect_uri)
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("resource", &self.resource);
            if !requested_scopes.is_empty() {
                query.append_pair("scope", &requested_scopes.join(" "));
            }
        }
        let key = hash(&state);
        if pending.contains_key(&key) {
            return Err(Error::Limit);
        }
        pending.insert(
            key,
            Pending {
                context,
                verifier,
                scopes: requested_scopes,
                created: Instant::now(),
            },
        );
        Ok(AuthorizationRequest { url: url.into() })
    }

    /// Single-use callback processing. Issuer is verified before error/code is
    /// acted on, and the record is consumed before any token HTTP request.
    pub fn finish(&self, context: &AuthContext, callback_uri: &str) -> Result<TokenSet, Error> {
        context.validate()?;
        if callback_uri.len() > 16384 {
            return Err(Error::Limit);
        }
        let callback = secure_url(callback_uri, true)?;
        let expected = secure_url(&self.registration.redirect_uri, false)?;
        if callback.origin() != expected.origin() || callback.path() != expected.path() {
            return Err(Error::Protocol);
        }
        let mut fields = BTreeMap::new();
        for (name, value) in callback.query_pairs() {
            if fields.len() >= 16
                || name.len() > 64
                || value.len() > 8192
                || fields
                    .insert(name.into_owned(), Zeroizing::new(value.into_owned()))
                    .is_some()
            {
                return Err(Error::Protocol);
            }
        }
        let state = fields.get("state").ok_or(Error::Unauthorized)?;
        let key = hash(state);
        let record = {
            let mut pending = self.pending.lock().map_err(|_| Error::Protocol)?;
            let record = pending.get(&key).ok_or(Error::Unauthorized)?;
            if record.context != *context {
                return Err(Error::Unauthorized);
            }
            let record = pending.remove(&key).ok_or(Error::Unauthorized)?;
            if record.created.elapsed() >= Duration::from_secs(STATE_SECONDS) {
                return Err(Error::Unauthorized);
            }
            record
        };
        match fields.get("iss") {
            Some(issuer) if issuer.as_str() != self.issuer => return Err(Error::Protocol),
            None if self.issuer_required => return Err(Error::Protocol),
            _ => {}
        }
        if fields.contains_key("error") {
            return Err(Error::Unauthorized);
        }
        let code = fields.get("code").ok_or(Error::Protocol)?;
        valid_secret(code)?;
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("redirect_uri", &self.registration.redirect_uri)
            .append_pair("code_verifier", &record.verifier)
            .append_pair("resource", &self.resource);
        let validated = self.tokens(form, &record.scopes)?;
        let token = validated.response;
        Ok(TokenSet {
            context: record.context,
            endpoint: self.endpoint.clone(),
            resource: self.resource.clone(),
            issuer: self.issuer.clone(),
            client_id: self.registration.client_id.clone(),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            scopes: validated.scopes,
            expires: validated.expires,
            status: CredentialStatus::Active,
        })
    }

    fn post_form(
        &self,
        endpoint: &Url,
        mut form: url::form_urlencoded::Serializer<'_, String>,
    ) -> Result<dunlin_http::Response, Error> {
        let mut request = self
            .agent
            .post(endpoint.as_str())
            .set("Accept", "application/json")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .timeout(Duration::from_secs(10));
        match &self.registration.authentication {
            Authentication::Public | Authentication::ClientMetadataDocument => {
                form.append_pair("client_id", &self.registration.client_id);
            }
            Authentication::Post(secret) => {
                form.append_pair("client_id", &self.registration.client_id)
                    .append_pair("client_secret", &secret.0);
            }
            Authentication::Basic(secret) => {
                let basic = Zeroizing::new(format!(
                    "{}:{}",
                    component(&self.registration.client_id).as_str(),
                    component(&secret.0).as_str()
                ));
                let encoded = Zeroizing::new(STANDARD.encode(basic.as_bytes()));
                let header = Zeroizing::new(format!("Basic {}", encoded.as_str()));
                request = request.set("Authorization", &header);
            }
        }
        let body = Zeroizing::new(form.finish());
        let response = request.send_string(&body).map_err(|error| match error {
            dunlin_http::Error::Status(status, _) => Error::Http(status),
            _ => Error::Transport,
        })?;
        if response.status() != 200 {
            return Err(Error::Http(response.status()));
        }
        Ok(response)
    }

    fn tokens(
        &self,
        form: url::form_urlencoded::Serializer<'_, String>,
        permitted: &[String],
    ) -> Result<Validated, Error> {
        let token_started = Instant::now();
        let response = self.post_form(&self.token_endpoint, form)?;
        if !response
            .header("Content-Type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(Error::Protocol);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response
            .into_reader()
            .take(MAX_METADATA as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Transport)?;
        if bytes.len() > MAX_METADATA {
            return Err(Error::Limit);
        }
        let token: TokenResponse = serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?;
        crate::validate_credential(&token.access_token.0)?;
        if !token.token_type.eq_ignore_ascii_case("Bearer") || token.expires_in == Some(0) {
            return Err(Error::Protocol);
        }
        if let Some(refresh) = &token.refresh_token {
            valid_secret(&refresh.0)?;
        }
        let granted = token
            .scope
            .as_deref()
            .map(super::scope_string)
            .transpose()?
            .unwrap_or_else(|| permitted.to_vec());
        if granted.iter().any(|scope| !permitted.contains(scope)) {
            return Err(Error::Protocol);
        }
        let expires = token_started + Duration::from_secs(token.expires_in.unwrap_or(60).min(3600));
        Ok(Validated {
            response: token,
            scopes: granted,
            expires,
        })
    }
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: Secret,
    token_type: String,
    refresh_token: Option<Secret>,
    expires_in: Option<u64>,
    scope: Option<String>,
}
struct Validated {
    response: TokenResponse,
    scopes: Vec<String>,
    expires: Instant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialStatus {
    Active,
    RefreshUncertain,
    RevocationUnconfirmed,
    /// The AS accepted revocation of the token values known to this client.
    /// This does not assert revocation of every token in a provider grant.
    Revoked,
}
pub struct TokenSet {
    context: AuthContext,
    endpoint: String,
    issuer: String,
    client_id: String,
    resource: String,
    access_token: Secret,
    refresh_token: Option<Secret>,
    scopes: Vec<String>,
    expires: Instant,
    status: CredentialStatus,
}
impl TokenSet {
    pub fn access_token(&self, context: &AuthContext, endpoint: &str) -> Result<&str, Error> {
        crate::validate_endpoint(endpoint)?;
        if self.status != CredentialStatus::Active
            || self.context != *context
            || Url::parse(endpoint).map_err(|_| Error::Endpoint)?.as_str() != self.endpoint
            || self.expires <= Instant::now()
        {
            return Err(Error::Unauthorized);
        }
        Ok(&self.access_token.0)
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }
    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }
    pub fn status(&self) -> CredentialStatus {
        self.status
    }
}

#[cfg(test)]
mod tests;
