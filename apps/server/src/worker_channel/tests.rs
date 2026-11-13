use super::*;
use crate::{
    config::Config,
    store::{Store, hosts::WorkerCredential},
};
use dunlin_domain::hosts::{HostCapabilities, HostDefinition};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

fn capabilities() -> HostCapabilities {
    serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"x86_64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap()
}
pub(super) struct Fixture {
    pub(super) state: AppState,
    owner: String,
    pub(super) credential: WorkerCredential,
    _dir: tempfile::TempDir,
}
pub(super) fn fixture(approved: bool, configured: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let owner = store.create_user("channel-owner", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "Channel fixture").unwrap();
    let definition:HostDefinition=serde_json::from_value(json!({"name":"Fixture worker","node_id":"n-worker","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap();
    let host = store
        .create_host(&owner.id, &workspace.id, definition)
        .unwrap();
    let credential = store
        .enroll_host(&host.enrollment.ticket, capabilities())
        .unwrap();
    if approved {
        let claimed = store
            .host(&owner.id, &workspace.id, &host.host.id)
            .unwrap()
            .unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &host.host.id, claimed.revision)
            .unwrap();
    }
    let config = Config {
        worker_controller: configured.then(|| ControllerConfig {
            execution_enabled: false,
            persistent_sessions: false,
            mcp_broker: false,
            socket: dir.path().join("absent-localapi.sock"),
            node_id: "n-controller".into(),
            serve_origin: "https://controller.tail123.ts.net".into(),
        }),
        ..Default::default()
    };
    Fixture {
        state: AppState::new(store, config),
        owner: owner.id,
        credential,
        _dir: dir,
    }
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(f: &Fixture) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new()
        .route("/v1/worker/channel", axum::routing::get(upgrade))
        .with_state(f.state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("ws://{address}/v1/worker/channel"), Server(task))
}
fn request(f: &Fixture, url: &str) -> tungstenite::http::Request<()> {
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        WORKER_SUBPROTOCOL.parse().unwrap(),
    );
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", f.credential.credential)
            .parse()
            .unwrap(),
    );
    request
}
fn no_lease(f: &Fixture) -> bool {
    f.state
        .store
        .lock()
        .unwrap()
        .worker_channel_diagnostic(&f.owner, &f.credential.workspace_id, &f.credential.host_id)
        .unwrap()
        .is_none()
}
#[test]
fn browser_auth_and_ambiguous_protocol_headers_never_pass_worker_upgrade_guards() {
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer dhw_fixture".parse().unwrap());
    headers.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        WORKER_SUBPROTOCOL.parse().unwrap(),
    );
    assert_eq!(protocol(&headers).unwrap(), WORKER_PROTOCOL_VERSION);
    let mut metadata = headers.clone();
    metadata.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        WORKER_EXPORT_SUBPROTOCOL.parse().unwrap(),
    );
    assert_eq!(protocol(&metadata).unwrap(), WORKER_EXPORT_PROTOCOL_VERSION);
    let mut persistent = headers.clone();
    persistent.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        worker::session::WORKER_SESSION_SUBPROTOCOL.parse().unwrap(),
    );
    assert_eq!(
        protocol(&persistent).unwrap(),
        worker::session::WORKER_SESSION_PROTOCOL_VERSION
    );
    for name in [header::COOKIE, header::ORIGIN] {
        let mut mixed = headers.clone();
        mixed.insert(name, "fixture".parse().unwrap());
        assert!(bearer(&mixed).is_err());
    }
    for auth in [
        "Bearer dhe_fixture",
        "Bearer kst_fixture",
        "Basic fixture",
        "",
    ] {
        let mut bad = headers.clone();
        bad.insert(header::AUTHORIZATION, auth.parse().unwrap());
        assert!(bearer(&bad).is_err());
    }
    let mut duplicate = headers.clone();
    duplicate.append(header::AUTHORIZATION, "Bearer dhw_second".parse().unwrap());
    assert!(bearer(&duplicate).is_err());
    for value in [
        "",
        "dunlin.worker.v6",
        "dunlin.worker.v1, other",
        "other, dunlin.worker.v1",
    ] {
        let mut bad = headers.clone();
        bad.insert(header::SEC_WEBSOCKET_PROTOCOL, value.parse().unwrap());
        assert!(protocol(&bad).is_err());
    }
    let mut duplicate = headers;
    duplicate.append(
        header::SEC_WEBSOCKET_PROTOCOL,
        WORKER_SUBPROTOCOL.parse().unwrap(),
    );
    assert!(protocol(&duplicate).is_err());
    let mut rate = Rate::new();
    for _ in 0..120 {
        rate.frame().unwrap();
    }
    assert_eq!(rate.frame().unwrap_err(), WorkerErrorCode::RateLimited);
}
#[tokio::test]
async fn missing_controller_configuration_or_unapproved_worker_never_upgrades_or_opens_lease() {
    for (approved, configured, status) in [
        (true, false, StatusCode::NOT_FOUND),
        (false, true, StatusCode::UNAUTHORIZED),
    ] {
        let f = fixture(approved, configured);
        let (url, _server) = server(&f).await;
        match tokio_tungstenite::connect_async(request(&f, &url)).await {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), status),
            _ => panic!("Worker upgrade did not reject invalid admission"),
        }
        assert!(no_lease(&f));
        assert_eq!(f.state.worker_channels.slots.available_permits(), SLOTS);
    }
}
#[tokio::test]
async fn actual_websocket_ready_without_private_reverse_proof_cannot_open_a_lease() {
    reverse_proof_required(false).await;
}
#[tokio::test]
async fn broker_hello5_requires_the_same_private_reverse_proof_before_any_lease() {
    reverse_proof_required(true).await;
}
async fn reverse_proof_required(broker: bool) {
    let mut f = fixture(true, true);
    let (sender, _events) = tokio::sync::mpsc::channel(8);
    let _producer = if broker {
        let config = Arc::get_mut(&mut f.state.config)
            .unwrap()
            .worker_controller
            .as_mut()
            .unwrap();
        config.execution_enabled = true;
        config.persistent_sessions = true;
        config.mcp_broker = true;
        Some(
            f.state
                .worker_channels
                .register_session_producer(sender)
                .unwrap(),
        )
    } else {
        None
    };
    let (url, _server) = server(&f).await;
    let mut request = request(&f, &url);
    let subprotocol = if broker {
        worker::session::broker::WORKER_BROKER_SUBPROTOCOL
    } else {
        WORKER_SUBPROTOCOL
    };
    request
        .headers_mut()
        .insert(header::SEC_WEBSOCKET_PROTOCOL, subprotocol.parse().unwrap());
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response.headers()[header::SEC_WEBSOCKET_PROTOCOL],
        subprotocol
    );
    let encoded = if broker {
        worker::session::broker::BrokerHello {
            protocol_version: 5,
            host_id: f.credential.host_id.clone(),
            workspace_id: f.credential.workspace_id.clone(),
            enrollment_epoch: f.credential.enrollment_epoch,
            credential_revision: f.credential.credential_revision,
            worker_boot_id: "worker-boot".into(),
            capabilities: capabilities(),
            broker_limits: worker::session::broker::BrokerLimits {
                max_pending: 8,
                max_reply_bytes: 262144,
            },
        }
        .encode()
        .unwrap()
    } else {
        serde_json::to_string(&WorkerMessage::Hello {
            protocol_version: 1,
            host_id: f.credential.host_id.clone(),
            workspace_id: f.credential.workspace_id.clone(),
            enrollment_epoch: f.credential.enrollment_epoch,
            credential_revision: f.credential.credential_revision,
            worker_boot_id: "worker-boot".into(),
            capabilities: capabilities(),
        })
        .unwrap()
    };
    socket
        .send(tungstenite::Message::Text(encoded.into()))
        .await
        .unwrap();
    assert!(no_lease(&f));
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ControllerMessage::Challenge {
        channel_id,
        controller_boot_id,
        nonce,
        ..
    } = worker::decode_controller_frame(frame.to_text().unwrap()).unwrap()
    else {
        panic!("Expected nonce challenge");
    };
    assert!(no_lease(&f));
    socket
        .send(tungstenite::Message::Text(
            serde_json::to_string(&WorkerMessage::Ready {
                channel_id,
                controller_boot_id,
                worker_boot_id: "worker-boot".into(),
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
    assert_eq!(
        worker::decode_controller_frame(frame.to_text().unwrap()).unwrap(),
        ControllerMessage::Error {
            code: WorkerErrorCode::PeerVerificationFailed
        }
    );
    assert!(no_lease(&f));
    let host = f
        .state
        .store
        .lock()
        .unwrap()
        .host(&f.owner, &f.credential.workspace_id, &f.credential.host_id)
        .unwrap()
        .unwrap();
    assert!(!host.execution_ready);
    assert_eq!(host.network_verification, "unverified");
    f.state.worker_channels.shutdown().await;
    assert_eq!(f.state.worker_channels.slots.available_permits(), SLOTS);
}
#[tokio::test]
async fn shutdown_interrupts_silent_handshake_and_releases_its_slot_without_database_lease() {
    let f = fixture(true, true);
    let (url, _server) = server(&f).await;
    let (_socket, _response) = tokio_tungstenite::connect_async(request(&f, &url))
        .await
        .unwrap();
    assert_eq!(f.state.worker_channels.slots.available_permits(), SLOTS - 1);
    tokio::time::timeout(Duration::from_secs(5), f.state.worker_channels.shutdown())
        .await
        .unwrap();
    assert!(no_lease(&f));
    assert_eq!(f.state.worker_channels.slots.available_permits(), SLOTS);
}
#[test]
fn exit_cleanup_marks_own_revoked_lease_offline_but_cannot_touch_replacement() {
    let f = fixture(true, true);
    let (identity, first, second) = {
        let store = f.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&f.credential.credential)
            .unwrap();
        let first = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "worker-one",
                "channel-one",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        let second = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "worker-two",
                "channel-two",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        (identity, first, second)
    };
    drop(LeaseExit {
        state: f.state.clone(),
        identity: identity.clone(),
        lease: first,
    });
    {
        let store = f.state.store.lock().unwrap();
        let diagnostic = store
            .worker_channel_diagnostic(&f.owner, &f.credential.workspace_id, &f.credential.host_id)
            .unwrap()
            .unwrap();
        assert_eq!(diagnostic.phase, "active");
        assert_eq!(diagnostic.lease.channel_id, "channel-two");
        let host = store
            .host(&f.owner, &f.credential.workspace_id, &f.credential.host_id)
            .unwrap()
            .unwrap();
        store
            .revoke_host(
                &f.owner,
                &f.credential.workspace_id,
                &f.credential.host_id,
                host.revision,
            )
            .unwrap();
    }
    drop(LeaseExit {
        state: f.state.clone(),
        identity,
        lease: second,
    });
    let store = f.state.store.lock().unwrap();
    let diagnostic = store
        .worker_channel_diagnostic(&f.owner, &f.credential.workspace_id, &f.credential.host_id)
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.phase, "offline");
}

fn metadata_descriptor(f: &Fixture) -> worker::ExportDescriptor {
    serde_json::from_value(json!({"scope":{"controller_node_id":"n-controller","worker_node_id":"n-worker","workspace_id":f.credential.workspace_id,"host_id":f.credential.host_id,"enrollment_epoch":f.credential.enrollment_epoch,"credential_revision":f.credential.credential_revision},"export":{"id":"export-test","name":"Approved","repo_kind":"directory","checkout_identity_digest":"a".repeat(64),"settings_digest":"b".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":f.owner},"permissions":["read"],"revision":1,"export_revision":1,"active":true}})).unwrap()
}
#[tokio::test]
async fn delayed_old_registration_and_cleanup_cannot_replace_or_remove_the_new_slot() {
    let f = fixture(true, true);
    let (identity, old, new) = {
        let store = f.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&f.credential.credential)
            .unwrap();
        let old = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "old-worker",
                "old-channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        let new = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "new-worker",
                "new-channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        (identity, old, new)
    };
    let (registration, mut requests) = f
        .state
        .worker_channels
        .register_exports(&f.state, &identity, &new, "n-controller".into())
        .unwrap();
    assert!(
        f.state
            .worker_channels
            .register_exports(&f.state, &identity, &old, "n-controller".into())
            .is_err()
    );
    // This is the old socket's delayed teardown, after the replacement registered.
    let stale = exports::Registration::for_test(
        f.state.worker_channels.clone(),
        identity.host_id().into(),
        old.channel_id.clone(),
    );
    drop(stale);
    let state = f.state.clone();
    let host = f.credential.host_id.clone();
    let user = f.owner.clone();
    let result = tokio::spawn(async move {
        state
            .worker_channels
            .describe_export(
                &state,
                &host,
                "export-test",
                worker::ExportPrincipal {
                    kind: worker::PrincipalKind::Human,
                    id: user,
                },
                Some(1),
            )
            .await
    });
    let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.export_id, "export-test");
    let _ = request.response.send(Ok(metadata_descriptor(&f)));
    let observed = result.await.unwrap().unwrap();
    assert_eq!(observed.lease().channel_id, new.channel_id);
    drop(registration);
}
#[tokio::test]
async fn response_for_another_principal_is_not_sealed_as_a_binding_observation() {
    let f = fixture(true, true);
    let (identity, lease) = {
        let store = f.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&f.credential.credential)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "worker",
                "channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        (identity, lease)
    };
    let (_registration, mut requests) = f
        .state
        .worker_channels
        .register_exports(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let state = f.state.clone();
    let host = f.credential.host_id.clone();
    let user = f.owner.clone();
    let result = tokio::spawn(async move {
        state
            .worker_channels
            .describe_export(
                &state,
                &host,
                "export-test",
                worker::ExportPrincipal {
                    kind: worker::PrincipalKind::Human,
                    id: user,
                },
                Some(1),
            )
            .await
    });
    let request = requests.recv().await.unwrap();
    let mut descriptor = metadata_descriptor(&f);
    descriptor.grant.principal.id = "another-user".into();
    let _ = request.response.send(Ok(descriptor));
    assert!(matches!(
        result.await.unwrap(),
        Err(DescribeError::InvalidResponse)
    ));
}
#[tokio::test]
async fn pending_metadata_capacity_is_held_until_the_actual_request_is_dropped() {
    let f = fixture(true, true);
    let (identity, lease) = {
        let store = f.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&f.credential.credential)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "worker",
                "channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        (identity, lease)
    };
    let (_registration, mut receiver) = f
        .state
        .worker_channels
        .register_exports(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let principal = worker::ExportPrincipal {
        kind: worker::PrincipalKind::Human,
        id: f.owner.clone(),
    };
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let state = f.state.clone();
        let host = f.credential.host_id.clone();
        let principal = principal.clone();
        tasks.spawn(async move {
            state
                .worker_channels
                .describe_export(&state, &host, "export-test", principal, Some(1))
                .await
        });
    }
    let mut held = Vec::new();
    for _ in 0..8 {
        held.push(
            tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert!(matches!(
        f.state
            .worker_channels
            .describe_export(
                &f.state,
                &f.credential.host_id,
                "export-test",
                principal,
                Some(1)
            )
            .await,
        Err(DescribeError::Busy)
    ));
    drop(held);
    while let Some(result) = tasks.join_next().await {
        assert!(matches!(result.unwrap(), Err(DescribeError::Unavailable)));
    }
}
#[tokio::test]
async fn buffered_metadata_after_a_runtime_stall_cannot_restart_an_expired_request_deadline() {
    let f = fixture(true, true);
    let (identity, lease) = {
        let store = f.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&f.credential.credential)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                &f.state.worker_channels.boot_id,
                "worker",
                "channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        (identity, lease)
    };
    let (_registration, mut receiver) = f
        .state
        .worker_channels
        .register_exports(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let state = f.state.clone();
    let host = f.credential.host_id.clone();
    let user = f.owner.clone();
    let result = tokio::spawn(async move {
        state
            .worker_channels
            .describe_export(
                &state,
                &host,
                "export-test",
                worker::ExportPrincipal {
                    kind: worker::PrincipalKind::Human,
                    id: user,
                },
                Some(1),
            )
            .await
    });
    let request = receiver.recv().await.unwrap();
    // Timeout polls an already-ready inner future first. Deliberately prevent
    // this runtime from polling either the request or timer until both are ready.
    std::thread::sleep(Duration::from_millis(5100));
    let _ = request.response.send(Ok(metadata_descriptor(&f)));
    assert!(matches!(
        result.await.unwrap(),
        Err(DescribeError::Unavailable)
    ));
}

#[tokio::test]
async fn v3_upgrade_requires_explicit_enabled_configuration_and_actual_producer_registration() {
    for (enabled, registered) in [(false, false), (false, true), (true, false)] {
        let mut f = fixture(true, true);
        Arc::get_mut(&mut f.state.config)
            .unwrap()
            .worker_controller
            .as_mut()
            .unwrap()
            .execution_enabled = enabled;
        let (send, _events) = tokio::sync::mpsc::channel(8);
        let _registration = registered.then(|| {
            f.state
                .worker_channels
                .register_execution_producer(send)
                .unwrap()
        });
        let (url, _server) = server(&f).await;
        let mut request = request(&f, &url);
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            WORKER_EXECUTION_SUBPROTOCOL.parse().unwrap(),
        );
        match tokio_tungstenite::connect_async(request).await {
            Err(tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE)
            }
            _ => panic!("Disabled/unregistered v3 must not upgrade or downgrade"),
        }
        assert!(no_lease(&f));
    }
}

#[tokio::test]
async fn v4_upgrade_requires_both_operator_flags_and_its_own_live_producer() {
    for (enabled, persistent, registered) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
        (true, true, false),
    ] {
        let mut fixture = fixture(true, true);
        let config = Arc::get_mut(&mut fixture.state.config)
            .unwrap()
            .worker_controller
            .as_mut()
            .unwrap();
        config.execution_enabled = enabled;
        config.persistent_sessions = persistent;
        let (sender, _events) = tokio::sync::mpsc::channel(8);
        let _producer = registered.then(|| {
            fixture
                .state
                .worker_channels
                .register_session_producer(sender)
                .unwrap()
        });
        let (url, _server) = server(&fixture).await;
        let mut request = request(&fixture, &url);
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            worker::session::WORKER_SESSION_SUBPROTOCOL.parse().unwrap(),
        );
        match tokio_tungstenite::connect_async(request).await {
            Err(tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE)
            }
            _ => panic!("Unconfigured/unregistered v4 must neither upgrade nor downgrade"),
        }
        assert!(no_lease(&fixture));
    }
}

