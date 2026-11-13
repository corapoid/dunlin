//! The callback's cookie proves possession of its flow. Original browser session
//! and device authority are reread before provider exchange and every journal op.
use crate::{api::AppState, store::Store};
use dunlin_mcp_client::oauth::{
    AuthContext,
    persistence::{CredentialIntent, CredentialJournal, CredentialRecord, PersistenceError},
};
#[derive(Clone)]
pub(crate) struct BrowserProof {
    pub user: String,
    pub device: String,
    pub session_hash: String,
}
impl BrowserProof {
    pub fn validate(
        &self,
        state: &AppState,
        store: &Store,
        context: &AuthContext,
    ) -> Result<(), PersistenceError> {
        if context.user_id != self.user {
            return Err(PersistenceError::Unauthorized);
        }
        self.validate_identity(state, store)
    }
    pub(crate) fn validate_identity(
        &self,
        state: &AppState,
        store: &Store,
    ) -> Result<(), PersistenceError> {
        let device = store
            .browser_session_device_hash(&self.session_hash)
            .map_err(|_| PersistenceError::Storage)?
            .ok_or(PersistenceError::Unauthorized)?;
        if device.id != self.device || device.user_id != self.user {
            return Err(PersistenceError::Unauthorized);
        }
        let user = store
            .active_device_user(&device.id)
            .map_err(|_| PersistenceError::Storage)?
            .ok_or(PersistenceError::Unauthorized)?;
        crate::api::check_host_account_policy(state, &user)
            .map_err(|_| PersistenceError::Unauthorized)?;
        if !store
            .oidc_device_allowed(&device.id, state.config.oidc.as_ref())
            .map_err(|_| PersistenceError::Storage)?
        {
            return Err(PersistenceError::Unauthorized);
        }
        Ok(())
    }
}
pub(crate) struct BrowserJournal {
    pub state: AppState,
    pub proof: BrowserProof,
    pub membership_revision: Option<u64>,
}
impl BrowserJournal {
    pub(super) fn writer_lease(
        &self,
        context: &AuthContext,
    ) -> Result<crate::store::mcp_oauth::WriterLease, PersistenceError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::time::Instant::now() >= deadline {
                return Err(PersistenceError::Conflict);
            }
            let result = {
                let store = self
                    .state
                    .store
                    .lock()
                    .map_err(|_| PersistenceError::Storage)?;
                self.validate(&store, context)?;
                store.mcp_oauth_writer_lease()
            };
            match result {
                Err(PersistenceError::Conflict) if std::time::Instant::now() < deadline => {
                    // Admission only, never a repeat of code/refresh or a vault
                    // write. Release Store before waiting for the live worker.
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }
    pub(super) fn validate(
        &self,
        store: &Store,
        context: &AuthContext,
    ) -> Result<(), PersistenceError> {
        self.proof.validate(&self.state, store, context)?;
        if let Some(revision) = self.membership_revision {
            let member = store
                .team_membership(&context.workspace_id, &context.user_id)
                .map_err(|_| PersistenceError::Storage)?
                .ok_or(PersistenceError::Unauthorized)?;
            if !member.active || member.revision != revision {
                return Err(PersistenceError::Unauthorized);
            }
        }
        Ok(())
    }
    fn scoped<T>(
        &self,
        operation: impl FnOnce(&dyn CredentialJournal) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?;
        let check = |store: &Store, context: &AuthContext| self.validate(store, context);
        let journal = store.mcp_oauth_journal(&check);
        operation(&journal)
    }
    pub fn authorizing(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<(), PersistenceError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?;
        store.mcp_oauth_authorizing_checked(context, binding, &|store, context| {
            self.validate(store, context)
        })
    }
}
impl CredentialJournal for BrowserJournal {
    fn active(&self, c: &AuthContext, b: &str) -> Result<CredentialRecord, PersistenceError> {
        self.scoped(|s| s.active(c, b))
    }
    fn begin_install(
        &self,
        c: &AuthContext,
        b: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        self.scoped(|s| s.begin_install(c, b))
    }
    fn begin_refresh(
        &self,
        c: &AuthContext,
        b: &str,
        r: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        self.scoped(|s| s.begin_refresh(c, b, r))
    }
    fn prepare_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.scoped(|s| s.prepare_write(c, b, i, h))
    }
    fn claim_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.scoped(|s| s.claim_write(c, b, i, h))
    }
    fn acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        if self.proof.user != c.user_id {
            return Err(PersistenceError::Unauthorized);
        }
        self.state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?
            .mcp_oauth_acknowledge_write(c, b, i, h)
    }
    fn commit(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.scoped(|s| s.commit(c, b, i, h))
    }
    fn disable(&self, c: &AuthContext) -> Result<u64, PersistenceError> {
        self.scoped(|s| s.disable(c))
    }
}

impl BrowserJournal {
    fn negative<T>(
        &self,
        operation: impl FnOnce(
            &dyn dunlin_mcp_client::oauth::persistence::RevocationJournal,
        ) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?;
        let check = |store: &Store, c: &AuthContext| self.validate(store, c);
        let journal = store.mcp_oauth_journal(&check);
        operation(&journal)
    }
}
impl dunlin_mcp_client::oauth::persistence::RevocationJournal for BrowserJournal {
    fn begin_revocation(
        &self,
        c: &AuthContext,
        b: &str,
    ) -> Result<dunlin_mcp_client::oauth::persistence::RevocationIntent, PersistenceError> {
        self.negative(|j| j.begin_revocation(c, b))
    }
    fn claim_revocation_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.negative(|j| j.claim_revocation_target(c, i, n))
    }
    fn check_revocation_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.negative(|j| j.check_revocation_target(c, i, n))
    }
    fn confirm_revocation_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.negative(|j| j.confirm_revocation_target(c, i, n))
    }
    fn finish_revocation(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::RevocationIntent,
    ) -> Result<dunlin_mcp_client::oauth::persistence::RevocationSummary, PersistenceError> {
        self.negative(|j| j.finish_revocation(c, i))
    }
}

impl BrowserJournal {
    fn cleanup_journal<T>(
        &self,
        operation: impl FnOnce(
            &dyn dunlin_mcp_client::oauth::persistence::CleanupJournal,
        ) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?;
        let check = |store: &Store, c: &AuthContext| self.validate(store, c);
        let journal = store.mcp_oauth_journal(&check);
        operation(&journal)
    }
}
impl dunlin_mcp_client::oauth::persistence::CleanupJournal for BrowserJournal {
    fn begin_cleanup(
        &self,
        c: &AuthContext,
        r: u64,
    ) -> Result<dunlin_mcp_client::oauth::persistence::CleanupIntent, PersistenceError> {
        self.cleanup_journal(|j| j.begin_cleanup(c, r))
    }
    fn claim_cleanup_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.cleanup_journal(|j| j.claim_cleanup_target(c, i, n))
    }
    fn check_cleanup_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.cleanup_journal(|j| j.check_cleanup_target(c, i, n))
    }
    fn acknowledge_cleanup_target(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.cleanup_journal(|j| j.acknowledge_cleanup_target(c, i, n))
    }
    fn finish_cleanup(
        &self,
        c: &AuthContext,
        i: &dunlin_mcp_client::oauth::persistence::CleanupIntent,
    ) -> Result<dunlin_mcp_client::oauth::persistence::CleanupSummary, PersistenceError> {
        self.cleanup_journal(|j| j.finish_cleanup(c, i))
    }
}
