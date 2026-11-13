use super::*;

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// Abort on unwind too: no fixture server survives a failed assertion.
pub(super) struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn start(state: &AppState) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = super::super::super::router(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (origin, Server(task))
}

async fn connect(
    origin: &str,
    cookie: &str,
    request_origin: &str,
) -> Result<Client, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("{}/v1/web/live", origin.replacen("http://", "ws://", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(header::ORIGIN, request_origin.parse().unwrap());
    request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    connect_async(request).await.map(|(client, _)| client)
}

async fn subscribe(client: &mut Client, project: Option<&str>) {
    client
        .send(ClientMessage::Text(
            json!({"type":"subscribe", "protocol_version":1, "project_id":project})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

async fn next_event(client: &mut Client) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match client.next().await.unwrap().unwrap() {
                ClientMessage::Text(text) => return serde_json::from_str(&text).unwrap(),
                ClientMessage::Ping(bytes) => {
                    client.send(ClientMessage::Pong(bytes)).await.unwrap()
                }
                message => panic!("Unexpected frame: {message:?}"),
            }
        }
    })
    .await
    .expect("live event deadline")
}

#[tokio::test]
async fn real_websocket_push_is_scoped_and_revocation_closes_authority() {
    let (state, code, project) = fixture();
    let paired = browser_pair(&state, &code).await;
    let cookie = paired.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (origin, _server) = start(&state).await;
    let mut client = connect(&origin, &cookie, &origin).await.unwrap();
    subscribe(&mut client, Some(&project)).await;
    let initial = next_event(&mut client).await;
    assert_eq!(initial["type"], "snapshot");
    assert_eq!(initial["snapshot"]["sequence"], 1);
    assert_eq!(initial["snapshot"]["project_id"], project);
    assert_eq!(initial["snapshot"]["sessions"], json!([]));
    {
        let store = state.store.lock().unwrap();
        let user = store.find_user_by_name("member").unwrap().unwrap();
        store
            .create_session(
                "live-session",
                &user.id,
                &project,
                "Pushed session",
                "fixture",
            )
            .unwrap();
    }
    let updated = next_event(&mut client).await;
    assert_eq!(updated["snapshot"]["sequence"], 2);
    assert_eq!(updated["snapshot"]["sessions"][0]["id"], "live-session");

    // A hidden-scope mutation produces no data frame or sequence increment.
    state
        .store
        .lock()
        .unwrap()
        .create_project("private", "/fixture/private")
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), next_event(&mut client))
            .await
            .is_err()
    );
    subscribe(&mut client, Some(&project)).await;
    assert_eq!(next_event(&mut client).await["snapshot"]["sequence"], 3);

    // Reconnection receives authoritative state in a new sequence namespace.
    let mut second = connect(&origin, &cookie, &origin).await.unwrap();
    subscribe(&mut second, Some(&project)).await;
    let recovered = next_event(&mut second).await;
    assert_ne!(
        initial["snapshot"]["stream_id"],
        recovered["snapshot"]["stream_id"]
    );
    assert_eq!(recovered["snapshot"]["sequence"], 1);
    assert_eq!(recovered["snapshot"]["sessions"][0]["id"], "live-session");

    let mut request_headers = headers(origin.strip_prefix("http://").unwrap(), Some(&origin));
    request_headers.insert(header::COOKIE, cookie.parse().unwrap());
    logout(State(state.clone()), request_headers).await.unwrap();
    assert_eq!(
        next_event(&mut client).await["code"],
        "authorization_required"
    );
    assert!(connect(&origin, &cookie, &origin).await.is_err());
}

#[tokio::test]
async fn real_websocket_rejects_cross_origin_and_ungranted_subscription() {
    let (state, code, _) = fixture();
    let paired = browser_pair(&state, &code).await;
    let cookie = paired.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (origin, _server) = start(&state).await;
    assert!(
        connect(&origin, &cookie, "http://evil.example")
            .await
            .is_err()
    );
    assert!(
        connect(&origin, "dunlin_web_session=invalid", &origin)
            .await
            .is_err()
    );
    let mut client = connect(&origin, &cookie, &origin).await.unwrap();
    subscribe(&mut client, Some("ungranted")).await;
    assert_eq!(next_event(&mut client).await["code"], "catalog_unavailable");
}

#[tokio::test]
async fn static_asset_responses_enforce_headers_and_root_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("build");
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(
        root.join("index.html"),
        "<!doctype html><title>Dunlin</title>",
    )
    .unwrap();
    std::fs::write(root.join("assets/app.js"), "export {};").unwrap();
    std::fs::write(directory.path().join("outside.js"), "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        directory.path().join("outside.js"),
        root.join("assets/escape.js"),
    )
    .unwrap();
    let state = AppState::new(
        Store::open_in_memory().unwrap(),
        Config {
            web_root: Some(root),
            ..Config::default()
        },
    );
    let router = super::super::super::router(state);
    let request = |method: &str, path: &str| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8787")
            .body(Body::empty())
            .unwrap()
    };
    let response = router.clone().oneshot(request("GET", "/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert!(
        response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("connect-src 'self' ws://127.0.0.1:8787")
    );
    assert!(
        to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .starts_with(b"<!doctype html>")
    );
    let head = router
        .clone()
        .oneshot(request("HEAD", "/assets/app.js"))
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "10");
    assert!(to_bytes(head.into_body(), 4096).await.unwrap().is_empty());
    for path in ["/package.json", "/.env", "/v1/unknown", "/assets/escape.js"] {
        assert_eq!(
            router
                .clone()
                .oneshot(request("GET", path))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}
