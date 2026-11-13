//! Native I/O uses one captured grant epoch, never an implicitly refreshed grant.
use super::{ApiError, AppState, Project, check_host_account_policy};
use crate::store::{NativeGrantStamp, Store};

/// Registry-safe proof: no AppState/Arc references back into its owner.
#[derive(Clone)]
pub(super) struct NativeProof {
    pub project: Project,
    pub user: String,
    pub device: String,
    grant: NativeGrantStamp,
    path: String,
}

#[derive(Clone)]
pub(super) struct NativeIo {
    pub project: Project,
    state: AppState,
    user: String,
    device: String,
    grant: NativeGrantStamp,
    path: String,
}
impl NativeIo {
    pub fn proof(&self) -> NativeProof {
        NativeProof {
            project: self.project.clone(),
            user: self.user.clone(),
            device: self.device.clone(),
            grant: self.grant.clone(),
            path: self.path.clone(),
        }
    }
    pub fn capture(
        state: &AppState,
        user: &str,
        device: &str,
        project: &str,
    ) -> Result<Self, ApiError> {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        Self::capture_in_store(state, &store, user, device, project)
    }
    pub fn capture_in_store(
        state: &AppState,
        store: &Store,
        user: &str,
        device: &str,
        project: &str,
    ) -> Result<Self, ApiError> {
        let project = store
            .project_for_user(project, user)?
            .ok_or_else(ApiError::forbidden)?;
        let grant = store
            .capture_native_grant(&project.id, user)
            .map_err(|_| ApiError::forbidden())?;
        let scope = Self {
            path: project.path.clone(),
            project,
            state: state.clone(),
            user: user.into(),
            device: device.into(),
            grant,
        };
        scope.validate_in_store(store)?;
        Ok(scope)
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        self.validate_in_store(&store)
    }
    pub fn validate_in_store(&self, store: &Store) -> Result<(), ApiError> {
        self.proof().validate_in_store(&self.state, store)
    }
    pub fn after(&self, mutation: bool) -> Result<(), ApiError> {
        self.validate().map_err(|error| {
            if mutation {
                ApiError::conflict("Authority changed during an operation; its result may be partial. Refresh state before a new operation; do not automatically retry.")
            } else {
                error
            }
        })
    }
    pub async fn read<T, F>(self, operation: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    {
        self.run(operation, false).await
    }
    pub async fn mutate<T, F>(self, operation: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    {
        self.run(operation, true).await
    }
    async fn run<T, F>(self, operation: F, mutation: bool) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    {
        let work = self.clone();
        let (started, result) = tokio::task::spawn_blocking(move || {
            if let Err(error) = work.validate() {
                return (false, Err(error));
            }
            let outcome = operation(&work.project);
            let result = work.after(mutation).and_then(|()| {
                outcome.map_err(|error| {
                    if error.starts_with("conflict:") {
                        ApiError::conflict(error)
                    } else if let Some(message) = error.strip_prefix("internal:") {
                        ApiError::internal(message)
                    } else if let Some(message) = error.strip_prefix("quota:") {
                        ApiError::new(super::StatusCode::TOO_MANY_REQUESTS, message)
                    } else {
                        ApiError::bad_request(error)
                    }
                })
            });
            (true, result)
        })
        .await
        .map_err(|_| {
            ApiError::internal("I/O worker stopped; inspect state before a new operation")
        })?;
        if started {
            self.after(mutation)?;
        }
        result
    }
}

impl NativeProof {
    pub fn validate_in_store(&self, state: &AppState, store: &Store) -> Result<(), ApiError> {
        let user = store
            .active_device_user(&self.device)?
            .filter(|u| u.id == self.user)
            .ok_or_else(ApiError::unauthorized)?;
        check_host_account_policy(state, &user)?;
        if !store.oidc_device_allowed(&self.device, state.config.oidc.as_ref())? {
            return Err(ApiError::unauthorized());
        }
        if self.grant.user_id != self.user
            || self.grant.project_id != self.project.id
            || self.path != self.project.path
        {
            return Err(ApiError::forbidden());
        }
        store
            .revalidate_native_grant(&self.grant)
            .map_err(|_| ApiError::forbidden())?;
        let project = store
            .project_for_user(&self.project.id, &self.user)?
            .ok_or_else(ApiError::forbidden)?;
        if project.path != self.path {
            return Err(ApiError::conflict(
                "Project changed; refresh before continuing",
            ));
        }
        Ok(())
    }
}

use crate::store::{local_metadata::MetadataPermission, native_checkout::NativeCheckoutAuthority};
/// Registry-safe Git proof: no AppState, credentials or process owner. The
/// canonical public project is separate from its original native path spelling.
#[derive(Clone)]
pub(super) struct NativeGitProof {
    pub project: Project,
    pub user: String,
    pub device: String,
    native: NativeProof,
    checkout: NativeCheckoutAuthority,
}
impl NativeGitProof {
    pub fn grant_stamp(&self) -> &NativeGrantStamp {
        &self.native.grant
    }
    pub fn authority(&self) -> &NativeCheckoutAuthority {
        &self.checkout
    }
    pub fn original_project_path(&self) -> &str {
        &self.native.path
    }
    pub fn validate_in_store(&self, state: &AppState, store: &Store) -> Result<(), ApiError> {
        store.native_checkout_transaction(|store| self.validate_in_transaction(state, store))
    }
    /// Revalidate inside a caller-owned fresh SQL transaction, without filesystem I/O.
    pub fn validate_in_transaction(&self, state: &AppState, store: &Store) -> Result<(), ApiError> {
        if self.project.id != self.native.project.id
            || self.project.path != self.checkout.root()
            || self.user != self.native.user
            || self.device != self.native.device
        {
            return Err(ApiError::forbidden());
        }
        self.native.validate_in_store(state, store)?;
        store
            .revalidate_native_checkout_in_transaction(&self.checkout)
            .map_err(git_authority_error)
    }
}
#[derive(Clone)]
pub(super) struct NativeGitIo {
    pub project: Project,
    state: AppState,
    proof: NativeGitProof,
}
fn git_authority_error(error: crate::store::StoreError) -> ApiError {
    match error {
        crate::store::StoreError::Policy(_) => ApiError::forbidden(),
        crate::store::StoreError::Conflict(message) => ApiError::conflict(message),
        // Callers supply a project ID, not operator-local roots. An unsupported
        // or unapproved checkout cannot authorize Git; it is not malformed input.
        crate::store::StoreError::Limit(_) => ApiError::forbidden(),
        _ => ApiError::conflict("Native Git checkout authority could not be verified"),
    }
}
fn uncertain_git(error: ApiError, mutation: bool) -> ApiError {
    if mutation {
        ApiError::conflict(
            "Git authority changed during an operation; its result may be partial. Inspect state before a new operation; do not automatically retry.",
        )
    } else {
        error
    }
}
fn git_operation_error(error: String) -> ApiError {
    if error.starts_with("conflict:") {
        ApiError::conflict(error)
    } else if let Some(message) = error.strip_prefix("internal:") {
        ApiError::internal(message)
    } else if let Some(message) = error.strip_prefix("quota:") {
        ApiError::new(super::StatusCode::TOO_MANY_REQUESTS, message)
    } else {
        ApiError::bad_request(error)
    }
}
impl NativeGitIo {
    pub async fn capture(
        state: &AppState,
        user: &str,
        device: &str,
        project: &str,
        permission: MetadataPermission,
    ) -> Result<Self, ApiError> {
        let permit = state.fs_permit()?;
        let worker_state = state.clone();
        let user = user.to_owned();
        let device = device.to_owned();
        let project = project.to_owned();
        let io = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let (native, candidate) = {
                let store = worker_state
                    .store
                    .lock()
                    .map_err(|_| ApiError::internal("Store unavailable"))?;
                let native =
                    NativeIo::capture_in_store(&worker_state, &store, &user, &device, &project)?
                        .proof();
                let candidate = store
                    .native_checkout_candidate(&project, &user, permission)
                    .map_err(git_authority_error)?;
                (native, candidate)
            };
            let validated = candidate
                .validate_filesystem()
                .map_err(git_authority_error)?;
            let checkout = {
                let store = worker_state
                    .store
                    .lock()
                    .map_err(|_| ApiError::internal("Store unavailable"))?;
                native.validate_in_store(&worker_state, &store)?;
                store
                    .authorize_native_checkout(validated)
                    .map_err(git_authority_error)?
            };
            let mut project = native.project.clone();
            project.path = checkout.root().into();
            let proof = NativeGitProof {
                project: project.clone(),
                user: native.user.clone(),
                device: native.device.clone(),
                native,
                checkout,
            };
            let io = Self {
                project,
                state: worker_state,
                proof,
            };
            io.validate_blocking()?;
            Ok::<_, ApiError>(io)
        })
        .await
        .map_err(|_| ApiError::internal("Git authority worker stopped"))??;
        io.validate_sql()?;
        Ok(io)
    }
    pub fn proof(&self) -> NativeGitProof {
        self.proof.clone()
    }
    pub fn authority(&self) -> &NativeCheckoutAuthority {
        &self.proof.checkout
    }
    pub async fn resume(state: &AppState, proof: NativeGitProof) -> Result<Self, ApiError> {
        let permit = state.fs_permit()?;
        let state = state.clone();
        let io = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Self::resume_blocking(&state, &proof)
        })
        .await
        .map_err(|_| ApiError::internal("Git authority worker stopped"))??;
        io.validate_sql()?;
        Ok(io)
    }
    /// Call only inside a bounded blocking worker; no filesystem work holds Store.
    pub fn resume_blocking(state: &AppState, proof: &NativeGitProof) -> Result<Self, ApiError> {
        let io = Self {
            project: proof.project.clone(),
            state: state.clone(),
            proof: proof.clone(),
        };
        io.validate_blocking()?;
        Ok(io)
    }
    pub fn validate_in_store(&self, store: &Store) -> Result<(), ApiError> {
        if self.project.id != self.proof.project.id || self.project.path != self.proof.project.path
        {
            return Err(ApiError::forbidden());
        }
        self.proof.validate_in_store(&self.state, store)
    }
    fn validate_sql(&self) -> Result<(), ApiError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        self.validate_in_store(&store)
    }
    pub fn validate_blocking(&self) -> Result<(), ApiError> {
        self.validate_sql()?;
        self.proof
            .checkout
            .validate_filesystem()
            .map_err(git_authority_error)?;
        self.validate_sql()
    }
    pub async fn validate(&self) -> Result<(), ApiError> {
        let permit = self.state.fs_permit()?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            worker.validate_blocking()
        })
        .await
        .map_err(|_| ApiError::internal("Git authority worker stopped"))??;
        self.validate_sql()
    }
    pub fn after_blocking(&self, mutation: bool) -> Result<(), ApiError> {
        self.validate_blocking()
            .map_err(|error| uncertain_git(error, mutation))
    }
    pub async fn after(&self, mutation: bool) -> Result<(), ApiError> {
        self.validate()
            .await
            .map_err(|error| uncertain_git(error, mutation))
    }
    fn validate_initialized_sql(&self) -> Result<(), ApiError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        store.native_checkout_transaction(|store| {
            self.proof.native.validate_in_store(&self.state, store)?;
            if self.proof.checkout.stamp().permission != MetadataPermission::Write {
                return Err(ApiError::forbidden());
            }
            store
                .revalidate_native_checkout_after_init_in_transaction(&self.proof.checkout)
                .map_err(git_authority_error)
        })
    }
    fn validate_initialized_blocking(&self) -> Result<(), ApiError> {
        self.validate_initialized_sql()?;
        self.proof
            .checkout
            .validate_initialized_git()
            .map_err(git_authority_error)?;
        self.validate_initialized_sql()
    }
    async fn after_initialized(&self) -> Result<(), ApiError> {
        let permit = self
            .state
            .fs_permit()
            .map_err(|error| uncertain_git(error, true))?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            worker.validate_initialized_blocking()
        })
        .await
        .map_err(|_| {
            ApiError::conflict("Git init worker stopped; inspect repository before retrying")
        })?
        .map_err(|error| uncertain_git(error, true))?;
        self.validate_initialized_sql()
            .map_err(|error| uncertain_git(error, true))
    }
    pub async fn read<
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    >(
        self,
        operation: F,
    ) -> Result<T, ApiError> {
        self.run(operation, false, false).await
    }
    pub async fn mutate<
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    >(
        self,
        operation: F,
    ) -> Result<T, ApiError> {
        self.run(operation, true, false).await
    }
    /// One terminal Directory→ordinary Git postcondition. No reusable proof is
    /// refreshed and generic operations continue to reject shape changes.
    pub async fn initialize<
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, String> + Send + 'static,
    >(
        self,
        operation: F,
    ) -> Result<T, ApiError> {
        self.run(operation, true, true).await
    }
    async fn run<T: Send + 'static, F: FnOnce(&Project) -> Result<T, String> + Send + 'static>(
        self,
        operation: F,
        mutation: bool,
        initialize: bool,
    ) -> Result<T, ApiError> {
        if mutation && self.proof.checkout.stamp().permission != MetadataPermission::Write {
            return Err(ApiError::forbidden());
        }
        let permit = self.state.fs_permit()?;
        let worker = self.clone();
        let (started, initialized, result) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Err(error) = worker.validate_blocking() {
                return (false, false, Err(error));
            }
            let outcome = operation(&worker.project);
            let initialized = initialize
                && outcome.is_ok()
                && worker.proof.checkout.stamp().repo_kind
                    == crate::checkout_identity::RepoKind::Directory;
            let post = if initialized {
                worker
                    .validate_initialized_blocking()
                    .map_err(|error| uncertain_git(error, true))
            } else {
                worker.after_blocking(mutation)
            };
            (
                true,
                initialized,
                post.and_then(|()| outcome.map_err(git_operation_error)),
            )
        })
        .await
        .map_err(|_| {
            ApiError::conflict("Git worker stopped; inspect state before a new operation")
        })?;
        if started {
            if initialized {
                self.after_initialized().await?;
            } else {
                self.after(mutation).await?;
            }
        }
        result
    }
}
