//! ACP harness integration for Dunlin.
//!
//! The runtime boundary is deliberately small: a worker thread owns the ACP
//! connection, the UI sends [`HarnessCommand`]s and receives [`HarnessEvent`]s
//! over channels. That keeps protocol I/O and child-process supervision off the
//! GPUI foreground thread while still allowing permission prompts to round-trip
//! through the UI.
//!
//! Prompt requests are spawned on the connection so the command loop stays
//! responsive. Permission requests are registered in a pending map and answered
//! from a spawned task, never while holding up the ACP dispatch loop.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub mod acceptance;
mod models;
/// Build a verified TLS client with only the explicitly supplied private CA.
pub fn relay_tls_config(pem: &[u8]) -> Result<Arc<rustls::ClientConfig>, String> {
    use rustls::pki_types::{CertificateDer, pem::PemObject};
    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(pem) {
        roots
            .add(cert.map_err(|_| "Invalid relay CA")?)
            .map_err(|_| "Invalid relay CA")?;
    }
    if roots.is_empty() {
        return Err("Empty relay CA".into());
    }
    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}
pub use models::{ModelChoice, SessionModels, SessionModes, SessionOption};
use models::{session_models, session_modes, session_options};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AvailableCommandsUpdate, CancelNotification, ContentBlock, EnvVariable, InitializeRequest,
    LoadSessionRequest, McpServer, McpServerStdio, NewSessionRequest, PermissionOptionKind,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, UsageUpdate,
};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, Error as AcpError, Lines, Result as AcpResult,
    UntypedMessage,
};
use async_channel::{Receiver, Sender};
use dunlin_domain::{ToolKind, ToolStatus};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

/// How the ACP connection reaches its agent.
#[derive(Clone, Debug)]
pub enum HarnessTransport {
    /// Spawn a local process and speak ACP over stdio.
    Local { command: String },
    /// Connect to a relay over WebSocket and speak ACP over text frames.
    Remote {
        url: String,
        token: String,
        project_id: String,
        /// Relay project ids explicitly granted for this group conversation.
        roots: Vec<String>,
    },
}

/// One additional MCP server passed to a session over ACP. Stdio is mandatory
/// for ACP agents, so this is the compatibility baseline.
#[derive(Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl McpServerConfig {
    pub fn stdio(
        name: impl Into<String>,
        command: impl Into<PathBuf>,
        args: Vec<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            env,
        }
    }
}

/// Everything needed to start one ACP agent process or remote connection.
#[derive(Clone)]
pub struct HarnessConfig {
    pub transport: HarnessTransport,
    /// Session working directory. Must be absolute. For remote sessions this is
    /// the path on the relay.
    pub cwd: PathBuf,
    /// Client name reported during `initialize`.
    pub client_name: String,
    pub resume_session_id: Option<String>,
    /// Additional environment for the local child only; never mutates the app.
    pub process_env: HashMap<String, String>,
    /// Apply only to new sessions, before accepting prompts. Never overrides a
    /// resumed session or falls back to a different model on failure.
    pub default_model: Option<String>,
    /// Explicit extra workspace roots for the initial session. Callers can also
    /// pass roots per command; this keeps the first automatic session scoped
    /// when a group conversation was already attached.
    pub additional_directories: Vec<PathBuf>,
    /// Per-session MCP servers. The vault proxy is attached here.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Optional local diagnostics. Protocol payloads must be filtered by the sink.
    pub diagnostics: Option<DiagnosticSink>,
    /// Explicit additional trust anchor; certificate/name verification remains on.
    /// Manual acceptance supplies this per connection, never via global trust.
    pub tls_root_pem: Option<Vec<u8>>,
}

pub type DiagnosticSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

impl HarnessConfig {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            transport: HarnessTransport::Local {
                command: command.into(),
            },
            cwd: cwd.into(),
            client_name: "Dunlin".into(),
            resume_session_id: None,
            process_env: HashMap::new(),
            default_model: None,
            additional_directories: Vec::new(),
            mcp_servers: Vec::new(),
            diagnostics: None,
            tls_root_pem: None,
        }
    }

    pub fn remote(
        url: impl Into<String>,
        token: impl Into<String>,
        project_id: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self::remote_with_roots(url, token, project_id, cwd, Vec::new())
    }

    pub fn remote_with_roots(
        url: impl Into<String>,
        token: impl Into<String>,
        project_id: impl Into<String>,
        cwd: impl Into<PathBuf>,
        roots: Vec<String>,
    ) -> Self {
        Self {
            transport: HarnessTransport::Remote {
                url: url.into(),
                token: token.into(),
                project_id: project_id.into(),
                roots,
            },
            cwd: cwd.into(),
            client_name: "Dunlin".into(),
            resume_session_id: None,
            process_env: HashMap::new(),
            default_model: None,
            additional_directories: Vec::new(),
            mcp_servers: Vec::new(),
            diagnostics: None,
            tls_root_pem: None,
        }
    }

    pub fn with_additional_directories(mut self, roots: Vec<PathBuf>) -> Self {
        self.additional_directories = roots;
        self
    }

    pub fn with_mcp_server(mut self, server: McpServerConfig) -> Self {
        self.mcp_servers.push(server);
        self
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.transport, HarnessTransport::Remote { .. })
    }

    pub fn description(&self) -> String {
        match &self.transport {
            HarnessTransport::Local { command } => command.clone(),
            HarnessTransport::Remote { url, .. } => format!("relay {url}"),
        }
    }
}

/// A tool call as rendered by the chat surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallView {
    pub id: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub detail: String,
    pub locations: Vec<String>,
    pub additions: u32,
    pub deletions: u32,
    /// Unified diff supplied by ACP, when the tool changed a file.
    #[serde(default)]
    pub diff: Option<String>,
}

/// Partial update for a tool call that already exists in the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub detail: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub content: String,
    pub status: PlanStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionChoice {
    pub id: String,
    pub label: String,
    pub kind: PermissionKind,
}

/// Streaming updates mapped away from the wire schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HarnessUpdate {
    UserMessage(String),
    AgentMessage(String),
    AgentThought(String),
    MessageChunk {
        id: String,
        text: String,
        thought: bool,
    },
    Child {
        id: String,
        parent_id: String,
        title: Option<String>,
        update: Box<HarnessUpdate>,
    },
    ToolCall(ToolCallView),
    ToolCallUpdate(ToolCallDelta),
    Plan(Vec<PlanItem>),
    Usage {
        used: u64,
        size: u64,
        cost_usd: Option<f64>,
    },
    AvailableCommands(Vec<String>),
    Configuration {
        models: Option<SessionModels>,
        modes: Option<SessionModes>,
        #[serde(default)]
        options: Option<Vec<SessionOption>>,
    },
}

