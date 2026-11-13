use super::*;
use axum::{
    Form, Router,
    body::{Body, to_bytes},
    http::Request,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use openidconnect::{
    AccessToken, JsonWebKeyId, PrivateSigningKey,
    core::{
        CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
        CoreRsaPrivateSigningKey,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

fn signing() -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(
        include_str!("../../../../../../tests/fixtures/oidc/test-only-private.pem"),
        Some(JsonWebKeyId::new("test-only".into())),
    )
    .unwrap()
}
#[derive(Clone)]
struct Provider {
    issuer: String,
    nonce: Arc<Mutex<String>>,
    challenge: Arc<Mutex<String>>,
}
async fn metadata(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"issuer":p.issuer,"authorization_endpoint":format!("{}/authorize",p.issuer),"token_endpoint":format!("{}/token",p.issuer),"jwks_uri":format!("{}/keys",p.issuer),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"code_challenge_methods_supported":["S256"]}),
    )
}
async fn keys() -> Json<Value> {
    Json(
        serde_json::to_value(CoreJsonWebKeySet::new(vec![
            signing().as_verification_key(),
        ]))
        .unwrap(),
    )
}
async fn authorize(
    State(p): State<Provider>,
    Query(query): Query<HashMap<String, String>>,
) -> Redirect {
    *p.nonce.lock().unwrap() = query["nonce"].clone();
    *p.challenge.lock().unwrap() = query["code_challenge"].clone();
    let mut callback = openidconnect::url::Url::parse(&query["redirect_uri"]).unwrap();
    assert_eq!(callback.host_str(), Some("127.0.0.1"));
    callback
        .query_pairs_mut()
        .append_pair("code", "valid")
        .append_pair("state", &query["state"]);
    Redirect::to(callback.as_str())
}

async fn token(
    State(p): State<Provider>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    let verifier = form.get("code_verifier").unwrap();
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        *p.challenge.lock().unwrap()
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = form.get("code").unwrap().as_str();
    let claims:CoreIdTokenClaims=serde_json::from_value(json!({"iss":if code=="bad-issuer"{"https://other.example"}else{&p.issuer},"sub":if code=="unbound"{"other"}else{"known-subject"},"aud":if code=="bad-audience"{"other"}else{"test-client"},"exp":if code=="expired"{now-1}else{now+600},"iat":if code=="future-iat"{now+3600}else{now},"azp":if code=="bad-azp"{"other"}else{"test-client"},"nonce":if code=="bad-nonce"{"other".into()}else{p.nonce.lock().unwrap().clone()}})).unwrap();
    let access = AccessToken::new("test-only-access-token".into());
    let signed = CoreIdToken::new(
        claims,
        &signing(),
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        Some(&access),
        None,
    )
    .unwrap()
    .to_string();
    let signed = if code == "bad-signature" {
        let (head, signature) = signed.rsplit_once('.').unwrap();
        let mut bytes = URL_SAFE_NO_PAD.decode(signature).unwrap();
        bytes[0] ^= 1;
        format!("{head}.{}", URL_SAFE_NO_PAD.encode(bytes))
    } else {
        signed
    };
    Json(
        json!({"access_token":if code=="bad-access"{"substituted"}else{access.secret()},"token_type":"Bearer","id_token":signed,"expires_in":600}),
    )
}

