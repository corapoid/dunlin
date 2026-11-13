//! Public CIMD document generation. No remote fetch, registration endpoint,
//! credentials, or browser-controlled redirect expansion.
use super::{Error, secure_url};
use serde::Serialize;
use std::collections::HashSet;

/// Immutable, bounded public-client document for operator-managed HTTPS hosting.
/// Serve its JSON directly at its exact client_id, without authentication. The
/// document is public configuration, not proof of permission to call any tool.
#[derive(Serialize)]
pub struct ClientMetadataDocument {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: [&'static str; 2],
    response_types: [&'static str; 1],
    token_endpoint_auth_method: &'static str,
}
impl ClientMetadataDocument {
    pub fn public(
        client_id: &str,
        client_name: &str,
        redirect_uris: &[String],
    ) -> Result<Self, Error> {
        identifier(client_id)?;
        if client_name.trim().is_empty()
            || client_name.len() > 120
            || client_name.chars().any(char::is_control)
            || redirect_uris.is_empty()
            || redirect_uris.len() > 16
        {
            return Err(Error::Credential);
        }
        let mut seen = HashSet::new();
        for redirect in redirect_uris {
            // Exact canonical callback URIs only; no wildcard or URL rewriting.
            let url = secure_url(redirect, false)?;
            if url.as_str() != redirect || redirect.contains('*') || !seen.insert(redirect) {
                return Err(Error::Endpoint);
            }
        }
        Ok(Self {
            client_id: client_id.into(),
            client_name: client_name.into(),
            redirect_uris: redirect_uris.to_vec(),
            grant_types: ["authorization_code", "refresh_token"],
            response_types: ["code"],
            token_endpoint_auth_method: "none",
        })
    }
}

pub(super) fn identifier(value: &str) -> Result<(), Error> {
    let url = secure_url(value, false)?;
    // Requiring the original canonical spelling also rejects raw/encoded dot
    // segments, backslashes, whitespace and implicit URL normalization. Keep
    // the identifier unchanged on the wire and in durable credential bindings.
    if value.len() > 256 || url.scheme() != "https" || url.path() == "/" || url.as_str() != value {
        return Err(Error::Endpoint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = "https://client.example/oauth/client.json";
    const CALLBACK: &str = "https://build.tail123.ts.net/v1/web/mcp/oauth/work/callback";
    #[test]
    fn cimds_have_exact_identifiers_and_bounded_explicit_public_metadata() {
        let document = ClientMetadataDocument::public(ID, "Dunlin", &[CALLBACK.into()]).unwrap();
        assert_eq!(
            serde_json::to_value(document).unwrap(),
            json!({
                "client_id":ID,"client_name":"Dunlin","redirect_uris":[CALLBACK],
                "grant_types":["authorization_code","refresh_token"],
                "response_types":["code"],"token_endpoint_auth_method":"none"
            })
        );
        for bad in [
            "http://127.0.0.1/client.json",
            "https://client.example/",
            "https://client.example",
            "https://user@client.example/client.json",
            "https://client.example/client.json#fragment",
            "https://client.example/client.json?query=1",
            "https://client.example/./client.json",
            "https://client.example/a/../client.json",
            "https://client.example/%2e/client.json",
            "https://client.example/a/%2E%2e/client.json",
            "https://client.example\\client.json",
            " https://client.example/client.json",
            "https://client.example/client.json\n",
        ] {
            assert!(identifier(bad).is_err(), "{bad:?}");
        }
        assert!(identifier(&format!("https://client.example/{}", "a".repeat(256))).is_err());
        for name in ["", " ", "bad\nname", &"x".repeat(121)] {
            assert!(ClientMetadataDocument::public(ID, name, &[CALLBACK.into()]).is_err());
        }
        for redirects in [
            vec![],
            vec![CALLBACK.into(), CALLBACK.into()],
            vec![CALLBACK.into(); 17],
            vec!["https://host.example/*".into()],
            vec!["http://host.example/callback".into()],
            vec![format!("{CALLBACK}?injected=1")],
            vec!["https://host.example/a/../callback".into()],
        ] {
            assert!(ClientMetadataDocument::public(ID, "Dunlin", &redirects).is_err());
        }
        ClientMetadataDocument::public(ID, "Dunlin", &["http://127.0.0.1:3333/callback".into()])
            .unwrap();
    }
}
