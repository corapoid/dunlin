use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn capabilities() -> Value {
    json!({"protocol_version":1,"version":"controlled-worker-1","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":2})
}
async fn worker_request(
    origin: &str,
    path: &str,
    body: Value,
    credential: Option<&str>,
) -> (u16, Value) {
    let address = origin
        .strip_prefix("http://")
        .unwrap()
        .parse::<std::net::SocketAddr>()
        .unwrap();
    assert!(address.ip().is_loopback());
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    let mut request = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", address.to_string())
        .header("Connection", "close")
        .header("Content-Type", "application/json");
    if let Some(credential) = credential {
        request = request.header("Authorization", format!("Bearer {credential}"));
    }
    let response = sender
        .send_request(
            request
                .body(Full::new(axum::body::Bytes::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Some(data) = frame.unwrap().data_ref() {
            assert!(bytes.len() + data.len() <= 64 * 1024);
            bytes.extend_from_slice(data);
        }
    }
    task.abort();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run browser sandbox suite"]
async fn production_browser_acceptance_hosts() {
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("production web build required");
    let store = Store::open_in_memory().unwrap();
    let owner = store.create_user("host-browser-owner", "admin").unwrap();
    let viewer = store.create_user("host-browser-viewer", "member").unwrap();
    let bootstrap = store
        .create_user("host-browser-bootstrap", "admin")
        .unwrap();
    let workspace = store
        .create_workspace(&bootstrap, "Host registry fixture")
        .unwrap();
    store
        .set_team_member(
            &bootstrap.id,
            &workspace.id,
            &owner.id,
            WorkspaceRole::Admin,
            true,
            None,
        )
        .unwrap();
    store
        .set_team_member(
            &bootstrap.id,
            &workspace.id,
            &viewer.id,
            WorkspaceRole::Viewer,
            true,
            None,
        )
        .unwrap();
    let owner_code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let viewer_code = store.create_pairing_code(&viewer.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            web_root: Some(root.into()),
            db: "/nonexistent/host-browser-fixture/relay.db".into(),
            ..Default::default()
        },
    );
    let (origin, _server) = super::live_transport::start(&state).await;
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/e2e/hosts.mjs"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let fixture = json!({"origin":origin,"ownerCode":owner_code,"viewerCode":viewer_code,"workspace":workspace.id});
    let result=tokio::time::timeout(Duration::from_secs(90),async {
        input.write_all(format!("{fixture}\n").as_bytes()).await.unwrap();
        let mut credential:Option<Value>=None;let mut log=String::new();
        while let Some(line)=output.next_line().await.unwrap() {
            if !line.starts_with('{') { log.push_str(&line);log.push('\n');continue; }
            let request:Value=serde_json::from_str(&line).unwrap();let command=request["command"].as_str().unwrap();
            match command {
                "claim" => {
                    let (status,accepted)=worker_request(&origin,"/v1/worker/enroll",json!({"ticket":request["ticket"],"capabilities":capabilities()}),None).await;
                    assert_eq!(status,200);assert_eq!(accepted["host_id"],request["host_id"]);
                    assert!(accepted["credential"].as_str().unwrap().starts_with("dhw_"));credential=Some(accepted);
                }
                "report" | "verify-revoked" => {
                    let current=credential.as_ref().unwrap();assert_eq!(current["host_id"],request["host_id"]);
                    let (status,host)=worker_request(&origin,"/v1/worker/report",json!({"host_id":current["host_id"],"enrollment_epoch":current["enrollment_epoch"],"credential_revision":current["credential_revision"],"capabilities":capabilities()}),current["credential"].as_str()).await;
                    if command=="report" { assert_eq!(status,200);assert_eq!(host["host"]["execution_ready"],false);assert_eq!(host["host"]["network_verification"],"unverified"); }
                    else {
                        assert_eq!(status,401);
                        let (status,_)=worker_request(&origin,"/v1/worker/enroll",json!({"ticket":request["ticket"],"capabilities":capabilities()}),None).await;
                        assert_eq!(status,401);
                    }
                }
                "revoke-manager" => {
                    state.store.lock().unwrap().set_team_member(&bootstrap.id,&workspace.id,&owner.id,WorkspaceRole::Viewer,true,Some(1)).unwrap();
                }
                _=>panic!("Unknown browser fixture command"),
            }
            input.write_all(format!("{}\n",json!({"ok":command})).as_bytes()).await.unwrap();
        }
        log
    }).await.expect("host browser deadline");
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "Host browser acceptance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        result.contains("PASS Chromium host registry:"),
        "Browser success marker required"
    );
    println!("{result}");
}
