//! Explorer transfers: move inside the project, copy external drops, never replace.
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

const MAX_IMPORT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_IMPORT_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transferred {
    pub source: String,
    pub destination: String,
    pub moved: bool,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct TransferReport {
    pub completed: Vec<Transferred>,
    pub error: Option<String>,
}

fn protected(relative: &Path) -> bool {
    relative
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")))
}

pub fn create_directory(root: &str, relative: &str) -> Result<crate::fs::FileStat, String> {
    let path = crate::fs::resolve(root, relative)?;
    let canonical_root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if path == canonical_root
        || protected(
            path.strip_prefix(&canonical_root)
                .map_err(|e| e.to_string())?,
        )
    {
        return Err("Choose a new folder inside the project, outside Git metadata".into());
    }
    fs::create_dir(&path).map_err(|e| format!("Could not create folder: {e}"))?;
    crate::fs::stat_file(root, relative)
}

/// Resolve an entry for a user-requested mutation, preserving the no-symlink,
/// project-root and Git-metadata protections used by Explorer transfers.
pub fn mutable_entry(root: &str, relative: &str) -> Result<PathBuf, String> {
    let resolved = crate::fs::resolve(root, relative)?;
    let canonical_root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if resolved == canonical_root
        || protected(Path::new(relative))
        || protected(
            resolved
                .strip_prefix(&canonical_root)
                .map_err(|e| e.to_string())?,
        )
    {
        return Err("Project roots and Git metadata cannot be changed through Explorer".into());
    }
    let lexical = Path::new(root).join(relative);
    let metadata = fs::symlink_metadata(&lexical).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err("Only regular files and folders can be changed".into());
    }
    Ok(resolved)
}

pub fn rename_entry(root: &str, relative: &str, name: &str) -> Result<Transferred, String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.eq_ignore_ascii_case(".git")
    {
        return Err("Enter a single file or folder name outside Git metadata".into());
    }
    let source = mutable_entry(root, relative)?;
    let parent = source.parent().ok_or("Missing parent folder")?;
    let destination = parent.join(name);
    if source != destination {
        rename_new(&source, &destination)
            .map_err(|e| format!("Could not rename without replacing an existing item: {e}"))?;
    }
    let canonical_root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let display = |p: &Path| {
        Path::new(root)
            .join(p.strip_prefix(&canonical_root).unwrap_or(p))
            .to_string_lossy()
            .into_owned()
    };
    Ok(Transferred {
        source: display(&source),
        destination: display(&destination),
        moved: true,
    })
}

/// Remote deletion is explicit and non-recursive: nonempty directories are refused.
pub fn delete_entry(root: &str, relative: &str) -> Result<(), String> {
    let path = mutable_entry(root, relative)?;
    if path.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| e.to_string())
}

struct Planned {
    source: PathBuf,
    destination: PathBuf,
    moved: bool,
}

fn inspect_import(
    path: &Path,
    entries: &mut usize,
    bytes: &mut u64,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err("Import folder nesting exceeds 64 levels".into());
    }
    *entries += 1;
    if *entries > MAX_IMPORT_ENTRIES {
        return Err("Import exceeds 20,000 items".into());
    }
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
    {
        return Err("Git metadata cannot be imported through Explorer".into());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not inspect {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Symbolic links are not imported: {}",
            path.display()
        ));
    }
    if metadata.is_dir() {
        for child in fs::read_dir(path).map_err(|e| e.to_string())? {
            inspect_import(
                &child.map_err(|e| e.to_string())?.path(),
                entries,
                bytes,
                depth + 1,
            )?;
        }
    } else if metadata.is_file() {
        *bytes = bytes.saturating_add(metadata.len());
        if *bytes > MAX_IMPORT_BYTES {
            return Err("Import exceeds 1 GiB; copy a smaller selection".into());
        }
    } else {
        return Err("Only regular files and folders can be imported".into());
    }
    Ok(())
}

fn plan(root: &str, sources: &[PathBuf], target: &str) -> Result<Vec<Planned>, String> {
    if sources.len() > 1000 {
        return Err("Drop at most 1,000 top-level items at once".into());
    }
    let root_path = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let directory = crate::fs::resolve(root, target)?;
    if !directory.is_dir()
        || protected(
            directory
                .strip_prefix(&root_path)
                .map_err(|e| e.to_string())?,
        )
    {
        return Err("Drop onto a project folder outside Git metadata".into());
    }
    let mut result: Vec<Planned> = Vec::new();
    let (mut count, mut bytes) = (0, 0);
    for source in sources {
        if !source.is_absolute() {
            return Err("Dropped files must have absolute paths".into());
        }
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("Moving or importing symbolic links is not supported".into());
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err("Only regular files and folders can be transferred".into());
        }
        let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
        if source == root_path || source.is_dir() && directory.starts_with(&source) {
            return Err("A folder cannot be dropped into itself or a descendant".into());
        }
        let moved = source.starts_with(&root_path);
        if moved && protected(source.strip_prefix(&root_path).map_err(|e| e.to_string())?) {
            return Err("Git metadata cannot be moved through Explorer".into());
        }
        let name = source.file_name().ok_or("The source has no file name")?;
        let destination = directory.join(name);
        if source == destination {
            continue;
        }
        if destination.try_exists().map_err(|e| e.to_string())?
            || fs::symlink_metadata(&destination).is_ok()
        {
            return Err(format!(
                "{} already exists in the destination; nothing was replaced",
                name.to_string_lossy()
            ));
        }
        if result.iter().any(|entry| {
            entry.destination == destination
                || source.starts_with(&entry.source)
                || entry.source.starts_with(&source)
        }) {
            return Err("The selection has duplicate names or overlapping folders".into());
        }
        if !moved {
            inspect_import(&source, &mut count, &mut bytes, 0)?;
        }
        result.push(Planned {
            source,
            destination,
            moved,
        });
    }
    Ok(result)
}

