//! A live blocking worker keeps this lease even if its HTTP waiter disappears.
//! The persistent inode is never unlinked. A second process must obtain the
//! same kernel lock before reconciling an unfinished operation.
use super::{PersistenceError, Store};
use std::{
    fs::File,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct RevocationLease {
    owner: Arc<AtomicBool>,
    file: Option<File>,
}
/// Positive credential writers and future retention share a separate gate.
/// AS revocation/local disconnect deliberately do not wait for this gate.
pub(crate) struct WriterLease {
    _lease: RevocationLease,
}
impl RevocationLease {
    pub(super) fn belongs_to(&self, store: &Store) -> bool {
        Arc::ptr_eq(&self.owner, &store.oauth_revocation_owner)
    }
}
impl Drop for RevocationLease {
    fn drop(&mut self) {
        // Release the kernel lock before reopening process-local admission.
        self.file.take();
        self.owner.store(false, Ordering::Release);
    }
}
impl Store {
    pub(crate) fn mcp_oauth_revocation_lease(&self) -> Result<RevocationLease, PersistenceError> {
        self.oauth_lease(&self.oauth_revocation_owner, ".mcp-oauth-revocation.lock")
    }
    pub(crate) fn mcp_oauth_writer_lease(&self) -> Result<WriterLease, PersistenceError> {
        Ok(WriterLease {
            _lease: self.oauth_lease(&self.oauth_writer_owner, ".mcp-oauth-writer.lock")?,
        })
    }
    fn oauth_lease(
        &self,
        owner: &Arc<AtomicBool>,
        suffix: &str,
    ) -> Result<RevocationLease, PersistenceError> {
        owner
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PersistenceError::Conflict)?;
        let mut lease = RevocationLease {
            owner: owner.clone(),
            file: None,
        };
        // In-memory databases cannot be reopened by another process; the same
        // Store-local gate still excludes replacement Service instances.
        if let Some(path) = self.connection.path().filter(|p| !p.is_empty()) {
            lease.file = Some(acquire(std::path::Path::new(path), suffix)?);
        }
        Ok(lease)
    }
}

#[cfg(unix)]
fn acquire(database: &std::path::Path, suffix: &str) -> Result<File, PersistenceError> {
    use std::os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let database = database
        .canonicalize()
        .map_err(|_| PersistenceError::Storage)?;
    // Two hard-link aliases must not create distinct sidecar lock namespaces.
    if std::fs::metadata(&database)
        .map_err(|_| PersistenceError::Storage)?
        .nlink()
        != 1
    {
        return Err(PersistenceError::Storage);
    }
    let mut name = database.as_os_str().to_owned();
    name.push(suffix);
    let path = std::path::PathBuf::from(name);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| PersistenceError::Storage)?;
    let metadata = file.metadata().map_err(|_| PersistenceError::Storage)?;
    let entry = std::fs::symlink_metadata(&path).map_err(|_| PersistenceError::Storage)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o600
        || !entry.is_file()
        || metadata.dev() != entry.dev()
        || metadata.ino() != entry.ino()
    {
        return Err(PersistenceError::Storage);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                PersistenceError::Conflict
            } else {
                PersistenceError::Storage
            },
        );
    }
    Ok(file)
}
#[cfg(not(unix))]
fn acquire(_: &std::path::Path, _: &str) -> Result<File, PersistenceError> {
    Err(PersistenceError::Storage)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    #[test]
    fn mcp_oauth_operation_leases_exclude_live_process_and_release_after_sigkill() {
        for suffix in [".mcp-oauth-revocation.lock", ".mcp-oauth-writer.lock"] {
            let dir = tempfile::tempdir().unwrap();
            let database = dir.path().join("relay.db");
            File::create(&database).unwrap();
            let first = acquire(&database, suffix).unwrap();
            assert!(matches!(
                acquire(&database, suffix),
                Err(PersistenceError::Conflict)
            ));
            drop(first);
            let path = database.with_file_name(format!("relay.db{suffix}"));
            let mut child = Command::new("python3")
                .args([
                    "-I",
                    "-c",
                    r#"
import fcntl,sys,time
with open(sys.argv[1],'r+') as f:
 fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
 print('locked',flush=True)
 time.sleep(30)
"#,
                ])
                .arg(&path)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut ready = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready.trim(), "locked");
            assert!(matches!(
                acquire(&database, suffix),
                Err(PersistenceError::Conflict)
            ));
            child.kill().unwrap();
            child.wait().unwrap();
            let recovered = acquire(&database, suffix).unwrap();
            use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
            assert_eq!(
                recovered.metadata().unwrap().ino(),
                std::fs::metadata(&path).unwrap().ino()
            );
            assert_ne!(
                unsafe { libc::fcntl(recovered.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
        }
    }
    #[test]
    fn mcp_oauth_revocation_lease_rejects_unsafe_inode_without_repairing_it() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for kind in ["symlink", "hardlink", "public", "fifo"] {
            let dir = tempfile::tempdir().unwrap();
            let database = dir.path().join("relay.db");
            File::create(&database).unwrap();
            let path = dir.path().join("relay.db.mcp-oauth-revocation.lock");
            let other = dir.path().join("other");
            File::create(&other).unwrap();
            match kind {
                "symlink" => symlink(&other, &path).unwrap(),
                "hardlink" => std::fs::hard_link(&other, &path).unwrap(),
                "public" => {
                    File::create(&path).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
                _ => {
                    use std::os::unix::ffi::OsStrExt;
                    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
            }
            assert!(matches!(
                acquire(&database, ".mcp-oauth-revocation.lock"),
                Err(PersistenceError::Storage)
            ));
        }
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("relay.db");
        File::create(&database).unwrap();
        std::fs::hard_link(&database, dir.path().join("alias.db")).unwrap();
        assert!(matches!(
            acquire(&database, ".mcp-oauth-revocation.lock"),
            Err(PersistenceError::Storage)
        ));
    }
}
