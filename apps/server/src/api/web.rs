//! Same-origin, cookie-authenticated browser bootstrap and read-only live catalog.
//! No credentials in URLs or JavaScript storage; existing bearer API is unchanged.
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use dunlin_relay_protocol::web::{
    CATALOG_VERSION, CatalogCommand, CatalogEvent, CatalogProject, CatalogSession, CatalogSnapshot,
    MAX_CATALOG_PROJECTS, MAX_CLIENT_FRAME, MAX_SNAPSHOT_BYTES,
};
use dunlin_relay_protocol::{PairRequest, User};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};

use super::{ApiError, AppState, AuthUser};
use crate::store::Device;

pub(super) mod agents;
pub(super) mod ahp;
pub(super) mod bindings;
pub(super) mod hosts;
pub(super) mod interactive;
pub(super) mod mcp_oauth;
pub(super) mod oidc;
mod projects;
pub(super) mod remote_bindings;
pub(super) mod scoped_secrets;
pub(super) mod service_principals;
pub(super) mod sharing;
mod teams;
mod workflows;
pub(super) use projects::{create_project, list_projects, update_project_grant};
pub(super) use teams::{
    accept_invitation, create_workspace, invite_member, list_workspaces, revoke_invitations,
    team_audit, team_members, update_team_member,
};
pub(super) use workflows::{
    create_workflow, get_workflow, publish_workflow, save_workflow, workflow_version,
};

const COOKIE: &str = "dunlin_web_session";
const SESSION_TTL: Duration = Duration::from_secs(8 * 60 * 60);
const HEARTBEAT: Duration = Duration::from_secs(15);

pub(super) struct BrowserSessions {
    connections: Arc<Semaphore>,
    boot: String,
}

impl Default for BrowserSessions {
    fn default() -> Self {
        Self {
            connections: Arc::new(Semaphore::new(64)),
            boot: uuid::Uuid::now_v7().to_string(),
        }
    }
}

fn register(state: &AppState, device: &Device) -> Result<String, ApiError> {
    state
        .store
        .lock()
        .map_err(|_| unavailable())?
        .create_browser_session(&device.id, SESSION_TTL.as_secs())
        .map_err(|error| match error {
            crate::store::StoreError::Limit(_) => ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Browser session limit reached",
            ),
            crate::store::StoreError::Policy(_) => ApiError::unauthorized(),
            _ => unavailable(),
        })
}

fn unavailable() -> ApiError {
    ApiError::internal("Browser session service unavailable")
}

/// No proxy headers are trusted here. Serve must preserve the original Host.
/// A syntactically valid ts.net name is NOT proof that Funnel is disabled.
fn request_origin(headers: &HeaderMap, required: bool) -> Result<bool, ApiError> {
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .ok_or_else(ApiError::forbidden)?;
    let host = authority.host().to_ascii_lowercase();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let local = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    let tailscale = dunlin_domain::relay::tailscale_hostname(host);
    if (!local && !tailscale) || authority.as_str().contains('@') {
        return Err(ApiError::forbidden());
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return if required {
            Err(ApiError::forbidden())
        } else {
            Ok(!local)
        };
    };
    let origin: Uri = origin
        .to_str()
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(ApiError::forbidden)?;
    let scheme = origin.scheme_str().ok_or_else(ApiError::forbidden)?;
    if !origin
        .authority()
        .is_some_and(|value| value.as_str().eq_ignore_ascii_case(authority.as_str()))
        || !matches!(scheme, "http" | "https")
        || (!local && scheme != "https")
        || origin.path() != "/"
        || origin.query().is_some()
    {
        return Err(ApiError::forbidden());
    }
    Ok(scheme == "https")
}

fn cookie_token(headers: &HeaderMap) -> Result<String, ApiError> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header
            .to_str()
            .map_err(|_| ApiError::unauthorized())?
            .split(';')
        {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == COOKIE
            {
                if found.is_some()
                    || value.len() != 47
                    || !value.starts_with("kst_")
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    return Err(ApiError::unauthorized());
                }
                found = Some(value.to_string());
            }
        }
    }
    found.ok_or_else(ApiError::unauthorized)
}

fn authorized(state: &AppState, token: &str) -> Result<AuthUser, ApiError> {
    let store = state.store.lock().map_err(|_| unavailable())?;
    authorized_in_store(state, &store, token)
}
fn authorized_in_store(
    state: &AppState,
    store: &crate::store::Store,
    token: &str,
) -> Result<AuthUser, ApiError> {
    let device = store
        .browser_session_device(token)?
        .ok_or_else(ApiError::unauthorized)?;
    if !store.oidc_device_allowed(&device.id, state.config.oidc.as_ref())? {
        return Err(ApiError::unauthorized());
    }
    let user = store
        .active_device_user(&device.id)?
        .filter(|u| u.id == device.user_id)
        .ok_or_else(ApiError::unauthorized)?;
    super::check_host_account_policy(state, &user)?;
    Ok(AuthUser { user, device })
}

