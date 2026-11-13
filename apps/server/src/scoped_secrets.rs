//! Host-owned scoped-secret provisioning. Browser identity is revalidated under
//! each SQL transaction; keyring I/O runs outside Store on bounded workers.
use crate::{
    api::AppState,
    store::{Store, scoped_secrets::Proof},
};
use dunlin_domain::vault::SecretPolicy;
use dunlin_vault::{
    scoped::{Candidate, Credentials, Error, Journal, Result, Version, WriteClaim},
    store::SecretStore,
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub(crate) struct Install {
    pub credential: String,
    pub expected_revision: u64,
    pub policy: SecretPolicy,
    pub value: Zeroizing<String>,
}
pub(crate) struct Service {
    slots: Arc<Semaphore>,
    backend: Option<Arc<dyn SecretStore>>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            backend: None,
        }
    }
}
impl Service {
    #[cfg(test)]
    pub(crate) fn testing(backend: Arc<dyn SecretStore>) -> Self {
        Self {
            backend: Some(backend),
            ..Self::default()
        }
    }
    pub(crate) async fn install(
        &self,
        state: AppState,
        proof: Proof,
        request: Install,
    ) -> Result<Version> {
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Limit)?;
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            // Retain capacity even if the HTTP waiter disconnects/cancels.
            let _slot = slot;
            let journal = Arc::new(BrowserJournal {
                state,
                proof: proof.clone(),
            });
            let credentials = match backend {
                Some(backend) => Credentials::new(journal, backend),
                None => Credentials::relay(journal),
            };
            credentials.install(
                &request.credential,
                proof.binding,
                request.expected_revision,
                request.policy,
                request.value,
            )
        })
        .await
        .map_err(|_| Error::WriteUnknown)?
    }
}
pub(crate) fn identity(state: &AppState, store: &Store, proof: &Proof) -> Result<()> {
    browser_identity(state, store, &proof.browser)
}
pub(crate) fn browser_identity(
    state: &AppState,
    store: &Store,
    browser: &crate::mcp_oauth::BrowserProof,
) -> Result<()> {
    browser
        .validate_identity(state, store)
        .map_err(|error| match error {
            dunlin_mcp_client::oauth::persistence::PersistenceError::Unauthorized => Error::Denied,
            _ => Error::Storage,
        })
}
struct BrowserJournal {
    state: AppState,
    proof: Proof,
}
impl Journal for BrowserJournal {
    fn reserve(&self, candidate: &Candidate) -> Result<WriteClaim> {
        let store = self.state.store.lock().map_err(|_| Error::Storage)?;
        store.scoped_secret_reserve(&self.proof, candidate, &|store| {
            identity(&self.state, store, &self.proof)
        })
    }
    fn acknowledge(&self, claim: &WriteClaim) -> Result<()> {
        self.state
            .store
            .lock()
            .map_err(|_| Error::Storage)?
            .scoped_secret_acknowledge(&self.proof, claim)
    }
    fn activate(&self, claim: WriteClaim) -> Result<Version> {
        let store = self.state.store.lock().map_err(|_| Error::Storage)?;
        store.scoped_secret_activate(&self.proof, claim, &|store| {
            identity(&self.state, store, &self.proof)
        })
    }
}
