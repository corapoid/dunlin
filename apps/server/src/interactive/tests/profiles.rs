use super::*;
use crate::store::agents::AgentVersion;
use dunlin_domain::agent::{AGENT_SCHEMA_VERSION, AgentDefinition, AgentSkill};

pub(super) fn definition(instructions: &str) -> AgentDefinition {
    AgentDefinition {
        schema_version: AGENT_SCHEMA_VERSION,
        name: "Published local fixture".into(),
        description: "No model or network".into(),
        harness: "opencode".into(),
        model: "fixture-model".into(),
        mode: "plan".into(),
        instructions: Some(instructions.into()),
        skills: vec![AgentSkill {
            id: "bounded-review".into(),
            version: "1.2.0".into(),
            instructions: "Report only concrete fixture findings.".into(),
        }],
        mcp: vec![],
        max_seconds: 30,
        max_output_bytes: 1024 * 1024,
    }
}
pub(super) fn publish(f: &Fixture, definition: AgentDefinition) -> AgentVersion {
    let store = f.state.store.lock().unwrap();
    let draft = store
        .create_agent(&f.auth.user.id, &f.workspace, &f.project, definition)
        .unwrap();
    store
        .publish_agent(
            &f.auth.user.id,
            &f.workspace,
            &f.project,
            &draft.id,
            draft.revision,
        )
        .unwrap()
}
pub(super) fn reference(version: &AgentVersion) -> ProfileRef {
    ProfileRef {
        agent_id: version.agent_id.clone(),
        version: version.version,
        digest: version.digest.clone(),
    }
}
#[tokio::test]
async fn mcp_profile_is_publishable_but_never_silently_runs_without_broker() {
    let f = fixture("normal");
    let mut d = definition("MCP policy");
    d.schema_version = 2;
    d.mcp.push(dunlin_domain::agent::AgentMcpTool {
        provider_id: "work".into(),
        name: "lookup".into(),
        definition_digest: "a".repeat(64),
    });
    let profile = publish(&f, d);
    assert_eq!(
        create(&f, &reference(&profile), "mcp-profile-deny-before-process").unwrap_err(),
        "mcp_connections_required"
    );
    assert_eq!(starts(&f), 0);
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
}
fn create(f: &Fixture, profile: &ProfileRef, key: &str) -> Result<CreateAcceptance> {
    Service::create_request_with_key(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        CreateRequest::Profile {
            profile: profile.clone(),
            mcp_connections: vec![],
            target: None,
        },
        Some(key.into()),
    )
}
fn starts(f: &Fixture) -> usize {
    std::fs::read_to_string(f.dir.path().join("checkout/actor-starts"))
        .unwrap_or_default()
        .lines()
        .count()
}
fn received(f: &Fixture) -> String {
    std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap_or_default()
}
pub(super) fn completed(f: &Fixture, session: &Session, count: usize) -> bool {
    let store = f.state.store.lock().unwrap();
    let context = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    let (_, turns) = store
        .interactive_journal()
        .snapshot(&context, &session.id)
        .unwrap();
    turns
        .iter()
        .filter(|turn| turn.phase == TurnPhase::Completed)
        .count()
        == count
}
fn pinned(f: &Fixture, session: &Session) -> Option<ProfileRef> {
    let store = f.state.store.lock().unwrap();
    let context = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    store
        .interactive_journal()
        .session_profile(&context, &session.id)
        .unwrap()
}

