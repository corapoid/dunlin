use super::*;
use serde_json::json;
fn template() -> RecipeTemplate {
    serde_json::from_value(json!({"name":"deploy","program_sha256":"a".repeat(64),"args":["--environment","{environment}"],"env_var":"API_TOKEN","parameters":[{"name":"environment","required":true,"max_len":32,"allow_leading_dash":false}],"timeout_secs":30})).unwrap()
}
#[test]
fn digest_pins_all_execution_semantics_but_not_controller_local_metadata_or_paths() {
    let original = template();
    let digest = original.digest().unwrap();
    // Stable vector for the existing operator/template v1 pin format.
    assert_eq!(
        digest,
        "9471abbc40b542eb748ce4d308330d97360010c59a758c42c6f5619626899c88"
    );
    let mut recipe = original.recipe("/controller/bin/deploy");
    assert_eq!(
        RecipeTemplate::from_recipe(&recipe)
            .unwrap()
            .digest()
            .unwrap(),
        digest
    );
    recipe.program = "/runner/bin/deploy".into();
    recipe.id = "different-local-id".into();
    recipe.secret_id = "different-credential".into();
    recipe.description = "display only".into();
    recipe.created_at = 99;
    assert_eq!(
        RecipeTemplate::from_recipe(&recipe)
            .unwrap()
            .digest()
            .unwrap(),
        digest
    );
    let roundtrip: RecipeTemplate =
        serde_json::from_str(&serde_json::to_string_pretty(&original).unwrap()).unwrap();
    assert_eq!(roundtrip.digest().unwrap(), digest);
    for (key, value) in [
        ("name", json!("release")),
        ("program_sha256", json!("b".repeat(64))),
        ("args", json!(["release", "{environment}"])),
        ("env_var", json!("OTHER_TOKEN")),
        (
            "parameters",
            json!([{"name":"environment","required":false,"max_len":31,"allow_leading_dash":true}]),
        ),
        ("timeout_secs", json!(29)),
    ] {
        let mut changed = serde_json::to_value(&original).unwrap();
        changed[key] = value;
        assert_ne!(
            serde_json::from_value::<RecipeTemplate>(changed)
                .unwrap()
                .digest()
                .unwrap(),
            digest,
            "{key}"
        );
    }
}
#[test]
fn templates_reject_ambient_config_missing_pins_unknown_fields_and_ambiguous_parameters() {
    for name in [
        "LD_PRELOAD",
        "DyLd_InSeRt_LiBrArIeS",
        "HOME",
        "PATH",
        "NODE_OPTIONS",
        "AWS_CONFIG_FILE",
        "XDG_CONFIG_HOME",
        "GIT_CONFIG_COUNT",
        "",
    ] {
        let mut t = template();
        t.env_var = name.into();
        assert!(t.validate().is_err(), "{name}");
    }
    for hash in [
        String::new(),
        "A".repeat(64),
        "a".repeat(63),
        "g".repeat(64),
    ] {
        let mut t = template();
        t.program_sha256 = hash;
        assert!(t.validate().is_err());
    }
    for path in [
        "relative",
        "/bin/sh",
        "/usr/bin/env",
        "/usr/bin/python3",
        "/opt/../bin/deploy",
        "/opt//deploy",
        "/opt/deploy\0",
    ] {
        assert!(template().validate_program(path).is_err(), "{path}");
    }
    for field in ["program", "secret", "cwd", "host"] {
        let mut value = serde_json::to_value(template()).unwrap();
        value[field] = json!("untrusted");
        assert!(serde_json::from_value::<RecipeTemplate>(value).is_err());
    }
    let mut value = serde_json::to_value(template()).unwrap();
    value["parameters"][0]["program"] = json!("untrusted");
    assert!(serde_json::from_value::<RecipeTemplate>(value).is_err());
    let mut t = template();
    t.parameters.push(t.parameters[0].clone());
    assert!(t.validate().is_err());
    let mut t = template();
    t.args = vec!["{unknown}".into()];
    assert!(t.validate().is_err());
    let mut t = template();
    t.args = vec!["literal\0".into()];
    assert!(t.validate().is_err());
    let mut t = template();
    t.args = vec!["x".repeat(4096); 5];
    assert!(t.validate().is_err());
}
#[test]
fn template_renders_only_declared_parameters_to_individual_argv_elements() {
    let t = template();
    let params = BTreeMap::from([("environment".into(), "test; echo no-shell".into())]);
    assert_eq!(
        t.render_args(&params).unwrap(),
        vec!["--environment", "test; echo no-shell"]
    );
    assert!(t.render_args(&BTreeMap::new()).is_err());
    for value in ["--escape", "line\nbreak", "null\0value"] {
        assert!(
            t.render_args(&BTreeMap::from([("environment".into(), value.into())]))
                .is_err()
        );
    }
    let mut extra = params;
    extra.insert("program".into(), "untrusted".into());
    assert!(t.render_args(&extra).is_err());
}
