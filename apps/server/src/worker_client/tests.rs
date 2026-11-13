use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;
pub(super) fn config() -> WorkerConfig {
    WorkerConfig {
        exports_state_dir: None,
        execution: None,
        network: NetworkConfig {
            socket: "/fixture/tailscaled.sock".into(),
            node_id: "n-worker".into(),
            serve_origin: "https://worker.tail123.ts.net".into(),
            listen: "127.0.0.1:9988".parse().unwrap(),
        },
        controller: ControllerConfig {
            node_id: "n-controller".into(),
            serve_origin: "https://controller.tail123.ts.net".into(),
        },
        host_id: "host".into(),
        workspace_id: "workspace".into(),
        enrollment_epoch: 3,
        credential_revision: 7,
        capabilities: HostCapabilities {
            protocol_version: 1,
            version: "fixture".into(),
            os: "linux".into(),
            architecture: "aarch64".into(),
            harnesses: vec!["opencode".into()],
            sandbox: dunlin_domain::hosts::SandboxStrength::None,
            max_parallel: 1,
        },
    }
}
fn challenge() -> ChannelProofRequest {
    ChannelProofRequest {
        channel_id: "channel".into(),
        controller_boot_id: "controller-boot".into(),
        nonce: "a".repeat(64),
    }
}
#[test]
fn proof_book_never_mints_from_requests_and_expires_without_extending_repeated_proofs() {
    let config = config();
    let book = ProofBook::new(&config, "worker-boot".into());
    let now = Instant::now();
    let request = challenge();
    assert!(book.prove(&request, now).is_err());
    book.install(request.clone(), now).unwrap();
    for forged in [
        ChannelProofRequest {
            channel_id: "other-channel".into(),
            ..request.clone()
        },
        ChannelProofRequest {
            controller_boot_id: "old-controller".into(),
            ..request.clone()
        },
        ChannelProofRequest {
            nonce: "b".repeat(64),
            ..request.clone()
        },
    ] {
        assert!(book.prove(&forged, now).is_err());
    }
    let proof = book.prove(&request, now).unwrap();
    assert_eq!(proof.host_id, "host");
    assert_eq!(proof.worker_boot_id, "worker-boot");
    assert_eq!(proof.enrollment_epoch, 3);
    assert_eq!(proof.credential_revision, 7);
    assert_eq!(
        book.prove(&request, now + Duration::from_secs(9)).unwrap(),
        proof
    );
    assert!(book.prove(&request, now + PROOF_TTL).is_err());
    assert!(
        book.prove(&request, now + PROOF_TTL + Duration::from_secs(1))
            .is_err()
    );
}
#[test]
fn proof_book_is_connection_owned_and_cleared_on_acceptance_or_cancellation() {
    let config = config();
    let first = Arc::new(ProofBook::new(&config, "old-worker-boot".into()));
    let second = ProofBook::new(&config, "new-worker-boot".into());
    let now = Instant::now();
    let request = challenge();
    first.install(request.clone(), now).unwrap();
    assert!(second.prove(&request, now).is_err());
    assert!(first.install(request.clone(), now).is_err());
    {
        let _guard = ClearProof(first.clone());
        assert!(first.prove(&request, now).is_ok());
    }
    assert!(first.prove(&request, now).is_err());
}
#[tokio::test]
async fn proof_http_rejects_browser_credentials_and_unknown_json_without_creating_pending_nonce() {
    let book = Arc::new(ProofBook::new(&config(), "worker-boot".into()));
    let app = proof_router(book.clone());
    let request = challenge();
    let body = serde_json::to_string(&request).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/worker/channel-proof")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    book.install(request, Instant::now()).unwrap();
    for (key, value) in [
        ("cookie", "browser=secret"),
        ("origin", "https://controller.tail123.ts.net"),
        ("authorization", "Bearer ignored"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/v1/worker/channel-proof")
                    .header(key, value)
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/worker/channel-proof")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let value: ChannelProofResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(value.host_id, "host");
    let unknown = body.trim_end_matches('}').to_owned() + ",\"verified\":true}";
    assert_eq!(
        app.oneshot(
            Request::post("/v1/worker/channel-proof")
                .body(Body::from(unknown))
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn actual_websocket_client_sends_pinned_identity_proves_nonce_and_rejects_wrong_lease_ack() {
    use tokio_tungstenite::tungstenite::handshake::server::{
        Request as WsRequest, Response as WsResponse,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = config();
    let book = Arc::new(ProofBook::new(&config, "worker-boot".into()));
    let server_book = book.clone();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // Tungstenite's callback ABI requires its unboxed ErrorResponse.
        #[allow(clippy::result_large_err)]
        let verify_headers = |request: &WsRequest, mut response: WsResponse| {
            assert_eq!(request.uri().path(), "/v1/worker/channel");
            assert!(request.headers().get("cookie").is_none());
            assert!(request.headers().get("origin").is_none());
            assert!(
                request.headers()["authorization"]
                    .to_str()
                    .unwrap()
                    .starts_with("Bearer dhw_")
            );
            response.headers_mut().insert(
                "sec-websocket-protocol",
                WORKER_EXPORT_SUBPROTOCOL.parse().unwrap(),
            );
            Ok(response)
        };
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, verify_headers)
            .await
            .unwrap();
        let hello = socket.next().await.unwrap().unwrap().into_text().unwrap();
        match dunlin_relay_protocol::worker::decode_frame(&hello).unwrap() {
            WorkerMessage::Hello {
                host_id,
                enrollment_epoch,
                credential_revision,
                ..
            } => {
                assert_eq!(host_id, "host");
                assert_eq!(enrollment_epoch, 3);
                assert_eq!(credential_revision, 7);
            }
            _ => panic!("Hello expected"),
        }
        let request = challenge();
        socket
            .send(Message::Text(
                serde_json::to_string(&ControllerMessage::Challenge {
                    channel_id: request.channel_id.clone(),
                    controller_boot_id: request.controller_boot_id.clone(),
                    nonce: request.nonce.clone(),
                    heartbeat_interval_ms: 1000,
                    lease_ttl_ms: 5000,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let ready = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(matches!(
            dunlin_relay_protocol::worker::decode_frame(&ready).unwrap(),
            WorkerMessage::Ready { .. }
        ));
        assert_eq!(
            server_book
                .prove(&request, Instant::now())
                .unwrap()
                .worker_boot_id,
            "worker-boot"
        );
        socket
            .send(Message::Text(
                serde_json::to_string(&ControllerMessage::Accepted {
                    channel_id: "channel".into(),
                    controller_boot_id: "controller-boot".into(),
                    worker_boot_id: "worker-boot".into(),
                    lease_epoch: 19,
                    expires_at_ms: 9999999999999,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let heartbeat = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(matches!(
            dunlin_relay_protocol::worker::decode_frame(&heartbeat).unwrap(),
            WorkerMessage::Heartbeat {
                lease_epoch: 19,
                sequence: 1,
                ..
            }
        ));
        assert!(server_book.prove(&request, Instant::now()).is_err());
        socket
            .send(Message::Text(
                serde_json::to_string(&ControllerMessage::HeartbeatAck {
                    channel_id: "channel".into(),
                    lease_epoch: 18,
                    sequence: 1,
                    expires_at_ms: 9999999999999,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
    });
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let credential = Zeroizing::new(format!("dhw_{}", "a".repeat(43)));
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        connect_channel(
            &config,
            credential,
            stream,
            book.clone(),
            "worker-boot".into(),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err(), "worker_heartbeat_ack_mismatch");
    peer.await.unwrap();
    assert!(book.prove(&challenge(), Instant::now()).is_err());
}
#[test]
fn worker_config_and_stdin_token_do_not_accept_cookie_or_arbitrary_endpoints() {
    config().validate().unwrap();
    for credential in ["dhw_short", "Bearer dhw_fake", " dhw_fake", "dhw_\nsecret"] {
        assert!(!valid_credential(credential));
    }
    assert!(valid_credential(&format!("dhw_{}", "a".repeat(43))));
    let mut config = config();
    config.network.listen = "0.0.0.0:9988".parse().unwrap();
    assert!(config.validate().is_err());
    config.network.listen = "127.0.0.1:9988".parse().unwrap();
    config.controller.serve_origin = "https://public.example".into();
    assert!(config.validate().is_err());
}
#[test]
fn acceptance_requires_a_completed_current_reverse_callback_and_consumes_pending_proof() {
    let config = config();
    let book = ProofBook::new(&config, "worker-boot".into());
    book.install(challenge(), Instant::now()).unwrap();
    assert_eq!(book.accepted().unwrap_err(), "worker_proof_not_completed");
    assert!(book.prove(&challenge(), Instant::now()).is_err());
    book.install(challenge(), Instant::now()).unwrap();
    book.prove(&challenge(), Instant::now()).unwrap();
    book.accepted().unwrap();
    assert!(book.prove(&challenge(), Instant::now()).is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn config_loader_rejects_fifo_without_writer_symlink_directory_and_oversized_regular_file() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("config.fifo");
    let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    // No writer is opened. The production loader must reject the descriptor
    // before delegating any read to Tokio's blocking filesystem adapter.
    let error = tokio::time::timeout(Duration::from_millis(500), load_config(&fifo))
        .await
        .expect("FIFO rejection must not wait for a writer")
        .err()
        .unwrap();
    assert_eq!(error, "worker_config_regular_file_required");
    let regular = dir.path().join("config.toml");
    std::fs::write(&regular, "invalid").unwrap();
    let link = dir.path().join("symlink.toml");
    symlink(&regular, &link).unwrap();
    assert_eq!(
        load_config(&link).await.err().unwrap(),
        "worker_config_unreadable"
    );
    assert_eq!(
        load_config(dir.path()).await.err().unwrap(),
        "worker_config_regular_file_required"
    );
    std::fs::write(&regular, vec![b' '; 64 * 1024 + 1]).unwrap();
    assert_eq!(
        load_config(&regular).await.err().unwrap(),
        "worker_config_too_large"
    );
}
#[cfg(unix)]
#[tokio::test]
async fn config_loader_reads_a_bounded_local_regular_file_and_validates_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.toml");
    let text = r#"host_id = "host"
workspace_id = "workspace"
enrollment_epoch = 3
credential_revision = 7
[network]
socket = "/fixture/tailscaled.sock"
node_id = "n-worker"
serve_origin = "https://worker.tail123.ts.net"
listen = "127.0.0.1:9988"
[controller]
node_id = "n-controller"
serve_origin = "https://controller.tail123.ts.net"
[capabilities]
protocol_version = 1
version = "fixture"
os = "linux"
architecture = "aarch64"
harnesses = ["opencode"]
sandbox = "none"
max_parallel = 1
"#;
    std::fs::write(&path, text).unwrap();
    let parsed = load_config(&path)
        .await
        .unwrap_or_else(|error| panic!("Valid worker fixture rejected: {error}"));
    assert_eq!(parsed.host_id, "host");
    assert_eq!(parsed.credential_revision, 7);
    std::fs::write(
        &path,
        text.replace(
            "https://controller.tail123.ts.net",
            "https://public.example",
        ),
    )
    .unwrap();
    assert_eq!(
        load_config(&path).await.err().unwrap(),
        "invalid_worker_peer"
    );
}
#[tokio::test]
async fn proof_request_slots_bound_unparsed_bodies_and_release_on_cancellation_and_response_drop() {
    let book = Arc::new(ProofBook::new(&config(), "worker-boot".into()));
    let slots = Arc::new(Semaphore::new(8));
    let app = proof_router_with_limit(book, slots.clone());
    let mut pending = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        pending.push(tokio::spawn(async move {
            let body = Body::from_stream(futures_util::stream::pending::<
                Result<Bytes, std::convert::Infallible>,
            >());
            app.oneshot(
                Request::post("/v1/worker/channel-proof")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap()
        }));
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while slots.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("All eight body requests must own admission slots");
    let denied = app
        .clone()
        .oneshot(
            Request::post("/v1/worker/channel-proof")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
    drop(denied);
    for task in pending {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    assert_eq!(slots.available_permits(), 8);
    let response = app
        .oneshot(
            Request::post("/v1/worker/channel-proof")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        slots.available_permits(),
        7,
        "Slot survives handler return until response-body completion/drop"
    );
    drop(response);
    assert_eq!(slots.available_permits(), 8);
}
#[tokio::test]
async fn proof_listener_drops_excess_incomplete_connections_and_reuses_released_slot() {
    use axum::serve::Listener;
    use tokio::io::AsyncReadExt;
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let mut listener = ProofListener::new(socket);
    let mut accepted = Vec::new();
    let mut clients = Vec::new();
    for _ in 0..64 {
        let client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (io, _) = Listener::accept(&mut listener).await;
        clients.push(client);
        accepted.push(io);
    }
    assert_eq!(listener.connections.available_permits(), 0);
    let mut excess = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut byte = [0u8; 1];
    {
        let accept = Listener::accept(&mut listener);
        tokio::pin!(accept);
        tokio::select! {
         result=tokio::time::timeout(Duration::from_secs(1),excess.read(&mut byte))=>assert_eq!(result.expect("Excess connection must be closed without HTTP headers").unwrap(),0),
         _=&mut accept=>panic!("An excess connection was admitted without a slot"),
        }
    }
    drop(accepted.pop());
    assert_eq!(listener.connections.available_permits(), 1);
    let fresh = tokio::net::TcpStream::connect(address).await.unwrap();
    let (_io, _) = tokio::time::timeout(Duration::from_secs(1), Listener::accept(&mut listener))
        .await
        .expect("Released connection slot must admit a fresh client");
    assert_eq!(listener.connections.available_permits(), 0);
    drop(fresh);
    drop(clients);
    drop(accepted);
}

#[tokio::test]
async fn expired_lease_rejects_an_already_buffered_ack_instead_of_resurrecting_authority() {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let (left, right) = tokio::io::duplex(4096);
    let mut client = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
    let mut controller = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
    controller
        .send(Message::Text(
            serde_json::to_string(&ControllerMessage::HeartbeatAck {
                channel_id: "channel".into(),
                lease_epoch: 1,
                sequence: 1,
                expires_at_ms: 99999,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        receive(
            &mut client,
            tokio::time::Instant::now() - Duration::from_millis(1)
        )
        .await
        .err(),
        Some("worker_channel_deadline")
    );
}
#[tokio::test]
async fn a_buffered_pong_streak_is_bounded_while_waiting_for_a_controller_message() {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let (left, right) = tokio::io::duplex(4096);
    let mut client = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
    let mut controller = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
    for _ in 0..121 {
        controller
            .send(Message::Pong(Vec::new().into()))
            .await
            .unwrap();
    }
    assert_eq!(
        receive(
            &mut client,
            tokio::time::Instant::now() + Duration::from_secs(10)
        )
        .await
        .err(),
        Some("worker_controller_rate_limited")
    );
}

#[tokio::test]
async fn an_ack_ready_after_runtime_stall_cannot_bypass_the_old_lease_deadline() {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let (left, right) = tokio::io::duplex(4096);
    let mut client = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
    let mut controller = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
    let mut waiting = Box::pin(receive(
        &mut client,
        tokio::time::Instant::now() + Duration::from_secs(1),
    ));
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    // Deliberately stall this test runtime: the inner socket future is ready
    // when Timeout is next polled, after its deadline has already passed.
    std::thread::sleep(Duration::from_millis(1100));
    controller
        .send(Message::Text(
            serde_json::to_string(&ControllerMessage::HeartbeatAck {
                channel_id: "channel".into(),
                lease_epoch: 1,
                sequence: 1,
                expires_at_ms: 99999,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(waiting.await.err(), Some("worker_channel_deadline"));
}

#[tokio::test]
async fn metadata_sql_wait_does_not_block_heartbeat_or_ack_and_metadata_rechecks_current_grant() {
    use crate::worker_exports::{
        CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind,
        WorkerStore,
    };
    use dunlin_relay_protocol::worker::{
        ExportErrorCode, ExportPrincipal as WirePrincipal, PrincipalKind as WireKind, decode_frame,
    };
    use tokio_tungstenite::tungstenite::protocol::Role;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config();
    let state = dir.path().join("state");
    config.exports_state_dir = Some(state.clone());
    let scope = ExportScope {
        controller_node_id: config.controller.node_id.clone(),
        worker_node_id: config.network.node_id.clone(),
        workspace_id: config.workspace_id.clone(),
        host_id: config.host_id.clone(),
        enrollment_epoch: config.enrollment_epoch,
        credential_revision: config.credential_revision,
    };
    let root = dir.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let store = WorkerStore::open(&state, scope.clone()).unwrap();
    let captured = CapturedCheckout::capture(&root, &"a".repeat(64), None).unwrap();
    let export = store
        .register_captured(&scope, "Private checkout", &captured)
        .unwrap();
    let principal = ExportPrincipal {
        kind: PrincipalKind::Human,
        id: "reader".into(),
    };
    store
        .set_grant(
            &scope,
            &export.id,
            &principal,
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    let connection = rusqlite::Connection::open(state.join("exports.sqlite3")).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (left, right) = tokio::io::duplex(65536);
    let mut client = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
    let mut controller = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
    let book = Arc::new(ProofBook::new(&config, "worker-boot".into()));
    let peer_book = book.clone();
    let worker =
        tokio::spawn(
            async move { session(&config, &mut client, book, "worker-boot".into()).await },
        );
    let hello = controller
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(matches!(
        decode_frame(&hello).unwrap(),
        WorkerMessage::Hello {
            protocol_version: WORKER_EXPORT_PROTOCOL_VERSION,
            ..
        }
    ));
    let proof = challenge();
    controller
        .send(Message::Text(
            serde_json::to_string(&ControllerMessage::Challenge {
                channel_id: proof.channel_id.clone(),
                controller_boot_id: proof.controller_boot_id.clone(),
                nonce: proof.nonce.clone(),
                heartbeat_interval_ms: 1000,
                lease_ttl_ms: 5000,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let ready = controller
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(matches!(
        decode_frame(&ready).unwrap(),
        WorkerMessage::Ready { .. }
    ));
    peer_book.prove(&proof, Instant::now()).unwrap();
    controller
        .send(Message::Text(
            serde_json::to_string(&ControllerMessage::Accepted {
                channel_id: "channel".into(),
                controller_boot_id: "controller-boot".into(),
                worker_boot_id: "worker-boot".into(),
                lease_epoch: 19,
                expires_at_ms: 9999999999999,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let request = ControllerMessage::DescribeExport {
        request_id: "describe".into(),
        channel_id: "channel".into(),
        lease_epoch: 19,
        export_id: export.id.clone(),
        principal: WirePrincipal {
            kind: WireKind::Human,
            id: "reader".into(),
        },
        expected_export_revision: Some(1),
    };
    controller
        .send(Message::Text(
            serde_json::to_string(&request).unwrap().into(),
        ))
        .await
        .unwrap();
    // The metadata job cannot finish its SQL cut while this writer owns the DB.
    // The same socket must still make a real heartbeat round trip first.
    let frame = tokio::time::timeout(Duration::from_secs(2), controller.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(matches!(
        decode_frame(&frame).unwrap(),
        WorkerMessage::Heartbeat {
            sequence: 1,
            lease_epoch: 19,
            ..
        }
    ));
    controller
        .send(Message::Text(
            serde_json::to_string(&ControllerMessage::HeartbeatAck {
                channel_id: "channel".into(),
                lease_epoch: 19,
                sequence: 1,
                expires_at_ms: 9999999999999,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    // Revoke atomically before releasing the SQL waiter: the metadata result
    // cannot describe the formerly authorized catalog snapshot.
    connection
        .execute("UPDATE export_grants SET active=0,revision=revision+1", [])
        .unwrap();
    connection.execute_batch("COMMIT").unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), controller.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(
        matches!(decode_frame(&frame).unwrap(), WorkerMessage::ExportError { request_id, code: ExportErrorCode::Forbidden, lease_epoch: 19, .. } if request_id == "describe")
    );
    // Metadata did not consume or reset the heartbeat sequence.
    let frame = tokio::time::timeout(Duration::from_secs(2), controller.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(matches!(
        decode_frame(&frame).unwrap(),
        WorkerMessage::Heartbeat { sequence: 2, .. }
    ));
    drop(controller);
    assert!(worker.await.unwrap().is_err());
}

#[test]
fn execution_configuration_is_explicit_strict_and_never_downgrades_an_enabled_invalid_runtime() {
    let mut config = config();
    assert!(matches!(
        config.hello("boot"),
        WorkerMessage::Hello {
            protocol_version: WORKER_EXPORT_PROTOCOL_VERSION,
            ..
        }
    ));
    config.execution =
        Some(serde_json::from_value(serde_json::json!({"enabled":false,"harnesses":{}})).unwrap());
    config.validate().unwrap();
    config.execution=Some(serde_json::from_value(serde_json::json!({"enabled":true,"harnesses":{"opencode":{"program":"/usr/bin/python3","args":[]}}})).unwrap());
    assert!(config.validate().is_err()); // no explicit operator-created catalog
    config.exports_state_dir = Some("/operator/catalog".into());
    assert!(config.validate().is_err()); // no claimed kernel adapter in config
    config.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    config.validate().unwrap();
    // Configuration validation alone never creates a v3 Hello/runtime proof.
    assert!(matches!(
        config.hello("boot"),
        WorkerMessage::Hello {
            protocol_version: WORKER_EXPORT_PROTOCOL_VERSION,
            ..
        }
    ));
    for extra in ["credential", "environment", "verified", "fallback"] {
        let mut value = serde_json::json!({"enabled":true,"harnesses":{}});
        value[extra] = serde_json::json!(true);
        assert!(serde_json::from_value::<execution::OperatorExecutionConfig>(value).is_err());
    }
}

#[test]
fn broker_opt_in_cannot_select_legacy_transport_or_fake_runtime_readiness() {
    let mut config = config();
    assert_eq!(
        selected_protocol(&config, None).unwrap(),
        (WORKER_EXPORT_PROTOCOL_VERSION, WORKER_EXPORT_SUBPROTOCOL)
    );
    config.exports_state_dir = Some("/operator/catalog".into());
    config.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    for (enabled, persistent) in [(false, false), (true, false), (false, true), (true, true)] {
        config.execution = Some(serde_json::from_value(serde_json::json!({"enabled":enabled,"persistent_sessions":persistent,"mcp_broker":true,"harnesses":{"opencode":{"program":"/usr/bin/python3","args":[]}}})).unwrap());
        if enabled && persistent {
            execution::validate(&config).unwrap();
            assert_eq!(
                selected_protocol(&config, None).unwrap_err(),
                "worker_execution_runtime_required"
            );
        } else {
            assert_eq!(
                selected_protocol(&config, None).unwrap_err(),
                "worker_broker_session_opt_in_required"
            );
        }
    }
}
