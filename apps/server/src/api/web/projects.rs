use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use dunlin_domain::collaboration::Permission;
use serde::Deserialize;

use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};

pub(in crate::api) async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let projects = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .team_projects_for_user(&auth.user.id, &workspace)
        .map_err(team_error)?;
    Ok(private_response(Json(projects)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct CreateProject {
    name: String,
}

pub(in crate::api) async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Json(request): Json<CreateProject>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let project = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .create_team_project(&auth.user.id, &workspace, &request.name)
        .map_err(team_error)?;
    Ok(private_response(Json(project)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct UpdateGrant {
    user_id: String,
    permissions: BTreeSet<Permission>,
    expected_revision: Option<u64>,
}

pub(in crate::api) async fn update_project_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(request): Json<UpdateGrant>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    if request.user_id.is_empty() || request.user_id.len() > 128 {
        return Err(ApiError::bad_request("Invalid user id"));
    }
    let grant = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .set_team_project_grant(
            &auth.user.id,
            &workspace,
            &project,
            &request.user_id,
            request.permissions,
            request.expected_revision,
        )
        .map_err(team_error)?;
    Ok(private_response(Json(grant)))
}
