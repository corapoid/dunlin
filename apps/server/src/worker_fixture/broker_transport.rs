//! An authenticated peer can speak v5 without possessing native execution or
//! private MCP authority. Exercise that boundary over the actual TLS/WSS path.
use super::*;
use dunlin_relay_protocol::worker::{
    self as wire, ChannelProofRequest, ChannelProofResponse, ControllerMessage, WorkerMessage,
    session::{SessionFence, TurnFence, broker::*},
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[tokio::test]
async fn proved_v5_transport_denies_unbound_mcp_without_disrupting_heartbeats() {
    exercise(false).await;
}
#[tokio::test]
async fn proved_v5_bound_producer_keeps_heartbeats_alive_while_waiting() {
    exercise(true).await;
}
async fn exercise(bound: bool) {
    let f = fixture_with_broker_transport(true, true, false, true).await;
    let (sender, mut events) = tokio::sync::mpsc::channel(8);
    let _producer = f
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
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
                        worker_boot_id: "v5-manual-worker".into(),
                        nonce: request.nonce,
                        host_id: host,
                        enrollment_epoch: 1,
                        credential_revision: 1,
                    })
                }
            },
        ),
    );
    let listener = TcpListener::bind(f.client.network.listen).await.unwrap();
    let _proof = Task(tokio::spawn(async move {
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
        WORKER_BROKER_SUBPROTOCOL.parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap();
    assert_eq!(
        response.headers()["Sec-WebSocket-Protocol"],
        WORKER_BROKER_SUBPROTOCOL
    );
    let limits = BrokerLimits {
        max_pending: 2,
        max_reply_bytes: 65536,
    };
    let hello = BrokerHello {
        protocol_version: 5,
        host_id: f.host.clone(),
        workspace_id: f.workspace.clone(),
        enrollment_epoch: 1,
        credential_revision: 1,
        worker_boot_id: "v5-manual-worker".into(),
        capabilities: f.client.capabilities.clone(),
        broker_limits: limits.clone(),
    }
    .encode()
    .unwrap();
    socket.send(Message::Text(hello.into())).await.unwrap();
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
    } = wire::decode_controller_frame(frame.to_text().unwrap()).unwrap()
    else {
        panic!("private nonce challenge required")
    };
    let fence = wire::ChannelFence {
        channel_id: channel_id.clone(),
        controller_boot_id: controller_boot_id.clone(),
        worker_boot_id: "v5-manual-worker".into(),
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::Ready {
                channel_id,
                controller_boot_id,
                worker_boot_id: fence.worker_boot_id.clone(),
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
    let ControllerMessage::Accepted { lease_epoch, .. } =
        wire::decode_controller_frame(frame.to_text().unwrap()).unwrap()
    else {
        panic!("verified acceptance required")
    };
    let handler = Arc::new(ControlledBroker::default());
    let transport = f.state.worker_channels.session_transport(&f.host).unwrap();
    let _binding = bound.then(|| transport.bind_broker(handler.clone()).unwrap());
    if bound {
        // Actual controller queue -> TLS/WSS -> controlled peer. These receipts
        // exercise correlation only, not native execution or MCP authority.
        let command = transport.broker_open_fixture();
        let caller = transport.clone();
        let sent = command.clone();
        let mut waiter = tokio::spawn(async move { caller.open_broker_session(sent).await });
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = frame.to_text().unwrap();
        assert!(wire::session::decode_session_controller_frame(text).is_err());
        assert_eq!(
            decode_broker_session_controller_frame(text.as_bytes(), &fence, &limits).unwrap(),
            BrokerSessionControllerMessage::OpenBrokerSession {
                command: Box::new(command.clone())
            }
        );
        let mut wrong = wire::session::SessionWorkerMessage::Rejected {
            channel: fence.clone(),
            request: wire::session::SessionRequestRef::Open {
                identity: command.open.identity.clone(),
            },
            code: wire::ExecutionErrorCode::AuthorityRevoked,
        };
        if let wire::session::SessionWorkerMessage::Rejected {
            request: wire::session::SessionRequestRef::Open { identity },
            ..
        } = &mut wrong
        {
            identity.fingerprint = "e".repeat(64);
        }
        socket
            .send(Message::Text(serde_json::to_string(&wrong).unwrap().into()))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err()
        );
        let receipt = wire::session::SessionReceipt {
            identity: command.open.identity.clone(),
            fence: SessionFence {
                channel: fence.clone(),
                controller_session_id: command.open.controller_session_id.clone(),
                worker_session_id: "controlled-native".into(),
                session_epoch: 1,
            },
            budgets: command.open.budgets.clone(),
        };
        let readiness = wire::session::SessionWorkerMessage::SessionReady {
            receipt: receipt.clone(),
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&readiness).unwrap().into(),
            ))
            .await
            .unwrap();
        let observed = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.message, readiness);
        assert!(!waiter.is_finished());
        let accepted = wire::session::SessionWorkerMessage::SessionAccepted { receipt };
        socket
            .send(Message::Text(
                serde_json::to_string(&accepted).unwrap().into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), waiter)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            accepted
        );
        let mut second = command;
        second.open.identity.intent_id = "second-open".into();
        second.open.identity.fingerprint = broker_session_open_fingerprint(&second).unwrap();
        let caller = transport.clone();
        let sent = second.clone();
        let waiter = tokio::spawn(async move { caller.open_broker_session(sent).await });
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            decode_broker_session_controller_frame(
                frame.to_text().unwrap().as_bytes(),
                &fence,
                &limits
            )
            .unwrap(),
            BrokerSessionControllerMessage::OpenBrokerSession {
                command: Box::new(second.clone())
            }
        );
        let rejected = wire::session::SessionWorkerMessage::Rejected {
            channel: fence.clone(),
            request: wire::session::SessionRequestRef::Open {
                identity: second.open.identity,
            },
            code: wire::ExecutionErrorCode::AuthorityRevoked,
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&rejected).unwrap().into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), waiter)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            rejected
        );
    }

    let sequences = if bound { [1, 3, 4] } else { [1, 2, 3] };
    for sequence in sequences {
        socket
            .send(Message::Text(
                serde_json::to_string(&WorkerMessage::Heartbeat {
                    channel_id: fence.channel_id.clone(),
                    lease_epoch,
                    sequence,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(wire::decode_controller_frame(frame.to_text().unwrap()).unwrap(), ControllerMessage::HeartbeatAck { sequence: ack, .. } if ack == sequence)
        );
        let transport = f.state.worker_channels.session_transport(&f.host).unwrap();
        assert_eq!(transport.fence(), &fence);
        assert_eq!(transport.broker_offer(), Some(&limits));
        if sequence == if bound { 4 } else { 3 } {
            break;
        }
        let session = SessionFence {
            channel: fence.clone(),
            controller_session_id: "no-admitted-session".into(),
            worker_session_id: "no-native-session".into(),
            session_epoch: 1,
        };
        let (scope, operation) = if sequence == 1 {
            (
                BrokerScope::Catalog { session },
                BrokerOperation::ToolsList { cursor: None },
            )
        } else {
            (
                BrokerScope::Turn {
                    turn: TurnFence {
                        session,
                        turn_id: "unapproved-turn".into(),
                        turn_index: 1,
                        turn_epoch: 1,
                    },
                },
                BrokerOperation::ToolsCall {
                    name: format!("mcp_{}", "a".repeat(32)),
                    arguments_json: r#"{"write":true}"#.into(),
                },
            )
        };
        let request = BrokerRequest {
            scope,
            request_id: format!("unbound-{sequence}"),
            operation,
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&BrokerWorkerMessage::McpRequest {
                    request: request.clone(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        if bound {
            if sequence == 1 {
                // Simulated human decision remains pending. The real WSS owner
                // must still accept and persist the next heartbeat meanwhile.
                socket
                    .send(Message::Text(
                        serde_json::to_string(&WorkerMessage::Heartbeat {
                            channel_id: fence.channel_id.clone(),
                            lease_epoch,
                            sequence: 2,
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await
                    .unwrap();
                let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(matches!(
                    wire::decode_controller_frame(frame.to_text().unwrap()).unwrap(),
                    ControllerMessage::HeartbeatAck { sequence: 2, .. }
                ));
                assert_eq!(handler.calls.load(std::sync::atomic::Ordering::Acquire), 1);
            }
            handler.gate.add_permits(1);
        }
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let reply =
            decode_broker_controller_frame(frame.to_text().unwrap().as_bytes(), &request).unwrap();
        if bound {
            let BrokerControllerMessage::McpReplyChunk { chunk } = reply else {
                panic!("bound fixture response required")
            };
            assert!(
                ReplyAssembly::new(&request)
                    .unwrap()
                    .push(&chunk)
                    .unwrap()
                    .is_some()
            );
        } else {
            assert_eq!(
                reply,
                BrokerControllerMessage::McpRejected {
                    request: request.reference().unwrap(),
                    code: BrokerError::Denied
                }
            );
        }
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
    {
        let store = f.state.store.lock().unwrap();
        let health = store
            .host_health_observation(&f.owner, &f.workspace, &f.host, &f.state.worker_channels)
            .unwrap();
        let connection = health.connection.unwrap();
        assert_eq!(connection.protocol_version, 5);
        assert_eq!(connection.heartbeat_sequence, if bound { 4 } else { 3 });
    }
    // A different boot cannot even obtain an in-band rejection for this channel.
    let mut foreign = fence;
    foreign.worker_boot_id = "different-worker".into();
    let request = BrokerRequest {
        scope: BrokerScope::Catalog {
            session: SessionFence {
                channel: foreign,
                controller_session_id: "foreign".into(),
                worker_session_id: "foreign".into(),
                session_epoch: 1,
            },
        },
        request_id: "foreign".into(),
        operation: BrokerOperation::ToolsList { cursor: None },
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&BrokerWorkerMessage::McpRequest { request })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        wire::decode_controller_frame(frame.to_text().unwrap()).unwrap(),
        ControllerMessage::Error {
            code: wire::WorkerErrorCode::InvalidMessage
        }
    );
    drop(socket);
    if bound {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !handler.stopped.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(handler.calls.load(std::sync::atomic::Ordering::Acquire), 2);
    }
    f.state.worker_channels.shutdown().await;
}

// Transport-only responder: no SQL session/native receipt, account, SDK consent
// or external MCP HTTP is manufactured by this fixture.
struct ControlledBroker {
    gate: tokio::sync::Semaphore,
    calls: std::sync::atomic::AtomicUsize,
    stopped: std::sync::atomic::AtomicBool,
}
impl crate::worker_channel::SessionBroker for ControlledBroker {
    fn session(&self) -> &str {
        "no-admitted-session"
    }
    fn is_live(&self) -> bool {
        !self.stopped.load(std::sync::atomic::Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
    }
    fn request(
        self: Arc<Self>,
        request: BrokerRequest,
        _: tokio::time::Instant,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, BrokerError>> + Send>>
    {
        Box::pin(async move {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            self.gate.acquire().await.unwrap().forget();
            Ok(match request.operation {
                BrokerOperation::ToolsList { .. } => json!({"tools":[]}),
                BrokerOperation::ToolsCall { .. } => {
                    json!({"content":[{"type":"text","text":"controlled fixture"}]})
                }
            }
            .to_string())
        })
    }
}

impl Default for ControlledBroker {
    fn default() -> Self {
        Self {
            gate: tokio::sync::Semaphore::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