/// Events emitted by the harness worker.
/// Optional cumulative counters from ACP's end-turn usage extension. These are
/// not the context-window `used` count. Missing data must remain unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HarnessEvent {
    Ready {
        agent: String,
        version: String,
    },
    SessionCreated {
        session_id: String,
    },
    SessionLoaded {
        session_id: String,
    },
    SessionLoadFailed {
        session_id: String,
        message: String,
    },
    Models {
        session_id: String,
        state: Option<SessionModels>,
    },
    ModelChangeFailed {
        session_id: String,
        message: String,
    },
    Modes {
        session_id: String,
        state: Option<SessionModes>,
    },
    ModeChangeFailed {
        session_id: String,
        message: String,
    },
    ConfigOptions {
        session_id: String,
        options: Vec<SessionOption>,
    },
    ConfigOptionChangeFailed {
        session_id: String,
        message: String,
    },
    Update {
        session_id: String,
        update: HarnessUpdate,
    },
    PermissionRequested {
        request_id: String,
        session_id: String,
        title: String,
        options: Vec<PermissionChoice>,
    },
    PromptFinished {
        session_id: String,
        stop_reason: String,
        #[serde(default)]
        usage: Option<TokenUsage>,
    },
    Error {
        message: String,
    },
    Exited {
        reason: String,
    },
}

/// Commands accepted by a running harness.
#[derive(Clone, Debug)]
pub enum HarnessCommand {
    Prompt {
        text: String,
        continuation: Option<String>,
    },
    Cancel,
    NewSession {
        cwd: PathBuf,
        additional_directories: Vec<PathBuf>,
        default_model: Option<String>,
    },
    LoadSession {
        session_id: String,
        cwd: PathBuf,
        additional_directories: Vec<PathBuf>,
    },
    SetModel {
        session_id: String,
        config_id: Option<String>,
        model_id: String,
    },
    SetMode {
        session_id: String,
        config_id: Option<String>,
        mode_id: String,
    },
    SetConfigOption {
        session_id: String,
        config_id: String,
        value: String,
    },
    RespondPermission {
        request_id: String,
        option_id: Option<String>,
    },
    Shutdown,
}

type PendingPermissions = Arc<Mutex<HashMap<String, Sender<Option<String>>>>>;

/// A handle to one ACP agent process and its session.
pub struct AcpHarness {
    commands: Sender<HarnessCommand>,
    stop: Sender<()>,
    events: Receiver<HarnessEvent>,
    join: Option<JoinHandle<()>>,
    next_request: Arc<AtomicU64>,
}

impl AcpHarness {
    /// Start the agent process on a dedicated worker thread.
    pub fn spawn(config: HarnessConfig) -> Self {
        let (command_tx, command_rx) = async_channel::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();
        let (stop_tx, stop_rx) = async_channel::bounded(1);
        let next_request = Arc::new(AtomicU64::new(1));

        let join = {
            let next_request = next_request.clone();
            let remote = config.is_remote();
            thread::Builder::new()
                .name("dunlin-acp".into())
                .spawn(move || {
                    if remote {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("failed to build the remote ACP runtime");
                        runtime.block_on(worker(
                            config,
                            command_rx,
                            event_tx,
                            next_request,
                            stop_rx,
                        ));
                    } else {
                        async_io::block_on(worker(
                            config,
                            command_rx,
                            event_tx,
                            next_request,
                            stop_rx,
                        ));
                    }
                })
                .expect("failed to spawn ACP worker thread")
        };

        Self {
            commands: command_tx,
            stop: stop_tx,
            events: event_rx,
            join: Some(join),
            next_request,
        }
    }

    pub fn events(&self) -> &Receiver<HarnessEvent> {
        &self.events
    }

    /// Queue a prompt for the active session. Returns false if the worker is gone.
    pub fn prompt(&self, text: impl Into<String>) -> bool {
        self.send(HarnessCommand::Prompt {
            text: text.into(),
            continuation: None,
        })
    }
    /// Relay verifies this approved continuation id atomically before forwarding.
    pub fn prompt_continuation(&self, text: impl Into<String>, id: String) -> bool {
        self.send(HarnessCommand::Prompt {
            text: text.into(),
            continuation: Some(id),
        })
    }

    /// Ask the agent to cancel the current turn.
    pub fn cancel(&self) -> bool {
        self.send(HarnessCommand::Cancel)
    }

    /// Create a fresh ACP session in `cwd` and make it active.
    pub fn new_session(&self, cwd: impl Into<PathBuf>) -> bool {
        self.new_session_with_model(cwd, None)
    }

    /// Snapshot the latest project default without restarting the process.
    /// Passing None uses the harness's own default, not the spawn-time setting.
    pub fn new_session_with_model(
        &self,
        cwd: impl Into<PathBuf>,
        default_model: Option<String>,
    ) -> bool {
        self.new_session_with_roots(cwd, Vec::new(), default_model)
    }

    /// Create a session with an explicit ACP multi-root workspace. The caller
    /// has already established trust for every additional directory.
    pub fn new_session_with_roots(
        &self,
        cwd: impl Into<PathBuf>,
        additional_directories: Vec<PathBuf>,
        default_model: Option<String>,
    ) -> bool {
        self.send(HarnessCommand::NewSession {
            cwd: cwd.into(),
            additional_directories,
            default_model,
        })
    }

    /// Load an existing ACP session and replay its history.
    pub fn load_session(&self, session_id: impl Into<String>, cwd: impl Into<PathBuf>) -> bool {
        self.load_session_with_roots(session_id, cwd, Vec::new())
    }

    /// Load a session with the complete additional-root set it should see.
    pub fn load_session_with_roots(
        &self,
        session_id: impl Into<String>,
        cwd: impl Into<PathBuf>,
        additional_directories: Vec<PathBuf>,
    ) -> bool {
        self.send(HarnessCommand::LoadSession {
            session_id: session_id.into(),
            cwd: cwd.into(),
            additional_directories,
        })
    }

    pub fn set_model(
        &self,
        session_id: String,
        config_id: Option<String>,
        model_id: String,
    ) -> bool {
        self.send(HarnessCommand::SetModel {
            session_id,
            config_id,
            model_id,
        })
    }

    pub fn set_mode(&self, session_id: String, config_id: Option<String>, mode_id: String) -> bool {
        self.send(HarnessCommand::SetMode {
            session_id,
            config_id,
            mode_id,
        })
    }

    pub fn set_config_option(&self, session_id: String, config_id: String, value: String) -> bool {
        self.send(HarnessCommand::SetConfigOption {
            session_id,
            config_id,
            value,
        })
    }

    /// Resolve a pending permission request.
    pub fn respond_permission(
        &self,
        request_id: impl Into<String>,
        option_id: Option<String>,
    ) -> bool {
        self.send(HarnessCommand::RespondPermission {
            request_id: request_id.into(),
            option_id,
        })
    }

    pub fn shutdown(&self) {
        // Also interrupt initialize/session creation if the agent never replies.
        let _ = self.stop.try_send(());
        let _ = self.commands.try_send(HarnessCommand::Shutdown);
    }

