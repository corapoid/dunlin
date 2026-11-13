//! Share a journal without retaining its lock across provider HTTP or secret
//! storage callbacks. A poisoned lock never permits credential operations.
use super::{AuthContext, CredentialIntent, CredentialJournal, CredentialRecord, PersistenceError};
use std::sync::Mutex;

impl<J: CredentialJournal> CredentialJournal for Mutex<J> {
    fn active(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialRecord, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .active(context, binding)
    }
    fn begin_install(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .begin_install(context, binding)
    }
    fn begin_refresh(
        &self,
        context: &AuthContext,
        binding: &str,
        expected: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .begin_refresh(context, binding, expected)
    }
    fn prepare_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .prepare_write(c, b, i, h)
    }
    fn claim_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .claim_write(c, b, i, h)
    }
    fn acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .acknowledge_write(c, b, i, h)
    }
    fn commit(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .commit(context, binding, intent, sha256)
    }
    fn disable(&self, context: &AuthContext) -> Result<u64, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .disable(context)
    }
}
