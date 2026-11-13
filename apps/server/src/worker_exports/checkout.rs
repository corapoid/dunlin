//! Worker-local state database handling. Checkout identity validation lives in
//! the independent checkout_identity module; this adapter owns no shared roots.
use super::{Connection, Error, Result};
use std::{fs, path::Path};
#[cfg(unix)]
pub(super) fn open_state(path: &Path, create: bool) -> Result<Connection> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    if !path.is_absolute() {
        return Err(Error::Invalid("Worker state directory must be absolute"));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
            {
                return Err(Error::Invalid(
                    "Worker state directory must be a private owned directory",
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
        }
        Err(e) => return Err(e.into()),
    }
    let db = path.join("exports.sqlite3");
    let existing = || {
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&db)
    };
    let file = if create {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&db)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&db)?,
            Err(e) => return Err(e.into()),
        }
    } else {
        existing()?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(Error::Invalid(
            "Worker state database must be a private owned regular file",
        ));
    }
    Ok(Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?)
}
#[cfg(not(unix))]
pub(super) fn open_state(_path: &Path, _create: bool) -> Result<Connection> {
    Err(Error::Invalid(
        "Worker export storage requires macOS or Linux",
    ))
}
