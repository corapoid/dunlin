//! Experimental session bootstrap/diagnostics. AHP remains the planned live
//! transport; no substitute polling or ad-hoc prompt protocol is exposed here.
use super::{ApiError, AppState, authorized, cookie_token, private_response, request_origin};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
pub(in crate::api) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(options): Json<crate::interactive::CreateRequest>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let key = idempotency_key(&headers)?;
    let expected = expected_identity(&headers)?;
    let token = cookie_token(&headers)?;
    let auth = authorized(&state, &token)?;
    check_expected_identity(&auth, expected.as_ref())?;
    let accepted = if let crate::interactive::CreateRequest::Profile {
        profile,
        mcp_connections,
        target: Some(dunlin_domain::binding::ProjectTargetRef::Remote { binding }),
    } = &options
    {
        binding
            .validate()
            .map_err(|_| ApiError::bad_request("invalid_project_target"))?;
        let service = state.remote_sessions.get().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_IMPLEMENTED,
                "remote_execution_not_supported",
            )
        })?;
        let key = key
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("Remote sessions require Idempotency-Key"))?;
        service
            .create_with_mcp(
                &state,
                &auth,
                &workspace,
                &project,
                crate::store::remote_bindings::resolver::sessions::RemoteSessionRequest {
                    binding: binding.clone(),
                    profile: profile.clone(),
                },
                key,
                mcp_connections.clone(),
            )
            .await
            .map_err(remote_error)?
    } else {
        options.validate_target().map_err(admission_error)?;
        if key.is_none() && !state.config.experimental_interactive_runtime {
            return Err(ApiError::not_found());
        }
        let permit = state.fs_permit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let auth = authorized(&state, &token)?;
            check_expected_identity(&auth, expected.as_ref())?;
            crate::interactive::Service::create_request_with_key(
                state, auth, workspace, project, options, key,
            )
            .map_err(admission_error)
        })
        .await
        .map_err(|_| super::unavailable())??
    };
    let mut response = private_response(
        (
            StatusCode::ACCEPTED,
            Json(json!({
                "session": accepted.session,
                "admission": {
                    "replayed": accepted.replayed,
                    "session_is_acceptance_snapshot": true,
                },
            })),
        )
            .into_response(),
    );
    response.headers_mut().insert(
        "idempotency-replayed",
        axum::http::HeaderValue::from_static(if accepted.replayed { "true" } else { "false" }),
    );
    Ok(response)
}
fn expected_identity(headers: &HeaderMap) -> Result<Option<(String, String)>, ApiError> {
    fn value(headers: &HeaderMap, name: &'static str) -> Result<Option<String>, ApiError> {
        let mut values = headers.get_all(name).iter();
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(ApiError::bad_request(
                "Expected identity headers must have exactly one value",
            ));
        }
        let value = value
            .to_str()
            .map_err(|_| ApiError::bad_request("Invalid expected identity header"))?;
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ApiError::bad_request("Invalid expected identity header"));
        }
        Ok(Some(value.into()))
    }
    match (
        value(headers, "x-dunlin-expected-user")?,
        value(headers, "x-dunlin-expected-device")?,
    ) {
        (None, None) => Ok(None),
        (Some(user), Some(device)) => Ok(Some((user, device))),
        _ => Err(ApiError::bad_request(
            "Both expected user and expected device headers are required together",
        )),
    }
}
fn check_expected_identity(
    auth: &crate::api::AuthUser,
    expected: Option<&(String, String)>,
) -> Result<(), ApiError> {
    if expected.is_some_and(|(user, device)| user != &auth.user.id || device != &auth.device.id) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Browser identity changed before admission; this request created no session",
        ));
    }
    Ok(())
}
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let mut values = headers.get_all("idempotency-key").iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(ApiError::bad_request(
            "Exactly one Idempotency-Key is required",
        ));
    }
    let key = value
        .to_str()
        .map_err(|_| ApiError::bad_request("Invalid Idempotency-Key"))?;
    if !crate::interactive::valid_admission_key(key) {
        return Err(ApiError::bad_request(
            "Idempotency-Key must contain 16–128 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(Some(key.into()))
}
fn remote_error(error: crate::remote_execution::Error) -> ApiError {
    use crate::remote_execution::Error;
    match error {
        Error::Disabled => ApiError::not_found(),
        Error::Unauthorized => ApiError::forbidden(),
        Error::Conflict => ApiError::conflict("Remote session identity or authority changed"),
        Error::Capacity => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Remote session capacity reached",
        ),
        Error::Unavailable | Error::Uncertain => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Remote session admission unavailable; retry with the same Idempotency-Key",
        ),
    }
}
fn admission_error(error: &'static str) -> ApiError {
    match error {
        "remote_execution_not_supported" => ApiError::new(StatusCode::NOT_IMPLEMENTED, error),
        "project_target_changed" | "checkout_identity_changed" | "checkout_or_grant_changed" => {
            ApiError::conflict(error)
        }
        "invalid_project_target" => ApiError::bad_request(error),
        "idempotency_conflict" => ApiError::conflict(
            "Idempotency-Key was already used for another request or authority epoch",
        ),
        "runtime_capacity_reached"
        | "checkout_validation_busy"
        | "harness_management_busy"
        | "session_admission_capacity_reached" => {
            ApiError::new(StatusCode::TOO_MANY_REQUESTS, error)
        }
        "runtime_shutting_down"
        | "runtime_executor_unavailable"
        | "store_unavailable"
        | "registry_unavailable"
        | "session_admission_failed" => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, error),
        "interactive_runtime_disabled" => ApiError::not_found(),
        "authority_revoked" | "identity_revoked" => ApiError::unauthorized(),
        "host_policy_changed"
        | "authority_unavailable"
        | "admission_authority_denied"
        | "checkout_or_grant_unavailable"
        | "native_grant_unavailable"
        | "view_grant_required"
        | "create_grant_required"
        | "binding_missing" => ApiError::forbidden(),
        _ => ApiError::bad_request(error),
    }
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    crate::management::authority::current_user_locked(
        &state,
        &store,
        &auth.user.id,
        &auth.device.id,
    )?;
    let (workspace, project) = store
        .interactive_scope(&id)
        .map_err(super::teams::team_error)?;
    let authority = store
        .team_project_context(&workspace, &project, &auth.user.id)
        .map_err(super::teams::team_error)?;
    let (session, turns) = store
        .interactive_journal()
        .snapshot(&authority, &id)
        .map_err(|_| ApiError::forbidden())?;
    let profile = store
        .interactive_journal()
        .session_profile(&authority, &id)
        .map_err(|_| super::unavailable())?;
    // Diagnostics are bounded metadata; transcripts will use AHP pagination.
    let turns = turns
        .into_iter()
        .map(|t| json!({"id":t.id,"phase":t.phase}))
        .collect::<Vec<_>>();
    Ok(private_response(Json(
        json!({"session":session,"turns":turns,"profile":profile}),
    )))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Stop {
    expected_epoch: u64,
}
pub(in crate::api) async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Stop>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let auth = authorized(&state, &cookie_token(&headers)?)?;
    state
        .interactive
        .stop(&state, &auth, &id, body.expected_epoch)
        .map_err(|e| {
            if e == "session_owner_required" {
                ApiError::forbidden()
            } else {
                ApiError::conflict(e)
            }
        })?;
    if let Some(service) = state.remote_sessions.get() {
        service.signal_stop(&id);
    }
    Ok(private_response(
        (StatusCode::ACCEPTED, Json(json!({"stop_requested":true}))).into_response(),
    ))
}
