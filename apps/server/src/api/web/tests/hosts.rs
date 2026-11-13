use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
fn definition() -> Value {
    json!({"name":"Fixture worker","node_id":"node-fixture","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":2,"harnesses":["claude"]}})
}
fn capabilities() -> Value {
    json!({"protocol_version":1,"version":"fixture-v1","os":"linux","architecture":"aarch64","harnesses":["claude","codex"],"sandbox":"linux_bubblewrap","max_parallel":8})
}
async fn worker(
    state: &AppState,
    path: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::post(path)
        .header("host", "build.tail123.ts.net")
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = crate::api::router(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn host_registry_and_worker_channels_preserve_distinct_authority_cas_and_epoch_fences() {
    let (state, _, _) = fixture();
    let (owner, member, w) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("host-owner", "admin").unwrap();
        let member = store.create_user("host-member", "member").unwrap();
        let w = store.create_workspace(&owner, "Workers").unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &member.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        (owner, member, w)
    };
    let cookie = paired_cookie(&state, &owner.id).await;
    let member_cookie = paired_cookie(&state, &member.id).await;
    let base = format!("/v1/web/workspaces/{}/hosts", w.id);
    assert_eq!(
        team_request(&state, "GET", &base, &member_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(&state, "POST", &base, &member_cookie, definition())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, created) = team_request(&state, "POST", &base, &cookie, definition()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["host"]["id"].as_str().unwrap();
    let path = format!("{base}/{id}");
    let ticket = created["enrollment"]["ticket"].as_str().unwrap();
    assert_eq!(created["host"]["status"], "pending");
    assert_eq!(created["host"]["network_verification"], "unverified");
    assert_eq!(created["host"]["execution_ready"], false);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/approve"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        worker(
            &state,
            "/v1/worker/enroll",
            &[("cookie", &cookie)],
            json!({"ticket":ticket,"capabilities":capabilities()})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "A browser cookie cannot act as worker identity or consume the ticket"
    );
    let (status, credential) = worker(
        &state,
        "/v1/worker/enroll",
        &[],
        json!({"ticket":ticket,"capabilities":capabilities()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credential}");
    let token = credential["credential"].as_str().unwrap();
    let bearer = format!("Bearer {token}");
    assert!(token.starts_with("dhw_"));
    assert_eq!(
        worker(
            &state,
            "/v1/worker/enroll",
            &[],
            json!({"ticket":ticket,"capabilities":capabilities()})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let report = json!({"host_id":id,"enrollment_epoch":1,"credential_revision":1,"capabilities":capabilities()});
    let native = state
        .store
        .lock()
        .unwrap()
        .create_device(&owner.id, "native fixture")
        .unwrap()
        .1;
    for wrong_identity in [ticket, native.as_str()] {
        let wrong_bearer = format!("Bearer {wrong_identity}");
        assert_eq!(
            worker(
                &state,
                "/v1/worker/report",
                &[("authorization", &wrong_bearer)],
                report.clone()
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED,
            "Enrollment tickets and native device tokens cannot report as workers"
        );
    }
    assert_eq!(
        worker(
            &state,
            "/v1/worker/report",
            &[("authorization", ticket)],
            report.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        worker(
            &state,
            "/v1/worker/report",
            &[("authorization", &bearer)],
            report.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, approved) = team_request(
        &state,
        "POST",
        &format!("{path}/approve"),
        &cookie,
        json!({"expected_revision":2}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Reports do not invalidate control CAS: {approved}"
    );
    assert_eq!(approved["status"], "approved");
    assert_eq!(approved["execution_ready"], false);
    let fake_cookie = format!("dunlin_web_session={token}");
    assert_eq!(
        team_request(&state, "GET", &base, &fake_cookie, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let response = crate::api::router(state.clone())
        .oneshot(
            Request::get("/v1/projects")
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Worker credentials are not native device credentials"
    );
    let listed = team_request(&state, "GET", &base, &cookie, Value::Null).await;
    let public = listed.1.to_string();
    assert!(!public.contains(ticket));
    assert!(!public.contains(token));
    assert!(!public.contains("credential_hash"));
    assert!(!public.contains("ticket_hash"));
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &w.id,
            &member.id,
            WorkspaceRole::Admin,
            true,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &base, &member_cookie, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let (status, revoked) = team_request(
        &state,
        "POST",
        &format!("{path}/revoke"),
        &member_cookie,
        json!({"expected_revision":3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["status"], "revoked");
    let receipt = team_request(
        &state,
        "GET",
        &format!("{path}/revocation"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(receipt.0, StatusCode::OK);
    assert_eq!(receipt.1["revocation"]["host_id"], id);
    assert_eq!(receipt.1["revocation"]["actor_id"], member.id);
    assert_eq!(receipt.1["revocation"]["revision"], 4);
    assert_eq!(
        worker(
            &state,
            "/v1/worker/report",
            &[("authorization", &bearer)],
            report
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/enrollment"),
            &cookie,
            json!({"expected_revision":3})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, rotated) = team_request(
        &state,
        "POST",
        &format!("{path}/enrollment"),
        &cookie,
        json!({"expected_revision":4}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rotated["host"]["enrollment_epoch"], 3);
    assert_ne!(rotated["enrollment"]["ticket"], ticket);
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &w.id,
            &member.id,
            WorkspaceRole::Member,
            true,
            Some(2),
        )
        .unwrap();
    assert_eq!(
        team_request(
            &state,
            "GET",
            &format!("{path}/revocation"),
            &member_cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(&state, "GET", &path, &member_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/drain"),
            &member_cookie,
            json!({"expected_revision":5,"drain":false})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let audit = state
        .store
        .lock()
        .unwrap()
        .team_audit(&owner.id, &w.id)
        .unwrap();
    for event in audit.iter().filter(|e| e.action.starts_with("host_")) {
        assert_eq!(event.resource_id.as_deref(), Some(id));
        assert_eq!(event.project_id, None);
    }
    let raw = serde_json::to_string(&audit).unwrap();
    assert!(!raw.contains(ticket));
    assert!(!raw.contains(token));
}
#[tokio::test]
async fn host_registry_cross_origin_and_revoked_cookie_never_create_or_disclose_enrollment() {
    let (state, _, _) = fixture();
    let (owner, w) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("host-owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "Workers").unwrap();
        (owner, w)
    };
    let cookie = paired_cookie(&state, &owner.id).await;
    let path = format!("/v1/web/workspaces/{}/hosts", w.id);
    let response = crate::api::router(state.clone())
        .oneshot(
            Request::post(&path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://attacker.invalid")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(definition().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let mut invalid = definition();
    invalid["serve_origin"] = json!("https://worker.tail123.ts.net/");
    assert_eq!(
        team_request(&state, "POST", &path, &cookie, invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .hosts(&owner.id, &w.id)
            .unwrap()
            .is_empty()
    );
    let device = state
        .store
        .lock()
        .unwrap()
        .list_devices(&owner.id)
        .unwrap()
        .remove(0);
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &path, &cookie, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        team_request(&state, "POST", &path, &cookie, definition())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .hosts(&owner.id, &w.id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn persisted_worker_channel_diagnostics_are_manager_scoped_and_never_claim_a_live_lease() {
    let (state, _, _) = fixture();
    let (owner, viewer, workspace, host) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("channel-api-owner", "admin").unwrap();
        let viewer = store.create_user("channel-api-viewer", "member").unwrap();
        let workspace = store.create_workspace(&owner, "Channel API").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &viewer.id,
                WorkspaceRole::Viewer,
                true,
                None,
            )
            .unwrap();
        let pending = store
            .create_host(
                &owner.id,
                &workspace.id,
                serde_json::from_value(definition()).unwrap(),
            )
            .unwrap();
        let credential = store
            .enroll_host(
                &pending.enrollment.ticket,
                serde_json::from_value(capabilities()).unwrap(),
            )
            .unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &credential.host_id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker_token(&credential.credential)
            .unwrap();
        store
            .open_worker_channel(
                &identity,
                "previous-controller-boot",
                "previous-worker-boot",
                "channel",
                1000,
                10000,
            )
            .unwrap();
        (owner, viewer, workspace, credential.host_id)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let viewer_cookie = paired_cookie(&state, &viewer.id).await;
    let path = format!("/v1/web/workspaces/{}/hosts/{host}/channel", workspace.id);
    let (status, value) = team_request(&state, "GET", &path, &owner_cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["reported_state_only"], true);
    assert_eq!(value["channel"]["phase"], "active");
    assert_eq!(
        value["channel"]["lease"]["controller_boot_id"],
        "previous-controller-boot"
    );
    assert_eq!(
        value["current_controller_boot_id"],
        state.worker_channels.boot_id
    );
    assert_ne!(
        value["channel"]["lease"]["controller_boot_id"],
        value["current_controller_boot_id"]
    );
    assert!(value.get("execution_ready").is_none());
    let health_path = format!("/v1/web/workspaces/{}/hosts/{host}/health", workspace.id);
    let (status, health) =
        team_request(&state, "GET", &health_path, &owner_cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["observational"], true);
    assert_eq!(health["host"]["id"], host);
    assert!(
        health["connection"].is_null(),
        "persisted rows cannot manufacture a live WSS observation"
    );
    assert_eq!(health["runtime"]["session_reservations"], 0);
    assert_eq!(health["runtime"]["one_shot_reservations"], 0);
    assert_eq!(
        team_request(&state, "GET", &health_path, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(&state, "GET", &path, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &workspace.id,
            &viewer.id,
            WorkspaceRole::Admin,
            true,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &path, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}