async fn fixture() -> (AppState, Provider, tokio::task::JoinHandle<()>, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = Provider {
        issuer: format!("http://{}", listener.local_addr().unwrap()),
        nonce: Arc::new(Mutex::new(String::new())),
        challenge: Arc::new(Mutex::new(String::new())),
    };
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(metadata))
        .route("/keys", get(keys))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(provider.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = crate::store::Store::open_in_memory().unwrap();
    let user = store.create_user("oidc-admin", "admin").unwrap();
    let id = user.id.clone();
    let config = crate::config::Config {
        oidc: Some(crate::oidc::Config {
            issuer: provider.issuer.clone(),
            client_id: "test-client".into(),
            redirect_uri: "http://127.0.0.1:8787/v1/web/oidc/callback".into(),
            client_secret_env: None,
            allowed_origins: vec![],
            bindings: BTreeMap::from([("known-subject".into(), user.id)]),
        }),
        ..Default::default()
    };
    (AppState::new(store, config), provider, server, id)
}
async fn request(state: &AppState, path: &str, method: &str, cookie: Option<&str>) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8787");
    if method == "POST" {
        request = request.header("origin", "http://127.0.0.1:8787");
    }
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    crate::api::router(state.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn begin(state: &AppState, provider: &Provider) -> (String, String) {
    let response = request(state, "/v1/web/oidc/start", "POST", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    let url = openidconnect::url::Url::parse(value["authorization_url"].as_str().unwrap()).unwrap();
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["scope"], "openid");
    *provider.nonce.lock().unwrap() = params["nonce"].clone();
    *provider.challenge.lock().unwrap() = params["code_challenge"].clone();
    (params["state"].clone(), cookie)
}

#[tokio::test]
async fn code_flow_issues_private_cookie_and_rejects_replay_wrong_browser_and_revocation() {
    let (state, provider, server, user) = fixture().await;
    let (flow, browser) = begin(&state, &provider).await;
    let path = format!("/v1/web/oidc/callback?code=valid&state={flow}");
    assert_eq!(
        request(&state, &path, "GET", None).await.status(),
        StatusCode::FORBIDDEN
    );
    let response = request(&state, &path, "GET", Some(&browser)).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|h| h.to_str().unwrap().to_string())
        .collect();
    let session = cookies
        .iter()
        .find(|h| h.starts_with("dunlin_web_session="))
        .unwrap();
    assert!(session.contains("HttpOnly; SameSite=Strict"));
    let cookie = session.split(';').next().unwrap();
    let response = request(&state, "/v1/web/session", "GET", Some(cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value["user"]["id"], user);
    assert_eq!(
        request(&state, &path, "GET", Some(&browser)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let mut revoked = state.clone();
    Arc::make_mut(&mut revoked.config).oidc = None;
    assert_eq!(
        request(&revoked, "/v1/web/session", "GET", Some(cookie))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    server.abort();
}

#[tokio::test]
async fn token_claims_and_access_token_are_verified_before_local_login() {
    let (state, provider, server, user) = fixture().await;
    for code in [
        "bad-signature",
        "bad-azp",
        "future-iat",
        "bad-issuer",
        "bad-audience",
        "expired",
        "bad-nonce",
        "unbound",
        "bad-access",
    ] {
        let (flow, browser) = begin(&state, &provider).await;
        let path = format!("/v1/web/oidc/callback?code={code}&state={flow}");
        assert_eq!(
            request(&state, &path, "GET", Some(&browser)).await.status(),
            StatusCode::UNAUTHORIZED,
            "{code}"
        );
    }
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .list_devices(&user)
            .unwrap()
            .is_empty()
    );
    server.abort();
}

#[tokio::test]
async fn login_start_rejects_cross_origin_and_wrong_callback_authority() {
    let (state, _, server, _) = fixture().await;
    for (host, origin) in [
        ("127.0.0.1:8787", "https://evil.example"),
        ("localhost:8787", "http://localhost:8787"),
    ] {
        let response = crate::api::router(state.clone())
            .oneshot(
                Request::post("/v1/web/oidc/start")
                    .header("host", host)
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
    server.abort();
}

#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run browser sandbox suite"]
async fn production_browser_acceptance_oidc() {
    use tokio::io::AsyncWriteExt;
    let (mut state, provider, provider_server, _) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let config = Arc::make_mut(&mut state.config);
    config.web_root = Some(
        std::env::var_os("DUNLIN_WEB_E2E_ROOT")
            .expect("production build required")
            .into(),
    );
    config.oidc.as_mut().unwrap().redirect_uri = format!("{origin}/v1/web/oidc/callback");
    let router = crate::api::router(state);
    let web = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/e2e/oidc.mjs"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"origin":origin,"issuer":provider.issuer})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    web.abort();
    provider_server.abort();
    assert!(
        output.status.success(),
        "OIDC browser acceptance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
