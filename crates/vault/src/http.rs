//! Outbound HTTP with a deliberately narrow policy surface.

use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use dunlin_domain::vault::{HttpMethod, Injection, Operation, SecretPolicy};
use dunlin_http as ureq;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

pub const MAX_REQUEST_BODY: usize = 256 * 1024;
pub const MAX_RESPONSE_BODY: usize = 512 * 1024;
const MAX_URL_LEN: usize = 8192;
const MAX_HEADER_VALUE: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HttpInput {
    pub secret: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Named policy operation. When set, method/url/body must be omitted.
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HttpOutput {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct ValidatedHttp {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HttpPlan {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl Drop for HttpPlan {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for (_, value) in &mut self.headers {
            value.zeroize();
        }
        self.url.zeroize();
        if let Some(body) = &mut self.body {
            body.zeroize();
        }
    }
}

pub trait HttpExecutor: Send + Sync {
    fn execute(&self, plan: &HttpPlan) -> Result<HttpOutput, String>;
}

/// Reject loopback, private, link-local and metadata addresses before connect.
/// The resolver is used by ureq, so the checked addresses are the ones dialed.
#[derive(Debug)]
pub struct PublicResolver;

impl ureq::Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::ResolverError> {
        let addresses = ureq::unversioned::resolver::DefaultResolver::default()
            .resolve(uri, config, timeout)?;
        if let Some(address) = addresses.iter().find(|address| !public_ip(address.ip())) {
            return Err(ureq::ResolverError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("refusing to connect to non-public address {}", address.ip()),
            )));
        }
        Ok(addresses)
    }
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return false;
    }
    // CGNAT, protocol assignments, benchmarking, reserved and this-network.
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return false;
    }
    if octets[0] == 0 || octets[0] >= 240 {
        return false;
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return false;
    }
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return false;
    }
    true
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return public_ipv4(mapped);
    }
    let segments = ip.segments();
    // Unique local fc00::/7 and link-local fe80::/10.
    if segments[0] & 0xfe00 == 0xfc00 || segments[0] & 0xffc0 == 0xfe80 {
        return false;
    }
    // Documentation prefix 2001:db8::/32.
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return false;
    }
    true
}

fn forbidden_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "upgrade"
            | "expect"
            | "te"
            | "trailer"
            | "proxy-connection"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-real-ip"
    )
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
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
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return path.starts_with('/');
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn validate_path(prefix: &str, path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err("The request path is invalid".into());
    }
    if path
        .chars()
        .any(|c| c == '\r' || c == '\n' || (c.is_control() && c != '\t'))
    {
        return Err("The request path contains control characters".into());
    }
    for segment in path.split('/') {
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map_err(|_| "The request path is not valid UTF-8".to_string())?;
        if decoded == "." || decoded == ".." || decoded.contains('/') || decoded.contains('\\') {
            return Err("The request path contains a traversal segment".into());
        }
    }
    if !path_prefix_matches(prefix, path) {
        return Err(format!(
            "The request path is outside this credential's allowed prefix ({prefix})"
        ));
    }
    Ok(())
}

