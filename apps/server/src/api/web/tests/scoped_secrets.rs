use super::*;
use dunlin_domain::{
    collaboration::{Permission, WorkspaceRole},
    hosts::{HostCapabilities, HostDefinition},
};
use dunlin_vault::store::{MemoryStore, SecretStore};
use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use zeroize::Zeroizing;

#[derive(Default)]
struct Secrets {
    values: MemoryStore,
    writes: Mutex<Vec<String>>,
    reads: AtomicUsize,
    lose_ack: AtomicBool,
    after_write: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.values.get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.values.set(id, value)?;
        self.writes.lock().unwrap().push(id.into());
        let hook = self.after_write.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        if self.lose_ack.swap(false, Ordering::AcqRel) {
            return Err("fixture private error after keyring effect".into());
        }
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        panic!("no implicit cleanup")
    }
    fn get_policy(&self, _: &str) -> Result<Zeroizing<String>, String> {
        panic!("no split policy")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("no split policy")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("no split policy")
    }
}
struct Scope {
    state: AppState,
    secrets: Arc<Secrets>,
    owner: String,
    workspace: String,
    project: String,
    host: String,
    cookie: String,
    base: String,
}
async fn scope() -> Scope {
    let store = Store::open_in_memory().unwrap();
    let owner = store.create_user("secret-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Scoped").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Secrets")
        .unwrap();
    let def:HostDefinition=serde_json::from_value(json!({"name":"Fixture","node_id":"scoped-worker","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["claude"]}})).unwrap();
    let host = store.create_host(&owner.id, &workspace.id, def).unwrap();
    let caps:HostCapabilities=serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap();
    store.enroll_host(&host.enrollment.ticket, caps).unwrap();
    store
        .approve_host(&owner.id, &workspace.id, &host.host.id, 2)
        .unwrap();
    let secrets = Arc::new(Secrets::default());
    let mut state = AppState::new(
        store,
        Config {
            db: "/nonexistent/scoped-fixture/relay.db".into(),
            ..Config::default()
        },
    );
    state.scoped_secrets = Arc::new(crate::scoped_secrets::Service::testing(secrets.clone()));
    let cookie = paired_cookie(&state, &owner.id).await;
    let base = format!(
        "/v1/web/workspaces/{}/projects/{}/hosts/{}/scoped-secrets",
        workspace.id, project.id, host.host.id
    );
    Scope {
        state,
        secrets,
        owner: owner.id,
        workspace: workspace.id,
        project: project.id,
        host: host.host.id,
        cookie,
        base,
    }
}
fn save(s: &Scope, revision: u64) -> Value {
    json!({"expected_revision":revision,"value":"fixture-scoped-private-value","policy":{"version":1,"managed":true,"injection":{"type":"header","name":"Authorization","prefix":"Bearer "},"allowed_hosts":["api.example.com"],"path_prefix":"/","allowed_methods":["GET"],"allowed_headers":[],"auto_approve":false,"expires_at":null,"project_ids":[s.project],"recipes":[],"operations":[]}})
}
async fn send(
    s: &Scope,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
    origin: &str,
    expected: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "build.tail123.ts.net")
        .header("origin", origin)
        .header("cookie", cookie)
        .header("content-type", "application/json");
    if let Some(expected) = expected {
        builder = builder.header("x-dunlin-expected-user", expected);
    }
    let response = crate::api::router(s.state.clone())
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 100000).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        !text.contains("fixture-scoped-private-value")
            && !text.contains("scoped-secret:")
            && !text.contains("fixture private error"),
        "private response leaked data"
    );
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}
async fn request(s: &Scope, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    send(
        s,
        method,
        path,
        &s.cookie,
        body,
        "https://build.tail123.ts.net",
        None,
    )
    .await
}

