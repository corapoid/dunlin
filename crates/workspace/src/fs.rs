//! Project-scoped filesystem access shared by the desktop and the relay.
//!
//! Every path is resolved against a project root and canonicalized before it is
//! used. That keeps `..`, absolute paths and symlinks that escape the project
//! out of both local and remote editing.

#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// The editor refuses to load anything larger than this.
pub const MAX_EDITOR_BYTES: u64 = 2 * 1024 * 1024;
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileContents {
    pub contents: String,
    pub readonly: bool,
    pub bytes: u64,
    pub mtime_ms: u64,
    pub binary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileStat {
    pub bytes: u64,
    pub mtime_ms: u64,
    pub is_dir: bool,
}

fn git_metadata(path: &Path) -> bool {
    path.components().any(
        |component| matches!(component, Component::Normal(name) if name.eq_ignore_ascii_case(".git")),
    )
}

/// Resolve `relative` inside `root`, rejecting traversal and escaping symlinks.
pub fn resolve(root: &str, relative: &str) -> Result<PathBuf, String> {
    let root_path = fs::canonicalize(root)
        .map_err(|error| format!("Project folder is not available: {error}"))?;
    if !root_path.is_dir() {
        return Err("Project folder is not a directory".into());
    }

    let relative = relative.trim();
    let relative = if relative.is_empty() { "." } else { relative };
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err("Absolute paths are not allowed".into());
    }
    if git_metadata(candidate) {
        return Err("Git metadata is not available through the file API".into());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err("Path escapes the project folder".into()),
        }
    }

    let joined = root_path.join(candidate);
    let resolved = if joined.exists() {
        fs::canonicalize(&joined).map_err(|error| format!("Could not resolve path: {error}"))?
    } else {
        let parent = joined
            .parent()
            .ok_or_else(|| "Path has no parent folder".to_string())?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| format!("Parent folder is not available: {error}"))?;
        if !parent.starts_with(&root_path) {
            return Err("Path escapes the project folder".into());
        }
        parent.join(
            joined
                .file_name()
                .ok_or_else(|| "Path has no file name".to_string())?,
        )
    };

    if !resolved.starts_with(&root_path) {
        return Err("Path escapes the project folder".into());
    }
    Ok(resolved)
}

