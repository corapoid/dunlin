//! Production bundle and a separate-site consent page; no provider accounts.
use super::*;
use axum::{
    Form, Router,
    extract::Query,
    middleware::{self, Next},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dunlin_vault::store::{MemoryStore, SecretStore};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use zeroize::Zeroizing;
type WriterCheck = Box<dyn Fn() + Send + Sync>;
#[derive(Clone)]
struct Provider {
    origin: String,
    callback: String,
    codes: Arc<Mutex<HashMap<String, String>>>,
    authorizations: Arc<AtomicUsize>,
    tokens: Arc<AtomicUsize>,
    catalog_reads: Arc<AtomicUsize>,
    tool_calls: Arc<AtomicUsize>,
    revocations: Arc<AtomicUsize>,
    writer_check: Arc<Mutex<Option<WriterCheck>>>,
}
async fn probe(
    State(p): State<Provider>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if headers.get(header::AUTHORIZATION).is_some() {
        if let Some(check) = p.writer_check.lock().unwrap().as_ref() {
            check();
        }
        assert_eq!(
            headers[header::AUTHORIZATION],
            "Bearer fixture-browser-private-access"
        );
        assert!(!headers.contains_key(header::COOKIE));
        let result = match request["method"].as_str().unwrap() {
            "server/discover" => {
                json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
            }
            "tools/list" => {
                p.catalog_reads.fetch_add(1, Ordering::SeqCst);
                json!({"resultType":"complete","tools":[{"name":"lookup","inputSchema":{"type":"object"}}],"ttlMs":0,"cacheScope":"private"})
            }
            "tools/call" => {
                assert_eq!(request["params"]["name"], "lookup");
                assert_eq!(
                    request["params"]["arguments"],
                    json!({"query":"approved fixture"})
                );
                p.tool_calls.fetch_add(1, Ordering::SeqCst);
                json!({"resultType":"complete","content":[{"type":"text","text":"private fixture result"}],"ttlMs":0,"cacheScope":"private"})
            }
            _ => panic!("unexpected MCP method"),
        };
        return Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response();
    }
    let mut response = (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}
async fn resource(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"resource":format!("{}/mcp",p.origin),"authorization_servers":[format!("{}/issuer",p.origin)]}),
    )
}
async fn metadata(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"issuer":format!("{}/issuer",p.origin),"authorization_endpoint":format!("{}/authorize",p.origin),"token_endpoint":format!("{}/token",p.origin),"revocation_endpoint":format!("{}/revoke",p.origin),"revocation_endpoint_auth_methods_supported":["none"],"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true}),
    )
}
async fn consent(
    State(p): State<Provider>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Html<String> {
    assert!(
        !headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("dunlin_web_session=")
    );
    assert_eq!(q["redirect_uri"], p.callback);
    assert_eq!(q["code_challenge_method"], "S256");
    let index = p.authorizations.fetch_add(1, Ordering::SeqCst) + 1;
    let code = format!("fixture-code-{index}");
    p.codes
        .lock()
        .unwrap()
        .insert(code.clone(), q["code_challenge"].clone());
    let mut callback = url::Url::parse(&p.callback).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &q["state"])
        .append_pair("iss", &format!("{}/issuer", p.origin));
    Html(format!(
        "<!doctype html><html lang=pl><meta name=referrer content=no-referrer><title>Fixture MCP</title><h1>Fixture MCP</h1><p>Lokalna strona zgody.</p><a href=\"{}\">Zatwierdź połączenie</a></html>",
        callback
            .as_str()
            .replace('&', "&amp;")
            .replace('"', "&quot;")
    ))
}
async fn token(
    State(p): State<Provider>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    let expected = p
        .codes
        .lock()
        .unwrap()
        .remove(&form["code"])
        .expect("single-use authorization code");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
        expected
    );
    assert_eq!(form["redirect_uri"], p.callback);
    assert_eq!(form["resource"], format!("{}/mcp", p.origin));
    p.tokens.fetch_add(1, Ordering::SeqCst);
    Json(
        json!({"access_token":"fixture-browser-private-access","refresh_token":"fixture-browser-private-refresh","token_type":"Bearer","expires_in":3600}),
    )
}
async fn revoke(
    State(p): State<Provider>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> StatusCode {
    assert!(!headers.contains_key(header::COOKIE));
    assert!(!headers.contains_key(header::AUTHORIZATION));
    assert_eq!(form["client_id"], "browser-client");
    match form["token_type_hint"].as_str() {
        "refresh_token" => assert_eq!(form["token"], "fixture-browser-private-refresh"),
        "access_token" => assert_eq!(form["token"], "fixture-browser-private-access"),
        _ => panic!("unexpected token hint"),
    }
    p.revocations.fetch_add(1, Ordering::SeqCst);
    StatusCode::OK
}
#[derive(Default)]
struct PrivateStore {
    values: MemoryStore,
    writes: AtomicUsize,
    deletes: Mutex<Vec<String>>,
    late_write: std::sync::atomic::AtomicBool,
    late_writes: AtomicUsize,
    writer_check: Mutex<Option<WriterCheck>>,
}
impl SecretStore for PrivateStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.values.get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        if let Some(check) = self.writer_check.lock().unwrap().as_ref() {
            check();
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.values.set(id, value)
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        let old = self.values.get(id).ok();
        self.values.delete(id)?;
        self.deletes.lock().unwrap().push(id.to_owned());
        if self.late_write.swap(false, Ordering::SeqCst)
            && let Some(old) = old
        {
            self.values.set(id, &old)?;
            self.late_writes.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    fn get_policy(&self, _: &str) -> Result<Zeroizing<String>, String> {
        panic!("OAuth is private")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("OAuth must not become a vault tool")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("OAuth is private")
    }
}
#[derive(Clone, Default)]
struct Observation {
    callback_cookies: Arc<Mutex<Vec<(bool, bool)>>>,
    disconnects: Arc<AtomicUsize>,
    starts: Arc<AtomicUsize>,
}
async fn observe(
    State(observation): State<Observation>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path.ends_with("/mcp/oauth/fixture/callback") {
        let raw = request
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let names = raw
            .split(';')
            .filter_map(|v| v.trim().split_once('=').map(|(key, _)| key))
            .collect::<Vec<_>>();
        observation.callback_cookies.lock().unwrap().push((
            names.contains(&"dunlin_web_session"),
            names.contains(&"dunlin_mcp_oauth_flow"),
        ));
    }
    if path.ends_with("/disconnect") {
        observation.disconnects.fetch_add(1, Ordering::SeqCst);
    }
    if path.ends_with("/authorize") {
        observation.starts.fetch_add(1, Ordering::SeqCst);
    }
    next.run(request).await
}
#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run browser sandbox suite"]
async fn production_browser_acceptance_mcp_oauth() {
    browser_mcp_oauth().await;
}

#[tokio::test]
#[ignore = "requires production relay helper, web bundle and Chromium in runtime sandbox"]
async fn production_browser_acceptance_runtime_mcp_oauth_revocation() {
    browser_mcp_oauth().await;
}

async fn browser_mcp_oauth() {
    use tokio::io::AsyncWriteExt;
    let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", relay_listener.local_addr().unwrap());
    let provider_listener = tokio::net::TcpListener::bind("127.0.0.2:0").await.unwrap();
    let provider = Provider {
        origin: format!("http://{}", provider_listener.local_addr().unwrap()),
        callback: format!("{origin}/v1/web/mcp/oauth/fixture/callback"),
        codes: Arc::new(Mutex::new(HashMap::new())),
        authorizations: Arc::new(AtomicUsize::new(0)),
        tokens: Arc::new(AtomicUsize::new(0)),
        catalog_reads: Arc::new(AtomicUsize::new(0)),
        tool_calls: Arc::new(AtomicUsize::new(0)),
        revocations: Arc::new(AtomicUsize::new(0)),
        writer_check: Default::default(),
    };
    let app = Router::new()
        .route("/mcp", post(probe))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource))
        .route(
            "/.well-known/oauth-authorization-server/issuer",
            get(metadata),
        )
        .route("/authorize", get(consent))
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .with_state(provider.clone());
    let provider_server = tokio::spawn(async move {
        axum::serve(provider_listener, app).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("relay.db")).unwrap();
    let user = store.create_user("mcp-browser-owner", "admin").unwrap();
    let workspace = store
        .create_workspace(&user, "MCP browser workspace")
        .unwrap();
    let code = store.create_pairing_code(&user.id, 900).unwrap().0;
    // A durable fixture cut after a one-use claim but before any secret/AS I/O.
    // Kernel process-death behavior has its own actual SIGKILL test.
    use dunlin_mcp_client::oauth::{
        AuthContext,
        persistence::{CredentialJournal, RevocationJournal},
    };
    let mut interrupted = AuthContext {
        user_id: user.id.clone(),
        workspace_id: workspace.id.clone(),
        connection_id: format!("mcp-{}", uuid::Uuid::now_v7()),
        connection_revision: 1,
    };
    let binding = "c".repeat(64);
    let digest = "d".repeat(64);
    store
        .mcp_oauth_prepare(
            &interrupted,
            &binding,
            "fixture",
            Some("Przerwane odwołanie"),
            &|_, _| Ok(()),
        )
        .unwrap();
    let installing = store.begin_install(&interrupted, &binding).unwrap();
    store
        .prepare_write(&interrupted, &binding, &installing, &digest)
        .unwrap();
    store
        .commit(&interrupted, &binding, &installing, &digest)
        .unwrap();
    let lease = store.mcp_oauth_revocation_lease().unwrap();
    let revoking = store.begin_revocation(&interrupted, &binding).unwrap();
    store
        .claim_revocation_target(&interrupted, &revoking, 0)
        .unwrap();
    interrupted.connection_revision = revoking.connection_revision;
    drop(lease);
    let checkout = directory.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let project = store
        .create_team_project(&user.id, &workspace.id, "MCP browser project")
        .unwrap();
    let native = store
        .create_project("MCP browser checkout", checkout.to_str().unwrap())
        .unwrap();
    store.grant(&native.id, &user.id).unwrap();
    store
        .set_local_binding(&user.id, &workspace.id, &project.id, &native.id, None)
        .unwrap();
    let real_proxy = std::env::var_os("DUNLIN_RELAY_E2E_BIN").is_some();
    let script = checkout.join("fake_acp.py");
    std::fs::write(
        &script,
        include_str!("../../../interactive/tests/fake_acp.py"),
    )
    .unwrap();
    let config = Config {
        db: directory.path().join("relay.db"),
        experimental_interactive_runtime: true,
        sandbox: real_proxy,
        web_root: Some(
            std::env::var_os("DUNLIN_WEB_E2E_ROOT")
                .expect("production build required")
                .into(),
        ),
        mcp_oauth: vec![crate::mcp_oauth::config::Provider {
            id: "fixture".into(),
            name: "Fixture MCP".into(),
            endpoint: format!("{}/mcp", provider.origin),
            issuer: format!("{}/issuer", provider.origin),
            client_id: "browser-client".into(),
            redirect_uri: provider.callback.clone(),
            allowed_origins: vec![],
            scopes: vec![],
            authentication: crate::mcp_oauth::config::Authentication::Public,
            client_secret_ref: None,
        }],
        ..Config::default()
    };
    let mut state = AppState::new(store, config);
    Arc::get_mut(&mut state.interactive).unwrap().command = Some((
        "/usr/bin/python3".into(),
        vec![
            script.to_str().unwrap().into(),
            if real_proxy { "mcp-real" } else { "mcp" }.into(),
        ],
    ));
    let private = Arc::new(PrivateStore::default());
    let checking = state.store.clone();
    *private.writer_check.lock().unwrap() = Some(Box::new(move || {
        let store = checking
            .try_lock()
            .expect("private I/O must not hold Store");
        assert!(
            store.mcp_oauth_writer_lease().is_err(),
            "secret publication must retain the writer lease"
        );
    }));
    let checking = state.store.clone();
    *provider.writer_check.lock().unwrap() = Some(Box::new(move || {
        let store = checking.try_lock().expect("MCP HTTP must not hold Store");
        assert!(
            store.mcp_oauth_writer_lease().is_ok(),
            "MCP catalog/tools must run after releasing the writer lease"
        );
    }));
    private.late_write.store(true, Ordering::SeqCst);
    state.mcp_oauth = Arc::new(crate::mcp_oauth::Service::testing(private.clone()));
    let observation = Observation::default();
    let app = crate::api::router(state.clone())
        .layer(middleware::from_fn_with_state(observation.clone(), observe));
    let web = tokio::spawn(async move {
        axum::serve(relay_listener, app).await.unwrap();
    });
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/mcp-oauth.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(json!({"origin":origin,"provider_origin":provider.origin,"pair_code":code,"workspace":workspace.id,"project":project.id}).to_string().as_bytes()).await.unwrap();
    let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    web.abort();
    provider_server.abort();
    assert!(
        output.status.success(),
        "MCP OAuth browser acceptance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(provider.authorizations.load(Ordering::SeqCst), 2);
    assert_eq!(provider.tokens.load(Ordering::SeqCst), 2);
    assert_eq!(provider.catalog_reads.load(Ordering::SeqCst), 4);
    assert_eq!(provider.tool_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.revocations.load(Ordering::SeqCst), 4);
    assert_eq!(private.writes.load(Ordering::SeqCst), 2);
    assert_eq!(private.deletes.lock().unwrap().len(), 4);
    assert_eq!(private.late_writes.load(Ordering::SeqCst), 1);
    for reference in private.deletes.lock().unwrap().iter() {
        assert!(private.values.get(reference).is_err());
    }

    assert_eq!(
        *observation.callback_cookies.lock().unwrap(),
        vec![(false, true), (false, true)]
    );
    assert_eq!(observation.disconnects.load(Ordering::SeqCst), 3);
    assert_eq!(observation.starts.load(Ordering::SeqCst), 4);
    let connections = state
        .store
        .lock()
        .unwrap()
        .mcp_oauth_connections(&user.id, &workspace.id)
        .unwrap();
    assert_eq!(connections.len(), 2);
    let recovered = connections
        .iter()
        .find(|c| c.id == interrupted.connection_id)
        .unwrap();
    let recovered = serde_json::to_value(recovered).unwrap();
    assert_eq!(recovered["status"], "disabled");
    assert_eq!(recovered["revocation"]["state"], "unconfirmed");
    assert_eq!(recovered["revocation"]["unconfirmed"], 1);
    let connection = connections
        .iter()
        .find(|c| c.id != interrupted.connection_id)
        .unwrap();
    assert_eq!(connection.display_name.as_deref(), Some("Konto służbowe"));
    assert_eq!(
        serde_json::to_value(connection).unwrap()["status"],
        "disabled"
    );
    let receipt = serde_json::to_value(connection).unwrap();
    let metadata = rusqlite::Connection::open(directory.path().join("relay.db")).unwrap();
    assert_eq!(metadata.query_row("SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE connection_id=?1 AND writer_protected=1",[&connection.id],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(receipt["revocation"]["state"], "confirmed_known_tokens");
    assert_eq!(receipt["revocation"]["confirmed"], 2);
    assert_eq!(receipt["cleanup"]["revision"], 2);
    assert_eq!(receipt["cleanup"]["status"], "sweep_complete");
    assert_eq!(receipt["cleanup"]["delete_acknowledged"], 2);

    println!("{}", String::from_utf8_lossy(&output.stdout));
}
