//! Host-only retirement of an immutable envelope whose single writer completed.
use super::*;
/// A durable cleanup intent. It implements neither Deserialize nor Clone.
/// ```compile_fail
/// use dunlin_vault::scoped::CleanupClaim;
/// let _: CleanupClaim = serde_json::from_str("{}").unwrap();
/// ```
pub struct CleanupClaim {
    version: Version,
    nonce: String,
}
impl CleanupClaim {
    pub fn reserved(version: Version, nonce: String) -> Result<Self> {
        version.binding.validate()?;
        if !token(&version.credential)
            || (version.revision == 0 || version.revision > 9_007_199_254_740_991)
            || !token(&nonce)
            || version
                .reference
                .strip_prefix("scoped-secret:")
                .is_none_or(|r| {
                    r.len() != 64
                        || !r
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                })
            || version.sha256.len() != 64
            || !version
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Error::Invalid);
        }
        Ok(Self { version, nonce })
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
}
pub trait CleanupJournal: Journal {
    /// Revalidate the manager and exact retirement intent before keyring I/O.
    /// Only a completed single writer and permanently unpublished reference qualify.
    fn check_cleanup(&self, claim: &CleanupClaim) -> Result<()>;
    /// Preserve the factual delete ACK even if the requesting browser logged out.
    fn acknowledge_cleanup(&self, claim: &CleanupClaim) -> Result<()>;
}
impl<J: CleanupJournal> Credentials<J> {
    pub fn cleanup(&self, claim: CleanupClaim) -> Result<()> {
        self.journal.check_cleanup(&claim)?;
        self.secrets
            .delete(&claim.version.reference)
            .map_err(|_| Error::OutcomeUnknown)?;
        self.journal
            .acknowledge_cleanup(&claim)
            .map_err(|_| Error::OutcomeUnknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Default)]
    struct JournalFixture {
        allowed: AtomicBool,
        fail_ack: AtomicBool,
    }
    impl Journal for JournalFixture {
        fn reserve(&self, _: &Candidate) -> Result<WriteClaim> {
            Err(Error::Denied)
        }
        fn acknowledge(&self, _: &WriteClaim) -> Result<()> {
            Err(Error::Denied)
        }
        fn activate(&self, _: WriteClaim) -> Result<Version> {
            Err(Error::Denied)
        }
    }
    impl CleanupJournal for JournalFixture {
        fn check_cleanup(&self, _: &CleanupClaim) -> Result<()> {
            if self.allowed.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(Error::Denied)
            }
        }
        fn acknowledge_cleanup(&self, _: &CleanupClaim) -> Result<()> {
            if self.fail_ack.load(Ordering::Acquire) {
                Err(Error::Storage)
            } else {
                Ok(())
            }
        }
    }
    fn claim() -> CleanupClaim {
        CleanupClaim::reserved(
            Version {
                credential: "api".into(),
                binding: Binding {
                    workspace: "w".into(),
                    project: "p".into(),
                    host: "h".into(),
                },
                revision: 1,
                reference: format!("scoped-secret:{}", "a".repeat(64)),
                sha256: "b".repeat(64),
            },
            "intent".into(),
        )
        .unwrap()
    }
    #[test]
    fn cleanup_requires_live_journal_and_lost_ack_is_unknown_even_after_effect() {
        let journal = Arc::new(JournalFixture::default());
        let store = Arc::new(MemoryStore::default());
        let reference = claim().version.reference.clone();
        store.set(&reference, "private-envelope").unwrap();
        let credentials = Credentials::new(journal.clone(), store.clone());
        assert_eq!(credentials.cleanup(claim()), Err(Error::Denied));
        assert!(store.get(&reference).is_ok());
        journal.allowed.store(true, Ordering::Release);
        journal.fail_ack.store(true, Ordering::Release);
        assert_eq!(credentials.cleanup(claim()), Err(Error::OutcomeUnknown));
        assert!(store.get(&reference).is_err());
        journal.fail_ack.store(false, Ordering::Release);
        assert_eq!(credentials.cleanup(claim()), Ok(()));
    }
}