#[tokio::test]
async fn published_profile_and_versioned_skills_reach_acp_and_new_publication_never_mutates_old_pin()
 {
    let f = fixture("normal");
    let first_definition = definition("Profile instruction version ONE.");
    let first = publish(&f, first_definition.clone());
    let first_ref = reference(&first);
    let accepted = create(&f, &first_ref, "profile-original-intent-0001").unwrap();
    assert!(!accepted.replayed);
    assert_eq!(pinned(&f, &accepted.session), Some(first_ref.clone()));
    enqueue(&f, &accepted.session, "first", "first user request");
    until(&f, || completed(&f, &accepted.session, 1)).await;
    let expected_first = format!(
        "{}\n\n## User request\nfirst user request\n",
        first_definition.render_instructions()
    );
    assert_eq!(received(&f), expected_first);
    {
        let store = f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let events = store
            .interactive_journal()
            .replay(&context, &accepted.session.id, 0)
            .unwrap();
        let ready = events
            .iter()
            .find(|event| event.kind == "harness_ready")
            .unwrap();
        assert_eq!(
            ready.payload["backend"]["profile"],
            serde_json::to_value(&first_ref).unwrap()
        );
    }
    let second_definition = definition("Profile instruction version TWO.");
    let second = {
        let store = f.state.store.lock().unwrap();
        let draft = store
            .save_agent(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                &first.agent_id,
                first.draft_revision,
                second_definition.clone(),
            )
            .unwrap();
        store
            .publish_agent(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                &draft.id,
                draft.revision,
            )
            .unwrap()
    };
    assert_eq!(second.version, first.version + 1);
    let replay = create(&f, &first_ref, "profile-original-intent-0001").unwrap();
    assert!(replay.replayed);
    assert_eq!(
        serde_json::to_value(replay.session).unwrap(),
        serde_json::to_value(&accepted.session).unwrap()
    );
    assert_eq!(starts(&f), 1);
    enqueue(&f, &accepted.session, "second", "second user request");
    until(&f, || completed(&f, &accepted.session, 2)).await;
    assert_eq!(pinned(&f, &accepted.session), Some(first_ref));
    assert_eq!(
        received(&f),
        format!(
            "{expected_first}{}\n\n## User request\nsecond user request\n",
            first_definition.render_instructions()
        )
    );
    let fresh = create(&f, &reference(&second), "profile-new-version-intent-0002").unwrap();
    enqueue(&f, &fresh.session, "third", "third user request");
    until(&f, || completed(&f, &fresh.session, 1)).await;
    assert_eq!(starts(&f), 2);
    assert!(received(&f).ends_with(&format!(
        "{}\n\n## User request\nthird user request\n",
        second_definition.render_instructions()
    )));
    assert_eq!(pinned(&f, &fresh.session), Some(reference(&second)));
    f.state.interactive.shutdown().await;
}

#[tokio::test]
async fn profile_digest_harness_and_request_override_mismatch_fail_before_session_or_process() {
    let f = fixture("normal");
    let version = publish(&f, definition("approved context"));
    let mut wrong = reference(&version);
    let replacement = if wrong.digest.starts_with('0') {
        "1"
    } else {
        "0"
    };
    wrong.digest.replace_range(..1, replacement);
    assert_eq!(
        create(&f, &wrong, "profile-invalid-digest-intent-0001").unwrap_err(),
        "agent_profile_changed"
    );
    let mut mismatch = definition("wrong harness");
    mismatch.harness = "codex".into();
    let mismatch = publish(&f, mismatch);
    assert_eq!(
        create(
            &f,
            &reference(&mismatch),
            "profile-wrong-harness-intent-0002"
        )
        .unwrap_err(),
        "agent_profile_harness_unavailable"
    );
    assert!(
        serde_json::from_value::<CreateRequest>(
            json!({"profile":reference(&version),"model":"unapproved-override"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<CreateRequest>(
            json!({"profile":reference(&version),"instructions":"unapproved override"})
        )
        .is_err()
    );
    assert_eq!(starts(&f), 0);
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .ahp_summaries(&f.auth.user.id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn startup_failure_keeps_atomic_profile_pin_and_receipt_without_restarting_on_retry() {
    let f = fixture("wrong-model");
    let version = publish(&f, definition("startup fixture"));
    let profile = reference(&version);
    let accepted = create(&f, &profile, "profile-startup-failure-intent-0001").unwrap();
    until(&f, || {
        phase(&f, &accepted.session) == SessionPhase::Interrupted
            && f.state.interactive.actors.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(pinned(&f, &accepted.session), Some(profile.clone()));
    assert!(received(&f).is_empty());
    let retry = create(&f, &profile, "profile-startup-failure-intent-0001").unwrap();
    assert!(retry.replayed);
    assert_eq!(
        serde_json::to_value(retry.session).unwrap(),
        serde_json::to_value(accepted.session).unwrap()
    );
    assert_eq!(starts(&f), 1);
    f.state.interactive.shutdown().await;
}

#[tokio::test]
async fn legacy_explicit_options_keep_plain_prompt_and_have_no_implicit_profile() {
    let f = fixture("normal");
    let session = start(&f);
    assert!(pinned(&f, &session).is_none());
    enqueue(&f, &session, "plain", "unchanged legacy user text");
    until(&f, || completed(&f, &session, 1)).await;
    assert_eq!(received(&f), "unchanged legacy user text\n");
    f.state.interactive.shutdown().await;
}
