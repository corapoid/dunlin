//! Host-owned live authority for a single credential request. This is never
//! deserialized from an agent or runner message and never carries secret values.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct LiveAccess(Arc<State>);

struct State {
    deadline: Instant,
    denied: AtomicBool,
    authorize: Box<dyn Fn() -> bool + Send + Sync>,
}

impl LiveAccess {
    pub fn new(deadline: Instant, authorize: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(State {
            deadline,
            denied: AtomicBool::new(false),
            authorize: Box::new(authorize),
        }))
    }

    /// A failed check is terminal, including expiry. Renewing a grant cannot
    /// revive an operation that already observed its revocation.
    pub fn allowed(&self) -> bool {
        if self.0.denied.load(Ordering::Acquire) {
            return false;
        }
        if Instant::now() >= self.0.deadline
            || !(self.0.authorize)()
            || Instant::now() >= self.0.deadline
        {
            self.revoke();
        }
        !self.0.denied.load(Ordering::Acquire)
    }

    pub fn revoke(&self) {
        self.0.denied.store(true, Ordering::Release);
    }

    /// Remaining I/O budget, never a substitute for `allowed`.
    pub fn remaining(&self) -> Duration {
        self.0.deadline.saturating_duration_since(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn denial_and_expiry_are_terminal_across_clones() {
        let granted = Arc::new(AtomicBool::new(true));
        let current = granted.clone();
        let access = LiveAccess::new(Instant::now() + Duration::from_secs(10), move || {
            current.load(Ordering::Acquire)
        });
        let copy = access.clone();
        assert!(access.allowed());
        granted.store(false, Ordering::Release);
        assert!(!copy.allowed());
        granted.store(true, Ordering::Release);
        assert!(!access.allowed());
        assert!(!LiveAccess::new(Instant::now(), || true).allowed());
    }
}
