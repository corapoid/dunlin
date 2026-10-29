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

mod models;
pub use models::{ModelChoice, SessionModels, SessionModes};
use models::{session_models, session_modes};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AvailableCommandsUpdate, CancelNotification, ContentBlock, Error as AcpError,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, UsageUpdate,
};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, ConnectionTo, Lines, Result as AcpResult, UntypedMessage,
};
use async_channel::{Receiver, Sender};
use dunlin_domain::{ToolKind, ToolStatus};
use futures_util::{SinkExt, StreamExt};
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
    pub resume_session_id: Option<String>,
}

impl HarnessConfig {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            transport: HarnessTransport::Local {
                command: command.into(),
            },
            cwd: cwd.into(),
            client_name: "Dunlin".into(),
            resume_session_id: None,
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
            client_name: "Dunlin".into(),
            resume_session_id: None,
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
    },
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
                .name("dunlin-acp".into())
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
                                "Dunlin",
                                env!("CARGO_PKG_VERSION"),
                            )
                            .title("Dunlin"),
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

                let mut active_session: Option<agent_client_protocol::schema::v1::SessionId> = None;
                if !config.is_remote() {
                    if let Some(session_id) = config.resume_session_id.as_ref() {
                        if init.agent_capabilities.load_session {
                            match open_session(&connection, &foreground_events, config.cwd.clone(), Some(session_id.clone())).await {
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
                        active_session = Some(open_session(&connection, &foreground_events, config.cwd.clone(), None).await?.into());
                    }
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
                            active_session = None;
                            match open_session(&connection, &foreground_events, cwd, None).await {
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
                        HarnessCommand::LoadSession { session_id, cwd } => {
                            active_session = None;
                            match open_session(&connection, &foreground_events, cwd, Some(session_id.clone())).await {
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
                                let request = if let Some(config_id) = config_id.as_ref() {
                                    UntypedMessage::new("session/set_config_option", serde_json::json!({
                                        "sessionId": session_id, "configId": config_id, "value": model_id,
                                    }))?
                                } else {
                                    UntypedMessage::new("session/set_model", serde_json::json!({
                                        "sessionId": session_id, "modelId": model_id,
                                    }))?
                                };
                                connection.send_request(request).block_task().await
                            }.await;
                            match result {
                                Ok(response) => {
                                    // Older agents return an empty object on success.
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

async fn open_session(
    connection: &ConnectionTo<Agent>,
    events: &Sender<HarnessEvent>,
    cwd: PathBuf,
    existing: Option<String>,
) -> AcpResult<String> {
    // Keep the raw response: some deployed harnesses still expose `models`,
    // which newer ACP schemas intentionally no longer deserialize.
    let request = if let Some(id) = existing.as_ref() {
        UntypedMessage::new("session/load", LoadSessionRequest::new(id.clone(), cwd))?
    } else {
        UntypedMessage::new("session/new", NewSessionRequest::new(cwd))?
    };
    let response = connection.send_request(request).block_task().await?;
    let id = existing
        .clone()
        .or_else(|| {
            response
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| AcpError::new(-32603, "The harness did not return a session id"))?;
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
            state: session_models(&response),
        })
        .await;
    let _ = events
        .send(HarnessEvent::Modes {
            session_id: id.clone(),
            state: session_modes(&response),
        })
        .await;
    Ok(id)
}

fn map_notification(notification: &SessionNotification) -> Option<HarnessUpdate> {
    match &notification.update {
        SessionUpdate::UserMessageChunk(chunk) => {
            content_text(&chunk.content).map(HarnessUpdate::UserMessage)
        }
        SessionUpdate::ConfigOptionUpdate(_) => serde_json::to_value(&notification.update)
            .ok()
            .map(|value| HarnessUpdate::Configuration {
                models: session_models(&value),
                modes: session_modes(&value),
            }),
        SessionUpdate::CurrentModeUpdate(update) => Some(HarnessUpdate::Configuration {
            models: None,
            modes: Some(SessionModes {
                config_id: None,
                current: update.current_mode_id.to_string(),
                choices: Vec::new(),
            }),
        }),
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
            } => {
                assert_eq!(models.current, "a");
                assert_eq!(modes.current, "plan");
            }
            other => panic!("unexpected update: {other:?}"),
        }
        let update = SessionUpdate::CurrentModeUpdate(
            agent_client_protocol::schema::v1::CurrentModeUpdate::new("build"),
        );
        match map_notification(&notification(update)).unwrap() {
            HarnessUpdate::Configuration {
                models: None,
                modes: Some(modes),
            } => assert_eq!(modes.current, "build"),
            other => panic!("unexpected mode notification: {other:?}"),
        }
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
        let (detail, additions, deletions, diff) = tool_content(&content);
        assert_eq!(detail, "first line\nsecond line");
        assert_eq!((additions, deletions), (0, 0));
        assert!(diff.is_none());
    }

    #[test]
    fn builds_a_unified_diff_for_file_edits() {
        let content = vec![ToolCallContent::Diff(
            agent_client_protocol::schema::v1::Diff::new("/tmp/hello.rs", "one\ntwo\n")
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
