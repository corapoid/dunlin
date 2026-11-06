//! Portable project snapshots. Git history travels as a bundle, never raw .git.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const MAX_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;
pub const CHUNK: usize = 1024 * 1024;
pub const BUNDLE: &str = ".dunlin-transfer.bundle";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub files: Vec<Entry>,
    pub branch: Option<String>,
    pub history: bool,
}

pub fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
        && !value.contains(['\\', '\0'])
        && Path::new(value).components().all(|part| {
            matches!(part, Component::Normal(name)
            if !name.eq_ignore_ascii_case(".git"))
        })
}

pub fn digest(path: &Path) -> Result<String, String> {
    digest_file(fs::File::open(path).map_err(|e| e.to_string())?)
}

pub fn digest_file(mut file: fs::File) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_BYTES {
            return Err("File grew past the transfer limit".into());
        }
        hasher.update(&buf[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Resolve every component relative to held directory descriptors, not a racy path.
#[cfg(unix)]
pub fn open_source(root: &Path, relative_path: &str) -> Result<fs::File, String> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    };
    if !relative(relative_path) {
        return Err("Unsafe source path".into());
    }
    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)
        .map_err(|e| e.to_string())?;
    let parts: Vec<_> = Path::new(relative_path).components().collect();
    for (index, part) in parts.iter().enumerate() {
        let name = std::ffi::CString::new(part.as_os_str().as_encoded_bytes())
            .map_err(|_| "Invalid source name")?;
        let last = index + 1 == parts.len();
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | if last { 0 } else { libc::O_DIRECTORY },
            )
        };
        if fd < 0 {
            return Err(
                "Source changed, is a symbolic link, or is unavailable. Preview again.".into(),
            );
        }
        let file = unsafe { fs::File::from_raw_fd(fd) };
        if last {
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Only regular files can be copied".into());
            }
            return Ok(file);
        }
        directory = file;
    }
    Err("Empty source path".into())
}
#[cfg(not(unix))]
pub fn open_source(_root: &Path, _relative: &str) -> Result<fs::File, String> {
    Err("Safe project copy requires Unix".into())
}

pub fn validate(manifest: &Manifest) -> Result<u64, String> {
    if manifest.name.trim().is_empty()
        || manifest.name.len() > 200
        || matches!(manifest.name.as_str(), "." | "..")
        || manifest.name.eq_ignore_ascii_case(".git")
        || manifest.name.starts_with(".transfer-")
        || manifest.name.contains(['/', '\\'])
        || manifest.name.chars().any(char::is_control)
        || manifest.files.len() > MAX_FILES
    {
        return Err("Invalid project name or too many files (maximum 20,000)".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for entry in &manifest.files {
        if !relative(&entry.path)
            || !seen.insert(entry.path.to_lowercase())
            || entry.sha256.len() != 64
            || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid, duplicate or non-portable project path/checksum".into());
        }
        bytes = bytes
            .checked_add(entry.size)
            .filter(|n| *n <= MAX_BYTES)
            .ok_or("Project exceeds 1 GiB; exclude generated files")?;
    }
    if manifest.history != manifest.files.iter().any(|f| f.path == BUNDLE) {
        return Err("Git bundle and history selection disagree".into());
    }
    if manifest.branch.as_ref().is_some_and(|b| !valid_branch(b)) {
        return Err("Unsupported Git branch name".into());
    }
    Ok(bytes)
}

pub fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 200
        && !branch.starts_with(['-', '/'])
        && !branch.ends_with(['.', '/'])
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.contains("//")
        && !branch.ends_with(".lock")
        && branch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_/ .".contains(&b) && b != b' ')
}

