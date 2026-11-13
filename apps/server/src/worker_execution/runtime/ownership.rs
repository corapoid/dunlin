//! Process-local admission is insufficient: another worker process must not run
//! boot recovery over a still-live runtime's state. Hold this kernel lock from
//! before recovery until the runtime (including its actors) actually drops.
use super::Code;
use std::{fs::File, path::Path};
pub(super) struct FileLease {
    _file: File,
}
impl FileLease {
    #[cfg(unix)]
    pub(super) fn acquire(state: &Path) -> Result<Self, Code> {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let path = state.join("execution-runtime.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| Code::Unavailable)?;
        let metadata = file.metadata().map_err(|_| Code::Unavailable)?;
        let entry = std::fs::symlink_metadata(&path).map_err(|_| Code::Unavailable)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7777 != 0o600
            || entry.dev() != metadata.dev()
            || entry.ino() != metadata.ino()
            || !entry.is_file()
        {
            return Err(Code::Unavailable);
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                    Code::CapacityReached
                } else {
                    Code::Unavailable
                },
            );
        }
        // Never unlink the inode: a concurrent opener must contend on the same
        // file after shutdown/relaunch. CLOEXEC keeps agents/guardians from
        // accidentally retaining ownership after the worker exits.
        Ok(Self { _file: file })
    }
    #[cfg(not(unix))]
    pub(super) fn acquire(_state: &Path) -> Result<Self, Code> {
        Err(Code::UnsupportedProfile)
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::{
        fd::AsRawFd,
        unix::fs::{PermissionsExt, symlink},
    };
    #[test]
    fn real_kernel_lock_excludes_another_process_and_releases_without_unlinking() {
        let dir = tempfile::tempdir().unwrap();
        let first = FileLease::acquire(dir.path()).unwrap();
        let path = dir.path().join("execution-runtime.lock");
        let inode = std::fs::metadata(&path).unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            unsafe { libc::fcntl(first._file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        let child = std::process::Command::new("python3")
            .args([
                "-I",
                "-c",
                r#"import fcntl,sys
with open(sys.argv[1],'r+') as file:
 try: fcntl.flock(file,fcntl.LOCK_EX|fcntl.LOCK_NB)
 except BlockingIOError: sys.exit(0)
 sys.exit(3)
"#,
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(child.success());
        assert!(matches!(
            FileLease::acquire(dir.path()),
            Err(Code::CapacityReached)
        ));
        drop(first);
        let second = FileLease::acquire(dir.path()).unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().ino(), inode.ino());
        drop(second);
    }
    #[test]
    fn unsafe_lock_entries_are_rejected_without_waiting_or_repairing_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        for kind in ["symlink", "hardlink", "fifo", "public"] {
            let state = dir.path().join(kind);
            std::fs::create_dir(&state).unwrap();
            let path = state.join("execution-runtime.lock");
            match kind {
                "symlink" => symlink(&target, &path).unwrap(),
                "hardlink" => std::fs::hard_link(&target, &path).unwrap(),
                "fifo" => {
                    use std::os::unix::ffi::OsStrExt;
                    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
                _ => {
                    std::fs::write(&path, b"").unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
            }
            assert!(matches!(FileLease::acquire(&state), Err(Code::Unavailable)));
        }
    }
}
