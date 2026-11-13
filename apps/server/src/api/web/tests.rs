use super::*;
use crate::{config::Config, store::Store};
use axum::body::{Body, to_bytes};
use axum::http::Request;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};
use tower::ServiceExt;

mod admission;
mod agents;
mod ahp_browser;
mod ahp_controls_browser;
mod ahp_create_browser;
mod ahp_dispatch_browser;
mod ahp_history_browser;
mod ahp_permissions_browser;
mod ahp_sharing_browser;
mod binding_metadata;
mod browser;
mod host_health;
mod hosts;
mod hosts_browser;
mod live_transport;
mod local_bindings_browser;
mod mcp_oauth;
mod mcp_oauth_browser;
mod mcp_oauth_flow;
mod remote_bindings;
mod runtime_restart;
mod scoped_secrets;
mod sharing;

fn fixture() -> (AppState, String, String) {
    let store = Store::open_in_memory().unwrap();
    let user = store.create_user("member", "member").unwrap();
    let project = store.create_project("visible", "/fixture/project").unwrap();
    store.grant(&project.id, &user.id).unwrap();
    let (code, _) = store.create_pairing_code(&user.id, 900).unwrap();
    let state = AppState::new(
        store,
        Config {
            db: "/nonexistent/dunlin-web-fixture/relay.db".into(),
            ..Config::default()
        },
    );
    (state, code, project.id)
}

fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, host.parse().unwrap());
    if let Some(origin) = origin {
        headers.insert(header::ORIGIN, origin.parse().unwrap());
    }
    headers
}

