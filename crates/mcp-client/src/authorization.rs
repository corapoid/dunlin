//! Application authority, independent of provider metadata and OAuth scopes.
use crate::Error;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A denial permanently fences this client, including cached reads. The
/// callback must load current authority without holding a lock across HTTP.
#[derive(Clone)]
pub struct DispatchGuard {
    check: Arc<dyn Fn() -> bool + Send + Sync>,
    denied: Arc<AtomicBool>,
}
impl DispatchGuard {
    pub fn new(check: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            check,
            denied: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.denied.load(Ordering::Acquire) || !(self.check)() {
            self.denied.store(true, Ordering::Release);
            return Err(Error::Unauthorized);
        }
        if self.denied.load(Ordering::Acquire) {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}
pub(crate) fn authorize(guard: Option<&DispatchGuard>) -> Result<(), Error> {
    guard.map_or(Ok(()), DispatchGuard::check)
}
