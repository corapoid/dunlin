//! Pluggable persistence for stable per-host `clientId`s.
//!
//! On [`super::MultiHostClient::add_host`], the multi-host client looks
//! up the host id in the configured [`ClientIdStore`]. If the store has
//! a value, that id is reused so the server can treat successive
//! launches as the same client (which the AHP `reconnect` flow needs to
//! replay missed actions). If the store has no entry, the client either
//! uses the explicit value from [`super::HostConfig::with_client_id`] or
//! generates a fresh UUID — and persists the resolved id back to the
//! store.
//!
//! The default [`InMemoryClientIdStore`] is session-stable only — it
//! does not survive process restarts. Production multi-host apps should
//! plug in a persistent implementation, either the bundled
//! [`FileClientIdStore`] (filesystem-backed) or a keychain / secure
//! enclave wrapper of their own.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::Mutex;

use super::types::HostId;

/// Persistence hook for stable `clientId`s per host.
///
/// Implementations must be safe to share across threads and the
/// supervisor task. The trait uses [`Pin<Box<dyn Future>>`](Pin) (rather
/// than `async fn`) so trait objects (`Arc<dyn ClientIdStore>`) remain
/// dyn-compatible — matching the pattern used by
/// [`super::HostTransportFactory`].
///
/// Errors bubble up as [`super::HostError::ClientIdStore`] from
/// [`super::MultiHostClient::add_host`] so persistent stores can fail
/// loudly instead of silently dropping ids.
pub trait ClientIdStore: Send + Sync + 'static {
    /// Look up the previously stored `clientId` for `host_id`, if any.
    fn load(
        &self,
        host_id: HostId,
    ) -> Pin<Box<dyn Future<Output = io::Result<Option<String>>> + Send + '_>>;

    /// Persist `client_id` for `host_id`. Implementations must
    /// overwrite any previous value.
    fn store(
        &self,
        host_id: HostId,
        client_id: String,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>>;
}

/// In-process [`ClientIdStore`] backed by a mutex-protected map.
///
/// Keeps assigned ids in memory. Survives reconnects within the same
/// process but **not** restarts. Fine for tests, ephemeral CLIs, and
/// as a starting point — production apps should provide a persistent
/// implementation (filesystem via [`FileClientIdStore`], keychain via
/// a user-supplied wrapper, …).
#[derive(Debug, Default)]
pub struct InMemoryClientIdStore {
    inner: Arc<Mutex<HashMap<HostId, String>>>,
}

impl InMemoryClientIdStore {
    /// Build an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClientIdStore for InMemoryClientIdStore {
    fn load(
        &self,
        host_id: HostId,
    ) -> Pin<Box<dyn Future<Output = io::Result<Option<String>>> + Send + '_>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let map = inner.lock().await;
            Ok(map.get(&host_id).cloned())
        })
    }

    fn store(
        &self,
        host_id: HostId,
        client_id: String,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let mut map = inner.lock().await;
            map.insert(host_id, client_id);
            Ok(())
        })
    }
}

/// Filesystem-backed [`ClientIdStore`] that survives process restarts.
///
/// Stores one file per host id under a configurable directory. Writes
/// go through a unique-per-invocation temp file that is created with
/// `0o600` mode on Unix from the start (no permissions window) and is
/// then atomically renamed into place. Per-store mutations are
/// serialized through an internal mutex so concurrent `load`/`store`
/// calls within the same process don't race on the directory's
/// contents.
///
/// **Cross-process behavior is last-writer-wins.** Two processes
/// sharing the same store can both miss on `load`, generate different
/// ids, and clobber each other on `store`. The same caveat applies to
/// the Swift `FileClientIdStore`; applications that need stronger
/// cross-process semantics should layer their own file lock (e.g.
/// `flock`) or move to a transactional backend.
///
/// **Keychain note (Apple platforms only):** the SDK does not ship a
/// Keychain-backed store to keep the crate dependency-free across
/// platforms. Wrap a Keychain implementation in your own
/// [`ClientIdStore`] impl if you need that profile.
///
/// The directory is created on first write if it doesn't already
/// exist. Filenames are derived from each host id via a percent-encoding
/// helper so arbitrary [`HostId`] strings (including `:`, `/`, etc.)
/// map to safe filesystem paths.
#[derive(Debug)]
pub struct FileClientIdStore {
    directory: PathBuf,
    /// In-process serialization. Cross-process races are unavoidable
    /// without an OS file lock; see the type docs.
    guard: Mutex<()>,
}

impl FileClientIdStore {
    /// Build a store rooted at `directory`.
    ///
    /// The directory is created on first `store` call; the caller is
    /// responsible for picking a location the process can write to
    /// (e.g. `$XDG_DATA_HOME/<app>/client-ids` on Linux,
    /// `Application Support/<app>/client-ids` on macOS, or
    /// `%APPDATA%\\<app>\\client-ids` on Windows — `<app>` is your
    /// product's directory name).
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            guard: Mutex::new(()),
        }
    }

    fn file_path(&self, host_id: &HostId) -> PathBuf {
        self.directory
            .join(format!("{}.clientid", encode_host_id(host_id.as_str())))
    }
}