    fn send(&self, command: HarnessCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    /// Request ids are allocated by the UI thread so permission events can be
    /// correlated without a second round trip.
    pub fn allocate_request_id(&self) -> String {
        format!("perm-{}", self.next_request.fetch_add(1, Ordering::Relaxed))
    }
}

impl Drop for AcpHarness {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(join) = self.join.take() {
            // Never block the UI while an agent shuts down. The worker exits as
            // soon as the connection closes; joining off-thread keeps the
            // handle from leaking without stalling a frame.
            thread::spawn(move || {
                let _ = join.join();
            });
        }
    }
}

async fn worker(
    config: HarnessConfig,
    commands: Receiver<HarnessCommand>,
    events: Sender<HarnessEvent>,
    next_request: Arc<AtomicU64>,
    stop: Receiver<()>,
) {
    let diagnostics = config.diagnostics.clone();
    let run = std::pin::pin!(run(config, commands, events.clone(), next_request));
    let stop = std::pin::pin!(stop.recv());
    let result = match futures_util::future::select(run, stop).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => return,
    };
    if let Err(error) = result {
        if let Some(sink) = diagnostics {
            sink("Error", &error.to_string());
        }
        let _ = events
            .send(HarnessEvent::Error {
                message: error.to_string(),
            })
            .await;
        let _ = events
            .send(HarnessEvent::Exited {
                reason: error.to_string(),
            })
            .await;
    }
}

