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
use crate::store::local_metadata::MetadataPermission;
use crate::store::native_checkout::{NativeCheckoutAuthority, SandboxMetadataRoots};
use crate::store::{Device, NativeGrantStamp, Project, SessionOwner, User};
use dunlin_domain::{PermissionMode, ProjectSettings, validate_project_environment};
mod coordination;
#[cfg(test)]
mod coordination_tests;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod native_grant_tests;

const SESSION_NOT_FOUND: i64 = -32001;
const SESSION_BUSY: i64 = -32002;
const INVALID_PARAMS: i64 = -32602;
const MAX_PENDING: usize = 256;

// Revoke the entire harness process group on disconnect, revocation or future cancellation.
struct ProcessGroup(Option<u32>);
impl ProcessGroup {
    /// Signal while the leader is still owned and unreaped, then disarm before
    /// any Child::kill().await can reap it and allow its PID to be reused.
    fn terminate(&mut self) {
        if let Some(pid) = self.0.take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = pid;
        }
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn bridge(
    socket: WebSocket,
    state: AppState,
    user: User,
    device: Device,
    project: Project,
    allowed_roots: Vec<Project>,
    native_grants: Vec<NativeGrantStamp>,
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

    if native_grants.len() != allowed_roots.len() + 1
        || !native_grants
            .iter()
            .any(|g| g.project_id == project.id && g.user_id == user.id)
    {
        return;
    }
    let mut tracker = Tracker {
        native_grants,
        native_projects: std::iter::once(project.clone())
            .chain(allowed_roots.iter().cloned())
            .collect(),
        ..Default::default()
    };
    let Some(checkouts) = capture_native_checkouts(&state, &user, &tracker.native_projects).await
    else {
        return;
    };
    tracker.native_checkouts = checkouts;
    if !tracker.connection_authorized(&state, &user, &device, &project) {
        return;
    }
    let mut authority_changes = match state.store.lock() {
        Ok(store) => store.subscribe_catalog(),
        Err(_) => return,
    };
    let selected_harness = crate::management::harness(&state.config);
    let (program, args) = command_parts(&selected_harness);
    let Some(primary_checkout) = tracker
        .native_checkouts
        .iter()
        .find(|a| a.stamp().native_grant.project_id == project.id)
    else {
        return;
    };
    let current_path = primary_checkout.root().to_owned();
    let extra_paths = tracker
        .native_checkouts
        .iter()
        .filter(|a| a.stamp().native_grant.project_id != project.id)
        .map(|a| a.root().to_owned())
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
    let mut broker_paths = Vec::new();
    if let Some(service) = &state.coordination {
        broker_paths.push(service.socket().to_path_buf());
    }
    if let Some(vault) = &state.vault {
        broker_paths.push(vault.socket_path().to_path_buf());
    }
    if !broker_paths.is_empty()
        && let Ok(executable) = std::env::current_exe()
    {
        broker_paths.push(executable);
    }
    let mut metadata_roots = SandboxMetadataRoots::default();
    for checkout in &tracker.native_checkouts {
        if metadata_roots.extend(&checkout.metadata_roots()).is_err() {
            return;
        }
    }
    let mut harness = harness_command_with_brokers(
        &state.config,
        &program,
        &args,
        &current_path,
        &extra_paths,
        &other_projects,
        &project_settings,
        &user.id,
        &broker_paths,
        &metadata_roots,
    );
    harness.env("PATH", crate::management::path(&state.config));
    let stderr = if std::env::var_os("DUNLIN_RELAY_DEBUG").is_some() {
        Stdio::inherit()
    } else {
        Stdio::null()
    };
    // Recheck after checkout/config preparation and before starting a process.
    if !checkout_filesystems_current(&state, &tracker.native_checkouts).await {
        return;
    }
    if !tracker.connection_authorized(&state, &user, &device, &project) {
        return;
    }
    #[cfg(unix)]
    harness.process_group(0);
    let mut child = match harness
        .current_dir(&current_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let message = format!("Could not start harness `{}`: {error}", selected_harness);
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

    let mut process_group = ProcessGroup(Some(child.id().expect("spawned harness")));
    let Some(mut stdin) = child.stdin.take() else {
        process_group.terminate();
        let _ = child.kill().await;
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        process_group.terminate();
        let _ = child.kill().await;
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    tracker = Tracker {
        native_grants: tracker.native_grants,
        native_projects: tracker.native_projects,
        native_checkouts: tracker.native_checkouts,
        pending: HashMap::new(),
        allowed_roots: allowed_paths,
        allowed_project_ids: allowed_roots
            .iter()
            .map(|project| project.id.clone())
            .collect(),
        vault_read_only: project_settings.permission_mode == Some(PermissionMode::ReadOnly),
        vault_sessions: Vec::new(),
        ..Default::default()
    };
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    let mut guardian = tokio::time::interval(Duration::from_secs(1));
    let mut last_activity = Instant::now();
    let mut client_gone = false;
    let mut taken_over = false;
    let mut harness_exited = false;

    loop {
        tokio::select! {
            _ = guardian.tick() => {
                if !tracker.connection_authorized(&state,&user,&device,&project)
                    || !checkout_filesystems_current(&state,&tracker.native_checkouts).await
                    || !tracker.connection_authorized(&state,&user,&device,&project) {break;}
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if !tracker.connection_authorized(&state,&user,&device,&project) {break;}
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
                                if !tracker.connection_authorized(&state,&user,&device,&project) {break;}
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
                                 tracker.confirm_coordination(&state, &value);
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
            _ = authority_changes.recv() => {
                if !tracker.connection_authorized(&state,&user,&device,&project) {break;}
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
                if !tracker.connection_authorized(&state, &user, &device, &project) {
                    eprintln!("[relay] authorization revoked · {connection_id}");
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

    if client_gone
        && !taken_over
        && !harness_exited
        && tracker.has_active_prompt()
        && still_authorized(&state, &user, &device, &project)
        && tracker.connection_authorized(&state, &user, &device, &project)
    {
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
            process_group,
        ));
        return;
    }

    for session in &tracker.vault_sessions {
        session.revoke();
    }
    tracker.stop_coordination(&state);
    process_group.terminate();
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
    mut process_group: ProcessGroup,
) {
    let (dummy_tx, _dummy_rx) = mpsc::channel::<()>(1);
    let deadline = tokio::time::Instant::now() + timeout;
    // These state-free pins are the same ones admitted before spawn. The
    // observer leaving must never reconstruct authority or refresh epochs.
    let guard_tracker = Tracker {
        native_grants: tracker.native_grants.clone(),
        native_projects: tracker.native_projects.clone(),
        native_checkouts: tracker.native_checkouts.clone(),
        allowed_project_ids: tracker.allowed_project_ids.clone(),
        allowed_roots: tracker.allowed_roots.clone(),
        ..Default::default()
    };
    let mut authority_changes = match state.store.lock() {
        Ok(store) => store.subscribe_catalog(),
        Err(_) => return,
    };
    // Keep both guards outside the I/O future. A peer that stops consuming a
    // cancellation response cannot suspend revocation, FS checks or deadlines.
    let authority_guard = async {
        let mut guardian = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = guardian.tick() => {},
                _ = authority_changes.recv() => {},
            }
            if !guard_tracker.connection_authorized(&state, &user, &device, &project) {
                break;
            }
        }
    };
    let filesystem_guard = async {
        let mut guardian = tokio::time::interval(Duration::from_secs(1));
        loop {
            guardian.tick().await;
            if !checkout_filesystems_current(&state, &guard_tracker.native_checkouts).await
                || !guard_tracker.connection_authorized(&state, &user, &device, &project)
            {
                break;
            }
        }
    };
    let exchange = async {
        loop {
            let Ok(Some(line)) = lines.next_line().await else {
                break;
            };
            if !tracker.connection_authorized(&state, &user, &device, &project)
                || line.len() > state.config.max_frame_bytes
            {
                break;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("method").and_then(Value::as_str) == Some("session/request_permission")
                && let Some(id) = value.get("id")
            {
                let response = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "outcome": { "outcome": "cancelled" } }
                });
                let mut bytes = response.to_string().into_bytes();
                bytes.push(b'\n');
                // A cancelled/partial write is terminal: the stream is never
                // reused and the process namespace is killed before reaping.
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(1), async {
                        stdin.write_all(&bytes).await?;
                        stdin.flush().await
                    })
                    .await,
                    Ok(Ok(()))
                ) {
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
    };
    tokio::select! {
        _ = authority_guard => {},
        _ = filesystem_guard => {},
        _ = exchange => {},
        _ = close_rx.recv() => {},
        _ = tokio::time::sleep_until(deadline) => {},
    }
    for session in &tracker.vault_sessions {
        session.revoke();
    }
    tracker.stop_coordination(&state);
    process_group.terminate();
    let _ = child.kill().await;
    if let Ok(mut leases) = state.leases.lock() {
        leases.retain(|_, lease| lease.connection_id != connection_id);
    }
    eprintln!("[relay] lingering turn finished · {connection_id}");
}

async fn capture_native_checkouts(
    state: &AppState,
    user: &User,
    projects: &[Project],
) -> Option<Vec<NativeCheckoutAuthority>> {
    let permit = state.fs_permit().ok()?;
    let candidates = {
        let store = state.store.lock().ok()?;
        projects
            .iter()
            .map(|p| store.native_checkout_candidate(&p.id, &user.id, MetadataPermission::Write))
            .collect::<crate::store::Result<Vec<_>>>()
            .ok()?
    };
    // The permit lives through the actual blocking work, including timeout.
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        candidates
            .into_iter()
            .map(|c| c.validate_filesystem())
            .collect::<crate::store::Result<Vec<_>>>()
    });
    let validated = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .ok()?
        .ok()?
        .ok()?;
    let store = state.store.lock().ok()?;
    validated
        .into_iter()
        .map(|v| store.authorize_native_checkout(v))
        .collect::<crate::store::Result<Vec<_>>>()
        .ok()
}

async fn checkout_filesystems_current(
    state: &AppState,
    checkouts: &[NativeCheckoutAuthority],
) -> bool {
    let Ok(permit) = state.fs_permit() else {
        return false;
    };
    let checkouts = checkouts.to_vec();
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        checkouts.iter().all(|a| a.validate_filesystem().is_ok())
    });
    matches!(
        tokio::time::timeout(Duration::from_secs(5), job).await,
        Ok(Ok(true))
    )
}

