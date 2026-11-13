//! Explicit unsupported platform; never substitutes an unsandboxed executor.
use super::*;
#[derive(Clone)]
pub(super) struct Platform;
impl Platform {
    pub(super) fn preflight(_: &RuntimeConfig) -> std::io::Result<Self> {
        Err(unsupported())
    }
    pub(super) fn program_directory(&self) -> PathBuf {
        PathBuf::from("/")
    }
    pub(super) fn observation(&self) -> (&str, &str) {
        ("unsupported", "unsupported")
    }
}
fn unsupported() -> std::io::Error {
    std::io::Error::other("Worker execution requires the verified Linux containment adapter")
}
pub(super) fn monotonic_ns() -> std::io::Result<u64> {
    Err(unsupported())
}
#[derive(Default)]
pub(super) struct Group;
impl Group {
    pub fn terminate(&self) {}
    pub fn guard_exited(&self) -> bool {
        false
    }
    pub fn renew(&self, _: u64) -> std::io::Result<()> {
        Err(unsupported())
    }
}
pub(super) struct SessionOwner {
    pub identity: wire::session::SessionIdentity,
    pub fence: wire::session::SessionFence,
    pub group: Arc<Group>,
}
pub(super) struct StopProof;
impl StopProof {
    pub fn attest(&self, _: &wire::AttemptFence) -> local::Result<()> {
        Err(local::Error::Denied)
    }
    pub fn never_spawned(_: wire::AttemptFence) -> Self {
        Self
    }
    pub fn attest_session(&self, _: &wire::session::SessionFence) -> local::Result<()> {
        Err(local::Error::Denied)
    }
    pub fn never_spawned_session(_: wire::session::SessionFence) -> Self {
        Self
    }
    pub fn recovered(&self) -> bool {
        true
    }
}
pub(super) struct Owned {
    pub child: tokio::process::Child,
}
impl Owned {
    pub fn spawn(
        _: &Platform,
        _: &RuntimeConfig,
        _: tokio::process::Command,
        _: &Active,
    ) -> std::io::Result<Self> {
        Err(unsupported())
    }
    pub fn spawn_session(
        _: &Platform,
        _: &RuntimeConfig,
        _: tokio::process::Command,
        _: SessionOwner,
        _: impl FnOnce() -> local::Result<(Duration, Duration)>,
    ) -> std::io::Result<Self> {
        Err(unsupported())
    }
    pub async fn ready(&mut self, _: &Connected) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub fn release(&mut self) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub async fn stop(&mut self) -> std::io::Result<StopProof> {
        Err(unsupported())
    }
    pub fn finalized(&mut self) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub async fn reap(&mut self) -> std::io::Result<()> {
        Err(unsupported())
    }
}
pub(super) fn guard_command(_: &[String]) -> Result<(), String> {
    Err(unsupported().to_string())
}