async fn run(
    config: HarnessConfig,
    commands: Receiver<HarnessCommand>,
    events: Sender<HarnessEvent>,
    next_request: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let transport = match &config.transport {
        HarnessTransport::Local { command } => {
            let agent = AcpAgent::from_str(command).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("could not parse harness command: {error}"),
                )
            })?;
            let mut agent = AcpAgent::new(agent.into_config().envs(config.process_env.clone()));
            if let Some(sink) = config.diagnostics.clone() {
                agent = agent.with_debug(move |line, direction| {
                    sink(&format!("{direction:?}"), line);
                });
            }
            Transport::Local(agent)
        }
        HarnessTransport::Remote {
            url,
            token,
            project_id,
            roots,
        } => {
            let mut transport = WsTransport::new(url, token, project_id, roots.clone())?;
            transport.tls_root_pem = config.tls_root_pem.clone();
            Transport::Remote(transport)
        }
    };
    let pending: PendingPermissions = Arc::new(Mutex::new(HashMap::new()));

    let notification_events = events.clone();
    let permission_events = events.clone();
    let foreground_events = events.clone();
    let permission_pending = pending.clone();
    let permission_next = next_request.clone();

    Client
        .v1()
        .name(config.client_name.clone())
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                if let Some(update) = map_notification(&notification) {
                    let _ = notification_events
                        .send(HarnessEvent::Update {
                            session_id: notification.session_id.to_string(),
                            update,
                        })
                        .await;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, cx| {
                let request_id =
                    format!("perm-{}", permission_next.fetch_add(1, Ordering::Relaxed));
                let options = request
                    .options
                    .iter()
                    .map(|option| PermissionChoice {
                        id: option.option_id.to_string(),
                        label: option.name.clone(),
                        kind: map_permission_kind(option.kind),
                    })
                    .collect::<Vec<_>>();

                let title = request
                    .tool_call
                    .fields
                    .title
                    .clone()
                    .or_else(|| request.tool_call.fields.name.clone())
                    .unwrap_or_else(|| "Tool call".into());

                let (tx, rx) = async_channel::bounded(1);
                if let Ok(mut pending) = permission_pending.lock() {
                    pending.insert(request_id.clone(), tx);
                }

                let _ = permission_events
                    .send(HarnessEvent::PermissionRequested {
                        request_id: request_id.clone(),
                        session_id: request.session_id.to_string(),
                        title,
                        options,
                    })
                    .await;

                // Hand the responder to a connection task. The dispatch loop
                // must not wait for the user to click a button.
                cx.spawn(async move {
                    let option_id = rx.recv().await.ok().flatten();
                    let response = match option_id {
                        Some(option_id) => {
                            RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(option_id),
                            ))
                        }
                        None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                    };
                    let _ = responder.respond(response);
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            transport,
            move |connection: ConnectionTo<Agent>| async move {
                let init = connection_timeout(connection
                    .send_request(
                        InitializeRequest::new(ProtocolVersion::V1).client_info(
                            agent_client_protocol::v1::schema::Implementation::new(
                                "Dunlin",
                                env!("CARGO_PKG_VERSION"),
                            )
                            .title("Dunlin"),
                        ),
                    )
                    .block_task())
                    .await?;

                let (agent_name, agent_version) = init
                    .agent_info
                    .as_ref()
                    .map(|info| {
                        (
                            info.title.clone().unwrap_or_else(|| info.name.clone()),
                            info.version.clone(),
                        )
                    })
                    .unwrap_or_else(|| ("ACP agent".into(), String::new()));

                let _ = foreground_events
                    .send(HarnessEvent::Ready {
                        agent: agent_name,
                        version: agent_version,
                    })
                    .await;

                let mut active_session: Option<agent_client_protocol::v1::schema::SessionId> = None;
                if !config.is_remote() {
                    if let Some(session_id) = config.resume_session_id.as_ref() {
                        if init.agent_capabilities.load_session {
                            match open_session(&connection, &foreground_events, config.cwd.clone(), config.additional_directories.clone(), mcp_server_wire(&config.mcp_servers), Some(session_id.clone()), None).await {
                                Ok(id) => active_session = Some(id.into()),
                                Err(message) => {
                                    let _ = foreground_events.send(HarnessEvent::SessionLoadFailed { session_id: session_id.clone(), message: message.to_string() }).await;
                                }
                            }
                        } else {
                            let _ = foreground_events.send(HarnessEvent::SessionLoadFailed {
                                session_id: session_id.clone(),
                                message: "This harness cannot resume sessions. The saved conversation is preserved; start a new session to continue.".into(),
                            }).await;
                        }
                    } else {
                        active_session = Some(open_session(&connection, &foreground_events, config.cwd.clone(), config.additional_directories.clone(), mcp_server_wire(&config.mcp_servers), None, config.default_model.as_deref()).await?.into());
                    }
                }

                let mut shutting_down = false;
                while let Ok(command) = commands.recv().await {
                    match command {
                        HarnessCommand::Prompt { text, continuation } => {
                            let Some(session_id) = active_session.clone() else {
                                let _ = foreground_events
                                    .send(HarnessEvent::Error {
                                        message: "No active ACP session".into(),
                                    })
                                    .await;
                                continue;
                            };
                            let prompt_connection = connection.clone();
                            let prompt_events = foreground_events.clone();
                            connection.spawn(async move {
                                let mut request = PromptRequest::new(
                                        session_id.clone(),
                                        vec![ContentBlock::Text(
                                            agent_client_protocol::v1::schema::TextContent::new(
                                                text,
                                            ),
                                        )],
                                    );
                                if let Some(id) = continuation {
                                    request.meta = Some(serde_json::Map::from_iter([("dunlin/continuation".into(), serde_json::Value::String(id))]));
                                }
                                let response = prompt_connection
                                    .send_request(request)
                                    .block_task()
                                    .await;
                                match response {
                                    Ok(response) => {
                                        let _ = prompt_events
                                            .send(HarnessEvent::PromptFinished {
                                                session_id: session_id.to_string(),
                                                stop_reason: stop_reason_label(
                                                    response.stop_reason,
                                                ),
                                                usage: response.usage.as_ref().map(|usage| TokenUsage {
                                                    total: usage.total_tokens,
                                                    input: usage.input_tokens,
                                                    output: usage.output_tokens,
                                                    reasoning: usage.thought_tokens,
                                                }),
                                            })
                                            .await;
                                    }
                                    Err(error) => {
                                        let _ = prompt_events
                                            .send(HarnessEvent::Error {
                                                message: format!("Prompt failed: {error}"),
                                            })
                                            .await;
                                    }
                                }
                                Ok(())
                            })?;
                        }
                        HarnessCommand::Cancel => {
                            if let Some(session_id) = active_session.as_ref() {
                                let _ = connection
                                    .send_notification(CancelNotification::new(session_id.clone()));
                            }
                        }
                        HarnessCommand::NewSession { cwd, additional_directories, default_model } => {
                            active_session = None;
                            match open_session(&connection, &foreground_events, cwd, additional_directories, mcp_server_wire(&config.mcp_servers), None, default_model.as_deref()).await {
                                Ok(id) => {
                                    active_session = Some(id.into());
                                }
                                Err(error) => {
                                    let _ = foreground_events
                                        .send(HarnessEvent::Error {
                                            message: format!("Could not create session: {error}"),
                                        })
                                        .await;
                                }
                            }
                        }
                        HarnessCommand::LoadSession { session_id, cwd, additional_directories } => {
                            active_session = None;
                            match open_session(&connection, &foreground_events, cwd, additional_directories, mcp_server_wire(&config.mcp_servers), Some(session_id.clone()), None).await {
                                Ok(id) => {
                                    active_session = Some(id.into());
                                }
                                Err(error) => {
                                    let _ = foreground_events
                                        .send(HarnessEvent::SessionLoadFailed {
                                            session_id,
                                            message: format!("Could not load session: {error}. The saved conversation is preserved."),
                                        })
                                        .await;
                                }
                            }
                        }
                        HarnessCommand::SetModel { session_id, config_id, model_id } => {
                            let result = async {
                                if active_session.as_ref().map(ToString::to_string).as_deref() != Some(&session_id) {
                                    return Err(AcpError::new(-32602, "Session is no longer active"));
                                }
                                let request = model_request(&session_id, config_id.as_deref(), &model_id)?;
                                connection.send_request(request).block_task().await
                            }.await;
                            match result {
                                Ok(response) => {
                                    // Older agents return an empty object on success.
                                    let _ = foreground_events.send(HarnessEvent::ConfigOptions {
                                        session_id: session_id.clone(), options: session_options(&response).unwrap_or_default(),
                                    }).await;
                                    let state = session_models(&response).unwrap_or(SessionModels {
                                        config_id, current: model_id, choices: Vec::new(),
                                    });
                                    if let Some(modes) = session_modes(&response) {
                                        let _ = foreground_events.send(HarnessEvent::Modes { session_id: session_id.clone(), state: Some(modes) }).await;
                                    }
                                    let _ = foreground_events.send(HarnessEvent::Models { session_id, state: Some(state) }).await;
                                }
                                Err(error) => {
                                    let _ = foreground_events.send(HarnessEvent::ModelChangeFailed {
                                        session_id, message: format!("Could not change model: {error}"),
                                    }).await;
                                }
                            }
                        }
                        HarnessCommand::SetMode { session_id, config_id, mode_id } => {
                            let result = async {
                                if active_session.as_ref().map(ToString::to_string).as_deref() != Some(&session_id) {
                                    return Err(AcpError::new(-32602, "Session is no longer active"));
                                }
                                let request = if let Some(config_id) = config_id.as_ref() {
                                    UntypedMessage::new("session/set_config_option", serde_json::json!({
                                        "sessionId": session_id, "configId": config_id, "value": mode_id,
                                    }))?
                                } else {
                                    UntypedMessage::new("session/set_mode", serde_json::json!({
                                        "sessionId": session_id, "modeId": mode_id,
                                    }))?
                                };
                                connection.send_request(request).block_task().await
                            }.await;
                            match result {
                                Ok(response) => {
                                    if let Some(options) = session_options(&response) {
                                        let _ = foreground_events.send(HarnessEvent::ConfigOptions { session_id: session_id.clone(), options }).await;
                                    }
                                    if let Some(models) = session_models(&response) {
                                        let _ = foreground_events.send(HarnessEvent::Models { session_id: session_id.clone(), state: Some(models) }).await;
                                    }
                                    let state = session_modes(&response).unwrap_or(SessionModes {
                                        config_id, current: mode_id, choices: Vec::new(),
                                    });
                                    let _ = foreground_events.send(HarnessEvent::Modes { session_id, state: Some(state) }).await;
                                }
                                Err(error) => {
                                    let _ = foreground_events.send(HarnessEvent::ModeChangeFailed {
                                        session_id, message: format!("Could not change mode: {error}"),
                                    }).await;
                                }
                            }
                        }
                        HarnessCommand::SetConfigOption { session_id, config_id, value } => {
                            let result = async {
                                if active_session.as_ref().map(ToString::to_string).as_deref() != Some(&session_id) {
                                    return Err(AcpError::new(-32602, "Session is no longer active"));
                                }
                                let request = UntypedMessage::new("session/set_config_option", serde_json::json!({
                                    "sessionId": session_id, "configId": config_id, "value": value,
                                }))?;
                                let response = connection.send_request(request).block_task().await?;
                                let options = session_options(&response).ok_or_else(|| AcpError::new(-32603, "Harness did not confirm configuration options"))?;
                                if !options.iter().any(|option| option.id == config_id && option.selector.current == value) {
                                    return Err(AcpError::new(-32603, "Harness did not accept the selected option"));
                                }
                                Ok((response, options))
                            }.await;
                            match result {
                                Ok((response, options)) => {
                                    if let Some(state) = session_models(&response) {
                                        let _ = foreground_events.send(HarnessEvent::Models { session_id: session_id.clone(), state: Some(state) }).await;
                                    }
                                    if let Some(state) = session_modes(&response) {
                                        let _ = foreground_events.send(HarnessEvent::Modes { session_id: session_id.clone(), state: Some(state) }).await;
                                    }
                                    let _ = foreground_events.send(HarnessEvent::ConfigOptions { session_id, options }).await;
                                }
                                Err(error) => {
                                    let _ = foreground_events.send(HarnessEvent::ConfigOptionChangeFailed { session_id, message: format!("Could not change model option: {error}") }).await;
                                }
                            }
                        }
                        HarnessCommand::RespondPermission {
                            request_id,
                            option_id,
                        } => {
                            let sender = pending
                                .lock()
                                .ok()
                                .and_then(|mut pending| pending.remove(&request_id));
                            if let Some(sender) = sender {
                                let _ = sender.send(option_id).await;
                            }
                        }
                        HarnessCommand::Shutdown => {
                            shutting_down = true;
                            break;
                        }
                    }
                }

                if !shutting_down {
                    let _ = foreground_events
                        .send(HarnessEvent::Exited {
                            reason: "command channel closed".into(),
                        })
                        .await;
                }
                Ok(())
            },
        )
        .await
        .map_err(|error| format!("ACP connection failed: {error}"))?;

    let _ = events
        .send(HarnessEvent::Exited {
            reason: "connection closed".into(),
        })
        .await;
    Ok(())
}

