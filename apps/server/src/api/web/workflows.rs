use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use dunlin_domain::workflow::WorkflowDefinition;
use serde::Deserialize;

use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct SaveDraft {
    expected_revision: u64,
    definition: WorkflowDefinition,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct PublishDraft {
    expected_revision: u64,
}

pub(in crate::api) async fn create_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(definition): Json<WorkflowDefinition>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let draft = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .create_workflow(&auth.user.id, &workspace, &project, definition)
        .map_err(team_error)?;
    Ok(private_response(Json(draft)))
}

pub(in crate::api) async fn get_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, workflow)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let draft = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .workflow_draft(&auth.user.id, &workspace, &project, &workflow)
        .map_err(team_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(draft)))
}

pub(in crate::api) async fn save_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, workflow)): Path<(String, String, String)>,
    Json(request): Json<SaveDraft>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let draft = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .save_workflow(
            &auth.user.id,
            &workspace,
            &project,
            &workflow,
            request.expected_revision,
            request.definition,
        )
        .map_err(team_error)?;
    Ok(private_response(Json(draft)))
}

pub(in crate::api) async fn publish_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, workflow)): Path<(String, String, String)>,
    Json(request): Json<PublishDraft>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let version = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .publish_workflow(
            &auth.user.id,
            &workspace,
            &project,
            &workflow,
            request.expected_revision,
        )
        .map_err(team_error)?;
    Ok(private_response(Json(version)))
}

pub(in crate::api) async fn workflow_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, workflow, version)): Path<(String, String, String, u64)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let version = state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .workflow_version(&auth.user.id, &workspace, &project, &workflow, version)
        .map_err(team_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(version)))
}
