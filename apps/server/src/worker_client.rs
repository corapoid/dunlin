//! Verified worker channel: metadata v2 by default, explicit operator-configured
//! v3/v4/v5 only after real runtime startup and separate session opt-in. No positive replay, credential
//! persistence or network-verification authority is inferred from peer JSON.
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use dunlin_domain::hosts::{HostCapabilities, HostCapabilityPolicy, HostDefinition};
use dunlin_relay_protocol::worker::session::{
    WORKER_SESSION_PROTOCOL_VERSION, WORKER_SESSION_SUBPROTOCOL,
};
use dunlin_relay_protocol::worker::{
    ChannelProofRequest, ChannelProofResponse, ControllerMessage, MAX_WORKER_FRAME_BYTES,
    WORKER_EXECUTION_PROTOCOL_VERSION, WORKER_EXECUTION_SUBPROTOCOL,
    WORKER_EXPORT_PROTOCOL_VERSION, WORKER_EXPORT_SUBPROTOCOL, WorkerMessage,
    decode_controller_frame, decode_proof_request,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(unix)]
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use zeroize::{Zeroize, Zeroizing};
const HANDSHAKE: Duration = Duration::from_secs(15);
const SEND: Duration = Duration::from_secs(5);
const PROOF_TTL: Duration = Duration::from_secs(10);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkConfig {
    pub socket: PathBuf,
    pub node_id: String,
    pub serve_origin: String,
    pub listen: SocketAddr,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerConfig {
    pub node_id: String,
    pub serve_origin: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerConfig {
    #[serde(default)]
    pub exports_state_dir: Option<PathBuf>,
    #[serde(default)]
    pub execution: Option<execution::OperatorExecutionConfig>,
    pub network: NetworkConfig,
    pub controller: ControllerConfig,
    pub host_id: String,
    pub workspace_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub capabilities: HostCapabilities,
}
impl WorkerConfig {
    fn hello(&self, boot: &str) -> WorkerMessage {
        WorkerMessage::Hello {
            protocol_version: WORKER_EXPORT_PROTOCOL_VERSION,
            host_id: self.host_id.clone(),
            workspace_id: self.workspace_id.clone(),
            enrollment_epoch: self.enrollment_epoch,
            credential_revision: self.credential_revision,
            worker_boot_id: boot.into(),
            capabilities: self.capabilities.clone(),
        }
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self
            .exports_state_dir
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err("worker_exports_absolute_state_dir_required");
        }
        execution::validate(self)?;
        self.hello("validate")
            .validate()
            .map_err(|_| "invalid_worker_config")?;
        for (node, origin) in [
            (&self.network.node_id, &self.network.serve_origin),
            (&self.controller.node_id, &self.controller.serve_origin),
        ] {
            HostDefinition {
                name: "Worker transport".into(),
                node_id: node.clone(),
                serve_origin: origin.clone(),
                capability_policy: HostCapabilityPolicy {
                    max_parallel: 1,
                    harnesses: vec![],
                },
            }
            .validate()
            .map_err(|_| "invalid_worker_peer")?;
        }
        if !self.network.listen.ip().is_loopback() || self.network.listen.port() == 0 {
            return Err("worker_proof_requires_loopback_listener");
        }
        if self.network.node_id == self.controller.node_id {
            return Err("worker_controller_must_be_distinct");
        }
        Ok(())
    }
}
struct PendingProof {
    request: ChannelProofRequest,
    response: ChannelProofResponse,
    expires: Instant,
    proven: bool,
}
impl Drop for PendingProof {
    fn drop(&mut self) {
        self.request.nonce.zeroize();
        self.response.nonce.zeroize();
    }
}
/// Only an authenticated current WSS challenge may install a nonce. HTTP proof
/// requests never create state or extend its deadline. No credential is stored.
pub(crate) struct ProofBook {
    host_id: String,
    enrollment_epoch: u64,
    credential_revision: u64,
    worker_boot_id: String,
    pending: Mutex<Option<PendingProof>>,
}
impl ProofBook {
    pub(crate) fn new(config: &WorkerConfig, worker_boot_id: String) -> Self {
        Self {
            host_id: config.host_id.clone(),
            enrollment_epoch: config.enrollment_epoch,
            credential_revision: config.credential_revision,
            worker_boot_id,
            pending: Mutex::new(None),
        }
    }
    fn install(&self, request: ChannelProofRequest, now: Instant) -> Result<(), &'static str> {
        request.validate().map_err(|_| "invalid_worker_challenge")?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "worker_proof_unavailable")?;
        if pending.is_some() {
            return Err("worker_challenge_already_pending");
        }
        let response = ChannelProofResponse {
            channel_id: request.channel_id.clone(),
            controller_boot_id: request.controller_boot_id.clone(),
            nonce: request.nonce.clone(),
            worker_boot_id: self.worker_boot_id.clone(),
            host_id: self.host_id.clone(),
            enrollment_epoch: self.enrollment_epoch,
            credential_revision: self.credential_revision,
        };
        response
            .validate()
            .map_err(|_| "invalid_worker_proof_identity")?;
        *pending = Some(PendingProof {
            request,
            response,
            expires: now + PROOF_TTL,
            proven: false,
        });
        Ok(())
    }
    fn prove(
        &self,
        request: &ChannelProofRequest,
        now: Instant,
    ) -> Result<ChannelProofResponse, &'static str> {
        request
            .validate()
            .map_err(|_| "invalid_worker_proof_request")?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "worker_proof_unavailable")?;
        if pending.as_ref().is_some_and(|p| now >= p.expires) {
            *pending = None;
        }
        let proof = pending.as_mut().ok_or("worker_proof_not_pending")?;
        if proof.request != *request {
            return Err("worker_proof_mismatch");
        }
        proof.proven = true;
        Ok(proof.response.clone())
    }
    fn accepted(&self) -> Result<(), &'static str> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "worker_proof_unavailable")?;
        if !pending
            .as_ref()
            .is_some_and(|p| p.proven && Instant::now() < p.expires)
        {
            *pending = None;
            return Err("worker_proof_not_completed");
        }
        *pending = None;
        Ok(())
    }
    pub(crate) fn clear(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = None;
        }
    }
}
struct ClearProof(Arc<ProofBook>);
impl Drop for ClearProof {
    fn drop(&mut self) {
        self.0.clear();
    }
}
async fn proof(State(book): State<Arc<ProofBook>>, headers: HeaderMap, body: Bytes) -> Response {
    if [header::COOKIE, header::ORIGIN, header::AUTHORIZATION]
        .iter()
        .any(|h| headers.contains_key(h))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let request = std::str::from_utf8(&body)
        .ok()
        .and_then(|s| decode_proof_request(s).ok());
    let response = request
        .as_ref()
        .and_then(|r| book.prove(r, Instant::now()).ok());
    match response {
        Some(value) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(value)).into_response(),
        None => StatusCode::FORBIDDEN.into_response(),
    }
}
pub(crate) fn proof_router(book: Arc<ProofBook>) -> Router {
    proof_router_with_limit(book, Arc::new(Semaphore::new(8)))
}
fn proof_router_with_limit(book: Arc<ProofBook>, requests: Arc<Semaphore>) -> Router {
    Router::new()
        .route("/v1/worker/channel-proof", post(proof))
        .layer(DefaultBodyLimit::max(MAX_WORKER_FRAME_BYTES))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let requests = requests.clone();
                async move {
                    let Ok(permit) = requests.try_acquire_owned() else {
                        return StatusCode::TOO_MANY_REQUESTS.into_response();
                    };
                    let response = tokio::time::timeout(SEND, next.run(request))
                        .await
                        .unwrap_or_else(|_| StatusCode::REQUEST_TIMEOUT.into_response());
                    // Keep the slot through response-body completion or cancellation,
                    // not merely until the extractor/handler has produced headers.
                    let (parts, body) = response.into_parts();
                    let stream = futures_util::stream::unfold(
                        (body.into_data_stream(), permit),
                        |(mut stream, permit)| async move {
                            stream.next().await.map(|chunk| (chunk, (stream, permit)))
                        },
                    );
                    Response::from_parts(parts, Body::from_stream(stream))
                }
            },
        ))
        .with_state(book)
}