async fn connection_timeout<T>(
    future: impl std::future::Future<Output = AcpResult<T>>,
) -> AcpResult<T> {
    let future = std::pin::pin!(future);
    let timeout = std::pin::pin!(async_io::Timer::after(std::time::Duration::from_secs(30)));
    match futures_util::future::select(future, timeout).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => Err(AcpError::new(
            -32000,
            "Harness connection timed out after 30 seconds. Check its configuration and retry.",
        )),
    }
}

fn mcp_server_wire(configs: &[McpServerConfig]) -> Vec<McpServer> {
    configs
        .iter()
        .map(|server| {
            McpServer::Stdio(
                McpServerStdio::new(server.name.clone(), server.command.clone())
                    .args(server.args.clone())
                    .env(
                        server
                            .env
                            .iter()
                            .map(|(name, value)| EnvVariable::new(name.clone(), value.clone()))
                            .collect(),
                    ),
            )
        })
        .collect()
}

async fn open_session(
    connection: &ConnectionTo<Agent>,
    events: &Sender<HarnessEvent>,
    cwd: PathBuf,
    additional_directories: Vec<PathBuf>,
    mcp_servers: Vec<McpServer>,
    existing: Option<String>,
    default_model: Option<&str>,
) -> AcpResult<String> {
    // Keep the raw response: some deployed harnesses still expose `models`,
    // which newer ACP schemas intentionally no longer deserialize.
    let request = if let Some(id) = existing.as_ref() {
        UntypedMessage::new(
            "session/load",
            LoadSessionRequest::new(id.clone(), cwd)
                .additional_directories(additional_directories)
                .mcp_servers(mcp_servers),
        )?
    } else {
        UntypedMessage::new(
            "session/new",
            NewSessionRequest::new(cwd)
                .additional_directories(additional_directories)
                .mcp_servers(mcp_servers),
        )?
    };
    let response = connection_timeout(connection.send_request(request).block_task()).await?;
    let id = existing
        .clone()
        .or_else(|| {
            response
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| AcpError::new(-32603, "The harness did not return a session id"))?;
    let mut models = session_models(&response);
    let mut modes = session_modes(&response);
    let mut options = session_options(&response).unwrap_or_default();
    if existing.is_none()
        && let Some(preferred) = default_model
    {
        let selector = models.as_ref().ok_or_else(|| AcpError::new(-32602,
            "The harness does not advertise models. Clear the project's default model to continue."))?;
        if !selector.choices.iter().any(|choice| choice.id == preferred) {
            return Err(AcpError::new(
                -32602,
                "The project's default model is unavailable. Choose another model in Project settings.",
            ));
        }
        if selector.current != preferred {
            let changed = connection_timeout(
                connection
                    .send_request(model_request(
                        &id,
                        selector.config_id.as_deref(),
                        preferred,
                    )?)
                    .block_task(),
            )
            .await?;
            let mut selected = session_models(&changed).unwrap_or_else(|| {
                let mut selected = selector.clone();
                selected.current = preferred.into();
                selected
            });
            if selected.current != preferred {
                return Err(AcpError::new(
                    -32602,
                    "The harness did not accept the project's default model",
                ));
            }
            if selected.choices.is_empty() {
                selected.choices = selector.choices.clone();
            }
            models = Some(selected);
            modes = session_modes(&changed).or(modes);
            options = session_options(&changed).unwrap_or_default();
        }
    }
    let event = if existing.is_some() {
        HarnessEvent::SessionLoaded {
            session_id: id.clone(),
        }
    } else {
        HarnessEvent::SessionCreated {
            session_id: id.clone(),
        }
    };
    let _ = events.send(event).await;
    let _ = events
        .send(HarnessEvent::Models {
            session_id: id.clone(),
            state: models,
        })
        .await;
    let _ = events
        .send(HarnessEvent::ConfigOptions {
            session_id: id.clone(),
            options,
        })
        .await;
    let _ = events
        .send(HarnessEvent::Modes {
            session_id: id.clone(),
            state: modes,
        })
        .await;
    Ok(id)
}

fn model_request(
    session_id: &str,
    config_id: Option<&str>,
    model_id: &str,
) -> AcpResult<UntypedMessage> {
    if let Some(config_id) = config_id {
        UntypedMessage::new(
            "session/set_config_option",
            serde_json::json!({
                "sessionId": session_id, "configId": config_id, "value": model_id,
            }),
        )
    } else {
        UntypedMessage::new(
            "session/set_model",
            serde_json::json!({
                "sessionId": session_id, "modelId": model_id,
            }),
        )
    }
}

fn map_notification(notification: &SessionNotification) -> Option<HarnessUpdate> {
    let update = map_session_update(&notification.update)?;
    let wire = serde_json::to_value(&notification.update).ok()?;
    if let Some(child) = wire
        .get("_meta")
        .and_then(|meta| meta.get("opencode/child-session"))
        && let (Some(id), Some(parent_id)) = (child["id"].as_str(), child["parentID"].as_str())
    {
        return Some(HarnessUpdate::Child {
            id: id.into(),
            parent_id: parent_id.into(),
            title: child["title"].as_str().map(str::to_owned),
            update: Box::new(update),
        });
    }
    Some(update)
}

fn map_session_update(update: &SessionUpdate) -> Option<HarnessUpdate> {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => {
            content_text(&chunk.content).map(HarnessUpdate::UserMessage)
        }
        SessionUpdate::ConfigOptionUpdate(_) => {
            serde_json::to_value(update)
                .ok()
                .map(|value| HarnessUpdate::Configuration {
                    models: session_models(&value),
                    modes: session_modes(&value),
                    options: session_options(&value),
                })
        }
        SessionUpdate::CurrentModeUpdate(update) => Some(HarnessUpdate::Configuration {
            models: None,
            options: None,
            modes: Some(SessionModes {
                config_id: None,
                current: update.current_mode_id.to_string(),
                choices: Vec::new(),
            }),
        }),
        SessionUpdate::AgentMessageChunk(chunk) => {
            content_text(&chunk.content).map(|text| match &chunk.message_id {
                Some(id) => HarnessUpdate::MessageChunk {
                    id: id.to_string(),
                    text,
                    thought: false,
                },
                None => HarnessUpdate::AgentMessage(text),
            })
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            content_text(&chunk.content).map(|text| match &chunk.message_id {
                Some(id) => HarnessUpdate::MessageChunk {
                    id: id.to_string(),
                    text,
                    thought: true,
                },
                None => HarnessUpdate::AgentThought(text),
            })
        }
        SessionUpdate::ToolCall(tool) => Some(HarnessUpdate::ToolCall(tool_view(tool))),
        SessionUpdate::ToolCallUpdate(update) => {
            Some(HarnessUpdate::ToolCallUpdate(tool_delta(update)))
        }
        SessionUpdate::Plan(plan) => Some(HarnessUpdate::Plan(
            plan.entries
                .iter()
                .map(|entry| PlanItem {
                    content: entry.content.clone(),
                    status: map_plan_status(entry.status.clone()),
                })
                .collect(),
        )),
        SessionUpdate::UsageUpdate(usage) => Some(usage_update(usage)),
        SessionUpdate::AvailableCommandsUpdate(commands) => Some(available_commands(commands)),
        _ => None,
    }
}

