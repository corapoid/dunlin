//! File-scoped changes share operation fingerprints, locks and symlink safeguards.
use super::*;
use std::collections::BTreeSet;

fn file_path(root: &str, path: &str) -> Result<(), String> {
    if !safe_relative(path)
        || path.len() > 4096
        || Path::new(path).components().any(
            |part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")),
        )
    {
        return Err("Choose an ordinary repository-relative file".into());
    }
    let mut full = Path::new(root).to_path_buf();
    let parts: Vec<_> = Path::new(path).components().collect();
    for (index, part) in parts.iter().enumerate() {
        full.push(part.as_os_str());
        match fs::symlink_metadata(&full) {
            Ok(meta)
                if index + 1 < parts.len() && (!meta.is_dir() || meta.file_type().is_symlink()) =>
            {
                return Err("A changed path has an unsafe parent; no files were discarded".into());
            }
            Ok(meta)
                if index + 1 == parts.len()
                    && !meta.is_file()
                    && !meta.file_type().is_symlink() =>
            {
                return Err(
                    "Directories, nested repositories and submodules cannot be discarded as files"
                        .into(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot verify a changed file".into()),
        }
        if index + 1 < parts.len() && fs::symlink_metadata(full.join(".git")).is_ok() {
            return Err("Nested repository files must be handled in their own repository".into());
        }
    }
    if read(root, &["ls-files", "--stage", "-z", "--", path])?
        .split_terminator('\0')
        .any(|entry| entry.starts_with("160000 "))
    {
        return Err("Submodule changes must be handled in the submodule".into());
    }
    if commit(root, "HEAD").is_ok()
        && read(root, &["ls-tree", "-z", "HEAD", "--", path])?
            .split_terminator('\0')
            .any(|entry| entry.starts_with("160000 "))
    {
        return Err("Submodule changes must be handled in the submodule".into());
    }
    Ok(())
}

pub(super) fn changed_paths(root: &str) -> Result<Vec<String>, String> {
    let status = read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut paths: Vec<_> = super::super::parse_changes(&status)
        .into_iter()
        .map(|change| change.path)
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(super) fn discard_paths(root: &str, requested: &[String]) -> Result<Vec<String>, String> {
    if requested.len() > 20_000 {
        return Err("Too many paths for safe discard confirmation".into());
    }
    let mut allowed = BTreeSet::new();
    let mut renames = Vec::new();
    let status = read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut records = status.split_terminator('\0');
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let path = record[3..].to_string();
        allowed.insert(path.clone());
        if record.as_bytes()[..2]
            .iter()
            .any(|kind| matches!(kind, b'R' | b'C'))
        {
            let original = records.next().ok_or("Missing rename source")?.to_string();
            if record.as_bytes()[..2].contains(&b'R') {
                allowed.insert(original.clone());
                renames.push((path, original));
            }
        }
    }
    let mut paths: BTreeSet<_> = requested.iter().cloned().collect();
    if !paths.iter().all(|path| allowed.contains(path)) {
        return Err("A selected path is no longer changed. Refresh source control.".into());
    }
    for (new, old) in renames {
        if paths.contains(&new) || paths.contains(&old) {
            paths.insert(new);
            paths.insert(old);
        }
    }
    for path in &paths {
        file_path(root, path)?;
    }
    Ok(paths.into_iter().collect())
}

// Pin parent descriptors so unlink never follows a concurrently replaced directory symlink.
#[cfg(unix)]
fn remove_file(root: &str, path: &str) -> Result<(), String> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
    };
    let mut parent: OwnedFd = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)
        .map_err(|_| "Cannot open repository directory")?
        .into();
    let mut parts = Path::new(path).components().peekable();
    while let Some(part) = parts.next() {
        let name = CString::new(part.as_os_str().as_bytes()).map_err(|_| "Invalid changed path")?;
        if parts.peek().is_none() {
            // SAFETY: parent is owned, names are validated relative components, and flags never recurse.
            let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
            if result == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
            {
                return Ok(());
            }
            return Err("Could not remove a selected untracked file; refresh state".into());
        }
        // SAFETY: directory-only, no-follow open anchored to the previous owned descriptor.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("Changed-file parent is no longer safe; refresh state".into());
        }
        // SAFETY: successful openat returned a new descriptor owned only here.
        parent = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    Err("Missing changed file".into())
}

