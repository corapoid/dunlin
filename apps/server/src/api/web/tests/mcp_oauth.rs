use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use dunlin_mcp_client::oauth::{
    AuthContext,
    persistence::{CredentialJournal, CredentialRecord},
};
const BINDING: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
fn connection(state: &AppState) -> (AuthContext, CredentialRecord, String) {
    let store = state.store.lock().unwrap();
    let owner = store.create_user("oauth-owner", "admin").unwrap();
    let other = store.create_user("oauth-other-admin", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "OAuth").unwrap();
    store
        .set_team_member(
            &owner.id,
            &workspace.id,
            &other.id,
            WorkspaceRole::Admin,
            true,
            None,
        )
        .unwrap();
    let context = AuthContext {
        user_id: owner.id,
        workspace_id: workspace.id,
        connection_id: uuid::Uuid::now_v7().to_string(),
        connection_revision: 1,
    };
    let intent = store.begin_install(&context, BINDING).unwrap();
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store.commit(&context, BINDING, &intent, DIGEST).unwrap();
    let record = store.active(&context, BINDING).unwrap();
    (context, record, other.id)
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    cookie: &str,
    origin: Option<&str>,
    body: Value,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "build.tail123.ts.net")
        .header("cookie", cookie)
        .header("content-type", "application/json");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    super::super::super::router(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
const ORIGIN: Option<&str> = Some("https://build.tail123.ts.net");
#[tokio::test]
async fn mcp_oauth_browser_status_disconnect_owner_cas_and_no_secret_projection() {
    let (state, _, _) = fixture();
    let (context, record, other) = connection(&state);
    let cookie = paired_cookie(&state, &context.user_id).await;
    let other_cookie = paired_cookie(&state, &other).await;
    let base = format!(
        "/v1/web/workspaces/{}/mcp-connections/{}",
        context.workspace_id, context.connection_id
    );
    let response = request(&state, "GET", &base, &cookie, ORIGIN, Value::Null).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let encoded = std::str::from_utf8(&bytes).unwrap();
    for private in [BINDING, DIGEST, record.secret_reference.as_str()] {
        assert!(!encoded.contains(private));
    }
    let metadata: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        metadata,
        json!({"id":context.connection_id,"revision":1,"status":"connected","storage":{"pending_references":1,"retained_history":0,"uncertain_history":0,"available_slots":255,"cleanup_available":false}})
    );
    assert_eq!(
        team_request(&state, "GET", &base, &other_cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let disconnect = format!("{base}/disconnect");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &disconnect,
            &other_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &disconnect,
            &cookie,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let pending = state
        .store
        .lock()
        .unwrap()
        .begin_refresh(&context, BINDING, &record)
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .1["status"],
        "reauthorization_required"
    );
    let (status, body) = team_request(
        &state,
        "POST",
        &disconnect,
        &cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"connection":{"id":context.connection_id,"revision":2,"status":"disabled","storage":{"pending_references":2,"retained_history":0,"uncertain_history":0,"available_slots":254,"cleanup_available":false}},"provider_revocation":"not_confirmed"})
    );
    let audit = state
        .store
        .lock()
        .unwrap()
        .team_audit(&context.user_id, &context.workspace_id)
        .unwrap();
    let audit_count = audit
        .iter()
        .filter(|event| event.action == "mcp_oauth_disconnected")
        .count();
    assert_eq!(audit_count, 1);
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .commit(&context, BINDING, &pending, DIGEST)
            .is_err()
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &disconnect,
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
            &disconnect,
            &cookie,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .team_audit(&context.user_id, &context.workspace_id)
            .unwrap()
            .iter()
            .filter(|event| event.action == "mcp_oauth_disconnected")
            .count(),
        audit_count
    );
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .1["status"],
        "disabled"
    );
}
#[tokio::test]
async fn mcp_oauth_browser_rejects_csrf_forged_actor_revoked_cookie_and_removed_membership() {
    let (state, _, _) = fixture();
    let (context, record, _) = connection(&state);
    let cookie = paired_cookie(&state, &context.user_id).await;
    let base = format!(
        "/v1/web/workspaces/{}/mcp-connections/{}",
        context.workspace_id, context.connection_id
    );
    let disconnect = format!("{base}/disconnect");
    let payload = json!({"expected_revision":1});
    assert_eq!(
        request(&state, "GET", &base, "", ORIGIN, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&state, "POST", &disconnect, &cookie, None, payload.clone())
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &state,
            "POST",
            &disconnect,
            &cookie,
            Some("https://attacker.example"),
            payload.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &state,
            "POST",
            &disconnect,
            &cookie,
            ORIGIN,
            json!({"expected_revision":1,"user_id":context.user_id})
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let wrong = format!(
        "/v1/web/workspaces/other-workspace/mcp-connections/{}",
        context.connection_id
    );
    assert_eq!(
        team_request(&state, "GET", &wrong, &cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .active(&context, BINDING)
            .unwrap(),
        record
    );
    let token = cookie.split_once('=').unwrap().1;
    let device = state
        .store
        .lock()
        .unwrap()
        .browser_session_device(token)
        .unwrap()
        .unwrap();
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
    assert_eq!(
        team_request(&state, "POST", &disconnect, &cookie, payload.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let cookie = paired_cookie(&state, &context.user_id).await;
    let other_owner = {
        let store = state.store.lock().unwrap();
        let other = store.create_user("oauth-workspace-owner", "admin").unwrap();
        store
            .set_team_member(
                &context.user_id,
                &context.workspace_id,
                &other.id,
                WorkspaceRole::Owner,
                true,
                None,
            )
            .unwrap();
        other.id
    };
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &other_owner,
            &context.workspace_id,
            &context.user_id,
            WorkspaceRole::Member,
            false,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        team_request(&state, "POST", &disconnect, &cookie, payload)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
