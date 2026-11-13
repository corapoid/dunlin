//! Verified controller/worker WSS leases. Legacy metadata stays separate from
//! explicitly enabled v3 execution and v4 session transports. Registration alone
//! grants no project/worker execution authority or OS-stop evidence.
use crate::{
    api::{ApiError, AppState},
    store::worker_channels::{ChannelLease, WorkerIdentity},
    tailscale::Transport,
};
use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use dunlin_relay_protocol::worker::{
    self, ChannelProofRequest, ChannelProofResponse, ControllerMessage, MAX_WORKER_FRAME_BYTES,
    WORKER_EXECUTION_PROTOCOL_VERSION, WORKER_EXECUTION_SUBPROTOCOL,
    WORKER_EXPORT_PROTOCOL_VERSION, WORKER_EXPORT_SUBPROTOCOL, WORKER_PROTOCOL_VERSION,
    WORKER_SUBPROTOCOL, WorkerErrorCode, WorkerMessage,
};
use rand::RngExt;
use serde::Deserialize;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

mod broker;
pub(crate) use broker::{BrokerRegistration, SessionBroker};
mod execution;
mod health;
pub(crate) use health::LiveChannelObservation;
mod sessions;
#[cfg(test)]
pub(crate) use execution::ExecutionTransportError;
pub(crate) use execution::{ExecutionEvent, ExecutionProducerRegistration, ExecutionTransport};
pub(crate) use sessions::{SessionEvent, SessionProducerRegistration, SessionTransport};
mod exports;
pub(crate) use exports::{DescribeError, ObservedExport};
const SLOTS: usize = 64;
const HANDSHAKE: Duration = Duration::from_secs(10);
const SEND: Duration = Duration::from_secs(5);
const TTL_MS: u64 = 10000;
const INTERVAL_MS: u64 = 2000;
type Result<T> = std::result::Result<T, WorkerErrorCode>;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    #[serde(default)]
    pub execution_enabled: bool,
    #[serde(default)]
    pub persistent_sessions: bool,
    #[serde(default)]
    pub mcp_broker: bool,
    pub socket: PathBuf,
    pub node_id: String,
    pub serve_origin: String,
}
pub struct Service {
    pub boot_id: String,
    slots: Arc<Semaphore>,
    shutdown: AtomicBool,
    exports: exports::Registry,
    executions: execution::Registry,
    sessions: sessions::Registry,
    health: health::Registry,
    #[cfg(test)]
    pub test_transport: Option<Transport>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            boot_id: uuid::Uuid::now_v7().to_string(),
            slots: Arc::new(Semaphore::new(SLOTS)),
            shutdown: AtomicBool::new(false),
            exports: exports::Registry::default(),
            executions: execution::Registry::default(),
            sessions: sessions::Registry::default(),
            health: health::Registry::default(),
            #[cfg(test)]
            test_transport: None,
        }
    }
}
impl Service {
    #[cfg(test)]
    pub(crate) fn register_broker_fixture(
        &self,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        node: String,
        offer: worker::session::broker::BrokerLimits,
    ) -> Result<(SessionTransport, Box<dyn Send>)> {
        let (registration, transport, receiver) =
            self.register_broker_session_transport(state, identity, lease, node, offer)?;
        Ok((transport, Box::new((registration, receiver))))
    }
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
    pub async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.executions.stop();
        self.sessions.stop();
        self.slots.close();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while self.slots.available_permits() != SLOTS && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    async fn stopped(&self) {
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
fn bearer(headers: &HeaderMap) -> std::result::Result<Zeroizing<String>, ApiError> {
    if headers.contains_key(header::COOKIE) || headers.contains_key(header::ORIGIN) {
        return Err(ApiError::unauthorized());
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or_else(ApiError::unauthorized)?;
    if values.next().is_some() {
        return Err(ApiError::unauthorized());
    }
    let token = value
        .to_str()
        .ok()
        .filter(|value| value.len() <= 128)
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    if !token.starts_with("dhw_") {
        return Err(ApiError::unauthorized());
    }
    Ok(Zeroizing::new(token.into()))
}
fn protocol(headers: &HeaderMap) -> std::result::Result<u16, ApiError> {
    let mut values = headers.get_all(header::SEC_WEBSOCKET_PROTOCOL).iter();
    let first = values.next().and_then(|value| value.to_str().ok());
    if values.next().is_some() {
        return Err(ApiError::bad_request(
            "A single worker WebSocket subprotocol is required",
        ));
    }
    match first {
        Some(WORKER_SUBPROTOCOL) => Ok(WORKER_PROTOCOL_VERSION),
        Some(WORKER_EXPORT_SUBPROTOCOL) => Ok(WORKER_EXPORT_PROTOCOL_VERSION),
        Some(WORKER_EXECUTION_SUBPROTOCOL) => Ok(WORKER_EXECUTION_PROTOCOL_VERSION),
        Some(worker::session::WORKER_SESSION_SUBPROTOCOL) => {
            Ok(worker::session::WORKER_SESSION_PROTOCOL_VERSION)
        }
        Some(worker::session::broker::WORKER_BROKER_SUBPROTOCOL) => {
            Ok(worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION)
        }
        _ => Err(ApiError::bad_request(
            "Use exactly one supported worker WebSocket subprotocol",
        )),
    }
}

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> std::result::Result<Response, ApiError> {
    let config = state
        .config
        .worker_controller
        .clone()
        .ok_or_else(ApiError::not_found)?;
    let token = bearer(&headers)?;
    let version = protocol(&headers)?;
    if matches!(
        version,
        worker::session::WORKER_SESSION_PROTOCOL_VERSION
            | worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION
    ) && (!config.execution_enabled
        || !config.persistent_sessions
        || (version == worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION
            && !config.mcp_broker)
        || !state.worker_channels.sessions.ready())
    {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Worker sessions are unavailable",
        ));
    }
    if version == WORKER_EXECUTION_PROTOCOL_VERSION
        && (!config.execution_enabled || !state.worker_channels.executions.ready())
    {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Worker execution is unavailable",
        ));
    }
    let identity = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Worker registry unavailable"))?
        .authenticate_worker_token(&token)
        .map_err(|_| ApiError::unauthorized())?;
    if state.worker_channels.shutdown.load(Ordering::Acquire) {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Worker channels are stopping",
        ));
    }
    let permit = state
        .worker_channels
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Worker channel capacity reached",
            )
        })?;
    let transport = Transport::new(
        config.socket,
        config.node_id,
        config.serve_origin,
        state.config.listen_address().map_err(ApiError::internal)?,
    )
    .map_err(|_| ApiError::internal("Worker transport configuration is unavailable"))?;
    #[cfg(test)]
    let transport = state
        .worker_channels
        .test_transport
        .clone()
        .unwrap_or(transport);
    Ok(ws
        .protocols([match version {
            WORKER_PROTOCOL_VERSION => WORKER_SUBPROTOCOL,
            WORKER_EXPORT_PROTOCOL_VERSION => WORKER_EXPORT_SUBPROTOCOL,
            worker::session::WORKER_SESSION_PROTOCOL_VERSION => {
                worker::session::WORKER_SESSION_SUBPROTOCOL
            }
            worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION => {
                worker::session::broker::WORKER_BROKER_SUBPROTOCOL
            }
            _ => WORKER_EXECUTION_SUBPROTOCOL,
        }])
        .max_message_size(MAX_WORKER_FRAME_BYTES)
        .max_frame_size(MAX_WORKER_FRAME_BYTES)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_WORKER_FRAME_BYTES * 2)
        .on_upgrade(move |socket| live(socket, state, token, identity, transport, permit, version)))
}
struct TaskGuard(tokio::task::JoinHandle<()>);
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct LeaseExit {
    state: AppState,
    identity: WorkerIdentity,
    lease: ChannelLease,
}
impl Drop for LeaseExit {
    fn drop(&mut self) {
        if let Ok(store) = self.state.store.lock() {
            let _ = store.close_worker_channel(&self.identity, &self.lease);
        }
    }
}
fn now_ms() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .ok_or(WorkerErrorCode::InternalError)
}
struct Rate {
    start: tokio::time::Instant,
    frames: usize,
}
impl Rate {
    fn new() -> Self {
        Self {
            start: tokio::time::Instant::now(),
            frames: 0,
        }
    }
    fn frame(&mut self) -> Result<()> {
        self.frame_for_protocol(WORKER_PROTOCOL_VERSION)
    }
    fn frame_for_protocol(&mut self, version: u16) -> Result<()> {
        if self.start.elapsed() >= Duration::from_secs(60) {
            self.start = tokio::time::Instant::now();
            self.frames = 0;
        }
        self.frames += 1;
        if self.frames > worker::worker_rate_limit(version) {
            return Err(WorkerErrorCode::RateLimited);
        }
        Ok(())
    }
}
async fn receive(socket: &mut WebSocket, rate: &mut Rate) -> Result<WorkerMessage> {
    loop {
        let message = socket
            .recv()
            .await
            .ok_or(WorkerErrorCode::ChannelExpired)?
            .map_err(|_| WorkerErrorCode::InvalidMessage)?;
        rate.frame()?;
        match message {
            Message::Text(text) => {
                return worker::decode_frame(text.as_str())
                    .map_err(|_| WorkerErrorCode::InvalidMessage);
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => return Err(WorkerErrorCode::ChannelExpired),
            _ => return Err(WorkerErrorCode::InvalidMessage),
        }
    }
}
async fn send(socket: &mut WebSocket, message: ControllerMessage) -> Result<()> {
    message
        .validate()
        .map_err(|_| WorkerErrorCode::InternalError)?;
    let text = serde_json::to_string(&message).map_err(|_| WorkerErrorCode::InternalError)?;
    if text.len() > MAX_WORKER_FRAME_BYTES {
        return Err(WorkerErrorCode::InternalError);
    }
    tokio::time::timeout(SEND, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| WorkerErrorCode::ChannelExpired)?
        .map_err(|_| WorkerErrorCode::ChannelExpired)
}
async fn live(
    mut socket: WebSocket,
    state: AppState,
    token: Zeroizing<String>,
    initial: WorkerIdentity,
    transport: Transport,
    _permit: OwnedSemaphorePermit,
    version: u16,
) {
    let guarded_state = state.clone();
    let guarded_identity = initial.clone();
    let (revoked, mut revocation) = tokio::sync::watch::channel(false);
    let _identity_guard = TaskGuard(tokio::spawn(async move {
        let mut changes = match guarded_state.store.lock() {
            Ok(store) => store.subscribe_catalog(),
            Err(_) => {
                let _ = revoked.send(true);
                return;
            }
        };
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _=changes.recv()=>{},_=tick.tick()=>{} }
            let allowed = guarded_state
                .store
                .lock()
                .is_ok_and(|store| store.validate_worker_identity(&guarded_identity).is_ok());
            if !allowed {
                let _ = revoked.send(true);
                return;
            }
        }
    }));
    let result = tokio::select! {
        result=session(&mut socket,&state,token,&initial,&transport,version)=>result,
        _=state.worker_channels.stopped()=>Err(WorkerErrorCode::ChannelExpired),
        _=revocation.changed()=>Err(WorkerErrorCode::AuthorityRevoked),
    };
    if let Err(code) = result {
        let _ = send(&mut socket, ControllerMessage::Error { code }).await;
    }
    let _ = tokio::time::timeout(
        SEND,
        socket.send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: "Worker channel closed".into(),
        }))),
    )
    .await;
}
async fn session(
    socket: &mut WebSocket,
    state: &AppState,
    token: Zeroizing<String>,
    initial: &WorkerIdentity,
    transport: &Transport,
    version: u16,
) -> Result<()> {
    let mut rate = Rate::new();
    let hello = tokio::time::timeout(HANDSHAKE, broker::receive_hello(socket, &mut rate, version))
        .await
        .map_err(|_| WorkerErrorCode::ChannelExpired)??;
    let broker::ChannelHello {
        protocol_version,
        host_id,
        workspace_id,
        enrollment_epoch,
        credential_revision,
        worker_boot_id,
        capabilities,
        broker_offer,
    } = hello;
    let metadata = version >= WORKER_EXPORT_PROTOCOL_VERSION;
    let execute = version == WORKER_EXECUTION_PROTOCOL_VERSION;
    if protocol_version != version {
        return Err(WorkerErrorCode::UnsupportedProtocol);
    }
    let identity = {
        let store = state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        let identity = store
            .authenticate_worker(&token, &host_id, enrollment_epoch, credential_revision)
            .map_err(|_| WorkerErrorCode::Unauthorized)?;
        if identity.host_id() != initial.host_id()
            || identity.workspace_id() != workspace_id
            || identity.workspace_id() != initial.workspace_id()
            || identity.host().enrollment_epoch != initial.host().enrollment_epoch
            || identity.host().credential_revision != initial.host().credential_revision
        {
            return Err(WorkerErrorCode::Unauthorized);
        }
        store
            .report_host(
                &token,
                &host_id,
                enrollment_epoch,
                credential_revision,
                capabilities,
            )
            .map_err(|_| WorkerErrorCode::Unauthorized)?;
        identity
    };
    drop(token);
    let channel_id = uuid::Uuid::now_v7().to_string();
    let controller_boot_id = state.worker_channels.boot_id.clone();
    let mut random = [0u8; 32];
    rand::rng().fill(&mut random);
    let nonce = hex::encode(random);
    let challenge_deadline = tokio::time::Instant::now() + HANDSHAKE;
    send(
        socket,
        ControllerMessage::Challenge {
            channel_id: channel_id.clone(),
            controller_boot_id: controller_boot_id.clone(),
            nonce: nonce.clone(),
            heartbeat_interval_ms: INTERVAL_MS,
            lease_ttl_ms: TTL_MS,
        },
    )
    .await?;
    let ready = tokio::time::timeout_at(challenge_deadline, receive(socket, &mut rate))
        .await
        .map_err(|_| WorkerErrorCode::ChannelExpired)??;
    if ready
        != (WorkerMessage::Ready {
            channel_id: channel_id.clone(),
            controller_boot_id: controller_boot_id.clone(),
            worker_boot_id: worker_boot_id.clone(),
            nonce: nonce.clone(),
        })
    {
        return Err(WorkerErrorCode::InvalidMessage);
    }
    let expected = ChannelProofResponse {
        channel_id: channel_id.clone(),
        controller_boot_id: controller_boot_id.clone(),
        worker_boot_id: worker_boot_id.clone(),
        nonce: nonce.clone(),
        host_id: host_id.clone(),
        enrollment_epoch,
        credential_revision,
    };
    let request = ChannelProofRequest {
        channel_id: channel_id.clone(),
        controller_boot_id: controller_boot_id.clone(),
        nonce,
    };
    let proof = tokio::time::timeout_at(challenge_deadline, prove(transport, &identity, &request))
        .await
        .map_err(|_| WorkerErrorCode::PeerVerificationFailed)??;
    if proof != expected {
        return Err(WorkerErrorCode::PeerVerificationFailed);
    }
    let mut lease = {
        let store = state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        store
            .open_worker_channel(
                &identity,
                &controller_boot_id,
                &worker_boot_id,
                &channel_id,
                now_ms()?,
                TTL_MS,
            )
            .map_err(|_| WorkerErrorCode::AuthorityRevoked)?
    };
    let _exit = LeaseExit {
        state: state.clone(),
        identity: identity.clone(),
        lease: lease.clone(),
    };
    // Independent guards can cancel a stalled Accepted/ACK send, not just a
    // receive loop. No database lock is held while network verification awaits.
    let guarded_state = state.clone();
    let guarded_identity = identity.clone();
    let guarded_lease = lease.clone();
    let guarded_boot = controller_boot_id.clone();
    let (lease_failed, mut fenced) = tokio::sync::watch::channel(false);
    let _lease_guard = TaskGuard(tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let allowed = now_ms().is_ok_and(|now| {
                guarded_state.store.lock().is_ok_and(|store| {
                    store
                        .validate_worker_channel(
                            &guarded_identity,
                            &guarded_lease,
                            &guarded_boot,
                            now,
                        )
                        .is_ok()
                })
            });
            if !allowed {
                let _ = lease_failed.send(true);
                return;
            }
        }
    }));
    let checked_transport = transport.clone();
    let node = identity.host().definition.node_id.clone();
    let origin = identity.host().definition.serve_origin.clone();
    let (network_failed, mut network) = tokio::sync::watch::channel(false);
    let _network_guard = TaskGuard(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if !tokio::time::timeout(
                Duration::from_secs(3),
                checked_transport.verify_peer(&node, &origin),
            )
            .await
            .is_ok_and(|result| result.is_ok())
            {
                let _ = network_failed.send(true);
                return;
            }
        }
    }));
    let messages = async {
        send(
            socket,
            ControllerMessage::Accepted {
                channel_id: channel_id.clone(),
                controller_boot_id,
                worker_boot_id,
                lease_epoch: lease.epoch,
                expires_at_ms: u64::try_from(lease.expires_at_ms)
                    .map_err(|_| WorkerErrorCode::InternalError)?,
            },
        )
        .await?;
        let _health = state
            .worker_channels
            .register_health(state, &identity, &lease, version)?;
        // Registration follows Accepted; v1 is kept diagnostic-only.
        let (_registration, mut requests, _legacy_sender) = if metadata {
            let node = state
                .config
                .worker_controller
                .as_ref()
                .ok_or(WorkerErrorCode::InternalError)?
                .node_id
                .clone();
            let (registration, receiver) = state
                .worker_channels
                .register_exports(state, &identity, &lease, node)?;
            (Some(registration), receiver, None)
        } else {
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            (None, receiver, Some(sender))
        };
        if matches!(
            version,
            worker::session::WORKER_SESSION_PROTOCOL_VERSION
                | worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION
        ) {
            let node = state
                .config
                .worker_controller
                .as_ref()
                .ok_or(WorkerErrorCode::InternalError)?
                .node_id
                .clone();
            let (_session_registration, transport, session_requests) = match broker_offer {
                Some(offer) => state
                    .worker_channels
                    .register_broker_session_transport(state, &identity, &lease, node, offer)?,
                None => state
                    .worker_channels
                    .register_session_transport(state, &identity, &lease, node)?,
            };
            return sessions::messages(
                socket,
                state,
                &identity,
                &mut lease,
                &mut rate,
                &mut requests,
                session_requests,
                transport,
            )
            .await;
        }
        if execute {
            let node = state
                .config
                .worker_controller
                .as_ref()
                .ok_or(WorkerErrorCode::InternalError)?
                .node_id
                .clone();
            let (_execution_registration, transport, execution_requests) = state
                .worker_channels
                .register_execution_transport(state, &identity, &lease, node)?;
            execution::messages(
                socket,
                state,
                &identity,
                &mut lease,
                &mut rate,
                &mut requests,
                execution_requests,
                transport,
            )
            .await
        } else {
            exports::messages(
                socket,
                state,
                &identity,
                &mut lease,
                &mut rate,
                &mut requests,
                metadata,
            )
            .await
        }
    };
    tokio::select! {
        result=messages=>result,
        _=fenced.changed()=>Err(WorkerErrorCode::LeaseExpired),
        _=network.changed()=>Err(WorkerErrorCode::PeerVerificationFailed),
    }
}

