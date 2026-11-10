use super::*;
use dunlin_vault::store::{MemoryStore, SecretStore};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn mock(cycles: usize) -> (String, thread::JoinHandle<()>) {
    mock_with_hook(cycles, None)
}

fn mock_with_hook(
    cycles: usize,
    hook: Option<Box<dyn Fn() + Send>>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for _ in 0..cycles * 4 {
            let start = Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(5) =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    _ => panic!("Missing Linear MCP request"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                headers.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "initialize" => json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}),
                "notifications/initialized" => Value::Null,
                "tools/list" => {
                    json!({"tools":[{"name":"list_issues","inputSchema":{"properties":{"label":{},"limit":{},"cursor":{},"team":{}}}}]})
                }
                "tools/call" => {
                    if let Some(hook) = &hook {
                        hook();
                    }
                    assert_eq!(request["params"]["arguments"]["label"], "dunlin");
                    json!({"structuredContent":{"issues":[{"id":"issue-id","identifier":"APP-1","title":"Implement this task","description":"Requirements","url":"https://linear.app/team/issue/APP-1/task","status":"Todo","labels":["dunlin"]}],"hasNextPage":false}})
                }
                _ => panic!("Unexpected MCP method"),
            };
            let status = if request["method"] == "notifications/initialized" {
                202
            } else {
                200
            };
            let body = if status == 202 {
                String::new()
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()
            };
            write!(socket,"HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    (endpoint, worker)
}

fn configure() -> Value {
    json!({"filter":{"label":"dunlin","team":null},"token":"fixture-linear-key"})
}

