//! ACP-over-WebSocket bridge.
//!
//! One WebSocket connection owns one `opencode acp` process. The relay is a
//! transparent JSON-RPC proxy with a small session tracker: it validates the
//! project root, records session ownership, and enforces a single active device
//! per session. Permission prompts are never answered by the relay.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::api::{AppState, Lease};
use crate::store::{Device, Project, SessionOwner, User};

const SESSION_NOT_FOUND: i64 = -32001;
const SESSION_BUSY: i64 = -32002;
const INVALID_PARAMS: i64 = -32602;

pub async fn bridge(
    socket: WebSocket,
    state: AppState,
    user: User,
    device: Device,
    project: Project,
    connection_id: String,
) {
    eprintln!(
        "[relay] acp connected · user {} · project {} · {connection_id}",
        user.name, project.name
    );
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);

    let (program, args) = command_parts(&state.config.harness);
    let mut child = match Command::new(&program)
        .args(&args)
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let message = format!(
                "Could not start harness `{}`: {error}",
                state.config.harness
            );
            eprintln!("[relay] {message}");
            let _ = ws_tx
                .send(Message::Text(
                    error_response(Value::Null, -32603, &message, None)
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = ws_tx
                .send(Message::Close(Some(CloseFrame {
                    code: 1011,
                    reason: "harness unavailable".into(),
                })))
                .await;
            return;
        }
    };

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill().await;
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut tracker = Tracker::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    let mut last_activity = Instant::now();

    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if line.len() > state.config.max_frame_bytes {
                            break;
                        }
                        let Ok(value) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        tracker.on_agent_message(
                            &state,
                            &user,
                            &device,
                            &project,
                            &connection_id,
                            &close_tx,
                            &value,
                        );
                        last_activity = Instant::now();
                        if ws_tx.send(Message::Text(line.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => {
                        eprintln!("[relay] harness stdout closed · {connection_id}");
                        break;
                    }
                }
            }
            message = ws_rx.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > state.config.max_frame_bytes {
                            break;
                        }
                        let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                            let _ = ws_tx
                                .send(Message::Text(
                                    error_response(Value::Null, -32700, "Invalid JSON", None)
                                        .to_string()
                                        .into(),
                                ))
                                .await;
                            continue;
                        };
                        match tracker.on_client_message(
                            &state,
                            &user,
                            &device,
                            &project,
                            &connection_id,
                            &close_tx,
                            &value,
                        ) {
                            Ok(Some(outgoing)) => {
                                if let Err(error) = stdin.write_all(outgoing.as_bytes()).await {
                                    eprintln!("[relay] harness stdin failed · {error}");
                                    break;
                                }
                                if let Err(error) = stdin.write_all(b"\n").await {
                                    eprintln!("[relay] harness stdin failed · {error}");
                                    break;
                                }
                                if let Err(error) = stdin.flush().await {
                                    eprintln!("[relay] harness stdin failed · {error}");
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                if ws_tx
                                    .send(Message::Text(error.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        last_activity = Instant::now();
                    }
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws_tx.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_activity = Instant::now();
                    }
                    Some(Ok(Message::Close(frame))) => {
                        eprintln!("[relay] client closed · {connection_id} · {frame:?}");
                        break;
                    }
                    None => break,
                    Some(Err(error)) => {
                        eprintln!("[relay] websocket error · {connection_id} · {error}");
                        break;
                    }
                }
            }
            _ = close_rx.recv() => {
                eprintln!("[relay] session taken over · {connection_id}");
                let _ = ws_tx
                    .send(Message::Close(Some(CloseFrame {
                        code: 4001,
                        reason: "Session taken over".into(),
                    })))
                    .await;
                break;
            }
            _ = ticker.tick() => {
                if last_activity.elapsed() > Duration::from_secs(state.config.idle_timeout_secs) {
                    eprintln!("[relay] idle timeout · {connection_id}");
                    break;
                }
                let _ = ws_tx.send(Message::Ping(Vec::new().into())).await;
                if let Ok(mut leases) = state.leases.lock() {
                    let now = Instant::now();
                    for lease in leases.values_mut() {
                        if lease.connection_id == connection_id && lease.expires_at > now {
                            lease.expires_at =
                                now + Duration::from_secs(state.config.lease_secs);
                        }
                    }
                    leases.retain(|_, lease| {
                        lease.connection_id != connection_id || lease.expires_at > now
                    });
                }
            }
        }
    }

    let _ = child.kill().await;
    eprintln!("[relay] acp disconnected · {connection_id}");
    if let Ok(mut leases) = state.leases.lock() {
        leases.retain(|_, lease| lease.connection_id != connection_id);
    }
}

