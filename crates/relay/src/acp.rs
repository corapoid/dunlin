//! ACP-over-WebSocket bridge.
//!
//! One WebSocket connection owns one `opencode acp` process. The relay is a
//! transparent JSON-RPC proxy with a small session tracker: it validates the
//! project root, records session ownership, and enforces a single active device
//! per session. Permission prompts are never answered by the relay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::api::{AppState, Lease};
use crate::store::{Device, Project, SessionOwner, User};
use dunlin_domain::{PermissionMode, ProjectSettings, validate_project_environment};

const SESSION_NOT_FOUND: i64 = -32001;
const SESSION_BUSY: i64 = -32002;
const INVALID_PARAMS: i64 = -32602;
const MAX_PENDING: usize = 256;

#[allow(clippy::too_many_arguments)]
pub async fn bridge(
    socket: WebSocket,
    state: AppState,
    user: User,
    device: Device,
    project: Project,
    allowed_roots: Vec<Project>,
    connection_id: String,
    _permit: OwnedSemaphorePermit,
) {
    // Names are user-controlled and unnecessary in operational logs. Keep the
    // server-generated connection identity, never raw user/project names.
    eprintln!(
        "[relay] acp connected · {connection_id} · {} explicit root(s)",
        allowed_roots.len() + 1
    );
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);

    let (program, args) = command_parts(&state.config.harness);
    let current_path = project.path.clone();
    let extra_paths = allowed_roots
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let mut all_projects = state
        .store
        .lock()
        .ok()
        .and_then(|store| store.list_projects().ok())
        .map(|projects| {
            projects
                .into_iter()
                .map(|candidate| candidate.path)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for path in &extra_paths {
        if !all_projects.contains(path) {
            all_projects.push(path.clone());
        }
    }
    let other_projects = all_projects
        .into_iter()
        .filter(|path| path != &current_path && !extra_paths.contains(path))
        .collect::<Vec<_>>();
    let mut allowed_paths = std::iter::once(&project.path)
        .chain(extra_paths.iter())
        .map(|path| {
            std::fs::canonicalize(path)
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.clone())
        })
        .collect::<Vec<_>>();
    allowed_paths.sort();
    allowed_paths.dedup();
    let project_settings = state
        .store
        .lock()
        .ok()
        .and_then(|store| store.project_settings(&project.id).ok().flatten())
        .and_then(|payload| serde_json::from_str::<ProjectSettings>(&payload).ok())
        .unwrap_or_default();
    let mut harness = harness_command(
        &state.config,
        &program,
        &args,
        &project.path,
        &extra_paths,
        &other_projects,
        &project_settings,
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
            eprintln!("[relay] could not start harness");
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
    let mut tracker = Tracker {
        pending: HashMap::new(),
        allowed_roots: allowed_paths,
        allowed_project_ids: allowed_roots
            .iter()
            .map(|project| project.id.clone())
            .collect(),
        vault_read_only: project_settings.permission_mode == Some(PermissionMode::ReadOnly),
        vault_sessions: Vec::new(),
    };
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
                if !still_authorized(&state, &user, &device, &project) {
                    eprintln!("[relay] authorization revoked · {connection_id}");
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
            _permit,
        ));
        return;
    }

    for session in &tracker.vault_sessions {
        session.revoke();
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
    _permit: OwnedSemaphorePermit,
) {
    let (dummy_tx, _dummy_rx) = mpsc::channel::<()>(1);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_authorization_check = Instant::now();
    loop {
        if last_authorization_check.elapsed() >= Duration::from_secs(30) {
            if !still_authorized(&state, &user, &device, &project) {
                eprintln!("[relay] lingering turn authorization revoked · {connection_id}");
                break;
            }
            last_authorization_check = Instant::now();
        }
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
    for session in &tracker.vault_sessions {
        session.revoke();
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
    /// Canonical project roots explicitly granted for this connection. An ACP
    /// session may request a subset, never an arbitrary host path.
    allowed_roots: Vec<String>,
    allowed_project_ids: Vec<String>,
    vault_read_only: bool,
    vault_sessions: Vec<crate::vault::VaultSession>,
}

enum Pending {
    New,
    Load { session_id: String },
    Prompt,
    Fork { source_id: String, title: String },
    Delete { session_id: String },
}

impl Tracker {
    fn roots_authorized(&self, state: &AppState, user: &User) -> bool {
        if self.allowed_project_ids.is_empty() {
            return true;
        }
        let Ok(store) = state.store.lock() else {
            return false;
        };
        let Ok(projects) = store.projects_for_user(&user.id) else {
            return false;
        };
        self.allowed_project_ids.iter().all(|id| {
            let Some(root) = projects.iter().find(|project| &project.id == id) else {
                return false;
            };
            let Ok(canonical) = std::fs::canonicalize(&root.path) else {
                return false;
            };
            let canonical = canonical.to_string_lossy();
            self.allowed_roots
                .iter()
                .any(|path| canonical == path.as_str())
        })
    }

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
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let mut forwarded = value.clone();
        if !still_authorized(state, user, device, project) || !self.roots_authorized(state, user) {
            return Err(error_response(
                id,
                -32003,
                "Relay authorization changed; reconnect to continue",
                None,
            ));
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            // A response to an agent request, such as a permission decision.
            return Ok(Some(value.to_string()));
        };
        eprintln!("[relay] acp method · {method:?}");
        let is_request = value.get("id").is_some();
        if is_request
            && !self.pending.contains_key(&id_key(&id))
            && self.pending.len() >= MAX_PENDING
        {
            return Err(error_response(
                id,
                SESSION_BUSY,
                "Too many pending ACP requests",
                None,
            ));
        }

        let result = match method {
            "session/new" => {
                check_cwd(project, value).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                check_session_scope(value, &self.allowed_roots).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                self.inject_vault(&mut forwarded, state, project, &id)?;
                if is_request {
                    self.pending.insert(id_key(&id), Pending::New);
                }
                Ok(())
            }
            "session/load" | "session/resume" => {
                check_cwd(project, value).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                check_session_scope(value, &self.allowed_roots).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                let session_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                check_owner(state, user, project, &session_id, &id)?;
                acquire_lease(state, &session_id, device, connection_id, close_tx, &id)?;
                self.inject_vault(&mut forwarded, state, project, &id)?;
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
                check_session_scope(value, &self.allowed_roots).map_err(|message| {
                    error_response(id.clone(), INVALID_PARAMS, &message, None)
                })?;
                let source_id = session_id(value).ok_or_else(|| {
                    error_response(id.clone(), INVALID_PARAMS, "Missing sessionId", None)
                })?;
                let owner = check_owner(state, user, project, &source_id, &id)?;
                self.inject_vault(&mut forwarded, state, project, &id)?;
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
                check_owner(state, user, project, &session_id, &id)?;
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
                check_owner(state, user, project, &session_id, &id)?;
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
                    check_owner(state, user, project, &session_id, &id)?;
                    refresh_lease(state, &session_id, connection_id);
                }
                Ok(())
            }
            _ => Ok(()),
        };

        match result {
            Ok(()) => Ok(Some(forwarded.to_string())),
            Err(error) => Err(error),
        }
    }

    fn inject_vault(
        &mut self,
        value: &mut Value,
        state: &AppState,
        project: &Project,
        id: &Value,
    ) -> Result<(), Value> {
        let Some(vault) = &state.vault else {
            return Ok(());
        };
        let session = vault.new_session(
            project.id.clone(),
            project.path.clone(),
            self.vault_read_only,
        );
        let executable = std::env::current_exe().map_err(|error| {
            error_response(
                id.clone(),
                INVALID_PARAMS,
                &format!("Could not locate the relay vault proxy: {error}"),
                None,
            )
        })?;
        let server = json!({
            "name": "dunlin-relay-secrets",
            "command": executable,
            "args": ["--mcp-vault"],
            "env": [
                {"name": "DUNLIN_VAULT_SOCKET", "value": vault.socket_path().to_string_lossy()},
                {"name": "DUNLIN_VAULT_TOKEN", "value": session.token}
            ]
        });
        let params = value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                error_response(id.clone(), INVALID_PARAMS, "Missing session params", None)
            })?;
        let servers = params
            .entry("mcpServers")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(array) = servers.as_array_mut() {
            array.push(server);
        } else {
            *servers = Value::Array(vec![server]);
        }
        self.vault_sessions.push(session);
        Ok(())
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
                    if let Ok(store) = state.store.lock()
                        && let Err(error) = store.create_session(
                            &session_id,
                            &user.id,
                            &project.id,
                            "New session",
                            &device.id,
                        )
                    {
                        eprintln!("[relay] session index rejected · {error}");
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
                        && let Err(error) = store.create_session(
                            &session_id,
                            &user.id,
                            &owner.project_id,
                            &format!("{title} (fork)"),
                            &device.id,
                        )
                    {
                        eprintln!("[relay] fork session index rejected · {error}");
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

/// Validate an ACP session's extra roots. A relay connection starts with the
/// anchor project plus an explicit, authenticated list of additional project
/// grants. A session may request a subset of those roots; anything else (or any
/// MCP server) is rejected instead of widening the harness silently.
fn check_session_scope(value: &Value, allowed_roots: &[String]) -> Result<(), String> {
    if let Some(servers) = value.pointer("/params/mcpServers") {
        match servers {
            Value::Null => {}
            Value::Array(items) if items.is_empty() => {}
            _ => {
                return Err(
                    "mcpServers are not allowed by this relay; configure tools on the host".into(),
                );
            }
        }
    }
    let requested = match value.pointer("/params/additionalDirectories") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err("additionalDirectories must be an array of absolute paths".into()),
    };
    if requested.len() > 32 {
        return Err("At most 32 additional workspace roots are allowed".into());
    }
    let mut seen = std::collections::HashSet::new();
    for item in requested {
        let path = item
            .as_str()
            .ok_or("additionalDirectories entries must be strings")?;
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| "Additional workspace root is not available")?
            .to_string_lossy()
            .to_string();
        if !allowed_roots.contains(&canonical) {
            return Err(
                "Additional workspace root is outside this relay connection's explicit grants"
                    .into(),
            );
        }
        if !seen.insert(canonical) {
            return Err("Duplicate additional workspace root".into());
        }
    }
    Ok(())
}

