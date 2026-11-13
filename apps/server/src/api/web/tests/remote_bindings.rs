use super::*;
use crate::worker_channel::ObservedExport;
use serde_json::json;
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
async fn setup() -> (
    AppState,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
) {
    let (mut state, _, _) = fixture();
    std::sync::Arc::make_mut(&mut state.config).worker_controller =
        Some(crate::worker_channel::ControllerConfig {
            execution_enabled: false,
            persistent_sessions: false,
            mcp_broker: false,
            socket: "/unused-resolver-fixture.sock".into(),
            node_id: "controller-node".into(),
            serve_origin: "https://controller.tail123.ts.net".into(),
        });
    let (owner, viewer, w, p, binding) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("remote-owner", "admin").unwrap();
        let viewer = store.create_user("remote-viewer", "member").unwrap();
        let w = store.create_workspace(&owner, "Remote").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "Project")
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &viewer.id,
                dunlin_domain::collaboration::WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &w.id,
                &p.id,
                &viewer.id,
                [dunlin_domain::collaboration::Permission::ViewProject].into(),
                None,
            )
            .unwrap();
        let host=store.create_host(&owner.id,&w.id,serde_json::from_value(json!({"name":"Remote worker","node_id":"worker-node","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["claude"]}})).unwrap()).unwrap();
        let credential=store.enroll_host(&host.enrollment.ticket,serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap()).unwrap();
        store
            .approve_host(&owner.id, &w.id, &host.host.id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker(&credential.credential, &host.host.id, 1, 1)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                &state.worker_channels.boot_id,
                "worker-boot",
                "remote-test-channel",
                clock(),
                15000,
            )
            .unwrap();
        let descriptor=serde_json::from_value(json!({"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node","workspace_id":w.id,"host_id":host.host.id,"enrollment_epoch":1,"credential_revision":1},"export":{"id":"export-fixture","name":"Export","repo_kind":"directory","checkout_identity_digest":"b".repeat(64),"settings_digest":"a".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":owner.id},"permissions":["read"],"revision":1,"export_revision":1,"active":true}})).unwrap();
        let proof = ObservedExport::for_test(identity.clone(), lease.clone(), descriptor, clock());
        let stamp = store
            .capture_remote_binding_authority(&owner.id, &w.id, &p.id)
            .unwrap();
        let binding = store
            .set_remote_binding(
                &owner.id,
                &w.id,
                &p.id,
                None,
                None,
                &host.host.id,
                "export-fixture",
                1,
                &proof,
                &state.worker_channels.boot_id,
                "controller-node",
                clock(),
                &stamp,
            )
            .unwrap();
        store.close_worker_channel(&identity, &lease).unwrap();
        (owner, viewer, w, p, binding)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let viewer_cookie = paired_cookie(&state, &viewer.id).await;
    (
        state,
        owner.id,
        viewer.id,
        w.id,
        p.id,
        binding.id,
        owner_cookie,
        viewer_cookie,
    )
}
#[tokio::test]
async fn remote_binding_http_reads_and_offline_revoke_require_current_management_and_preserve_tombstone()
 {
    let (state, owner, viewer, w, p, id, cookie, viewer_cookie) = setup().await;
    let base = format!("/v1/web/workspaces/{w}/projects/{p}/remote-bindings");
    let path = format!("{base}/{id}");
    assert_eq!(
        team_request(&state, "GET", &base, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, list) = team_request(&state, "GET", &base, &cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["observed_state_only"], true);
    assert_eq!(list["execution_ready"], false);
    assert_eq!(list["bindings"][0]["revision"], 1);
    assert_eq!(list["bindings"][0]["principal"]["id"], owner);
    let host = list["bindings"][0]["host_id"].as_str().unwrap();
    let body = json!({"host_id":host,"export_id":"export-fixture","expected_export_revision":1,"expected_revision":1});
    assert_eq!(
        team_request(&state, "PUT", &path, &cookie, body).await.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "A persisted channel row cannot replace a live proved metadata channel or trigger local fallback"
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/revoke"),
            &viewer_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, revoked) = team_request(
        &state,
        "POST",
        &format!("{path}/revoke"),
        &cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["active"], false);
    assert_eq!(revoked["revision"], 2);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{path}/revoke"),
            &cookie,
            json!({"expected_revision":2})
        )
        .await
        .1,
        revoked
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner,
            &w,
            &viewer,
            dunlin_domain::collaboration::WorkspaceRole::Admin,
            true,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &base, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN,
        "Workspace management still requires an explicit project management grant"
    );
    let device = state
        .store
        .lock()
        .unwrap()
        .list_devices(&owner)
        .unwrap()
        .remove(0);
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn browser_descriptors_paths_and_cross_origin_requests_cannot_supply_remote_binding_proof() {
    let (state, _, _, w, p, _, cookie, _) = setup().await;
    let base = format!("/v1/web/workspaces/{w}/projects/{p}/remote-bindings");
    for extra in ["descriptor", "root", "principal", "path"] {
        let mut body = json!({"host_id":"host-fixture","export_id":"export-fixture","expected_export_revision":1});
        body[extra] = json!("forged");
        let response = crate::api::router(state.clone())
            .oneshot(
                Request::post(&base)
                    .header("host", "build.tail123.ts.net")
                    .header("origin", "https://build.tail123.ts.net")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response=crate::api::router(state.clone()).oneshot(Request::post(&base).header("host","build.tail123.ts.net").header("origin","https://attacker.invalid").header("cookie",&cookie).header("content-type","application/json").body(Body::from(json!({"host_id":"host-fixture","export_id":"export-fixture","expected_export_revision":1}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(team_request(&state,"POST",&base,&cookie,json!({"host_id":"host-fixture","export_id":"export-fixture","expected_export_revision":0})).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .1["bindings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_device_revocation_committed_while_waiting_for_sql_cut_cannot_return_binding_metadata()
 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let owner = store.create_user("cut-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Remote").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Project")
        .unwrap();
    let state = AppState::new(
        store,
        Config {
            db: path.clone(),
            ..Default::default()
        },
    );
    let cookie = paired_cookie(&state, &owner.id).await;
    let device = state
        .store
        .lock()
        .unwrap()
        .list_devices(&owner.id)
        .unwrap()
        .remove(0)
        .id;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer_barrier = barrier.clone();
    let writer = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
        connection
            .execute(
                "UPDATE devices SET revoked_at=unixepoch() WHERE id=?1",
                [device],
            )
            .unwrap();
        writer_barrier.wait();
        std::thread::sleep(std::time::Duration::from_millis(150));
        connection.execute_batch("COMMIT;").unwrap();
    });
    barrier.wait();
    let response = team_request(
        &state,
        "GET",
        &format!(
            "/v1/web/workspaces/{}/projects/{}/remote-bindings",
            workspace.id, project.id
        ),
        &cookie,
        Value::Null,
    )
    .await;
    writer.join().unwrap();
    assert_eq!(
        response.0,
        StatusCode::UNAUTHORIZED,
        "Browser authority must be read after acquiring the SQL cut, not from a pre-lock WAL snapshot"
    );
}
#[tokio::test]
async fn remote_target_resolution_uses_actual_create_caller_and_never_administrator_or_local_fallback()
 {
    let (state, owner, viewer, w, p, id, cookie, viewer_cookie) = setup().await;
    let base = format!("/v1/web/workspaces/{w}/projects/{p}/remote-bindings");
    let resolve = format!("{base}/resolve");
    let body = json!({"binding":{"id":id,"revision":1}});
    assert_eq!(
        team_request(&state, "POST", &resolve, &viewer_cookie, body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN,
        "ViewProject alone cannot resolve a session target"
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_project_grant(
            &owner,
            &w,
            &p,
            &viewer,
            [
                dunlin_domain::collaboration::Permission::ViewProject,
                dunlin_domain::collaboration::Permission::CreateSession,
            ]
            .into(),
            Some(1),
        )
        .unwrap();
    let result = team_request(&state, "POST", &resolve, &viewer_cookie, body.clone()).await;
    assert_eq!(
        result.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "Create-only callers pass controller authority, but there is no current proved metadata channel"
    );
    assert!(!result.1.to_string().contains("checkout_identity_digest"));
    assert_eq!(
        team_request(
            &state,
            "POST",
            &resolve,
            &cookie,
            json!({"binding":{"id":id,"revision":2}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &resolve,
            &cookie,
            json!({"binding":{"id":"unknown-binding","revision":1}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &resolve,
            &cookie,
            json!({"binding":{"id":id,"revision":0}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let unrelated = state
        .store
        .lock()
        .unwrap()
        .create_user("unrelated-resolver-admin", "admin")
        .unwrap();
    let unrelated_cookie = paired_cookie(&state, &unrelated.id).await;
    assert_eq!(
        team_request(&state, "POST", &resolve, &unrelated_cookie, body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let unchanged = team_request(&state, "GET", &base, &cookie, Value::Null).await;
    assert_eq!(unchanged.1["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(unchanged.1["bindings"][0]["revision"], 1);
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .ahp_summaries(&owner)
            .unwrap()
            .is_empty(),
        "Resolution never creates a session"
    );
    let raw_token = viewer_cookie.split_once('=').unwrap().1;
    state
        .store
        .lock()
        .unwrap()
        .expire_browser_session(raw_token)
        .unwrap();
    assert_eq!(
        team_request(&state, "POST", &resolve, &viewer_cookie, body)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn resolver_rejects_ambiguous_paths_principals_and_cross_origin_bodies_before_remote_requests()
 {
    let (state, _, _, w, p, id, cookie, _) = setup().await;
    let path = format!("/v1/web/workspaces/{w}/projects/{p}/remote-bindings/resolve");
    for body in [
        json!({"binding":{"id":id,"revision":1},"principal":{"kind":"human","id":"admin"}}),
        json!({"binding":{"id":id,"revision":1,"path":"/private"}}),
        json!({"binding":{"kind":"local","revision":1}}),
    ] {
        let response = crate::api::router(state.clone())
            .oneshot(
                Request::post(&path)
                    .header("host", "build.tail123.ts.net")
                    .header("origin", "https://build.tail123.ts.net")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = crate::api::router(state.clone())
        .oneshot(
            Request::post(&path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://attacker.invalid")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"binding":{"id":id,"revision":1}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
