//! Cookie-only binding configuration. Filesystem observations never hold Store;
//! fresh SQL cuts fence identity, logical grants and immutable checkout pins.
use super::{
    ApiError, AppState, authorized, cookie_token, private_response, request_origin,
    teams::team_error,
};
use crate::store::{Store, StoreError, local_metadata::MetadataPermission};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use dunlin_domain::collaboration::Permission;
use serde::Deserialize;
use serde_json::json;
struct BindingError(ApiError);
impl From<StoreError> for BindingError {
    fn from(error: StoreError) -> Self {
        Self(team_error(error))
    }
}
fn cut<T>(
    store: &Store,
    operation: impl FnOnce(&Store) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    store
        .native_checkout_transaction(|store| operation(store).map_err(BindingError))
        .map_err(|error| error.0)
}
fn current(
    state: &AppState,
    store: &Store,
    token: &str,
) -> Result<(crate::store::User, crate::store::Device), ApiError> {
    let device = store
        .browser_session_device(token)
        .map_err(team_error)?
        .ok_or_else(ApiError::unauthorized)?;
    let user = crate::management::authority::current_user_locked(
        state,
        store,
        &device.user_id,
        &device.id,
    )
    .map_err(|error| {
        if error.status.is_server_error() {
            super::unavailable()
        } else {
            error
        }
    })?;
    Ok((user, device))
}
fn same_identity(
    state: &AppState,
    store: &Store,
    token: &str,
    user: &str,
    device: &str,
) -> Result<(), ApiError> {
    let (actual, actual_device) = current(state, store, token)?;
    if actual.id != user || actual_device.id != device {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}
fn can_manage(store: &Store, user: &crate::store::User, w: &str, p: &str) -> bool {
    user.role == "admin"
        && store
            .capture_remote_binding_authority(&user.id, w, p)
            .is_ok()
}
fn optional_authority<T>(result: Result<T, StoreError>) -> Result<Option<T>, ApiError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(
            StoreError::Policy(_)
            | StoreError::Conflict(_)
            | StoreError::Limit(_)
            | StoreError::Io(_),
        ) => Ok(None),
        Err(error) => Err(team_error(error)),
    }
}
pub(in crate::api) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    let token = cookie_token(&headers)?;
    authorized(&state, &token)?;
    let permit = state.fs_permit()?;
    let worker_state = state.clone();
    let worker_token = token.clone();
    let w = workspace.clone();
    let p = project.clone();
    let (user, device, stamp, binding, authority, filesystem_current) =
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let (user, device, stamp, binding, native, candidate) = {
                let store = worker_state
                    .store
                    .lock()
                    .map_err(|_| super::unavailable())?;
                cut(&store, |store| {
                    let (user, device) = current(&worker_state, store, &worker_token)?;
                    let stamp = store
                        .authorize_team_project(&user.id, &w, &p, Permission::ViewProject)
                        .map_err(team_error)?;
                    let binding = store
                        .local_binding_for_user(&user.id, &w, &p)
                        .map_err(team_error)?;
                    let native = binding
                        .as_ref()
                        .map(|b| store.project_for_user(&b.native_project_id, &user.id))
                        .transpose()
                        .map_err(team_error)?
                        .flatten();
                    let candidate = if let Some(b) = binding.as_ref().filter(|b| b.active) {
                        optional_authority(store.native_checkout_candidate_in_transaction(
                            &b.native_project_id,
                            &user.id,
                            MetadataPermission::Read,
                        ))?
                    } else {
                        None
                    };
                    Ok::<_, ApiError>((user.id, device.id, stamp, binding, native, candidate))
                })?
            };
            let validated = candidate
                .map(|c| optional_authority(c.validate_filesystem()))
                .transpose()?
                .flatten();
            let authority = if let Some(validated) = validated {
                let store = worker_state
                    .store
                    .lock()
                    .map_err(|_| super::unavailable())?;
                cut(&store, |store| {
                    same_identity(&worker_state, store, &worker_token, &user, &device)?;
                    optional_authority(store.authorize_native_checkout_in_transaction(validated))
                })?
            } else {
                None
            };
            let filesystem_current = match (&binding, &native, &authority) {
                (Some(b), Some(n), Some(a)) => b.active && b.validate_native_checkout(n, a).is_ok(),
                _ => false,
            };
            Ok::<_, ApiError>((user, device, stamp, binding, authority, filesystem_current))
        })
        .await
        .map_err(|_| super::unavailable())??;
    // No await follows this fresh cut. Missing own metadata Read affects only
    // current configuration, never erases an otherwise authorized saved binding.
    let value = {
        let store = state.store.lock().map_err(|_| super::unavailable())?;
        cut(&store, |store| {
            let (actual, actual_device) = current(&state, store, &token)?;
            if actual.id != user || actual_device.id != device {
                return Err(ApiError::unauthorized());
            }
            let now = store
                .authorize_team_project(&user, &workspace, &project, Permission::ViewProject)
                .map_err(team_error)?;
            if now != stamp {
                return Err(ApiError::forbidden());
            }
            let saved = store
                .local_binding_for_user(&user, &workspace, &project)
                .map_err(team_error)?;
            if saved != binding {
                return Err(ApiError::conflict(
                    "Binding changed; reload its saved configuration",
                ));
            }
            let configuration_current = filesystem_current
                && match &authority {
                    Some(a) => {
                        optional_authority(store.revalidate_native_checkout_in_transaction(a))?
                            .is_some()
                    }
                    None => false,
                };
            Ok::<_, ApiError>(
                json!({"binding":binding,"configuration_current":configuration_current,"can_manage":can_manage(store,&actual,&workspace,&project)}),
            )
        })?
    };
    Ok(private_response(Json(value)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Set {
    native_project_id: String,
    expected_revision: Option<u64>,
    metadata_approval_revision: Option<u64>,
}
pub(in crate::api) async fn set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(body): Json<Set>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let auth = authorized(&state, &token)?;
    let permit = state.fs_permit()?;
    let worker_state = state.clone();
    let worker_token = token.clone();
    let w = workspace.clone();
    let p = project.clone();
    let expected_user = auth.user.id.clone();
    let expected_device = auth.device.id.clone();
    let user = expected_user.clone();
    let device = expected_device.clone();
    let binding = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let preparation = {
            let store = worker_state
                .store
                .lock()
                .map_err(|_| super::unavailable())?;
            cut(&store, |store| {
                same_identity(&worker_state, store, &worker_token, &user, &device)?;
                store
                    .prepare_local_binding_in_transaction(
                        &user,
                        &w,
                        &p,
                        &body.native_project_id,
                        body.expected_revision,
                        body.metadata_approval_revision,
                    )
                    .map_err(team_error)
            })?
        };
        let prepared = preparation.validate_filesystem().map_err(team_error)?;
        let store = worker_state
            .store
            .lock()
            .map_err(|_| super::unavailable())?;
        store
            .commit_local_binding_authorized(prepared, |store, actor| {
                if actor != user {
                    return Err(BindingError(ApiError::unauthorized()));
                }
                same_identity(&worker_state, store, &worker_token, &user, &device)
                    .map_err(BindingError)
            })
            .map_err(|error| error.0)
    })
    .await
    .map_err(|_| super::unavailable())??;
    mutation_response(
        &state,
        &token,
        &expected_user,
        &expected_device,
        &workspace,
        &project,
        binding,
    )
}
fn mutation_response<T: serde::Serialize>(
    state: &AppState,
    token: &str,
    user: &str,
    device: &str,
    w: &str,
    p: &str,
    binding: T,
) -> Result<Response, ApiError> {
    let store = state.store.lock().map_err(|_| super::unavailable())?;
    cut(&store, |store| {
        same_identity(state, store, token, user, device)?;
        let (actual, _) = current(state, store, token)?;
        if !can_manage(store, &actual, w, p) {
            return Err(ApiError::forbidden());
        }
        Ok::<_, ApiError>(())
    })?;
    Ok(private_response(Json(binding)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Revoke {
    expected_revision: u64,
}
pub(in crate::api) async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, project)): Path<(String, String)>,
    Json(body): Json<Revoke>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    let auth = authorized(&state, &token)?;
    // No filesystem permit or checkout lookup is needed to remove authority.
    let binding = {
        let store = state.store.lock().map_err(|_| super::unavailable())?;
        store
            .revoke_local_binding_authorized(
                &auth.user.id,
                &workspace,
                &project,
                body.expected_revision,
                |store| {
                    same_identity(&state, store, &token, &auth.user.id, &auth.device.id)
                        .map_err(BindingError)
                },
            )
            .map_err(|error| error.0)?
    };
    mutation_response(
        &state,
        &token,
        &auth.user.id,
        &auth.device.id,
        &workspace,
        &project,
        binding,
    )
}
