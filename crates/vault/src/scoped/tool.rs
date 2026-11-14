//! Built-in MCP surface. Slots identify policy, never keyring records. The host
//! supplies the credential binding after checking its private grant.
use crate::{
    http::HttpInput,
    scoped::{Error, Result},
};
use dunlin_mcp_client::{Tool, schema::Schema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PROVIDER_PREFIX: &str = "dunlin-secret-";
pub fn is_provider(id: &str) -> bool {
    id.strip_prefix(PROVIDER_PREFIX).is_some_and(|slot| {
        !slot.is_empty()
            && id.len() <= 64
            && slot
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}
pub fn http() -> Tool {
    Tool {
        name: "http".into(),
        output_schema: None,
        input_schema: json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "method":{"type":"string","enum":["GET","HEAD","POST","PUT","PATCH","DELETE"]},
                "url":{"type":"string","maxLength":2048},
                "headers":{"type":"object","maxProperties":8,"additionalProperties":{"type":"string","maxLength":1024}},
                "body":{"type":["string","null"],"maxLength":4096},
                "operation":{"type":"string","pattern":"^[a-z0-9-]{1,64}$"},
                "params":{"type":"object","maxProperties":8,"additionalProperties":{"type":"string","maxLength":512}}
            },
            "oneOf":[
                {"required":["method","url"],"not":{"required":["operation"]}},
                {"required":["operation"],"not":{"anyOf":[{"required":["method"]},{"required":["url"]},{"required":["body"]}]}}
            ]
        }),
    }
}
/// Named CLI template; program, environment and checkout are host-owned.
pub fn recipe() -> Tool {
    Tool {
        name: "recipe".into(),
        output_schema: None,
        input_schema: json!({"type":"object","additionalProperties":false,
        "required":["recipe"],"properties":{
            "recipe":{"type":"string","pattern":"^[a-z0-9_-]{1,64}$"},
            "params":{"type":"object","maxProperties":16,"additionalProperties":{"type":"string","maxLength":4096}}
        }}),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeCall {
    recipe: String,
    #[serde(default)]
    params: BTreeMap<String, String>,
}
pub fn recipe_input(credential: &str, arguments: Value) -> Result<crate::recipe::RecipeInput> {
    Schema::compile(&recipe().input_schema)
        .map_err(|_| Error::Invalid)?
        .validate(&arguments)
        .map_err(|_| Error::Invalid)?;
    let call: RecipeCall = serde_json::from_value(arguments).map_err(|_| Error::Invalid)?;
    Ok(crate::recipe::RecipeInput {
        secret: credential.into(),
        recipe: call.recipe,
        params: call.params,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    #[serde(default)]
    method: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
}
pub fn input(credential: &str, arguments: Value) -> Result<HttpInput> {
    Schema::compile(&http().input_schema)
        .map_err(|_| Error::Invalid)?
        .validate(&arguments)
        .map_err(|_| Error::Invalid)?;
    let call: Call = serde_json::from_value(arguments).map_err(|_| Error::Invalid)?;
    Ok(HttpInput {
        secret: credential.into(),
        method: call.method,
        url: call.url,
        headers: call.headers,
        body: call.body,
        operation: call.operation,
        params: call.params,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definition_is_pinned_and_arguments_cannot_select_a_credential_or_mix_operations() {
        assert!(is_provider("dunlin-secret-work"));
        assert!(!is_provider(PROVIDER_PREFIX));
        assert!(!is_provider("remote-work"));
        assert_eq!(
            http().definition_digest().unwrap(),
            http().definition_digest().unwrap()
        );
        let request = input(
            "host-selected",
            json!({"method":"GET","url":"https://api.example.com/item"}),
        )
        .unwrap();
        assert_eq!(request.secret, "host-selected");
        assert!(
            input(
                "host-selected",
                json!({"secret":"other","method":"GET","url":"https://api.example.com"})
            )
            .is_err()
        );
        assert!(
            input(
                "host-selected",
                json!({"operation":"list","url":"https://evil.example"})
            )
            .is_err()
        );
        assert!(
            input(
                "host-selected",
                json!({"operation":"list","params":{"id":"one"}})
            )
            .is_ok()
        );
    }
    #[test]
    fn recipe_arguments_cannot_choose_host_program_environment_or_credential() {
        let arguments = json!({"recipe":"deploy","params":{"environment":"test"}});
        let parsed = recipe_input("host-selected", arguments.clone()).unwrap();
        assert_eq!(parsed.secret, "host-selected");
        assert_eq!(parsed.recipe, "deploy");
        for field in ["secret", "program", "cwd", "env", "host", "checkout_stamp"] {
            let mut changed = arguments.clone();
            changed[field] = json!("untrusted");
            assert!(recipe_input("host-selected", changed).is_err());
        }
        assert!(recipe_input("host-selected", json!({"recipe":"../other"})).is_err());
        assert_ne!(
            recipe().definition_digest().unwrap(),
            http().definition_digest().unwrap()
        );
    }
}