/// Atomically reject an existing target, including a target created after preflight.
fn rename_new(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: both C strings are live and NUL-terminated. The flags prohibit replacement.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(target_os = "windows")]
    {
        fs::rename(source, destination)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (source, destination);
        Err(std::io::Error::other(
            "Safe, non-replacing moves are unavailable on this platform",
        ))
    }
}

fn copy_contents(
    source: &Path,
    destination: &Path,
    count: &mut usize,
    bytes: &mut u64,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err("Import folder nesting exceeds 64 levels".into());
    }
    *count += 1;
    if *count > MAX_IMPORT_ENTRIES {
        return Err("Import exceeds 20,000 items".into());
    }
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("The source changed into a symbolic link; import stopped".into());
    }
    if metadata.is_dir() {
        fs::create_dir(destination).map_err(|e| e.to_string())?;
        for child in fs::read_dir(source).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            if child.file_name().eq_ignore_ascii_case(".git") {
                return Err("Git metadata cannot be imported".into());
            }
            copy_contents(
                &child.path(),
                &destination.join(child.file_name()),
                count,
                bytes,
                depth + 1,
            )?;
        }
    } else if metadata.is_file() {
        if bytes.saturating_add(metadata.len()) > MAX_IMPORT_BYTES {
            return Err("Import exceeds 1 GiB".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut input = options.open(source).map_err(|e| e.to_string())?;
        if !input.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("The source is no longer a regular file; import stopped".into());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        // Bound growth of a file concurrently being written during import.
        use std::io::Read;
        let copied = std::io::copy(
            &mut input.by_ref().take(MAX_IMPORT_BYTES - *bytes + 1),
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        *bytes = bytes.saturating_add(copied);
        if *bytes > MAX_IMPORT_BYTES {
            return Err("Import exceeds 1 GiB".into());
        }
    } else {
        return Err("Only regular files and folders can be imported".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            (metadata.permissions().mode() & 0o777) | if metadata.is_dir() { 0o700 } else { 0 };
        fs::set_permissions(destination, fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        Ok(())
    }
}

fn copy_new(
    source: &Path,
    destination: &Path,
    count: &mut usize,
    bytes: &mut u64,
) -> Result<(), String> {
    let parent = destination.parent().ok_or("Missing destination folder")?;
    let staging = parent.join(format!(".dunlin-import-{}", uuid::Uuid::now_v7()));
    // Reserve a private staging directory; cleanup only ever touches this owned path.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&staging).map_err(|e| e.to_string())?;
    let payload = staging.join("payload");
    let result = copy_contents(source, &payload, count, bytes, 0)
        .and_then(|()| rename_new(&payload, destination).map_err(|e| e.to_string()));
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Preflight the whole batch before changing anything; report completed operations
/// if a later filesystem error interrupts execution. Never delete external originals.
pub fn transfer(root: &str, sources: &[PathBuf], target: &str) -> TransferReport {
    let canonical_root = fs::canonicalize(root).ok();
    let plans = match plan(root, sources, target) {
        Ok(plans) => plans,
        Err(error) => {
            return TransferReport {
                error: Some(error),
                ..Default::default()
            };
        }
    };
    let mut report = TransferReport::default();
    let (mut count, mut bytes) = (0, 0);
    for entry in plans {
        let result = if entry.moved {
            rename_new(&entry.source, &entry.destination).map_err(|e| e.to_string())
        } else {
            copy_new(&entry.source, &entry.destination, &mut count, &mut bytes)
        };
        if let Err(error) = result {
            report.error = Some(format!(
                "Could not transfer {}: {error}",
                entry.source.display()
            ));
            break;
        }
        let display_path = |path: &Path| {
            canonical_root
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .map(|relative| Path::new(root).join(relative))
                .unwrap_or_else(|| path.to_path_buf())
                .to_string_lossy()
                .into_owned()
        };
        report.completed.push(Transferred {
            source: display_path(&entry.source),
            destination: display_path(&entry.destination),
            moved: entry.moved,
        });
    }
    report
}

/// Move-only entry point for local/relay project backends. Validate the entire
/// request before constructing source paths; never treat an escaping path as an import.
pub fn move_entries(
    root: &str,
    sources: &[String],
    target: &str,
) -> Result<TransferReport, String> {
    if sources.len() > 1000 {
        return Err("Move at most 1,000 top-level items at once".into());
    }
    let sources = sources
        .iter()
        .map(|path| {
            crate::fs::resolve(root, path)?;
            Ok(Path::new(root).join(path))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(transfer(root, &sources, target))
}

#[cfg(test)]
mod tests;