fn still_authorized(state: &AppState, user: &User, device: &Device, project: &Project) -> bool {
    let Ok(store) = state.store.lock() else {
        return false;
    };
    let active_device = store.list_devices(&user.id).ok().is_some_and(|devices| {
        devices
            .iter()
            .any(|entry| entry.id == device.id && entry.revoked_at.is_none())
    });
    if !active_device {
        return false;
    }
    store
        .project_for_user(&project.id, &user.id)
        .ok()
        .flatten()
        .is_some_and(|current| current.path == project.path)
}

fn check_owner(
    state: &AppState,
    user: &User,
    project: &Project,
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
        Some(owner) if owner.user_id == user.id && owner.project_id == project.id => Ok(owner),
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
    allowed_roots: &[String],
    other_projects: &[String],
    settings: &ProjectSettings,
) -> Command {
    let mut command = if config.sandbox
        && cfg!(target_os = "macos")
        && Path::new("/usr/bin/sandbox-exec").is_file()
    {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        let profile = sandbox_profile(
            project_path,
            allowed_roots,
            other_projects,
            program,
            &config.db,
        );
        if std::env::var_os("DUNLIN_RELAY_DEBUG").is_some() {
            eprintln!("[relay] sandbox profile:\n{profile}");
        }
        command.arg("-p").arg(profile).arg(program).args(args);
        command
    } else {
        let mut command = Command::new(program);
        command.args(args);
        command
    };
    scrub_harness_environment(&mut command);
    if validate_project_environment(&settings.environment).is_ok() {
        for (key, value) in &settings.environment {
            if !key.starts_with("DUNLIN_")
                && !key.starts_with("DYLD_")
                && !key.starts_with("LD_")
                && !matches!(key.as_str(), "PATH" | "BASH_ENV" | "ENV" | "ZDOTDIR")
            {
                command.env(key, value);
            }
        }
    }
    command
}

