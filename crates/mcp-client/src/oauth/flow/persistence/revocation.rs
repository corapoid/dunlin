//! Negative operations have their own journal. A reservation fences credentials
//! before secret reads or provider I/O. Uncertain operations never reactivate a
//! connection and are not automatically replayed after restart.
use super::{AuthContext, AuthorizationFlow, CredentialRecord, Error, PersistenceError};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

#[derive(Clone, PartialEq, Eq)]
pub struct RevocationTarget {
    pub context: AuthContext,
    pub record: CredentialRecord,
}
/// Host-created, non-serializable operation; never accept it from an API client.
/// None denotes a retained reference without a proven envelope/configuration.
pub struct RevocationIntent {
    pub operation_id: String,
    pub connection_revision: u64,
    pub binding: String,
    pub targets: Vec<Option<RevocationTarget>>,
    /// Frozen acknowledgements inherited from an earlier completed operation.
    pub already_confirmed: Vec<bool>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct RevocationSummary {
    pub connection_revision: u64,
    pub confirmed: usize,
    pub unconfirmed: usize,
    pub confirmed_known_tokens: bool,
}
pub trait RevocationJournal {
    fn begin_revocation(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<RevocationIntent, PersistenceError>;
    /// One-use claim, durably unknown BEFORE any secret read or HTTP request.
    fn claim_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    /// Fresh owner/configuration/operation authority before and after each POST.
    fn check_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    fn confirm_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError>;
    fn finish_revocation(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
    ) -> Result<RevocationSummary, PersistenceError>;
}
impl AuthorizationFlow {
    /// Explicit, bounded negative operation. Only exact, journal-bound envelopes
    /// are sent to the frozen AS. Failure of one target does not replay it or
    /// suppress cleanup of other known targets. This confirms known tokens,
    /// never an entire grant or tokens lost in a rotating refresh response.
    pub fn revoke_durable(
        &self,
        context: &AuthContext,
        journal: &impl RevocationJournal,
        get: impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
    ) -> Result<RevocationSummary, PersistenceError> {
        let intent = journal.begin_revocation(context, &self.credential_binding())?;
        self.continue_revocation(context, journal, &intent, get)
    }

    /// Continue only the caller's live, host-created reservation. A restarted
    /// server must expose its receipt, never reconstruct and replay this call.
    pub fn continue_revocation(
        &self,
        context: &AuthContext,
        journal: &impl RevocationJournal,
        intent: &RevocationIntent,
        mut get: impl FnMut(&str) -> Result<Zeroizing<String>, PersistenceError>,
    ) -> Result<RevocationSummary, PersistenceError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        if intent.binding != self.credential_binding()
            || intent.targets.len() > 256
            || intent.already_confirmed.len() != intent.targets.len()
        {
            return Err(PersistenceError::Conflict);
        }
        for (index, target) in intent.targets.iter().enumerate() {
            if intent.already_confirmed[index] {
                continue;
            }
            if Instant::now() >= deadline {
                break;
            }
            journal.claim_revocation_target(context, intent, index)?;
            let Some(target) = target else { continue };
            journal.check_revocation_target(context, intent, index)?;
            let result = (|| {
                let value = get(&target.record.secret_reference)?;
                let mut token = self.decode(&target.context, &target.record, &value)?;
                self.revoke_checked(&target.context, &mut token, || {
                    if Instant::now() >= deadline {
                        return Err(Error::Unauthorized);
                    }
                    journal
                        .check_revocation_target(context, intent, index)
                        .map_err(|_| Error::Unauthorized)
                })?;
                Ok::<_, PersistenceError>(())
            })();
            if result.is_ok() {
                journal.confirm_revocation_target(context, intent, index)?;
            }
        }
        journal.finish_revocation(context, intent)
    }
}

impl<J: RevocationJournal> RevocationJournal for std::sync::Mutex<J> {
    fn begin_revocation(
        &self,
        c: &AuthContext,
        b: &str,
    ) -> Result<RevocationIntent, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .begin_revocation(c, b)
    }
    fn claim_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .claim_revocation_target(c, i, n)
    }
    fn check_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .check_revocation_target(c, i, n)
    }
    fn confirm_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .confirm_revocation_target(c, i, n)
    }
    fn finish_revocation(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
    ) -> Result<RevocationSummary, PersistenceError> {
        self.lock()
            .map_err(|_| PersistenceError::Storage)?
            .finish_revocation(c, i)
    }
}
