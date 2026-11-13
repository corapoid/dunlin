//! Project-scoped definitions; publication does not execute a harness.
use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};
use crate::store::{Store, StoreError};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use dunlin_domain::{agent::AgentDefinition, collaboration::Permission};
use serde::Deserialize;
use serde_json::json;

// Revalidate the browser identity under the same Store cut as policy and writes.
fn scoped<T>(
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
pub(in crate::api) struct Save {
    expected_revision: u64,
    definition: AgentDefinition,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Publish {
    expected_revision: u64,
}

pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, false, |store, user| {
        let drafts = store.agent_drafts(user, &workspace, &project)?;
        let can_edit = store
            .authorize_team_project(user, &workspace, &project, Permission::ManageProject)
            .is_ok();
        Ok(json!({"drafts":drafts,"can_edit":can_edit}))
    })?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, false, |store, user| {
        let profiles = store.agent_profiles(user, &workspace, &project)?;
        let can_edit = store
            .authorize_team_project(user, &workspace, &project, Permission::ManageProject)
            .is_ok();
        Ok(
            json!({"profiles":profiles,"can_edit":can_edit,"remote_sessions_supported":state.remote_sessions.get().is_some(),"remote_mcp_sessions_supported":cfg!(unix)&&state.remote_sessions.get().is_some()&&state.config.worker_controller.as_ref().is_some_and(|config|config.mcp_broker)&&!state.config.mcp_oauth.is_empty(),"mcp_sessions_supported":cfg!(unix)&&state.config.experimental_interactive_runtime&&!state.config.mcp_oauth.is_empty(),"configured_harness":crate::interactive::configured_harness_id(&state.config)}),
        )
    })?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(definition): Json<AgentDefinition>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, true, |store, user| {
        store.create_agent(user, &workspace, &project, definition)
    })?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, agent)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, false, |store, user| {
        store.agent_draft(user, &workspace, &project, &agent)
    })?
    .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, agent)): Path<(String, String, String)>,
    Json(body): Json<Save>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, true, |store, user| {
        store.save_agent(
            user,
            &workspace,
            &project,
            &agent,
            body.expected_revision,
            body.definition,
        )
    })?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, agent)): Path<(String, String, String)>,
    Json(body): Json<Publish>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, true, |store, user| {
        store.publish_agent(user, &workspace, &project, &agent, body.expected_revision)
    })?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, agent, version)): Path<(String, String, String, u64)>,
) -> Result<Response, ApiError> {
    let result = scoped(&state, &headers, false, |store, user| {
        store.agent_version(user, &workspace, &project, &agent, version)
    })?
    .ok_or_else(ApiError::not_found)?;
    Ok(private_response(Json(result)))
}