#[tokio::test]
async fn v5_upgrade_requires_all_operator_flags_and_a_live_session_producer() {
    for enabled in [false, true] {
        for persistent in [false, true] {
            for broker in [false, true] {
                for registered in [false, true] {
                    if enabled && persistent && broker && registered {
                        continue;
                    }
                    let mut fixture = fixture(true, true);
                    let config = Arc::get_mut(&mut fixture.state.config)
                        .unwrap()
                        .worker_controller
                        .as_mut()
                        .unwrap();
                    config.execution_enabled = enabled;
                    config.persistent_sessions = persistent;
                    config.mcp_broker = broker;
                    let (sender, _events) = tokio::sync::mpsc::channel(8);
                    let _producer = registered.then(|| {
                        fixture
                            .state
                            .worker_channels
                            .register_session_producer(sender)
                            .unwrap()
                    });
                    let (url, _server) = server(&fixture).await;
                    let mut request = request(&fixture, &url);
                    request.headers_mut().insert(
                        header::SEC_WEBSOCKET_PROTOCOL,
                        worker::session::broker::WORKER_BROKER_SUBPROTOCOL
                            .parse()
                            .unwrap(),
                    );
                    match tokio_tungstenite::connect_async(request).await {
                        Err(tungstenite::Error::Http(response)) => {
                            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE)
                        }
                        _ => panic!(
                            "v5 must not upgrade or silently downgrade without all prerequisites"
                        ),
                    }
                    assert!(no_lease(&fixture));
                }
            }
        }
    }
}