struct ProofIo {
    stream: tokio::net::TcpStream,
    _connection: OwnedSemaphorePermit,
}
impl AsyncRead for ProofIo {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for ProofIo {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
struct ProofListener {
    listener: tokio::net::TcpListener,
    connections: Arc<Semaphore>,
}
impl ProofListener {
    fn new(listener: tokio::net::TcpListener) -> Self {
        Self {
            listener,
            connections: Arc::new(Semaphore::new(64)),
        }
    }
}
impl axum::serve::Listener for ProofListener {
    type Io = ProofIo;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((stream, address)) => match self.connections.clone().try_acquire_owned() {
                    Ok(slot) => {
                        return (
                            ProofIo {
                                stream,
                                _connection: slot,
                            },
                            address,
                        );
                    }
                    Err(_) => {
                        drop(stream);
                        tokio::task::yield_now().await;
                    }
                },
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
fn valid_credential(value: &str) -> bool {
    value.len() == 47
        && value.starts_with("dhw_")
        && value[4..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(crate) async fn command(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) == Some("export") {
        return crate::worker_exports::command(&args[1..]).await;
    }
    if args.len() != 4
        || args[0] != "run"
        || args[1] != "--config"
        || args[3] != "--credential-stdin"
    {
        return Err("usage: worker run --config PATH --credential-stdin".into());
    }
    let config = load_config(std::path::Path::new(&args[2]))
        .await
        .map_err(str::to_owned)?;
    let credential = credential_stdin().await.map_err(str::to_owned)?;
    run(config, credential).await.map_err(str::to_owned)
}

/// Operator-local regular files only. Nonblocking open rejects FIFOs without
/// creating an uncancellable Tokio blocking-open task; O_NOFOLLOW rejects a
/// symlink at the supplied filename. Local filesystem kernel stalls are outside
/// this adapter's timing guarantee, so this is not a network-filesystem loader.
#[cfg(unix)]
fn open_config_file(path: &std::path::Path) -> Result<std::fs::File, &'static str> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "worker_config_unreadable")?;
    let metadata = file.metadata().map_err(|_| "worker_config_unreadable")?;
    if !metadata.is_file() {
        return Err("worker_config_regular_file_required");
    }
    if metadata.len() > 64 * 1024 {
        return Err("worker_config_too_large");
    }
    Ok(file)
}

#[cfg(unix)]
pub(crate) async fn load_config(path: &std::path::Path) -> Result<WorkerConfig, &'static str> {
    let file = tokio::fs::File::from_std(open_config_file(path)?);
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "worker_config_unreadable")?;
    if bytes.len() > 64 * 1024 {
        return Err("worker_config_too_large");
    }
    let config: WorkerConfig =
        toml::from_str(std::str::from_utf8(&bytes).map_err(|_| "invalid_worker_config")?)
            .map_err(|_| "invalid_worker_config")?;
    config.validate()?;
    Ok(config)
}

#[cfg(not(unix))]
pub(crate) async fn load_config(_path: &std::path::Path) -> Result<WorkerConfig, &'static str> {
    Err("worker_config_loader_unsupported")
}

