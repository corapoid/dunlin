//! ACP harness integration for Kestrel.
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

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AvailableCommandsUpdate, CancelNotification, ContentBlock, Error as AcpError,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, UsageUpdate,
};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, ConnectionTo, Lines, Result as AcpResult,
};
use async_channel::{Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use kestrel_domain::{ToolKind, ToolStatus};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
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
    },
}

/// Everything needed to start one ACP agent process or remote connection.
#[derive(Clone, Debug)]
pub struct HarnessConfig {
    pub transport: HarnessTransport,
    /// Session working directory. Must be absolute. For remote sessions this is
    /// the path on the relay.
    pub cwd: PathBuf,
    /// Client name reported during `initialize`.
    pub client_name: String,
}

impl HarnessConfig {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            transport: HarnessTransport::Local {
                command: command.into(),
            },
            cwd: cwd.into(),
            client_name: "Kestrel".into(),
        }
    }

    pub fn remote(
        url: impl Into<String>,
        token: impl Into<String>,
        project_id: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            transport: HarnessTransport::Remote {
                url: url.into(),
                token: token.into(),
                project_id: project_id.into(),
            },
            cwd: cwd.into(),
            client_name: "Kestrel".into(),
        }
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
    AgentMessage(String),
    AgentThought(String),
    ToolCall(ToolCallView),
    ToolCallUpdate(ToolCallDelta),
    Plan(Vec<PlanItem>),
    Usage {
        used: u64,
        size: u64,
        cost_usd: Option<f64>,
    },
    AvailableCommands(Vec<String>),
}

/// Events emitted by the harness worker.
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
    },
    Cancel,
    NewSession {
        cwd: PathBuf,
    },
    LoadSession {
        session_id: String,
        cwd: PathBuf,
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
    events: Receiver<HarnessEvent>,
    join: Option<JoinHandle<()>>,
    next_request: Arc<AtomicU64>,
}

impl AcpHarness {
    /// Start the agent process on a dedicated worker thread.
    pub fn spawn(config: HarnessConfig) -> Self {
        let (command_tx, command_rx) = async_channel::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();
        let next_request = Arc::new(AtomicU64::new(1));

        let join = {
            let next_request = next_request.clone();
            let remote = config.is_remote();
            thread::Builder::new()
                .name("kestrel-acp".into())
                .spawn(move || {
                    if remote {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("failed to build the remote ACP runtime");
                        runtime.block_on(worker(config, command_rx, event_tx, next_request));
                    } else {
                        async_io::block_on(worker(config, command_rx, event_tx, next_request));
                    }
                })
                .expect("failed to spawn ACP worker thread")
        };

        Self {
            commands: command_tx,
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
        self.send(HarnessCommand::Prompt { text: text.into() })
    }

    /// Ask the agent to cancel the current turn.
    pub fn cancel(&self) -> bool {
        self.send(HarnessCommand::Cancel)
    }

    /// Create a fresh ACP session in `cwd` and make it active.
    pub fn new_session(&self, cwd: impl Into<PathBuf>) -> bool {
        self.send(HarnessCommand::NewSession { cwd: cwd.into() })
    }

    /// Load an existing ACP session and replay its history.
    pub fn load_session(&self, session_id: impl Into<String>, cwd: impl Into<PathBuf>) -> bool {
        self.send(HarnessCommand::LoadSession {
            session_id: session_id.into(),
            cwd: cwd.into(),
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
) {
    let result = run(config, commands, events.clone(), next_request).await;
    if let Err(error) = result {
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
            Transport::Local(AcpAgent::from_str(command).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("could not parse harness command: {error}"),
                )
            })?)
        }
        HarnessTransport::Remote {
            url,
            token,
            project_id,
        } => Transport::Remote(WsTransport::new(url, token, project_id)?),
    };
    let pending: PendingPermissions = Arc::new(Mutex::new(HashMap::new()));

    let notification_events = events.clone();
    let permission_events = events.clone();
    let foreground_events = events.clone();
    let permission_pending = pending.clone();
    let permission_next = next_request.clone();

    Client
        .builder()
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
                let init = connection
                    .send_request(
                        InitializeRequest::new(ProtocolVersion::V1).client_info(
                            agent_client_protocol::schema::v1::Implementation::new(
                                "Kestrel",
                                env!("CARGO_PKG_VERSION"),
                            )
                            .title("Kestrel"),
                        ),
                    )
                    .block_task()
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

                let mut active_session = None;
                if !config.is_remote() {
                    let session = connection
                        .send_request(NewSessionRequest::new(config.cwd.clone()))
                        .block_task()
                        .await?;
                    active_session = Some(session.session_id.clone());
                    let _ = foreground_events
                        .send(HarnessEvent::SessionCreated {
                            session_id: session.session_id.to_string(),
                        })
                        .await;
                }

