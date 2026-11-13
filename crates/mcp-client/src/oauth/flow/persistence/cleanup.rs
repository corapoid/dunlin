//! Explicit negative sweeps. Uncertain references remain sweepable; host
//! retention may prune only after independent writer/write/delete proofs.
use super::{AuthContext, PersistenceError};
use std::time::{Duration, Instant};

/// Host-created reservation, never deserializable from a browser or runner.
pub struct CleanupIntent {
    pub operation_id: String,
    pub revision: u64,
    pub connection_revision: u64,
    pub references: Vec<String>,
}
#[derive(Clone)]
pub struct CleanupSummary {
    pub revision: u64,
    pub connection_revision: u64,
    pub delete_acknowledged: usize,
    pub unconfirmed: usize,
    pub reclaimed: usize,
    pub pruned: usize,
}
pub trait CleanupJournal {
    fn begin_cleanup(
        &self,
        context: &AuthContext,
        expected_cleanup_revision: u64,
    ) -> Result<CleanupIntent, PersistenceError>;
    fn claim_cleanup_target(
        &self,
        context: &AuthContext,
        intent: &CleanupIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    fn check_cleanup_target(
        &self,
        context: &AuthContext,
        intent: &CleanupIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    fn acknowledge_cleanup_target(
        &self,
        context: &AuthContext,
        intent: &CleanupIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    fn finish_cleanup(
        &self,
        context: &AuthContext,
        intent: &CleanupIntent,
    ) -> Result<CleanupSummary, PersistenceError>;
}
/// A fresh explicit sweep retries every eligible reference, including previously
/// acknowledged deletes. It performs no reads of credential plaintext or AS I/O.
pub fn cleanup_credentials(
    context: &AuthContext,
    journal: &impl CleanupJournal,
    expected_cleanup_revision: u64,
    mut delete: impl FnMut(&str) -> Result<(), PersistenceError>,
) -> Result<CleanupSummary, PersistenceError> {
    let intent = journal.begin_cleanup(context, expected_cleanup_revision)?;
    if intent.references.is_empty() || intent.references.len() > 256 {
        return Err(PersistenceError::Conflict);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    for (index, reference) in intent.references.iter().enumerate() {
        if Instant::now() >= deadline {
            break;
        }
        journal.claim_cleanup_target(context, &intent, index)?;
        journal.check_cleanup_target(context, &intent, index)?;
        if delete(reference).is_ok() {
            journal.check_cleanup_target(context, &intent, index)?;
            journal.acknowledge_cleanup_target(context, &intent, index)?;
        }
    }
    journal.finish_cleanup(context, &intent)
}
impl<J: CleanupJournal> CleanupJournal for std::sync::Mutex<J> {
    fn begin_cleanup(&self, c: &AuthContext, r: u64) -> Result<CleanupIntent, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .begin_cleanup(c, r)
    }
    fn claim_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .claim_cleanup_target(c, i, n)
    }
    fn check_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .check_cleanup_target(c, i, n)
    }
    fn acknowledge_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .acknowledge_cleanup_target(c, i, n)
    }
    fn finish_cleanup(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
    ) -> Result<CleanupSummary, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .finish_cleanup(c, i)
    }
}
