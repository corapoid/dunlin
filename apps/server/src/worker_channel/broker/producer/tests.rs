use super::*;
use crate::worker_channel::tests::{Fixture, fixture};
use std::sync::atomic::AtomicUsize;

struct Handler {
    session: String,
    live: AtomicBool,
    calls: AtomicUsize,
    gate: Semaphore,
    payload: String,
}
impl Handler {
    fn new(payload: &str) -> Arc<Self> {
        Arc::new(Self {
            session: "session".into(),
            live: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            gate: Semaphore::new(0),
            payload: payload.into(),
        })
    }
}
impl SessionBroker for Handler {
    fn session(&self) -> &str {
        &self.session
    }
    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.live.store(false, Ordering::Release);
    }
    fn request(
        self: Arc<Self>,
        _: wire::BrokerRequest,
        _: Instant,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Release);
            self.gate.acquire().await.unwrap().forget();
            Ok(self.payload.clone())
        })
    }
}
struct TransportFixture {
    _source: Fixture,
    _producer: sessions::SessionProducerRegistration,
    _events: tokio::sync::mpsc::Receiver<sessions::SessionEvent>,
    registration: Option<sessions::Registration>,
    _requests: tokio::sync::mpsc::Receiver<sessions::SessionRequest>,
    transport: sessions::SessionTransport,
}
fn channel(broker: bool, pending: u8, maximum: u32) -> TransportFixture {
    let mut source = fixture(true, true);
    let config = Arc::get_mut(&mut source.state.config)
        .unwrap()
        .worker_controller
        .as_mut()
        .unwrap();
    config.execution_enabled = true;
    config.persistent_sessions = true;
    config.mcp_broker = broker;
    let (sender, events) = tokio::sync::mpsc::channel(8);
    let producer = source
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
    let (identity, lease) = {
        let store = source.state.store.lock().unwrap();
        let identity = store
            .authenticate_worker_token(&source.credential.credential)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                &source.state.worker_channels.boot_id,
                "worker",
                "channel",
                now_ms().unwrap(),
                15000,
            )
            .unwrap();
        (identity, lease)
    };
    let (registration, transport, receiver) = if broker {
        source
            .state
            .worker_channels
            .register_broker_session_transport(
                &source.state,
                &identity,
                &lease,
                "n-controller".into(),
                wire::BrokerLimits {
                    max_pending: pending,
                    max_reply_bytes: maximum,
                },
            )
            .unwrap()
    } else {
        source
            .state
            .worker_channels
            .register_session_transport(&source.state, &identity, &lease, "n-controller".into())
            .unwrap()
    };
    TransportFixture {
        _source: source,
        _producer: producer,
        _events: events,
        registration: Some(registration),
        _requests: receiver,
        transport,
    }
}
fn request(transport: &sessions::SessionTransport, id: &str) -> wire::BrokerRequest {
    wire::BrokerRequest {
        scope: wire::BrokerScope::Catalog {
            session: dunlin_relay_protocol::worker::session::SessionFence {
                channel: transport.fence().clone(),
                controller_session_id: "session".into(),
                worker_session_id: "native".into(),
                session_epoch: 1,
            },
        },
        request_id: id.into(),
        operation: wire::BrokerOperation::ToolsList { cursor: None },
    }
}
fn bind(transport: &sessions::SessionTransport, handler: &Arc<Handler>) -> BrokerRegistration {
    transport.bind_broker(handler.clone()).unwrap()
}
async fn frame(exchange: &mut Exchange) -> wire::BrokerControllerMessage {
    tokio::time::timeout(Duration::from_secs(2), exchange.next())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn bound_work_keeps_capacity_until_last_fragment_and_never_replays_ids() {
    let f = channel(true, 1, 65536);
    let handler = Handler::new(&serde_json::json!({"data":"ż".repeat(7000)}).to_string());
    let _binding = bind(&f.transport, &handler);
    let mut exchange = f.transport.broker_exchange();
    let request = request(&f.transport, "one");
    assert!(exchange.submit(request.clone()).unwrap().is_none());
    assert!(matches!(
        exchange.submit(self::request(&f.transport, "two")).unwrap(),
        Some(wire::BrokerControllerMessage::McpRejected {
            code: wire::BrokerError::CapacityReached,
            ..
        })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), exchange.next())
            .await
            .is_err()
    );
    assert_eq!(handler.calls.load(Ordering::Acquire), 1);
    handler.gate.add_permits(1);
    let mut assembly = wire::ReplyAssembly::new(&request).unwrap();
    loop {
        assert_eq!(exchange.slots.available_permits(), 0);
        let wire::BrokerControllerMessage::McpReplyChunk { chunk } = frame(&mut exchange).await
        else {
            panic!("expected complete bounded reply")
        };
        if let Some(payload) = assembly.push(&chunk).unwrap() {
            assert_eq!(payload, handler.payload);
            break;
        }
    }
    assert_eq!(exchange.slots.available_permits(), 1);
    assert_eq!(
        exchange.submit(request).unwrap_err(),
        WorkerErrorCode::InvalidSequence
    );
    assert_eq!(handler.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn pending_owner_drop_and_replacement_never_reroute_old_work_or_emit_partial_results() {
    let f = channel(true, 2, 65536);
    let old = Handler::new(&serde_json::json!({"data":"x".repeat(9000)}).to_string());
    let binding = bind(&f.transport, &old);
    let mut exchange = f.transport.broker_exchange();
    exchange.submit(request(&f.transport, "old")).unwrap();
    old.gate.add_permits(1);
    assert!(matches!(
        frame(&mut exchange).await,
        wire::BrokerControllerMessage::McpReplyChunk { .. }
    ));
    drop(binding);
    let new = Handler::new("{\"tools\":[]}");
    let _new = bind(&f.transport, &new);
    assert!(matches!(
        frame(&mut exchange).await,
        wire::BrokerControllerMessage::McpRejected {
            code: wire::BrokerError::OutcomeUnknown,
            ..
        }
    ));
    assert_eq!(new.calls.load(Ordering::Acquire), 0);
    exchange.submit(request(&f.transport, "new")).unwrap();
    new.gate.add_permits(1);
    assert!(matches!(
        frame(&mut exchange).await,
        wire::BrokerControllerMessage::McpReplyChunk { .. }
    ));
    assert_eq!(old.calls.load(Ordering::Acquire), 1);
    assert_eq!(new.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn expiry_and_drop_interrupt_work_without_retry_or_reopening_capacity() {
    let f = channel(true, 2, 65536);
    let handler = Handler::new("{}");
    let _binding = bind(&f.transport, &handler);
    let mut exchange = f.transport.broker_exchange();
    exchange.submit(request(&f.transport, "expires")).unwrap();
    tokio::task::yield_now().await;
    exchange.expire_at(Instant::now() + WAIT).unwrap();
    assert!(matches!(
        frame(&mut exchange).await,
        wire::BrokerControllerMessage::McpRejected {
            code: wire::BrokerError::OutcomeUnknown,
            ..
        }
    ));
    assert!(!handler.is_live());
    assert_eq!(handler.calls.load(Ordering::Acquire), 1);
    let other = Handler::new("{}");
    drop(_binding);
    let _other = bind(&f.transport, &other);
    exchange.submit(request(&f.transport, "drop")).unwrap();
    tokio::task::yield_now().await;
    drop(exchange);
    assert!(!other.is_live());
    assert_eq!(other.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn invalid_json_or_negotiated_oversize_never_emits_a_fragment() {
    for payload in [
        "invalid".into(),
        serde_json::json!({"data":"x".repeat(5000)}).to_string(),
    ] {
        let f = channel(true, 1, 4096);
        let handler = Handler::new(&payload);
        let _binding = bind(&f.transport, &handler);
        let mut exchange = f.transport.broker_exchange();
        exchange.submit(request(&f.transport, "bounded")).unwrap();
        handler.gate.add_permits(1);
        assert!(matches!(
            frame(&mut exchange).await,
            wire::BrokerControllerMessage::McpRejected {
                code: wire::BrokerError::OutcomeUnknown,
                ..
            }
        ));
        assert!(!handler.is_live());
    }
}

#[test]
fn registration_is_unique_bounded_and_source_stop_interrupts_all_owners() {
    let mut f = channel(true, 1, 4096);
    let mut owners = Vec::new();
    for index in 0..SESSIONS {
        let mut handler = Handler::new("{}");
        Arc::get_mut(&mut handler).unwrap().session = format!("session-{index}");
        let registration = bind(&f.transport, &handler);
        assert!(f.transport.bind_broker(handler.clone()).is_err());
        assert!(handler.is_live());
        owners.push((handler, registration));
    }
    assert!(f.transport.bind_broker(Handler::new("{}")).is_err());
    drop(f.registration.take());
    assert!(owners.iter().all(|(handler, _)| !handler.is_live()));
}

#[test]
fn v4_unbound_foreign_boot_and_source_drop_never_dispatch() {
    let old = channel(false, 1, 4096);
    let handler = Handler::new("{}");
    assert!(old.transport.bind_broker(handler.clone()).is_err());
    let mut f = channel(true, 1, 4096);
    let mut exchange = f.transport.broker_exchange();
    assert!(matches!(
        exchange.submit(request(&f.transport, "unbound")).unwrap(),
        Some(wire::BrokerControllerMessage::McpRejected {
            code: wire::BrokerError::Denied,
            ..
        })
    ));
    let _binding = bind(&f.transport, &handler);
    let mut foreign = request(&f.transport, "foreign");
    if let wire::BrokerScope::Catalog { session } = &mut foreign.scope {
        session.channel.worker_boot_id = "foreign".into();
    }
    assert_eq!(
        exchange.submit(foreign).unwrap_err(),
        WorkerErrorCode::InvalidMessage
    );
    drop(f.registration.take());
    assert!(!handler.is_live());
    assert_eq!(handler.calls.load(Ordering::Acquire), 0);
    assert!(exchange.submit(request(&f.transport, "stopped")).is_err());
}
