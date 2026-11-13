//! Cookie-authenticated, opt-in team metadata. No host grants are widened here.
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use dunlin_domain::collaboration::WorkspaceRole;
use serde::Deserialize;

use super::{ApiError, AppState, authorized, cookie_token, private_response, request_origin};
use crate::store::StoreError;

pub(super) fn team_error(error: StoreError) -> ApiError {
    match error {
        StoreError::Policy(_) => ApiError::forbidden(),
        StoreError::Conflict(message) => ApiError::conflict(message),
        StoreError::Limit(message) => ApiError::bad_request(message),
        _ => ApiError::internal("Team metadata operation failed"),
    }
}

pub(in crate::api) async fn list_workspaces(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let workspaces = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .workspaces_for_user(&auth.user.id)
        .map_err(team_error)?;
    Ok(private_response(Json(workspaces)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct CreateWorkspace {
    name: String,
}

pub(in crate::api) async fn create_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateWorkspace>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let workspace = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .create_workspace(&auth.user, &request.name)
        .map_err(team_error)?;
    Ok(private_response(Json(workspace)))
}

pub(in crate::api) async fn team_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let members = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .team_members(&auth.user.id, &workspace)
        .map_err(team_error)?;
    Ok(private_response(Json(members)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct UpdateMember {
    user_id: String,
    role: WorkspaceRole,
    active: bool,
    expected_revision: Option<u64>,
}

pub(in crate::api) async fn update_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Json(request): Json<UpdateMember>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    if workspace.len() > 128 || request.user_id.is_empty() || request.user_id.len() > 128 {
        return Err(ApiError::bad_request("Invalid workspace or user id"));
    }
    let member = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .set_team_member(
            &auth.user.id,
            &workspace,
            &request.user_id,
            request.role,
            request.active,
            request.expected_revision,
        )
        .map_err(team_error)?;
    Ok(private_response(Json(member)))
}

pub(in crate::api) async fn team_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let events = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .team_audit(&auth.user.id, &workspace)
        .map_err(team_error)?;
    Ok(private_response(Json(events)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct InviteMember {
    user_id: String,
    role: WorkspaceRole,
}

pub(in crate::api) async fn invite_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Json(request): Json<InviteMember>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let token = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .invite_team_member(&auth.user.id, &workspace, &request.user_id, request.role)
        .map_err(team_error)?;
    Ok(private_response(Json(
        serde_json::json!({"token":token,"expires_in":86400}),
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct AcceptInvitation {
    token: String,
}

pub(in crate::api) async fn accept_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AcceptInvitation>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let member = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .accept_team_invitation(&auth.user.id, &request.token)
        .map_err(team_error)?;
    Ok(private_response(Json(member)))
}

pub(in crate::api) async fn revoke_invitations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, recipient)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .revoke_team_invitations(&auth.user.id, &workspace, &recipient)
        .map_err(team_error)?;
    Ok(private_response(Json(serde_json::json!({"revoked":true}))))
}
