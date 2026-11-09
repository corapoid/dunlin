use super::*;
use dunlin_coordination::{Execution, Run, RunStatus, Scope, new_id, now_ms};
use std::sync::Arc;

#[tokio::test]
async fn snapshots_are_user_and_project_scoped_and_revoked_devices_fail() {
    let mut fixture = Fixture::new();
    fixture.state.coordination = Some(Arc::new(
        dunlin_coordination::service::Service::start(&fixture.root.join("coordination")).unwrap(),
    ));
    let (user, device) = fixture
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&fixture.token)
        .unwrap()
        .unwrap();
    let (_, other_token) = {
        let db = fixture.state.store.lock().unwrap();
        let other = db.create_user("same-project-peer", "member").unwrap();
        db.grant(&fixture.project, &other.id).unwrap();
        db.create_device(&other.id, "other-device").unwrap()
    };
    let run = Run {
        id: new_id(),
        session: "observed".into(),
        scope: Scope {
            authority: user.id.clone(),
            project: fixture.project.clone(),
        },
        execution: Execution {
            root: fixture.root.to_string_lossy().into(),
            harness: "mock".into(),
            model: "model".into(),
            mode: "plan".into(),
            permission: "Ask".into(),
            configuration: "digest".into(),
        },
        title: "Private result".into(),
        status: RunStatus::Running,
        queued_prompts: None,
        created_ms: now_ms(),
        updated_ms: now_ms(),
        stop_reason: None,
        report: None,
        continuation: None,
    };
    fixture
        .state
        .coordination
        .as_ref()
        .unwrap()
        .store
        .lock()
        .unwrap()
        .start_run(run)
        .unwrap();
    let body = json!({"project":fixture.project,"operation":"snapshot"});
    let (status, mine) = fixture
        .post("/v1/coordination", Some(&fixture.token), body.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["ledger"]["runs"].as_array().unwrap().len(), 1);
    assert_eq!(mine["device"], device.id);
    let (status, other) = fixture
        .post("/v1/coordination", Some(&other_token), body.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(other["ledger"]["runs"].as_array().unwrap().is_empty());
    assert_eq!(
        fixture
            .post(
                "/v1/coordination",
                Some(&fixture.ungranted_token),
                body.clone()
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture.post("/v1/coordination", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        fixture
            .post("/v1/coordination", Some(&fixture.token), body)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn offline_coordinator_returns_an_explicit_error_not_empty_success() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .post(
                "/v1/coordination",
                Some(&fixture.token),
                json!({"project":fixture.project,"operation":"snapshot"})
            )
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn exact_approval_binds_the_live_device_connection_and_reconnect_revokes_it() {
    let mut f = Fixture::new();
    f.state.coordination = Some(Arc::new(
        dunlin_coordination::service::Service::start(&f.root.join("coordination")).unwrap(),
    ));
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let (_, second_token) = {
        let db = f.state.store.lock().unwrap();
        db.create_session("c", &user.id, &f.project, "New session", &device.id)
            .unwrap();
        db.create_device(&user.id, "second").unwrap()
    };
    let service = f.state.coordination.as_ref().unwrap().clone();
    let scope = Scope {
        authority: user.id.clone(),
        project: f.project.clone(),
    };
    let run = Run {
        id: new_id(),
        session: "c".into(),
        scope: scope.clone(),
        execution: Execution {
            root: f.root.to_string_lossy().into(),
            harness: "mock".into(),
            model: "model".into(),
            mode: "plan".into(),
            permission: "Ask".into(),
            configuration: "digest".into(),
        },
        title: "Propose".into(),
        status: RunStatus::Running,
        queued_prompts: None,
        created_ms: now_ms(),
        updated_ms: now_ms(),
        stop_reason: None,
        report: None,
        continuation: None,
    };
    service
        .store
        .lock()
        .unwrap()
        .start_run(run.clone())
        .unwrap();
    let handle = service
        .grant(scope, "c".into(), f.root.to_string_lossy().into())
        .unwrap();
    let proposal = dunlin_coordination::service::request(
        service.socket(),
        &handle.token,
        "continuations_create",
        json!({"prompt":"Approved task","dependencies":[],"idempotency_key":"once"}),
    )
    .unwrap();
    service
        .store
        .lock()
        .unwrap()
        .finish_run(&run.id, RunStatus::TurnFinished, "end_turn", None, None)
        .unwrap();
    let (close, _) = tokio::sync::mpsc::channel(1);
    f.state.leases.lock().unwrap().insert(
        "c".into(),
        crate::api::Lease {
            device_id: device.id.clone(),
            connection_id: "original".into(),
            close_tx: close,
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
        },
    );
    let body = json!({"project":f.project,"operation":"approve","session":"c","id":proposal["id"],"expected":proposal});
    let mut changed = body.clone();
    changed["expected"]["prompt"] = json!("Different prompt");
    assert_eq!(
        f.post("/v1/coordination", Some(&f.token), changed).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.post("/v1/coordination", Some(&second_token), body.clone())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.post("/v1/coordination", Some(&f.token), body).await.0,
        StatusCode::OK
    );
    {
        let db = service.store.lock().unwrap();
        assert_eq!(
            db.ledger().continuations[0].executor_device.as_deref(),
            Some(device.id.as_str())
        );
        assert_eq!(
            db.ledger().continuations[0].executor_connection.as_deref(),
            Some("original")
        );
    }
    f.state
        .leases
        .lock()
        .unwrap()
        .get_mut("c")
        .unwrap()
        .connection_id = "reconnected".into();
    let (status, snapshot) = f
        .post(
            "/v1/coordination",
            Some(&f.token),
            json!({"project":f.project,"operation":"snapshot"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        snapshot["ledger"]["continuations"][0]["status"],
        "needs_attention"
    );
}
