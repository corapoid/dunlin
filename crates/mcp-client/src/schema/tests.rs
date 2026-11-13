use super::*;
use serde_json::json;

#[test]
fn full_schema_checks_refs_composition_and_unevaluated_properties_without_coercion() {
    let schema=Schema::compile(&json!({"$defs":{"id":{"type":"integer","minimum":1}},"type":"object",
        "allOf":[{"properties":{"id":{"$ref":"#/$defs/id"}},"required":["id"]},
        {"if":{"properties":{"id":{"minimum":10}}},"then":{"properties":{"tag":{"const":"large"}},"required":["tag"]}}],
        "unevaluatedProperties":false})).unwrap();
    for value in [json!({"id":1}), json!({"id":10,"tag":"large"})] {
        schema.validate(&value).unwrap();
    }
    for value in [
        json!({"id":"1"}),
        json!({"id":0}),
        json!({"id":10}),
        json!({"id":1,"unknown":"private"}),
    ] {
        assert_eq!(schema.validate(&value), Err(Error::Arguments));
    }
}
#[test]
fn schemas_support_declared_drafts_tuples_and_recursive_local_references() {
    let tuple=Schema::compile(&json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"integer"},{"type":"boolean"}],"additionalItems":false})).unwrap();
    tuple.validate(&json!([1, true])).unwrap();
    assert!(tuple.validate(&json!([1, "true"])).is_err());
    assert!(tuple.validate(&json!([1, true, 0])).is_err());
    let node=Schema::compile(&json!({"$defs":{"node":{"anyOf":[{"type":"null"},{"type":"object","properties":{"next":{"$ref":"#/$defs/node"}},"required":["next"],"additionalProperties":false}]}},"$ref":"#/$defs/node"})).unwrap();
    node.validate(&json!({"next":{"next":null}})).unwrap();
    assert!(node.validate(&json!({"next":42})).is_err());
}
#[test]
fn schemas_never_resolve_external_resources_or_custom_meta_schemas() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    assert!(
        Schema::compile(
            &json!({"$ref":format!("http://{}/schema",listener.local_addr().unwrap())})
        )
        .is_err()
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    for reference in [
        "https://127.0.0.1:9/private",
        "file:///etc/passwd",
        "urn:private:schema",
    ] {
        assert!(matches!(
            Schema::compile(&json!({"$ref":reference})),
            Err(Error::Schema)
        ));
    }
    assert!(matches!(
        Schema::compile(&json!({"$schema":"https://example.test/custom"})),
        Err(Error::Schema)
    ));
    assert!(matches!(
        Schema::compile(&json!({"type":"not-a-type"})),
        Err(Error::Schema)
    ));
}
#[test]
fn bounds_and_linear_regex_fail_closed_including_under_negation() {
    Schema::compile(&json!(true))
        .unwrap()
        .validate(&json!({"any":"value"}))
        .unwrap();
    assert_eq!(
        Schema::compile(&json!(false)).unwrap().validate(&json!({})),
        Err(Error::Arguments)
    );
    assert!(matches!(
        Schema::compile(&json!({"description":"x".repeat(65537)})),
        Err(Error::Limit)
    ));
    assert!(matches!(
        Schema::compile(&json!({"not":{"pattern":"^(a+)\\1$"}})),
        Err(Error::Schema)
    ));
    let schema =
        Schema::compile(&json!({"type":"string","pattern":"^[a-z]+$","minLength":2,"maxLength":4}))
            .unwrap();
    schema.validate(&json!("abcd")).unwrap();
    assert!(schema.validate(&json!("abcde")).is_err());
    let mut deep = json!(null);
    for _ in 0..66 {
        deep = json!([deep]);
    }
    assert_eq!(schema.validate(&deep), Err(Error::Limit));
    let error = schema
        .validate(&json!("private-instance-value"))
        .unwrap_err();
    assert!(!error.to_string().contains("private-instance"));
}

#[test]
fn reference_expansion_is_bounded_before_the_validator_can_evaluate_it() {
    let shared=Schema::compile(&json!({"$defs":{"text":{"type":"string"}},"allOf":[{"$ref":"#/$defs/text"},{"$ref":"#/$defs/text"}]})).unwrap();
    shared.validate(&json!("allowed")).unwrap();
    let mut definitions = serde_json::Map::new();
    definitions.insert("last".into(), json!({"type":"string"}));
    let mut next = "last".to_owned();
    for index in 0..16 {
        let name = format!("node{index}");
        let reference = format!("#/$defs/{next}");
        definitions.insert(
            name.clone(),
            json!({"allOf":[{"$ref":reference},{"$ref":reference}]}),
        );
        next = name;
    }
    assert!(matches!(
        Schema::compile(&json!({"$defs":definitions,"$ref":format!("#/$defs/{next}")})),
        Err(Error::Limit)
    ));
    assert!(matches!(
        Schema::compile(&json!({"$ref":"#"})),
        Err(Error::Schema)
    ));
    let anchored = Schema::compile(
        &json!({"$defs":{"named":{"$anchor":"name","type":"string","minLength":2}},"$ref":"#name"}),
    )
    .unwrap();
    anchored.validate(&json!("valid")).unwrap();
    assert!(anchored.validate(&json!("x")).is_err());
}

#[test]
fn enum_and_default_data_are_not_interpreted_as_schema_directives() {
    let values:Vec<Value>=(0..20).map(|i|json!({"pattern":format!("literal-{i}"),"$ref":"https://example.test/data-not-schema"})).collect();
    let schema = Schema::compile(
        &json!({"enum":values,"default":{"$id":"literal-data","pattern":"literal-data"}}),
    )
    .unwrap();
    schema.validate(&values[0]).unwrap();
    assert!(schema.validate(&json!({"pattern":"other"})).is_err());
}
