use super::*;
use crate::{config::Config, store::Store};
use dunlin_session_runtime::SessionPhase;
mod admission;
mod checkout;
mod profiles;
mod scoped;
mod targets;
struct Fixture {
    state: AppState,
    auth: AuthUser,
    workspace: String,
    project: String,
    dir: tempfile::TempDir,
}
fn fixture(behavior: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let store = Store::open_in_memory().unwrap();
    let user = store.create_user("runtime-owner", "admin").unwrap();
    let (device, _) = store.create_device(&user.id, "browser").unwrap();
    let workspace = store.create_workspace(&user, "runtime").unwrap();
    let project = store
        .create_team_project(&user.id, &workspace.id, "logical")
        .unwrap();
    let native = store
        .create_project("checkout", root.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &user.id).unwrap();
    store
        .set_local_binding(&user.id, &workspace.id, &project.id, &native.id, None)
        .unwrap();
    let mut state = AppState::new(
        store,
        Config {
            sandbox: false,
            db: dir.path().join("relay.db"),
            experimental_interactive_runtime: true,
            ..Default::default()
        },
    );
    Arc::get_mut(&mut state.interactive).unwrap().command = Some((
        "python3".into(),
        vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/interactive/tests/fake_acp.py"
            )
            .into(),
            behavior.into(),
        ],
    ));
    Fixture {
        state,
        auth: AuthUser { user, device },
        workspace: workspace.id,
        project: project.id,
        dir,
    }
}
fn start(f: &Fixture) -> Session {
    Service::create(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        Options {
            model: "fixture-model".into(),
            mode: "plan".into(),
            max_seconds: 10,
            max_output_bytes: 1024 * 1024,
        },
    )
    .unwrap()
}
fn enqueue(f: &Fixture, session: &Session, key: &str, text: &str) -> String {
    let store = f.state.store.lock().unwrap();
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    store
        .interactive_journal()
        .enqueue(&a, &session.id, key, text)
        .unwrap()
}
async fn until(f: &Fixture, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !check() {
        if Instant::now() > deadline {
            panic!(
                "Actor deadline; registry entries: {}",
                f.state.interactive.actors.lock().unwrap().len()
            );
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}
fn phase(f: &Fixture, session: &Session) -> SessionPhase {
    let store = f.state.store.lock().unwrap();
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    store
        .interactive_journal()
        .snapshot(&a, &session.id)
        .unwrap()
        .0
        .phase
}
async fn stop(f: &Fixture, s: &Session) {
    f.state
        .interactive
        .stop(&f.state, &f.auth, &s.id, s.epoch)
        .unwrap();
    until(f, || {
        !f.state
            .interactive
            .actors
            .lock()
            .unwrap()
            .contains_key(&s.id)
    })
    .await;
}
#[tokio::test]
async fn process_and_serial_turns_survive_all_observers_disconnecting() {
    let f = fixture("normal");
    let s = start(&f);
    let observer = f.state.interactive.actors.lock().unwrap()[&s.id]
        .events
        .subscribe();
    drop(observer);
    let first = enqueue(&f, &s, "one", "first");
    let second = enqueue(&f, &s, "two", "second");
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&a, &s.id)
            .unwrap()
            .1
            .iter()
            .filter(|t| t.phase == TurnPhase::Completed)
            .count()
            == 2
    })
    .await;
    {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let events = store.interactive_journal().replay(&a, &s.id, 0).unwrap();
        assert!(events.iter().any(|e| e.kind == "harness_ready"));
        assert!(events.iter().any(|e| e.kind == "agent_update"));
        let turns = store.interactive_journal().snapshot(&a, &s.id).unwrap().1;
        assert_eq!(
            [turns[0].id.as_str(), turns[1].id.as_str()],
            [first.as_str(), second.as_str()]
        );
    }
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "first\nsecond\n"
    );
    let mut observer = f.state.interactive.actors.lock().unwrap()[&s.id]
        .events
        .subscribe();
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert!(matches!(
        observer.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    stop(&f, &s).await;
}
#[tokio::test]
async fn revocation_stops_a_running_process_and_leaves_unknown_without_replay() {
    let f = fixture("hang");
    let s = start(&f);
    enqueue(&f, &s, "one", "uncertain");
    until(&f, || {
        f.dir.path().join("checkout/received-prompts").exists()
    })
    .await;
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&f.auth.device.id)
        .unwrap();
    until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
    let store = f.state.store.lock().unwrap();
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    assert_eq!(
        store.interactive_journal().snapshot(&a, &s.id).unwrap().1[0].phase,
        TurnPhase::Unknown
    );
}
#[tokio::test]
async fn unapproved_permission_wrong_scope_and_profile_mismatch_fail_closed() {
    for behavior in ["permission", "wrong-session", "wrong-model", "oversize"] {
        let f = fixture(behavior);
        let s = start(&f);
        enqueue(&f, &s, "one", "test");
        until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
        if behavior == "wrong-model" {
            assert!(!f.dir.path().join("checkout/received-prompts").exists());
        }
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        assert!(
            store
                .interactive_journal()
                .snapshot(&a, &s.id)
                .unwrap()
                .1
                .iter()
                .all(|t| t.phase != TurnPhase::Completed)
        );
    }
}