fn content_text(content: &ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(text) => Some(text.text.clone()),
        ContentBlock::ResourceLink(link) => Some(link.name.clone()),
        _ => None,
    }
}

fn tool_view(tool: &ToolCall) -> ToolCallView {
    let (detail, additions, deletions, diff) = tool_content(&tool.content);
    ToolCallView {
        id: tool.tool_call_id.to_string(),
        title: tool.title.clone(),
        kind: map_tool_kind(tool.kind),
        status: map_tool_status(tool.status),
        detail,
        locations: tool
            .locations
            .iter()
            .map(|location| location.path.display().to_string())
            .collect(),
        additions,
        deletions,
        diff,
    }
}

fn tool_delta(update: &ToolCallUpdate) -> ToolCallDelta {
    let (detail, additions, deletions, diff) = update
        .fields
        .content
        .as_ref()
        .map(|content| tool_content(content))
        .unwrap_or_default();
    ToolCallDelta {
        id: update.tool_call_id.to_string(),
        title: update.fields.title.clone(),
        kind: update.fields.kind.map(map_tool_kind),
        status: update.fields.status.map(map_tool_status),
        detail: (!detail.is_empty()).then_some(detail),
        additions: (additions > 0).then_some(additions),
        deletions: (deletions > 0).then_some(deletions),
        diff,
    }
}

fn tool_content(content: &[ToolCallContent]) -> (String, u32, u32, Option<String>) {
    let mut detail = String::new();
    let mut additions = 0;
    let mut deletions = 0;
    let mut patch = String::new();

    for item in content {
        match item {
            ToolCallContent::Content(content) => {
                if let ContentBlock::Text(text) = &content.content {
                    if !detail.is_empty() {
                        detail.push('\n');
                    }
                    detail.push_str(&text.text);
                }
            }
            ToolCallContent::Diff(diff) => {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                let path = diff.path.display().to_string();
                detail.push_str(&path);
                additions += diff.new_text.lines().count() as u32;
                deletions += diff
                    .old_text
                    .as_deref()
                    .map(|text| text.lines().count() as u32)
                    .unwrap_or(0);
                let old = diff.old_text.as_deref().unwrap_or("");
                let unified = similar::TextDiff::from_lines(old, &diff.new_text)
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string();
                if !patch.is_empty() {
                    patch.push('\n');
                }
                patch.push_str(&unified);
            }
            _ => {}
        }
    }

    (
        detail,
        additions,
        deletions,
        (!patch.is_empty()).then_some(patch),
    )
}

fn map_tool_kind(kind: agent_client_protocol::v1::schema::ToolKind) -> ToolKind {
    use agent_client_protocol::v1::schema::ToolKind as Acp;
    match kind {
        Acp::Read => ToolKind::Read,
        Acp::Edit | Acp::Delete | Acp::Move => ToolKind::Edit,
        Acp::Search => ToolKind::Search,
        Acp::Execute => ToolKind::Shell,
        Acp::Fetch => ToolKind::Web,
        Acp::Think | Acp::SwitchMode => ToolKind::Plan,
        _ => ToolKind::Task,
    }
}

fn map_tool_status(status: ToolCallStatus) -> ToolStatus {
    match status {
        ToolCallStatus::Pending => ToolStatus::Waiting,
        ToolCallStatus::InProgress => ToolStatus::Running,
        ToolCallStatus::Completed => ToolStatus::Done,
        ToolCallStatus::Failed => ToolStatus::Failed,
        _ => ToolStatus::Waiting,
    }
}

fn map_plan_status(status: agent_client_protocol::v1::schema::PlanEntryStatus) -> PlanStatus {
    use agent_client_protocol::v1::schema::PlanEntryStatus as Acp;
    match status {
        Acp::Pending => PlanStatus::Pending,
        Acp::InProgress => PlanStatus::InProgress,
        Acp::Completed => PlanStatus::Completed,
        _ => PlanStatus::Pending,
    }
}

fn map_permission_kind(kind: PermissionOptionKind) -> PermissionKind {
    match kind {
        PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
        PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
        PermissionOptionKind::RejectOnce => PermissionKind::RejectOnce,
        PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
        _ => PermissionKind::Unknown,
    }
}

fn usage_update(usage: &UsageUpdate) -> HarnessUpdate {
    HarnessUpdate::Usage {
        used: usage.used,
        size: usage.size,
        cost_usd: usage
            .cost
            .as_ref()
            .filter(|cost| {
                cost.currency.eq_ignore_ascii_case("USD")
                    && cost.amount.is_finite()
                    && cost.amount >= 0.
            })
            .map(|cost| cost.amount),
    }
}

fn available_commands(update: &AvailableCommandsUpdate) -> HarnessUpdate {
    HarnessUpdate::AvailableCommands(
        update
            .available_commands
            .iter()
            .map(|command| command.name.clone())
            .collect(),
    )
}

fn stop_reason_label(reason: agent_client_protocol::v1::schema::StopReason) -> String {
    use agent_client_protocol::v1::schema::StopReason as Acp;
    match reason {
        Acp::EndTurn => "end_turn",
        Acp::MaxTokens => "max_tokens",
        Acp::MaxTurnRequests => "max_turn_requests",
        Acp::Refusal => "refusal",
        Acp::Cancelled => "cancelled",
        _ => "unknown",
    }
    .into()
}

/// The two ACP transports share one client setup; this enum lets the builder
/// stay generic while the local path spawns a process and the remote path
/// connects a WebSocket.
enum Transport {
    Local(AcpAgent),
    Remote(WsTransport),
}