async fn browser_pair(state: &AppState, code: &str) -> Response {
    super::super::router(state.clone())
        .oneshot(
            Request::post("/v1/web/pair")
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://build.tail123.ts.net")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"code":code,"device_name":"fixture browser"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[test]
fn origin_accepts_only_same_origin_tailscale_or_loopback() {
    for (host, origin, accepted) in [
        ("build.tail123.ts.net", "https://build.tail123.ts.net", true),
        ("build.tail123.ts.net", "http://build.tail123.ts.net", false),
        ("build.example.com", "https://build.example.com", false),
        ("192.168.1.8:8787", "http://192.168.1.8:8787", false),
        ("127.0.0.1:8787", "http://127.0.0.1:8787", true),
        ("localhost:8787", "http://localhost:8787", true),
        ("[::1]:8787", "http://[::1]:8787", true),
        ("127.0.0.1:8787", "http://localhost:8787", false),
        ("build.tail123.ts.net", "https://evil.example", false),
        (
            "build.tail123.ts.net",
            "https://build.tail123.ts.net.evil.test",
            false,
        ),
        ("build.tail123.ts.net", "null", false),
        (
            "build.tail123.ts.net",
            "https://build.tail123.ts.net/path",
            false,
        ),
    ] {
        assert_eq!(
            request_origin(&headers(host, Some(origin)), true).is_ok(),
            accepted,
            "{host} / {origin}"
        );
    }
    assert!(request_origin(&headers("localhost:8787", None), true).is_err());
}

#[tokio::test]
async fn pairing_sets_private_cookie_without_returning_a_token() {
    let (state, code, _) = fixture();
    let response = browser_pair(&state, &code).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    for expected in [
        "HttpOnly",
        "SameSite=Strict",
        "Secure",
        "Path=/v1/web",
        "Max-Age=28800",
    ] {
        assert!(cookie.contains(expected));
    }
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert!(body.get("token").is_none());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        cookie.split(';').next().unwrap().parse().unwrap(),
    );
    let token = cookie_token(&headers).unwrap();
    assert!(authorized(&state, &token).is_ok());
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .authenticate(&token)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        browser_pair(&state, &code).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn browser_cookie_survives_server_restart_but_never_authenticates_native_api() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let config = Config {
        db: path.clone(),
        ..Config::default()
    };
    let store = Store::open(&path).unwrap();
    let user = store.create_user("restart-member", "member").unwrap();
    let code = store.create_pairing_code(&user.id, 900).unwrap().0;
    let state = AppState::new(store, config.clone());
    let response = browser_pair(&state, &code).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    drop(state);
    let state = AppState::new(Store::open(&path).unwrap(), config);
    let router = super::super::router(state.clone());
    let request = |path| {
        Request::get(path)
            .header("host", "build.tail123.ts.net")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap()
    };
    let response = router
        .clone()
        .oneshot(request("/v1/web/session"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let current: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(current["user"]["id"], user.id);
    assert_eq!(
        router
            .clone()
            .oneshot(request("/v1/me"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut request_headers = headers("build.tail123.ts.net", Some("https://build.tail123.ts.net"));
    request_headers.insert(header::COOKIE, cookie.parse().unwrap());
    logout(State(state), request_headers).await.unwrap();
    assert_eq!(
        router
            .oneshot(request("/v1/web/session"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn cross_origin_pairing_does_not_consume_code() {
    let (state, code, _) = fixture();
    let response = super::super::router(state.clone())
        .oneshot(
            Request::post("/v1/web/pair")
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"code":code,"device_name":"fixture"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(browser_pair(&state, &code).await.status(), StatusCode::OK);
}

#[test]
fn expiry_and_revocation_invalidate_existing_browser_authority() {
    let (state, _, _) = fixture();
    let device = {
        let store = state.store.lock().unwrap();
        let user = store.find_user_by_name("member").unwrap().unwrap();
        store.create_device(&user.id, "browser").unwrap().0
    };
    let token = register(&state, &device).unwrap();
    assert!(authorized(&state, &token).is_ok());
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert!(authorized(&state, &token).is_err());
    state
        .store
        .lock()
        .unwrap()
        .expire_browser_session(&token)
        .unwrap();
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .browser_session_device(&token)
            .unwrap()
            .is_none()
    );
}

#[test]
fn catalog_never_inherits_another_users_sessions_or_ungranted_project() {
    let (state, _, project) = fixture();
    let device = {
        let store = state.store.lock().unwrap();
        let member = store.find_user_by_name("member").unwrap().unwrap();
        let other = store.create_user("other", "member").unwrap();
        let hidden = store.create_project("hidden", "/fixture/hidden").unwrap();
        store.grant(&project, &other.id).unwrap();
        store.grant(&hidden.id, &other.id).unwrap();
        let device = store.create_device(&member.id, "browser").unwrap().0;
        store
            .create_session("mine", &member.id, &project, "my session", &device.id)
            .unwrap();
        store
            .create_session(
                "theirs",
                &other.id,
                &project,
                "private session",
                "other-device",
            )
            .unwrap();
        device
    };
    let token = register(&state, &device).unwrap();
    let auth = authorized(&state, &token).unwrap();
    let catalog = snapshot(&state, &auth, &Some(project), "fixture", 1).unwrap();
    assert_eq!(catalog.projects.len(), 1);
    assert_eq!(catalog.sessions.len(), 1);
    assert_eq!(catalog.sessions[0].id, "mine");
    assert!(snapshot(&state, &auth, &Some("unknown-project".into()), "fixture", 2).is_err());
}

#[test]
fn committed_catalog_writes_emit_payload_free_invalidation() {
    let (state, _, project) = fixture();
    let mut changes = state.store.lock().unwrap().subscribe_catalog();
    let store = state.store.lock().unwrap();
    let user = store.find_user_by_name("member").unwrap().unwrap();
    store
        .create_session("s", &user.id, &project, "New session", "device")
        .unwrap();
    assert!(changes.try_recv().is_ok());
    store.rename_session_if_new("s", "updated").unwrap();
    assert!(changes.try_recv().is_ok());
    assert_eq!(
        store.list_sessions(&user.id, &project).unwrap()[0].title,
        "updated"
    );
    store.delete_session("s").unwrap();
    assert!(changes.try_recv().is_ok());
}

#[test]
fn duplicate_session_cookies_are_rejected() {
    let token = crate::auth::random_token();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("{COOKIE}={token}; {COOKIE}={token}")
            .parse()
            .unwrap(),
    );
    assert!(cookie_token(&headers).is_err());
}

#[test]
fn static_assets_never_expose_sources_secrets_or_api_fallbacks() {
    for path in ["", "index.html", "REUI-LICENSE.txt", "assets/index-abc.js"] {
        assert!(asset_path(path).is_some(), "{path}");
    }
    for path in [
        ".env",
        "package.json",
        "src/App.tsx",
        "v1/missing",
        "assets/../secret",
        "assets/%2e%2e/secret",
        "assets/.secret",
        "assets//secret",
        "assets\\secret",
    ] {
        assert!(asset_path(path).is_none(), "{path}");
    }
}

#[tokio::test]
async fn logout_revokes_device_and_browser_session() {
    let (state, code, _) = fixture();
    let response = browser_pair(&state, &code).await;
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let mut request_headers = headers("build.tail123.ts.net", Some("https://build.tail123.ts.net"));
    request_headers.insert(header::COOKIE, cookie.parse().unwrap());
    let token = cookie_token(&request_headers).unwrap();
    let device = authorized(&state, &token).unwrap().device;
    let response = logout(State(state.clone()), request_headers).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert!(authorized(&state, &token).is_err());
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .active_device_user(&device.id)
            .unwrap()
            .is_none()
    );
}

async fn team_request(
    state: &AppState,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = super::super::router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://build.tail123.ts.net")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn paired_cookie(state: &AppState, user: &str) -> String {
    let code = state
        .store
        .lock()
        .unwrap()
        .create_pairing_code(user, 900)
        .unwrap()
        .0;
    browser_pair(state, &code).await.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn team_api_uses_cookie_authority_and_scope_instead_of_request_actor() {
    let (state, _, _) = fixture();
    let (owner, member) = {
        let store = state.store.lock().unwrap();
        (
            store.create_user("owner", "admin").unwrap(),
            store.find_user_by_name("member").unwrap().unwrap(),
        )
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let member_cookie = paired_cookie(&state, &member.id).await;
    assert_eq!(
        team_request(
            &state,
            "POST",
            "/v1/web/workspaces",
            &member_cookie,
            json!({"name":"forbidden"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, workspace) = team_request(
        &state,
        "POST",
        "/v1/web/workspaces",
        &owner_cookie,
        json!({"name":"fixture team"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workspace["membership"]["role"], "owner");
    let path = format!(
        "/v1/web/workspaces/{}/members",
        workspace["id"].as_str().unwrap()
    );
    let request =
        json!({"user_id":member.id,"role":"reviewer","active":true,"expected_revision":null});
    assert_eq!(
        team_request(&state, "PUT", &path, &owner_cookie, request.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        team_request(&state, "PUT", &path, &owner_cookie, request.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(&state, "PUT", &path, &member_cookie, request)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(
            &state,
            "GET",
            "/v1/web/workspaces/unknown/members",
            &member_cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, list) = team_request(
        &state,
        "GET",
        "/v1/web/workspaces",
        &member_cookie,
        Value::Null,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["membership"]["role"], "reviewer");
}

#[tokio::test]
async fn team_api_rejects_request_supplied_actor_and_cross_origin_mutations() {
    let (state, _, _) = fixture();
    let owner = state
        .store
        .lock()
        .unwrap()
        .create_user("owner", "admin")
        .unwrap();
    let cookie = paired_cookie(&state, &owner.id).await;
    let response = super::super::router(state.clone())
        .oneshot(
            Request::post("/v1/web/workspaces")
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://evil.example")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from("{\"name\":\"forbidden\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .workspaces_for_user(&owner.id)
            .unwrap()
            .is_empty()
    );
    let response = super::super::router(state.clone())
        .oneshot(
            Request::post("/v1/web/workspaces")
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://build.tail123.ts.net")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":"forbidden","actor_id":owner.id}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn team_project_api_requires_explicit_grants_and_keeps_host_authority_separate() {
    let (state, _, _) = fixture();
    let (owner, viewer, workspace) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let viewer = store.create_user("viewer", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &viewer.id,
                dunlin_domain::collaboration::WorkspaceRole::Viewer,
                true,
                None,
            )
            .unwrap();
        (owner, viewer, workspace)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let viewer_cookie = paired_cookie(&state, &viewer.id).await;
    let projects_path = format!("/v1/web/workspaces/{}/projects", workspace.id);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &projects_path,
            &viewer_cookie,
            json!({"name":"forbidden"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, project) = team_request(
        &state,
        "POST",
        &projects_path,
        &owner_cookie,
        json!({"name":"Logical project"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        team_request(&state, "GET", &projects_path, &viewer_cookie, Value::Null)
            .await
            .1,
        json!([])
    );
    let grants_path = format!("{projects_path}/{}/grants", project["id"].as_str().unwrap());
    let request = json!({"user_id":viewer.id,"permissions":["view_project", "run_workflow"],"expected_revision":null});
    assert_eq!(
        team_request(&state, "PUT", &grants_path, &viewer_cookie, request.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        team_request(&state, "PUT", &grants_path, &owner_cookie, request.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        team_request(&state, "PUT", &grants_path, &owner_cookie, request)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, visible) =
        team_request(&state, "GET", &projects_path, &viewer_cookie, Value::Null).await;
    assert_eq!(visible[0]["name"], "Logical project");
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .projects_for_user(&viewer.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        team_request(
            &state,
            "PUT",
            &grants_path,
            &owner_cookie,
            json!({"user_id":viewer.id,"permissions":["manage_hosts"],"expected_revision":1})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &workspace.id,
            &viewer.id,
            dunlin_domain::collaboration::WorkspaceRole::Viewer,
            false,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &projects_path, &viewer_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn workflow_api_separates_draft_authority_publication_and_stale_writes() {
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};
    let (state, _, _) = fixture();
    let (owner, editor, workspace, project) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let editor = store.create_user("editor", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "project")
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &editor.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &editor.id,
                [
                    Permission::ViewProject,
                    Permission::EditWorkflow,
                    Permission::Publish,
                ]
                .into(),
                None,
            )
            .unwrap();
        (owner, editor, workspace, project)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let editor_cookie = paired_cookie(&state, &editor.id).await;
    let definition: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/workflows/basic.json"
    ))
    .unwrap();
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/workflows",
        workspace.id, project.id
    );
    let (status, created) =
        team_request(&state, "POST", &path, &editor_cookie, definition.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let path = format!("{path}/{}", created["id"].as_str().unwrap());
    let publish_path = format!("{path}/publish");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish_path,
            &editor_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, published) = team_request(
        &state,
        "POST",
        &publish_path,
        &owner_cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(published["version"], 1);
    assert_eq!(published["digest"].as_str().unwrap().len(), 64);
    let request = json!({"expected_revision":1,"definition":definition});
    let (left, right) = tokio::join!(
        team_request(&state, "PUT", &path, &editor_cookie, request.clone()),
        team_request(&state, "PUT", &path, &editor_cookie, request),
    );
    assert!([left.0, right.0].contains(&StatusCode::OK));
    assert!([left.0, right.0].contains(&StatusCode::CONFLICT));
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish_path,
            &owner_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(
            &state,
            "GET",
            &format!("{path}/versions/1"),
            &editor_cookie,
            Value::Null
        )
        .await
        .1,
        published
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_project_grant(
            &owner.id,
            &workspace.id,
            &project.id,
            &editor.id,
            Default::default(),
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &path, &editor_cookie, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn invitation_http_requires_recipient_cookie_and_consumes_token_once() {
    let (state, _, _) = fixture();
    let (owner, member, workspace) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("invite-owner", "admin").unwrap();
        let member = store.find_user_by_name("member").unwrap().unwrap();
        let workspace = store.create_workspace(&owner, "invitations").unwrap();
        (owner, member, workspace)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let member_cookie = paired_cookie(&state, &member.id).await;
    let path = format!("/v1/web/workspaces/{}/invitations", workspace.id);
    let request = json!({"user_id":member.id,"role":"reviewer"});
    assert_eq!(
        team_request(&state, "POST", &path, &member_cookie, request.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, invitation) = team_request(&state, "POST", &path, &owner_cookie, request).await;
    assert_eq!(status, StatusCode::OK);
    let accept = "/v1/web/invitations/accept";
    let request = json!({"token":invitation["token"]});
    assert_eq!(
        team_request(&state, "POST", accept, &owner_cookie, request.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, member) =
        team_request(&state, "POST", accept, &member_cookie, request.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(member["role"], "reviewer");
    assert_eq!(
        team_request(&state, "POST", accept, &member_cookie, request)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn automation_credentials_read_only_published_versions_and_revoke_immediately() {
    let (state, _, _) = fixture();
    let (owner, workspace, project, version) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("service-owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "automation").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "logical")
            .unwrap();
        let definition = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/workflows/basic.json"
        ))
        .unwrap();
        let draft = store
            .create_workflow(&owner.id, &w.id, &p.id, definition)
            .unwrap();
        let version = store
            .publish_workflow(&owner.id, &w.id, &p.id, &draft.id, 1)
            .unwrap();
        (owner, w, p, version)
    };
    let cookie = paired_cookie(&state, &owner.id).await;
    let path = format!("/v1/web/workspaces/{}/service-principals", workspace.id);
    let (status,created)=team_request(&state,"POST",&path,&cookie,json!({"project_id":project.id,"name":"reader","permissions":["view_project"],"expires_in":600})).await;
    assert_eq!(status, StatusCode::OK);
    let token = created["token"].as_str().unwrap();
    let version_path = format!(
        "/v1/service/workspaces/{}/projects/{}/workflows/{}/versions/1",
        workspace.id, project.id, version.workflow_id
    );
    for (path, credential, expected) in [
        (version_path.as_str(), Some(token), StatusCode::OK),
        (version_path.as_str(), None, StatusCode::UNAUTHORIZED),
        ("/v1/web/session", Some(token), StatusCode::UNAUTHORIZED),
        ("/v1/projects", Some(token), StatusCode::UNAUTHORIZED),
    ] {
        let mut request = Request::get(path).header("host", "build.tail123.ts.net");
        if let Some(token) = credential {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = super::super::router(state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path}");
    }
    let revoke = format!(
        "{path}/{}/revoke",
        created["principal"]["id"].as_str().unwrap()
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &revoke,
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let response = super::super::router(state.clone())
        .oneshot(
            Request::get(&version_path)
                .header("host", "build.tail123.ts.net")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn local_binding_api_is_explicit_revision_checked_and_does_not_expose_host_paths() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _, _) = fixture();
    let (owner, workspace, project, native) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("binding-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "bindings").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "logical")
            .unwrap();
        let native = store
            .create_project("checkout", dir.path().to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &owner.id).unwrap();
        (owner, workspace, project, native)
    };
    let cookie = paired_cookie(&state, &owner.id).await;
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/binding",
        workspace.id, project.id
    );
    let (status, empty) = team_request(&state, "GET", &path, &cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["binding"], Value::Null);
    assert_eq!(empty["configuration_current"], false);
    let body = json!({"native_project_id":native.id,"expected_revision":null});
    let (status, binding) = team_request(&state, "PUT", &path, &cookie, body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(binding["host_id"], "local");
    assert_eq!(binding["revision"], 1);
    assert!(binding.get("root").is_none());
    assert!(binding.get("identity").is_none());
    assert_eq!(
        team_request(&state, "PUT", &path, &cookie, body).await.0,
        StatusCode::CONFLICT
    );
    let (status, current) = team_request(&state, "GET", &path, &cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["configuration_current"], true);
    let revoke = format!("{path}/revoke");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &revoke,
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, revoked) = team_request(&state, "GET", &path, &cookie, Value::Null).await;
    assert_eq!(revoked["configuration_current"], false);
    assert_eq!(revoked["binding"]["revision"], 2);
}
