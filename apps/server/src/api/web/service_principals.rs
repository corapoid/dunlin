//! Human-managed automation identities; dedicated bearer-only read API.
use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::Response,
};
use dunlin_domain::collaboration::Permission;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Create {
    project_id: String,
    name: String,
    permissions: BTreeSet<Permission>,
    expires_in: u64,
}
pub(in crate::api) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Json(body): Json<Create>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let (principal, token) = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .create_service_principal(
            &auth.user.id,
            &workspace,
            &body.project_id,
            &body.name,
            body.permissions,
            body.expires_in,
        )
        .map_err(team_error)?;
    Ok(private_response(Json(
        serde_json::json!({"principal":principal,"token":token}),
    )))
}
pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let principals = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .service_principals(&auth.user.id, &workspace)
        .map_err(team_error)?;
    Ok(private_response(Json(principals)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Revoke {
    expected_revision: u64,
}
pub(in crate::api) async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, id)): Path<(String, String)>,
    Json(body): Json<Revoke>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let principal = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .revoke_service_principal(&auth.user.id, &workspace, &id, body.expected_revision)
        .map_err(team_error)?;
    Ok(private_response(Json(principal)))
}
pub(in crate::api) async fn workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, id, version)): Path<(String, String, String, u64)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(ApiError::unauthorized());
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    if store.authenticate_service(token)?.is_none() {
        return Err(ApiError::unauthorized());
    }
    let version = store
        .service_workflow_version(token, &workspace, &project, &id, version)
        .map_err(team_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(version)))
}