pub fn validate_url(policy: &SecretPolicy, value: &str) -> Result<Url, String> {
    if value.len() > MAX_URL_LEN {
        return Err("The request URL is too long".into());
    }
    let url = Url::parse(value).map_err(|_| "The request URL is invalid".to_string())?;
    if url.scheme() != "https" {
        return Err("Credentials may only be sent over HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The request URL must not contain credentials".into());
    }
    if url.fragment().is_some() {
        return Err("The request URL must not contain a fragment".into());
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err("Only the default HTTPS port is allowed".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "The request URL needs a host".to_string())?;
    let host = normalize_host(host);
    if host.parse::<IpAddr>().is_ok() {
        return Err("IP address hosts are not allowed; use a hostname".into());
    }
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".localdomain")
    {
        return Err("Local hostnames are not allowed".into());
    }
    if !policy
        .allowed_hosts
        .iter()
        .any(|allowed| normalize_host(allowed) == host)
    {
        return Err(format!(
            "The host is not allowed for this credential. Allowed: {}",
            policy
                .allowed_hosts
                .iter()
                .map(|host| host.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    validate_path(&policy.path_prefix, url.path())?;
    Ok(url)
}

fn render_operation_url(
    operation: &Operation,
    params: &BTreeMap<String, String>,
) -> Result<String, String> {
    for parameter in &operation.parameters {
        let value = params
            .get(parameter)
            .ok_or_else(|| format!("Missing operation parameter '{parameter}'"))?;
        if value.len() > 512 || value.contains(['\0', '\r', '\n']) {
            return Err(format!("Operation parameter '{parameter}' is invalid"));
        }
    }
    if params
        .keys()
        .any(|name| !operation.parameters.contains(name))
    {
        return Err("An operation received an undeclared parameter".into());
    }
    let mut path = String::new();
    let mut rest = operation.path.as_str();
    while let Some(start) = rest.find('{') {
        path.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| "Operation path has an unclosed parameter".to_string())?;
        let name = &after[..end];
        let value = params
            .get(name)
            .ok_or_else(|| format!("Missing operation parameter '{name}'"))?;
        path.push_str(
            &percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                .to_string(),
        );
        rest = &after[end + 1..];
    }
    path.push_str(rest);
    Ok(format!("https://{}{}", operation.host, path))
}

pub fn validate_input(policy: &SecretPolicy, input: &HttpInput) -> Result<ValidatedHttp, String> {
    let operation = match input.operation.as_deref() {
        Some(name) => Some(
            policy
                .operations
                .iter()
                .find(|operation| operation.name == name)
                .ok_or_else(|| format!("Unknown operation '{name}'"))?,
        ),
        None => None,
    };
    let (method, url, body) = if let Some(operation) = operation {
        if !input.method.is_empty()
            || !input.url.is_empty()
            || input.body.is_some()
            || input.operation.as_deref() != Some(operation.name.as_str())
        {
            return Err("An operation request must not also set method, url or body".into());
        }
        operation.validate(policy)?;
        let url = render_operation_url(operation, &input.params)?;
        (operation.method, validate_url(policy, &url)?, None)
    } else {
        if !input.params.is_empty() {
            return Err("Operation parameters require an operation name".into());
        }
        let method = HttpMethod::parse(&input.method)
            .ok_or_else(|| "Unsupported HTTP method".to_string())?;
        if !policy.allowed_methods.contains(&method) {
            return Err(format!(
                "{} is not allowed for this credential. Allowed: {}",
                method.as_str(),
                policy
                    .allowed_methods
                    .iter()
                    .map(|method| method.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let url = validate_url(policy, &input.url)?;
        (method, url, input.body.clone())
    };
    if !policy.allowed_methods.contains(&method) {
        return Err("The operation method is not allowed for this credential".into());
    }
    let injected_header = match &policy.injection {
        Injection::Header { name, .. } => Some(name.to_ascii_lowercase()),
        Injection::Basic { .. } => Some("authorization".into()),
        Injection::Query { .. } => None,
        Injection::OAuth2 { .. } | Injection::AwsSigV4 { .. } => Some("authorization".into()),
    };
    let allowed = policy
        .allowed_headers
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let mut headers = Vec::new();
    for (name, value) in &input.headers {
        let lower = name.to_ascii_lowercase();
        if !valid_header_name(name) {
            return Err(format!("Invalid request header name: {name}"));
        }
        if forbidden_header(&lower) {
            return Err(format!(
                "The {name} header is managed by Dunlin and cannot be set"
            ));
        }
        if injected_header.as_deref() == Some(lower.as_str()) {
            return Err("The credential header is injected by Dunlin and cannot be set".into());
        }
        if !allowed.contains(&lower) {
            return Err(format!(
                "The {name} header is not allowed for this credential"
            ));
        }
        if value.len() > MAX_HEADER_VALUE || value.contains(['\0', '\r', '\n']) {
            return Err(format!("The {name} header value is invalid"));
        }
        headers.push((name.clone(), value.clone()));
    }
    if let Some(body) = &body {
        if method == HttpMethod::Get || method == HttpMethod::Head {
            return Err("GET and HEAD requests cannot have a body".into());
        }
        if body.len() > MAX_REQUEST_BODY {
            return Err("The request body is too large".into());
        }
    }
    if let Injection::Query { name } = &policy.injection
        && url.query_pairs().any(|(key, _)| key == name.as_str())
    {
        return Err("The request URL already contains the credential query parameter".into());
    }
    Ok(ValidatedHttp {
        method,
        url,
        headers,
        body,
    })
}

pub fn apply_injection(
    policy: &SecretPolicy,
    mut validated: ValidatedHttp,
    value: &str,
) -> HttpPlan {
    let mut headers = validated.headers;
    match &policy.injection {
        Injection::Header { name, prefix } => {
            headers.push((name.clone(), format!("{prefix}{value}")));
        }
        Injection::Basic { username } => {
            let pair = format!("{username}:{value}");
            headers.push((
                "Authorization".into(),
                format!("Basic {}", BASE64.encode(pair.as_bytes())),
            ));
        }
        Injection::Query { name } => {
            validated.url.query_pairs_mut().append_pair(name, value);
        }
        Injection::OAuth2 { .. } => {
            headers.push(("Authorization".into(), format!("Bearer {value}")));
        }
        Injection::AwsSigV4 { .. } => {}
    }
    let body = validated.body.map(|body| body.into_bytes());
    HttpPlan {
        method: validated.method.as_str(),
        url: validated.url.to_string(),
        headers,
        body,
    }
}

/// Exchange an OAuth refresh token for a short-lived access token. The token
/// endpoint response is parsed locally and never returned to the model.
pub fn refresh_oauth(
    executor: &dyn HttpExecutor,
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<Zeroizing<String>, String> {
    let url = Url::parse(token_url).map_err(|_| "The OAuth token URL is invalid".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("OAuth refresh requires a plain HTTPS token URL".into());
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", refresh_token)
        .append_pair("client_id", client_id)
        .finish();
    let plan = HttpPlan {
        method: "POST",
        url: token_url.to_string(),
        headers: vec![
            (
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
            ("Accept".into(), "application/json".into()),
        ],
        body: Some(body.into_bytes()),
    };
    let output = executor.execute(&plan)?;
    if !(200..300).contains(&output.status) {
        return Err(format!(
            "OAuth token refresh failed (HTTP {})",
            output.status
        ));
    }
    let value: serde_json::Value = serde_json::from_str(&output.body)
        .map_err(|_| "OAuth token response was not JSON".to_string())?;
    let token = value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "OAuth token response did not contain an access token".to_string())?;
    Ok(Zeroizing::new(token.to_string()))
}

fn aws_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Sign a plan with AWS Signature Version 4. The stored value is the secret
/// access key; the access key ID, region and service live in the policy.
pub fn sign_aws(policy: &SecretPolicy, plan: &mut HttpPlan, secret: &str) -> Result<(), String> {
    sign_aws_at(policy, plan, secret, Utc::now())
}

fn sign_aws_at(
    policy: &SecretPolicy,
    plan: &mut HttpPlan,
    secret: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), String> {
    let Injection::AwsSigV4 {
        access_key_id,
        region,
        service,
    } = &policy.injection
    else {
        return Ok(());
    };
    let url = Url::parse(&plan.url).map_err(|_| "AWS request URL is invalid".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "AWS request URL has no host".to_string())?;
    let date = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let payload_hash = hex(&Sha256::digest(plan.body.as_deref().unwrap_or(&[])));
    let mut query = url
        .query_pairs()
        .map(|(key, value)| (aws_encode(&key), aws_encode(&value)))
        .collect::<Vec<_>>();
    query.sort();
    let canonical_query = query
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let canonical_uri = if url.path().is_empty() {
        "/".to_string()
    } else {
        url.path().to_string()
    };
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
    let canonical_request = format!(
        "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        plan.method
    );
    let credential_scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signing_key = hmac_sha256(
        &hmac_sha256(
            &hmac_sha256(
                &hmac_sha256(format!("AWS4{secret}").as_bytes(), &date),
                region,
            ),
            service,
        ),
        "aws4_request",
    );
    let signature = hex(&hmac_sha256(&signing_key, &string_to_sign));
    plan.headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("x-amz-date")
            && !name.eq_ignore_ascii_case("x-amz-content-sha256")
            && !name.eq_ignore_ascii_case("authorization")
    });
    plan.headers.push(("x-amz-date".into(), amz_date));
    plan.headers
        .push(("x-amz-content-sha256".into(), payload_hash));
    plan.headers.push((
        "Authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={access_key_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
    ));
    Ok(())
}

pub struct UreqExecutor {
    agent: ureq::Agent,
}

impl Default for UreqExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqExecutor {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .resolver(PublicResolver)
            .build();
        Self { agent }
    }
}

impl HttpExecutor for UreqExecutor {
    fn execute(&self, plan: &HttpPlan) -> Result<HttpOutput, String> {
        let mut request = self.agent.request(plan.method, &plan.url);
        for (name, value) in &plan.headers {
            request = request.set(name, value);
        }
        let result = match &plan.body {
            Some(body) => request.send_bytes(body),
            None => request.call(),
        };
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                if (300..400).contains(&status) {
                    return Err(
                        "The server attempted a redirect; Dunlin does not follow redirects with credentials"
                            .into(),
                    );
                }
                *response
            }
            Err(ureq::Error::Transport(error)) => {
                let reason = match error.kind() {
                    ureq::ErrorKind::Dns => "DNS resolution failed",
                    ureq::ErrorKind::ConnectionFailed => "the connection failed",
                    ureq::ErrorKind::Io => "a network I/O error occurred",
                    ureq::ErrorKind::InsecureRequestHttpsOnly => "the request was not HTTPS",
                    ureq::ErrorKind::TooManyRedirects => "too many redirects",
                    ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
                        "the URL was not supported"
                    }
                    _ => "the request failed",
                };
                // Do not copy ureq's transport error verbatim: it can contain
                // the full URL, including query parameters.
                return Err(format!("The request could not be completed: {reason}"));
            }
        };
        let status = response.status();
        let content_type = response.header("content-type").map(str::to_string);
        let textual = content_type.as_deref().is_none_or(|value| {
            let value = value.to_ascii_lowercase();
            value.starts_with("text/")
                || value.contains("json")
                || value.contains("xml")
                || value.contains("javascript")
                || value.contains("x-www-form-urlencoded")
                || value.contains("yaml")
        });
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE_BODY as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read the response body".to_string())?;
        let truncated = bytes.len() > MAX_RESPONSE_BODY;
        if truncated {
            bytes.truncate(MAX_RESPONSE_BODY);
        }
        let body = if textual {
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            format!(
                "[binary response omitted: {} bytes{}]",
                bytes.len(),
                if truncated { ", truncated" } else { "" }
            )
        };
        Ok(HttpOutput {
            status,
            content_type,
            body,
            truncated,
        })
    }
}

/// Replace the exact credential and common encodings in a response or error.
pub fn redact(text: &str, secret: &str, basic_pair: Option<&str>) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    let mut output = text.to_string();
    let mut needles = vec![
        secret.to_string(),
        percent_encoding::utf8_percent_encode(secret, percent_encoding::NON_ALPHANUMERIC)
            .to_string(),
        serde_json::to_string(secret)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string(),
        BASE64.encode(secret.as_bytes()),
    ];
    if let Some(pair) = basic_pair {
        needles.push(BASE64.encode(pair.as_bytes()));
    }
    needles.sort();
    needles.dedup();
    needles.retain(|needle| !needle.is_empty());
    for needle in needles {
        output = output.replace(&needle, "[REDACTED]");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> SecretPolicy {
        SecretPolicy {
            version: 1,
            managed: true,
            injection: Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            },
            allowed_hosts: vec!["api.github.com".into()],
            path_prefix: "/repos/acme".into(),
            allowed_methods: vec![HttpMethod::Get, HttpMethod::Post],
            allowed_headers: vec!["Accept".into(), "Content-Type".into()],
            auto_approve: true,
            expires_at: None,
            project_ids: vec!["project".into()],
            recipes: Vec::new(),
            operations: Vec::new(),
        }
    }

    #[test]
    fn url_policy_is_exact_and_rejects_traversal_redirect_hosts_and_private_targets() {
        let secret = secret();
        assert!(validate_url(&secret, "https://api.github.com/repos/acme/app").is_ok());
        assert!(validate_url(&secret, "https://API.GITHUB.COM/repos/acme/app").is_ok());
        for url in [
            "http://api.github.com/repos/acme/app",
            "https://evil.test/repos/acme/app",
            "https://api.github.com.evil.test/repos/acme/app",
            "https://api.github.com/other",
            "https://api.github.com/repos/acme/../secret",
            "https://api.github.com/repos/acme/%2e%2e/secret",
            "https://user:pass@api.github.com/repos/acme/app",
            "https://api.github.com:444/repos/acme/app",
            "https://127.0.0.1/repos/acme/app",
            "https://localhost/repos/acme/app",
            "https://api.github.com/repos/acme/app#fragment",
        ] {
            assert!(validate_url(&secret, url).is_err(), "{url}");
        }
        let mut limited = secret;
        limited.allowed_hosts = vec!["api.github.com".into()];
        limited.path_prefix = "/".into();
        assert!(validate_url(&limited, "https://api.github.com/other").is_ok());
    }

    #[test]
    fn named_operations_render_policy_paths_and_reject_free_form_overrides() {
        let mut policy = secret();
        policy.operations = vec![Operation {
            name: "issues".into(),
            description: "List issues".into(),
            method: HttpMethod::Get,
            host: "api.github.com".into(),
            path: "/repos/acme/{owner}/{repo}/issues".into(),
            parameters: vec!["owner".into(), "repo".into()],
        }];
        let input = HttpInput {
            secret: "github".into(),
            operation: Some("issues".into()),
            params: BTreeMap::from([
                ("owner".into(), "acme".into()),
                ("repo".into(), "app".into()),
            ]),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
        };
        let validated = validate_input(&policy, &input).unwrap();
        assert_eq!(
            validated.url.as_str(),
            "https://api.github.com/repos/acme/acme/app/issues"
        );

        let mut missing = input.clone();
        missing.params.remove("repo");
        assert!(validate_input(&policy, &missing).is_err());

        let mut extra = input.clone();
        extra.params.insert("host".into(), "evil.test".into());
        assert!(validate_input(&policy, &extra).is_err());

        let mut traversal = input.clone();
        traversal
            .params
            .insert("repo".into(), "app/../../secret".into());
        assert!(validate_input(&policy, &traversal).is_err());

        let mut overridden = input.clone();
        overridden.url = "https://evil.test/".into();
        assert!(validate_input(&policy, &overridden).is_err());

        let mut undeclared = input;
        undeclared.operation = Some("delete-everything".into());
        assert!(validate_input(&policy, &undeclared).is_err());
    }

    type SeenRequest = Option<(String, String, Option<Vec<u8>>)>;

    struct FakeExecutor {
        status: u16,
        body: &'static str,
        seen: std::sync::Mutex<SeenRequest>,
    }

    impl HttpExecutor for FakeExecutor {
        fn execute(&self, plan: &HttpPlan) -> Result<HttpOutput, String> {
            *self.seen.lock().unwrap() =
                Some((plan.method.to_string(), plan.url.clone(), plan.body.clone()));
            Ok(HttpOutput {
                status: self.status,
                content_type: Some("application/json".into()),
                body: self.body.into(),
                truncated: false,
            })
        }
    }

    #[test]
    fn oauth_refresh_exchanges_the_refresh_token_without_exposing_it() {
        let executor = FakeExecutor {
            status: 200,
            body: r#"{"access_token":"short-lived-token","expires_in":3600}"#,
            seen: Default::default(),
        };
        let token = refresh_oauth(
            &executor,
            "https://auth.example.com/token",
            "client-1",
            "refresh-token-value",
        )
        .unwrap();
        assert_eq!(token.as_str(), "short-lived-token");
        let (method, url, body) = executor.seen.lock().unwrap().clone().unwrap();
        assert_eq!(method, "POST");
        assert_eq!(url, "https://auth.example.com/token");
        let body = String::from_utf8(body.unwrap()).unwrap();
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("refresh_token=refresh-token-value"));
        assert!(body.contains("client_id=client-1"));

        let failed = FakeExecutor {
            status: 500,
            body: r#"{"access_token":"must-not-leak"}"#,
            seen: Default::default(),
        };
        let error = refresh_oauth(
            &failed,
            "https://auth.example.com/token",
            "client-1",
            "refresh-token-value",
        )
        .unwrap_err();
        assert!(!error.contains("must-not-leak"));
        assert!(!error.contains("refresh-token-value"));
    }

    #[test]
    fn aws_sigv4_headers_match_a_fixed_reference_signature() {
        let mut policy = secret();
        policy.injection = Injection::AwsSigV4 {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            region: "us-east-1".into(),
            service: "execute-api".into(),
        };
        let mut plan = HttpPlan {
            method: "GET",
            url: "https://api.github.com/repos/acme/app?b=2&a=1".into(),
            headers: Vec::new(),
            body: None,
        };
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2024, 5, 1, 12, 0, 0).unwrap();
        sign_aws_at(
            &policy,
            &mut plan,
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            now,
        )
        .unwrap();
        assert!(
            plan.headers
                .iter()
                .any(|(name, value)| { name == "x-amz-date" && value == "20240501T120000Z" })
        );
        assert!(plan.headers.iter().any(|(name, value)| {
            name == "x-amz-content-sha256"
                && value == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        }));
        let authorization = plan
            .headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20240501/us-east-1/execute-api/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=89c3da934bd33ac726bd55aef6780c90538788d9453cbde3197d04fca9338342"
        );
    }

    #[test]
    fn caller_cannot_set_auth_or_unlisted_headers() {
        let secret = secret();
        let input = HttpInput {
            secret: "github".into(),
            method: "GET".into(),
            url: "https://api.github.com/repos/acme/app".into(),
            headers: BTreeMap::from([("Authorization".into(), "Bearer evil".into())]),
            body: None,
            operation: None,
            params: BTreeMap::new(),
        };
        assert!(validate_input(&secret, &input).is_err());
        let input = HttpInput {
            headers: BTreeMap::from([("X-Evil".into(), "value".into())]),
            ..input
        };
        assert!(validate_input(&secret, &input).is_err());
        let input = HttpInput {
            headers: BTreeMap::from([("Accept".into(), "application/vnd.github+json".into())]),
            ..input
        };
        assert!(validate_input(&secret, &input).is_ok());
    }

    #[test]
    fn method_and_body_limits_are_enforced() {
        let secret = secret();
        let mut input = HttpInput {
            secret: "github".into(),
            method: "DELETE".into(),
            url: "https://api.github.com/repos/acme/app".into(),
            headers: BTreeMap::new(),
            body: None,
            operation: None,
            params: BTreeMap::new(),
        };
        assert!(validate_input(&secret, &input).is_err());
        input.method = "POST".into();
        input.body = Some("x".repeat(MAX_REQUEST_BODY + 1));
        assert!(validate_input(&secret, &input).is_err());
        input.body = Some("{}".into());
        assert!(validate_input(&secret, &input).is_ok());
        input.method = "GET".into();
        assert!(validate_input(&secret, &input).is_err());
    }

    #[test]
    fn injection_and_redaction_cover_common_encodings() {
        let secret = secret();
        let input = HttpInput {
            secret: "github".into(),
            method: "GET".into(),
            url: "https://api.github.com/repos/acme/app".into(),
            headers: BTreeMap::new(),
            body: None,
            operation: None,
            params: BTreeMap::new(),
        };
        let validated = validate_input(&secret, &input).unwrap();
        let plan = apply_injection(&secret, validated, "tok_value-123");
        assert!(
            plan.headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "Bearer tok_value-123")
        );
        let redacted = redact(
            "echo tok_value-123 and tok_value-123 and dG9rX3ZhbHVlLTEyMw==",
            "tok_value-123",
            None,
        );
        assert!(!redacted.contains("tok_value-123"));
        assert!(!redacted.contains("dG9rX3ZhbHVlLTEyMw=="));
    }

    #[test]
    fn resolver_rejects_private_and_metadata_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn ureq3_resolver_checks_the_actual_dial_addresses() {
        use ureq::Resolver;
        for host in ["127.0.0.1", "169.254.169.254", "[::1]"] {
            let uri = format!("http://{host}/").parse().unwrap();
            assert!(
                PublicResolver
                    .resolve(
                        &uri,
                        &ureq::config::Config::default(),
                        ureq::unversioned::transport::NextTimeout::default(),
                    )
                    .is_err()
            );
        }
        let addresses = PublicResolver
            .resolve(
                &"https://1.1.1.1/".parse().unwrap(),
                &ureq::config::Config::default(),
                ureq::unversioned::transport::NextTimeout::default(),
            )
            .unwrap();
        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].ip().to_string(), "1.1.1.1");
        assert_eq!(addresses[0].port(), 443);
    }

    #[test]
    fn ureq3_executor_blocks_local_connections_without_echoing_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let plan = HttpPlan {
            method: "GET",
            url: format!(
                "http://{}/?token=fixture-secret",
                listener.local_addr().unwrap()
            ),
            headers: vec![("Authorization".into(), "Bearer fixture-secret".into())],
            body: None,
        };
        let error = UreqExecutor::new().execute(&plan).unwrap_err();
        assert!(!error.contains("fixture-secret"));
        assert!(!error.contains("127.0.0.1"));
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    }
}
