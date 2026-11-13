//! Private OAuth persistence through the host's SecretStore. Records have their
//! own reference prefix and are never registered as vault tools or policies.
//! These synchronous methods belong on the application's blocking worker pool.
use crate::store::{SecretStore, ServiceKeychainStore};
use dunlin_mcp_client::oauth::{
    AuthContext, AuthorizationFlow, TokenSet,
    persistence::{CredentialJournal, CredentialRecord, PersistenceError},
};
use std::sync::Arc;
use zeroize::Zeroizing;

pub struct OAuthCredentials<J> {
    journal: Arc<J>,
    secrets: Arc<dyn SecretStore>,
}
impl<J: CredentialJournal> OAuthCredentials<J> {
    /// Production relay namespace. Construction performs no keyring operations.
    pub fn relay(journal: Arc<J>) -> Self {
        Self::new(journal, Arc::new(ServiceKeychainStore::relay()))
    }
    /// Explicit secret backend for an embedded service or isolated tests.
    pub fn new(journal: Arc<J>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { journal, secrets }
    }
    fn read(&self, reference: &str) -> Result<Zeroizing<String>, PersistenceError> {
        reference_id(reference)?;
        let value = self
            .secrets
            .get(reference)
            .map_err(|_| PersistenceError::Storage)?;
        if value.len() > 65536 {
            return Err(PersistenceError::Limit);
        }
        Ok(value)
    }
    fn write(&self, reference: &str, value: &str) -> Result<(), PersistenceError> {
        reference_id(reference)?;
        if value.len() > 65536 {
            return Err(PersistenceError::Limit);
        }
        // The journal reserves a unique reference before this call. Never use
        // a user-provided alias or reuse the previous generation's reference.
        self.secrets
            .set(reference, value)
            .map_err(|_| PersistenceError::Storage)
    }
    pub fn install(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
        token: TokenSet,
    ) -> Result<CredentialRecord, PersistenceError> {
        flow.install_durable(context, token, self.journal.as_ref(), |id, value| {
            self.write(id, value)
        })
    }
    pub fn refresh(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
    ) -> Result<CredentialRecord, PersistenceError> {
        flow.refresh_durable(
            context,
            self.journal.as_ref(),
            |id| self.read(id),
            |id, value| self.write(id, value),
        )
    }
    /// Only the broker receives this value, immediately before provider dispatch.
    /// Never return it to a runner, browser, transcript, or application log.
    pub fn access_token(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
    ) -> Result<Zeroizing<String>, PersistenceError> {
        flow.access_token_durable(context, self.journal.as_ref(), |id| self.read(id))
    }
    /// Dispatch preparation; an uncertain refresh remains durably blocked.
    pub fn access_token_or_refresh(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
    ) -> Result<(CredentialRecord, Zeroizing<String>), PersistenceError> {
        flow.access_token_or_refresh_durable(
            context,
            self.journal.as_ref(),
            |id| self.read(id),
            |id, value| self.write(id, value),
        )
    }
    /// Local disconnect is durable and fences outstanding writers. Provider
    /// revocation and secret cleanup are separate, explicit operations.
    pub fn disable(&self, context: &AuthContext) -> Result<u64, PersistenceError> {
        self.journal.disable(context)
    }
}
fn reference_id(reference: &str) -> Result<(), PersistenceError> {
    let id = reference
        .strip_prefix("mcp-oauth:")
        .ok_or(PersistenceError::Storage)?;
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
    {
        return Err(PersistenceError::Storage);
    }
    Ok(())
}

impl<J: CredentialJournal + dunlin_mcp_client::oauth::persistence::RevocationJournal>
    OAuthCredentials<J>
{
    /// Explicit provider revoke; secret records and their tombstones are retained
    /// for subsequent local cleanup, including any late fenced writer.
    pub fn revoke(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
    ) -> Result<dunlin_mcp_client::oauth::persistence::RevocationSummary, PersistenceError> {
        flow.revoke_durable(context, self.journal.as_ref(), |id| self.read(id))
    }
}

impl<J: CredentialJournal + dunlin_mcp_client::oauth::persistence::RevocationJournal>
    OAuthCredentials<J>
{
    pub fn continue_revocation(
        &self,
        flow: &AuthorizationFlow,
        context: &AuthContext,
        intent: &dunlin_mcp_client::oauth::persistence::RevocationIntent,
    ) -> Result<dunlin_mcp_client::oauth::persistence::RevocationSummary, PersistenceError> {
        flow.continue_revocation(context, self.journal.as_ref(), intent, |id| self.read(id))
    }
}

impl<J: CredentialJournal + dunlin_mcp_client::oauth::persistence::CleanupJournal>
    OAuthCredentials<J>
{
    /// A delete acknowledgement is scoped to this sweep. The journal keeps
    /// tombstones so a late write can be removed by another explicit sweep.
    pub fn cleanup(
        &self,
        context: &AuthContext,
        expected_cleanup_revision: u64,
    ) -> Result<dunlin_mcp_client::oauth::persistence::CleanupSummary, PersistenceError> {
        dunlin_mcp_client::oauth::persistence::cleanup_credentials(
            context,
            self.journal.as_ref(),
            expected_cleanup_revision,
            |id| {
                reference_id(id)?;
                self.secrets
                    .delete(id)
                    .map_err(|_| PersistenceError::Storage)
            },
        )
    }
}

#[cfg(test)]
mod tests;
