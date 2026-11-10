//! Authenticated relay PTY over WebSocket.
//!
//! Binary frames carry raw PTY bytes in both directions. Text frames are small
//! JSON control messages (`ready`, `exit`, `resize`, `close`, `ping`). The shell
//! runs on the relay host inside a project the authenticated user was granted;
//! there is no local fallback on the desktop.

use std::path::PathBuf;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use dunlin_terminal::{Exit, Session, SpawnOptions};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::OwnedSemaphorePermit;

use crate::api::{ApiError, AppState, AuthUser};

const MAX_INPUT_FRAME: usize = 2 * 1024 * 1024;
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

pub async fn ws(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
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
        let project = state.project_for_user(&auth.user.id, &query.project)?;
        let path = PathBuf::from(&project.path);
        let options = SpawnOptions::shell(path).map_err(ApiError::conflict)?;
        (project.id, options)
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
        bridge(socket, connection_id, project_id, options, permit).await;
    }))
}

async fn bridge(
    mut socket: WebSocket,
    connection_id: String,
    project_id: String,
    options: SpawnOptions,
    _permit: OwnedSemaphorePermit,
) {
    let session = match tokio::task::spawn_blocking(move || Session::spawn(options)).await {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "message": error})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
        Err(error) => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "message": format!("Relay terminal task failed: {error}")})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let (mut sink, mut stream) = socket.split();
    let ready = json!({
        "type": "ready",
        "connection": connection_id,
        "project": project_id,
        "pid": session.pid(),
    });
    if sink
        .send(Message::Text(ready.to_string().into()))
        .await
        .is_err()
    {
        session.kill();
        return;
    }
    let output = session.output();
    let exit = session.exit();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut reported = false;

    loop {
        tokio::select! {
            chunk = output.recv() => {
                match chunk {
                    Ok(bytes) => {
                        if sink.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            status = exit.recv() => {
                if let Ok(exit) = status {
                    let _ = send_exit(&mut sink, exit).await;
                    reported = true;
                }
                break;
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() > MAX_INPUT_FRAME {
                            let _ = send_json_sink(
                                &mut sink,
                                json!({"type": "error", "message": "Terminal input frame is too large"}),
                            ).await;
                            break;
                        }
                        if let Err(error) = session.write(&bytes) {
                            let _ = send_json_sink(&mut sink, json!({"type": "error", "message": error})).await;
                            break;
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
                                session.kill();
                                let _ = sink.send(Message::Close(None)).await;
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
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    session.kill();
    if !reported
        && let Ok(Ok(status)) = tokio::time::timeout(Duration::from_secs(2), exit.recv()).await
    {
        let _ = send_exit(&mut sink, status).await;
    }
}

async fn send_json_sink(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: serde_json::Value,
) -> Result<(), axum::Error> {
    sink.send(Message::Text(value.to_string().into())).await
}

async fn send_exit(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    exit: Exit,
) -> Result<(), axum::Error> {
    send_json_sink(sink, json!({"type": "exit", "exit": exit})).await?;
    let _ = sink.send(Message::Close(None)).await;
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