async fn prove(
    transport: &Transport,
    identity: &WorkerIdentity,
    request: &ChannelProofRequest,
) -> Result<ChannelProofResponse> {
    let stream = transport
        .connect(
            &identity.host().definition.node_id,
            &identity.host().definition.serve_origin,
        )
        .await
        .map_err(|_| WorkerErrorCode::PeerVerificationFailed)?;
    tokio::time::timeout(Duration::from_secs(3), async {
        use axum::body::Bytes;
        use http_body_util::{BodyExt, Full};
        use hyper_util::rt::TokioIo;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(16 * 1024)
            .max_headers(64)
            .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(|_| WorkerErrorCode::PeerVerificationFailed)?;
        let _connection = TaskGuard(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let body = serde_json::to_vec(request).map_err(|_| WorkerErrorCode::InternalError)?;
        let hostname = identity
            .host()
            .definition
            .serve_origin
            .strip_prefix("https://")
            .ok_or(WorkerErrorCode::PeerVerificationFailed)?;
        let request = hyper::Request::builder()
            .method("POST")
            .uri("/v1/worker/channel-proof")
            .header("Host", hostname)
            .header("Content-Type", "application/json")
            .header("Connection", "close")
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| WorkerErrorCode::InternalError)?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| WorkerErrorCode::PeerVerificationFailed)?;
        if response.status() != StatusCode::OK {
            return Err(WorkerErrorCode::PeerVerificationFailed);
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| WorkerErrorCode::PeerVerificationFailed)?;
            if let Some(data) = frame.data_ref() {
                if data.len() > MAX_WORKER_FRAME_BYTES.saturating_sub(bytes.len()) {
                    return Err(WorkerErrorCode::PeerVerificationFailed);
                }
                bytes.extend_from_slice(data);
            }
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| WorkerErrorCode::PeerVerificationFailed)?;
        worker::decode_proof_response(text).map_err(|_| WorkerErrorCode::PeerVerificationFailed)
    })
    .await
    .map_err(|_| WorkerErrorCode::PeerVerificationFailed)?
}
#[cfg(test)]
mod tests;
