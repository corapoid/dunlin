//! Operator-controlled MCP providers. Browser requests select an ID, never an
//! arbitrary endpoint, issuer, redirect, origin allowlist, or client secret.
use dunlin_mcp_client::oauth::{AuthorizationFlow, Discovery, Registration};
use dunlin_vault::store::SecretStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub authentication: Authentication,
    pub client_secret_ref: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Public,
    ClientMetadataDocument,
    Basic,
    Post,
}
impl Provider {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || self.name.trim().is_empty()
            || self.name.len() > 120
            || self.name.chars().any(char::is_control)
        {
            return Err("Invalid MCP provider ID or name".into());
        }
        Discovery::new(&self.endpoint, &self.allowed_origins)
            .map_err(|_| "Invalid MCP endpoint or origin allowlist")?;
        match self.authentication {
            Authentication::ClientMetadataDocument => Registration::client_metadata_document(
                &self.issuer,
                &self.client_id,
                &self.redirect_uri,
            ),
            _ => Registration::public(&self.issuer, &self.client_id, &self.redirect_uri),
        }
        .map_err(|_| "Invalid MCP OAuth registration")?;
        let redirect = Url::parse(&self.redirect_uri).map_err(|_| "Invalid MCP callback")?;
        let host = redirect.host_str().unwrap_or("").trim_matches(['[', ']']);
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if redirect.path() != format!("/v1/web/mcp/oauth/{}/callback", self.id)
            || redirect.query().is_some()
            || !(loopback || dunlin_domain::relay::tailscale_hostname(host))
            || self.scopes.len() > 64
            || self.scopes.iter().any(|s| {
                s.is_empty()
                    || s.len() > 256
                    || !s.bytes().all(|b| {
                        b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b)
                    })
            })
            || self
                .scopes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.scopes.len()
        {
            return Err("Invalid MCP callback or scopes".into());
        }
        match (self.authentication, self.client_secret_ref.as_deref()) {
            (Authentication::Public | Authentication::ClientMetadataDocument, None) => {}
            (Authentication::Basic | Authentication::Post, Some(reference))
                if reference.starts_with("mcp-oauth-client:")
                    && reference.len() <= 160
                    && reference.len() > "mcp-oauth-client:".len()
                    && reference
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b)) => {}
            _ => {
                return Err(
                    "MCP client authentication needs its dedicated secret reference".into(),
                );
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::to_vec(self).expect("provider serialization"),
        ))
    }
    pub(crate) fn flow(&self, secrets: &dyn SecretStore) -> Result<AuthorizationFlow, String> {
        self.validate()?;
        let discovery = Discovery::new(&self.endpoint, &self.allowed_origins)
            .map_err(|_| "MCP discovery unavailable")?;
        let discovered = discovery
            .protected_resource_from_server()
            .map_err(|_| "MCP discovery unavailable")?;
        let index = discovered
            .metadata
            .authorization_servers
            .iter()
            .position(|issuer| issuer == &self.issuer)
            .ok_or("Configured MCP issuer not advertised")?;
        let metadata = discovery
            .authorization_server(&discovered.metadata, index)
            .map_err(|_| "MCP issuer metadata unavailable")?;
        let registration = match self.authentication {
            Authentication::Public => {
                Registration::public(&self.issuer, &self.client_id, &self.redirect_uri)
            }
            Authentication::ClientMetadataDocument => Registration::client_metadata_document(
                &self.issuer,
                &self.client_id,
                &self.redirect_uri,
            ),
            Authentication::Basic | Authentication::Post => {
                let secret = secrets
                    .get(
                        self.client_secret_ref
                            .as_deref()
                            .ok_or("Missing MCP client registration")?,
                    )
                    .map_err(|_| "MCP client secret unavailable")?;
                let value = secret.to_string();
                match self.authentication {
                    Authentication::Basic => Registration::confidential_basic(
                        &self.issuer,
                        &self.client_id,
                        &self.redirect_uri,
                        value,
                    ),
                    Authentication::Post => Registration::confidential_post(
                        &self.issuer,
                        &self.client_id,
                        &self.redirect_uri,
                        value,
                    ),
                    Authentication::Public | Authentication::ClientMetadataDocument => {
                        unreachable!()
                    }
                }
            }
        }
        .map_err(|_| "MCP registration unavailable")?;
        discovery
            .authorization_flow(&discovered.metadata, metadata, registration)
            .map_err(|_| "MCP authorization unavailable".into())
    }
}
pub fn validate_registry(providers: &[Provider]) -> Result<(), String> {
    if providers.len() > 16
        || providers
            .iter()
            .map(|p| &p.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != providers.len()
    {
        return Err("MCP provider registry must have at most16 distinct IDs".into());
    }
    for provider in providers {
        provider.validate()?;
    }
    Ok(())
}

/// Export only public configuration, without opening the database/keyring or
/// networking. Hosting this JSON at the configured HTTPS URL is an explicit
/// operator deployment action, independent of the private relay listener.
pub(crate) fn export_client_metadata(args: &[String]) -> Result<String, String> {
    let mut config = None;
    let mut provider = None;
    if args.len() != 4 {
        return Err("Usage: mcp-client-metadata --config PATH --provider ID".into());
    }
    for pair in args.as_chunks::<2>().0 {
        let target = match pair[0].as_str() {
            "--config" => &mut config,
            "--provider" => &mut provider,
            _ => return Err("Unknown MCP metadata export argument".into()),
        };
        if target.replace(pair[1].as_str()).is_some() || pair[1].is_empty() {
            return Err("Duplicate or empty MCP metadata export argument".into());
        }
    }
    let config =
        crate::config::Config::load(std::path::PathBuf::from(config.ok_or("Missing --config")?))?;
    validate_registry(&config.mcp_oauth)?;
    let selected = config
        .mcp_oauth
        .iter()
        .find(|p| Some(p.id.as_str()) == provider)
        .ok_or("Unknown MCP provider")?;
    if !matches!(
        selected.authentication,
        Authentication::ClientMetadataDocument
    ) {
        return Err("MCP metadata export requires client_metadata_document authentication".into());
    }
    let document = dunlin_mcp_client::oauth::ClientMetadataDocument::public(
        &selected.client_id,
        &selected.name,
        std::slice::from_ref(&selected.redirect_uri),
    )
    .map_err(|_| "Invalid MCP client metadata document")?;
    serde_json::to_string_pretty(&document).map_err(|_| "Cannot encode MCP client metadata".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider() -> Provider {
        Provider {
            id: "fixture".into(),
            name: "Fixture".into(),
            endpoint: "http://127.0.0.1:1/mcp".into(),
            issuer: "http://127.0.0.1:1/issuer".into(),
            client_id: "client".into(),
            redirect_uri: "https://build.tail123.ts.net/v1/web/mcp/oauth/fixture/callback".into(),
            allowed_origins: vec![],
            scopes: vec!["files:read".into()],
            authentication: Authentication::Public,
            client_secret_ref: None,
        }
    }
    #[test]
    fn mcp_provider_configuration_pins_callback_ids_origins_scopes_and_client_secret_namespace() {
        let valid = provider();
        valid.validate().unwrap();
        for fault in [
            "id",
            "callback_host",
            "callback_path",
            "query",
            "scope",
            "auth",
            "secret",
        ] {
            let mut invalid = valid.clone();
            match fault {
                "id" => invalid.id = "../../other".into(),
                "callback_host" => {
                    invalid.redirect_uri =
                        "https://public.example/v1/web/mcp/oauth/fixture/callback".into()
                }
                "callback_path" => {
                    invalid.redirect_uri =
                        "https://build.tail123.ts.net/v1/web/oidc/callback".into()
                }
                "query" => invalid.redirect_uri.push_str("?code=preloaded"),
                "scope" => invalid.scopes.push("files:read".into()),
                "auth" => invalid.authentication = Authentication::Basic,
                _ => {
                    invalid.authentication = Authentication::Post;
                    invalid.client_secret_ref =
                        Some("mcp-oauth:01234567-89ab-7cde-8123-456789abcdef".into());
                }
            }
            assert!(invalid.validate().is_err(), "{fault}");
        }
        let mut confidential = valid.clone();
        confidential.authentication = Authentication::Basic;
        confidential.client_secret_ref = Some("mcp-oauth-client:fixture".into());
        confidential.validate().unwrap();
        assert_ne!(valid.digest(), confidential.digest());
        assert!(validate_registry(&[valid.clone(), valid]).is_err());
        let mut raw = serde_json::to_value(provider()).unwrap();
        raw["client_secret"] = serde_json::json!("fixture-private-secret");
        assert!(serde_json::from_value::<Provider>(raw).is_err());
    }
    #[test]
    fn mcp_provider_registry_loads_from_operator_toml_without_reading_keyring_or_network() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.toml");
        std::fs::write(
            &path,
            r#"[[mcp_oauth]]
id="fixture"
name="Fixture MCP"
endpoint="http://127.0.0.1:1/mcp"
issuer="http://127.0.0.1:1/issuer"
client_id="client"
redirect_uri="https://build.tail123.ts.net/v1/web/mcp/oauth/fixture/callback"
scopes=["files:read"]
"#,
        )
        .unwrap();
        let config = crate::config::Config::load(path).unwrap();
        assert_eq!(config.mcp_oauth.len(), 1);
        config.mcp_oauth[0].validate().unwrap();
    }
    #[test]
    fn mcp_cimd_export_is_public_configuration_and_rejects_ambiguous_arguments_and_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.toml");
        let mut cimd = provider();
        cimd.authentication = Authentication::ClientMetadataDocument;
        cimd.client_id = "https://client.example/oauth/client.json".into();
        cimd.validate().unwrap();
        let document = toml::to_string(&std::collections::BTreeMap::from([(
            "mcp_oauth",
            vec![cimd.clone()],
        )]))
        .unwrap();
        std::fs::write(&path, document).unwrap();
        let args = vec![
            "--config".into(),
            path.to_str().unwrap().into(),
            "--provider".into(),
            "fixture".into(),
        ];
        let output = export_client_metadata(&args).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "client_id":cimd.client_id,"client_name":cimd.name,"redirect_uris":[cimd.redirect_uri],
                "grant_types":["authorization_code","refresh_token"],"response_types":["code"],
                "token_endpoint_auth_method":"none"
            })
        );
        for bad in [
            vec![],
            vec!["--config".into()],
            vec!["--config".into(), "x".into(), "--config".into(), "y".into()],
            vec![
                "--provider".into(),
                "fixture".into(),
                "--unknown".into(),
                "x".into(),
            ],
        ] {
            assert!(export_client_metadata(&bad).is_err());
        }
        let mut unknown = args.clone();
        unknown[3] = "unknown".into();
        assert!(export_client_metadata(&unknown).is_err());
        cimd.client_secret_ref = Some("mcp-oauth-client:must-not-be-used".into());
        assert!(cimd.validate().is_err());
        cimd.client_secret_ref = None;
        cimd.client_id = "http://127.0.0.1/client.json".into();
        assert!(cimd.validate().is_err());
        std::fs::write(
            &path,
            toml::to_string(&std::collections::BTreeMap::from([(
                "mcp_oauth",
                vec![provider()],
            )]))
            .unwrap(),
        )
        .unwrap();
        assert!(export_client_metadata(&args).is_err());
    }
}