/// Stdin is bounded and interruptible; a timed-out blocking stdin task must not
/// keep the Tokio runtime alive. Only the standalone CLI uses this fd adapter.
#[cfg(unix)]
async fn credential_stdin() -> Result<Zeroizing<String>, &'static str> {
    use std::{
        io::Read,
        os::fd::{AsRawFd, FromRawFd},
    };
    struct Input {
        file: std::fs::File,
        flags: i32,
    }
    impl Drop for Input {
        fn drop(&mut self) {
            unsafe {
                libc::fcntl(self.file.as_raw_fd(), libc::F_SETFL, self.flags);
            }
        }
    }
    let fd = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err("worker_credential_stdin_unavailable");
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("worker_credential_stdin_unavailable");
    }
    let mut input = Input { file, flags };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut buffer = Zeroizing::new(Vec::new());
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("worker_credential_stdin_timeout");
        }
        let mut part = Zeroizing::new([0u8; 128]);
        match input.file.read(&mut *part) {
            Ok(size) => {
                buffer.extend_from_slice(&part[..size]);
                if buffer.len() > 128 {
                    return Err("invalid_worker_credential");
                }
                if size == 0 || buffer.contains(&b'\n') {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(5)).await
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err("worker_credential_stdin_unavailable"),
        }
    }
    let mut value = Zeroizing::new(
        std::str::from_utf8(&buffer)
            .map_err(|_| "invalid_worker_credential")?
            .to_owned(),
    );
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    if !valid_credential(&value) {
        return Err("invalid_worker_credential");
    }
    Ok(value)
}
#[cfg(not(unix))]
async fn credential_stdin() -> Result<Zeroizing<String>, &'static str> {
    Err("worker_credential_stdin_unsupported")
}

pub(crate) async fn run(
    config: WorkerConfig,
    credential: Zeroizing<String>,
) -> Result<(), &'static str> {
    config.validate()?;
    let transport = crate::tailscale::Transport::new(
        config.network.socket.clone(),
        config.network.node_id.clone(),
        config.network.serve_origin.clone(),
        config.network.listen,
    )?;
    run_transport(config, credential, transport).await
}

