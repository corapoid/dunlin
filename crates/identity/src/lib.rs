//! OIDC relying party. No self-registration, email linking or implicit admins.
mod auth {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use rand::RngExt;
    use sha2::{Digest, Sha256};
    pub fn hash_token(value: &str) -> String {
        hex::encode(Sha256::digest(value.as_bytes()))
    }
    pub fn random_token() -> String {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        format!("kst_{}", URL_SAFE_NO_PAD.encode(bytes))
    }
}
use openidconnect::url::Url;
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    /// Environment variable NAME only. Never serialize a client secret.
    pub client_secret_env: Option<String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Explicit subject -> existing local user ID. Provision users through CLI.
    pub bindings: BTreeMap<String, String>,
}

fn url(value: &str) -> Result<Url, String> {
    let u = Url::parse(value).map_err(|_| "Invalid OIDC URL")?;
    let loopback = u.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || !(u.scheme() == "https" || (u.scheme() == "http" && loopback))
        || u.host_str().is_none()
    {
        return Err(
            "OIDC requires HTTPS (or explicit loopback HTTP), without credentials or fragments"
                .into(),
        );
    }
    Ok(u)
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        let issuer = url(&self.issuer)?;
        let redirect = url(&self.redirect_uri)?;
        if issuer.query().is_some()
            || redirect.query().is_some()
            || redirect.path() != "/v1/web/oidc/callback"
            || self.client_id.is_empty()
            || self.client_id.len() > 256
        {
            return Err("Invalid OIDC issuer, client ID or callback path".into());
        }
        let host = redirect.host_str().unwrap_or("");
        if !(dunlin_domain::relay::tailscale_hostname(host)
            || host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
        {
            return Err("OIDC callback must use private Tailscale Serve or loopback".into());
        }
        if self.allowed_origins.len() > 16
            || self.bindings.len() > 1024
            || self.bindings.iter().any(|(subject, user)| {
                subject.is_empty() || subject.len() > 255 || user.is_empty() || user.len() > 128
            })
        {
            return Err("Invalid OIDC bindings or endpoint limits".into());
        }
        for origin in &self.allowed_origins {
            let u = url(origin)?;
            if u.origin().ascii_serialization() != *origin {
                return Err("OIDC allowlist entries must be exact origins".into());
            }
        }
        if self.client_secret_env.as_ref().is_some_and(|name| {
            name.is_empty()
                || name.len() > 128
                || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        }) {
            return Err("Invalid OIDC client secret environment variable name".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        auth::hash_token(&serde_json::to_string(self).expect("serializable OIDC config"))
    }
    fn allows(&self, endpoint: &str) -> bool {
        let Ok(endpoint) = url(endpoint) else {
            return false;
        };
        let Ok(issuer) = url(&self.issuer) else {
            return false;
        };
        endpoint.origin() == issuer.origin()
            || self
                .allowed_origins
                .contains(&endpoint.origin().ascii_serialization())
    }
}

struct Pending {
    browser_hash: String,
    created: Instant,
    config_digest: String,
    client: Client,
    nonce: Nonce,
    verifier: PkceCodeVerifier,
}

pub struct Service {
    pending: Mutex<HashMap<String, Pending>>,
    pub slots: Arc<tokio::sync::Semaphore>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

fn http(
    config: &Config,
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let denied = || std::io::Error::other("OIDC provider request failed");
    if !config.allows(&request.uri().to_string()) || request.body().len() > 16384 {
        return Err(denied());
    }
    let agent = dunlin_http::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(8))
        .build();
    let mut outgoing = agent.request(request.method().as_str(), &request.uri().to_string());
    for (name, value) in request.headers() {
        outgoing = outgoing.set(name.as_str(), value.to_str().map_err(|_| denied())?);
    }
    let response = match outgoing.send_bytes(request.body()) {
        Ok(r) => r,
        Err(dunlin_http::Error::Status(_, r)) => *r,
        Err(_) => return Err(denied()),
    };
    let mut builder = openidconnect::http::Response::builder().status(response.status());
    if let Some(content) = response.header("content-type") {
        builder = builder.header("content-type", content);
    }
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| denied())?;
    if body.len() > 1024 * 1024 {
        return Err(denied());
    }
    builder.body(body).map_err(|_| denied())
}

impl Service {
    pub fn start(&self, config: &Config) -> Result<(String, String), String> {
        config.validate()?;
        // Fail before outbound discovery if the bounded registry is full.
        {
            let mut pending = self.pending.lock().map_err(|_| "OIDC unavailable")?;
            pending.retain(|_, p| p.created.elapsed() < Duration::from_secs(300));
            if pending.len() >= 128 {
                return Err("OIDC login limit reached".into());
            }
        }
        let transport = |request| http(config, request);
        let metadata = CoreProviderMetadata::discover(
            &IssuerUrl::new(config.issuer.clone()).map_err(|_| "Invalid issuer")?,
            &transport,
        )
        .map_err(|_| "OIDC discovery failed")?;
        if !config.allows(metadata.authorization_endpoint().as_str())
            || !metadata
                .token_endpoint()
                .is_some_and(|u| config.allows(u.as_str()))
        {
            return Err("OIDC endpoint is outside the allowlist".into());
        }
        let secret = match &config.client_secret_env {
            Some(name) => Some(ClientSecret::new(
                std::env::var(name)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .ok_or("OIDC client secret unavailable")?,
            )),
            None => None,
        };
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id.clone()),
            secret,
        )
        .set_redirect_uri(
            RedirectUrl::new(config.redirect_uri.clone()).map_err(|_| "Invalid redirect URI")?,
        );
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (authorization, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .url();
        let browser = auth::random_token();
        let mut pending = self.pending.lock().map_err(|_| "OIDC unavailable")?;
        if pending.len() >= 128 {
            return Err("OIDC login limit reached".into());
        }
        pending.insert(
            auth::hash_token(state.secret()),
            Pending {
                browser_hash: auth::hash_token(&browser),
                created: Instant::now(),
                config_digest: config.digest(),
                client,
                nonce,
                verifier,
            },
        );
        Ok((authorization.to_string(), browser))
    }

    pub fn finish(
        &self,
        config: &Config,
        state: &str,
        browser: &str,
        code: &str,
    ) -> Result<(String, u64), String> {
        if state.len() > 256 || browser.len() != 47 {
            return Err("Invalid OIDC callback".into());
        }
        let pending = {
            let mut entries = self.pending.lock().map_err(|_| "OIDC unavailable")?;
            let key = auth::hash_token(state);
            let p = entries.get(&key).ok_or("OIDC login expired")?;
            if p.browser_hash != auth::hash_token(browser) {
                return Err("OIDC browser mismatch".into());
            }
            entries.remove(&key).ok_or("OIDC login expired")?
        };
        if pending.created.elapsed() >= Duration::from_secs(300)
            || pending.config_digest != config.digest()
        {
            return Err("OIDC login expired".into());
        }
        if code.is_empty() || code.len() > 4096 {
            return Err("Invalid OIDC callback".into());
        }
        let transport = |request| http(config, request);
        let tokens = pending
            .client
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(|_| "OIDC token endpoint missing")?
            .set_pkce_verifier(pending.verifier)
            .request(&transport)
            .map_err(|_| "OIDC code exchange failed")?;
        let token = tokens.id_token().ok_or("OIDC ID token missing")?;
        let verifier = pending.client.id_token_verifier().set_allowed_algs([
            openidconnect::core::CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            openidconnect::core::CoreJwsSigningAlgorithm::EcdsaP256Sha256,
            openidconnect::core::CoreJwsSigningAlgorithm::EdDsa,
        ]);
        let claims = token
            .claims(&verifier, &pending.nonce)
            .map_err(|_| "OIDC ID token rejected")?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                tokens.access_token(),
                token.signing_alg().map_err(|_| "OIDC algorithm rejected")?,
                token
                    .signing_key(&verifier)
                    .map_err(|_| "OIDC key rejected")?,
            )
            .map_err(|_| "OIDC access token rejected")?;
            if actual != *expected {
                return Err("OIDC access token rejected".into());
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid clock")?
            .as_secs() as i64;
        if claims.issue_time().timestamp() > now.saturating_add(60)
            || claims.issue_time().timestamp() < now.saturating_sub(600)
            || claims
                .authorized_party()
                .is_some_and(|party| party.as_str() != config.client_id)
            || (claims.audiences().len() > 1 && claims.authorized_party().is_none())
        {
            return Err("OIDC issue time or authorized party rejected".into());
        }
        let lifetime = claims
            .expiration()
            .timestamp()
            .saturating_sub(now)
            .clamp(0, 28800) as u64;
        if lifetime == 0 || !config.bindings.contains_key(claims.subject().as_str()) {
            return Err("OIDC identity is not provisioned".into());
        }
        Ok((claims.subject().as_str().to_owned(), lifetime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            issuer: "https://issuer.example".into(),
            client_id: "client".into(),
            redirect_uri: "https://relay.tail123.ts.net/v1/web/oidc/callback".into(),
            client_secret_env: None,
            allowed_origins: vec![],
            bindings: BTreeMap::new(),
        }
    }
    #[test]
    fn configuration_requires_private_callback_and_explicit_provider_origins() {
        let config = config();
        assert!(config.validate().is_ok());
        assert!(config.allows("https://issuer.example/keys"));
        for address in [
            "http://issuer.example/keys",
            "https://evil.example/keys",
            "https://issuer.example@evil.example/keys",
            "https://user:pass@issuer.example/keys",
        ] {
            assert!(!config.allows(address));
        }
        for redirect in [
            "https://public.example/v1/web/oidc/callback",
            "http://relay.tail123.ts.net/v1/web/oidc/callback",
            "http://192.168.1.10/v1/web/oidc/callback",
            "https://relay.tail123.ts.net/v1/web/oidc/callback?extra=1",
        ] {
            let mut c = config.clone();
            c.redirect_uri = redirect.into();
            assert!(c.validate().is_err(), "{redirect}");
        }
        let mut c = config.clone();
        c.allowed_origins.push("https://keys.example/path".into());
        assert!(c.validate().is_err());
        c.allowed_origins = vec!["https://keys.example".into()];
        assert!(c.validate().is_ok());
        assert!(c.allows("https://keys.example/jwks"));
    }
}
