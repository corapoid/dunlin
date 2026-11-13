//! State-free proof of the human device that authorized host publication.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdminAuthority {
    pub user: String,
    device: String,
}
impl AdminAuthority {
    pub fn capture(state: &AppState, auth: &AuthUser) -> Result<Self, ApiError> {
        let proof = Self {
            user: auth.user.id.clone(),
            device: auth.device.id.clone(),
        };
        proof.validate(state)?;
        Ok(proof)
    }
    pub fn validate(&self, state: &AppState) -> Result<(), ApiError> {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        self.validate_locked(state, &store)
    }
    pub fn validate_locked(
        &self,
        state: &AppState,
        store: &crate::store::Store,
    ) -> Result<(), ApiError> {
        let user = current_user_locked(state, store, &self.user, &self.device)?;
        if user.role != "admin" {
            return Err(ApiError::forbidden());
        }
        Ok(())
    }
}
pub(crate) fn current_user_locked(
    state: &AppState,
    store: &crate::store::Store,
    user_id: &str,
    device_id: &str,
) -> Result<crate::store::User, ApiError> {
    let user = store
        .active_device_user(device_id)?
        .filter(|user| user.id == user_id)
        .ok_or_else(ApiError::unauthorized)?;
    crate::api::check_host_account_policy(state, &user)?;
    if !store.oidc_device_allowed(device_id, state.config.oidc.as_ref())? {
        return Err(ApiError::unauthorized());
    }
    Ok(user)
}
pub(super) fn uncertain(detail: impl std::fmt::Display) -> ApiError {
    ApiError::conflict(format!(
        "Publication may have partially completed: {detail}. Inspect the destination and registered projects before a new operation; do not automatically retry."
    ))
}
pub(super) async fn worker<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| uncertain("Publication worker stopped"))?
}
/// An unpublished, generated directory. It never removes a published target.
pub(super) struct PrivateStage(pub PathBuf);
impl Drop for PrivateStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