#[derive(Default)]
struct Tracker {
    pending: HashMap<String, Pending>,
    /// Canonical project roots explicitly granted for this connection. An ACP
    /// session may request a subset, never an arbitrary host path.
    allowed_roots: Vec<String>,
    allowed_project_ids: Vec<String>,
    native_grants: Vec<NativeGrantStamp>,
    native_projects: Vec<Project>,
    native_checkouts: Vec<NativeCheckoutAuthority>,
    vault_read_only: bool,
    vault_sessions: Vec<crate::vault::VaultSession>,
    coordination_sessions: HashMap<String, dunlin_coordination::service::SessionHandle>,
    coordination_bindings: HashMap<String, dunlin_coordination::service::SessionHandle>,
    coordination_runs: HashMap<String, String>,
    coordination_models: HashMap<String, (String, String)>,
    coordination_outputs: HashMap<String, String>,
}

enum Pending {
    New,
    Load { session_id: String },
    Prompt,
    Fork { source_id: String, title: String },
    Delete { session_id: String },
}

impl Tracker {
    fn authority_guard(
        &self,
        state: &AppState,
        user: &User,
        device: &Device,
        project: &Project,
    ) -> std::sync::Arc<dyn Fn() -> bool + Send + Sync> {
        let store = state.store.clone();
        let config = state.config.clone();
        let user_id = user.id.clone();
        let device_id = device.id.clone();
        let grants = if self.native_grants.is_empty() {
            store
                .lock()
                .ok()
                .and_then(|db| db.capture_native_grant(&project.id, &user.id).ok())
                .map(|g| vec![g])
        } else {
            Some(self.native_grants.clone())
        };
        let projects = if self.native_projects.is_empty() {
            vec![project.clone()]
        } else {
            self.native_projects.clone()
        };
        let checkouts = if self.native_checkouts.is_empty() {
            projects
                .iter()
                .map(|p| {
                    let candidate = {
                        store
                            .lock()
                            .ok()?
                            .native_checkout_candidate(&p.id, &user_id, MetadataPermission::Write)
                            .ok()?
                    };
                    let validated = candidate.validate_filesystem().ok()?;
                    store.lock().ok()?.authorize_native_checkout(validated).ok()
                })
                .collect::<Option<Vec<_>>>()
        } else {
            Some(self.native_checkouts.clone())
        };
        std::sync::Arc::new(move || {
            let fresh = || {
                let host_accounts = crate::management::host_accounts(&config);
                let Ok(db) = store.lock() else {
                    return false;
                };
                let (Some(grants), Some(checkouts)) = (grants.as_ref(), checkouts.as_ref()) else {
                    return false;
                };
                db.native_checkout_transaction::<_, crate::store::StoreError>(|db| {
                    let Some(active) = db
                        .active_device_user(&device_id)?
                        .filter(|u| u.id == user_id)
                    else {
                        return Ok(false);
                    };
                    Ok((active.role == "admin" || !host_accounts)
                        && db.oidc_device_allowed(&device_id, config.oidc.as_ref())?
                        && checkouts
                            .iter()
                            .all(|a| db.revalidate_native_checkout_in_transaction(a).is_ok())
                        && grants
                            .iter()
                            .all(|g| g.user_id == user_id && db.revalidate_native_grant(g).is_ok())
                        && projects.iter().all(|p| {
                            db.project_for_user(&p.id, &user_id)
                                .ok()
                                .flatten()
                                .is_some_and(|v| v.path == p.path)
                        }))
                })
                .unwrap_or(false)
            };
            fresh()
                && checkouts
                    .as_ref()
                    .is_some_and(|items| items.iter().all(|a| a.validate_filesystem().is_ok()))
                && fresh()
        })
    }