                let mut shutting_down = false;
                while let Ok(command) = commands.recv().await {
                    match command {
                        HarnessCommand::Prompt { text } => {
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
                                let response = prompt_connection
                                    .send_request(PromptRequest::new(
                                        session_id.clone(),
                                        vec![ContentBlock::Text(
                                            agent_client_protocol::schema::v1::TextContent::new(
                                                text,
                                            ),
                                        )],
                                    ))
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
                        HarnessCommand::NewSession { cwd } => {
                            match connection
                                .send_request(NewSessionRequest::new(cwd))
                                .block_task()
                                .await
                            {
                                Ok(session) => {
                                    active_session = Some(session.session_id.clone());
                                    let _ = foreground_events
                                        .send(HarnessEvent::SessionCreated {
                                            session_id: session.session_id.to_string(),
                                        })
                                        .await;
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
                        HarnessCommand::LoadSession { session_id, cwd } => {
                            match connection
                                .send_request(LoadSessionRequest::new(session_id.clone(), cwd))
                                .block_task()
                                .await
                            {
                                Ok(_) => {
                                    active_session = Some(session_id.clone().into());
                                    let _ = foreground_events
                                        .send(HarnessEvent::SessionLoaded {
                                            session_id: session_id.clone(),
                                        })
                                        .await;
                                }
                                Err(error) => {
                                    let _ = foreground_events
                                        .send(HarnessEvent::Error {
                                            message: format!("Could not load session: {error}"),
                                        })
                                        .await;
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

fn map_notification(notification: &SessionNotification) -> Option<HarnessUpdate> {
    match &notification.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            content_text(&chunk.content).map(HarnessUpdate::AgentMessage)
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            content_text(&chunk.content).map(HarnessUpdate::AgentThought)
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
    let (detail, additions, deletions) = tool_content(&tool.content);
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
    }
}

fn tool_delta(update: &ToolCallUpdate) -> ToolCallDelta {
    let (detail, additions, deletions) = update
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
    }
}

fn tool_content(content: &[ToolCallContent]) -> (String, u32, u32) {
    let mut detail = String::new();
    let mut additions = 0;
    let mut deletions = 0;

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
                detail.push_str(&diff.path.display().to_string());
                additions += diff.new_text.lines().count() as u32;
                deletions += diff
                    .old_text
                    .as_deref()
                    .map(|text| text.lines().count() as u32)
                    .unwrap_or(0);
            }
            _ => {}
        }
    }

    (detail, additions, deletions)
}

fn map_tool_kind(kind: agent_client_protocol::schema::v1::ToolKind) -> ToolKind {
    use agent_client_protocol::schema::v1::ToolKind as Acp;
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

fn map_plan_status(status: agent_client_protocol::schema::v1::PlanEntryStatus) -> PlanStatus {
    use agent_client_protocol::schema::v1::PlanEntryStatus as Acp;
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
        cost_usd: usage.cost.as_ref().map(|cost| cost.amount),
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

fn stop_reason_label(reason: agent_client_protocol::schema::v1::StopReason) -> String {
    use agent_client_protocol::schema::v1::StopReason as Acp;
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
}

impl WsTransport {
    fn new(url: &str, token: &str, project_id: &str) -> io::Result<Self> {
        let base = url.trim_end_matches('/');
        let ws = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            base.to_string()
        };
        let endpoint = if ws.ends_with("/acp") {
            format!("{ws}?project={project_id}")
        } else {
            format!("{ws}/acp?project={project_id}")
        };
        let mut request = endpoint
            .into_client_request()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        Ok(Self { request })
    }
}

impl ConnectTo<Client> for WsTransport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> AcpResult<()> {
        let (socket, _) = connect_async(self.request).await.map_err(|error| {
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

    fn notification(update: SessionUpdate) -> SessionNotification {
        SessionNotification::new("session-1", update)
    }

    #[test]
    fn maps_message_and_thought_chunks() {
        let message = notification(SessionUpdate::AgentMessageChunk(
            agent_client_protocol::schema::v1::ContentChunk::new(ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new("hello"),
            )),
        ));
        assert_eq!(
            map_notification(&message),
            Some(HarnessUpdate::AgentMessage("hello".into()))
        );

        let thought = notification(SessionUpdate::AgentThoughtChunk(
            agent_client_protocol::schema::v1::ContentChunk::new(ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new("thinking"),
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
            .cost(agent_client_protocol::schema::v1::Cost::new(0.25, "USD"));
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
    fn maps_available_commands() {
        let commands = AvailableCommandsUpdate::new(vec![
            agent_client_protocol::schema::v1::AvailableCommand::new("init", "Initialize"),
            agent_client_protocol::schema::v1::AvailableCommand::new("review", "Review"),
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
        use agent_client_protocol::schema::v1::{ToolCallStatus as AcpStatus, ToolKind as AcpKind};

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
        use agent_client_protocol::schema::v1::StopReason as Acp;
        assert_eq!(stop_reason_label(Acp::EndTurn), "end_turn");
        assert_eq!(stop_reason_label(Acp::Refusal), "refusal");
        assert_eq!(stop_reason_label(Acp::Cancelled), "cancelled");
        assert_eq!(stop_reason_label(Acp::MaxTokens), "max_tokens");
    }

    #[test]
    fn extracts_text_from_tool_content() {
        let content = vec![ToolCallContent::Content(
            agent_client_protocol::schema::v1::Content::new(ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new("first line\nsecond line"),
            )),
        )];
        let (detail, additions, deletions) = tool_content(&content);
        assert_eq!(detail, "first line\nsecond line");
        assert_eq!((additions, deletions), (0, 0));
    }
}
