//! Explicit check may rotate an expired credential, so require POST + CAS.
use super::*;
use crate::{
    auth,
    mcp_oauth::{BrowserProof, CatalogScope},
};

pub(in crate::api) async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
    Json(request): Json<Disconnect>,
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
            || !matches!(row.status, ConnectionStatus::Connected)
        {
            return Err(error(PersistenceError::Conflict));
        }
        crate::mcp_oauth::config::validate_registry(&state.config.mcp_oauth)
            .map_err(|_| ApiError::internal("Invalid MCP provider configuration"))?;
        let provider = state
            .config
            .mcp_oauth
            .iter()
            .find(|provider| Some(&provider.id) == row.provider_id.as_ref())
            .ok_or_else(|| ApiError::conflict("MCP provider is no longer available"))?
            .clone();
        let membership = store
            .team_membership(&workspace, &auth.user.id)?
            .filter(|member| member.active)
            .ok_or_else(ApiError::forbidden)?;
        CatalogScope {
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
            provider,
        }
    };
    let permit = state
        .mcp_oauth
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "MCP connection service busy"))?;
    let service = state.mcp_oauth.clone();
    let catalog = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.inspect_catalog(state, scope)
    })
    .await
    .map_err(|_| ApiError::internal("MCP catalog unavailable"))?
    .map_err(|failure| match failure {
        PersistenceError::OAuth(_) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "MCP catalog check failed; request was not retried",
        ),
        failure => error(failure),
    })?;
    Ok(private_response(Json(catalog)))
}

pub(in crate::api) async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
    Json(request): Json<Disconnect>,
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
        if row.revision != request.expected_revision {
            return Err(error(PersistenceError::Conflict));
        }
        crate::mcp_oauth::config::validate_registry(&state.config.mcp_oauth)
            .map_err(|_| ApiError::internal("Invalid MCP provider configuration"))?;
        let provider = state
            .config
            .mcp_oauth
            .iter()
            .find(|provider| Some(&provider.id) == row.provider_id.as_ref())
            .ok_or_else(|| ApiError::conflict("MCP provider is no longer available"))?
            .clone();
        let membership = store
            .team_membership(&workspace, &auth.user.id)?
            .filter(|member| member.active)
            .ok_or_else(ApiError::forbidden)?;
        CatalogScope {
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
            provider,
        }
    };
    let permit = state
        .mcp_oauth
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "MCP connection service busy"))?;
    let connection_id = scope.context.connection_id.clone();
    let service = state.mcp_oauth.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.revoke_connection(state, scope)
    })
    .await
    .map_err(|_| ApiError::internal("MCP revocation unavailable; reload connection status"))?
    .map_err(|failure| match failure {
        PersistenceError::OAuth(_) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "MCP revocation unconfirmed; request was not retried",
        ),
        failure => error(failure),
    })?;
    Ok(private_response(Json(
        json!({"connection_id":connection_id,"revision":result.connection_revision,"confirmed":result.confirmed,"unconfirmed":result.unconfirmed,"provider_revocation":if result.confirmed_known_tokens {"confirmed_known_tokens"}else{"unconfirmed"}}),
    )))
}
