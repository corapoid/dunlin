use super::*;

#[test]
fn mirrored_names_and_values_cannot_inject_or_impersonate_encoded_literals() {
    for (input, expected) in [
        ("us-west1", "us-west1"),
        ("Hello, 世界", "=?base64?SGVsbG8sIOS4lueVjA==?="),
        (" padded ", "=?base64?IHBhZGRlZCA=?="),
        ("line1\nline2", "=?base64?bGluZTEKbGluZTI=?="),
        ("=?base64?literal?=", "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?="),
        (
            "\r\nAuthorization: stolen",
            "=?base64?DQpBdXRob3JpemF0aW9uOiBzdG9sZW4=?=",
        ),
    ] {
        assert_eq!(header_value(input).unwrap(), expected);
    }
    assert_eq!(
        header_value(&"x".repeat(MAX_HEADER_BYTES + 1)),
        Err(Error::Limit)
    );
}

#[test]
fn nested_exact_paths_preserve_numbers_and_omit_missing_or_null_values() {
    let plan = ToolHeaders::from_schema(&json!({"type":"object","properties":{
        "tenant":{"type":"object","properties":{
            "region":{"type":"string","x-mcp-header":"Region"},
            "account":{"type":"integer","x-mcp-header":"Account"}
        }},
        "enabled":{"type":"boolean","x-mcp-header":"Enabled"},
        "absent":{"type":"string","x-mcp-header":"Absent"},
        "nullable":{"type":"string","x-mcp-header":"Nullable"}
    }}))
    .unwrap();
    let headers = plan
        .project(&json!({"tenant":{"region":"eu","account":42.0}, "enabled":false,"nullable":null}))
        .unwrap();
    assert_eq!(headers.len(), 3);
    for (name, value) in [("Region", "eu"), ("Account", "42"), ("Enabled", "false")] {
        assert!(headers.contains(&(format!("Mcp-Param-{name}"), value.into())));
    }
    for account in [
        json!(9_007_199_254_740_992_u64),
        json!(-9_007_199_254_740_992_i64),
        json!(1.5),
        json!("42"),
    ] {
        assert!(
            plan.project(&json!({"tenant":{"account":account}}))
                .is_err()
        );
    }
    assert!(
        plan.project(&json!({"tenant":{"account":-9_007_199_254_740_991_i64}}))
            .is_ok()
    );
}

#[test]
fn annotations_in_arrays_refs_composition_and_conditional_schemas_are_rejected() {
    let annotated = json!({"type":"string","x-mcp-header":"Tenant"});
    for schema in [
        annotated.clone(),
        json!({"properties":{"x":{"type":"number","x-mcp-header":"Tenant"}}}),
        json!({"properties":{"x":{"type":"array","items":annotated}}}),
        json!({"$defs":{"tenant":annotated},"properties":{"x":{"$ref":"#/$defs/tenant"}}}),
        json!({"allOf":[{"properties":{"tenant":annotated}}]}),
        json!({"if":{"properties":{"tenant":annotated}}}),
        json!({"properties":{"a":{"type":"string","x-mcp-header":"Tenant"},"b":{"type":"string","x-mcp-header":"tenant"}}}),
    ] {
        assert!(ToolHeaders::from_schema(&schema).is_err(), "{schema}");
    }
    // Example/default instances are data, not schema annotations.
    assert!(
        ToolHeaders::from_schema(&json!({"examples":[{"x-mcp-header":"not an annotation"}]}))
            .is_ok()
    );
    for name in ["", "Bad Name", "bad\r\nInjected", "zażółć"] {
        assert!(
            ToolHeaders::from_schema(
                &json!({"properties":{"x":{"type":"string","x-mcp-header":name}}})
            )
            .is_err()
        );
    }
}

#[test]
fn metadata_is_owned_by_transport_and_mirrors_the_same_request() {
    let PreparedRequest { body, headers } = request(
        7,
        "resources/read",
        json!({"uri":"file:///Hello 世界", "_meta":{"trace":"opaque"}}),
    )
    .unwrap();
    assert_eq!(body["params"]["_meta"]["trace"], "opaque");
    assert_eq!(
        body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        VERSION
    );
    assert_eq!(
        body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
        json!({})
    );
    assert!(headers.contains(&(
        "Mcp-Name".into(),
        header_value("file:///Hello 世界").unwrap()
    )));
    assert!(headers.contains(&("Mcp-Method".into(), body["method"].as_str().unwrap().into())));
    assert!(request(1, "tools/call", json!({"arguments":{}})).is_err());
    assert!(
        request(
            1,
            "tools/list",
            json!({"_meta":{"io.modelcontextprotocol/clientCapabilities":{"sampling":{}}}})
        )
        .is_err()
    );
    assert!(request(1, "bad\r\nInjected", json!({})).is_err());
    assert!(request(SAFE_INTEGER as u64 + 1, "server/discover", json!({})).is_err());
}

#[test]
fn projection_budgets_bound_schema_work_and_aggregate_http_headers() {
    let mut properties = serde_json::Map::new();
    for i in 0..=MAX_HEADERS {
        properties.insert(
            format!("p{i}"),
            json!({"type":"string","x-mcp-header":format!("P{i}")}),
        );
    }
    assert!(matches!(
        ToolHeaders::from_schema(&json!({"properties":properties})),
        Err(Error::Limit)
    ));
    let plan = ToolHeaders::from_schema(&json!({"properties":{
        "a":{"type":"string","x-mcp-header":"A"},
        "b":{"type":"string","x-mcp-header":"B"}
    }}))
    .unwrap();
    assert_eq!(
        plan.project(&json!({"a":"x".repeat(9000),"b":"y".repeat(9000)})),
        Err(Error::Limit)
    );
}
