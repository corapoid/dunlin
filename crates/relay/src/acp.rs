//! ACP-over-WebSocket bridge.
//!
//! One WebSocket connection owns one `opencode acp` process. The relay is a
//! transparent JSON-RPC proxy with a small session tracker: it validates the
//! project root, records session ownership, and enforces a single active device
//! per session. Permission prompts are never answered by the relay.

use std::collections::HashMap;
use std::path::Path;
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
    let current_path = project.path.clone();
    let other_projects = state
        .store
        .lock()
        .ok()
        .and_then(|store| store.list_projects().ok())
        .map(|projects| {
            projects
                .into_iter()
                .filter(|candidate| candidate.path != current_path)
                .map(|candidate| candidate.path)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut harness = harness_command(
        &state.config,
        &program,
        &args,
        &project.path,
        &other_projects,
    );
    let stderr = if std::env::var_os("DUNLIN_RELAY_DEBUG").is_some() {
        Stdio::inherit()
    } else {
        Stdio::null()
    };
    let mut child = match harness
        .current_dir(&project.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
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
    let mut client_gone = false;
    let mut taken_over = false;
    let mut harness_exited = false;

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
                        harness_exited = true;
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
                        client_gone = true;
                        break;
                    }
                    None => {
                        client_gone = true;
                        break;
                    }
                    Some(Err(error)) => {
                        eprintln!("[relay] websocket error · {connection_id} · {error}");
                        client_gone = true;
                        break;
                    }
                }
            }
            _ = close_rx.recv() => {
                eprintln!("[relay] session taken over · {connection_id}");
                taken_over = true;
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
                    client_gone = true;
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

    if client_gone && !taken_over && !harness_exited && tracker.has_active_prompt() {
        let timeout = Duration::from_secs(state.config.linger_secs);
        if let Ok(mut leases) = state.leases.lock() {
            for lease in leases.values_mut() {
                if lease.connection_id == connection_id {
                    lease.expires_at = Instant::now() + timeout + Duration::from_secs(60);
                }
            }
        }
        eprintln!(
            "[relay] client left with an active turn · lingering for {}s · {connection_id}",
            state.config.linger_secs
        );
        tokio::spawn(linger_turn(
            child,
            stdin,
            lines,
            tracker,
            state,
            user,
            device,
            project,
            connection_id,
            close_rx,
            timeout,
        ));
        return;
    }

    let _ = child.kill().await;
    eprintln!("[relay] acp disconnected · {connection_id}");
    if let Ok(mut leases) = state.leases.lock() {
        leases.retain(|_, lease| lease.connection_id != connection_id);
    }
}

/// Keep a harness alive after the last client disconnects so an in-flight turn
/// can finish. Permission requests are auto-cancelled; the result stays in
/// OpenCode and can be loaded from another device later.
#[allow(clippy::too_many_arguments)]
async fn linger_turn(
    mut child: tokio::process::Child,
    mut stdin: tokio::process::ChildStdin,
    mut lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    mut tracker: Tracker,
    state: AppState,
    user: User,
    device: Device,
    project: Project,
    connection_id: String,
    mut close_rx: mpsc::Receiver<()>,
    timeout: Duration,
) {
    let (dummy_tx, _dummy_rx) = mpsc::channel::<()>(1);
    let deadline = tokio::time::Instant::now() + timeout;
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
                        if value.get("method").and_then(Value::as_str)
                            == Some("session/request_permission")
                            && let Some(id) = value.get("id")
                        {
                            let response = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": { "outcome": { "outcome": "cancelled" } }
                            });
                            if stdin.write_all(response.to_string().as_bytes()).await.is_err()
                                || stdin.write_all(b"\n").await.is_err()
                                || stdin.flush().await.is_err()
                            {
                                break;
                            }
                        }
                        tracker.on_agent_message(
                            &state,
                            &user,
                            &device,
                            &project,
                            &connection_id,
                            &dummy_tx,
                            &value,
                        );
                        if !tracker.has_active_prompt() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            _ = close_rx.recv() => {
                eprintln!("[relay] linger cancelled · {connection_id}");
                break;
            }
            _ = tokio::time::sleep_until(deadline) => {
                eprintln!("[relay] linger timeout · {connection_id}");
                break;
            }
        }
    }
    let _ = child.kill().await;
    if let Ok(mut leases) = state.leases.lock() {
        leases.retain(|_, lease| lease.connection_id != connection_id);
    }
    eprintln!("[relay] lingering turn finished · {connection_id}");
}