#[tokio::test]
async fn scoped_secret_api_provisions_rotates_lists_and_disables_without_reading_values() {
    let s = scope().await;
    let path = format!("{}/work-api", s.base);
    let (status, receipt) = request(&s, "POST", &path, save(&s, 0)).await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["activation"], "confirmed");
    let (status, list) = request(&s, "GET", &s.base, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["credentials"][0]["state"], "active");
    assert_eq!(list["execution_supported"], false);
    let mut updated = save(&s, 1);
    updated["value"] = json!("different-fixture-secret");
    assert_eq!(request(&s, "POST", &path, updated).await.0, StatusCode::OK);
    assert_eq!(s.secrets.writes.lock().unwrap().len(), 2);
    {
        let writes = s.secrets.writes.lock().unwrap();
        assert_ne!(writes[0], writes[1]);
    }
    let disable = format!("{path}/disable");
    assert_eq!(
        request(&s, "POST", &disable, json!({"expected_revision":2}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&s, "POST", &path, save(&s, 2)).await.0,
        StatusCode::CONFLICT
    );
    let (_, list) = request(&s, "GET", &s.base, Value::Null).await;
    assert_eq!(list["credentials"][0]["state"], "disabled");
    assert_eq!(list["credentials"][0]["revision"], 3);
    assert_eq!(s.secrets.reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn scoped_secret_lost_write_ack_is_retained_and_old_retry_never_writes_again() {
    let s = scope().await;
    let path = format!("{}/work-api", s.base);
    s.secrets.lose_ack.store(true, Ordering::Release);
    assert_eq!(
        request(&s, "POST", &path, save(&s, 0)).await.0,
        StatusCode::CONFLICT
    );
    let (_, list) = request(&s, "GET", &s.base, Value::Null).await;
    assert_eq!(list["credentials"][0]["state"], "installing");
    assert_eq!(
        request(&s, "POST", &path, save(&s, 0)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(s.secrets.writes.lock().unwrap().len(), 1);
    assert_eq!(
        request(&s, "POST", &path, save(&s, 1)).await.0,
        StatusCode::OK
    );
    let refs = s.secrets.writes.lock().unwrap();
    assert_eq!(refs.len(), 2);
    assert_ne!(refs[0], refs[1]);
    assert!(s.secrets.values.get(&refs[0]).is_ok());
}

#[tokio::test]
async fn scoped_secret_grant_aba_during_keyring_write_keeps_ack_but_blocks_activation() {
    let s = scope().await;
    let state = s.state.clone();
    let actor = s.owner.clone();
    let workspace = s.workspace.clone();
    let project = s.project.clone();
    *s.secrets.after_write.lock().unwrap() = Some(Box::new(move || {
        let store = state.store.lock().unwrap();
        let grant = store
            .team_project_grant(&workspace, &project, &actor)
            .unwrap()
            .unwrap();
        store
            .set_team_project_grant(
                &actor,
                &workspace,
                &project,
                &actor,
                BTreeSet::from([Permission::ViewProject, Permission::ManageProject]),
                Some(grant.revision),
            )
            .unwrap();
        store
            .set_team_project_grant(
                &actor,
                &workspace,
                &project,
                &actor,
                grant.permissions,
                Some(grant.revision + 1),
            )
            .unwrap();
    }));
    let path = format!("{}/work-api", s.base);
    assert_eq!(
        request(&s, "POST", &path, save(&s, 0)).await.0,
        StatusCode::FORBIDDEN
    );
    let (_, list) = request(&s, "GET", &s.base, Value::Null).await;
    assert_eq!(list["credentials"][0]["state"], "installing");
    assert_eq!(
        request(&s, "POST", &path, save(&s, 1)).await.0,
        StatusCode::OK
    );
    assert_eq!(s.secrets.writes.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn scoped_secret_logout_during_write_prevents_activation_and_never_reveals_result() {
    let s = scope().await;
    let state = s.state.clone();
    let token = s.cookie.split_once('=').unwrap().1.to_owned();
    *s.secrets.after_write.lock().unwrap() = Some(Box::new(move || {
        state
            .store
            .lock()
            .unwrap()
            .remove_browser_session(&token)
            .unwrap();
    }));
    let path = format!("{}/work-api", s.base);
    assert_eq!(
        request(&s, "POST", &path, save(&s, 0)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(s.secrets.writes.lock().unwrap().len(), 1);
    let new_cookie = paired_cookie(&s.state, &s.owner).await;
    let (_, list) = send(
        &s,
        "GET",
        &s.base,
        &new_cookie,
        Value::Null,
        "https://build.tail123.ts.net",
        None,
    )
    .await;
    assert_eq!(list["credentials"][0]["state"], "installing");
}

#[tokio::test]
async fn scoped_secret_requests_require_cookie_origin_exact_actor_and_explicit_host_and_project_grants()
 {
    let s = scope().await;
    let path = format!("{}/work-api", s.base);
    assert_eq!(
        send(
            &s,
            "POST",
            &path,
            &s.cookie,
            save(&s, 0),
            "https://evil.example",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &s,
            "POST",
            &path,
            &s.cookie,
            save(&s, 0),
            "https://build.tail123.ts.net",
            Some("another-user")
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut extra = save(&s, 0);
    extra["actor"] = json!(s.owner);
    assert_eq!(
        request(&s, "POST", &path, extra).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let member = {
        let store = s.state.store.lock().unwrap();
        let member = store.create_user("scoped-member", "member").unwrap();
        store
            .set_team_member(
                &s.owner,
                &s.workspace,
                &member.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &s.owner,
                &s.workspace,
                &s.project,
                &member.id,
                BTreeSet::from([Permission::ViewProject, Permission::ManageProject]),
                None,
            )
            .unwrap();
        member
    };
    let cookie = paired_cookie(&s.state, &member.id).await;
    assert_eq!(
        send(
            &s,
            "POST",
            &path,
            &cookie,
            save(&s, 0),
            "https://build.tail123.ts.net",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let wrong_host = path.replace(&s.host, "another-host");
    assert_eq!(
        request(&s, "POST", &wrong_host, save(&s, 0)).await.0,
        StatusCode::FORBIDDEN
    );
    assert!(s.secrets.writes.lock().unwrap().is_empty());
}
