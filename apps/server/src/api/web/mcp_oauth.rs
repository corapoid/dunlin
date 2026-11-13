//! Owner-only OAuth status and durable local disconnect. This surface never
//! reads credentials and does not claim successful provider revocation.
use super::{
    ApiError, AppState, authorized_in_store, cookie_token, private_response, request_origin,
};
use crate::store::{Store, mcp_oauth::ConnectionStatus};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use dunlin_mcp_client::oauth::{
    AuthContext,
    persistence::{CredentialJournal, PersistenceError},
};
use serde::Deserialize;
use serde_json::json;

fn error(error: PersistenceError) -> ApiError {
    match error {
        PersistenceError::Unauthorized => ApiError::forbidden(),
        PersistenceError::Conflict => {
            ApiError::conflict("OAuth connection changed; reload its status")
        }
        PersistenceError::Limit => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "OAuth connection limit reached",
        ),
        _ => ApiError::internal("OAuth connection service unavailable"),
    }
}
fn owner<T>(
    state: &AppState,
    headers: &HeaderMap,
    write: bool,
    operation: impl FnOnce(&Store, &str) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    request_origin(headers, write)?;
    let token = cookie_token(headers)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    // Recheck cookie/device, IdP and host-account policy under the same Store
    // lock as the read/mutation; do not rely on a cached AuthUser.
    let auth = authorized_in_store(state, &store, &token)?;
    expected_user(headers, &auth.user.id)?;
    operation(&store, &auth.user.id)
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let result = owner(&state, &headers, false, |store, user| {
        store
            .mcp_oauth_connection(user, &workspace, &connection)
            .map_err(error)?
            .ok_or_else(ApiError::not_found)
    })?;
    Ok(private_response(Json(result)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Disconnect {
    expected_revision: u64,
}
pub(in crate::api) async fn disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
    Json(request): Json<Disconnect>,
) -> Result<Response, ApiError> {
    let result = owner(&state, &headers, true, |store, user| {
        let mut current = store
            .mcp_oauth_connection(user, &workspace, &connection)
            .map_err(error)?
            .ok_or_else(ApiError::not_found)?;
        if current.revision != request.expected_revision {
            return Err(error(PersistenceError::Conflict));
        }
        if !matches!(current.status, ConnectionStatus::Disabled) {
            current.revision = store
                .disable(&AuthContext {
                    user_id: user.into(),
                    workspace_id: workspace.clone(),
                    connection_id: connection.clone(),
                    connection_revision: request.expected_revision,
                })
                .map_err(error)?;
            current.status = ConnectionStatus::Disabled;
        }
        Ok(current)
    })?;
    Ok(private_response(Json(
        json!({"connection":result,"provider_revocation":"not_confirmed"}),
    )))
}

mod authorization;
mod catalog;
mod cleanup;
mod reconcile;
pub(in crate::api) use authorization::{callback, providers, start};
pub(in crate::api) use catalog::{inspect, revoke};
pub(in crate::api) use cleanup::cleanup;
pub(in crate::api) use reconcile::reconcile;

pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    let connections = owner(&state, &headers, false, |store, user| {
        store.mcp_oauth_connections(user, &workspace).map_err(error)
    })?;
    Ok(private_response(Json(json!({"connections":connections}))))
}

/// Callback URLs may contain a code even when extraction or authorization fails.
/// Apply this at the route boundary, including every early ApiError response.
pub(in crate::api) async fn callback_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}

/// A UI identity expectation is not an authority credential. The cookie remains
/// authoritative; a mismatch prevents a stale tab from acting as another user.
fn expected_user(headers: &HeaderMap, user: &str) -> Result<(), ApiError> {
    let values = headers
        .get_all("x-dunlin-expected-user")
        .iter()
        .collect::<Vec<_>>();
    if values.len() > 1
        || values
            .first()
            .is_some_and(|value| value.to_str().ok() != Some(user))
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}