#[derive(Default)]
struct Tracker {
    pending: HashMap<String, Pending>,
}

enum Pending {
    New,
    Load { session_id: String },
    Prompt,
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
                if is_request {
                    self.pending.insert(id_key(&id), Pending::Prompt);
                }
                Ok(())
            }
            "session/cancel"
            | "session/set_config_option"
            | "session/set_mode"
            | "session/set_model" => {
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
            Pending::Prompt => {}
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

    fn has_active_prompt(&self) -> bool {
        self.pending
            .values()
            .any(|pending| matches!(pending, Pending::Prompt))
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

/// Build the harness command, optionally wrapped in a macOS seatbelt profile.
///
/// The profile is allow-by-default with an explicit deny list: other project
/// roots, credential stores, browser profiles and personal folders. That keeps
/// OpenCode working while preventing an agent from reading another project or a
/// user's keys. It is a best-effort privacy boundary, not a container; use
/// separate OS accounts or containers for hard isolation.
fn harness_command(
    config: &crate::config::Config,
    program: &str,
    args: &[String],
    project_path: &str,
    other_projects: &[String],
) -> Command {
    if config.sandbox && cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        let profile = sandbox_profile(project_path, other_projects, &config.harness);
        if std::env::var_os("DUNLIN_RELAY_DEBUG").is_some() {
            eprintln!("[relay] sandbox profile:\n{profile}");
        }
        command.arg("-p").arg(profile).arg(program).args(args);
        command
    } else {
        let mut command = Command::new(program);
        command.args(args);
        command
    }
}

fn sandbox_profile(project_path: &str, other_projects: &[String], harness: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let project = Path::new(project_path);
    let mut denied = Vec::new();

    if !home.is_empty() {
        let home_path = Path::new(&home);
        let mut private = vec![
            ".ssh",
            ".gnupg",
            ".aws",
            ".azure",
            ".config/gcloud",
            ".kube",
            ".docker",
            ".netrc",
            ".npmrc",
            ".pypirc",
            "Library/Keychains",
            "Library/Messages",
            "Library/Mail",
            "Library/Safari",
            "Library/Application Support/Google/Chrome",
            "Documents",
            "Desktop",
            "Downloads",
            "Pictures",
            "Movies",
            "Music",
        ];
        if !harness.contains("codex") {
            private.push(".codex");
        }
        if !harness.contains("claude") {
            private.push(".claude");
        }
        for suffix in private {
            let path = home_path.join(suffix);
            // A project can legitimately live under Documents or Desktop; never
            // deny the active project itself.
            if !project.starts_with(&path) {
                denied.push(path.to_string_lossy().to_string());
            }
        }
    }
    for path in other_projects {
        if path != project_path {
            denied.push(path.clone());
        }
    }

    let mut profile = String::from("(version 1)\n(allow default)\n");
    profile.push_str("(deny file-read*");
    for path in &denied {
        profile.push_str(&format!(" (subpath \"{}\")", escape_sandbox_path(path)));
    }
    profile.push_str(")\n(deny file-write*");
    for path in &denied {
        profile.push_str(&format!(" (subpath \"{}\")", escape_sandbox_path(path)));
    }
    profile.push_str(")\n");
    profile
}

fn escape_sandbox_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{command_parts, prompt_title, sandbox_profile};
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

    #[test]
    fn sandbox_denies_other_projects_and_credentials() {
        let profile = sandbox_profile(
            "/Users/me/project",
            &["/Users/me/other".to_string()],
            "opencode acp",
        );
        assert!(profile.starts_with("(version 1)\n(allow default)"));
        assert!(profile.contains("/Users/me/other"));
        assert!(profile.contains(".ssh"));
        assert!(profile.contains(".codex"));
        assert!(!profile.contains("(subpath \"/Users/me/project\")"));
    }

    #[test]
    fn sandbox_does_not_deny_the_active_project_under_documents() {
        let profile = sandbox_profile("/Users/me/Documents/project", &[], "opencode acp");
        assert!(!profile.contains("(subpath \"/Users/me/Documents\")"));
    }
}
