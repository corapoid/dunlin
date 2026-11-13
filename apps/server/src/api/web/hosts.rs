//! Human registry control and separate, provisional worker credentials.
mod live;
use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};
use crate::store::{Store, StoreError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::Response,
};
use dunlin_domain::hosts::{HostCapabilities, HostDefinition};
pub(in crate::api) use live::upgrade as live_health;
use serde::Deserialize;
use serde_json::json;
fn human<T>(
    state: &AppState,
    headers: &HeaderMap,
    write: bool,
    operation: impl FnOnce(&Store, &str) -> Result<T, StoreError>,
) -> Result<T, ApiError> {
    request_origin(headers, write)?;
    let auth = authorized(state, &cookie_token(headers)?)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let user = crate::management::authority::current_user_locked(
        state,
        &store,
        &auth.user.id,
        &auth.device.id,
    )?;
    operation(&store, &user.id).map_err(team_error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Revision {
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Drain {
    expected_revision: u64,
    drain: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Enroll {
    ticket: String,
    capabilities: HostCapabilities,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Report {
    host_id: String,
    enrollment_epoch: u64,
    credential_revision: u64,
    capabilities: HostCapabilities,
}
pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    let hosts = human(&state, &headers, false, |store, user| {
        store.hosts(user, &workspace)
    })?;
    Ok(private_response(Json(json!({"hosts":hosts}))))
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, false, |store, user| {
        store.host(user, &workspace, &id)
    })?
    .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(host)))
}
pub(in crate::api) async fn revocation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let revocation = human(&state, &headers, false, |store, user| {
        store.host_revocation(user, &workspace, &id)
    })?;
    Ok(private_response(Json(json!({"revocation":revocation}))))
}
pub(in crate::api) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Json(body): Json<HostDefinition>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, true, |store, user| {
        store.create_host(user, &workspace, body)
    })?;
    Ok(private_response(Json(host)))
}
pub(in crate::api) async fn enrollment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
    Json(body): Json<Revision>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, true, |store, user| {
        store.rotate_host_enrollment(user, &workspace, &id, body.expected_revision)
    })?;
    Ok(private_response(Json(host)))
}
pub(in crate::api) async fn approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
    Json(body): Json<Revision>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, true, |store, user| {
        store.approve_host(user, &workspace, &id, body.expected_revision)
    })?;
    Ok(private_response(Json(host)))
}
pub(in crate::api) async fn drain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
    Json(body): Json<Drain>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, true, |store, user| {
        store.drain_host(user, &workspace, &id, body.expected_revision, body.drain)
    })?;
    Ok(private_response(Json(host)))
}
pub(in crate::api) async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
    Json(body): Json<Revision>,
) -> Result<Response, ApiError> {
    let host = human(&state, &headers, true, |store, user| {
        store.revoke_host(user, &workspace, &id, body.expected_revision)
    })?;
    Ok(private_response(Json(host)))
}
fn worker_error(error: StoreError) -> ApiError {
    match error {
        StoreError::Policy(_) => ApiError::unauthorized(),
        error => team_error(error),
    }
}
fn no_browser(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key(header::COOKIE) || headers.contains_key(header::ORIGIN) {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}
pub(in crate::api) async fn worker_enroll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Enroll>,
) -> Result<Response, ApiError> {
    no_browser(&headers)?;
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(ApiError::unauthorized());
    }
    let credential = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .enroll_host(&body.ticket, body.capabilities)
        .map_err(worker_error)?;
    Ok(private_response(Json(credential)))
}
pub(in crate::api) async fn worker_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Report>,
) -> Result<Response, ApiError> {
    no_browser(&headers)?;
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or_else(ApiError::unauthorized)?;
    if values.next().is_some() {
        return Err(ApiError::unauthorized());
    }
    let token = value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let host = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .report_host(
            token,
            &body.host_id,
            body.enrollment_epoch,
            body.credential_revision,
            body.capabilities,
        )
        .map_err(worker_error)?;
    Ok(private_response(Json(json!({"host":host}))))
}

/// Persisted diagnostic data, never an assertion of live network or execution
/// authority. Operators compare boot/expiry; all reads still require ManageHosts.
pub(in crate::api) async fn channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let channel = human(&state, &headers, false, |store, user| {
        store.worker_channel_diagnostic(user, &workspace, &id)
    })?;
    Ok(private_response(Json(
        json!({"channel":channel,"current_controller_boot_id":state.worker_channels.boot_id,"reported_state_only":true}),
    )))
}

/// Current accepted WSS observation plus manager-scoped operational counts.
/// The existing /channel endpoint retains its persisted-diagnostic contract.
pub(in crate::api) async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let observation: crate::store::HostHealthObservation =
        human(&state, &headers, false, |store, user| {
            store.host_health_observation(user, &workspace, &id, &state.worker_channels)
        })?;
    Ok(private_response(Json(observation)))
}
