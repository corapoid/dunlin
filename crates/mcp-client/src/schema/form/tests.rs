use super::*;
use serde_json::json;

fn schema() -> Value {
    json!({"type":"object","properties":{
        "name":{"type":"string","minLength":2,"maxLength":8},
        "age":{"type":"integer","minimum":18,"maximum":120},
        "email":{"type":"string","format":"email"},
        "date":{"type":"string","format":"date"},
        "enabled":{"type":"boolean","default":false},
        "color":{"type":"string","oneOf":[{"const":"r","title":"Red"},{"const":"b","title":"Blue"}],"default":"r"},
        "tags":{"type":"array","minItems":1,"maxItems":2,"items":{"anyOf":[{"const":"a","title":"A"},{"const":"b","title":"B"}]}}
    },"required":["name","age","tags"]})
}
#[test]
fn flat_forms_validate_primitive_constraints_and_enum_choices_without_defaults_or_coercion() {
    let form = FormSchema::compile(&schema()).unwrap();
    let valid = json!({"name":"世界","age":18,"tags":["a"],"email":"a@example.test","date":"2024-02-29","color":"b"});
    form.validate(&valid).unwrap();
    assert_eq!(form.schema()["properties"]["enabled"]["default"], false);
    for (key, value) in [
        ("name", json!("x")),
        ("age", json!("18")),
        ("age", json!(18.5)),
        ("age", json!(17)),
        ("enabled", json!("true")),
        ("email", json!("not email")),
        ("date", json!("2025-02-29")),
        ("color", json!("green")),
        ("tags", json!(["a", "a"])),
        ("tags", json!(["unknown"])),
    ] {
        let mut changed = valid.clone();
        changed[key] = value;
        assert!(form.validate(&changed).is_err(), "{key}");
    }
    let mut unknown = valid.clone();
    unknown["unrequested-secret"] = json!("private-value");
    assert_eq!(form.validate(&unknown), Err(Error::Arguments));
    assert!(form.validate(&json!({"name":"ok","tags":["a"]})).is_err());
}
#[test]
fn forms_reject_nested_types_unknown_constraints_invalid_defaults_and_ambiguous_choices() {
    for field in [
        json!({"type":"object","properties":{}}),
        json!({"type":"array","items":{"type":"object"}}),
        json!({"type":"string","pattern":".*"}),
        json!({"type":"string","format":"password"}),
        json!({"type":"integer","default":"18"}),
        json!({"type":"string","enum":["x","x"]}),
        json!({"type":"string","oneOf":[{"const":"x","title":"One"},{"const":"x","title":"Two"}]}),
    ] {
        assert!(
            FormSchema::compile(&json!({"type":"object","properties":{"value":field}})).is_err()
        );
    }
    assert!(
        FormSchema::compile(&json!({"type":"object","properties":{},"required":["missing"]}))
            .is_err()
    );
    assert!(
        FormSchema::compile(&json!({"type":"object","properties":{},"additionalProperties":true}))
            .is_err()
    );
    assert!(
        FormSchema::compile(
            &json!({"type":"object","properties":{},"$ref":"https://example.test/schema"})
        )
        .is_err()
    );
}
#[test]
fn optional_defaults_are_display_metadata_not_implicit_user_responses() {
    let form=FormSchema::compile(&json!({"type":"object","properties":{"choice":{"type":"string","enum":["a","b"],"default":"a"}},"required":["choice"]})).unwrap();
    assert_eq!(form.validate(&json!({})), Err(Error::Arguments));
    form.validate(&json!({"choice":"b"})).unwrap();
    let empty = FormSchema::compile(&json!({"type":"object","properties":{}})).unwrap();
    empty.validate(&json!({})).unwrap();
    assert!(empty.validate(&json!({"extra":false})).is_err());
}
#[test]
fn forms_enforce_limits_and_safe_integer_boundaries() {
    let fields: Map<String, Value> = (0..33)
        .map(|i| (format!("field-{i}"), json!({"type":"boolean"})))
        .collect();
    assert!(matches!(
        FormSchema::compile(&json!({"type":"object","properties":fields})),
        Err(Error::Limit)
    ));
    let form = FormSchema::compile(
        &json!({"type":"object","properties":{"n":{"type":"integer"},"text":{"type":"string"}}}),
    )
    .unwrap();
    assert!(
        form.validate(&json!({"n":9_007_199_254_740_992_u64}))
            .is_err()
    );
    assert!(matches!(
        form.validate(&json!({"text":"x".repeat(8193)})),
        Err(Error::Limit)
    ));
}
