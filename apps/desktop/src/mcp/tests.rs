use super::*;

#[test]
fn session_exclusions_round_trip_without_changing_other_permissions_or_metadata() {
    let original = json!({"permissions":[{"action":"shell","resource":"*","effect":"ask"}],"metadata":{"user-setting":{"keep":true}}});
    let disabled = patch(&original, "docs", true).unwrap();
    assert_eq!(disabled["permissions"].as_array().unwrap().len(), 4);
    assert_eq!(disabled["permissions"][0], original["permissions"][0]);
    assert_eq!(
        disabled["metadata"]["user-setting"],
        original["metadata"]["user-setting"]
    );
    assert_eq!(patch(&disabled, "docs", false).unwrap(), original);
    assert_eq!(original["permissions"].as_array().unwrap().len(), 1);

    let both = patch(&disabled, "search", true).unwrap();
    let search_only = patch(&both, "docs", false).unwrap();
    assert!(
        search_only["permissions"]
            .as_array()
            .unwrap()
            .contains(&rules("search")[0])
    );
    assert_eq!(patch(&search_only, "search", false).unwrap(), original);

    // External edits may remove a rule while leaving our ownership metadata.
    // Restoring that exclusion must still preserve unrelated user policy.
    let mut stale = disabled;
    stale["permissions"].as_array_mut().unwrap().remove(1);
    let repaired = patch(&stale, "docs", true).unwrap();
    assert_eq!(patch(&repaired, "docs", false).unwrap(), original);
}

#[test]
fn reenabling_never_removes_existing_user_policy_or_unknown_metadata() {
    let user = json!({"permissions":rules("docs"),"metadata":{"user":123}});
    assert!(patch(&user, "docs", false).is_err());
    assert!(patch(&user, "docs", true).is_err());
    assert!(patch(&json!({"metadata":{(OWNED):"unknown"}}), "docs", true).is_err());
    assert!(session_data(&json!({"data":{"id":"other"}}), "expected").is_err());
    assert!(check_location(&json!({"location":{"directory":"/other"}}), "/project").is_err());
}

#[test]
fn ambiguous_globs_and_disconnected_servers_cannot_affect_other_servers() {
    assert_eq!(normalized("docs😀"), "docs__");
    assert!(!safe_name("foo", &["foo".into(), "foo_bar".into()]));
    assert!(!safe_name("foo bar", &["foo bar".into(), "foo_bar".into()]));
    assert!(!safe_name("*", &["*".into()]));
    assert!(!safe_name("opencode", &["opencode".into()]));
    assert!(!safe_name("opencode_read", &["opencode_read".into()]));
    assert!(safe_name("my docs", &["my docs".into(), "search".into()]));
    let session = patch(&json!({}), "docs", true).unwrap();
    let servers = parse_servers(
        &json!({"data":[
            {"name":"docs","status":{"status":"connected"}},
            {"name":"search","status":{"status":"connected"}},
            {"name":"offline","status":{"status":"disabled"}}
        ]}),
        &session,
    )
    .unwrap();
    let catalog = Catalog {
        servers,
        ..Default::default()
    };
    assert_eq!(catalog.active_count(), 1);
    assert!(
        !catalog
            .servers
            .iter()
            .find(|s| s.name == "offline")
            .unwrap()
            .toggleable
    );
    assert!(
        catalog
            .servers
            .iter()
            .find(|s| s.name == "docs")
            .unwrap()
            .disabled
    );
}