#[derive(Default)]
struct Tracker {
    pending: HashMap<String, Pending>,
}

enum Pending {
    New,
    Load { session_id: String },
    Fork { source_id: String, title: String },
    Delete { session_id: String },
}

impl Tracker {
    #[allow(clippy::too_many_arguments)]
    fn on_client_message(
        &mut self,
        state: &AppState,
        user: &User,
        device: &Device,
        project: &Project,
        connection_id: &str,
        close_tx: &mpsc::Sender<()>,
        value: &Value,
    ) -> Result<Option<String>, Value> {
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            // A response to an agent request, such as a permission decision.
            return Ok(Some(value.to_string()));
        };
        eprintln!("[relay] acp method · {method}");
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let is_request = value.get("id").is_some();

        let result = match method {
            "session/new" => {
                check_cwd(project, value).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                if is_request {
                    self.pending.insert(id_key(&id), Pending::New);
                }
                Ok(())
            }
            "session/load" | "session/resume" => {
                check_cwd(project, value).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                let session_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                check_owner(state, user, &session_id, &id)?;
                acquire_lease(state, &session_id, device, connection_id, close_tx, &id)?;
                if is_request {
                    self.pending
                        .insert(id_key(&id), Pending::Load { session_id });
                }
                Ok(())
            }
            "session/fork" => {
                check_cwd(project, value).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                let source_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                let owner = check_owner(state, user, &source_id, &id)?;
                if is_request {
                    self.pending.insert(
                        id_key(&id),
                        Pending::Fork {
                            source_id,
                            title: owner.title,
                        },
                    );
                }
                Ok(())
            }
            "session/delete" => {
                let session_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                check_owner(state, user, &session_id, &id)?;
                if is_request {
                    self.pending
                        .insert(id_key(&id), Pending::Delete { session_id });
                }
                Ok(())
            }
            "session/prompt" => {
                let session_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                check_owner(state, user, &session_id, &id)?;
                acquire_lease(state, &session_id, device, connection_id, close_tx, &id)?;
                if let Some(title) = prompt_title(value)
                    && let Ok(store) = state.store.lock()
                {
                    let _ = store.rename_session_if_new(&session_id, &title);
                }
                Ok(())
            }
            "session/cancel" | "session/set_config_option" | "session/set_mode" => {
                if let Some(session_id) = session_id(value) {
                    check_owner(state, user, &session_id, &id)?;
                    refresh_lease(state, &session_id, connection_id);
                }
                Ok(())
            }
            _ => Ok(()),
        };

