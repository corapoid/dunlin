//! Bounded, manager-scoped push observations. Full snapshots have no execution
//! authority and no replay history; reconnect starts a new observation stream.
use super::*;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;
const PROTOCOL: &str = "dunlin.host-health.v1";
const SEND: Duration = Duration::from_secs(5);

pub(in crate::api) async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, host)): Path<(String, String)>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let mut protocols = headers.get_all(header::SEC_WEBSOCKET_PROTOCOL).iter();
    if protocols.next().and_then(|value| value.to_str().ok()) != Some(PROTOCOL)
        || protocols.next().is_some()
    {
        return Err(ApiError::bad_request(
            "Use the host-health WebSocket protocol",
        ));
    }
    observation(&state, &headers, &workspace, &host)?;
    let permit = state
        .browser_sessions
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "Live connection limit reached",
            )
        })?;
    // Retain only the three bounded validated credentials/origin fields, never
    // arbitrary request headers. No credential is sent in frames or URL params.
    let token = cookie_token(&headers)?;
    let mut auth = HeaderMap::new();
    auth.insert(header::HOST, headers[header::HOST].clone());
    auth.insert(header::ORIGIN, headers[header::ORIGIN].clone());
    auth.insert(
        header::COOKIE,
        format!("dunlin_web_session={token}")
            .parse()
            .map_err(|_| ApiError::unauthorized())?,
    );
    Ok(ws
        .protocols([PROTOCOL])
        .max_message_size(1024)
        .max_frame_size(1024)
        .on_upgrade(move |socket| stream(socket, state, auth, workspace, host, permit))
        .into_response())
}
fn observation(
    state: &AppState,
    headers: &HeaderMap,
    workspace: &str,
    host: &str,
) -> Result<Value, ApiError> {
    let value = human(state, headers, false, |store, user| {
        store.host_health_observation(user, workspace, host, &state.worker_channels)
    })?;
    serde_json::to_value(value).map_err(|_| super::super::unavailable())
}
fn fingerprint(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("observed_at_ms");
    }
    if let Some(connection) = value.get_mut("connection").and_then(Value::as_object_mut) {
        connection.remove("heartbeat_age_ms");
        connection.remove("lease_remaining_ms");
    }
    value
}
async fn send(socket: &mut WebSocket, value: Value) -> Result<(), ()> {
    let text = serde_json::to_string(&value).map_err(|_| ())?;
    if text.len() > 16 * 1024 {
        return Err(());
    }
    tokio::time::timeout(SEND, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}
async fn stream(
    mut socket: WebSocket,
    state: AppState,
    headers: HeaderMap,
    workspace: String,
    host: String,
    _permit: OwnedSemaphorePermit,
) {
    let stream = uuid::Uuid::now_v7().to_string();
    let mut sequence = 0_u64;
    let mut previous = None;
    let mut ticks = tokio::time::interval(Duration::from_millis(500));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut heartbeat = Instant::now();
    let mut last_pong = Instant::now();
    let mut frame_window = Instant::now();
    let mut frames = 0_u16;
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                if frame_window.elapsed() >= Duration::from_secs(60) {
                    frame_window = Instant::now(); frames = 0;
                }
                frames += 1;
                if frames > 120 { break; }
                match incoming {
                    Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                    Some(Ok(Message::Ping(bytes))) => {
                        if !tokio::time::timeout(SEND, socket.send(Message::Pong(bytes)))
                            .await.is_ok_and(|result| result.is_ok()) { break; }
                    },
                    _ => break, // This stream accepts no application commands.
                }
            },
            _ = ticks.tick() => {
                if state.worker_channels.is_shutting_down()
                    || last_pong.elapsed() >= Duration::from_secs(15) { break; }
                // Fresh device, native authority, membership and exact host
                // scope on every tick, including when there is no data change.
                let value = match observation(&state, &headers, &workspace, &host) {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = send(&mut socket, serde_json::json!({
                            "type":"unavailable", "protocol_version":1, "stream_id":stream,
                        })).await;
                        break;
                    },
                };
                let key = fingerprint(&value);
                if previous.as_ref() != Some(&key) {
                    let Some(next) = sequence.checked_add(1)
                        .filter(|next| *next <= dunlin_relay_protocol::worker::MAX_EXECUTION_INTEGER)
                    else { break; };
                    sequence = next;
                    if send(&mut socket, serde_json::json!({
                        "type":"snapshot", "protocol_version":1, "stream_id":stream,
                        "sequence":sequence, "health":value,
                    })).await.is_err() { break; }
                    previous = Some(key);
                }
                if heartbeat.elapsed() >= Duration::from_secs(5) {
                    if send(&mut socket, serde_json::json!({
                        "type":"heartbeat", "protocol_version":1, "stream_id":stream,
                    })).await.is_err() { break; }
                    if !tokio::time::timeout(SEND, socket.send(Message::Ping(Vec::new().into())))
                        .await.is_ok_and(|result| result.is_ok()) { break; }
                    heartbeat = Instant::now();
                }
            },
        }
    }
    let _ = tokio::time::timeout(SEND, socket.send(Message::Close(None))).await;
}
