//! OAuth discovery and explicit authorization-code flow. No browser navigation
//! or automatic retry of MCP operations is performed.
use crate::Error;
use dunlin_http::{Agent, AgentBuilder};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    time::{Duration, Instant},
};
use url::Url;

const MAX_METADATA: usize = 64 * 1024;
mod client_metadata;
mod flow;
pub use client_metadata::ClientMetadataDocument;
pub use flow::persistence;
pub use flow::{
    AuthContext, AuthorizationFlow, AuthorizationRequest, CredentialStatus, Registration, TokenSet,
};

pub struct Challenge {
    pub resource_metadata: Option<String>,
    pub scopes: Option<Vec<String>>,
}
pub struct DiscoveredResource {
    pub metadata: ResourceMetadata,
    pub challenge: Option<Challenge>,
}
impl DiscoveredResource {
    pub fn requested_scopes(&self) -> Vec<String> {
        self.metadata.requested_scopes(self.challenge.as_ref())
    }
}
impl Challenge {
    /// All WWW-Authenticate values from one response. Quoted commas/escapes
    /// are parsed before selecting Bearer; ambiguous Bearer challenges fail.
    pub fn parse(headers: &[&str]) -> Result<Option<Self>, Error> {
        if headers.len() > 8 || headers.iter().map(|s| s.len()).sum::<usize>() > 16384 {
            return Err(Error::Limit);
        }
        let mut bearer = None;
        let mut params = BTreeMap::new();
        for header in headers {
            let mut active = false;
            if !header
                .bytes()
                .all(|b| (0x20..=0x7e).contains(&b) || b == b'\t')
            {
                return Err(Error::Protocol);
            }
            let mut quoted = false;
            let mut escaped = false;
            let mut start = 0;
            let mut parts = Vec::new();
            for (i, byte) in header.bytes().enumerate() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quoted && byte == b'\\' {
                    escaped = true;
                    continue;
                }
                if byte == b'"' {
                    quoted = !quoted;
                }
                if byte == b',' && !quoted {
                    parts.push(&header[start..i]);
                    start = i + 1;
                }
            }
            if quoted || escaped {
                return Err(Error::Protocol);
            }
            parts.push(&header[start..]);
            for part in parts {
                let part = part.trim();
                if part.is_empty() {
                    return Err(Error::Protocol);
                }
                let equals = part.find('=');
                let space = part.find([' ', '\t']);
                let new_scheme = equals.is_none_or(|e| !part[..e].trim().bytes().all(token));
                let param = if new_scheme {
                    let (scheme, rest) = space
                        .map(|i| (&part[..i], part[i..].trim()))
                        .unwrap_or((part, ""));
                    if !scheme.bytes().all(token) {
                        return Err(Error::Protocol);
                    }
                    active = scheme.eq_ignore_ascii_case("bearer");
                    if active {
                        if bearer.is_some() {
                            return Err(Error::Protocol);
                        }
                        bearer = Some(());
                    }
                    rest
                } else {
                    part
                };
                if active && !param.is_empty() {
                    let (name, value) = param.split_once('=').ok_or(Error::Protocol)?;
                    let name = name.trim();
                    if name.is_empty() || !name.bytes().all(token) {
                        return Err(Error::Protocol);
                    }
                    let value = auth_value(value.trim())?;
                    if params.insert(name.to_ascii_lowercase(), value).is_some() {
                        return Err(Error::Protocol);
                    }
                }
            }
        }
        if bearer.is_none() {
            return Ok(None);
        }
        let resource_metadata = params.remove("resource_metadata");
        let scopes = params
            .remove("scope")
            .map(|s| scope_string(&s))
            .transpose()?;
        Ok(Some(Self {
            resource_metadata,
            scopes,
        }))
    }
}
fn token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn auth_value(value: &str) -> Result<String, Error> {
    if value.starts_with('"') {
        if value.len() < 2 || !value.ends_with('"') {
            return Err(Error::Protocol);
        }
        let mut output = String::new();
        let mut escaped = false;
        for c in value[1..value.len() - 1].chars() {
            if escaped {
                output.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return Err(Error::Protocol);
            } else {
                output.push(c);
            }
        }
        if escaped {
            return Err(Error::Protocol);
        }
        Ok(output)
    } else if !value.is_empty() && value.bytes().all(token) {
        Ok(value.into())
    } else {
        Err(Error::Protocol)
    }
}
fn scope_string(value: &str) -> Result<Vec<String>, Error> {
    let values: Vec<String> = value.split(' ').map(str::to_owned).collect();
    scopes(&values)?;
    Ok(values)
}
fn scopes(values: &[String]) -> Result<(), Error> {
    if values.len() > 64 {
        return Err(Error::Limit);
    }
    let mut seen = HashSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
            || !seen.insert(value)
        {
            return Err(Error::Protocol);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Option<Vec<String>>,
}
impl ResourceMetadata {
    fn validate(&self, endpoint: &Url) -> Result<(), Error> {
        let resource = secure_url(&self.resource, false)?;
        let path = resource.path().trim_end_matches('/');
        if resource.origin() != endpoint.origin()
            || !(path.is_empty()
                || endpoint.path() == path
                || endpoint
                    .path()
                    .strip_prefix(path)
                    .is_some_and(|tail| tail.starts_with('/')))
        {
            return Err(Error::Protocol);
        }
        if self.authorization_servers.is_empty() || self.authorization_servers.len() > 8 {
            return Err(Error::Limit);
        }
        let mut seen = HashSet::new();
        for issuer in &self.authorization_servers {
            secure_url(issuer, false)?;
            if !seen.insert(issuer) {
                return Err(Error::Protocol);
            }
        }
        if let Some(values) = &self.scopes_supported {
            scopes(values)?;
        }
        Ok(())
    }
    pub fn requested_scopes(&self, challenge: Option<&Challenge>) -> Vec<String> {
        challenge
            .and_then(|c| c.scopes.clone())
            .or_else(|| self.scopes_supported.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
pub struct AuthorizationMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
    #[serde(default)]
    pub client_id_metadata_document_supported: bool,
    pub scopes_supported: Option<Vec<String>>,
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    pub revocation_endpoint: Option<String>,
    pub revocation_endpoint_auth_methods_supported: Option<Vec<String>>,
}

/// Operator-provided origin allowlist. Discovered metadata cannot widen it.
pub struct Discovery {
    endpoint: Url,
    allowed: HashSet<String>,
    agent: Agent,
    started: Instant,
}
impl Discovery {
    /// Read-only unauthenticated MCP probe. Never carries a token or retries a
    /// tool. Uses every WWW-Authenticate value from an actual 401 response.
    pub fn protected_resource_from_server(&self) -> Result<DiscoveredResource, Error> {
        let left = Duration::from_secs(30).saturating_sub(self.started.elapsed());
        if left.is_zero() {
            return Err(Error::Transport);
        }
        let prepared = crate::metadata::request(1, "server/discover", serde_json::json!({}))?;
        let mut request = self
            .agent
            .post(self.endpoint.as_str())
            .set("Accept", "application/json, text/event-stream")
            .set("Content-Type", "application/json")
            .timeout(left.min(Duration::from_secs(10)));
        for (name, value) in prepared.headers {
            request = request.set(&name, &value);
        }
        let response = match request.send_json(prepared.body) {
            Ok(response) => response,
            Err(dunlin_http::Error::Status(_, response)) => *response,
            Err(_) => return Err(Error::Transport),
        };
        if response.status() != 401 {
            return Err(if response.status() == 200 {
                Error::Unsupported
            } else {
                Error::Http(response.status())
            });
        }
        let headers = response
            .header_values("WWW-Authenticate")
            .take(9)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Protocol)?;
        let challenge = Challenge::parse(&headers)?;
        drop(headers);
        drop(response);
        let metadata = self.protected_resource(challenge.as_ref())?;
        Ok(DiscoveredResource {
            metadata,
            challenge,
        })
    }
    pub fn new(endpoint: &str, allowed_authorization_origins: &[String]) -> Result<Self, Error> {
        crate::validate_endpoint(endpoint)?;
        if allowed_authorization_origins.len() > 8 {
            return Err(Error::Limit);
        }
        let endpoint = secure_url(endpoint, false)?;
        let mut allowed = HashSet::from([endpoint.origin().ascii_serialization()]);
        for origin in allowed_authorization_origins {
            let url = secure_url(origin, false)?;
            if url.path() != "/" {
                return Err(Error::Endpoint);
            }
            allowed.insert(url.origin().ascii_serialization());
        }
        Ok(Self {
            endpoint,
            allowed,
            agent: AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0)
                .build(),
            started: Instant::now(),
        })
    }
    fn allowed_url(&self, value: &str, query: bool) -> Result<Url, Error> {
        let url = secure_url(value, query)?;
        if !self.allowed.contains(&url.origin().ascii_serialization()) {
            return Err(Error::Unsupported);
        }
        Ok(url)
    }
    pub fn protected_resource(
        &self,
        challenge: Option<&Challenge>,
    ) -> Result<ResourceMetadata, Error> {
        let urls = match challenge.and_then(|c| c.resource_metadata.as_ref()) {
            Some(url) => vec![self.allowed_url(url, true)?],
            None => resource_urls(&self.endpoint),
        };
        let metadata: ResourceMetadata = self.fetch_first(urls)?;
        metadata.validate(&self.endpoint)?;
        Ok(metadata)
    }
    pub fn authorization_server(
        &self,
        resource: &ResourceMetadata,
        index: usize,
    ) -> Result<AuthorizationMetadata, Error> {
        resource.validate(&self.endpoint)?;
        let issuer = resource
            .authorization_servers
            .get(index)
            .ok_or(Error::Protocol)?;
        let url = self.allowed_url(issuer, false)?;
        let metadata: AuthorizationMetadata = self.fetch_first(issuer_urls(&url))?;
        if metadata.issuer != *issuer {
            return Err(Error::Protocol);
        }
        self.allowed_url(&metadata.authorization_endpoint, true)?;
        self.allowed_url(&metadata.token_endpoint, true)?;
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
        if let Some(values) = &metadata.scopes_supported {
            scopes(values)?;
        }
        Ok(metadata)
    }
    fn fetch_first<T: serde::de::DeserializeOwned>(&self, urls: Vec<Url>) -> Result<T, Error> {
        for url in urls {
            let left = Duration::from_secs(30).saturating_sub(self.started.elapsed());
            if left.is_zero() {
                return Err(Error::Transport);
            }
            let response = match self
                .agent
                .get(url.as_str())
                .set("Accept", "application/json")
                .timeout(left.min(Duration::from_secs(10)))
                .call()
            {
                Ok(response) => response,
                Err(dunlin_http::Error::Status(_, response)) => *response,
                Err(_) => return Err(Error::Transport),
            };
            if matches!(response.status(), 404 | 405) {
                continue;
            }
            if response.status() != 200 {
                return Err(Error::Http(response.status()));
            }
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
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(MAX_METADATA as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Transport)?;
            if bytes.len() > MAX_METADATA {
                return Err(Error::Limit);
            }
            return serde_json::from_slice(&bytes).map_err(|_| Error::Protocol);
        }
        Err(Error::Unsupported)
    }
}
fn secure_url(value: &str, query: bool) -> Result<Url, Error> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(Error::Endpoint);
    }
    let url = Url::parse(value).map_err(|_| Error::Endpoint)?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.host().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || (!query && url.query().is_some())
    {
        return Err(Error::Endpoint);
    }
    Ok(url)
}
fn resource_urls(endpoint: &Url) -> Vec<Url> {
    let mut urls = Vec::new();
    if endpoint.path() != "/" {
        let mut u = endpoint.clone();
        u.set_path(&format!(
            "/.well-known/oauth-protected-resource{}",
            endpoint.path()
        ));
        urls.push(u);
    }
    let mut root = endpoint.clone();
    root.set_path("/.well-known/oauth-protected-resource");
    urls.push(root);
    urls
}
fn issuer_urls(issuer: &Url) -> Vec<Url> {
    let path = if issuer.path() == "/" {
        ""
    } else {
        issuer.path()
    };
    let mut oauth = issuer.clone();
    oauth.set_path(&format!("/.well-known/oauth-authorization-server{path}"));
    let mut oidc = issuer.clone();
    oidc.set_path(&format!("/.well-known/openid-configuration{path}"));
    let mut urls = vec![oauth, oidc];
    if !path.is_empty() {
        let mut appended = issuer.clone();
        appended.set_path(&format!(
            "{}/.well-known/openid-configuration",
            path.trim_end_matches('/')
        ));
        urls.push(appended);
    }
    urls
}

#[cfg(test)]
mod tests;