fn cookie(value: &str, secure: bool, ttl: u64) -> HeaderValue {
    // value is generated locally and restricted to URL-safe token characters.
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}; Path=/v1/web; HttpOnly; SameSite=Strict; Max-Age={ttl}{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("generated cookie contains only ASCII")
}

fn private_response(body: impl IntoResponse) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Serve only build artifacts, never arbitrary files from an operator's home.
pub(super) async fn asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    method: Method,
) -> Result<Response, ApiError> {
    let root = state
        .config
        .web_root
        .as_ref()
        .ok_or_else(ApiError::not_found)?;
    let requested = uri.path().trim_start_matches('/');
    let relative = asset_path(requested).ok_or_else(ApiError::not_found)?;
    if method != Method::GET && method != Method::HEAD {
        return Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "Method not allowed",
        ));
    }
    let secure = request_origin(&headers, false)?;
    let candidate = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|_| ApiError::not_found())?;
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|_| ApiError::not_found())?;
    if !candidate.starts_with(&canonical_root) {
        return Err(ApiError::not_found());
    }
    let metadata = tokio::fs::metadata(&candidate)
        .await
        .map_err(|_| ApiError::not_found())?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(ApiError::not_found());
    }
    let mime = match candidate.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => return Err(ApiError::not_found()),
    };
    let bytes = if method == Method::HEAD {
        Vec::new()
    } else {
        tokio::fs::read(candidate)
            .await
            .map_err(|_| ApiError::not_found())?
    };
    let mut response = private_response(bytes);
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    if method == Method::HEAD {
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from(metadata.len()));
    }
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    let ws_scheme = if secure { "wss" } else { "ws" };
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' {ws_scheme}://{authority}; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&csp).map_err(|_| ApiError::forbidden())?,
    );
    Ok(response)
}

fn asset_path(path: &str) -> Option<&str> {
    if path.is_empty() {
        return Some("index.html");
    }
    if path.contains('%')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return None;
    }
    if path == "index.html" || path == "REUI-LICENSE.txt" || path.starts_with("assets/") {
        Some(path)
    } else {
        None
    }
}

pub(super) async fn pair(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PairRequest>,
) -> Result<Response, ApiError> {
    let secure = request_origin(&headers, true)?;
    let paired = super::pair(State(state.clone()), Json(request)).await?.0;
    // The native bearer token is discarded. Browser credentials cannot authenticate v1 bearer APIs.
    let device = state
        .store
        .lock()
        .map_err(|_| unavailable())?
        .list_devices(&paired.user.id)?
        .into_iter()
        .find(|device| device.id == paired.device_id)
        .ok_or_else(ApiError::unauthorized)?;
    super::check_host_account_policy(&state, &paired.user)?;
    let token = register(&state, &device)?;
    let mut response = private_response(Json(
        json!({"user": paired.user, "device_id": paired.device_id}),
    ));
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie(&token, secure, SESSION_TTL.as_secs()),
    );
    Ok(response)
}

pub(super) async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    Ok(private_response(Json(json!({
        "user": auth.user, "device_id": auth.device.id,
        "server_name": state.config.name, "protocol_version": CATALOG_VERSION,
        "capabilities": {"catalog": true, "chat": false, "workflow": false,
            "ahp_observer": (state.config.experimental_interactive_runtime || state.remote_sessions.get().is_some()) && state.config.store_transcripts},
        "ahp_endpoint": "/v1/web/ahp"
    }))))
}

pub(super) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let secure = request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let auth = authorized(&state, &token)?;
    state
        .store
        .lock()
        .map_err(|_| unavailable())?
        .revoke_device(&auth.device.id)?;
    state
        .store
        .lock()
        .map_err(|_| unavailable())?
        .remove_browser_session(&token)?;
    let mut response = private_response(StatusCode::NO_CONTENT);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie("", secure, 0));
    Ok(response)
}

pub(super) async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    authorized(&state, &token)?;
    let permit = state
        .browser_sessions
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Live connection limit reached",
            )
        })?;
    Ok(ws
        .max_message_size(MAX_CLIENT_FRAME)
        .max_frame_size(MAX_CLIENT_FRAME)
        .on_upgrade(move |socket| live(socket, state, token, permit))
        .into_response())
}

