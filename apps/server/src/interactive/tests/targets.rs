use super::*;
use dunlin_domain::binding::{BindingRef, ProjectTargetRef};
const KEY: &str = "target-admission-fixture-0001";
fn options() -> Options {
    Options {
        model: "fixture-model".into(),
        mode: "plan".into(),
        max_seconds: 10,
        max_output_bytes: 1048576,
    }
}
fn request(target: Option<ProjectTargetRef>) -> CreateRequest {
    let mut request = LegacyCreateRequest::from(options());
    request.target = target;
    CreateRequest::Legacy(request)
}
fn local(revision: u64) -> Option<ProjectTargetRef> {
    Some(ProjectTargetRef::Local { revision })
}
fn remote() -> Option<ProjectTargetRef> {
    Some(ProjectTargetRef::Remote {
        binding: BindingRef {
            id: "rbd-explicit".into(),
            revision: 1,
        },
    })
}
fn create(f: &Fixture, request: CreateRequest, key: &str) -> Result<CreateAcceptance> {
    Service::create_request_with_key(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        request,
        Some(key.into()),
    )
}
fn starts(f: &Fixture) -> usize {
    std::fs::read_to_string(f.dir.path().join("checkout/actor-starts"))
        .unwrap_or_default()
        .lines()
        .count()
}
#[test]
fn strict_create_targets_do_not_fall_through_to_legacy_or_profile_overrides() {
    let base = serde_json::to_value(options()).unwrap();
    for target in [
        json!({"kind":"local","revision":1}),
        json!({"kind":"remote","binding":{"id":"rbd-explicit","revision":1}}),
    ] {
        let mut body = base.clone();
        body["target"] = target;
        assert!(serde_json::from_value::<CreateRequest>(body).is_ok());
    }
    for target in [
        json!({"kind":"unknown","revision":1}),
        json!({"kind":"local","revision":1,"path":"/outside"}),
        json!({"kind":"remote","binding":{"id":"rbd-explicit","revision":1,"root":"/outside"}}),
    ] {
        let mut body = base.clone();
        body["target"] = target;
        assert!(serde_json::from_value::<CreateRequest>(body).is_err());
    }
    let profile = json!({"agent_id":"agent-one","version":1,"digest":"a".repeat(64)});
    let valid = json!({"profile":profile,"target":{"kind":"local","revision":1}});
    assert!(serde_json::from_value::<CreateRequest>(valid.clone()).is_ok());
    let mut mixed = valid;
    mixed["model"] = json!("unapproved");
    assert!(serde_json::from_value::<CreateRequest>(mixed).is_err());
    let invalid = request(local(0));
    assert_eq!(
        invalid.validate_target().unwrap_err(),
        "invalid_project_target"
    );
    assert_eq!(
        request(remote()).validate_target().unwrap_err(),
        "remote_execution_not_supported"
    );
}
#[tokio::test]
async fn omitted_target_keeps_pre_target_admission_fingerprint_and_null_replays_exact_receipt() {
    let f = fixture("normal");
    let legacy_request = request(None);
    let legacy_body = serde_json::to_value(&legacy_request).unwrap();
    assert!(legacy_body.get("target").is_none());
    // Preserve the published pre-target fingerprint contract, including omission
    // of the new field rather than introducing a null/default target component.
    let old_fingerprint = {
        let store = f.state.store.lock().unwrap();
        let (binding, native) = store
            .resolve_local_binding(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                Permission::CreateSession,
            )
            .unwrap()
            .unwrap();
        let native_grant = store
            .capture_native_grant(&native.id, &f.auth.user.id)
            .unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let authority = context
            .authorize_project(&f.workspace, &f.project, Permission::CreateSession)
            .unwrap();
        let old = json!({"workspace":f.workspace,"project":f.project,"options":options(),"authority":authority,"binding":binding,"nativeGrant":native_grant,"nativePath":native.path,"harness":crate::management::harness(&f.state.config)});
        hex::encode(Sha256::digest(serde_json::to_vec(&old).unwrap()))
    };
    assert_eq!(
        prepare_create_scope(&f.state, &f.auth, &f.workspace, &f.project, &legacy_request)
            .unwrap()
            .scope
            .fingerprint,
        old_fingerprint,
    );
    assert_eq!(old_fingerprint.len(), 64);
    let accepted = create(&f, legacy_request, KEY).unwrap();
    let mut body = legacy_body;
    body["target"] = Value::Null;
    let replay = create(&f, serde_json::from_value(body).unwrap(), KEY).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        serde_json::to_value(replay.session).unwrap(),
        serde_json::to_value(accepted.session).unwrap()
    );
    assert_eq!(
        create(&f, request(local(1)), KEY).unwrap_err(),
        "idempotency_conflict"
    );
    f.state.interactive.shutdown().await;
}
#[tokio::test]
async fn explicit_remote_never_replays_or_creates_local_work_even_when_local_runtime_is_unavailable()
 {
    let f = fixture("normal");
    let profile_remote = CreateRequest::Profile {
        mcp_connections: vec![],
        profile: ProfileRef {
            agent_id: "unresolved-profile".into(),
            version: 1,
            digest: "a".repeat(64),
        },
        target: remote(),
    };
    assert_eq!(
        create(&f, profile_remote, KEY).unwrap_err(),
        "remote_execution_not_supported"
    );
    assert_eq!(
        create(&f, request(remote()), KEY).unwrap_err(),
        "remote_execution_not_supported"
    );
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
    let first = create(&f, request(local(1)), KEY).unwrap();
    assert!(
        !first.replayed,
        "Remote refusal must not reserve the request identity"
    );
    until(&f, || starts(&f) == 1).await;
    assert_eq!(
        create(&f, request(remote()), KEY).unwrap_err(),
        "remote_execution_not_supported"
    );
    assert_eq!(starts(&f), 1);
    let slots = f
        .state
        .interactive
        .slots
        .clone()
        .try_acquire_many_owned(7)
        .unwrap();
    assert_eq!(
        create(&f, request(remote()), KEY).unwrap_err(),
        "remote_execution_not_supported"
    );
    f.state.interactive.shutdown().await;
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_binding(&f.auth.user.id, &f.workspace, &f.project, 1)
        .unwrap();
    let mut disabled = f.state.clone();
    Arc::make_mut(&mut disabled.config).experimental_interactive_runtime = false;
    assert_eq!(
        Service::create_request_with_key(
            disabled,
            AuthUser {
                user: f.auth.user.clone(),
                device: f.auth.device.clone()
            },
            f.workspace.clone(),
            f.project.clone(),
            request(remote()),
            Some(KEY.into())
        )
        .unwrap_err(),
        "remote_execution_not_supported"
    );
    drop(slots);
    assert_eq!(starts(&f), 1);
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .ahp_summaries(&f.auth.user.id)
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn explicit_local_pin_is_checked_on_receipt_replay_and_keeps_live_authority_fences() {
    let f = fixture("normal");
    assert_eq!(
        create(&f, request(local(2)), KEY).unwrap_err(),
        "project_target_changed"
    );
    let first = create(&f, request(local(1)), KEY).unwrap();
    let held = f
        .state
        .interactive
        .slots
        .clone()
        .try_acquire_many_owned(7)
        .unwrap();
    let replay = create(&f, request(local(1)), KEY).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.session.id, first.session.id);
    assert_eq!(
        create(&f, request(None), KEY).unwrap_err(),
        "idempotency_conflict"
    );
    {
        let store = f.state.store.lock().unwrap();
        let binding = store
            .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
            .unwrap()
            .unwrap();
        store
            .revoke_grant(&binding.native_project_id, &f.auth.user.id)
            .unwrap();
    }
    assert!(create(&f, request(local(1)), KEY).is_err());
    {
        let store = f.state.store.lock().unwrap();
        let native = store
            .list_projects()
            .unwrap()
            .into_iter()
            .find(|p| p.path == f.dir.path().join("checkout").to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &f.auth.user.id).unwrap();
        store
            .set_local_binding(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                &native.id,
                Some(1),
            )
            .unwrap();
    }
    assert_eq!(
        create(&f, request(local(1)), KEY).unwrap_err(),
        "project_target_changed"
    );
    assert_eq!(
        create(&f, request(local(2)), KEY).unwrap_err(),
        "idempotency_conflict"
    );
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&f.auth.device.id)
        .unwrap();
    assert_eq!(
        create(&f, request(local(2)), KEY).unwrap_err(),
        "authority_revoked"
    );
    assert_eq!(
        create(&f, request(None), KEY).unwrap_err(),
        "authority_revoked"
    );
    drop(held);
    f.state.interactive.shutdown().await;
}
