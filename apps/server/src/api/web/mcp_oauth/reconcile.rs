//! Explicit reconciliation needs neither provider configuration nor vault I/O.
use super::*;
use crate::{auth, mcp_oauth::BrowserProof};
pub(in crate::api) async fn reconcile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, connection)): Path<(String, String)>,
    Json(request): Json<Disconnect>,
) -> Result<Response, ApiError> {
    let token = cookie_token(&headers)?;
    let result = owner(&state, &headers, true, |store, user| {
        let row = store
            .mcp_oauth_connection(user, &workspace, &connection)
            .map_err(error)?
            .ok_or_else(ApiError::not_found)?;
        if row.revision != request.expected_revision {
            return Err(error(PersistenceError::Conflict));
        }
        // A live blocking worker owns this gate until it actually returns,
        // including after cancellation of its original HTTP waiter.
        let lease = store.mcp_oauth_revocation_lease().map_err(error)?;
        let auth = authorized_in_store(&state, store, &token)?;
        let proof = BrowserProof {
            user: auth.user.id,
            device: auth.device.id,
            session_hash: auth::hash_token(&token),
        };
        let membership = store
            .team_membership(&workspace, user)?
            .filter(|m| m.active)
            .ok_or_else(ApiError::forbidden)?;
        store
            .mcp_oauth_reconcile_revocation(
                &AuthContext {
                    user_id: user.into(),
                    workspace_id: workspace,
                    connection_id: connection.clone(),
                    connection_revision: request.expected_revision,
                },
                &lease,
                &|store, context| {
                    proof.validate(&state, store, context)?;
                    let current = store
                        .team_membership(&context.workspace_id, &context.user_id)
                        .map_err(|_| PersistenceError::Storage)?
                        .ok_or(PersistenceError::Unauthorized)?;
                    if !current.active || current.revision != membership.revision {
                        return Err(PersistenceError::Unauthorized);
                    }
                    Ok(())
                },
            )
            .map_err(error)
    })?;
    Ok(private_response(Json(json!({"connection_id":connection,
        "revision":result.connection_revision,"confirmed":result.confirmed,"unconfirmed":result.unconfirmed,
        "provider_revocation":if result.confirmed_known_tokens {"confirmed_known_tokens"} else {"unconfirmed"}
    }))))
}