#[tokio::test]
async fn broker_and_legacy_hello_are_never_interchangeable_after_upgrade() {
    for selected in [4, 5] {
        let mut fixture = fixture(true, true);
        let config = Arc::get_mut(&mut fixture.state.config)
            .unwrap()
            .worker_controller
            .as_mut()
            .unwrap();
        config.execution_enabled = true;
        config.persistent_sessions = true;
        config.mcp_broker = true;
        let (sender, _events) = tokio::sync::mpsc::channel(8);
        let _producer = fixture
            .state
            .worker_channels
            .register_session_producer(sender)
            .unwrap();
        let (url, _server) = server(&fixture).await;
        let mut request = request(&fixture, &url);
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            if selected == 5 {
                worker::session::broker::WORKER_BROKER_SUBPROTOCOL
            } else {
                worker::session::WORKER_SESSION_SUBPROTOCOL
            }
            .parse()
            .unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let common = WorkerMessage::Hello {
            protocol_version: 4,
            host_id: fixture.credential.host_id.clone(),
            workspace_id: fixture.credential.workspace_id.clone(),
            enrollment_epoch: fixture.credential.enrollment_epoch,
            credential_revision: fixture.credential.credential_revision,
            worker_boot_id: "mixed-worker".into(),
            capabilities: capabilities(),
        };
        let encoded = if selected == 5 {
            serde_json::to_string(&common).unwrap()
        } else {
            worker::session::broker::BrokerHello {
                protocol_version: 5,
                host_id: fixture.credential.host_id.clone(),
                workspace_id: fixture.credential.workspace_id.clone(),
                enrollment_epoch: fixture.credential.enrollment_epoch,
                credential_revision: fixture.credential.credential_revision,
                worker_boot_id: "mixed-worker".into(),
                capabilities: capabilities(),
                broker_limits: worker::session::broker::BrokerLimits {
                    max_pending: 8,
                    max_reply_bytes: 262144,
                },
            }
            .encode()
            .unwrap()
        };
        socket
            .send(tungstenite::Message::Text(encoded.into()))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            worker::decode_controller_frame(frame.to_text().unwrap()).unwrap(),
            ControllerMessage::Error {
                code: WorkerErrorCode::InvalidMessage
            }
        );
        assert!(no_lease(&fixture));
    }
}
