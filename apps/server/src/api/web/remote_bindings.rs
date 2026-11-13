//! Remote targets come only from a correlated current worker channel. Browser
//! JSON chooses opaque IDs and CAS revisions, never paths or descriptors.
use super::{
    ApiError, AppState, cookie_token, private_response, request_origin, teams::team_error,
};
use crate::{
    store::{Device, Store, User},
    worker_channel::DescribeError,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use dunlin_relay_protocol::worker::{ExportPrincipal, PrincipalKind};
use serde::Deserialize;
use serde_json::json;
fn cut<T>(
    store: &Store,
    mutation: bool,
    operation: impl FnOnce(&Store) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    store
        .remote_binding_transaction(mutation, operation)
        .map_err(|error: ApiError| {
            if error.status.is_server_error() {
                super::unavailable()
            } else {
                error
            }
        })
}
fn current(state: &AppState, store: &Store, token: &str) -> Result<(User, Device), ApiError> {
    let device = store
        .browser_session_device(token)?
        .ok_or_else(ApiError::unauthorized)?;
    let user = crate::management::authority::current_user_locked(
        state,
        store,
        &device.user_id,
        &device.id,
    )?;
    Ok((user, device))
}
fn now_ms() -> Result<i64, ApiError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .ok_or_else(super::unavailable)
}
fn describe_error(error: DescribeError) -> ApiError {
    match error {
        DescribeError::Unavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Current worker channel is unavailable",
        ),
        DescribeError::Forbidden => ApiError::forbidden(),
        DescribeError::StaleExport => {
            ApiError::conflict("Worker export changed; review its current revision")
        }
        DescribeError::Busy => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Worker metadata capacity reached",
        ),
        DescribeError::InvalidResponse => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "Worker metadata response could not be verified",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Create {
    host_id: String,
    export_id: String,
    expected_export_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Save {
    host_id: String,
    export_id: String,
    expected_export_revision: u64,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Revoke {
    expected_revision: u64,
}
pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let token = cookie_token(&headers)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let bindings = cut(&store, false, |store| {
        let (user, _) = current(&state, store, &token)?;
        store
            .remote_bindings_in_transaction(&user.id, &workspace, &project)
            .map_err(team_error)
    })?;
    Ok(private_response(Json(
        json!({"bindings":bindings,"observed_state_only":true,"execution_ready":false}),
    )))
}
pub(in crate::api) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(body): Json<Create>,
) -> Result<Response, ApiError> {
    save_operation(state, headers, workspace, project, None, None, body).await
}
pub(in crate::api) async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, id)): Path<(String, String, String)>,
    Json(body): Json<Save>,
) -> Result<Response, ApiError> {
    save_operation(
        state,
        headers,
        workspace,
        project,
        Some(id),
        Some(body.expected_revision),
        Create {
            host_id: body.host_id,
            export_id: body.export_id,
            expected_export_revision: body.expected_export_revision,
        },
    )
    .await
}
async fn save_operation(
    state: AppState,
    headers: HeaderMap,
    workspace: String,
    project: String,
    id: Option<String>,
    expected_revision: Option<u64>,
    body: Create,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let valid_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    };
    let maximum = dunlin_relay_protocol::worker::MAX_EXPORT_REVISION;
    if !valid_id(&body.host_id)
        || !valid_id(&body.export_id)
        || !(1..=maximum).contains(&body.expected_export_revision)
        || expected_revision.is_some_and(|v| !(1..=maximum).contains(&v))
        || id.as_deref().is_some_and(|id| !valid_id(id))
    {
        return Err(ApiError::bad_request(
            "Invalid remote binding IDs or revisions",
        ));
    }
    let token = cookie_token(&headers)?;
    let (user, device, authority) = {
        let store = state.store.lock().map_err(|_| super::unavailable())?;
        cut(&store, false, |store| {
            let (user, device) = current(&state, store, &token)?;
            let authority = store
                .capture_remote_binding_authority(&user.id, &workspace, &project)
                .map_err(team_error)?;
            store
                .host(&user.id, &workspace, &body.host_id)
                .map_err(team_error)?
                .ok_or_else(ApiError::not_found)?;
            Ok((user.id, device.id, authority))
        })?
    };
    let proof = state
        .worker_channels
        .describe_export(
            &state,
            &body.host_id,
            &body.export_id,
            ExportPrincipal {
                kind: PrincipalKind::Human,
                id: user.clone(),
            },
            Some(body.expected_export_revision),
        )
        .await
        .map_err(describe_error)?;
    let controller = state
        .config
        .worker_controller
        .as_ref()
        .ok_or_else(super::unavailable)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let binding = cut(&store, true, |store| {
        let (current_user, current_device) = current(&state, store, &token)?;
        if current_user.id != user || current_device.id != device {
            return Err(ApiError::unauthorized());
        }
        store
            .set_remote_binding_in_transaction(
                &user,
                &workspace,
                &project,
                id.as_deref(),
                expected_revision,
                &body.host_id,
                &body.export_id,
                body.expected_export_revision,
                &proof,
                &state.worker_channels.boot_id,
                &controller.node_id,
                now_ms()?,
                &authority,
            )
            .map_err(team_error)
    })?;
    Ok(private_response(Json(binding)))
}
pub(in crate::api) async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, id)): Path<(String, String, String)>,
    Json(body): Json<Revoke>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let binding = cut(&store, true, |store| {
        let (user, _) = current(&state, store, &token)?;
        store
            .revoke_remote_binding_in_transaction(
                &user.id,
                &workspace,
                &project,
                &id,
                body.expected_revision,
            )
            .map_err(team_error)
    })?;
    Ok(private_response(Json(binding)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Resolve {
    binding: dunlin_domain::binding::BindingRef,
}
/// This endpoint resolves read-only metadata for one exact reference. It never
/// chooses a local target, creates a session or admits an execution attempt.
pub(in crate::api) async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(body): Json<Resolve>,
) -> Result<Response, ApiError> {
    use crate::store::remote_bindings::{
        RemoteTargetCandidate, ResolvedRemoteTarget, ResolvedRemoteTargetSummary,
    };
    request_origin(&headers, true)?;
    body.binding
        .validate()
        .map_err(|_| ApiError::bad_request("Invalid remote binding reference"))?;
    let token = cookie_token(&headers)?;
    let controller_node = state
        .config
        .worker_controller
        .as_ref()
        .ok_or_else(super::unavailable)?
        .node_id
        .clone();
    let (user, device, candidate): (String, String, RemoteTargetCandidate) = {
        let store = state.store.lock().map_err(|_| super::unavailable())?;
        cut(&store, false, |store| {
            let (user, device) = current(&state, store, &token)?;
            let candidate = store
                .capture_remote_target_in_transaction(
                    &user.id,
                    &workspace,
                    &project,
                    &body.binding,
                    &controller_node,
                )
                .map_err(team_error)?;
            Ok((user.id, device.id, candidate))
        })?
    };
    let proof = state
        .worker_channels
        .describe_export(
            &state,
            candidate.host_id(),
            candidate.export_id(),
            ExportPrincipal {
                kind: PrincipalKind::Human,
                id: user.clone(),
            },
            Some(candidate.export_revision()),
        )
        .await
        .map_err(describe_error)?;
    let current_node = &state
        .config
        .worker_controller
        .as_ref()
        .ok_or_else(super::unavailable)?
        .node_id;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let resolved: ResolvedRemoteTarget = cut(&store, false, |store| {
        let (actual, actual_device) = current(&state, store, &token)?;
        if actual.id != user || actual_device.id != device {
            return Err(ApiError::unauthorized());
        }
        store
            .finish_remote_target_in_transaction(
                &user,
                &workspace,
                &project,
                candidate,
                proof,
                &state.worker_channels.boot_id,
                current_node,
                now_ms()?,
            )
            .map_err(team_error)
    })?;
    let summary: ResolvedRemoteTargetSummary = resolved.summary().clone();
    Ok(private_response(Json(summary)))
}