fn scrub_harness_environment(command: &mut Command) {
    for key in [
        "DUNLIN_REMOTE_TOKEN",
        "DUNLIN_RELAY_LISTEN",
        "DUNLIN_RELAY_DB",
        "DUNLIN_RELAY_HARNESS",
        "DUNLIN_RELAY_LINGER_SECS",
        "DUNLIN_RELAY_SANDBOX",
        "DUNLIN_RELAY_STORE_TRANSCRIPTS",
        "DUNLIN_RELAY_DEBUG",
        "DUNLIN_AUTOPROMPT",
        "DUNLIN_DATA_DIR",
        "DUNLIN_LOG_FILE",
    ] {
        command.env_remove(key);
    }
}

fn deny_path(
    home: &Path,
    project: &Path,
    allowed: &[PathBuf],
    path: &Path,
    denied: &mut Vec<String>,
) {
    if allowed.iter().any(|root| root == path) {
        return;
    }
    if allowed.iter().any(|root| root.starts_with(path)) || project.starts_with(path) {
        deny_siblings(home, path, allowed, denied);
    } else {
        denied.push(path.to_string_lossy().to_string());
    }
}

/// Deny every sibling of the active project under a protected parent. Denying
/// the parent itself would also deny the project; enumerating only siblings
/// keeps the project usable without granting access to the rest of Documents,
/// Desktop, Downloads, etc. The cap keeps the profile bounded on huge folders.
fn deny_siblings(home: &Path, parent: &Path, allowed: &[PathBuf], denied: &mut Vec<String>) {
    // The profile is built from configured paths. Canonicalize and require the
    // enumerated parent to remain inside the canonical home directory before
    // reading names; this is a sandbox deny-list, never a project file read.
    let Ok(parent) = parent.canonicalize() else {
        return;
    };
    let Ok(home) = home.canonicalize() else {
        return;
    };
    if !parent.starts_with(&home) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return;
    };
    let mut added = 0usize;
    for entry in entries.flatten() {
        if added >= 512 {
            return;
        }
        let path = entry.path();
        if allowed.iter().any(|root| root == &path) {
            continue;
        }
        if allowed.iter().any(|root| root.starts_with(&path)) {
            if path.is_dir() {
                deny_siblings(&home, &path, allowed, denied);
            }
        } else {
            denied.push(path.to_string_lossy().to_string());
            added += 1;
        }
    }
}