impl ConnectTo<Client> for Transport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> AcpResult<()> {
        match self {
            Transport::Local(agent) => ConnectTo::<Client>::connect_to(agent, client).await,
            Transport::Remote(remote) => ConnectTo::<Client>::connect_to(remote, client).await,
        }
    }
}

/// ACP over a WebSocket text-frame transport.
struct WsTransport {
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    tls_root_pem: Option<Vec<u8>>,
}

impl WsTransport {
    fn new(url: &str, token: &str, project_id: &str, roots: Vec<String>) -> io::Result<Self> {
        let parsed = url::Url::parse(url.trim()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid relay URL: {error}"),
            )
        })?;
        let loopback = match parsed.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if parsed.host().is_none()
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Relay URL must not contain credentials, a query or a fragment",
            ));
        }
        match parsed.scheme() {
            "https" | "wss" => {}
            "http" | "ws" if loopback => {}
            "http" | "ws" => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Plain WebSocket transport is only allowed on loopback; use HTTPS/WSS",
                ));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Relay URL must use http(s) or ws(s)",
                ));
            }
        }
        let base = url.trim_end_matches('/');
        let ws = match parsed.scheme() {
            "https" => format!("wss://{}", base.strip_prefix("https://").unwrap_or(base)),
            "http" => format!("ws://{}", base.strip_prefix("http://").unwrap_or(base)),
            _ => base.to_string(),
        };
        let mut roots = roots;
        roots.sort();
        roots.dedup();
        if roots.len() > 32
            || roots.iter().any(|root| {
                root.is_empty()
                    || root.len() > 128
                    || !root
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Relay workspace roots must be explicit project ids (max 32)",
            ));
        }
        let root_query = if roots.is_empty() {
            String::new()
        } else {
            format!("&roots={}", roots.join(","))
        };
        let endpoint = if ws.ends_with("/acp") {
            format!("{ws}?project={project_id}{root_query}")
        } else {
            format!("{ws}/acp?project={project_id}{root_query}")
        };
        let mut request = endpoint
            .into_client_request()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        Ok(Self {
            request,
            tls_root_pem: None,
        })
    }
}