    fn roots_authorized_in_store(&self, store: &crate::store::Store, user: &User) -> bool {
        if self
            .native_grants
            .iter()
            .any(|g| g.user_id != user.id || store.revalidate_native_grant(g).is_err())
            || self.native_projects.iter().any(|p| {
                !store
                    .project_for_user(&p.id, &user.id)
                    .ok()
                    .flatten()
                    .is_some_and(|current| current.path == p.path)
            })
            || self.native_checkouts.len() != self.native_projects.len()
            || self
                .native_checkouts
                .iter()
                .any(|a| store.revalidate_native_checkout_in_transaction(a).is_err())
        {
            return false;
        }
        self.allowed_project_ids.iter().all(|id| {
            self.native_checkouts
                .iter()
                .find(|a| a.stamp().native_grant.project_id == *id)
                .is_some_and(|a| self.allowed_roots.iter().any(|root| root == a.root()))
        })
    }
    #[cfg(test)]
    fn roots_authorized(&self, state: &AppState, user: &User) -> bool {
        let Ok(store) = state.store.lock() else {
            return false;
        };
        store
            .native_checkout_transaction::<_, crate::store::StoreError>(|db| {
                Ok(self.roots_authorized_in_store(db, user))
            })
            .unwrap_or(false)
    }
    fn connection_authorized(
        &self,
        state: &AppState,
        user: &User,
        device: &Device,
        project: &Project,
    ) -> bool {
        let host_accounts = crate::management::host_accounts(&state.config);
        let Ok(store) = state.store.lock() else {
            return false;
        };
        store
            .native_checkout_transaction::<_, crate::store::StoreError>(|db| {
                let Some(current) = db
                    .active_device_user(&device.id)?
                    .filter(|u| u.id == user.id)
                else {
                    return Ok(false);
                };
                Ok((current.role == "admin" || !host_accounts)
                    && db.oidc_device_allowed(&device.id, state.config.oidc.as_ref())?
                    && db
                        .project_for_user(&project.id, &current.id)?
                        .is_some_and(|p| p.path == project.path)
                    && self.roots_authorized_in_store(db, &current))
            })
            .unwrap_or(false)
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
        if !self.connection_authorized(state, user, device, project) {
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
                self.inject_vault(&mut forwarded, state, project, user, device, &id)?;
                self.inject_coordination(&mut forwarded, state, user, device, project, &id)?;
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
                self.inject_vault(&mut forwarded, state, project, user, device, &id)?;
                self.inject_coordination(&mut forwarded, state, user, device, project, &id)?;
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
                self.inject_vault(&mut forwarded, state, project, user, device, &id)?;
                self.inject_coordination(&mut forwarded, state, user, device, project, &id)?;
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
                self.start_coordination(
                    &mut forwarded,
                    state,
                    user,
                    device,
                    project,
                    &id,
                    &session_id,
                    connection_id,
                )?;
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
        user: &User,
        device: &Device,
        id: &Value,
    ) -> Result<(), Value> {
        let Some(vault) = &state.vault else {
            return Ok(());
        };
        let session = vault.new_guarded_session(
            project.id.clone(),
            project.path.clone(),
            self.vault_read_only,
            self.authority_guard(state, user, device, project),
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
        self.observe_coordination(state, user, project, value);
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn harness_command(
    config: &crate::config::Config,
    program: &str,
    args: &[String],
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    settings: &ProjectSettings,
    user_id: &str,
) -> Command {
    harness_command_with_brokers(
        config,
        program,
        args,
        project_path,
        allowed_roots,
        other_projects,
        settings,
        user_id,
        &[],
        &SandboxMetadataRoots::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn harness_command_with_metadata(
    config: &crate::config::Config,
    program: &str,
    args: &[String],
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    settings: &ProjectSettings,
    user_id: &str,
    metadata_roots: &SandboxMetadataRoots,
) -> Command {
    harness_command_with_brokers(
        config,
        program,
        args,
        project_path,
        allowed_roots,
        other_projects,
        settings,
        user_id,
        &[],
        metadata_roots,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn harness_command_with_brokers(
    config: &crate::config::Config,
    program: &str,
    args: &[String],
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    settings: &ProjectSettings,
    user_id: &str,
    broker_paths: &[PathBuf],
    metadata_roots: &SandboxMetadataRoots,
) -> Command {
    if !metadata_roots_safe(config, metadata_roots)
        || metadata_roots.iter().any(|(root, _)| {
            other_projects.iter().any(|p| {
                let p = Path::new(p)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(p));
                p.starts_with(root)
            })
        })
    {
        return Command::new("/nonexistent/dunlin-metadata-sandbox-scope-denied");
    }
    let isolated = crate::management::isolated_agents(config);
    let home = crate::management::agent_home(config, user_id);
    if isolated && crate::management::private_dir(&home).is_err() {
        return Command::new("/nonexistent/dunlin-agent-home-unavailable");
    }
    let mut command = if config.sandbox
        && cfg!(target_os = "macos")
        && Path::new("/usr/bin/sandbox-exec").is_file()
    {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        let profile = macos_harness_profile(
            config,
            program,
            project_path,
            allowed_roots,
            other_projects,
            &home,
            broker_paths,
            metadata_roots,
            isolated,
        );
        if std::env::var_os("DUNLIN_RELAY_DEBUG").is_some() {
            eprintln!("[relay] sandbox profile:\n{profile}");
        }
        command.arg("-p").arg(profile).arg(program).args(args);
        command
    } else if cfg!(target_os = "macos") && config.sandbox {
        Command::new("/nonexistent/dunlin-sandbox-unavailable")
    } else if cfg!(target_os = "linux") && config.sandbox {
        linux_harness_command(
            config,
            program,
            args,
            project_path,
            allowed_roots,
            &home,
            broker_paths,
            metadata_roots,
        )
    } else {
        let mut command = Command::new(program);
        command.args(args);
        command
    };
    scrub_harness_environment(&mut command);
    if isolated {
        for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY"] {
            command.env_remove(key);
        }
    }
    if isolated {
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local/share"));
    }
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

// Keep metadata roots distinct from workspace and broker paths.
#[allow(clippy::too_many_arguments)]
fn linux_harness_command(
    config: &crate::config::Config,
    program: &str,
    args: &[String],
    project: &str,
    roots: &[String],
    home: &Path,
    broker_paths: &[PathBuf],
    metadata_roots: &SandboxMetadataRoots,
) -> Command {
    // Missing bubblewrap fails at spawn, never by silently running unsandboxed.
    let mut command = Command::new(
        crate::management::executable(config, "bwrap").unwrap_or_else(|| "/usr/bin/bwrap".into()),
    );
    command.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--share-net",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
    ]);
    for directory in ["/usr", "/bin", "/lib", "/lib64"] {
        if Path::new(directory).exists() {
            command.args(["--ro-bind", directory, directory]);
        }
    }
    for file in [
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/passwd",
        "/etc/group",
        "/etc/ssl",
        "/etc/pki",
    ] {
        if Path::new(file).exists() {
            command.args(["--ro-bind", file, file]);
        }
    }
    command.arg("--bind").arg(home).arg("/home/dunlin").args([
        "--setenv",
        "HOME",
        "/home/dunlin",
        "--setenv",
        "XDG_CONFIG_HOME",
        "/home/dunlin/.config",
        "--setenv",
        "XDG_CACHE_HOME",
        "/home/dunlin/.cache",
        "--setenv",
        "XDG_DATA_HOME",
        "/home/dunlin/.local/share",
    ]);
    let tools = crate::management::tools(config);
    if tools.is_dir() {
        command.arg("--ro-bind").arg(&tools).arg(&tools);
    }
    let node = crate::management::root(config).join("node");
    if node.is_dir() {
        command.arg("--ro-bind").arg(&node).arg(&node);
    }
    command.args(["--bind", project, project]);
    for root in roots {
        command.args(["--bind", root, root]);
    }
    for (root, permission) in metadata_roots.iter() {
        let binding = if permission == MetadataPermission::Write {
            "--bind"
        } else {
            "--ro-bind"
        };
        command.args([binding, root, root]);
    }
    // /tmp is private inside bubblewrap. Bind only exact broker socket inodes
    // and the proxy executable, never the host temp directory or ledger.
    for path in broker_paths {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command
        .args(["--chdir", project, "--"])
        .arg(program)
        .args(args);
    command
}

/// Complete production seatbelt profile, also exported verbatim by the focused
/// Linux generator for an actual macOS-kernel probe without a host Rust build.
#[allow(clippy::too_many_arguments)]
fn macos_harness_profile(
    config: &crate::config::Config,
    program: &str,
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    home: &Path,
    broker_paths: &[PathBuf],
    metadata_roots: &SandboxMetadataRoots,
    isolated: bool,
) -> String {
    let mut runtime_roots = allowed_roots.to_vec();
    runtime_roots.push(project_path.to_owned());
    runtime_roots.extend(
        broker_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned()),
    );
    let tools = crate::management::tools(config);
    let tools = tools.canonicalize().unwrap_or(tools);
    let node = crate::management::root(config).join("node");
    let node = node.canonicalize().unwrap_or(node);
    for path in [home, tools.as_path(), node.as_path()] {
        runtime_roots.push(path.to_string_lossy().to_string());
    }
    let mut profile = sandbox_profile_with_metadata(
        project_path,
        &runtime_roots,
        other_projects,
        program,
        &config.db,
        metadata_roots,
    );
    profile.push_str(&server_sandbox_policy(
        &crate::management::root(config),
        &runtime_roots,
    ));
    if isolated && let Some(host_home) = std::env::var_os("HOME") {
        for suffix in [
            ".codex",
            ".claude",
            ".local/share/opencode",
            ".local/state/opencode",
            ".config/opencode",
        ] {
            let path = PathBuf::from(&host_home).join(suffix);
            profile.push_str(&format!(
                "\n(deny file-read* file-write* (subpath \"{}\"))\n",
                escape_sandbox_path(&path.to_string_lossy())
            ));
        }
    }
    profile.push_str(&format!(
        "\n(deny file-write* (subpath \"{}\") (subpath \"{}\"))\n",
        escape_sandbox_path(&tools.to_string_lossy()),
        escape_sandbox_path(&node.to_string_lossy())
    ));
    profile
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
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_ACCESS_TOKEN",
        "OAUTH_TOKEN",
        "DBUS_SESSION_BUS_ADDRESS",
        "SSH_AUTH_SOCK",
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

#[cfg(test)]
fn sandbox_profile(
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    program: &str,
    db_path: &Path,
) -> String {
    sandbox_profile_with_metadata(
        project_path,
        allowed_roots,
        other_projects,
        program,
        db_path,
        &SandboxMetadataRoots::default(),
    )
}

fn metadata_roots_safe(config: &crate::config::Config, roots: &SandboxMetadataRoots) -> bool {
    if roots.is_empty() {
        return true;
    }
    if !config.sandbox {
        return false;
    }
    let mut protected = vec![
        crate::management::root(config),
        config.db.clone(),
        config.db.with_extension("coordination"),
    ];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = config.db.as_os_str().to_os_string();
        sidecar.push(suffix);
        protected.push(PathBuf::from(sidecar));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for suffix in [
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
            ".codex",
            ".claude",
            ".local/share/opencode",
            ".local/state/opencode",
            ".config/opencode",
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
        ] {
            protected.push(home.join(suffix));
        }
    }
    let protected = protected
        .into_iter()
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect::<Vec<_>>();
    roots.iter().all(|(root, _)| {
        !root.chars().any(char::is_control)
            && Path::new(root).is_absolute()
            && !protected.iter().any(|p| {
                !p.as_os_str().is_empty() && (Path::new(root).starts_with(p) || p.starts_with(root))
            })
    })
}

fn sandbox_profile_with_metadata(
    project_path: &str,
    allowed_roots: &[String],
    other_projects: &[String],
    program: &str,
    db_path: &Path,
    metadata_roots: &SandboxMetadataRoots,
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
            private.extend([
                ".local/share/opencode",
                ".local/state/opencode",
                ".config/opencode",
            ]);
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
        deny_path(
            home_path,
            &project,
            &allowed,
            &db_path.with_extension("coordination"),
            &mut denied,
        );
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
            denied.push(canonical.to_string_lossy().into_owned());
        }
    }

    let mut profile = String::from("(version 1)\n(allow default)\n");
    for operation in ["file-read*", "file-write*"] {
        profile.push_str(&format!("(deny {operation}"));
        for path in &denied {
            if metadata_roots.is_empty() {
                profile.push_str(&format!(" (subpath \"{}\")", escape_sandbox_path(path)));
            } else {
                profile.push_str(&format!(
                    " (require-all (subpath \"{}\")",
                    escape_sandbox_path(path)
                ));
                for (root, permission) in metadata_roots.iter() {
                    if operation == "file-read*" || permission == MetadataPermission::Write {
                        profile.push_str(&format!(
                            " (require-not (subpath \"{}\"))",
                            escape_sandbox_path(root)
                        ));
                    }
                }
                profile.push(')');
            }
        }
        profile.push_str(")\n");
    }
    for (root, permission) in metadata_roots.iter() {
        if permission == MetadataPermission::Read {
            profile.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n",
                escape_sandbox_path(root)
            ));
        }
    }
    profile
}

fn escape_sandbox_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

fn server_sandbox_policy(root: &Path, allowed: &[String]) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
    let mut policy = format!(
        "\n(deny file-read* file-write* (require-all (subpath \"{}\")",
        escape_sandbox_path(&root.to_string_lossy())
    );
    for path in allowed {
        let path = Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path));
        if path != root && path.starts_with(&root) {
            policy.push_str(&format!(
                " (require-not (subpath \"{}\"))",
                escape_sandbox_path(&path.to_string_lossy())
            ));
        }
    }
    policy.push_str("))\n(deny file-link)\n");
    policy
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

    #[cfg(target_os = "macos")]
    #[test]
    fn server_namespace_policy_denies_future_siblings_not_only_existing_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("server");
        let project = root.join("projects/allowed");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("file"), "allowed").unwrap();
        let policy = format!(
            "(version 1)\n(allow default)\n{}",
            super::server_sandbox_policy(&root, &[project.to_string_lossy().to_string()])
        );
        let future = root.join("projects/.transfer-created-later");
        std::fs::create_dir_all(&future).unwrap();
        std::fs::write(future.join("secret"), "never read").unwrap();
        let read = |path: &std::path::Path| {
            std::process::Command::new("/usr/bin/sandbox-exec")
                .args(["-p", &policy, "/bin/cat"])
                .arg(path)
                .output()
                .unwrap()
        };
        let allowed = read(&project.join("file"));
        assert!(
            allowed.status.success(),
            "{}",
            String::from_utf8_lossy(&allowed.stderr)
        );
        assert!(!read(&future.join("secret")).status.success());
    }

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
            ..Default::default()
        };
        let mut value = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {"cwd": project.path, "mcpServers": []}
        });
        tracker
            .inject_vault(
                &mut value,
                &state,
                &project,
                &crate::store::User {
                    id: "test-user".into(),
                    name: "test".into(),
                    role: "admin".into(),
                },
                &crate::store::Device {
                    id: "test-device".into(),
                    user_id: "test-user".into(),
                    name: "test".into(),
                    created_at: 0,
                    last_seen_at: None,
                    revoked_at: None,
                },
                &json!(1),
            )
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
