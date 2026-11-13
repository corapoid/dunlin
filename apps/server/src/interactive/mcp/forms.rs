//! Translate the already compiled flat MCP form profile into standard AHP.
use super::*;
use ahp_types::state::ChatInputRequest;
use dunlin_mcp_client::{Error, negotiated::FormElicitation};

pub(super) fn request(
    provider: &Provider,
    tool: &str,
    form: &FormElicitation,
) -> std::result::Result<ChatInputRequest, Error> {
    let schema = form.schema.schema();
    let properties = schema["properties"].as_object().ok_or(Error::Schema)?;
    let mut questions = Vec::new();
    for (id, field) in properties {
        let kind = match field["type"].as_str() {
            Some("string") if field.get("enum").is_some() || field.get("oneOf").is_some() => {
                "single-select"
            }
            Some("string") => "text",
            Some("array") => "multi-select",
            Some("number") => "number",
            Some("integer") => "integer",
            Some("boolean") => "boolean",
            _ => return Err(Error::Schema),
        };
        let mut question = json!({"kind":kind,"id":id,"message":field["description"].as_str().unwrap_or(id),"required":schema["required"].as_array().is_some_and(|names|names.contains(&json!(id)))});
        for (from, to) in [
            ("title", "title"),
            ("format", "format"),
            ("default", "defaultValue"),
        ] {
            if let Some(value) = field.get(from) {
                question[to] = value.clone();
            }
        }
        let (min, max) = match kind {
            "text" => ("minLength", "maxLength"),
            "multi-select" => ("minItems", "maxItems"),
            _ => ("minimum", "maximum"),
        };
        for (from, to) in [(min, "min"), (max, "max")] {
            if let Some(v) = field.get(from) {
                question[to] = v.clone();
            }
        }
        if matches!(kind, "single-select" | "multi-select") {
            let default = field.get("default");
            let field = if kind == "multi-select" {
                &field["items"]
            } else {
                field
            };
            let options = if let Some(values) = field["enum"].as_array() {
                values
                    .iter()
                    .map(|v| json!({"id":v,"label":v}))
                    .collect::<Vec<_>>()
            } else {
                field[if kind=="multi-select" {"anyOf"}else{"oneOf"}].as_array().ok_or(Error::Schema)?.iter().map(|v|json!({"id":v["const"],"label":v["title"].as_str().or(v["const"].as_str())})).collect()
            };
            let options = options
                .into_iter()
                .map(|mut option| {
                    if default.is_some_and(|v| {
                        v == &option["id"]
                            || v.as_array().is_some_and(|vs| vs.contains(&option["id"]))
                    }) {
                        option["recommended"] = json!(true);
                    }
                    option
                })
                .collect::<Vec<_>>();
            question["options"] = json!(options);
            question["allowFreeformInput"] = json!(false);
            question.as_object_mut().unwrap().remove("defaultValue"); // No select defaults in AHP 1.0.
        }
        questions.push(question);
    }
    let origin = url::Url::parse(&provider.endpoint)
        .map_err(|_| Error::Endpoint)?
        .origin()
        .ascii_serialization();
    serde_json::from_value(json!({"id":"host-assigned","message":format!("MCP {} ({origin}) / {tool}\n{}",provider.id,form.message),"questions":questions})).map_err(|_|Error::Schema)
}

pub(crate) fn validate(schema: &Value, content: &Value) -> dunlin_session_runtime::Result<()> {
    dunlin_mcp_client::schema::FormSchema::compile(schema)
        .and_then(|schema| schema.validate(content))
        .map_err(|_| {
            dunlin_session_runtime::Error::Conflict(
                "MCP form response does not match its pinned schema",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flat_form_mapping_preserves_types_constraints_identity_and_never_submits_defaults() {
        let schema = json!({"type":"object","properties":{
            "name":{"type":"string","title":"Name","minLength":1,"maxLength":8,"default":"Alice"},
            "count":{"type":"integer","minimum":1,"maximum":9},"ratio":{"type":"number"},"active":{"type":"boolean","default":true},
            "choice":{"type":"string","oneOf":[{"const":"a","title":"First"},{"const":"b"}]},
            "many":{"type":"array","items":{"anyOf":[{"const":"a","title":"First"},{"const":"b"}]},"minItems":1,"maxItems":2}},"required":["name","count"]});
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
        let form = FormElicitation {
            message: "Review values".into(),
            schema: dunlin_mcp_client::schema::FormSchema::compile(&schema).unwrap(),
        };
        let req = request(&provider, "lookup", &form).unwrap();
        assert!(req.answers.is_none());
        assert!(req.url.is_none());
        let value = serde_json::to_value(req).unwrap();
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("https://tools.example.test")
        );
        let q = |id: &str| {
            value["questions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == id)
                .unwrap()
        };
        assert_eq!(q("name")["defaultValue"], "Alice");
        assert_eq!(q("name")["min"], 1);
        assert_eq!(q("count")["kind"], "integer");
        assert_eq!(q("ratio")["kind"], "number");
        assert_eq!(q("active")["kind"], "boolean");
        assert_eq!(q("choice")["options"][0]["label"], "First");
        assert_eq!(q("many")["kind"], "multi-select");
        assert!(validate(&schema, &json!({"name":"Alice","count":2,"many":["a"]})).is_ok());
        for bad in [
            json!({}),
            json!({"name":"too long value","count":2}),
            json!({"name":"A","count":2.5}),
            json!({"name":"A","count":1,"extra":true}),
            json!({"name":"A","count":1,"many":["a","a"]}),
        ] {
            assert!(validate(&schema, &bad).is_err());
        }
    }
}