impl ConnectTo<Client> for WsTransport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> AcpResult<()> {
        let connector = if let Some(pem) = &self.tls_root_pem {
            Some(tokio_tungstenite::Connector::Rustls(
                relay_tls_config(pem).map_err(|error| AcpError::new(-32603, error))?,
            ))
        } else {
            None
        };
        let (socket, _) =
            tokio_tungstenite::connect_async_tls_with_config(self.request, None, false, connector)
                .await
                .map_err(|error| {
                    AcpError::new(-32603, format!("Could not connect to the relay: {error}"))
                })?;
        let (write, read) = socket.split();
        let sink = write
            .with(|line: String| async move {
                Ok::<Message, tokio_tungstenite::tungstenite::Error>(Message::Text(line.into()))
            })
            .sink_map_err(io::Error::other);
        let stream = read.filter_map(|message| async move {
            match message {
                Ok(Message::Text(text)) => Some(Ok(text.to_string())),
                Ok(Message::Binary(_))
                | Ok(Message::Ping(_))
                | Ok(Message::Pong(_))
                | Ok(Message::Close(_))
                | Ok(Message::Frame(_)) => None,
                Err(error) => Some(Err(io::Error::other(error))),
            }
        });
        ConnectTo::<Client>::connect_to(Lines::new(sink, stream), client).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_transport_requires_tls_off_loopback() {
        assert!(WsTransport::new("https://relay.example.test", "token", "prj", vec![]).is_ok());
        assert!(WsTransport::new("http://127.0.0.1:8787", "token", "prj", vec![]).is_ok());
        assert!(WsTransport::new("http://192.168.1.10:8787", "token", "prj", vec![]).is_err());
        assert!(
            WsTransport::new(
                "https://user:secret@relay.example.test",
                "token",
                "prj",
                vec![]
            )
            .is_err()
        );
        assert!(
            WsTransport::new(
                "https://relay.example.test",
                "token",
                "prj",
                vec!["prj-a".into(), "prj-b".into()]
            )
            .is_ok()
        );
        assert!(
            WsTransport::new(
                "https://relay.example.test",
                "token",
                "prj",
                vec!["../escape".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn preserves_message_ids_and_child_origin_in_projected_acp_updates() {
        let update: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate":"agent_message_chunk","messageId":"message-1",
            "content":{"type":"text","text":"# Child answer"},
            "_meta":{"opencode/child-session":{"id":"child-1","parentID":"root-1","depth":1,"title":"Delegated city"}}
        })).unwrap();
        let mapped = map_notification(&notification(update)).unwrap();
        assert!(
            matches!(mapped,HarnessUpdate::Child {id,parent_id,update,..}
            if id=="child-1" && parent_id=="root-1" && matches!(update.as_ref(),HarnessUpdate::MessageChunk {id,thought:false,..} if id=="message-1"))
        );
        let update: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate":"agent_message_chunk","messageId":"root-message",
            "content":{"type":"text","text":"# Main answer"}
        }))
        .unwrap();
        assert!(
            matches!(map_notification(&notification(update)),Some(HarnessUpdate::MessageChunk {id,thought:false,..}) if id=="root-message")
        );
    }

    fn notification(update: SessionUpdate) -> SessionNotification {
        SessionNotification::new("session-1", update)
    }

    #[test]
    fn maps_model_and_mode_configuration_updates_together() {
        let update: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {"id": "model", "name": "Model", "type": "select", "currentValue": "a", "options": [{"value": "a", "name": "Model A"}]},
                {"id": "agent-mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "plan", "options": [{"value": "plan", "name": "Plan"}]}
            ]
        })).unwrap();
        match map_notification(&notification(update)).unwrap() {
            HarnessUpdate::Configuration {
                models: Some(models),
                modes: Some(modes),
                ..
            } => {
                assert_eq!(models.current, "a");
                assert_eq!(modes.current, "plan");
            }
            other => panic!("unexpected update: {other:?}"),
        }
        let update = SessionUpdate::CurrentModeUpdate(
            agent_client_protocol::v1::schema::CurrentModeUpdate::new("build"),
        );
        match map_notification(&notification(update)).unwrap() {
            HarnessUpdate::Configuration {
                models: None,
                modes: Some(modes),
                ..
            } => assert_eq!(modes.current, "build"),
            other => panic!("unexpected mode notification: {other:?}"),
        }
    }

    #[test]
    fn maps_message_and_thought_chunks() {
        let message = notification(SessionUpdate::AgentMessageChunk(
            agent_client_protocol::v1::schema::ContentChunk::new(ContentBlock::Text(
                agent_client_protocol::v1::schema::TextContent::new("hello"),
            )),
        ));
        assert_eq!(
            map_notification(&message),
            Some(HarnessUpdate::AgentMessage("hello".into()))
        );

        let thought = notification(SessionUpdate::AgentThoughtChunk(
            agent_client_protocol::v1::schema::ContentChunk::new(ContentBlock::Text(
                agent_client_protocol::v1::schema::TextContent::new("thinking"),
            )),
        ));
        assert_eq!(
            map_notification(&thought),
            Some(HarnessUpdate::AgentThought("thinking".into()))
        );
    }

    #[test]
    fn maps_usage_and_cost() {
        let usage = UsageUpdate::new(1_200, 200_000)
            .cost(agent_client_protocol::v1::schema::Cost::new(0.25, "USD"));
        assert_eq!(
            map_notification(&notification(SessionUpdate::UsageUpdate(usage))),
            Some(HarnessUpdate::Usage {
                used: 1_200,
                size: 200_000,
                cost_usd: Some(0.25),
            })
        );
    }
    #[test]
    fn foreign_missing_and_invalid_costs_never_become_usd() {
        for (amount, currency) in [
            (1., "EUR"),
            (1., "PLN"),
            (-1., "USD"),
            (f64::INFINITY, "USD"),
            (f64::NAN, "USD"),
        ] {
            let usage = UsageUpdate::new(10, 100).cost(
                agent_client_protocol::v1::schema::Cost::new(amount, currency),
            );
            assert!(matches!(
                usage_update(&usage),
                HarnessUpdate::Usage { cost_usd: None, .. }
            ));
        }
        assert!(matches!(
            usage_update(&UsageUpdate::new(10, 100)),
            HarnessUpdate::Usage { cost_usd: None, .. }
        ));
        let usage =
            UsageUpdate::new(10, 100).cost(agent_client_protocol::v1::schema::Cost::new(0., "usd"));
        assert!(matches!(
            usage_update(&usage),
            HarnessUpdate::Usage {
                cost_usd: Some(0.),
                ..
            }
        ));
    }

    #[test]
    fn older_relay_completion_events_keep_usage_unknown() {
        let event: HarnessEvent = serde_json::from_str(
            r#"{"PromptFinished":{"session_id":"session","stop_reason":"end_turn"}}"#,
        )
        .unwrap();
        assert!(matches!(
            event,
            HarnessEvent::PromptFinished { usage: None, .. }
        ));
    }

    #[test]
    fn maps_available_commands() {
        let commands = AvailableCommandsUpdate::new(vec![
            agent_client_protocol::v1::schema::AvailableCommand::new("init", "Initialize"),
            agent_client_protocol::v1::schema::AvailableCommand::new("review", "Review"),
        ]);
        assert_eq!(
            map_notification(&notification(SessionUpdate::AvailableCommandsUpdate(
                commands
            ))),
            Some(HarnessUpdate::AvailableCommands(vec![
                "init".into(),
                "review".into()
            ]))
        );
    }

    #[test]
    fn maps_tool_kinds_and_statuses() {
        use agent_client_protocol::v1::schema::{ToolCallStatus as AcpStatus, ToolKind as AcpKind};

        assert_eq!(map_tool_kind(AcpKind::Read), ToolKind::Read);
        assert_eq!(map_tool_kind(AcpKind::Edit), ToolKind::Edit);
        assert_eq!(map_tool_kind(AcpKind::Delete), ToolKind::Edit);
        assert_eq!(map_tool_kind(AcpKind::Search), ToolKind::Search);
        assert_eq!(map_tool_kind(AcpKind::Execute), ToolKind::Shell);
        assert_eq!(map_tool_kind(AcpKind::Fetch), ToolKind::Web);

        assert_eq!(map_tool_status(AcpStatus::Pending), ToolStatus::Waiting);
        assert_eq!(map_tool_status(AcpStatus::InProgress), ToolStatus::Running);
        assert_eq!(map_tool_status(AcpStatus::Completed), ToolStatus::Done);
        assert_eq!(map_tool_status(AcpStatus::Failed), ToolStatus::Failed);
    }

    #[test]
    fn maps_permission_kinds() {
        assert_eq!(
            map_permission_kind(PermissionOptionKind::AllowOnce),
            PermissionKind::AllowOnce
        );
        assert_eq!(
            map_permission_kind(PermissionOptionKind::AllowAlways),
            PermissionKind::AllowAlways
        );
        assert_eq!(
            map_permission_kind(PermissionOptionKind::RejectOnce),
            PermissionKind::RejectOnce
        );
        assert_eq!(
            map_permission_kind(PermissionOptionKind::RejectAlways),
            PermissionKind::RejectAlways
        );
    }

    #[test]
    fn labels_stop_reasons() {
        use agent_client_protocol::v1::schema::StopReason as Acp;
        assert_eq!(stop_reason_label(Acp::EndTurn), "end_turn");
        assert_eq!(stop_reason_label(Acp::Refusal), "refusal");
        assert_eq!(stop_reason_label(Acp::Cancelled), "cancelled");
        assert_eq!(stop_reason_label(Acp::MaxTokens), "max_tokens");
    }

    #[test]
    fn extracts_text_from_tool_content() {
        let content = vec![ToolCallContent::Content(
            agent_client_protocol::v1::schema::Content::new(ContentBlock::Text(
                agent_client_protocol::v1::schema::TextContent::new("first line\nsecond line"),
            )),
        )];
        let (detail, additions, deletions, diff) = tool_content(&content);
        assert_eq!(detail, "first line\nsecond line");
        assert_eq!((additions, deletions), (0, 0));
        assert!(diff.is_none());
    }

    #[test]
    fn mcp_servers_are_sent_as_stdio_without_leaking_into_the_harness_env() {
        let wire = mcp_server_wire(&[McpServerConfig::stdio(
            "dunlin-secrets",
            "/Applications/Dunlin.app/Contents/MacOS/Dunlin",
            vec!["--mcp-vault".into()],
            vec![
                ("DUNLIN_VAULT_SOCKET".into(), "/tmp/vault.sock".into()),
                ("DUNLIN_VAULT_TOKEN".into(), "token".into()),
            ],
        )]);
        let value = serde_json::to_value(&wire).unwrap();
        assert_eq!(value[0]["name"], "dunlin-secrets");
        assert_eq!(
            value[0]["command"],
            "/Applications/Dunlin.app/Contents/MacOS/Dunlin"
        );
        assert_eq!(value[0]["args"][0], "--mcp-vault");
        assert_eq!(value[0]["env"][0]["name"], "DUNLIN_VAULT_SOCKET");
        assert_eq!(value[0]["env"][1]["value"], "token");
    }

    #[test]
    fn builds_a_unified_diff_for_file_edits() {
        let content = vec![ToolCallContent::Diff(
            agent_client_protocol::v1::schema::Diff::new("/tmp/hello.rs", "one\ntwo\n")
                .old_text("one\nthree\n"),
        )];
        let (detail, additions, deletions, diff) = tool_content(&content);
        assert_eq!(detail, "/tmp/hello.rs");
        assert_eq!((additions, deletions), (2, 2));
        let diff = diff.expect("diff should be present");
        assert!(diff.contains("-three"));
        assert!(diff.contains("+two"));
    }
}