fn sandbox_profile(
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    program: &str,
    db_path: &Path,
) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = Path::new(&home);
    let project = Path::new(project_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(project_path));
    let mut allowed = allowed_roots.iter().map(PathBuf::from).collect::<Vec<_>>();
    allowed.push(project.clone());
    allowed.sort();
    allowed.dedup();
    let mut denied = Vec::new();

    if !home.is_empty() {
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
            ".git-credentials",
            ".config/gh",
            "Library/Keychains",
            "Library/Messages",
            "Library/Mail",
            "Library/Safari",
            "Library/Application Support/Google/Chrome",
            "Library/Application Support/Firefox",
            "Library/Application Support/Microsoft Edge",
            "Library/Application Support/BraveSoftware",
            "Library/Application Support/Arc",
            "Library/Application Support/1Password",
            "Library/Application Support/Bitwarden",
            "Library/Application Support/Slack",
            "Library/Application Support/Discord",
            "Library/Application Support/Signal",
            "Library/Application Support/Telegram Desktop",
            "Library/Application Support/Code",
            "Library/Application Support/Cursor",
            "Library/Application Support/gh",
            "Library/Application Support/Dunlin",
            "Library/Application Support/Kestrel",
            "Library/Logs/Dunlin",
            "Library/Logs/Kestrel",
            "Library/Caches/Dunlin",
            "Library/Caches/Kestrel",
            "Library/Application Support/ai.opencode.desktop",
            ".local/share/dunlin",
            ".local/share/kestrel",
            ".config/dunlin",
            ".cache/dunlin",
            "Documents",
            "Desktop",
            "Downloads",
            "Pictures",
            "Movies",
            "Music",
        ];
        if !program.contains("codex") {
            private.push(".codex");
        }
        if !program.contains("claude") {
            private.push(".claude");
        }
        if !program.contains("opencode") {
            private.push(".local/share/opencode");
            private.push(".local/state/opencode");
            private.push(".config/opencode");
        }
        for suffix in private {
            deny_path(
                home_path,
                &project,
                &allowed,
                &home_path.join(suffix),
                &mut denied,
            );
        }
    }
    if !db_path.as_os_str().is_empty() {
        let db_path = if db_path.is_absolute() || home.is_empty() {
            db_path.to_path_buf()
        } else {
            Path::new(&home).join(db_path)
        };
        deny_path(home_path, &project, &allowed, &db_path, &mut denied);
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = db_path.as_os_str().to_os_string();
            sidecar.push(suffix);
            deny_path(
                home_path,
                &project,
                &allowed,
                Path::new(&sidecar),
                &mut denied,
            );
        }
    }
    for path in other_projects {
        let canonical = Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path));
        if canonical != project && !allowed.contains(&canonical) {
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
    use super::{
        Tracker, check_session_scope, command_parts, prompt_title, sandbox_profile,
        still_authorized,
    };
    use crate::api::AppState;
    use crate::config::Config;
    use crate::store::{Project, Store};
    use serde_json::json;

    #[test]
    fn relay_vault_is_injected_as_a_stdio_mcp_server() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("relay.db");
        let vault = crate::vault::RelayVault::start(&db).unwrap();
        let state = AppState::new_with_vault(
            Store::open_in_memory().unwrap(),
            Config::default(),
            Some(vault),
        );
        let project = Project {
            id: "project-1".into(),
            name: "Acme".into(),
            path: temp.path().to_string_lossy().to_string(),
        };
        let mut tracker = Tracker {
            pending: Default::default(),
            allowed_roots: Vec::new(),
            allowed_project_ids: Vec::new(),
            vault_read_only: false,
            vault_sessions: Vec::new(),
        };
        let mut value = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {"cwd": project.path, "mcpServers": []}
        });
        tracker
            .inject_vault(&mut value, &state, &project, &json!(1))
            .unwrap();
        let server = &value.pointer("/params/mcpServers").unwrap()[0];
        assert_eq!(server["name"], "dunlin-relay-secrets");
        assert!(
            server["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "--mcp-vault")
        );
        assert!(
            server["env"]
                .as_array()
                .unwrap()
                .iter()
                .any(|env| env["name"] == "DUNLIN_VAULT_TOKEN")
        );
        assert_eq!(tracker.vault_sessions.len(), 1);
        for session in &tracker.vault_sessions {
            session.revoke();
        }
    }

    #[test]
    fn revoked_devices_and_project_grants_fail_reauthorization() {
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("member", "member").unwrap();
        let project = store
            .create_project("project", "/tmp/dunlin-authorization")
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let (device, _) = store.create_device(&user.id, "device").unwrap();
        let state = AppState::new(store, Config::default());
        assert!(still_authorized(&state, &user, &device, &project));

        state
            .store
            .lock()
            .unwrap()
            .revoke_device(&device.id)
            .unwrap();
        assert!(!still_authorized(&state, &user, &device, &project));
    }

    #[test]
    fn rejects_client_requested_extra_roots_and_mcp_servers() {
        let empty = json!({"params": {"additionalDirectories": [], "mcpServers": []}});
        assert!(check_session_scope(&empty, &[]).is_ok());
        let extra = json!({"params": {"additionalDirectories": ["/etc"]}});
        assert!(check_session_scope(&extra, &[]).is_err());
        let mcp = json!({"params": {"mcpServers": [{"type": "stdio", "name": "x"}]}});
        assert!(check_session_scope(&mcp, &[]).is_err());
        let malformed = json!({"params": {"mcpServers": {}}});
        assert!(check_session_scope(&malformed, &[]).is_err());
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let allowed = temp.join("dunlin-root-allowed");
        std::fs::create_dir_all(&allowed).unwrap();
        let allowed = allowed
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let accepted =
            json!({"params": {"additionalDirectories": [allowed.clone()], "mcpServers": []}});
        assert!(check_session_scope(&accepted, std::slice::from_ref(&allowed)).is_ok());
        let outside = json!({"params": {"additionalDirectories": [temp.to_string_lossy()]}});
        assert!(check_session_scope(&outside, &[allowed]).is_err());
    }

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
            &[],
            &["/Users/me/other".to_string()],
            "opencode",
            std::path::Path::new("/Users/me/Library/Application Support/Dunlin/relay.db"),
        );
        assert!(profile.starts_with("(version 1)\n(allow default)"));
        assert!(profile.contains("/Users/me/other"));
        assert!(profile.contains(".ssh"));
        assert!(profile.contains(".codex"));
        assert!(profile.contains("Library/Application Support/Dunlin/relay.db"));
        assert!(profile.contains("Library/Application Support/Dunlin/relay.db-wal"));
        assert!(!profile.contains("(subpath \"/Users/me/project\")"));
    }

    #[test]
    fn sandbox_does_not_deny_the_active_project_under_documents() {
        let profile = sandbox_profile(
            "/Users/me/Documents/project",
            &["/Users/me/Desktop/peer".to_string()],
            &[],
            "opencode",
            std::path::Path::new("/tmp/relay.db"),
        );
        assert!(!profile.contains("(subpath \"/Users/me/Documents\")"));
        assert!(!profile.contains("(subpath \"/Users/me/Documents/project\")"));
        assert!(!profile.contains("(subpath \"/Users/me/Desktop/peer\")"));
    }
}
