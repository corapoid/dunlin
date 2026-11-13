//! Authenticated relay PTY over WebSocket.
//!
//! Binary frames carry raw PTY bytes in both directions. Text frames are small
//! JSON control messages (`ready`, `exit`, `resize`, `close`, `ping`). The shell
//! runs on the relay host inside a project the authenticated user was granted;
//! there is no local fallback on the desktop. Each input frame has a two-second
//! PTY write budget; a backpressured frame terminates the terminal rather than
//! retaining its process and permit after a disconnected client.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use dunlin_terminal::{Exit, Session, SpawnOptions};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::OwnedSemaphorePermit;

use crate::api::{ApiError, AppState, AuthUser};

const MAX_INPUT_FRAME: usize = 2 * 1024 * 1024;
const INPUT_WRITE_BUDGET: Duration = Duration::from_secs(2);
// A reaped shell may leave a slave FD inherited by another process. Drain its
// final output before exit, but never retain the relay indefinitely for that FD.
const OUTPUT_DRAIN_BUDGET: Duration = Duration::from_secs(2);
const HEARTBEAT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct TerminalQuery {
    #[serde(default)]
    pub project: String,
    pub setup: Option<String>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientControl {
    Resize { rows: u16, cols: u16 },
    Close,
    Ping,
}

#[derive(Clone)]
struct Authority {
    user: String,
    device: String,
    // No AppState/Store reference: the immutable checkout proof cannot form a
    // cycle with the terminal owner or retain refreshed grants implicitly.
    project: Option<(
        crate::store::Project,
        crate::store::native_checkout::NativeCheckoutAuthority,
    )>,
}
impl Authority {
    async fn capture(
        state: &AppState,
        auth: &AuthUser,
        project: Option<&str>,
    ) -> Result<Self, ApiError> {
        let captured = Self {
            user: auth.user.id.clone(),
            device: auth.device.id.clone(),
            project: None,
        };
        if project.is_none() {
            if !captured.check_sql(state) {
                return Err(ApiError::forbidden());
            }
            return Ok(captured);
        }
        let permit = state.fs_permit()?;
        let state = state.clone();
        let project = project.unwrap().to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let (mut current, candidate) = {
                let store = state
                    .store
                    .lock()
                    .map_err(|_| ApiError::internal("Store unavailable"))?;
                crate::management::authority::current_user_locked(
                    &state,
                    &store,
                    &captured.user,
                    &captured.device,
                )?;
                let current = store
                    .project_for_user(&project, &captured.user)?
                    .ok_or_else(ApiError::forbidden)?;
                let candidate = store
                    .native_checkout_candidate(
                        &project,
                        &captured.user,
                        crate::store::local_metadata::MetadataPermission::Write,
                    )
                    .map_err(|_| ApiError::forbidden())?;
                (current, candidate)
            };
            let validated = candidate
                .validate_filesystem()
                .map_err(|_| ApiError::forbidden())?;
            let checkout = state
                .store
                .lock()
                .map_err(|_| ApiError::internal("Store unavailable"))?
                .authorize_native_checkout(validated)
                .map_err(|_| ApiError::forbidden())?;
            current.path = checkout.root().into();
            let captured = Self {
                project: Some((current, checkout)),
                ..captured
            };
            if !captured.check_blocking(&state) {
                return Err(ApiError::forbidden());
            }
            Ok(captured)
        })
        .await
        .map_err(|_| ApiError::internal("Terminal authority worker stopped"))?
    }
    fn allowed_in_transaction(&self, store: &crate::store::Store, state: &AppState) -> bool {
        let Ok(user) = crate::management::authority::current_user_locked(
            state,
            store,
            &self.user,
            &self.device,
        ) else {
            return false;
        };
        if self.project.is_none() && user.role != "admin" {
            return false;
        }
        self.project.as_ref().is_none_or(|(project, checkout)| {
            checkout.root() == project.path
                && checkout.stamp().native_grant.user_id == self.user
                && checkout.stamp().native_grant.project_id == project.id
                && checkout.stamp().permission
                    == crate::store::local_metadata::MetadataPermission::Write
                && store
                    .revalidate_native_checkout_in_transaction(checkout)
                    .is_ok()
        })
    }
    fn check_sql(&self, state: &AppState) -> bool {
        state.store.lock().is_ok_and(|store| {
            store
                .native_checkout_transaction(|store| {
                    if self.allowed_in_transaction(store, state) {
                        Ok(())
                    } else {
                        Err(ApiError::forbidden())
                    }
                })
                .is_ok()
        })
    }
    // Called only from workers already holding a bounded I/O slot.
    fn check_blocking(&self, state: &AppState) -> bool {
        self.check_sql(state)
            && self
                .project
                .as_ref()
                .is_none_or(|(_, checkout)| checkout.validate_filesystem().is_ok())
            && self.check_sql(state)
    }
    async fn check(&self, state: &AppState) -> bool {
        if !self.check_sql(state) {
            return false;
        }
        if self.project.is_none() {
            return true;
        }
        let Ok(permit) = state.fs_permit() else {
            return false;
        };
        let state = state.clone();
        let authority = self.clone();
        // A stalled filesystem never stalls the independent process guardian.
        // The actual worker retains its I/O slot after this deadline expires.
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                authority.check_blocking(&state)
            }),
        )
        .await
        .is_ok_and(|result| result.is_ok_and(|allowed| allowed))
    }
}
struct Guardian(tokio::task::JoinHandle<()>);
impl Drop for Guardian {
    fn drop(&mut self) {
        self.0.abort();
    }
}
// The bridge owns termination independently of Arc-held I/O workers.
struct RelayOwner(Arc<RelaySession>);
impl Drop for RelayOwner {
    fn drop(&mut self) {
        self.0.hard_terminate();
    }
}
struct RelaySession(Session);
impl std::ops::Deref for RelaySession {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0
    }
}
impl Drop for RelaySession {
    fn drop(&mut self) {
        self.0.hard_terminate();
    }
}
fn revoked() -> Message {
    Message::Close(Some(CloseFrame {
        code: 1008,
        reason: "Terminal authorization changed".into(),
    }))
}