#[cfg(not(unix))]
fn remove_file(root: &str, path: &str) -> Result<(), String> {
    file_path(root, path)?;
    fs::remove_file(Path::new(root).join(path))
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|_| "Could not remove a selected untracked file".into())
}

pub(super) fn discard(root: &str, requested: &[String]) -> Result<(), String> {
    let paths = discard_paths(root, requested)?;
    let head = commit(root, "HEAD").is_ok();
    let mut tracked = Vec::new();
    let mut added = Vec::new();
    for path in paths {
        if head && !read(root, &["ls-tree", "--name-only", "-z", "HEAD", "--", &path])?.is_empty() {
            tracked.push(path);
        } else {
            added.push(path);
        }
    }
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
        args.extend(tracked.iter().map(String::as_str));
        read(root, &args)?;
    }
    for path in added {
        if !read(root, &["ls-files", "--stage", "-z", "--", &path])?.is_empty() {
            read(root, &["rm", "--cached", "-f", "--", &path])?;
        }
        if fs::symlink_metadata(Path::new(root).join(&path)).is_ok() {
            remove_file(root, &path)?;
        }
    }
    Ok(())
}

pub(super) fn check_switch(
    root: &str,
    branch: &str,
    inventory: &Overview,
) -> Result<String, String> {
    if branch.is_empty()
        || branch.len() > 512
        || branch.starts_with('-')
        || branch.chars().any(char::is_control)
    {
        return Err("Choose an existing local branch".into());
    }
    let reference = format!("refs/heads/{branch}");
    read(root, &["check-ref-format", &reference])?;
    let literal = read(
        root,
        &[
            "rev-parse",
            "--verify",
            "--symbolic-full-name",
            "--end-of-options",
            branch,
        ],
    )?;
    if literal.trim() != reference {
        return Err("Branch name is ambiguous or no longer exists".into());
    }
    if inventory
        .worktrees
        .iter()
        .any(|tree| !tree.current && tree.branch.as_deref() == Some(reference.as_str()))
    {
        return Err(
            "This branch is already checked out in another worktree; no changes were discarded"
                .into(),
        );
    }
    let target = commit(root, &reference)?;
    protect_ignored(root, &target)?;
    Ok(target)
}

fn ignore_contents(root: &str) -> Result<String, String> {
    match fs::symlink_metadata(Path::new(root).join(".gitignore")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Ok(meta)
            if meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 1024 * 1024 =>
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err(".gitignore must not be hard-linked to another file".into());
                }
            }
            crate::fs::read_file(root, ".gitignore").map(|file| file.contents)
        }
        _ => Err(".gitignore must be a regular, bounded UTF-8 file, never a symlink".into()),
    }
}

pub(super) fn check_ignore(root: &str, path: &str) -> Result<(), String> {
    file_path(root, path)?;
    if path.chars().any(char::is_control) {
        return Err("Control characters cannot be added to .gitignore".into());
    }
    ignore_contents(root)?;
    Ok(())
}

pub(super) fn ignore(root: &str, path: &str) -> Result<(), String> {
    check_ignore(root, path)?;
    let existing = ignore_contents(root)?;
    let mut pattern = String::from("/");
    for ch in path.chars() {
        if matches!(ch, '\\' | '*' | '?' | '[' | ']' | '#' | '!' | ' ') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    if existing.lines().any(|line| line == pattern) {
        return Ok(());
    }
    let mut options = fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(Path::new(root).join(".gitignore"))
        .map_err(|_| "Cannot append to .gitignore")?;
    let meta = file.metadata().map_err(|_| "Cannot verify .gitignore")?;
    if !meta.is_file() || meta.len() != existing.len() as u64 {
        return Err(".gitignore changed; refresh confirmation".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(".gitignore must not be hard-linked to another file".into());
        }
    }
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")
            .map_err(|_| "Cannot update .gitignore")?;
    }
    file.write_all(format!("{pattern}\n").as_bytes())
        .map_err(|_| "Cannot update .gitignore")?;
    Ok(())
}