pub struct Snapshot {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub bundle: Option<PathBuf>,
    pub exclusions: Vec<String>,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        if let Some(path) = &self.bundle {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn snapshot(
    root: &Path,
    name: String,
    history: bool,
    include_ignored: bool,
    exclusions: &[String],
    scratch: &Path,
) -> Result<Snapshot, String> {
    if exclusions.iter().any(|path| !relative(path)) {
        return Err("Exclusions must be relative paths inside the project".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let git_repo = git(&root, &["rev-parse", "--show-toplevel"])
        .ok()
        .is_some_and(|out| Path::new(out.trim()).canonicalize().ok().as_ref() == Some(&root));
    if history && !git_repo {
        return Err("History requires a Git repository root".into());
    }
    let mut paths = Vec::new();
    if git_repo {
        let out = if include_ignored {
            git(&root, &["ls-files", "-z", "--cached", "--others"])?
        } else {
            git(
                &root,
                &[
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
            )?
        };
        paths.extend(out.split('\0').filter(|p| !p.is_empty()).map(PathBuf::from));
    } else {
        fn walk(
            root: &Path,
            path: &Path,
            paths: &mut Vec<PathBuf>,
            depth: usize,
        ) -> Result<(), String> {
            if depth > 64 {
                return Err("Project nesting exceeds 64 levels".into());
            }
            for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let file_type = entry.file_type().map_err(|e| e.to_string())?;
                let name = entry.file_name();
                if [".git", "node_modules", "target", ".DS_Store"]
                    .iter()
                    .any(|s| name == *s)
                {
                    continue;
                }
                if file_type.is_dir() {
                    walk(root, &entry.path(), paths, depth + 1)?;
                } else {
                    paths.push(entry.path().strip_prefix(root).unwrap().to_owned());
                }
                if paths.len() > MAX_FILES {
                    return Err("Project exceeds 20,000 files".into());
                }
            }
            Ok(())
        }
        walk(&root, &root, &mut paths, 0)?;
    }
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_FILES {
        return Err("Project exceeds 20,000 files".into());
    }
    let mut result = Snapshot {
        root: root.clone(),
        manifest: Manifest {
            name,
            files: Vec::new(),
            branch: None,
            history,
        },
        bundle: None,
        exclusions: exclusions.to_vec(),
    };
    let mut total = 0u64;
    for path in paths {
        let path_str = path.to_str().ok_or("Non UTF-8 project path")?;
        if exclusions
            .iter()
            .any(|s| path_str == s || path_str.starts_with(&format!("{s}/")))
        {
            continue;
        }
        if !relative(path_str) || path_str == BUNDLE {
            return Err("Unsupported project path".into());
        }
        let full = root.join(&path);
        // A tracked file may have been deleted; its absence is part of the snapshot.
        let metadata = match fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        let mut ancestor = root.clone();
        for part in path.components() {
            ancestor.push(part);
            if fs::symlink_metadata(&ancestor)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err(format!(
                    "Symbolic links need explicit handling before copying: {path_str}"
                ));
            }
        }
        if !metadata.is_file() {
            return Err(format!(
                "Submodules/special files must be copied separately: {path_str}"
            ));
        }
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        total = total
            .checked_add(metadata.len())
            .filter(|n| *n <= MAX_BYTES)
            .ok_or("Project exceeds 1 GiB; exclude generated files")?;
        result.manifest.files.push(Entry {
            path: path_str.into(),
            size: metadata.len(),
            sha256: digest_file(open_source(&root, path_str)?)?,
            executable,
        });
    }
    if history {
        let branch = git(&root, &["symbolic-ref", "--short", "HEAD"])?;
        let branch = branch.trim().to_owned();
        if !valid_branch(&branch) {
            return Err("Detached HEAD or unsupported branch; copy files only".into());
        }
        fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
        let bundle = scratch.join(format!("dunlin-{}.bundle", uuid::Uuid::now_v7()));
        result.bundle = Some(bundle.clone());
        git(
            &root,
            &[
                "bundle",
                "create",
                bundle.to_str().ok_or("Invalid temporary path")?,
                "--branches",
                "--tags",
            ],
        )?;
        result.manifest.branch = Some(branch);
        result.manifest.files.push(Entry {
            path: BUNDLE.into(),
            size: fs::metadata(&bundle).map_err(|e| e.to_string())?.len(),
            sha256: digest(&bundle)?,
            executable: false,
        });
    }
    validate(&result.manifest)?;
    Ok(result)
}

pub fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A history bundle can contain secrets absent from the working snapshot.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o077);
                Ok(())
            });
        }
    }
    command
        .arg("-C")
        .arg(root)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    let (status, bytes) = crate::github::process::run(
        command,
        None,
        std::time::Duration::from_secs(120),
        8 * 1024 * 1024,
    )?;
    if !status.success() {
        return Err("Git snapshot operation failed; check repository state".into());
    }
    String::from_utf8(bytes).map_err(|_| "Non UTF-8 Git output".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_git_metadata_and_case_collisions() {
        for value in ["../a", "/a", ".git/config", "a/.GIT/config", "a\\b"] {
            assert!(!relative(value));
        }
        let entry = Entry {
            path: "file".into(),
            size: 1,
            sha256: "a".repeat(64),
            executable: false,
        };
        let mut manifest = Manifest {
            name: "demo".into(),
            files: vec![entry.clone()],
            branch: None,
            history: false,
        };
        assert_eq!(validate(&manifest).unwrap(), 1);
        manifest.files.push(Entry {
            path: "FILE".into(),
            ..entry
        });
        assert!(validate(&manifest).is_err());
    }

    #[test]
    fn snapshot_preserves_working_changes_without_copying_git_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]).unwrap();
        fs::write(root.join("file.txt"), "original").unwrap();
        fs::write(root.join("deleted.txt"), "delete me").unwrap();
        fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(
            &root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "initial",
            ],
        )
        .unwrap();
        fs::write(root.join("file.txt"), "changed").unwrap();
        fs::write(root.join("untracked.txt"), "new").unwrap();
        fs::write(root.join("ignored.txt"), "private").unwrap();
        fs::remove_file(root.join("deleted.txt")).unwrap();
        let snapshot = snapshot(&root, "copy".into(), true, false, &[], dir.path()).unwrap();
        assert_eq!(snapshot.manifest.branch.as_deref(), Some("main"));
        let names: Vec<_> = snapshot
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(
            names.contains(&"file.txt")
                && names.contains(&"untracked.txt")
                && names.contains(&BUNDLE)
        );
        assert!(!names.contains(&"ignored.txt") && !names.contains(&"deleted.txt"));
        assert!(!names.iter().any(|name| name.starts_with(".git/")));
        assert_eq!(
            fs::read_to_string(root.join("file.txt")).unwrap(),
            "changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_resolution_rejects_symlinked_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(dir.path().join("secret"), "never copy").unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("linked")).unwrap();
        assert!(open_source(&root, "linked/secret").is_err());
    }
}
