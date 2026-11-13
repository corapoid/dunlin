use super::*;
use dunlin_domain::collaboration::{Permission, WorkspaceRole};
use tokio::io::AsyncWriteExt;
const KEY: &str = "api-admission-fixture-0001";
fn options() -> Value {
    json!({"model":"fixture-model","mode":"plan","max_seconds":10,"max_output_bytes":1048576})
}
struct AdmissionFixture {
    dir: tempfile::TempDir,
    state: AppState,
    workspace: String,
    project: String,
    native: String,
    user: String,
    device: String,
    cookie: String,
}
impl AdmissionFixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let user = store.create_user("admission-owner", "admin").unwrap();
        let workspace = store.create_workspace(&user, "Admission").unwrap();
        let project = store
            .create_team_project(&user.id, &workspace.id, "Runtime")
            .unwrap();
        let native = store
            .create_project("Admission checkout", checkout.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &user.id).unwrap();
        store
            .set_local_binding(&user.id, &workspace.id, &project.id, &native.id, None)
            .unwrap();
        let code = store.create_pairing_code(&user.id, 900).unwrap().0;
        let mut state = AppState::new(
            store,
            Config {
                db: dir.path().join("relay.db"),
                sandbox: false,
                experimental_interactive_runtime: true,
                ..Default::default()
            },
        );
        configure_peer(&mut state);
        let (cookie, device) = pair(&state, &code).await;
        Self {
            dir,
            state,
            workspace: workspace.id,
            project: project.id,
            native: native.id,
            user: user.id,
            device,
            cookie,
        }
    }
    fn path(&self) -> String {
        format!(
            "/v1/web/workspaces/{}/projects/{}/interactive-sessions",
            self.workspace, self.project
        )
    }
    fn count(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "interactive_sessions" | "interactive_session_admissions"
        ));
        rusqlite::Connection::open(self.dir.path().join("relay.db"))
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
    async fn initialized(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while std::fs::read_to_string(self.dir.path().join("checkout/received-initializes"))
                .unwrap_or_default()
                .lines()
                .count()
                != count
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    fn starts(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("checkout/actor-starts"))
            .unwrap_or_default()
            .lines()
            .count()
    }
}
fn configure_peer(state: &mut AppState) {
    std::sync::Arc::get_mut(&mut state.interactive)
        .unwrap()
        .command = Some((
        "python3".into(),
        vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/interactive/tests/fake_acp.py"
            )
            .into(),
            "normal".into(),
        ],
    ));
}
async fn pair(state: &AppState, code: &str) -> (String, String) {
    let response = browser_pair(state, code).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    (cookie, body["device_id"].as_str().unwrap().into())
}
async fn post(
    state: &AppState,
    cookie: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::post(path)
        .header("host", "build.tail123.ts.net")
        .header("origin", "https://build.tail123.ts.net")
        .header("cookie", cookie)
        .header("content-type", "application/json");
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = crate::api::router(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}
async fn keyed(f: &AdmissionFixture, key: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
    post(
        &f.state,
        &f.cookie,
        &f.path(),
        &[("idempotency-key", key)],
        body,
    )
    .await
}

#[tokio::test]
async fn cookie_admission_concurrent_duplicates_conflicts_and_legacy_unkeyed_requests_are_distinct()
{
    let f = AdmissionFixture::new().await;
    let (first, second) = tokio::join!(keyed(&f, KEY, options()), keyed(&f, KEY, options()));
    for result in [&first, &second] {
        assert_eq!(result.0, StatusCode::ACCEPTED, "{}", result.2);
        assert_eq!(
            result.2["admission"]["session_is_acceptance_snapshot"],
            true
        );
        assert_eq!(
            result.1["idempotency-replayed"],
            if result.2["admission"]["replayed"] == true {
                "true"
            } else {
                "false"
            }
        );
    }
    assert_ne!(
        first.2["admission"]["replayed"],
        second.2["admission"]["replayed"]
    );
    assert_eq!(first.2["session"], second.2["session"]);
    f.initialized(1).await;
    assert_eq!(f.starts(), 1);
    assert_eq!(f.count("interactive_sessions"), 1);
    assert_eq!(f.count("interactive_session_admissions"), 1);
    let mut changed = options();
    changed["max_seconds"] = json!(11);
    assert_eq!(keyed(&f, KEY, changed).await.0, StatusCode::CONFLICT);
    let other = {
        let store = f.state.store.lock().unwrap();
        let other = store
            .create_team_project(&f.user, &f.workspace, "Other scope")
            .unwrap();
        store
            .set_local_binding(&f.user, &f.workspace, &other.id, &f.native, None)
            .unwrap();
        other.id
    };
    let path = format!(
        "/v1/web/workspaces/{}/projects/{other}/interactive-sessions",
        f.workspace
    );
    assert_eq!(
        post(
            &f.state,
            &f.cookie,
            &path,
            &[("idempotency-key", KEY)],
            options()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(f.count("interactive_sessions"), 1);
    let first = post(&f.state, &f.cookie, &f.path(), &[], options()).await;
    let second = post(&f.state, &f.cookie, &f.path(), &[], options()).await;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    assert_eq!(second.0, StatusCode::ACCEPTED);
    assert_ne!(first.2["session"]["id"], second.2["session"]["id"]);
    assert_eq!(first.1["idempotency-replayed"], "false");
    assert_eq!(second.1["idempotency-replayed"], "false");
    f.initialized(3).await;
    assert_eq!(f.starts(), 3);
    f.state.interactive.shutdown().await;
}

#[tokio::test]
async fn malformed_admission_headers_never_fall_back_to_unkeyed_creation() {
    let f = AdmissionFixture::new().await;
    for key in [
        "",
        "short",
        "invalid key with spaces",
        "invalid:key:with:punctuation",
        "niepoprawny-klucz-żółć",
    ] {
        assert_eq!(keyed(&f, key, options()).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        keyed(&f, &"x".repeat(129), options()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            &f.state,
            &f.cookie,
            &f.path(),
            &[
                ("idempotency-key", KEY),
                ("idempotency-key", "another-valid-key-0002")
            ],
            options()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for headers in [
        vec![("x-dunlin-expected-user", f.user.as_str())],
        vec![("x-dunlin-expected-device", f.device.as_str())],
        vec![
            ("x-dunlin-expected-user", f.user.as_str()),
            ("x-dunlin-expected-user", f.user.as_str()),
            ("x-dunlin-expected-device", f.device.as_str()),
        ],
    ] {
        assert_eq!(
            post(&f.state, &f.cookie, &f.path(), &headers, options())
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(f.count("interactive_sessions"), 0);
    assert_eq!(f.count("interactive_session_admissions"), 0);
    assert_eq!(f.starts(), 0);
}

#[tokio::test]
async fn changed_cookie_user_or_device_cannot_commit_an_intent_pinned_to_the_discovery_identity() {
    for different_user in [false, true] {
        let f = AdmissionFixture::new().await;
        let user = {
            let store = f.state.store.lock().unwrap();
            if different_user {
                let user = store
                    .create_user("second-authorized-owner", "admin")
                    .unwrap();
                store
                    .set_team_member(
                        &f.user,
                        &f.workspace,
                        &user.id,
                        WorkspaceRole::Member,
                        true,
                        None,
                    )
                    .unwrap();
                store
                    .set_team_project_grant(
                        &f.user,
                        &f.workspace,
                        &f.project,
                        &user.id,
                        [Permission::ViewProject, Permission::CreateSession]
                            .into_iter()
                            .collect(),
                        None,
                    )
                    .unwrap();
                store.grant(&f.native, &user.id).unwrap();
                user.id
            } else {
                f.user.clone()
            }
        };
        let code = f
            .state
            .store
            .lock()
            .unwrap()
            .create_pairing_code(&user, 900)
            .unwrap()
            .0;
        let (cookie, device) = pair(&f.state, &code).await;
        let stale = [
            ("idempotency-key", KEY),
            ("x-dunlin-expected-user", f.user.as_str()),
            ("x-dunlin-expected-device", f.device.as_str()),
        ];
        assert_eq!(
            post(&f.state, &cookie, &f.path(), &stale, options())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.count("interactive_sessions"), 0);
        assert_eq!(f.starts(), 0);
        let current = [
            ("idempotency-key", KEY),
            ("x-dunlin-expected-user", user.as_str()),
            ("x-dunlin-expected-device", device.as_str()),
        ];
        let accepted = post(&f.state, &cookie, &f.path(), &current, options()).await;
        assert_eq!(accepted.0, StatusCode::ACCEPTED, "{}", accepted.2);
        assert_eq!(accepted.2["session"]["owner"], user);
        f.initialized(1).await;
        assert_eq!(f.count("interactive_sessions"), 1);
        assert_eq!(f.starts(), 1);
        f.state.interactive.shutdown().await;
    }
}

#[tokio::test]
async fn lost_http_response_and_reopened_recovered_relay_replay_the_receipt_without_restarting_actor()
 {
    let f = AdmissionFixture::new().await;
    let (origin, _server) = super::live_transport::start(&f.state).await;
    let address = origin.strip_prefix("http://").unwrap();
    let body = options().to_string();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nCookie: {}\r\nContent-Type: application/json\r\nIdempotency-Key: {KEY}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        f.path(),
        f.cookie,
        body.len()
    );
    socket.write_all(request.as_bytes()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.count("interactive_sessions") != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(socket); // Never consume the actual HTTP acceptance response.
    f.initialized(1).await;
    let replay = keyed(&f, KEY, options()).await;
    assert_eq!(replay.0, StatusCode::ACCEPTED);
    assert_eq!(replay.1["idempotency-replayed"], "true");
    let receipt = replay.2["session"].clone();
    assert_eq!(f.starts(), 1);
    f.state.interactive.shutdown().await;
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    reopened.recover_interactive_sessions().unwrap();
    let mut state = AppState::new(reopened, (*f.state.config).clone());
    configure_peer(&mut state);
    std::sync::Arc::make_mut(&mut state.config).experimental_interactive_runtime = false;
    let after = post(
        &state,
        &f.cookie,
        &f.path(),
        &[("idempotency-key", KEY)],
        options(),
    )
    .await;
    assert_eq!(after.0, StatusCode::ACCEPTED, "{}", after.2);
    assert_eq!(after.1["idempotency-replayed"], "true");
    assert_eq!(after.2["session"], receipt);
    assert_eq!(
        receipt["phase"], "active",
        "The receipt describes original admission, not a fresh actor"
    );
    let current_phase = {
        let store = state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.user)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&context, receipt["id"].as_str().unwrap())
            .unwrap()
            .0
            .phase
    };
    assert_eq!(
        current_phase,
        dunlin_session_runtime::SessionPhase::Interrupted
    );
    assert_eq!(
        post(
            &state,
            &f.cookie,
            &f.path(),
            &[("idempotency-key", "new-disabled-intent-0002")],
            options()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.starts(), 1);
    assert_eq!(f.count("interactive_sessions"), 1);
}

#[tokio::test]
async fn native_epoch_changes_or_revoked_browser_identity_never_replay_or_create_a_second_runtime()
{
    let f = AdmissionFixture::new().await;
    let original = keyed(&f, KEY, options()).await;
    assert_eq!(original.0, StatusCode::ACCEPTED);
    f.initialized(1).await;
    {
        let store = f.state.store.lock().unwrap();
        store.revoke_grant(&f.native, &f.user).unwrap();
        store.grant(&f.native, &f.user).unwrap();
    }
    assert_eq!(keyed(&f, KEY, options()).await.0, StatusCode::CONFLICT);
    assert_eq!(f.count("interactive_sessions"), 1);
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&f.device)
        .unwrap();
    assert_eq!(keyed(&f, KEY, options()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(f.count("interactive_sessions"), 1);
    assert_eq!(f.starts(), 1);
    f.state.interactive.shutdown().await;
}

#[tokio::test]
async fn typed_target_http_never_falls_back_remote_to_local_or_replays_a_different_target() {
    let f = AdmissionFixture::new().await;
    let mut remote = options();
    remote["target"] = json!({"kind":"remote","binding":{"id":"rbd-explicit","revision":1}});
    let mut fs_permits = Vec::new();
    while let Ok(permit) = f.state.fs_permit() {
        fs_permits.push(permit);
    }
    assert!(!fs_permits.is_empty());
    let denied = keyed(&f, KEY, remote.clone()).await;
    assert_eq!(denied.0, StatusCode::NOT_IMPLEMENTED, "{}", denied.2);
    let mut invalid = options();
    invalid["target"] = json!({"kind":"local","revision":0});
    let invalid = keyed(&f, KEY, invalid).await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST, "{}", invalid.2);
    assert_eq!(invalid.2["error"], "invalid_project_target");
    drop(fs_permits);
    let mut disabled = f.state.clone();
    std::sync::Arc::make_mut(&mut disabled.config).experimental_interactive_runtime = false;
    let disabled_remote = post(&disabled, &f.cookie, &f.path(), &[], remote.clone()).await;
    assert_eq!(
        disabled_remote.0,
        StatusCode::NOT_IMPLEMENTED,
        "{}",
        disabled_remote.2
    );
    assert_eq!(denied.2["error"], "remote_execution_not_supported");
    assert_eq!(f.count("interactive_sessions"), 0);
    assert_eq!(f.count("interactive_session_admissions"), 0);
    assert_eq!(f.starts(), 0);
    for target in [
        json!({"kind":"local","revision":0}),
        json!({"kind":"local","revision":1,"path":"/outside"}),
    ] {
        let mut invalid = options();
        invalid["target"] = target;
        let denied = keyed(&f, KEY, invalid).await;
        assert!(denied.0.is_client_error(), "{}", denied.2);
    }
    assert_eq!(f.count("interactive_session_admissions"), 0);
    let mut stale = options();
    stale["target"] = json!({"kind":"local","revision":2});
    assert_eq!(keyed(&f, KEY, stale).await.0, StatusCode::CONFLICT);
    let mut local = options();
    local["target"] = json!({"kind":"local","revision":1});
    let accepted = keyed(&f, KEY, local.clone()).await;
    assert_eq!(accepted.0, StatusCode::ACCEPTED, "{}", accepted.2);
    assert_eq!(accepted.2["admission"]["replayed"], false);
    f.initialized(1).await;
    let replay = keyed(&f, KEY, local).await;
    assert_eq!(replay.0, StatusCode::ACCEPTED, "{}", replay.2);
    assert_eq!(replay.2["admission"]["replayed"], true);
    assert_eq!(replay.2["session"], accepted.2["session"]);
    assert_eq!(keyed(&f, KEY, options()).await.0, StatusCode::CONFLICT);
    assert_eq!(
        keyed(&f, KEY, remote.clone()).await.0,
        StatusCode::NOT_IMPLEMENTED
    );
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_binding(&f.user, &f.workspace, &f.project, 1)
        .unwrap();
    assert_eq!(keyed(&f, KEY, remote).await.0, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(f.count("interactive_sessions"), 1);
    assert_eq!(f.count("interactive_session_admissions"), 1);
    assert_eq!(f.starts(), 1);
    f.state.interactive.shutdown().await;
}