#[tokio::test]
async fn checkout_binding_and_policy_changes_interrupt_the_actor() {
    for change in ["binding", "settings", "root"] {
        let f = fixture("hang");
        let s = start(&f);
        enqueue(&f, &s, "one", "uncertain");
        until(&f, || {
            f.dir.path().join("checkout/received-prompts").exists()
        })
        .await;
        if change == "root" {
            std::fs::rename(
                f.dir.path().join("checkout"),
                f.dir.path().join("old-checkout"),
            )
            .unwrap();
            std::fs::create_dir(f.dir.path().join("checkout")).unwrap();
        } else {
            let store = f.state.store.lock().unwrap();
            if change == "binding" {
                store
                    .revoke_local_binding(&f.auth.user.id, &f.workspace, &f.project, 1)
                    .unwrap();
            } else {
                let binding = store
                    .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
                    .unwrap()
                    .unwrap();
                store
                    .save_project_settings(
                        &binding.native_project_id,
                        "{\"permission_mode\":\"ReadOnly\"}",
                    )
                    .unwrap();
            }
        }
        until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
    }
}
#[tokio::test]
async fn wall_time_budget_terminates_a_hung_turn() {
    let f = fixture("hang");
    let s = Service::create(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        Options {
            model: "fixture-model".into(),
            mode: "plan".into(),
            max_seconds: 1,
            max_output_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    enqueue(&f, &s, "one", "uncertain");
    until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
}
#[cfg(unix)]
#[tokio::test]
async fn explicit_stop_kills_the_process_group_and_does_not_report_success() {
    let f = fixture("descendant");
    let s = start(&f);
    enqueue(&f, &s, "one", "uncertain");
    until(&f, || {
        f.dir.path().join("checkout/received-prompts").exists()
    })
    .await;
    let parent: i32 = std::fs::read_to_string(f.dir.path().join("checkout/actor-pid"))
        .unwrap()
        .parse()
        .unwrap();
    let child: i32 = std::fs::read_to_string(f.dir.path().join("checkout/descendant-pid"))
        .unwrap()
        .parse()
        .unwrap();
    stop(&f, &s).await;
    fn running(pid: i32) -> bool {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && stat
                .rsplit_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with('Z'))
        {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }
    until(&f, || !running(parent) && !running(child)).await;
    assert_eq!(phase(&f, &s), SessionPhase::Interrupted);
}

#[tokio::test]
async fn server_shutdown_stops_owned_processes_and_rejects_new_work() {
    let f = fixture("hang");
    let s = start(&f);
    enqueue(&f, &s, "one", "uncertain");
    until(&f, || {
        f.dir.path().join("checkout/received-prompts").exists()
    })
    .await;
    f.state.interactive.shutdown().await;
    assert_eq!(phase(&f, &s), SessionPhase::Interrupted);
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
    assert_eq!(
        Service::create(
            f.state.clone(),
            AuthUser {
                user: f.auth.user.clone(),
                device: f.auth.device.clone()
            },
            f.workspace.clone(),
            f.project.clone(),
            Options {
                model: "fixture-model".into(),
                mode: "plan".into(),
                max_seconds: 10,
                max_output_bytes: 1024
            }
        )
        .err(),
        Some("runtime_shutting_down")
    );
}

#[tokio::test]
async fn cookie_bootstrap_is_gated_scoped_and_does_not_expose_prompts() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    async fn request(
        f: &Fixture,
        cookie: &str,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Vec<u8>) {
        let response = crate::api::router(f.state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "localhost:8787")
                    .header("origin", "http://localhost:8787")
                    .header("cookie", cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        (
            response.status(),
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
    }
    let mut f = fixture("normal");
    let cookie = format!(
        "dunlin_web_session={}",
        f.state
            .store
            .lock()
            .unwrap()
            .create_browser_session(&f.auth.device.id, 3600)
            .unwrap()
    );
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/interactive-sessions",
        f.workspace, f.project
    );
    let options =
        json!({"model":"fixture-model","mode":"plan","max_seconds":10,"max_output_bytes":1048576});
    Arc::make_mut(&mut f.state.config).experimental_interactive_runtime = false;
    assert_eq!(
        request(&f, &cookie, "POST", &path, options.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    Arc::make_mut(&mut f.state.config).experimental_interactive_runtime = true;
    let mut invalid = options.clone();
    invalid["command"] = json!("unapproved");
    assert_eq!(
        request(&f, &cookie, "POST", &path, invalid).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, body) = request(&f, &cookie, "POST", &path, options).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let s: Value = serde_json::from_slice(&body).unwrap();
    let id = s["session"]["id"].as_str().unwrap();
    let session = {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let session = store.interactive_journal().snapshot(&a, id).unwrap().0;
        store
            .interactive_journal()
            .enqueue(&a, id, "secret", "private prompt marker")
            .unwrap();
        session
    };
    let metadata = format!("/v1/web/interactive-sessions/{id}");
    let (status, body) = request(&f, &cookie, "GET", &metadata, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !String::from_utf8(body)
            .unwrap()
            .contains("private prompt marker")
    );
    let stranger = {
        let store = f.state.store.lock().unwrap();
        let user = store.create_user("stranger", "member").unwrap();
        let (device, _) = store.create_device(&user.id, "browser").unwrap();
        format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        )
    };
    assert_ne!(
        request(&f, &stranger, "GET", &metadata, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f,
            &cookie,
            "POST",
            &format!("{metadata}/stop"),
            json!({"expected_epoch":session.epoch+1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &f,
            &cookie,
            "POST",
            &format!("{metadata}/stop"),
            json!({"expected_epoch":session.epoch})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    until(&f, || phase(&f, &session) == SessionPhase::Interrupted).await;
}

#[tokio::test]
async fn read_only_policy_requires_a_kernel_adapter_before_any_process_starts() {
    let f = fixture("normal");
    {
        let store = f.state.store.lock().unwrap();
        let binding = store
            .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
            .unwrap()
            .unwrap();
        let settings = ProjectSettings {
            permission_mode: Some(dunlin_domain::PermissionMode::ReadOnly),
            ..Default::default()
        };
        store
            .save_project_settings(
                &binding.native_project_id,
                &serde_json::to_string(&settings).unwrap(),
            )
            .unwrap();
        store
            .set_local_binding(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                &binding.native_project_id,
                Some(binding.revision),
            )
            .unwrap();
    }
    let error = Service::create(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        Options {
            model: "fixture-model".into(),
            mode: "plan".into(),
            max_seconds: 10,
            max_output_bytes: 1024,
        },
    )
    .err();
    assert_eq!(error, Some("read_only_kernel_adapter_not_available"));
    assert!(!f.dir.path().join("checkout/actor-pid").exists());
}

fn control_fixture(behavior: &str) -> Fixture {
    let mut f = fixture("normal");
    Arc::get_mut(&mut f.state.interactive).unwrap().command = Some((
        "python3".into(),
        vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/acp/control_peer.py"
            )
            .into(),
            behavior.into(),
        ],
    ));
    f
}
fn actor_cancel(f: &Fixture, s: &Session, turn: &str) -> ahp_types::commands::DispatchActionParams {
    let p=serde_json::from_value(json!({"channel":format!("ahp-chat:/{}",s.id),"clientSeq":1,"action":{"type":"chat/turnCancelled","turnId":turn,"duration":99999999}})).unwrap();
    let store = f.state.store.lock().unwrap();
    assert!(
        f.state
            .interactive
            .queue_text_allowed(&f.state, &store, &s.id, &f.auth.user.id)
    );
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    let elapsed = f.state.interactive.turn_elapsed(&s.id, turn);
    let envelope = store
        .interactive_journal()
        .dispatch_action(
            &a,
            &s.id,
            &f.auth.device.id,
            "actor-control",
            &p,
            dunlin_session_runtime::DispatchAdmission {
                rejection: None,
                elapsed_ms: elapsed,
            },
        )
        .unwrap();
    assert!(envelope.rejection_reason.is_none());
    if let ahp_types::actions::StateAction::ChatTurnCancelled(a) = envelope.action {
        assert!(a.duration < 99999999);
    } else {
        panic!("Expected cancellation");
    }
    p
}
#[tokio::test]
async fn cooperative_cancel_is_forwarded_once_and_actor_continues_only_after_prompt_settles() {
    let f = control_fixture("cooperative");
    let s = start(&f);
    let first = enqueue(&f, &s, "held", "hold active");
    until(&f, || {
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts"))
            .unwrap_or_default()
            .contains("hold active")
    })
    .await;
    let second = enqueue(&f, &s, "after", "after cancellation");
    let p = actor_cancel(&f, &s, &first);
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let turns = store.interactive_journal().snapshot(&a, &s.id).unwrap().1;
        turns
            .iter()
            .any(|t| t.id == first && t.phase == TurnPhase::Cancelled)
            && turns
                .iter()
                .any(|t| t.id == second && t.phase == TurnPhase::Completed)
    })
    .await;
    {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let ack = store
            .interactive_journal()
            .dispatch_action(
                &a,
                &s.id,
                &f.auth.device.id,
                "actor-control",
                &p,
                dunlin_session_runtime::DispatchAdmission {
                    rejection: None,
                    elapsed_ms: None,
                },
            )
            .unwrap();
        assert!(ack.rejection_reason.is_none());
    }
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "hold active\nafter cancellation\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-cancels"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let events = std::fs::read_to_string(f.dir.path().join("checkout/control-events")).unwrap();
    assert!(
        events.find("cancelled-result").unwrap()
            < events.find("prompt:after cancellation").unwrap()
    );
    assert_eq!(phase(&f, &s), SessionPhase::Active);
    stop(&f, &s).await;
}
#[tokio::test]
async fn unconfirmed_cancel_kills_actor_marks_unknown_and_never_advances_queued_work() {
    let f = control_fixture("ignore-cancel");
    let s = start(&f);
    let first = enqueue(&f, &s, "held", "hold noncooperative");
    until(&f, || {
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts"))
            .unwrap_or_default()
            .contains("hold noncooperative")
    })
    .await;
    let queued = enqueue(&f, &s, "after", "must stay queued");
    actor_cancel(&f, &s, &first);
    until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
    {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let j = store.interactive_journal();
        let turns = j.snapshot(&a, &s.id).unwrap().1;
        assert!(
            turns
                .iter()
                .any(|t| t.id == first && t.phase == TurnPhase::Unknown)
        );
        assert!(
            turns
                .iter()
                .any(|t| t.id == queued && t.phase == TurnPhase::Queued)
        );
        let summary = j.channel_summary(&a, &s.id).unwrap();
        assert_ne!(summary.status & 2, 0);
        assert_eq!(
            summary.meta.unwrap()["dunlin.cancellationUnconfirmed"],
            true
        );
        assert!(
            !f.state
                .interactive
                .queue_text_allowed(&f.state, &store, &s.id, &f.auth.user.id)
        );
    }
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "hold noncooperative\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-cancels"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    f.state.interactive.shutdown().await;
}

fn permission_fixture(behavior: &str) -> Fixture {
    let mut f = fixture("normal");
    Arc::get_mut(&mut f.state.interactive).unwrap().command = Some((
        "python3".into(),
        vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/acp/permission_peer.py"
            )
            .into(),
            behavior.into(),
        ],
    ));
    f
}
fn permission_action(
    f: &Fixture,
    s: &Session,
) -> Option<ahp_types::commands::DispatchActionParams> {
    let store = f.state.store.lock().unwrap();
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
        .unwrap();
    let chat = serde_json::to_value(
        store
            .interactive_journal()
            .projected_chat(&a, &s.id)
            .unwrap(),
    )
    .unwrap();
    let turn = chat["activeTurn"].as_object()?;
    let parts = turn.get("responseParts")?.as_array()?;
    let tool =
        parts.iter().find(|p| {
            p["kind"] == "toolCall" && p["toolCall"]["status"] == "pending-confirmation"
        })?["toolCall"]
            .clone();
    let option = tool["options"]
        .as_array()?
        .iter()
        .find(|o| o["kind"] == "approve")?["id"]
        .clone();
    Some(serde_json::from_value(json!({"channel":format!("ahp-chat:/{}",s.id),"clientSeq":1,"action":{"type":"chat/toolCallConfirmed","turnId":turn["id"],"toolCallId":tool["toolCallId"],"approved":true,"selectedOptionId":option}})).unwrap())
}
#[tokio::test]
async fn approval_device_revoked_before_consumption_never_grants_acp_permission() {
    let f = permission_fixture("normal");
    let s = start(&f);
    let turn = enqueue(&f, &s, "permission", "test revoke before forward");
    until(&f, || permission_action(&f, &s).is_some()).await;
    let action = permission_action(&f, &s).unwrap();
    {
        // One Store critical section makes the accepted-decision/revocation race
        // deterministic; the actor cannot consume it between these operations.
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let (device, _) = store.create_device(&f.auth.user.id, "approver").unwrap();
        let result = store
            .interactive_journal()
            .dispatch_action(
                &a,
                &s.id,
                &device.id,
                "approval-test",
                &action,
                dunlin_session_runtime::DispatchAdmission::default(),
            )
            .unwrap();
        assert!(result.rejection_reason.is_none());
        store.revoke_device(&device.id).unwrap();
    }
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&a, &s.id)
            .unwrap()
            .1
            .iter()
            .any(|t| t.id == turn && t.phase == TurnPhase::Completed)
    })
    .await;
    assert!(!f.dir.path().join("checkout/allowed.txt").exists());
    assert!(!f.dir.path().join("checkout/performed-writes").exists());
    let responses =
        std::fs::read_to_string(f.dir.path().join("checkout/received-decisions")).unwrap();
    let responses = responses
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["response"]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert_eq!(phase(&f, &s), SessionPhase::Active);
    stop(&f, &s).await;
}
#[tokio::test]
async fn cancelling_a_human_permission_prompt_revokes_challenge_without_a_grant() {
    let f = permission_fixture("hold");
    let s = start(&f);
    let turn = enqueue(&f, &s, "permission", "test cancel while awaiting human");
    until(&f, || permission_action(&f, &s).is_some()).await;
    actor_cancel(&f, &s, &turn);
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&a, &s.id)
            .unwrap()
            .1
            .iter()
            .any(|t| t.id == turn && t.phase == TurnPhase::Cancelled)
    })
    .await;
    assert!(!f.dir.path().join("checkout/allowed.txt").exists());
    assert!(!f.dir.path().join("checkout/performed-writes").exists());
    assert_eq!(phase(&f, &s), SessionPhase::Active);
    let responses =
        std::fs::read_to_string(f.dir.path().join("checkout/received-decisions")).unwrap();
    assert_eq!(responses.lines().count(), 1);
    assert!(responses.contains("cancelled"));
    stop(&f, &s).await;
}
fn actor_process_running(pid: i32) -> bool {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        && stat.split_whitespace().nth(2) == Some("Z")
    {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}
fn actor_pid(f: &Fixture) -> i32 {
    std::fs::read_to_string(f.dir.path().join("checkout/actor-pid"))
        .unwrap()
        .parse()
        .unwrap()
}
#[tokio::test]
async fn native_grant_revoke_regrant_fences_existing_process_and_queued_work() {
    let f = fixture("normal");
    let s = start(&f);
    let first = enqueue(&f, &s, "before-epoch-change", "before native epoch");
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&context, &s.id)
            .unwrap()
            .1
            .iter()
            .any(|t| t.id == first && t.phase == TurnPhase::Completed)
    })
    .await;
    let old_pid = actor_pid(&f);
    assert!(actor_process_running(old_pid));
    {
        // The actor never observes a moment without an active native grant.
        // Current-access checks alone therefore cannot fence the old process.
        let store = f.state.store.lock().unwrap();
        let native = store
            .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
            .unwrap()
            .unwrap()
            .native_project_id;
        let captured = store
            .capture_native_grant(&native, &f.auth.user.id)
            .unwrap();
        store.revoke_grant(&native, &f.auth.user.id).unwrap();
        store.grant(&native, &f.auth.user.id).unwrap();
        assert!(
            store
                .project_for_user(&native, &f.auth.user.id)
                .unwrap()
                .is_some()
        );
        assert!(store.revalidate_native_grant(&captured).is_err());
        assert!(
            !f.state
                .interactive
                .queue_text_allowed(&f.state, &store, &s.id, &f.auth.user.id)
        );
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .enqueue(&context, &s.id, "stale-epoch", "must not reach old actor")
            .unwrap();
    }
    until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
    until(&f, || !actor_process_running(old_pid)).await;
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "before native epoch\n"
    );
    let fresh = start(&f);
    let fresh_turn = enqueue(&f, &fresh, "fresh-epoch", "fresh explicit create");
    until(&f, || {
        let store = f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&context, &fresh.id)
            .unwrap()
            .1
            .iter()
            .any(|t| t.id == fresh_turn && t.phase == TurnPhase::Completed)
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "before native epoch\nfresh explicit create\n"
    );
    stop(&f, &fresh).await;
}
#[tokio::test]
async fn native_grant_regrant_cannot_forward_an_accepted_old_permission() {
    let f = permission_fixture("normal");
    let s = start(&f);
    enqueue(&f, &s, "stale-permission", "old grant permission");
    until(&f, || permission_action(&f, &s).is_some()).await;
    let action = permission_action(&f, &s).unwrap();
    let old_pid = actor_pid(&f);
    {
        let store = f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let accepted = store
            .interactive_journal()
            .dispatch_action(
                &context,
                &s.id,
                &f.auth.device.id,
                "before-native-epoch-change",
                &action,
                dunlin_session_runtime::DispatchAdmission::default(),
            )
            .unwrap();
        assert!(accepted.rejection_reason.is_none());
        let native = store
            .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
            .unwrap()
            .unwrap()
            .native_project_id;
        store.revoke_grant(&native, &f.auth.user.id).unwrap();
        store.grant(&native, &f.auth.user.id).unwrap();
        assert!(store.capture_native_grant(&native, &f.auth.user.id).is_ok());
        assert!(
            !f.state
                .interactive
                .queue_text_allowed(&f.state, &store, &s.id, &f.auth.user.id)
        );
    }
    until(&f, || phase(&f, &s) == SessionPhase::Interrupted).await;
    until(&f, || !actor_process_running(old_pid)).await;
    assert!(!f.dir.path().join("checkout/allowed.txt").exists());
    assert!(!f.dir.path().join("checkout/performed-writes").exists());
    assert!(!f.dir.path().join("checkout/received-decisions").exists());
    // A newly and explicitly created actor captures the new grant epoch.
    let fresh = start(&f);
    enqueue(&f, &fresh, "fresh-permission", "fresh grant permission");
    until(&f, || permission_action(&f, &fresh).is_some()).await;
    let fresh_action = permission_action(&f, &fresh).unwrap();
    {
        let store = f.state.store.lock().unwrap();
        assert!(f.state.interactive.queue_text_allowed(
            &f.state,
            &store,
            &fresh.id,
            &f.auth.user.id
        ));
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let accepted = store
            .interactive_journal()
            .dispatch_action(
                &context,
                &fresh.id,
                &f.auth.device.id,
                "fresh-native-epoch",
                &fresh_action,
                dunlin_session_runtime::DispatchAdmission::default(),
            )
            .unwrap();
        assert!(accepted.rejection_reason.is_none());
    }
    until(&f, || {
        std::fs::read_to_string(f.dir.path().join("checkout/allowed.txt"))
            .is_ok_and(|text| text == "fresh grant permission")
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("checkout/allowed.txt")).unwrap(),
        "fresh grant permission"
    );
    let responses =
        std::fs::read_to_string(f.dir.path().join("checkout/received-decisions")).unwrap();
    let responses = responses
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["prompt"], "fresh grant permission");
    assert_eq!(
        responses[0]["response"]["result"]["outcome"]["optionId"],
        "once"
    );
    stop(&f, &fresh).await;
}