impl ClientIdStore for FileClientIdStore {
    fn load(
        &self,
        host_id: HostId,
    ) -> Pin<Box<dyn Future<Output = io::Result<Option<String>>> + Send + '_>> {
        Box::pin(async move {
            let _guard = self.guard.lock().await;
            let path = self.file_path(&host_id);
            // Offload the blocking read so we don't tie up the async
            // executor thread on slow/distributed filesystems.
            tokio::task::spawn_blocking(move || match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    // Preserve the exact bytes we wrote. `store` writes
                    // raw bytes with no newline, so any leading/trailing
                    // whitespace is intentional. Treat only a fully
                    // empty file as "no value".
                    if contents.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(contents))
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(err),
            })
            .await
            .map_err(|join_err| {
                io::Error::other(format!("client id store load join error: {join_err}"))
            })?
        })
    }

    fn store(
        &self,
        host_id: HostId,
        client_id: String,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let _guard = self.guard.lock().await;
            let directory = self.directory.clone();
            let final_path = self.file_path(&host_id);
            // Same rationale as `load`: keep slow filesystem syscalls
            // off the async executor thread.
            tokio::task::spawn_blocking(move || {
                ensure_directory(&directory)?;
                atomic_write(&final_path, client_id.as_bytes())
            })
            .await
            .map_err(|join_err| {
                io::Error::other(format!("client id store store join error: {join_err}"))
            })?
        })
    }
}

/// Percent-encode a host id into a filesystem-safe filename. Only
/// alphanumerics and the unreserved URI characters (`-`, `.`, `_`, `~`)
/// pass through unchanged; everything else is `%XX`-encoded. The
/// reverse direction isn't needed because the store only ever reads
/// files it wrote.
fn encode_host_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            other => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", other);
            }
        }
    }
    out
}

fn ensure_directory(dir: &Path) -> io::Result<()> {
    match std::fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(_) => {
            // Path exists but isn't a directory — bail out early with a
            // clear error rather than letting later `create_new`/rename
            // fail with a harder-to-diagnose `NotADirectory`.
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "client id store path {} exists but is not a directory",
                    dir.display()
                ),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::fs::create_dir_all(dir)?;
    // Best-effort restrict the directory to owner-only on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Atomically write `content` to `final_path` via a unique-per-call
/// temp file in the same directory, opened with `0o600` mode from the
/// start on Unix so there's no permissions window.
///
/// Retries on `AlreadyExists` so a stale temp file left behind by a
/// crashed process with the same PID can't permanently break writes.
fn atomic_write(final_path: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let parent = final_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "store path has no parent"))?;
    let file_name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "store path is not utf-8"))?;

    // Try a handful of unique temp-name candidates so a crashed peer
    // process that reused our PID can't poison this slot forever.
    const MAX_TEMP_RETRIES: u32 = 8;
    let mut last_err: Option<io::Error> = None;
    for _ in 0..MAX_TEMP_RETRIES {
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{file_name}.{pid}.{counter}.tmp"));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&temp_path) {
            Ok(mut file) => {
                if let Err(err) = file.write_all(content).and_then(|_| file.flush()) {
                    drop(file);
                    let _ = std::fs::remove_file(&temp_path);
                    return Err(err);
                }
                drop(file);
                return match std::fs::rename(&temp_path, final_path) {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        let _ = std::fs::remove_file(&temp_path);
                        Err(err)
                    }
                };
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                last_err = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exhausted temp-file candidates while atomically writing client id",
        )
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn encode_passes_unreserved_chars_through() {
        assert_eq!(encode_host_id("abcXYZ-._~123"), "abcXYZ-._~123");
    }

    #[test]
    fn encode_percent_escapes_reserved_chars() {
        assert_eq!(encode_host_id("ahp://host?q=1"), "ahp%3A%2F%2Fhost%3Fq%3D1");
    }

    #[tokio::test]
    async fn in_memory_round_trips() {
        let store = InMemoryClientIdStore::new();
        assert_eq!(store.load(HostId::new("a")).await.unwrap(), None);
        store.store(HostId::new("a"), "id-a".into()).await.unwrap();
        assert_eq!(
            store.load(HostId::new("a")).await.unwrap(),
            Some("id-a".into())
        );
    }

    #[tokio::test]
    async fn in_memory_store_overwrites_previous_value() {
        let store = InMemoryClientIdStore::new();
        store.store(HostId::new("k"), "first".into()).await.unwrap();
        store
            .store(HostId::new("k"), "second".into())
            .await
            .unwrap();
        assert_eq!(
            store.load(HostId::new("k")).await.unwrap(),
            Some("second".into())
        );
    }
}
