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

#[test]
#[ignore = "creates and deletes one idle local OpenCode test session; never sends a prompt or connects/disconnects MCP"]
fn live_session_exclusion_round_trip_without_prompt_or_restart() {
    let directory = std::env::current_dir().unwrap();
    let directory = directory.to_str().unwrap();
    let created=local_request(directory,"POST","session",Some(json!({"title":"Dunlin MCP isolation check (no prompts)","metadata":{"dunlin-test":true},"permissions":[{"action":"shell","resource":"*","effect":"ask"}]}))).unwrap();
    let id = created["data"]["id"]
        .as_str()
        .expect("created test session id");
    let resource = format!("session/{}", encode(id));
    let result = (|| -> Result<(), String> {
        let before = local_request(directory, "GET", &resource, None)?;
        let original = session_data(&before, id)?;
        local_request(
            directory,
            "PATCH",
            &resource,
            Some(patch(original, "dunlin-isolation-fixture", true)?),
        )?;
        let disabled = local_request(directory, "GET", &resource, None)?;
        let disabled = session_data(&disabled, id)?;
        if !disabled["permissions"]
            .as_array()
            .is_some_and(|p| p.contains(&rules("dunlin-isolation-fixture")[0]))
        {
            return Err("The session did not retain its exclusion".into());
        }
        local_request(
            directory,
            "PATCH",
            &resource,
            Some(patch(disabled, "dunlin-isolation-fixture", false)?),
        )?;
        let restored = local_request(directory, "GET", &resource, None)?;
        let restored = session_data(&restored, id)?;
        if restored["permissions"] != original["permissions"]
            || restored["metadata"] != original["metadata"]
        {
            return Err("Unrelated session settings changed".into());
        }
        Ok(())
    })();
    let cleanup = local_request(directory, "DELETE", &resource, None);
    result.unwrap();
    cleanup.unwrap();
}