fn snapshot(
    state: &AppState,
    auth: &AuthUser,
    project_id: &Option<String>,
    stream_id: &str,
    sequence: u64,
) -> Result<CatalogSnapshot, ApiError> {
    let store = state.store.lock().map_err(|_| unavailable())?;
    let projects = store.projects_for_user(&auth.user.id)?;
    if projects.len() > MAX_CATALOG_PROJECTS {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Catalog project limit reached",
        ));
    }
    let sessions = if let Some(project) = project_id {
        if !projects.iter().any(|candidate| candidate.id == *project) {
            return Err(ApiError::forbidden());
        }
        store
            .list_sessions(&auth.user.id, project)?
            .into_iter()
            .map(|session| CatalogSession {
                id: session.id,
                title: session.title,
                updated_at: session.updated_at,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(CatalogSnapshot {
        protocol_version: CATALOG_VERSION,
        stream_id: stream_id.to_string(),
        sequence,
        workspace_revision: store.team_catalog_revision(&auth.user.id)?,
        server_name: state.config.name.clone(),
        user: User {
            id: auth.user.id.clone(),
            name: auth.user.name.clone(),
            role: auth.user.role.clone(),
        },
        projects: projects
            .into_iter()
            .map(|project| CatalogProject {
                id: project.id,
                name: project.name,
            })
            .collect(),
        project_id: project_id.clone(),
        sessions,
    })
}

async fn send(socket: &mut WebSocket, event: CatalogEvent) -> Result<(), ()> {
    let text = serde_json::to_string(&event).map_err(|_| ())?;
    if text.len() > MAX_SNAPSHOT_BYTES {
        return Err(());
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn live(
    mut socket: WebSocket,
    state: AppState,
    token: String,
    _permit: OwnedSemaphorePermit,
) {
    // Subscribe before reading a snapshot, so a concurrent write is never missed.
    let mut changes = match state.store.lock() {
        Ok(store) => store.subscribe_catalog(),
        Err(_) => return,
    };
    let stream_id = uuid::Uuid::now_v7().to_string();
    let mut sequence = 0;
    let mut project_id = None;
    let mut subscribed = false;
    let mut last_snapshot: Option<CatalogSnapshot> = None;
    let mut window = Instant::now();
    let mut commands = 0;
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pong = Instant::now();
    loop {
        let mut publish = false;
        let mut force_snapshot = false;
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if window.elapsed() >= Duration::from_secs(60) {
                            commands = 0;
                            window = Instant::now();
                        }
                        commands += 1;
                        if commands > 60 || text.len() > MAX_CLIENT_FRAME { break; }
                        match serde_json::from_str::<CatalogCommand>(&text) {
                            Ok(CatalogCommand::Subscribe { protocol_version, project_id: selected })
                                if protocol_version == CATALOG_VERSION && selected.as_ref().is_none_or(|id| id.len() <= 128) => {
                                project_id = selected;
                                subscribed = true;
                                publish = true;
                                force_snapshot = true;
                            }
                            _ => {
                                let _ = send(&mut socket, CatalogEvent::Error { code: "unsupported_command".into() }).await;
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                    Some(Ok(Message::Ping(_))) => {},
                    _ => break,
                }
            }
            changed = changes.recv() => {
                match changed {
                    Ok(()) => publish = subscribed,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if subscribed {
                            if send(&mut socket, CatalogEvent::ResyncRequired).await.is_err() { break; }
                            publish = true;
                            force_snapshot = true;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                if last_pong.elapsed() > HEARTBEAT * 3 { break; }
                if authorized(&state, &token).is_err() {
                    let _ = send(&mut socket, CatalogEvent::Error { code: "authorization_required".into() }).await;
                    break;
                }
                if let Some(project) = &project_id {
                    let access = authorized(&state, &token)
                        .and_then(|auth| state.project_for_user(&auth.user.id, project));
                    if access.is_err() {
                        let _ = send(&mut socket, CatalogEvent::Error { code: "catalog_unavailable".into() }).await;
                        break;
                    }
                }
                // This is a liveness/auth check, not data polling.
                if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(Vec::new().into())))
                    .await, Ok(Ok(()))) { break; }
            }
        }
        if publish {
            let result = authorized(&state, &token)
                .and_then(|auth| snapshot(&state, &auth, &project_id, &stream_id, sequence));
            match result {
                Ok(mut snapshot) => {
                    // Hidden-scope writes must not emit observable sequence bumps.
                    // A subscription/resync still receives an explicit snapshot.
                    if !force_snapshot && last_snapshot.as_ref() == Some(&snapshot) {
                        continue;
                    }
                    sequence += 1;
                    snapshot.sequence = sequence;
                    last_snapshot = Some(snapshot.clone());
                    if send(
                        &mut socket,
                        CatalogEvent::Snapshot {
                            snapshot: Box::new(snapshot),
                        },
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let code = if error.status == StatusCode::UNAUTHORIZED {
                        "authorization_required"
                    } else {
                        "catalog_unavailable"
                    };
                    let _ = send(&mut socket, CatalogEvent::Error { code: code.into() }).await;
                    break;
                }
            }
        }
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: "Live subscription ended".into(),
        }))),
    )
    .await;
}

#[cfg(test)]
mod tests;
