//! Owner-managed explicit transcript readers. Sharing never authorizes execution.
use super::*;
use axum::extract::Path;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Replace {
    expected_revision: u64,
    reader_ids: Vec<String>,
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let token = cookie_token(&headers)?;
    let store = state.store.lock().map_err(|_| unavailable())?;
    let auth = authorized_in_store(&state, &store, &token)?;
    let view = store
        .session_sharing(&auth.user.id, &id)
        .map_err(super::teams::team_error)?;
    Ok(private_response(Json(view)))
}
pub(in crate::api) async fn replace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Replace>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let store = state.store.lock().map_err(|_| unavailable())?;
    let auth = authorized_in_store(&state, &store, &token)?;
    let view = store
        .replace_session_readers(&auth.user.id, &id, body.expected_revision, &body.reader_ids)
        .map_err(super::teams::team_error)?;
    Ok(private_response(Json(view)))
}
