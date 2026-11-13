use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use serde_json::json;
fn definition(name: &str) -> AgentDefinition {
    serde_json::from_value(json!({"schema_version":1,"name":name,"description":"Fixture profile","harness":"claude","model":"fixture-model","mode":"plan","instructions":"private-instruction-marker","skills":[{"id":"review","version":"v1","instructions":"Review carefully"}],"max_seconds":60,"max_output_bytes":1048576})).unwrap()
}
fn scope(store: &Store) -> (String, String, String) {
    let owner = store.create_user("agent-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Agents").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Profiles")
        .unwrap();
    (owner.id, workspace.id, project.id)
}
#[test]
fn mcp_publication_cannot_extend_or_mutate_a_previously_published_profile() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db")).unwrap();
    let (user, w, p) = scope(&store);
    let original = definition("Original");
    let draft = store.create_agent(&user, &w, &p, original.clone()).unwrap();
    let first = store
        .publish_agent(&user, &w, &p, &draft.id, draft.revision)
        .unwrap();
    let mut changed = original.clone();
    changed.schema_version = 2;
    changed.mcp.push(dunlin_domain::agent::AgentMcpTool {
        provider_id: "work".into(),
        name: "lookup".into(),
        definition_digest: "a".repeat(64),
    });
    let second = store
        .save_agent(&user, &w, &p, &draft.id, draft.revision, changed)
        .unwrap();
    let second = store
        .publish_agent(&user, &w, &p, &draft.id, second.revision)
        .unwrap();
    assert_ne!(first.digest, second.digest);
    let restored = store
        .agent_version(&user, &w, &p, &first.agent_id, first.version)
        .unwrap()
        .unwrap();
    assert_eq!(restored.definition, original);
    assert_eq!(restored.digest, first.digest);
    assert!(restored.definition.mcp.is_empty());
}
#[test]
fn published_agent_versions_are_immutable_cas_audited_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let (owner, w, p) = scope(&store);
    let draft = store
        .create_agent(&owner, &w, &p, definition("First"))
        .unwrap();
    assert!(store.agent_profiles(&owner, &w, &p).unwrap().is_empty());
    let first = store.publish_agent(&owner, &w, &p, &draft.id, 1).unwrap();
    assert_eq!(
        first.digest,
        hex::encode(Sha256::digest(
            serde_json::to_string(&first.definition).unwrap().as_bytes()
        ))
    );
    let duplicate = store.publish_agent(&owner, &w, &p, &draft.id, 1).unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(duplicate).unwrap()
    );
    assert_eq!(
        store
            .team_audit(&owner, &w)
            .unwrap()
            .iter()
            .filter(|e| e.action == "agent_published")
            .count(),
        1
    );
    let second = store
        .save_agent(&owner, &w, &p, &draft.id, 1, definition("Second"))
        .unwrap();
    assert_eq!(second.revision, 2);
    assert!(matches!(
        store.save_agent(&owner, &w, &p, &draft.id, 1, definition("Stale")),
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.publish_agent(&owner, &w, &p, &draft.id, 1),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store.agent_profiles(&owner, &w, &p).unwrap()[0]
            .definition
            .name,
        "First"
    );
    let published = store.publish_agent(&owner, &w, &p, &draft.id, 2).unwrap();
    assert_eq!(published.version, 2);
    assert_ne!(first.digest, published.digest);
    let audit = serde_json::to_string(&store.team_audit(&owner, &w).unwrap()).unwrap();
    assert!(!audit.contains("private-instruction-marker"));
    let events = store.team_audit(&owner, &w).unwrap();
    for event in events
        .iter()
        .filter(|event| event.action.starts_with("agent_"))
    {
        assert_eq!(event.resource_id.as_deref(), Some(draft.id.as_str()));
        assert_eq!(
            event.target_id, owner,
            "User FK remains the actor; resource_id identifies the agent"
        );
    }
    for action in ["agent_created", "agent_draft_saved", "agent_published"] {
        assert!(events.iter().any(|event| event.action == action));
    }
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(
        store
            .agent_version(&owner, &w, &p, &draft.id, 1)
            .unwrap()
            .unwrap()
            .definition
            .name,
        "First"
    );
    assert_eq!(store.agent_drafts(&owner, &w, &p).unwrap()[0].revision, 2);
    assert_eq!(store.agent_profiles(&owner, &w, &p).unwrap()[0].version, 2);
}
#[test]
fn agents_require_current_view_for_reads_and_manage_project_for_every_write() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w, p) = scope(&store);
    let editor = store.create_user("editor", "member").unwrap();
    let stranger = store.create_user("unrelated-admin", "admin").unwrap();
    store
        .set_team_member(&owner, &w, &editor.id, WorkspaceRole::Member, true, None)
        .unwrap();
    store
        .set_team_project_grant(
            &owner,
            &w,
            &p,
            &editor.id,
            [Permission::ViewProject].into(),
            None,
        )
        .unwrap();
    let draft = store
        .create_agent(&owner, &w, &p, definition("First"))
        .unwrap();
    assert!(
        store
            .agent_draft(&editor.id, &w, &p, &draft.id)
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .create_agent(&editor.id, &w, &p, definition("Denied"))
            .is_err()
    );
    assert!(
        store
            .save_agent(&editor.id, &w, &p, &draft.id, 1, definition("Denied"))
            .is_err()
    );
    assert!(
        store
            .publish_agent(&editor.id, &w, &p, &draft.id, 1)
            .is_err()
    );
    assert!(store.agent_drafts(&stranger.id, &w, &p).is_err());
    store
        .set_team_project_grant(
            &owner,
            &w,
            &p,
            &editor.id,
            [Permission::ViewProject, Permission::ManageProject].into(),
            Some(1),
        )
        .unwrap();
    assert!(
        store
            .save_agent(
                &editor.id,
                &w,
                &p,
                &draft.id,
                1,
                definition("Denied by role")
            )
            .is_err(),
        "A project grant cannot bypass the Member role ceiling"
    );
    store
        .set_team_member(&owner, &w, &editor.id, WorkspaceRole::Admin, true, Some(1))
        .unwrap();
    store
        .save_agent(&editor.id, &w, &p, &draft.id, 1, definition("Edited"))
        .unwrap();
    store
        .publish_agent(&editor.id, &w, &p, &draft.id, 2)
        .unwrap();
    store
        .set_team_project_grant(
            &owner,
            &w,
            &p,
            &editor.id,
            [Permission::ViewProject].into(),
            Some(2),
        )
        .unwrap();
    assert!(
        store
            .publish_agent(&editor.id, &w, &p, &draft.id, 2)
            .is_err(),
        "Duplicate publish still requires current write authority"
    );
    store
        .set_team_member(&owner, &w, &editor.id, WorkspaceRole::Admin, false, Some(2))
        .unwrap();
    assert!(store.agent_drafts(&editor.id, &w, &p).is_err());
    assert!(store.agent_profiles(&editor.id, &w, &p).is_err());
    assert!(
        store
            .agent_version(&editor.id, &w, &p, &draft.id, 1)
            .is_err()
    );
}
#[test]
fn agent_definition_and_draft_version_quotas_fail_without_partial_publication() {
    let store = Store::open_in_memory().unwrap();
    let (owner, w, p) = scope(&store);
    let mut invalid = definition("Invalid");
    invalid.harness = "sh arbitrary command".into();
    assert!(matches!(
        store.create_agent(&owner, &w, &p, invalid),
        Err(StoreError::Limit(_))
    ));
    assert!(store.agent_drafts(&owner, &w, &p).unwrap().is_empty());
    let draft = store
        .create_agent(&owner, &w, &p, definition("Version 1"))
        .unwrap();
    for version in 1..=128 {
        if version > 1 {
            store
                .save_agent(
                    &owner,
                    &w,
                    &p,
                    &draft.id,
                    version - 1,
                    definition(&format!("Version {version}")),
                )
                .unwrap();
        }
        assert_eq!(
            store
                .publish_agent(&owner, &w, &p, &draft.id, version)
                .unwrap()
                .version,
            version
        );
    }
    assert_eq!(
        store
            .publish_agent(&owner, &w, &p, &draft.id, 128)
            .unwrap()
            .version,
        128,
        "Duplicate stays valid at capacity"
    );
    store
        .save_agent(
            &owner,
            &w,
            &p,
            &draft.id,
            128,
            definition("Unpublished 129"),
        )
        .unwrap();
    assert!(matches!(
        store.publish_agent(&owner, &w, &p, &draft.id, 129),
        Err(StoreError::Limit(_))
    ));
    assert_eq!(
        store.agent_profiles(&owner, &w, &p).unwrap()[0].version,
        128
    );
    for index in 1..128 {
        store
            .create_agent(&owner, &w, &p, definition(&format!("Agent {index}")))
            .unwrap();
    }
    assert!(matches!(
        store.create_agent(&owner, &w, &p, definition("Over quota")),
        Err(StoreError::Limit(_))
    ));
    assert_eq!(store.agent_drafts(&owner, &w, &p).unwrap().len(), 128);
    assert!(
        store
            .agent_version(&owner, &w, &p, &draft.id, 129)
            .unwrap()
            .is_none()
    );
}
