//! Complete synthetic private-daemon/TLS/WSS fixture. No real tailnet or daemon.
#[cfg(target_os = "linux")]
mod broker_client;
mod broker_transport;
#[cfg(target_os = "linux")]
mod execution_vertical;
#[cfg(target_os = "linux")]
mod mcp_provider;
#[cfg(target_os = "linux")]
mod session_process;
#[cfg(target_os = "linux")]
mod session_vertical;
use crate::{api::AppState, config::Config, store::Store, tailscale::Transport};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UnixListener},
    task::{JoinHandle, JoinSet},
};
use zeroize::Zeroizing;
const CA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/worker-channel/test-ca.der"
));
const CERT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/worker-channel/test-server.der"
));
const KEY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/worker-channel/test-only-server-key.der"
));
fn roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(CA.to_vec()))
        .unwrap();
    roots
}
struct Task(JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn tls_proxy(target: SocketAddr) -> (SocketAddr, Task) {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![rustls::pki_types::CertificateDer::from(CERT.to_vec())],
        rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
            KEY.to_vec(),
        )),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{let(stream,_)=accepted.unwrap();let acceptor=acceptor.clone();tasks.spawn(async move{if let Ok(mut tls)=acceptor.accept(stream).await{let mut backend=TcpStream::connect(target).await.unwrap();let _=tokio::io::copy_bidirectional(&mut tls,&mut backend).await;}});},
                _=tasks.join_next(),if !tasks.is_empty()=>{}
            }
        }
    });
    (addr, Task(task))
}
#[derive(Clone)]
struct DaemonState {
    node: &'static str,
    hostname: &'static str,
    listen: SocketAddr,
    peer_node: &'static str,
    peer_hostname: &'static str,
    peer_ip: &'static str,
    peer_tls: SocketAddr,
    funnel: bool,
    expired: bool,
}
fn status(state: &DaemonState) -> Value {
    json!({"Version":"1.92.0-controlled-fixture","BackendState":"Running","Self":{"ID":state.node,"DNSName":format!("{}.",state.hostname),"Online":true},"Peer":{"nodekey:controlled":{"ID":state.peer_node,"DNSName":format!("{}.",state.peer_hostname),"TailscaleIPs":[state.peer_ip],"Online":true,"InNetworkMap":true,"Expired":state.expired}}})
}
fn serve(state: &DaemonState) -> Value {
    json!({"TCP":{"443":{"HTTPS":true}},"Web":{format!("{}:443",state.hostname):{"Handlers":{"/":{"Proxy":format!("http://{}",state.listen)}}}},"AllowFunnel":{format!("{}:443",state.hostname):state.funnel}})
}
async fn daemon(path: PathBuf, state: Arc<Mutex<DaemonState>>) -> Task {
    let listener = UnixListener::bind(path).unwrap();
    Task(tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{let(mut stream,_)=accepted.unwrap();let state=state.clone();tasks.spawn(async move{
                    let mut bytes=Vec::new();loop{let mut b=[0;512];let n=stream.read(&mut b).await.unwrap();if n==0{return;}bytes.extend_from_slice(&b[..n]);if bytes.windows(4).any(|w|w==b"\r\n\r\n"){break;}assert!(bytes.len()<=16*1024);}
                    let request=String::from_utf8(bytes).unwrap();let lower=request.to_ascii_lowercase();assert!(lower.contains("host: local-tailscaled.sock\r\n"));assert!(!lower.contains("authorization:"));assert!(!lower.contains("origin:"));let snapshot=state.lock().unwrap().clone();
                    if request.starts_with("POST /localapi/v0/dial HTTP/1.1\r\n"){
                        assert!(lower.contains(&format!("dial-host: {}\r\n",snapshot.peer_ip)));assert!(lower.contains("dial-port: 443\r\n"));assert!(lower.contains("dial-network: tcp\r\n"));
                        let mut backend=TcpStream::connect(snapshot.peer_tls).await.unwrap();stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: ts-dial\r\nConnection: upgrade\r\n\r\n").await.unwrap();let _=tokio::io::copy_bidirectional(&mut stream,&mut backend).await;
                    }else{
                        let body=if request.starts_with("GET /localapi/v0/serve-config HTTP/1.1\r\n"){serve(&snapshot)}else{assert!(request.starts_with("GET /localapi/v0/status"));status(&snapshot)}.to_string();
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                    }
                });},_=tasks.join_next(),if !tasks.is_empty()=>{}
            }
        }
    }))
}
struct Fixture {
    _dir: tempfile::TempDir,
    _tasks: Vec<Task>,
    state: AppState,
    owner: String,
    workspace: String,
    host: String,
    client: crate::worker_client::WorkerConfig,
    credential: String,
    transport: Transport,
    worker_daemon: Arc<Mutex<DaemonState>>,
}
async fn fixture() -> Fixture {
    fixture_with_execution(false).await
}
async fn fixture_with_execution(execution_enabled: bool) -> Fixture {
    fixture_with_modes(execution_enabled, false).await
}
async fn fixture_with_modes(execution_enabled: bool, persistent_sessions: bool) -> Fixture {
    fixture_with_storage(execution_enabled, persistent_sessions, false).await
}
async fn fixture_with_storage(
    execution_enabled: bool,
    persistent_sessions: bool,
    persisted: bool,
) -> Fixture {
    fixture_with_broker_transport(execution_enabled, persistent_sessions, persisted, false).await
}
async fn fixture_with_broker_transport(
    execution_enabled: bool,
    persistent_sessions: bool,
    persisted: bool,
    mcp_broker: bool,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let controller_socket = dir.path().join("controller.sock");
    let worker_socket = dir.path().join("worker.sock");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let controller_listen = listener.local_addr().unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let worker_listen = reserved.local_addr().unwrap();
    drop(reserved);
    let (controller_tls, controller_proxy) = tls_proxy(controller_listen).await;
    let (worker_tls, worker_proxy) = tls_proxy(worker_listen).await;
    let controller_daemon = Arc::new(Mutex::new(DaemonState {
        node: "n-controller",
        hostname: "controller.tail123.ts.net",
        listen: controller_listen,
        peer_node: "n-worker",
        peer_hostname: "worker.tail123.ts.net",
        peer_ip: "100.64.0.2",
        peer_tls: worker_tls,
        funnel: false,
        expired: false,
    }));
    let worker_daemon = Arc::new(Mutex::new(DaemonState {
        node: "n-worker",
        hostname: "worker.tail123.ts.net",
        listen: worker_listen,
        peer_node: "n-controller",
        peer_hostname: "controller.tail123.ts.net",
        peer_ip: "100.64.0.1",
        peer_tls: controller_tls,
        funnel: false,
        expired: false,
    }));
    let controller_api = daemon(controller_socket.clone(), controller_daemon).await;
    let worker_api = daemon(worker_socket.clone(), worker_daemon.clone()).await;
    let store = if persisted {
        Store::open(dir.path().join("controller.db")).unwrap()
    } else {
        Store::open_in_memory().unwrap()
    };
    let owner = store.create_user("channel-owner", "admin").unwrap();
    let w = store
        .create_workspace(&owner, "Worker channel fixture")
        .unwrap();
    let definition=serde_json::from_value(json!({"name":"Controlled worker","node_id":"n-worker","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap();
    let host = store.create_host(&owner.id, &w.id, definition).unwrap();
    let caps=serde_json::from_value(json!({"protocol_version":1,"version":"controlled-worker","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"none","max_parallel":1})).unwrap();
    let credential = store.enroll_host(&host.enrollment.ticket, caps).unwrap();
    store
        .approve_host(&owner.id, &w.id, &host.host.id, 2)
        .unwrap();
    let caps = store
        .host(&owner.id, &w.id, &host.host.id)
        .unwrap()
        .unwrap()
        .capabilities
        .unwrap();
    let mut state = AppState::new(
        store,
        Config {
            listen: controller_listen.to_string(),
            web_root: std::env::var_os("DUNLIN_WEB_E2E_ROOT").map(PathBuf::from),
            db: dir.path().join("controller.db"),
            worker_controller: Some(crate::worker_channel::ControllerConfig {
                execution_enabled,
                persistent_sessions,
                mcp_broker,
                socket: controller_socket.clone(),
                node_id: "n-controller".into(),
                serve_origin: "https://controller.tail123.ts.net".into(),
            }),
            ..Default::default()
        },
    );
    Arc::get_mut(&mut state.worker_channels)
        .unwrap()
        .test_transport = Some(
        Transport::new(
            controller_socket,
            "n-controller".into(),
            "https://controller.tail123.ts.net".into(),
            controller_listen,
        )
        .unwrap()
        .with_test_roots(roots()),
    );
    let app = crate::api::router(state.clone());
    let server = Task(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let transport = Transport::new(
        worker_socket.clone(),
        "n-worker".into(),
        "https://worker.tail123.ts.net".into(),
        worker_listen,
    )
    .unwrap()
    .with_test_roots(roots());
    let client = crate::worker_client::WorkerConfig {
        exports_state_dir: None,
        execution: None,
        network: crate::worker_client::NetworkConfig {
            socket: worker_socket,
            node_id: "n-worker".into(),
            serve_origin: "https://worker.tail123.ts.net".into(),
            listen: worker_listen,
        },
        controller: crate::worker_client::ControllerConfig {
            node_id: "n-controller".into(),
            serve_origin: "https://controller.tail123.ts.net".into(),
        },
        host_id: credential.host_id.clone(),
        workspace_id: credential.workspace_id.clone(),
        enrollment_epoch: credential.enrollment_epoch,
        credential_revision: credential.credential_revision,
        capabilities: caps,
    };
    Fixture {
        _dir: dir,
        _tasks: vec![
            controller_proxy,
            worker_proxy,
            controller_api,
            worker_api,
            server,
        ],
        state,
        owner: owner.id,
        workspace: w.id,
        host: credential.host_id,
        client,
        credential: credential.credential,
        transport,
        worker_daemon,
    }
}
async fn await_heartbeats(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = f
                .state
                .store
                .lock()
                .unwrap()
                .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
                .unwrap();
            if current.is_some_and(|c| c.phase == "active" && c.lease.last_sequence >= 1) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("real WSS heartbeat deadline");
}
async fn await_offline(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.state
                .store
                .lock()
                .unwrap()
                .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
                .unwrap()
                .is_some_and(|c| c.phase == "offline")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("channel exit must persist offline");
}
#[tokio::test]
async fn real_private_dial_tls_wss_nonce_callback_and_heartbeat_revoke_without_execution() {
    let f = fixture().await;
    let client = crate::worker_client::run_with_transport(
        f.client.clone(),
        Zeroizing::new(f.credential.clone()),
        f.transport.clone(),
    );
    let task = tokio::spawn(client);
    await_heartbeats(&f).await;
    {
        let store = f.state.store.lock().unwrap();
        let host = store
            .host(&f.owner, &f.workspace, &f.host)
            .unwrap()
            .unwrap();
        assert!(!host.execution_ready);
        assert_eq!(host.network_verification, "unverified");
        let observation = store
            .host_health_observation(&f.owner, &f.workspace, &f.host, &f.state.worker_channels)
            .unwrap();
        let connection = observation.connection.unwrap();
        assert_eq!(connection.protocol_version, 2);
        assert!(connection.heartbeat_sequence >= 1);
        assert!(connection.lease_remaining_ms > 0);
        assert_eq!(observation.runtime.session_reservations, 0);
        assert_eq!(observation.runtime.one_shot_reservations, 0);
        store
            .revoke_host(&f.owner, &f.workspace, &f.host, host.revision)
            .unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    await_offline(&f).await;
    let record = f
        .state
        .store
        .lock()
        .unwrap()
        .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
        .unwrap()
        .unwrap();
    assert_eq!(record.phase, "offline");
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .host_health_observation(&f.owner, &f.workspace, &f.host, &f.state.worker_channels)
            .unwrap()
            .connection
            .is_none()
    );
}
#[tokio::test]
async fn loss_of_private_serve_evidence_closes_an_already_authenticated_channel() {
    let f = fixture().await;
    let task = tokio::spawn(crate::worker_client::run_with_transport(
        f.client.clone(),
        Zeroizing::new(f.credential.clone()),
        f.transport.clone(),
    ));
    await_heartbeats(&f).await;
    f.worker_daemon.lock().unwrap().funnel = true;
    assert!(
        tokio::time::timeout(Duration::from_secs(8), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    await_offline(&f).await;
    let record = f
        .state
        .store
        .lock()
        .unwrap()
        .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
        .unwrap()
        .unwrap();
    assert_eq!(record.phase, "offline");
}
#[tokio::test]
async fn public_roots_reject_the_fixture_ca_before_any_channel_lease_is_minted() {
    let f = fixture().await;
    let plain = Transport::new(
        f.client.network.socket.clone(),
        f.client.network.node_id.clone(),
        f.client.network.serve_origin.clone(),
        f.client.network.listen,
    )
    .unwrap();
    assert_eq!(
        plain
            .connect("n-controller", "https://controller.tail123.ts.net")
            .await
            .err(),
        Some("peer_tls_verification_failed")
    );
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn trusted_test_ca_does_not_disable_server_name_verification() {
    let f = fixture().await;
    f.worker_daemon.lock().unwrap().peer_hostname = "wrong.tail123.ts.net";
    assert_eq!(
        f.transport
            .connect("n-controller", "https://wrong.tail123.ts.net")
            .await
            .err(),
        Some("peer_tls_verification_failed")
    );
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn credential_and_ready_without_the_bound_reverse_nonce_cannot_mint_a_lease() {
    manual_v1_channel(true).await;
}
#[tokio::test]
async fn legacy_v1_proved_channel_heartbeats_but_cannot_describe_exports() {
    manual_v1_channel(false).await;
}
async fn manual_v1_channel(tamper: bool) {
    use dunlin_relay_protocol::worker::{
        self, ChannelProofRequest, ChannelProofResponse, ControllerMessage, WorkerMessage,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let f = fixture().await;
    let host = f.host.clone();
    let proof = axum::Router::new().route(
        "/v1/worker/channel-proof",
        axum::routing::post(
            move |axum::Json(request): axum::Json<ChannelProofRequest>| {
                let host = host.clone();
                async move {
                    axum::Json(ChannelProofResponse {
                        channel_id: request.channel_id,
                        controller_boot_id: request.controller_boot_id,
                        worker_boot_id: "unbound-worker-boot".into(),
                        nonce: {
                            let mut nonce = request.nonce;
                            let changed = if nonce.starts_with('0') { "1" } else { "0" };
                            if tamper {
                                nonce.replace_range(..1, changed);
                            }
                            nonce
                        },
                        host_id: host,
                        enrollment_epoch: 1,
                        credential_revision: 1,
                    })
                }
            },
        ),
    );
    let listener = TcpListener::bind(f.client.network.listen).await.unwrap();
    let _server = Task(tokio::spawn(async move {
        axum::serve(listener, proof).await.unwrap();
    }));
    let stream = f
        .transport
        .connect("n-controller", "https://controller.tail123.ts.net")
        .await
        .unwrap();
    let mut request = "wss://controller.tail123.ts.net/v1/worker/channel"
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", f.credential).parse().unwrap(),
    );
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        worker::WORKER_SUBPROTOCOL.parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap();
    let hello = WorkerMessage::Hello {
        protocol_version: 1,
        host_id: f.host.clone(),
        workspace_id: f.workspace.clone(),
        enrollment_epoch: 1,
        credential_revision: 1,
        worker_boot_id: "unbound-worker-boot".into(),
        capabilities: f.client.capabilities.clone(),
    };
    socket
        .send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
        .await
        .unwrap();
    let frame = socket.next().await.unwrap().unwrap();
    let ControllerMessage::Challenge {
        channel_id,
        controller_boot_id,
        nonce,
        ..
    } = worker::decode_controller_frame(frame.to_text().unwrap()).unwrap()
    else {
        panic!("challenge required")
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::Ready {
                channel_id,
                controller_boot_id,
                worker_boot_id: "unbound-worker-boot".into(),
                nonce,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response = worker::decode_controller_frame(frame.to_text().unwrap()).unwrap();
    if tamper {
        assert!(matches!(
            response,
            ControllerMessage::Error {
                code: worker::WorkerErrorCode::PeerVerificationFailed
            }
        ));
        assert!(
            f.state
                .store
                .lock()
                .unwrap()
                .worker_channel_diagnostic(&f.owner, &f.workspace, &f.host)
                .unwrap()
                .is_none()
        );
    } else {
        let ControllerMessage::Accepted {
            channel_id,
            lease_epoch,
            ..
        } = response
        else {
            panic!("v1 acceptance required")
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&WorkerMessage::Heartbeat {
                    channel_id: channel_id.clone(),
                    lease_epoch,
                    sequence: 1,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let ack = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            worker::decode_controller_frame(ack.to_text().unwrap()).unwrap(),
            ControllerMessage::HeartbeatAck { sequence: 1, .. }
        ));
        {
            let store = f.state.store.lock().unwrap();
            let observation = store
                .host_health_observation(&f.owner, &f.workspace, &f.host, &f.state.worker_channels)
                .unwrap();
            let connection = observation.connection.unwrap();
            assert_eq!(connection.protocol_version, 1);
            assert_eq!(connection.heartbeat_sequence, 1);
        }

        assert!(matches!(
            f.state
                .worker_channels
                .describe_export(
                    &f.state,
                    &f.host,
                    "export-test",
                    worker::ExportPrincipal {
                        kind: worker::PrincipalKind::Human,
                        id: f.owner.clone()
                    },
                    Some(1)
                )
                .await,
            Err(crate::worker_channel::DescribeError::Unavailable)
        ));
        socket.close(None).await.unwrap();
    }
}

/// Full production worker and controller, real TLS/WSS and reverse callback;
/// only the LocalAPI daemon and certificate trust roots are test adapters.
#[tokio::test]
async fn remote_export_metadata_binds_through_real_wss_and_local_grant_revoke_is_observed() {
    use crate::worker_exports::{
        CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, LinkedRoots,
        PrincipalKind, WorkerStore,
    };
    use dunlin_relay_protocol::worker as wire;
    let mut f = fixture().await;
    let main = f._dir.path().join("PRIVATE_WORKER_MAIN");
    std::fs::create_dir(&main).unwrap();
    fixture_git(&main, &["init", "-q"]);
    fixture_git(&main, &["commit", "--allow-empty", "-qm", "fixture"]);
    let root = f._dir.path().join("PRIVATE_WORKER_CHECKOUT");
    fixture_git(
        &main,
        &[
            "worktree",
            "add",
            "-qb",
            "fixture-linked",
            root.to_str().unwrap(),
        ],
    );
    let pointer = std::fs::read_to_string(root.join(".git")).unwrap();
    let git_dir = PathBuf::from(pointer.trim().strip_prefix("gitdir: ").unwrap());
    let common = main.join(".git");
    let linked = LinkedRoots {
        git_dir,
        common_dir: common.clone(),
    };
    let directory = f._dir.path().join("worker-private-state");
    let scope = ExportScope {
        controller_node_id: "n-controller".into(),
        worker_node_id: "n-worker".into(),
        workspace_id: f.workspace.clone(),
        host_id: f.host.clone(),
        enrollment_epoch: f.client.enrollment_epoch,
        credential_revision: f.client.credential_revision,
    };
    let worker = WorkerStore::open(&directory, scope.clone()).unwrap();
    let checkout = CapturedCheckout::capture(&root, &"a".repeat(64), Some(&linked)).unwrap();
    let export = worker
        .register_captured(&scope, "Approved checkout", &checkout)
        .unwrap();
    let principal = ExportPrincipal {
        kind: PrincipalKind::Human,
        id: f.owner.clone(),
    };
    worker
        .set_grant(
            &scope,
            &export.id,
            &principal,
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(worker);
    let worker = WorkerStore::open_existing(&directory, scope.clone()).unwrap();
    f.client.exports_state_dir = Some(directory);
    let client = crate::worker_client::run_with_transport(
        f.client.clone(),
        Zeroizing::new(f.credential.clone()),
        f.transport.clone(),
    );
    let _client = Task(tokio::spawn(async move {
        let _ = client.await;
    }));
    await_heartbeats(&f).await;
    let requested = wire::ExportPrincipal {
        kind: wire::PrincipalKind::Human,
        id: f.owner.clone(),
    };
    let proof = f
        .state
        .worker_channels
        .describe_export(&f.state, &f.host, &export.id, requested.clone(), Some(1))
        .await
        .unwrap();
    assert!(proof.is_fresh());
    assert_eq!(proof.descriptor().export.id, export.id);
    assert_eq!(proof.descriptor().grant.principal, requested);
    let json = serde_json::to_string(proof.descriptor()).unwrap();
    assert!(!json.contains("PRIVATE_WORKER_CHECKOUT"));
    assert!(!json.contains(root.to_str().unwrap()));
    let project = {
        let store = f.state.store.lock().unwrap();
        store
            .create_team_project(&f.owner, &f.workspace, "Remote target")
            .unwrap()
    };
    let saved = {
        let store = f.state.store.lock().unwrap();
        let authority = store
            .capture_remote_binding_authority(&f.owner, &f.workspace, &project.id)
            .unwrap();
        store
            .set_remote_binding(
                &f.owner,
                &f.workspace,
                &project.id,
                None,
                None,
                &f.host,
                &export.id,
                1,
                &proof,
                &f.state.worker_channels.boot_id,
                "n-controller",
                fixture_now_ms(),
                &authority,
            )
            .unwrap()
    };
    assert_eq!(saved.export_id, export.id);
    assert_eq!(saved.repo_kind, wire::RepoKind::LinkedWorktree);
    assert!(saved.active);
    let (member, member_device, cookie) = {
        use dunlin_domain::collaboration::{Permission, WorkspaceRole};
        let store = f.state.store.lock().unwrap();
        let member = store.create_user("actual-target-caller", "member").unwrap();
        store
            .set_team_member(
                &f.owner,
                &f.workspace,
                &member.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &f.owner,
                &f.workspace,
                &project.id,
                &member.id,
                [Permission::ViewProject, Permission::CreateSession].into(),
                None,
            )
            .unwrap();
        let (device, _) = store.create_device(&member.id, "target-resolver").unwrap();
        let cookie = format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        );
        (member.id, device.id, cookie)
    };
    let member_principal = ExportPrincipal {
        kind: PrincipalKind::Human,
        id: member.clone(),
    };
    worker
        .set_grant(
            &scope,
            &export.id,
            &member_principal,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    worker
        .set_grant(&scope, &export.id, &member_principal, &[], false, Some(1))
        .unwrap();
    worker
        .set_grant(
            &scope,
            &export.id,
            &member_principal,
            &[ExportPermission::Read],
            true,
            Some(2),
        )
        .unwrap();
    let body = json!({"binding":{"id":saved.id,"revision":saved.revision}});
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/remote-bindings/resolve",
        f.workspace, project.id
    );
    let (status, resolved) = fixture_http(&f.state, &cookie, &path, body.clone()).await;
    assert_eq!(status, 200, "{resolved}");
    assert_eq!(resolved["principal"]["id"], member);
    assert_eq!(resolved["grant_revision"], 3);
    assert_eq!(resolved["grant_permissions"], json!(["read"]));
    assert_eq!(resolved["repo_kind"], "linked_worktree");
    assert_eq!(resolved["execution_ready"], false);
    assert!(!resolved.to_string().contains(root.to_str().unwrap()));
    worker
        .set_grant(&scope, &export.id, &member_principal, &[], false, Some(3))
        .unwrap();
    assert_eq!(
        fixture_http(&f.state, &cookie, &path, body.clone()).await.0,
        403
    );
    worker
        .set_grant(
            &scope,
            &export.id,
            &member_principal,
            &[ExportPermission::Read],
            true,
            Some(4),
        )
        .unwrap();
    assert_eq!(
        fixture_http(&f.state, &cookie, &path, body.clone()).await.0,
        200
    );
    // Hold the worker catalogue so the response cannot finish before the
    // controller device is revoked after its first authorization cut.
    let lock = rusqlite::Connection::open(
        f.client
            .exports_state_dir
            .as_ref()
            .unwrap()
            .join("exports.sqlite3"),
    )
    .unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let state = f.state.clone();
    let pending_cookie = cookie.clone();
    let pending_path = path.clone();
    let pending_body = body.clone();
    let pending = tokio::spawn(async move {
        fixture_http(&state, &pending_cookie, &pending_path, pending_body).await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.state.worker_channels.pending_exports_for_test(&f.host) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Resolver must pass first auth and queue a descriptor");
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&member_device)
        .unwrap();
    lock.execute_batch("COMMIT;").unwrap();
    assert_eq!(pending.await.unwrap().0, 401);
    let cookie = {
        let store = f.state.store.lock().unwrap();
        let (device, _) = store.create_device(&member, "new-target-browser").unwrap();
        format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        )
    };
    assert_eq!(
        fixture_http(&f.state, &cookie, &path, body.clone()).await.0,
        200
    );
    std::fs::rename(common.join("objects"), common.join("previous-objects")).unwrap();
    std::fs::create_dir(common.join("objects")).unwrap();
    assert_eq!(fixture_http(&f.state, &cookie, &path, body).await.0, 409);
    worker
        .set_grant(&scope, &export.id, &principal, &[], false, Some(1))
        .unwrap();
    assert!(matches!(
        f.state
            .worker_channels
            .describe_export(&f.state, &f.host, &export.id, requested, Some(1))
            .await,
        Err(crate::worker_channel::DescribeError::Forbidden)
    ));
    // The persisted record is a prior observation, not a live claim or a grant.
    let record = f
        .state
        .store
        .lock()
        .unwrap()
        .remote_bindings(&f.owner, &f.workspace, &project.id)
        .unwrap();
    assert_eq!(record.len(), 1);
    assert_eq!(record[0].grant_revision, 1);
    assert!(
        !f.state
            .store
            .lock()
            .unwrap()
            .host(&f.owner, &f.workspace, &f.host)
            .unwrap()
            .unwrap()
            .execution_ready
    );
}
fn fixture_git(directory: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(directory)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn fixture_http(state: &AppState, cookie: &str, path: &str, body: Value) -> (u16, Value) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let response = crate::api::router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("Host", state.config.listen.as_str())
                .header("Origin", format!("http://{}", state.config.listen))
                .header("Cookie", cookie)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn fixture_now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run browser sandbox suite"]
async fn production_browser_acceptance_remote_bindings() {
    use crate::worker_exports::{
        CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind,
        WorkerStore,
    };
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};
    use tokio::io::{AsyncBufReadExt, BufReader};
    assert!(std::env::var_os("DUNLIN_WEB_E2E_ROOT").is_some());
    let mut f = fixture().await;
    let (project, owner_code, viewer_code, bootstrap) = {
        let store = f.state.store.lock().unwrap();
        let project = store
            .create_team_project(&f.owner, &f.workspace, "Remote browser target")
            .unwrap();
        let viewer = store
            .create_user("remote-browser-viewer", "member")
            .unwrap();
        let bootstrap = store
            .create_user("remote-browser-bootstrap", "admin")
            .unwrap();
        store
            .set_team_member(
                &f.owner,
                &f.workspace,
                &viewer.id,
                WorkspaceRole::Viewer,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_member(
                &f.owner,
                &f.workspace,
                &bootstrap.id,
                WorkspaceRole::Owner,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &f.owner,
                &f.workspace,
                &project.id,
                &viewer.id,
                [Permission::ViewProject].into(),
                None,
            )
            .unwrap();
        let owner_code = store.create_pairing_code(&f.owner, 900).unwrap().0;
        let viewer_code = store.create_pairing_code(&viewer.id, 900).unwrap().0;
        (project, owner_code, viewer_code, bootstrap.id)
    };
    let root = f._dir.path().join("PRIVATE_REMOTE_BROWSER_CHECKOUT");
    std::fs::create_dir(&root).unwrap();
    let directory = f._dir.path().join("browser-worker-state");
    let scope = ExportScope {
        controller_node_id: "n-controller".into(),
        worker_node_id: "n-worker".into(),
        workspace_id: f.workspace.clone(),
        host_id: f.host.clone(),
        enrollment_epoch: f.client.enrollment_epoch,
        credential_revision: f.client.credential_revision,
    };
    let worker = WorkerStore::open(&directory, scope.clone()).unwrap();
    let checkout = CapturedCheckout::capture(&root, &"a".repeat(64), None).unwrap();
    let export = worker
        .register_captured(&scope, "Browser approved export", &checkout)
        .unwrap();
    let principal = ExportPrincipal {
        kind: PrincipalKind::Human,
        id: f.owner.clone(),
    };
    worker
        .set_grant(
            &scope,
            &export.id,
            &principal,
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    f.client.exports_state_dir = Some(directory);
    let client = crate::worker_client::run_with_transport(
        f.client.clone(),
        Zeroizing::new(f.credential.clone()),
        f.transport.clone(),
    );
    let client = Task(tokio::spawn(async move {
        let _ = client.await;
    }));
    await_heartbeats(&f).await;
    let origin = format!("http://{}", f.state.config.listen);
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/remote-bindings.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let settings = json!({"origin":origin,"ownerCode":owner_code,"viewerCode":viewer_code,"workspace":f.workspace,"project":project.id,"host":f.host,"exportId":export.id,"exportRevision":1});
    let log = tokio::time::timeout(Duration::from_secs(90), async {
        input
            .write_all(format!("{settings}\n").as_bytes())
            .await
            .unwrap();
        let mut log = String::new();
        while let Some(line) = output.next_line().await.unwrap() {
            if !line.starts_with('{') {
                log.push_str(&line);
                log.push('\n');
                continue;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            let command = request["command"].as_str().unwrap();
            match command {
                "revoke-worker-grant" => {
                    worker
                        .set_grant(&scope, &export.id, &principal, &[], false, Some(1))
                        .unwrap();
                }
                "disconnect-worker" => {
                    client.0.abort();
                    await_offline(&f).await;
                }
                "revoke-manager" => {
                    f.state
                        .store
                        .lock()
                        .unwrap()
                        .set_team_member(
                            &bootstrap,
                            &f.workspace,
                            &f.owner,
                            WorkspaceRole::Viewer,
                            true,
                            Some(1),
                        )
                        .unwrap();
                }
                _ => panic!("Unknown remote binding fixture command"),
            }
            input
                .write_all(format!("{}\n", json!({"ok":command})).as_bytes())
                .await
                .unwrap();
        }
        log
    })
    .await
    .expect("Remote bindings browser deadline");
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "Remote binding browser failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(log.contains("PASS Chromium remote bindings:"));
    println!("{log}");
}

/// Real private TLS/WSS and the actual Runtime connection guard, deliberately
/// no Execute/ACP process: this is transport/correlation evidence only.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_worker_execution_transport() {
    use crate::worker_channel::ExecutionTransportError;
    use dunlin_relay_protocol::worker::{
        ExecutionControllerMessage, ExecutionIdentity, ExecutionWorkerMessage,
    };
    fn early_exit(
        result: Result<Result<(), &'static str>, tokio::task::JoinError>,
        phase: &str,
    ) -> ! {
        // Only static bounded machine codes may appear in this diagnostic.
        // JoinError panic payloads and worker config/credentials are omitted.
        let code = match result {
            Ok(Err(code))
                if code.len() <= 128
                    && !code.is_empty()
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') =>
            {
                code
            }
            Ok(Err(_)) => "worker_error_redacted",
            Ok(Ok(())) => "unexpected_clean_worker_exit",
            Err(error) if error.is_cancelled() => "worker_task_cancelled",
            Err(_) => "worker_task_panicked_payload_redacted",
        };
        panic!("Execution transport worker exited before {phase}: {code}");
    }
    let mut f = fixture_with_execution(true).await;
    let state_dir = f._dir.path().join("worker-execution-state");
    let scope = crate::worker_exports::ExportScope {
        controller_node_id: f.client.controller.node_id.clone(),
        worker_node_id: f.client.network.node_id.clone(),
        workspace_id: f.client.workspace_id.clone(),
        host_id: f.client.host_id.clone(),
        enrollment_epoch: f.client.enrollment_epoch,
        credential_revision: f.client.credential_revision,
    };
    drop(crate::worker_exports::WorkerStore::open(&state_dir, scope).unwrap());
    f.client.exports_state_dir = Some(state_dir);
    // This opt-in fixture runs the real bwrap/pidfd preflight before Hello3.
    // Ordinary v2 fixtures keep their metadata-only sandbox capability.
    f.client.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    f.client.execution=Some(serde_json::from_value(json!({"enabled":true,"harnesses":{"opencode":{"program":"/usr/bin/python3","args":[]}}})).unwrap());
    let (sender, _events) = tokio::sync::mpsc::channel(8);
    let producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let config = f.client.clone();
    let credential = f.credential.clone();
    let dial = f.transport.clone();
    let mut worker = tokio::spawn(async move {
        crate::worker_client::run_with_transport(config, Zeroizing::new(credential), dial).await
    });
    tokio::select! {
        result = &mut worker => early_exit(result, "first heartbeat"),
        _ = await_heartbeats(&f) => {}
    }
    let transport_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(transport) = f.state.worker_channels.execution_transport(&f.host) {
                break transport;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let transport = tokio::select! {
        result = &mut worker => early_exit(result, "initial execution transport"),
        result = transport_wait => result.unwrap(),
    };
    {
        let store = f.state.store.lock().unwrap();
        assert!(
            transport
                .attest_in_transaction(
                    &store,
                    transport.identity(),
                    transport.lease(),
                    transport.fence()
                )
                .is_err()
        );
        store
            .remote_binding_transaction(false, |store| {
                transport.attest_in_transaction(
                    store,
                    transport.identity(),
                    transport.lease(),
                    transport.fence(),
                )
            })
            .unwrap();
    }
    let identity = ExecutionIdentity {
        command_id: "absent-command".into(),
        attempt_id: "absent-attempt".into(),
        fingerprint: "a".repeat(64),
    };
    let reply = transport
        .request(ExecutionControllerMessage::Reconcile {
            channel: transport.fence().clone(),
            identity: identity.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        reply,
        ExecutionWorkerMessage::Reconciled {
            channel: transport.fence().clone(),
            identity,
            record: None
        }
    );
    let old = transport.clone();
    drop(producer);
    assert!(
        tokio::time::timeout(Duration::from_secs(8), worker)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let (sender, _next_events) = tokio::sync::mpsc::channel(8);
    let _next_producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let config = f.client.clone();
    let credential = f.credential.clone();
    let dial = f.transport.clone();
    let mut worker = tokio::spawn(async move {
        crate::worker_client::run_with_transport(config, Zeroizing::new(credential), dial).await
    });
    let reconnect_wait = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(next) = f.state.worker_channels.execution_transport(&f.host)
                && next.fence() != old.fence()
            {
                break next;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let next = tokio::select! {
        result = &mut worker => early_exit(result, "reconnected execution transport"),
        result = reconnect_wait => result.unwrap(),
    };
    assert_ne!(next.fence().worker_boot_id, old.fence().worker_boot_id);
    let identity = ExecutionIdentity {
        command_id: "absent-command".into(),
        attempt_id: "absent-attempt".into(),
        fingerprint: "a".repeat(64),
    };
    assert_eq!(
        old.request(ExecutionControllerMessage::Reconcile {
            channel: old.fence().clone(),
            identity
        })
        .await
        .unwrap_err(),
        ExecutionTransportError::Unavailable
    );
    {
        let store = f.state.store.lock().unwrap();
        assert!(
            store
                .remote_binding_transaction(false, |store| old.attest_in_transaction(
                    store,
                    old.identity(),
                    old.lease(),
                    old.fence()
                ))
                .is_err()
        );
        store
            .remote_binding_transaction(false, |store| {
                next.attest_in_transaction(store, next.identity(), next.lease(), next.fence())
            })
            .unwrap();
    }
    f.state.worker_channels.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(8), worker)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