/// Tests may provision their own certificate trust roots on the real transport.
/// Listener, proof service, handshake and lease loop remain production code.
#[cfg(test)]
pub(crate) async fn run_with_transport(
    config: WorkerConfig,
    credential: Zeroizing<String>,
    transport: crate::tailscale::Transport,
) -> Result<(), &'static str> {
    run_transport(config, credential, transport).await
}

async fn run_transport(
    config: WorkerConfig,
    credential: Zeroizing<String>,
    transport: crate::tailscale::Transport,
) -> Result<(), &'static str> {
    config.validate()?;
    if !valid_credential(&credential) {
        return Err("invalid_worker_credential");
    }
    let boot = uuid::Uuid::now_v7().to_string();
    let runtime = execution::start(&config, &boot).await?;
    let book = Arc::new(ProofBook::new(&config, boot.clone()));
    let _clear = ClearProof(book.clone());
    let listener = tokio::net::TcpListener::bind(config.network.listen)
        .await
        .map_err(|_| "worker_proof_bind_failed")?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app = proof_router(book.clone());
    let mut server = tokio::spawn(async move {
        axum::serve(ProofListener::new(listener), app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    // Cancellation of run must stop the listener even if an HTTP proof client
    // stalls a graceful shutdown with an unfinished request body.
    struct AbortServer(tokio::task::AbortHandle);
    impl Drop for AbortServer {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server_owner = AbortServer(server.abort_handle());
    let result = async {
        let stream = tokio::time::timeout(
            HANDSHAKE,
            transport.connect(&config.controller.node_id, &config.controller.serve_origin),
        )
        .await
        .map_err(|_| "worker_transport_timeout")??;
        // Local Serve/Funnel state can change while the TLS/WebSocket remains
        // open. Revalidation runs independently of sends, ACK waits and leases.
        tokio::select! {
            result = connect_channel(&config, credential, stream, book.clone(), boot, runtime.clone()) => result,
            result = watch_network(&transport, &config.controller) => result,
        }
    }
    .await;
    if let Some(runtime) = runtime {
        runtime.shutdown().await;
    }
    book.clear();
    server.abort();
    let _ = stop.send(());
    if tokio::time::timeout(Duration::from_secs(1), &mut server)
        .await
        .is_err()
    {
        server.abort();
    }
    result
}

async fn watch_network(
    transport: &crate::tailscale::Transport,
    controller: &ControllerConfig,
) -> Result<(), &'static str> {
    let period = Duration::from_secs(5);
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        tokio::time::timeout(
            Duration::from_secs(3),
            transport.verify_peer(&controller.node_id, &controller.serve_origin),
        )
        .await
        .map_err(|_| "worker_network_guard_timeout")??;
    }
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: WorkerMessage,
) -> Result<(), &'static str> {
    message.validate().map_err(|_| "invalid_worker_message")?;
    let text = serde_json::to_string(&message).map_err(|_| "invalid_worker_message")?;
    if text.len() > MAX_WORKER_FRAME_BYTES {
        return Err("worker_frame_too_large");
    }
    tokio::time::timeout(SEND, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| "worker_send_timeout")?
        .map_err(|_| "worker_channel_closed")
}
async fn send_before<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: WorkerMessage,
    deadline: tokio::time::Instant,
) -> Result<(), &'static str> {
    if tokio::time::Instant::now() >= deadline {
        return Err("worker_lease_expired");
    }
    tokio::time::timeout_at(deadline, send(socket, message))
        .await
        .map_err(|_| "worker_lease_expired")?
}
async fn receive<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    deadline: tokio::time::Instant,
) -> Result<ControllerMessage, &'static str> {
    let mut control_frames = 0;
    loop {
        // timeout_at may poll an already-ready inner future first. Check the
        // absolute lease boundary even when a peer keeps frames buffered.
        if tokio::time::Instant::now() >= deadline {
            return Err("worker_channel_deadline");
        }
        if control_frames >= 120 {
            return Err("worker_controller_rate_limited");
        }
        control_frames += 1;
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| "worker_channel_deadline")?
            .ok_or("worker_channel_closed")?
            .map_err(|_| "worker_channel_closed")?;
        if tokio::time::Instant::now() >= deadline {
            return Err("worker_channel_deadline");
        }
        match message {
            Message::Text(text) => {
                return decode_controller_frame(&text).map_err(|_| "invalid_controller_message");
            }
            Message::Ping(bytes) => {
                tokio::time::timeout(
                    SEND.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                    socket.send(Message::Pong(bytes)),
                )
                .await
                .map_err(|_| "worker_send_timeout")?
                .map_err(|_| "worker_channel_closed")?;
            }
            Message::Pong(_) => {}
            _ => return Err("invalid_controller_frame"),
        }
    }
}
fn selected_protocol(
    config: &WorkerConfig,
    runtime: Option<&crate::worker_execution::runtime::Runtime>,
) -> Result<(u16, &'static str), &'static str> {
    execution::validate(config)?;
    if config.execution.as_ref().is_some_and(|c| c.enabled) != runtime.is_some() {
        return Err("worker_execution_runtime_required");
    }
    if let Some(execution) = config.execution.as_ref().filter(|e| e.persistent_sessions) {
        if !runtime.is_some_and(|runtime| runtime.supports_sessions()) {
            return Err("worker_session_runtime_required");
        }
        if execution.mcp_broker {
            use dunlin_relay_protocol::worker::session::broker::*;
            return Ok((WORKER_BROKER_PROTOCOL_VERSION, WORKER_BROKER_SUBPROTOCOL));
        }
        return Ok((WORKER_SESSION_PROTOCOL_VERSION, WORKER_SESSION_SUBPROTOCOL));
    }
    Ok(if runtime.is_some() {
        (
            WORKER_EXECUTION_PROTOCOL_VERSION,
            WORKER_EXECUTION_SUBPROTOCOL,
        )
    } else {
        (WORKER_EXPORT_PROTOCOL_VERSION, WORKER_EXPORT_SUBPROTOCOL)
    })
}
async fn connect_channel<S: AsyncRead + AsyncWrite + Unpin>(
    config: &WorkerConfig,
    mut credential: Zeroizing<String>,
    stream: S,
    book: Arc<ProofBook>,
    boot: String,
    runtime: Option<Arc<crate::worker_execution::runtime::Runtime>>,
) -> Result<(), &'static str> {
    let (_, subprotocol) = selected_protocol(config, runtime.as_deref())?;
    let mut request = format!(
        "{}/v1/worker/channel",
        config
            .controller
            .serve_origin
            .replacen("https://", "wss://", 1)
    )
    .into_client_request()
    .map_err(|_| "invalid_worker_endpoint")?;
    let mut bearer = Zeroizing::new(format!("Bearer {}", *credential));
    let mut authorization: axum::http::HeaderValue =
        bearer.parse().map_err(|_| "invalid_worker_credential")?;
    authorization.set_sensitive(true);
    request.headers_mut().insert("authorization", authorization);
    request.headers_mut().insert(
        "sec-websocket-protocol",
        subprotocol.parse().map_err(|_| "invalid_worker_protocol")?,
    );
    credential.zeroize();
    bearer.zeroize();
    let ws = WebSocketConfig::default()
        .max_message_size(Some(MAX_WORKER_FRAME_BYTES))
        .max_frame_size(Some(MAX_WORKER_FRAME_BYTES));
    let (mut socket, response) = tokio::time::timeout(
        HANDSHAKE,
        tokio_tungstenite::client_async_with_config(request, stream, Some(ws)),
    )
    .await
    .map_err(|_| "worker_handshake_timeout")?
    .map_err(|_| "worker_handshake_failed")?;
    let mut selected = response.headers().get_all("sec-websocket-protocol").iter();
    if response.status() != StatusCode::SWITCHING_PROTOCOLS
        || selected.next().and_then(|h| h.to_str().ok()) != Some(subprotocol)
        || selected.next().is_some()
    {
        return Err("worker_subprotocol_required");
    }
    session_with_runtime(config, &mut socket, book, boot, runtime).await
}
#[cfg(test)]
async fn session<S: AsyncRead + AsyncWrite + Unpin>(
    config: &WorkerConfig,
    socket: &mut WebSocketStream<S>,
    book: Arc<ProofBook>,
    boot: String,
) -> Result<(), &'static str> {
    session_with_runtime(config, socket, book, boot, None).await
}
async fn session_with_runtime<S: AsyncRead + AsyncWrite + Unpin>(
    config: &WorkerConfig,
    socket: &mut WebSocketStream<S>,
    book: Arc<ProofBook>,
    boot: String,
    runtime: Option<Arc<crate::worker_execution::runtime::Runtime>>,
) -> Result<(), &'static str> {
    let (selected_version, _) = selected_protocol(config, runtime.as_deref())?;
    let _clear = ClearProof(book.clone());
    let challenge_started = tokio::time::Instant::now();
    let mut hello = config.hello(&boot);
    let persistent = config
        .execution
        .as_ref()
        .is_some_and(|execution| execution.persistent_sessions);
    if runtime.is_some()
        && let WorkerMessage::Hello {
            protocol_version, ..
        } = &mut hello
    {
        *protocol_version = selected_version;
    }
    let broker_limits = (selected_version
        == dunlin_relay_protocol::worker::session::broker::WORKER_BROKER_PROTOCOL_VERSION)
        .then_some(
            dunlin_relay_protocol::worker::session::broker::BrokerLimits {
                max_pending: 8,
                max_reply_bytes: dunlin_relay_protocol::worker::session::broker::MAX_REPLY_BYTES
                    as u32,
            },
        );
    if let Some(limits) = &broker_limits {
        let hello = dunlin_relay_protocol::worker::session::broker::BrokerHello {
            protocol_version: selected_version,
            host_id: config.host_id.clone(),
            workspace_id: config.workspace_id.clone(),
            enrollment_epoch: config.enrollment_epoch,
            credential_revision: config.credential_revision,
            worker_boot_id: boot.clone(),
            capabilities: config.capabilities.clone(),
            broker_limits: limits.clone(),
        }
        .encode()
        .map_err(|_| "invalid_broker_hello")?;
        tokio::time::timeout(SEND, socket.send(Message::Text(hello.into())))
            .await
            .map_err(|_| "worker_send_timeout")?
            .map_err(|_| "worker_channel_closed")?;
    } else {
        send(socket, hello).await?;
    }
    let (channel, controller, nonce, interval, ttl) =
        match receive(socket, challenge_started + HANDSHAKE).await? {
            ControllerMessage::Challenge {
                channel_id,
                controller_boot_id,
                nonce,
                heartbeat_interval_ms,
                lease_ttl_ms,
            } => (
                channel_id,
                controller_boot_id,
                nonce,
                Duration::from_millis(heartbeat_interval_ms),
                Duration::from_millis(lease_ttl_ms),
            ),
            _ => return Err("worker_challenge_required"),
        };
    book.install(
        ChannelProofRequest {
            channel_id: channel.clone(),
            controller_boot_id: controller.clone(),
            nonce: nonce.clone(),
        },
        Instant::now(),
    )?;
    let ready_at = tokio::time::Instant::now();
    send(
        socket,
        WorkerMessage::Ready {
            channel_id: channel.clone(),
            controller_boot_id: controller.clone(),
            worker_boot_id: boot.clone(),
            nonce,
        },
    )
    .await?;
    let lease_epoch = match receive(socket, (ready_at + PROOF_TTL).min(ready_at + ttl)).await? {
        ControllerMessage::Accepted {
            channel_id,
            controller_boot_id,
            worker_boot_id,
            lease_epoch,
            ..
        } if channel_id == channel
            && controller_boot_id == controller
            && worker_boot_id == boot =>
        {
            lease_epoch
        }
        _ => return Err("worker_acceptance_mismatch"),
    };
    book.accepted()?;
    let mut deadline = ready_at + ttl;
    let fence = dunlin_relay_protocol::worker::ChannelFence {
        channel_id: channel.clone(),
        controller_boot_id: controller.clone(),
        worker_boot_id: boot.clone(),
    };
    let (mut session_requests, mut broker_owner) = if persistent {
        let runtime = runtime
            .as_ref()
            .ok_or("worker_session_runtime_required")?
            .clone();
        match broker_limits {
            Some(limits) => {
                let (requests, owner) =
                    sessions::Requests::new_broker(runtime, fence, deadline, limits)?;
                (Some(requests), Some(owner))
            }
            None => (
                Some(sessions::Requests::new(runtime, fence, deadline)?),
                None,
            ),
        }
    } else {
        (None, None)
    };
    #[cfg(test)]
    let _broker_probe = broker_owner
        .as_ref()
        .map(|owner| broker::fixture::publish(&config.host_id, owner));
    let mut broker_tick = tokio::time::interval(Duration::from_millis(100));
    let mut execution = runtime
        .filter(|_| !persistent)
        .map(|runtime| {
            execution::Requests::new(
                runtime,
                dunlin_relay_protocol::worker::ChannelFence {
                    channel_id: channel.clone(),
                    controller_boot_id: controller.clone(),
                    worker_boot_id: boot.clone(),
                },
                deadline,
            )
        })
        .transpose()?;
    let mut outbox_tick = tokio::time::interval(Duration::from_millis(500));
    let mut next = ready_at + interval;
    let mut sequence = 0u64;
    let mut pending_heartbeat = None;
    let mut requests = exports::Requests::new(config);
    let mut window = tokio::time::Instant::now();
    let mut frames = 0usize;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("worker_lease_expired");
        }
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => return Err("worker_lease_expired"),
            _ = tokio::time::sleep_until(next), if pending_heartbeat.is_none() => {
                sequence = sequence.checked_add(1).filter(|s| *s <= i64::MAX as u64).ok_or("worker_sequence_exhausted")?;
                let sent_at = tokio::time::Instant::now();
                send_before(socket, WorkerMessage::Heartbeat { channel_id: channel.clone(), lease_epoch, sequence }, deadline).await?;
                pending_heartbeat = Some((sequence, sent_at));
                next = sent_at + interval;
            }
            _ = broker_tick.tick(), if broker_owner.is_some() => {
                broker_owner.as_mut().ok_or("worker_broker_owner_missing")?.expire()
                    .map_err(|_| "worker_broker_expired")?;
            }
            frame = socket.next() => {
                let now = tokio::time::Instant::now();
                if now >= deadline { return Err("worker_lease_expired"); }
                if now.duration_since(window) >= Duration::from_secs(60) { window = now; frames = 0; }
                frames += 1;
                if frames > dunlin_relay_protocol::worker::worker_rate_limit(selected_version) { return Err("worker_controller_rate_limited"); }
                let frame = frame.ok_or("worker_channel_closed")?.map_err(|_| "worker_channel_closed")?;
                let message = match frame {
                    Message::Text(text) => {
                        if let Ok(message)=decode_controller_frame(&text) { message }
                        else {
                            if let Some(owner) = broker_owner.as_mut()
                                && owner.accept_frame(&text).map_err(|_| "invalid_worker_broker_reply")? {
                                continue;
                            }
                            if let (Some(owner),Some(sessions)) = (broker_owner.as_ref(),session_requests.as_mut())
                                && let Ok(message) = dunlin_relay_protocol::worker::session::broker::decode_broker_session_controller_frame(
                                    text.as_bytes(), &dunlin_relay_protocol::worker::ChannelFence {channel_id:channel.clone(),controller_boot_id:controller.clone(),worker_boot_id:boot.clone()}, owner.limits()) {
                                let dunlin_relay_protocol::worker::session::broker::BrokerSessionControllerMessage::OpenBrokerSession {command} = message;
                                #[cfg(test)]
                                session_faults::incoming_broker(&config.host_id,&command);
                                if let Some(reply) = sessions.enqueue_broker(*command)? {send_session_before(socket,reply,deadline).await?;}
                                continue;
                            }
                            if let Some(sessions) = session_requests.as_mut() {
                                let message = dunlin_relay_protocol::worker::session::decode_session_controller_frame(&text).map_err(|_| "invalid_session_message")?;
                                #[cfg(test)]
                                session_faults::incoming(&config.host_id,&message);
                                if let Some(reply) = sessions.enqueue(message)? { send_session_before(socket, reply, deadline).await?; }
                            } else {
                                let message=dunlin_relay_protocol::worker::decode_execution_controller_frame(&text).map_err(|_|"invalid_controller_message")?;
                                let execution=execution.as_mut().ok_or("worker_execution_protocol_required")?;
                                if let Some(reply)=execution.enqueue(message)? { send_execution_before(socket,reply,deadline).await?; }
                            }
                            continue;
                        }
                    },
                    Message::Ping(bytes) => {
                        tokio::time::timeout_at(deadline.min(now + SEND), socket.send(Message::Pong(bytes))).await.map_err(|_| "worker_send_timeout")?.map_err(|_| "worker_channel_closed")?;
                        continue;
                    }
                    Message::Pong(_) => continue,
                    _ => return Err("invalid_controller_frame"),
                };
                match message {
                    ControllerMessage::HeartbeatAck { channel_id, lease_epoch: epoch, sequence: ack, .. } if channel_id == channel && epoch == lease_epoch => {
                        let (expected, sent_at) = pending_heartbeat.take().ok_or("worker_heartbeat_ack_mismatch")?;
                        if ack != expected { return Err("worker_heartbeat_ack_mismatch"); }
                        // A delayed ACK never grants a fresh TTL measured at arrival.
                        deadline = sent_at + ttl;
                        if let Some(execution)=&execution { execution.deadline(deadline)?; }
                        if let Some(sessions)=&session_requests { sessions.deadline(deadline)?; }
                    }
                    ControllerMessage::DescribeExport { request_id, channel_id, lease_epoch: epoch, export_id, principal, expected_export_revision } if channel_id == channel && epoch == lease_epoch => {
                        if let Some(code) = requests.enqueue(request_id.clone(), export_id, principal, expected_export_revision)? {
                            send_before(socket, WorkerMessage::ExportError { request_id, channel_id: channel.clone(), lease_epoch, code }, deadline).await?;
                        }
                    }
                    ControllerMessage::HeartbeatAck { .. } => return Err("worker_heartbeat_ack_mismatch"),
                    _ => return Err("worker_controller_message_mismatch"),
                }
            }
            outgoing = async {broker_owner.as_mut().expect("guarded broker branch").next().await}, if broker_owner.is_some() => {
                let outgoing = outgoing.map_err(|_| "worker_broker_closed")?;
                send_broker_before(socket, outgoing.message, deadline.min(outgoing.deadline)).await?;
            }
            _=outbox_tick.tick(), if execution.is_some() || session_requests.is_some()=>{
                if let Some(execution)=execution.as_mut() { execution.poll_outbox(); }
                if let Some(sessions)=session_requests.as_mut() { sessions.poll_outbox(); }
            }
            replies=async {session_requests.as_mut().expect("guarded session branch").completed().await}, if session_requests.as_ref().is_some_and(|sessions| sessions.pending())=>{
                #[cfg(test)]
                let replies=session_faults::outgoing(&config.host_id,replies?);
                #[cfg(not(test))]
                let replies=replies?;
                for reply in replies {
                    #[cfg(test)]
                    if !session_faults::before_send(&config.host_id,&reply).await {continue;}
                    send_session_before(socket, reply, deadline).await?;
                }
            }
            replies=async {execution.as_mut().expect("guarded execution branch").completed().await}, if execution.as_ref().is_some_and(|e|e.pending())=>{
                for reply in replies? { send_execution_before(socket,reply,deadline).await?; }
            }
            result = requests.completed(), if requests.pending() => {
                let (request_id, result) = result?;
                let message = match result {
                    Ok(descriptor) => WorkerMessage::ExportDescribed { request_id, channel_id: channel.clone(), lease_epoch, descriptor },
                    Err(code) => WorkerMessage::ExportError { request_id, channel_id: channel.clone(), lease_epoch, code },
                };
                send_before(socket, message, deadline).await?;
            }
        }
    }
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "v5 socket owner and runner proxy integration is in progress"
    )
)]
pub(crate) mod broker;
mod execution;
#[cfg(test)]
pub(crate) mod session_faults;
mod sessions;
async fn send_broker_before<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: dunlin_relay_protocol::worker::session::broker::BrokerWorkerMessage,
    deadline: tokio::time::Instant,
) -> Result<(), &'static str> {
    if tokio::time::Instant::now() >= deadline {
        return Err("worker_broker_deadline");
    }
    message
        .validate()
        .map_err(|_| "invalid_worker_broker_request")?;
    let text = serde_json::to_string(&message).map_err(|_| "invalid_worker_broker_request")?;
    tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + SEND),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| "worker_send_timeout")?
    .map_err(|_| "worker_channel_closed")
}
async fn send_session_before<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: dunlin_relay_protocol::worker::session::SessionWorkerMessage,
    deadline: tokio::time::Instant,
) -> Result<(), &'static str> {
    if tokio::time::Instant::now() >= deadline {
        return Err("worker_lease_expired");
    }
    message.validate().map_err(|_| "invalid_session_reply")?;
    let text = serde_json::to_string(&message).map_err(|_| "invalid_session_reply")?;
    tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + SEND),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| "worker_send_timeout")?
    .map_err(|_| "worker_channel_closed")
}
async fn send_execution_before<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: dunlin_relay_protocol::worker::ExecutionWorkerMessage,
    deadline: tokio::time::Instant,
) -> Result<(), &'static str> {
    if tokio::time::Instant::now() >= deadline {
        return Err("worker_lease_expired");
    }
    message.validate().map_err(|_| "invalid_execution_reply")?;
    let text = serde_json::to_string(&message).map_err(|_| "invalid_execution_reply")?;
    tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + SEND),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| "worker_send_timeout")?
    .map_err(|_| "worker_channel_closed")
}
mod exports;
#[cfg(test)]
mod tests;