#[tokio::test]
async fn linear_integration_is_user_scoped_uses_mcp_and_never_returns_credentials() {
    let mut fixture = Fixture::new();
    let secrets = Arc::new(MemoryStore::new());
    let (endpoint, worker) = mock(2);
    fixture.state.linear = Arc::new(crate::linear::Service::testing(secrets.clone(), endpoint));
    assert_eq!(
        fixture.get_json("/v1/integrations/linear", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture
            .put("/v1/integrations/linear", None, configure())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, inbox) = fixture
        .put("/v1/integrations/linear", Some(&fixture.token), configure())
        .await;
    assert_eq!(status, StatusCode::OK, "{inbox}");
    assert_eq!(inbox["issues"][0]["identifier"], "APP-1");
    assert!(!inbox.to_string().contains("fixture-linear-key"));
    assert!(inbox.get("credential_id").is_none());
    let (user, device) = fixture
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&fixture.token)
        .unwrap()
        .unwrap();
    let credential = fixture
        .state
        .store
        .lock()
        .unwrap()
        .linear_connection(&user.id)
        .unwrap()
        .unwrap()
        .credential_id;
    assert_eq!(
        secrets.get(&credential).unwrap().as_str(),
        "fixture-linear-key"
    );
    let (_, other) = fixture
        .get_json("/v1/integrations/linear", Some(&fixture.ungranted_token))
        .await;
    assert!(other["filter"].is_null());
    assert_eq!(
        fixture
            .post(
                "/v1/integrations/linear/sync",
                Some(&fixture.ungranted_token),
                json!({})
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .post(
                "/v1/integrations/linear/sync",
                Some(&fixture.token),
                json!({})
            )
            .await
            .0,
        StatusCode::OK
    );
    worker.join().unwrap();
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        fixture
            .get_json("/v1/integrations/linear", Some(&fixture.token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn linear_failure_preserves_the_last_complete_snapshot_and_disconnect_removes_the_key() {
    let mut fixture = Fixture::new();
    let secrets = Arc::new(MemoryStore::new());
    let (endpoint, worker) = mock(1);
    fixture.state.linear = Arc::new(crate::linear::Service::testing(secrets.clone(), endpoint));
    assert_eq!(
        fixture
            .put("/v1/integrations/linear", Some(&fixture.token), configure())
            .await
            .0,
        StatusCode::OK
    );
    worker.join().unwrap();
    let (user, _) = fixture
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&fixture.token)
        .unwrap()
        .unwrap();
    let credential = fixture
        .state
        .store
        .lock()
        .unwrap()
        .linear_connection(&user.id)
        .unwrap()
        .unwrap()
        .credential_id;
    secrets.delete(&credential).unwrap();
    let (status, inbox) = fixture
        .post(
            "/v1/integrations/linear/sync",
            Some(&fixture.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(inbox["error"].as_str().unwrap().contains("keyring"));
    assert_eq!(inbox["issues"].as_array().unwrap().len(), 1);
    secrets.set(&credential, "fixture-linear-key").unwrap();
    assert_eq!(
        fixture
            .delete("/v1/integrations/linear", Some(&fixture.token))
            .await,
        StatusCode::OK
    );
    assert!(secrets.get(&credential).is_err());
    assert!(
        fixture
            .state
            .store
            .lock()
            .unwrap()
            .linear_connection(&user.id)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn linear_configuration_does_not_accept_arbitrary_endpoints_users_or_invalid_filters() {
    let fixture = Fixture::new();
    for body in [
        json!({"filter":{"label":"","team":null},"token":"x"}),
        json!({"filter":{"label":"dunlin","team":null},"token":"bad\nheader"}),
    ] {
        assert_eq!(
            fixture
                .put("/v1/integrations/linear", Some(&fixture.token), body)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut body = configure();
    body["endpoint"] = json!("https://127.0.0.1/private");
    assert_eq!(
        fixture
            .put("/v1/integrations/linear", Some(&fixture.token), body)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn linear_background_sync_does_not_require_a_live_desktop_device() {
    let mut fixture = Fixture::new();
    let (endpoint, worker) = mock(2);
    fixture.state.linear = Arc::new(crate::linear::Service::testing(
        Arc::new(MemoryStore::new()),
        endpoint,
    ));
    assert_eq!(
        fixture
            .put("/v1/integrations/linear", Some(&fixture.token), configure())
            .await
            .0,
        StatusCode::OK
    );
    let user = {
        let store = fixture.state.store.lock().unwrap();
        let (user, device) = store.authenticate(&fixture.token).unwrap().unwrap();
        store.revoke_device(&device.id).unwrap();
        store.linear_make_due();
        user
    };
    crate::linear::background_once(fixture.state.clone()).await;
    worker.join().unwrap(); // A complete second MCP cycle, without a desktop request.
    let store = fixture.state.store.lock().unwrap();
    let inbox = store.linear_connection(&user.id).unwrap().unwrap().inbox;
    assert!(inbox.error.is_none());
    assert_eq!(inbox.issues.len(), 1);
    assert!(store.linear_due().unwrap().is_empty());
}

#[tokio::test]
async fn revocation_during_mcp_work_prevents_saving_the_connection() {
    let mut fixture = Fixture::new();
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (endpoint, worker) = mock_with_hook(
        1,
        Some(Box::new(move || {
            arrived_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        })),
    );
    fixture.state.linear = Arc::new(crate::linear::Service::testing(
        Arc::new(MemoryStore::new()),
        endpoint,
    ));
    let (user, device) = fixture
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&fixture.token)
        .unwrap()
        .unwrap();
    let configure = fixture.put("/v1/integrations/linear", Some(&fixture.token), configure());
    let revoke = async {
        tokio::task::spawn_blocking(move || {
            arrived_rx.recv_timeout(Duration::from_secs(5)).unwrap()
        })
        .await
        .unwrap();
        fixture
            .state
            .store
            .lock()
            .unwrap()
            .revoke_device(&device.id)
            .unwrap();
        release_tx.send(()).unwrap();
    };
    let (result, _) = tokio::join!(configure, revoke);
    assert_eq!(result.0, StatusCode::UNAUTHORIZED);
    worker.join().unwrap();
    assert!(
        fixture
            .state
            .store
            .lock()
            .unwrap()
            .linear_connection(&user.id)
            .unwrap()
            .is_none()
    );
}