pub async fn ws(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let authority = Authority::capture(
        &state,
        &auth,
        query.setup.is_none().then_some(query.project.as_str()),
    )
    .await?;
    let (project_id, mut options) = if let Some(agent) = &query.setup {
        crate::management::admin(&auth)?;
        let (program, args): (&str, Vec<String>) = match agent.as_str() {
            "opencode" => ("opencode", vec!["auth".into(), "login".into()]),
            "codex" => ("codex", vec!["login".into(), "--device-auth".into()]),
            "claude" => ("claude", vec!["auth".into(), "login".into()]),
            _ => return Err(ApiError::bad_request("Unknown agent setup target")),
        };
        let program = crate::management::executable(&state.config, program)
            .ok_or_else(|| ApiError::conflict("Install this agent on the server first"))?;
        let home = crate::management::agent_home(&state.config, &auth.user.id);
        if crate::management::isolated_agents(&state.config) {
            crate::management::private_dir(&home).map_err(ApiError::internal)?;
        }
        let command = crate::acp::harness_command(
            &state.config,
            program
                .to_str()
                .ok_or_else(|| ApiError::bad_request("Invalid agent path"))?,
            &args,
            home.to_str()
                .ok_or_else(|| ApiError::bad_request("Invalid agent home"))?,
            &[],
            &[],
            &dunlin_domain::ProjectSettings::default(),
            &auth.user.id,
        );
        let command = command.as_std();
        let mut options = SpawnOptions::command(
            home,
            command.get_program().to_string_lossy().to_string(),
            command
                .get_args()
                .map(|a| a.to_string_lossy().to_string())
                .collect(),
        );
        options.env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value
                        .map(|v| v.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            })
            .collect();
        // PTYs intentionally reject caller-supplied PATH. This fixed server argv
        // supplies its own managed path without weakening that trust boundary.
        options.args.insert(0, options.program.clone());
        options.args.insert(
            0,
            format!(
                "PATH={}",
                crate::management::path(&state.config).to_string_lossy()
            ),
        );
        options.program = "/usr/bin/env".into();
        (format!("agent-setup:{agent}"), options)
    } else {
        let project = &authority
            .project
            .as_ref()
            .ok_or_else(ApiError::forbidden)?
            .0;
        let path = PathBuf::from(&project.path);
        let options = SpawnOptions::shell(path).map_err(ApiError::conflict)?;
        (project.id.clone(), options)
    };
    let path = &options.cwd;
    if !path.is_dir() {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "The granted project folder is not available on this relay",
        ));
    }
    let permit = state.terminal_permit()?;
    let rows = query
        .rows
        .unwrap_or(dunlin_terminal::DEFAULT_ROWS)
        .clamp(4, 400);
    let cols = query
        .cols
        .unwrap_or(dunlin_terminal::DEFAULT_COLS)
        .clamp(20, 500);
    options.rows = rows;
    options.cols = cols;
    let connection_id = format!("term-{}", uuid::Uuid::now_v7());
    Ok(ws.on_upgrade(move |socket| async move {
        bridge(
            socket,
            state,
            authority,
            connection_id,
            project_id,
            options,
            permit,
        )
        .await;
    }))
}