/// Open a project-relative path without following symlinks in any component.
///
/// `resolve` is still used for user-facing validation and canonical paths, but
/// the actual open walks from a directory descriptor with `O_NOFOLLOW`. This
/// closes the window where a parent directory could be swapped for a symlink
/// between validation and `open`/`write`.
#[cfg(unix)]
fn open_relative(
    root: &str,
    relative: &str,
    write: bool,
    create_new: bool,
) -> Result<File, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};

    let resolved = resolve(root, relative)?;
    if let Ok(metadata) = fs::symlink_metadata(&resolved)
        && !metadata.is_file()
        && !metadata.is_dir()
    {
        return Err("Only regular files and directories are available through the file API".into());
    }
    let root_path = fs::canonicalize(root)
        .map_err(|error| format!("Project folder is not available: {error}"))?;
    let root_name = CString::new(root_path.as_os_str().as_bytes())
        .map_err(|_| "Project path contains a NUL byte".to_string())?;
    let root_fd = unsafe {
        libc::open(
            root_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(format!(
            "Could not open the project folder: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut directory = unsafe { File::from_raw_fd(root_fd) };
    let parts = Path::new(relative)
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_os_string()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err("A file path is required".into());
    }
    for part in &parts[..parts.len() - 1] {
        let name = CString::new(part.as_bytes()).map_err(|_| "Invalid path component")?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "Could not open a parent folder: {}",
                std::io::Error::last_os_error()
            ));
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    let name = CString::new(parts.last().expect("non-empty").as_bytes())
        .map_err(|_| "Invalid path component")?;
    let mut flags = if write {
        libc::O_WRONLY
    } else {
        libc::O_RDONLY
    } | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | libc::O_NOCTTY;
    if create_new {
        flags |= libc::O_CREAT | libc::O_EXCL;
    } else if write {
        flags |= libc::O_TRUNC;
    }
    let fd = unsafe {
        if create_new {
            libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o666)
        } else {
            libc::openat(directory.as_raw_fd(), name.as_ptr(), flags)
        }
    };
    if fd < 0 {
        return Err(format!(
            "Could not open the requested path: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Only regular files and directories are available through the file API".into());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_relative(
    root: &str,
    relative: &str,
    write: bool,
    create_new: bool,
) -> Result<File, String> {
    let path = resolve(root, relative)?;
    let mut options = OpenOptions::new();
    options.read(!write).write(write);
    if create_new {
        options.create_new(true);
    } else if write {
        options.truncate(true);
    }
    options
        .open(path)
        .map_err(|error| format!("Could not open the requested path: {error}"))
}

pub(crate) fn open_read(root: &str, relative: &str) -> Result<File, String> {
    open_relative(root, relative, false, false)
}

/// Relative path from the project root, using `/` separators.
pub fn relative_to(root: &str, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn read_file(root: &str, relative: &str) -> Result<FileContents, String> {
    let file = open_read(root, relative)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not read file: {error}"))?;
    if metadata.is_dir() {
        return Err("This path is a folder".into());
    }
    if metadata.len() > MAX_EDITOR_BYTES {
        return Err(format!(
            "This file is too large to edit ({} KiB)",
            metadata.len() / 1024
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_EDITOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read file: {error}"))?;
    if bytes.len() as u64 > MAX_EDITOR_BYTES {
        return Err("File grew beyond the editor size limit".into());
    }
    let binary = bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0);
    let mtime_ms = mtime_ms(&metadata);
    if binary {
        return Ok(FileContents {
            contents: "Binary file — editing is disabled.".into(),
            readonly: true,
            bytes: metadata.len(),
            mtime_ms,
            binary: true,
        });
    }
    match String::from_utf8(bytes) {
        Ok(contents) => Ok(FileContents {
            contents,
            readonly: false,
            bytes: metadata.len(),
            mtime_ms,
            binary: false,
        }),
        Err(_) => Ok(FileContents {
            contents: "File is not valid UTF-8 — editing is disabled.".into(),
            readonly: true,
            bytes: metadata.len(),
            mtime_ms,
            binary: true,
        }),
    }
}

pub fn write_file(root: &str, relative: &str, contents: &str) -> Result<FileStat, String> {
    write_file_checked(root, relative, contents, None)
}

/// Serialize application writers, including local editors and relay requests.
/// The expected version is checked under the same lock as the atomic replacement.
pub fn write_file_checked(
    root: &str,
    relative: &str,
    contents: &str,
    expected_mtime_ms: Option<u64>,
) -> Result<FileStat, String> {
    use std::hash::{Hash, Hasher};
    static WRITERS: [Mutex<()>; 64] = [const { Mutex::new(()) }; 64];
    if contents.len() as u64 > MAX_EDITOR_BYTES {
        return Err("This file is too large to save".into());
    }
    let path = resolve(root, relative)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    let _writer = WRITERS[hash.finish() as usize % WRITERS.len()]
        .lock()
        .map_err(|_| "File writer lock is unavailable")?;
    let metadata = open_read(root, relative)?
        .metadata()
        .map_err(|error| format!("Could not write file: {error}"))?;
    if !metadata.is_file() {
        return Err("Saving requires a regular file".into());
    }
    if expected_mtime_ms.is_some_and(|expected| expected != mtime_ms(&metadata)) {
        return Err("conflict: file changed on disk since it was opened".into());
    }
    if metadata.permissions().readonly() {
        return Err("This file is read-only".into());
    }
    let parent = Path::new(relative).parent().unwrap_or(Path::new(""));
    let temporary_name = format!(".dunlin-write-{}", uuid::Uuid::now_v7());
    let temporary_relative = if parent.as_os_str().is_empty() {
        temporary_name.clone()
    } else {
        format!("{}/{}", parent.to_string_lossy(), temporary_name)
    };
    let temporary = path.with_file_name(temporary_name);
    let result = (|| {
        let mut file = open_relative(root, &temporary_relative, true, true)?;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("Could not save file: {error}"))?;
        file.set_permissions(metadata.permissions())
            .map_err(|error| format!("Could not preserve file permissions: {error}"))?;
        // Millisecond versions must advance even for two saves in the same tick.
        let modified = std::time::SystemTime::now().max(
            UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms(&metadata).saturating_add(1)),
        );
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .map_err(|error| format!("Could not set file version: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not save file: {error}"))?;
        // Return the version we wrote, not a later external edit observed by
        // reopening the destination after rename.
        let saved = file.metadata().map_err(|error| error.to_string())?;
        let current = open_read(root, relative)?
            .metadata()
            .map_err(|error| error.to_string())?;
        if current.modified().ok() != metadata.modified().ok() || current.len() != metadata.len() {
            return Err("conflict: file changed while it was being saved".into());
        }
        fs::rename(&temporary, &path)
            .map_err(|error| format!("Could not replace file atomically: {error}"))?;
        Ok(FileStat {
            bytes: saved.len(),
            mtime_ms: mtime_ms(&saved),
            is_dir: false,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn create_file(root: &str, relative: &str) -> Result<FileStat, String> {
    if Path::new(relative)
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")))
    {
        return Err("Files cannot be created inside Git metadata through Explorer".into());
    }
    let path = resolve(root, relative)?;
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    if path
        .strip_prefix(&canonical_root)
        .map_err(|error| error.to_string())?
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")))
    {
        return Err("Files cannot be created inside Git metadata through Explorer".into());
    }
    if path.exists() {
        return Err("A file with that name already exists".into());
    }
    open_relative(root, relative, true, true)
        .map_err(|error| format!("Could not create file: {error}"))?;
    stat_file(root, relative)
}

pub fn stat_file(root: &str, relative: &str) -> Result<FileStat, String> {
    let file = open_read(root, relative)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not stat file: {error}"))?;
    Ok(FileStat {
        bytes: metadata.len(),
        mtime_ms: mtime_ms(&metadata),
        is_dir: metadata.is_dir(),
    })
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{create_file, read_file, resolve, stat_file, write_file};
    use std::fs;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-fs-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn reads_and_writes_inside_the_project() {
        let root = temp_root("read-write");
        let root_text = root.to_string_lossy().to_string();
        create_file(&root_text, "notes.md").unwrap();
        write_file(&root_text, "notes.md", "hello\n").unwrap();
        let file = read_file(&root_text, "notes.md").unwrap();
        assert_eq!(file.contents, "hello\n");
        assert!(!file.readonly);
        assert_eq!(stat_file(&root_text, "notes.md").unwrap().bytes, 6);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        let root = temp_root("traversal");
        let root_text = root.to_string_lossy().to_string();
        assert!(resolve(&root_text, "../escape").is_err());
        assert!(resolve(&root_text, "/etc/passwd").is_err());
        assert!(resolve(&root_text, "src/../../escape").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn marks_binary_files_readonly() {
        let root = temp_root("binary");
        let root_text = root.to_string_lossy().to_string();
        fs::write(root.join("image.bin"), [0u8, 1, 2, 3]).unwrap();
        let file = read_file(&root_text, "image.bin").unwrap();
        assert!(file.binary);
        assert!(file.readonly);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn git_metadata_is_not_available_through_the_file_api() {
        let root = temp_root("git-metadata");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "[core]\n").unwrap();
        let root_text = root.to_string_lossy().to_string();
        assert!(resolve(&root_text, ".git/config").is_err());
        assert!(read_file(&root_text, ".git/config").is_err());
        assert!(write_file(&root_text, ".git/config", "x").is_err());
        assert!(stat_file(&root_text, ".git/config").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let root = temp_root("create");
        let root_text = root.to_string_lossy().to_string();
        create_file(&root_text, "new.txt").unwrap();
        assert!(create_file(&root_text, "new.txt").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_writers_cannot_both_save_the_same_version() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("file.txt"), "original").unwrap();
        let root = root.path().to_string_lossy().into_owned();
        let expected = stat_file(&root, "file.txt").unwrap().mtime_ms;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let writes: Vec<_> = ["first", "second"]
                .into_iter()
                .map(|text| {
                    let barrier = barrier.clone();
                    let root = &root;
                    scope.spawn(move || {
                        barrier.wait();
                        super::write_file_checked(root, "file.txt", text, Some(expected))
                    })
                })
                .collect();
            let results: Vec<_> = writes
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert!(
                results
                    .iter()
                    .any(|result| matches!(result, Err(error) if error.starts_with("conflict:")))
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn fifo_reads_stats_and_previews_fail_without_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        for name in ["pipe.txt", "pipe.png"] {
            let path =
                std::ffi::CString::new(root.path().join(name).as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        }
        let path = root.path().to_string_lossy().into_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            tx.send((
                read_file(&path, "pipe.txt").is_err(),
                stat_file(&path, "pipe.txt").is_err(),
                crate::preview::read(&path, "pipe.png").is_err(),
            ))
            .unwrap();
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
            (true, true, true)
        );
        worker.join().unwrap();
    }
}
