//! Explicit owner-authorized negative sweep; no provider configuration or AS I/O.
use super::*;
use crate::{
    auth,
    mcp_oauth::{BrowserProof, CleanupScope},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Cleanup {
    expected_revision: u64,
    expected_cleanup_revision: u64,
}
pub(in crate::api) async fn cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
    Json(request): Json<Cleanup>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let scope = {
        let store = state
            .store
            .lock()
            .map_err(|_| super::super::unavailable())?;
        let auth = authorized_in_store(&state, &store, &token)?;
        expected_user(&headers, &auth.user.id)?;
        let row = store
            .mcp_oauth_connection(&auth.user.id, &workspace, &connection)
            .map_err(error)?
            .ok_or_else(ApiError::not_found)?;
        if row.revision != request.expected_revision
            || row.cleanup.as_ref().map_or(0, |s| s.revision) != request.expected_cleanup_revision
        {
            return Err(error(PersistenceError::Conflict));
        }
        let membership = store
            .team_membership(&workspace, &auth.user.id)?
            .filter(|m| m.active)
            .ok_or_else(ApiError::forbidden)?;
        CleanupScope {
            context: AuthContext {
                user_id: auth.user.id.clone(),
                workspace_id: workspace,
                connection_id: connection,
                connection_revision: row.revision,
            },
            proof: BrowserProof {
                user: auth.user.id,
                device: auth.device.id,
                session_hash: auth::hash_token(&token),
            },
            membership_revision: membership.revision,
        }
    };
    let permit = state
        .mcp_oauth
        .cleanup_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "MCP cleanup service busy"))?;
    let connection_id = scope.context.connection_id.clone();
    let service = state.mcp_oauth.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.cleanup_connection(state, scope, request.expected_cleanup_revision)
    })
    .await
    .map_err(|_| ApiError::internal("MCP cleanup unavailable; reload connection status"))?
    .map_err(error)?;
    Ok(private_response(Json(
        json!({"connection_id":connection_id,"revision":result.revision,"connection_revision":result.connection_revision,
        "status":if result.unconfirmed==0{"sweep_complete"}else{"unconfirmed"},"delete_acknowledged":result.delete_acknowledged,"unconfirmed":result.unconfirmed,"reclaimed":result.reclaimed,"pruned":result.pruned}),
    )))
}
