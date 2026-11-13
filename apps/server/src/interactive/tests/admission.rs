use super::*;
const KEY: &str = "runtime-admission-fixture-0001";
fn options() -> Options {
    Options {
        model: "fixture-model".into(),
        mode: "plan".into(),
        max_seconds: 10,
        max_output_bytes: 1024 * 1024,
    }
}
fn create(f: &Fixture, key: &str) -> Result<CreateAcceptance> {
    Service::create_with_key(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        options(),
        Some(key.into()),
    )
}
fn lines(f: &Fixture, name: &str) -> usize {
    std::fs::read_to_string(f.dir.path().join("checkout").join(name))
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test]
async fn exact_admission_retry_bypasses_full_runtime_and_connection_capacity_then_shutdown_without_resurrection()
 {
    let f = fixture("normal");
    let first = create(&f, KEY).unwrap();
    assert!(!first.replayed);
    until(&f, || lines(&f, "received-initializes") == 1).await;
    let slots = f
        .state
        .interactive
        .slots
        .clone()
        .try_acquire_many_owned(7)
        .unwrap();
    let mut connections = Vec::new();
    while let Ok(permit) = f.state.connection_permit() {
        connections.push(permit);
    }
    assert!(!connections.is_empty());
    let replay = create(&f, KEY).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        serde_json::to_value(&first.session).unwrap(),
        serde_json::to_value(&replay.session).unwrap()
    );
    assert_eq!(
        create(&f, "new-capacity-intent-0002").unwrap_err(),
        "runtime_capacity_reached"
    );
    drop(slots);
    assert_eq!(
        create(&f, "new-connection-intent-0003").unwrap_err(),
        "harness_management_busy"
    );
    assert_eq!(f.state.interactive.actors.lock().unwrap().len(), 1);
    assert_eq!(lines(&f, "actor-starts"), 1);
    drop(connections);
    f.state.interactive.shutdown().await;
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
    assert!(create(&f, KEY).unwrap().replayed);
    assert_eq!(
        create(&f, "new-shutdown-intent-0004").unwrap_err(),
        "runtime_shutting_down"
    );
    let mut disabled = f.state.clone();
    Arc::make_mut(&mut disabled.config).experimental_interactive_runtime = false;
    let replay = Service::create_with_key(
        disabled,
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        options(),
        Some(KEY.into()),
    )
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.session.id, first.session.id);
    assert_eq!(lines(&f, "actor-starts"), 1);
    assert_eq!(phase(&f, &first.session), SessionPhase::Interrupted);
}

#[tokio::test]
async fn concurrent_duplicate_admission_starts_one_real_acp_process_and_acquires_one_lease() {
    let f = fixture("normal");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let state = f.state.clone();
        let auth = AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        };
        let workspace = f.workspace.clone();
        let project = f.project.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            barrier.wait();
            Service::create_with_key(state, auth, workspace, project, options(), Some(KEY.into()))
                .unwrap()
        }));
    }
    let first = tasks.remove(0).await.unwrap();
    let second = tasks.remove(0).await.unwrap();
    assert_eq!(first.session.id, second.session.id);
    assert_ne!(first.replayed, second.replayed);
    until(&f, || lines(&f, "received-initializes") == 1).await;
    assert_eq!(lines(&f, "actor-starts"), 1);
    assert_eq!(f.state.interactive.actors.lock().unwrap().len(), 1);
    {
        let store = f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let events = store
            .interactive_journal()
            .replay(&context, &first.session.id, 0)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "runtime_acquired")
                .count(),
            1
        );
        assert_eq!(store.ahp_summaries(&f.auth.user.id).unwrap().len(), 1);
    }
    f.state.interactive.shutdown().await;
}

#[tokio::test]
async fn startup_failure_retains_original_acceptance_and_retry_never_starts_a_second_process() {
    let f = fixture("wrong-model");
    let first = create(&f, KEY).unwrap();
    until(&f, || {
        phase(&f, &first.session) == SessionPhase::Interrupted
            && f.state.interactive.actors.lock().unwrap().is_empty()
    })
    .await;
    let replay = create(&f, KEY).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.session.id, first.session.id);
    assert_eq!(
        replay.session.phase,
        SessionPhase::Active,
        "Receipt is the historical acceptance, not current runtime state"
    );
    assert_eq!(lines(&f, "actor-starts"), 1);
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
}

#[tokio::test]
async fn original_binding_native_and_logical_epochs_are_part_of_admission_identity() {
    for change in ["native", "binding", "logical"] {
        let f = fixture("normal");
        let first = create(&f, KEY).unwrap();
        until(&f, || lines(&f, "received-initializes") == 1).await;
        {
            let store = f.state.store.lock().unwrap();
            let binding = store
                .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
                .unwrap()
                .unwrap();
            match change {
                "native" => {
                    store
                        .revoke_grant(&binding.native_project_id, &f.auth.user.id)
                        .unwrap();
                    store
                        .grant(&binding.native_project_id, &f.auth.user.id)
                        .unwrap();
                }
                "binding" => {
                    store
                        .revoke_local_binding(
                            &f.auth.user.id,
                            &f.workspace,
                            &f.project,
                            binding.revision,
                        )
                        .unwrap();
                    store
                        .set_local_binding(
                            &f.auth.user.id,
                            &f.workspace,
                            &f.project,
                            &binding.native_project_id,
                            Some(binding.revision + 1),
                        )
                        .unwrap();
                }
                _ => {
                    let grant = store
                        .team_project_grant(&f.workspace, &f.project, &f.auth.user.id)
                        .unwrap()
                        .unwrap();
                    let mut reduced = grant.permissions.clone();
                    reduced.remove(&Permission::CreateSession);
                    store
                        .set_team_project_grant(
                            &f.auth.user.id,
                            &f.workspace,
                            &f.project,
                            &f.auth.user.id,
                            reduced,
                            Some(grant.revision),
                        )
                        .unwrap();
                    store
                        .set_team_project_grant(
                            &f.auth.user.id,
                            &f.workspace,
                            &f.project,
                            &f.auth.user.id,
                            grant.permissions,
                            Some(grant.revision + 1),
                        )
                        .unwrap();
                }
            }
        }
        assert_eq!(
            create(&f, KEY).unwrap_err(),
            "idempotency_conflict",
            "{change}"
        );
        until(&f, || {
            phase(&f, &first.session) == SessionPhase::Interrupted
        })
        .await;
        assert_eq!(lines(&f, "actor-starts"), 1);
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
}

#[test]
fn missing_executor_and_invalid_admission_keys_fail_before_any_durable_session() {
    let f = fixture("normal");
    for key in [
        "short",
        "invalid key with spaces",
        "invalid:key:with:punctuation",
        "niepoprawny-klucz-żółć",
    ] {
        assert_eq!(create(&f, key).unwrap_err(), "invalid_idempotency_key");
    }
    assert_eq!(create(&f, KEY).unwrap_err(), "runtime_executor_unavailable");
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .ahp_summaries(&f.auth.user.id)
            .unwrap()
            .is_empty()
    );
    assert!(!f.dir.path().join("checkout/actor-starts").exists());
}
