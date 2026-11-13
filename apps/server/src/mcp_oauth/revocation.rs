//! Local fencing precedes discovery, keyring reads, and provider revocation.
use super::{CatalogScope, Service, journal::BrowserJournal};
use crate::api::AppState;
use dunlin_mcp_client::oauth::persistence::{
    PersistenceError, RevocationJournal, RevocationSummary,
};
use dunlin_vault::oauth::OAuthCredentials;
use std::sync::Arc;
impl Service {
    pub(crate) fn revoke_connection(
        &self,
        state: AppState,
        scope: CatalogScope,
    ) -> Result<RevocationSummary, PersistenceError> {
        let context = scope.context;
        let journal = Arc::new(BrowserJournal {
            state,
            proof: scope.proof,
            membership_revision: Some(scope.membership_revision),
        });
        let (binding, _lease) = {
            let store = journal
                .state
                .store
                .lock()
                .map_err(|_| PersistenceError::Storage)?;
            journal.validate(&store, &context)?;
            let row = store
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id,
                )?
                .ok_or(PersistenceError::Unauthorized)?;
            if row.provider_id.as_deref() != Some(scope.provider.id.as_str()) {
                return Err(PersistenceError::Conflict);
            }
            (
                store.mcp_oauth_credential_binding(&context)?,
                store.mcp_oauth_revocation_lease()?,
            )
        };
        let intent = journal.begin_revocation(&context, &binding)?;
        let flow = scope.provider.flow(self.secrets.as_ref());
        match flow {
            Ok(flow) if flow.credential_binding() == binding => {
                OAuthCredentials::new(journal.clone(), self.secrets.clone())
                    .continue_revocation(&flow, &context, &intent)
            }
            _ => journal.finish_revocation(&context, &intent),
        }
    }
}

pub(crate) struct CleanupScope {
    pub context: dunlin_mcp_client::oauth::AuthContext,
    pub proof: super::BrowserProof,
    pub membership_revision: u64,
}
impl Service {
    pub(crate) fn cleanup_connection(
        &self,
        state: AppState,
        scope: CleanupScope,
        expected: u64,
    ) -> Result<dunlin_mcp_client::oauth::persistence::CleanupSummary, PersistenceError> {
        let journal = Arc::new(BrowserJournal {
            state,
            proof: scope.proof,
            membership_revision: Some(scope.membership_revision),
        });
        let _writer = journal.writer_lease(&scope.context)?;
        OAuthCredentials::new(journal, self.secrets.clone()).cleanup(&scope.context, expected)
    }
}
