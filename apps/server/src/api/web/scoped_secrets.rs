//! Private provisioning/status only. No endpoint returns a credential value.
use super::{
    ApiError, AppState, authorized_in_store, cookie_token, private_response, request_origin,
};
use crate::{
    auth,
    mcp_oauth::BrowserProof,
    scoped_secrets::{self, Install},
    store::scoped_secrets::Proof,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use dunlin_domain::vault::SecretPolicy;
use dunlin_vault::scoped::{Binding, Error};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use zeroize::Zeroizing;

fn error(error: Error) -> ApiError {
    match error {
        Error::Denied => ApiError::forbidden(),
        Error::Invalid => ApiError::new(
            StatusCode::BAD_REQUEST,
            "Invalid scoped secret policy or request",
        ),
        Error::Conflict => ApiError::conflict("Scoped secret changed; reload its status"),
        Error::Limit => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Scoped secret capacity reached",
        ),
        Error::WriteUnknown | Error::OutcomeUnknown => ApiError::conflict(
            "Scoped secret outcome unknown; inspect status before making a new change",
        ),
        _ => ApiError::internal("Scoped secret service unavailable"),
    }
}
fn browser(state: &AppState, headers: &HeaderMap, write: bool) -> Result<BrowserProof, ApiError> {
    request_origin(headers, write)?;
    let cookie = Zeroizing::new(cookie_token(headers)?);
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let auth = authorized_in_store(state, &store, &cookie)?;
    let expected = headers
        .get_all("x-dunlin-expected-user")
        .iter()
        .collect::<Vec<_>>();
    if expected.len() > 1
        || expected
            .first()
            .is_some_and(|v| v.to_str().ok() != Some(auth.user.id.as_str()))
    {
        return Err(ApiError::unauthorized());
    }
    Ok(BrowserProof {
        user: auth.user.id,
        device: auth.device.id,
        session_hash: auth::hash_token(&cookie),
    })
}
fn proof(
    state: &AppState,
    headers: &HeaderMap,
    binding: Binding,
    write: bool,
) -> Result<Proof, ApiError> {
    let browser = browser(state, headers, write)?;
    state
        .store
        .lock()
        .map_err(|_| super::unavailable())?
        .scoped_secret_proof(browser, binding)
        .map_err(error)
}

fn binding((workspace, project, host): (String, String, String)) -> Binding {
    Binding {
        workspace,
        project,
        host,
    }
}
fn secret<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Save {
    expected_revision: u64,
    policy: SecretPolicy,
    #[serde(deserialize_with = "secret")]
    value: Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Disable {
    expected_revision: u64,
}

pub(in crate::api) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let proof = proof(&state, &headers, binding(scope), false)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let credentials = store
        .scoped_secret_list(&proof, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(
        json!({"credentials":credentials,"execution_supported":scoped_secrets::execution_supported(&state,&proof.binding.host)}),
    )))
}
pub(in crate::api) async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
    Json(request): Json<Save>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        true,
    )?;
    let version = state
        .scoped_secrets
        .install(
            state.clone(),
            proof.clone(),
            Install {
                credential,
                expected_revision: request.expected_revision,
                policy: request.policy,
                value: request.value,
            },
        )
        .await
        .map_err(error)?;
    // An activation receipt is historical, not a claim that no later writer
    // changed the credential. Recheck the originating identity before returning.
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    store
        .scoped_secret_validate(&proof, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(
        json!({"credential":version.credential,"revision":version.revision,"activation":"confirmed"}),
    )))
}
pub(in crate::api) async fn disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
    Json(request): Json<Disable>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        true,
    )?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let status = store
        .scoped_secret_disable(&proof, &credential, request.expected_revision, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(status)))
}

pub(in crate::api) async fn private_headers(response: Response) -> Response {
    private_response(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct ToolQuery {
    kind: Option<String>,
}
pub(in crate::api) async fn tool_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ToolQuery>,
    Path(scope): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    use crate::store::scoped_secrets::Principal;
    let proof = proof(&state, &headers, binding(scope), false)?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    store
        .scoped_secret_validate(&proof, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    let recipe = match query.kind.as_deref() {
        None | Some("http") => false,
        Some("recipe") => true,
        _ => return Err(error(Error::Invalid)),
    };
    let tool = if recipe {
        dunlin_vault::scoped::tool::recipe()
    } else {
        dunlin_vault::scoped::tool::http()
    };
    let supported = scoped_secrets::execution_supported(&state, &proof.binding.host)
        && (!recipe
            || (proof.binding.host == "local"
                && cfg!(target_os = "linux")
                && state.config.sandbox
                && crate::management::isolated_agents(&state.config)));
    let digest = tool
        .definition_digest()
        .map_err(|_| ApiError::internal("Scoped tool schema unavailable"))?;
    Ok(private_response(Json(
        json!({"provider_prefix":dunlin_vault::scoped::tool::PROVIDER_PREFIX,"tool":tool,"definition_digest":digest,"principal_kinds":[Principal::Human,Principal::Service],"execution_supported":supported}),
    )))
}
pub(in crate::api) async fn grants(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        false,
    )?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let grants: Vec<crate::store::scoped_secrets::Grant> = store
        .scoped_secret_grants(&proof, &credential, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(json!({"grants":grants}))))
}
pub(in crate::api) async fn set_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
    Json(request): Json<crate::store::scoped_secrets::GrantRequest>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        true,
    )?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let grant = store
        .scoped_secret_set_grant(&proof, &credential, &request, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(grant)))
}
pub(in crate::api) async fn revoke_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential, grant)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Json(request): Json<Disable>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        true,
    )?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let grant = store
        .scoped_secret_revoke_grant(
            &proof,
            &credential,
            &grant,
            request.expected_revision,
            &|store| scoped_secrets::identity(&state, store, &proof),
        )
        .map_err(error)?;
    Ok(private_response(Json(grant)))
}
pub(in crate::api) async fn available(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<(String, String, String)>,
    Query(profile): Query<dunlin_domain::agent::AgentProfileRef>,
) -> Result<Response, ApiError> {
    let browser = browser(&state, &headers, false)?;
    let binding = binding(scope);
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let grants = store
        .scoped_secret_available(&browser.user, &binding, &profile, &|store| {
            scoped_secrets::browser_identity(&state, store, &browser)
        })
        .map_err(error)?;
    Ok(private_response(Json(
        json!({"bindings":grants,"execution_supported":scoped_secrets::execution_supported(&state,&binding.host)}),
    )))
}

pub(in crate::api) async fn retention(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        false,
    )?;
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    let result = store
        .scoped_secret_retention(&proof, &credential, &|store| {
            scoped_secrets::identity(&state, store, &proof)
        })
        .map_err(error)?;
    Ok(private_response(Json(result)))
}
pub(in crate::api) async fn cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project, host, credential)): Path<(String, String, String, String)>,
    Json(request): Json<Disable>,
) -> Result<Response, ApiError> {
    let proof = proof(
        &state,
        &headers,
        Binding {
            workspace,
            project,
            host,
        },
        true,
    )?;
    let result = state
        .scoped_secrets
        .cleanup(state.clone(), proof, credential, request.expected_revision)
        .await
        .map_err(error)?;
    Ok(private_response(Json(result)))
}