async fn bridge(
    mut socket: WebSocket,
    state: AppState,
    authority: Authority,
    connection_id: String,
    project_id: String,
    options: SpawnOptions,
    _permit: OwnedSemaphorePermit,
) {
    let mut authority_changes = match state.store.lock() {
        Ok(store) => store.subscribe_catalog(),
        Err(_) => return,
    };
    let spawn_permit = match state.fs_permit() {
        Ok(permit) => permit,
        Err(_) => {
            let _ = socket.send(revoked()).await;
            return;
        }
    };
    let spawn_state = state.clone();
    let spawn_authority = authority.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        let _permit = spawn_permit;
        if !spawn_authority.check_blocking(&spawn_state) {
            return Ok(None);
        }
        let store = spawn_state
            .store
            .lock()
            .map_err(|_| "Store unavailable".to_owned())?;
        store
            .native_checkout_transaction(|store| {
                if !spawn_authority.allowed_in_transaction(store, &spawn_state) {
                    return Ok(None);
                }
                // Filesystem validation precedes this fresh identity/checkout SQL
                // cut; the exact epochs remain pinned through process creation.
                let session = RelaySession(Session::spawn(options).map_err(ApiError::internal)?);
                session.enable_relay_mode().map_err(ApiError::internal)?;
                Ok::<_, ApiError>(Some(Arc::new(session)))
            })
            .map_err(|_| "Terminal authority or process creation failed".to_owned())
    })
    .await;
    let session = match spawned {
        Ok(Ok(Some(session))) => session,
        Ok(Ok(None)) => {
            let _ = socket.send(revoked()).await;
            return;
        }
        failure => {
            if !authority.check(&state).await {
                let _ = socket.send(revoked()).await;
                return;
            }
            let message = match failure {
                Ok(Err(error)) => error,
                Err(error) => format!("Relay terminal task failed: {error}"),
                _ => unreachable!(),
            };
            let _ = socket
                .send(Message::Text(
                    json!({"type":"error","message":message}).to_string().into(),
                ))
                .await;
            return;
        }
    };
    let _session_owner = RelayOwner(session.clone());
    if !authority.check(&state).await {
        session.hard_terminate();
        let _ = socket.send(revoked()).await;
        return;
    }

    let guarded_session = session.clone();
    let guarded_state = state.clone();
    let guarded_authority = authority.clone();
    let (revocation, mut revoked_rx) = tokio::sync::watch::channel(false);
    let _guardian = Guardian(tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = authority_changes.recv() => {}, _ = tick.tick() => {} }
            if !guarded_authority.check(&guarded_state).await {
                guarded_session.hard_terminate();
                let _ = revocation.send(true);
                break;
            }
        }
    }));

    let (mut sink, mut stream) = socket.split();
    let ready = json!({
        "type": "ready",
        "connection": connection_id,
        "project": project_id,
        "pid": session.pid(),
    });
    if !tokio::time::timeout(
        Duration::from_secs(1),
        sink.send(Message::Text(ready.to_string().into())),
    )
    .await
    .is_ok_and(|result| result.is_ok())
    {
        session.hard_terminate();
        return;
    }
    let output = session.output();
    let exit = session.exit();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pending_exit = None;
    let mut output_drain_deadline = tokio::time::Instant::now();
    let mut authority_lost = false;

    loop {
        tokio::select! {
            chunk = output.recv() => {
                match chunk {
                    Ok(bytes) => {
                        if !authority.check(&state).await { authority_lost = true; break; }
                        if !tokio::time::timeout(Duration::from_secs(1), sink.send(Message::Binary(bytes.into()))).await.is_ok_and(|result| result.is_ok()) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            status = exit.recv(), if pending_exit.is_none() => {
                if !authority.check(&state).await { authority_lost = true; break; }
                match status {
                    Ok(exit) => {
                        // Waiter completion is independent of the PTY reader.
                        // Keep forwarding authorized final bytes until EOF.
                        pending_exit = Some(exit);
                        output_drain_deadline = tokio::time::Instant::now() + OUTPUT_DRAIN_BUDGET;
                    }
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(output_drain_deadline), if pending_exit.is_some() => break,
            message = stream.next() => {
                // Disconnect/close only decreases authority. It must still
                // terminate this exact terminal after grant/metadata revocation.
                if matches!(&message,Some(Ok(Message::Close(_)))|None)
                    || matches!(&message,Some(Ok(Message::Text(text)))
                        if matches!(serde_json::from_str::<ClientControl>(text),Ok(ClientControl::Close))) {
                    session.hard_terminate();
                    let _=tokio::time::timeout(Duration::from_secs(1),sink.send(Message::Close(None))).await;
                    break;
                }
                if !authority.check(&state).await { authority_lost = true; break; }
                match message {
                    Some(Ok(Message::Binary(bytes))) if pending_exit.is_none() => {
                        if bytes.len() > MAX_INPUT_FRAME {
                            let _ = send_json_sink(
                                &mut sink,
                                json!({"type": "error", "message": "Terminal input frame is too large"}),
                            ).await;
                            break;
                        }
                        let permit = match state.fs_permit() {
                            Ok(permit)=>permit,
                            Err(_)=>{authority_lost=true;break;}
                        };
                        let writer = session.clone();
                        let write_state = state.clone();
                        let write_authority = authority.clone();
                        let write = tokio::task::spawn_blocking(move || {
                            let _permit = permit;
                            if !write_authority.check_blocking(&write_state) { return Err("Terminal authorization changed".to_owned()); }
                            writer.write_relay(&bytes)?;
                            if !write_authority.check_blocking(&write_state) { return Err("Terminal authorization changed".to_owned()); }
                            Ok::<_,String>(())
                        });
                        tokio::select! {
                            _ = tokio::time::sleep(INPUT_WRITE_BUDGET) => { session.hard_terminate(); break; }
                            _ = revoked_rx.changed() => { authority_lost = true; break; }
                            result = write => {
                                if !matches!(result, Ok(Ok(()))) {
                                    if !authority.check(&state).await { authority_lost = true; }
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientControl>(&text) {
                            Ok(ClientControl::Resize { rows, cols }) => {
                                let result = session.resize(rows.clamp(4, 400), cols.clamp(20, 500));
                                if let Err(error) = result {
                                    let _ = send_json_sink(&mut sink, json!({"type": "error", "message": error})).await;
                                }
                            }
                            Ok(ClientControl::Close) => {
                                session.hard_terminate();
                                let _ = tokio::time::timeout(Duration::from_secs(1), sink.send(Message::Close(None))).await;
                                break;
                            }
                            Ok(ClientControl::Ping) => {
                                let _ = send_json_sink(&mut sink, json!({"type": "pong"})).await;
                            }
                            Err(error) => {
                                let _ = send_json_sink(
                                    &mut sink,
                                    json!({"type": "error", "message": format!("Invalid terminal control message: {error}")}),
                                ).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            _ = revoked_rx.changed() => { authority_lost = true; break; }
            _ = heartbeat.tick() => {
                if !authority.check(&state).await { authority_lost = true; break; }
                if !tokio::time::timeout(Duration::from_secs(1), sink.send(Message::Ping(Vec::new().into()))).await.is_ok_and(|result| result.is_ok()) {
                    break;
                }
            }
        }
    }
    session.hard_terminate();
    if authority_lost || !authority.check(&state).await {
        let _ = tokio::time::timeout(Duration::from_secs(1), sink.send(revoked())).await;
        return;
    }
    let final_exit = match pending_exit {
        Some(status) => Some(status),
        None => tokio::time::timeout(Duration::from_secs(2), exit.recv())
            .await
            .ok()
            .and_then(Result::ok),
    };
    if let Some(status) = final_exit {
        if authority.check(&state).await {
            let _ = send_exit(&mut sink, status).await;
        } else {
            let _ = tokio::time::timeout(Duration::from_secs(1), sink.send(revoked())).await;
        }
    }
}

async fn send_json_sink(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: serde_json::Value,
) -> Result<(), ()> {
    tokio::time::timeout(
        Duration::from_secs(1),
        sink.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn send_exit(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    exit: Exit,
) -> Result<(), ()> {
    send_json_sink(sink, json!({"type": "exit", "exit": exit})).await?;
    let _ = tokio::time::timeout(Duration::from_secs(1), sink.send(Message::Close(None))).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_control_frames_and_rejects_unknown_actions() {
        let resize: ClientControl =
            serde_json::from_str(r#"{"type":"resize","rows":24,"cols":80}"#).unwrap();
        assert!(matches!(
            resize,
            ClientControl::Resize { rows: 24, cols: 80 }
        ));
        assert!(matches!(
            serde_json::from_str::<ClientControl>(r#"{"type":"close"}"#).unwrap(),
            ClientControl::Close
        ));
        assert!(serde_json::from_str::<ClientControl>(r#"{"type":"exec","argv":["sh"]}"#).is_err());
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    #[tokio::test]
    async fn setup_requires_current_admin_identity_without_a_project_grant_or_provider_login() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open_in_memory().unwrap();
        let admin = store.create_user("setup-admin", "admin").unwrap();
        let (device, _) = store.create_device(&admin.id, "setup-device").unwrap();
        let member = store.create_user("ordinary-member", "member").unwrap();
        let (member_device, _) = store.create_device(&member.id, "member-device").unwrap();
        let state = AppState::new(
            store,
            crate::config::Config {
                db: dir.path().join("relay.db"),
                ..Default::default()
            },
        );
        let auth = AuthUser {
            user: admin,
            device,
        };
        let authority = Authority::capture(&state, &auth, None).await.unwrap();
        assert!(authority.project.is_none());
        assert!(authority.check(&state).await);
        let mut cached_admin_claim = member;
        cached_admin_claim.role = "admin".into();
        assert!(
            Authority::capture(
                &state,
                &AuthUser {
                    user: cached_admin_claim,
                    device: member_device
                },
                None
            )
            .await
            .is_err()
        );
        state
            .store
            .lock()
            .unwrap()
            .revoke_device(&auth.device.id)
            .unwrap();
        assert!(!authority.check(&state).await);
    }
}
