//! Display-only URL challenge; the controller never fetches or opens it.
use super::*;
use ahp_types::state::ChatInputRequest;
use dunlin_mcp_client::{Error, negotiated::UrlElicitation};

pub(super) fn request(
    provider: &Provider,
    tool: &str,
    input: &UrlElicitation,
) -> std::result::Result<ChatInputRequest, Error> {
    let target = url::Url::parse(&input.url).map_err(|_| Error::Protocol)?;
    if input.url.len() > 4096
        || input.message.len() > 8192
        || input
            .url
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
        || target.scheme() != "https"
        || target.host().is_none()
        || !target.username().is_empty()
        || target.password().is_some()
    {
        return Err(Error::Protocol);
    }
    let origin = url::Url::parse(&provider.endpoint)
        .map_err(|_| Error::Endpoint)?
        .origin()
        .ascii_serialization();
    serde_json::from_value(json!({"id":"host-assigned",
        "message":format!("MCP {} ({origin}) / {tool}\n{}",provider.id,input.message),
        "url":input.url}))
    .map_err(|_| Error::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_mapping_pins_requesting_server_and_never_turns_the_address_into_form_data() {
        let provider = Provider {
            id: "work".into(),
            name: "Work".into(),
            endpoint: "https://tools.example.test/mcp".into(),
            issuer: "https://auth.example.test".into(),
            client_id: "client".into(),
            redirect_uri: "http://127.0.0.1/callback".into(),
            allowed_origins: vec![],
            scopes: vec![],
            authentication: Default::default(),
            client_secret_ref: None,
        };
        let address = "https://review.example.test/connect?step=1#details";
        let input = UrlElicitation {
            message: "Review third-party interaction".into(),
            url: address.into(),
        };
        let mapped = request(&provider, "lookup", &input).unwrap();
        assert_eq!(mapped.url.as_deref(), Some(address));
        assert!(mapped.questions.is_none() && mapped.answers.is_none());
        assert!(
            mapped
                .message
                .unwrap()
                .contains("MCP work (https://tools.example.test) / lookup")
        );
        for url in [
            "http://review.example.test",
            "javascript:alert(1)",
            "https://user:pass@review.example.test",
            "https://review.example.test/a b",
            "https://review.example.test/\\other",
        ] {
            assert!(
                request(
                    &provider,
                    "lookup",
                    &UrlElicitation {
                        message: "Review".into(),
                        url: url.into()
                    }
                )
                .is_err()
            );
        }
    }
}