        match result {
            Ok(()) => Ok(Some(value.to_string())),
            Err(error) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_agent_message(
        &mut self,
        state: &AppState,
        user: &User,
        device: &Device,
        project: &Project,
        connection_id: &str,
        close_tx: &mpsc::Sender<()>,
        value: &Value,
    ) {
        // Agent-initiated requests also carry an `id`; only responses have no
        // method. Keeping the two directions apart avoids id collisions.
        if value.get("method").is_some() {
            return;
        }
        let Some(id) = value.get("id") else {
            return;
        };
        let Some(pending) = self.pending.remove(&id_key(id)) else {
            return;
        };
        if value.get("error").is_some() {
            return;
        }
        let Some(result) = value.get("result") else {
            return;
        };

        match pending {
            Pending::New => {
                if let Some(session_id) = result_session_id(result) {
                    if let Ok(store) = state.store.lock() {
                        let _ = store.create_session(
                            &session_id,
                            &user.id,
                            &project.id,
                            "New session",
                            &device.id,
                        );
                    }
                    let _ = acquire_lease(
                        state,
                        &session_id,
                        device,
                        connection_id,
                        close_tx,
                        &Value::Null,
                    );
                }
            }
            Pending::Load { session_id } => {
                if let Ok(store) = state.store.lock() {
                    let _ = store.touch_session(&session_id, &device.id);
                }
                refresh_lease(state, &session_id, connection_id);
            }
            Pending::Fork { source_id, title } => {
                if let Some(session_id) = result_session_id(result) {
                    if let Ok(store) = state.store.lock()
                        && let Some(owner) = store.session_owner(&source_id).ok().flatten()
                    {
                        let _ = store.create_session(
                            &session_id,
                            &user.id,
                            &owner.project_id,
                            &format!("{title} (fork)"),
                            &device.id,
                        );
                    }
                    let _ = acquire_lease(
                        state,
                        &session_id,
                        device,
                        connection_id,
                        close_tx,
                        &Value::Null,
                    );
                }
            }
            Pending::Delete { session_id } => {
                if let Ok(store) = state.store.lock() {
                    let _ = store.delete_session(&session_id);
                }
                if let Ok(mut leases) = state.leases.lock() {
                    leases.remove(&session_id);
                }
            }
        }
    }
}

fn check_cwd(project: &Project, value: &Value) -> Result<(), String> {
    let cwd = value
        .pointer("/params/cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| "Session requests need an absolute cwd".to_string())?;
    let requested =
        std::fs::canonicalize(cwd).map_err(|error| format!("cwd is not available: {error}"))?;
    let root = std::fs::canonicalize(&project.path)
        .map_err(|error| format!("Project is not available: {error}"))?;
    if requested != root {
        return Err("cwd must be the project root".into());
    }
    Ok(())
}

fn check_owner(
    state: &AppState,
    user: &User,
    session_id: &str,
    id: &Value,
) -> Result<SessionOwner, Value> {
    let owner = state
        .store
        .lock()
        .map_err(|_| error_response(id.clone(), -32603, "Store unavailable", None))?
        .session_owner(session_id)
        .map_err(|_| error_response(id.clone(), -32603, "Store error", None))?;
    match owner {
        Some(owner) if owner.user_id == user.id => Ok(owner),
        _ => Err(error_response(
            id.clone(),
            SESSION_NOT_FOUND,
            "Session not found",
            None,
        )),
    }
}

fn acquire_lease(
    state: &AppState,
    session_id: &str,
    device: &Device,
    connection_id: &str,
    close_tx: &mpsc::Sender<()>,
    id: &Value,
) -> Result<(), Value> {
    let mut leases = state
        .leases
        .lock()
        .map_err(|_| error_response(id.clone(), -32603, "Lease store unavailable", None))?;
    let now = Instant::now();
    if let Some(lease) = leases.get(session_id)
        && lease.expires_at > now
        && lease.connection_id != connection_id
    {
        return Err(error_response(
            id.clone(),
            SESSION_BUSY,
            "Session is active on another device",
            Some(json!({ "device_id": lease.device_id })),
        ));
    }
    leases.insert(
        session_id.to_string(),
        Lease {
            device_id: device.id.clone(),
            connection_id: connection_id.to_string(),
            close_tx: close_tx.clone(),
            expires_at: now + Duration::from_secs(state.config.lease_secs),
        },
    );
    Ok(())
}

fn refresh_lease(state: &AppState, session_id: &str, connection_id: &str) {
    if let Ok(mut leases) = state.leases.lock()
        && let Some(lease) = leases.get_mut(session_id)
        && lease.connection_id == connection_id
    {
        lease.expires_at = Instant::now() + Duration::from_secs(state.config.lease_secs);
    }
}

fn session_id(value: &Value) -> Option<String> {
    value
        .pointer("/params/sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn result_session_id(result: &Value) -> Option<String> {
    result
        .get("sessionId")
        .or_else(|| result.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn prompt_title(value: &Value) -> Option<String> {
    let prompt = value.pointer("/params/prompt")?.as_array()?;
    for block in prompt {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = block.get("text").and_then(Value::as_str)?;
        let line = text.lines().find(|line| !line.trim().is_empty())?;
        return Some(line.trim().chars().take(80).collect());
    }
    None
}

fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".into())
}

fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({
        "code": code,
        "message": message,
    });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error,
    })
}

fn command_parts(command: &str) -> (String, Vec<String>) {
    let mut parts = command.split_whitespace();
    let program = parts.next().unwrap_or("opencode").to_string();
    let args = parts.map(str::to_string).collect();
    (program, args)
}

#[cfg(test)]
mod tests {
    use super::{command_parts, prompt_title};
    use serde_json::json;

    #[test]
    fn splits_the_harness_command() {
        let (program, args) = command_parts("opencode acp");
        assert_eq!(program, "opencode");
        assert_eq!(args, vec!["acp"]);
    }

    #[test]
    fn takes_the_first_prompt_line_as_a_title() {
        let value = json!({
            "params": {
                "prompt": [
                    { "type": "text", "text": "  Fix the relay  \nand add tests" }
                ]
            }
        });
        assert_eq!(prompt_title(&value).as_deref(), Some("Fix the relay"));
    }
}
