//! Local checkout identity validation, independent of worker catalog/grants.
//! Captures pin identities, not filesystem or execution authority. Metadata roots
//! remain private and are never supplied as extra roots to a filesystem tool.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("Worker export filesystem unavailable")]
    Io(#[from] std::io::Error),
    #[error("Invalid worker export record")]
    Json(#[from] serde_json::Error),
}
pub(crate) type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepoKind {
    Directory,
    Git,
    LinkedWorktree,
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
#[derive(Clone)]
pub(crate) struct LinkedRoots {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Directory {
    path: PathBuf,
    device: u64,
    inode: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    root: Directory,
    git: Option<Directory>,
    common: Option<Directory>,
    objects: Option<Directory>,
    repo_kind: RepoKind,
    settings_digest: String,
}
/// Opaque local capture, not deserializable from controller or browser JSON.
#[derive(Clone)]
pub(crate) struct CapturedCheckout(Record);
fn canonical(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "Checkout roots must be absolute operator-local paths",
        ));
    }
    let path = fs::canonicalize(path)?;
    if path.to_str().is_none() {
        return Err(Error::Invalid("Checkout roots must be UTF-8"));
    }
    Ok(path)
}
#[cfg(unix)]
fn directory(path: &Path) -> Result<Directory> {
    use std::os::unix::fs::MetadataExt;
    let path = canonical(path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Invalid(
            "Checkout metadata roots must be real directories",
        ));
    }
    Ok(Directory {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
#[cfg(not(unix))]
fn directory(_path: &Path) -> Result<Directory> {
    Err(Error::Invalid(
        "Worker checkout exports require macOS or Linux",
    ))
}
fn regular(path: &Path) -> Result<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err(Error::Invalid(
            "Invalid or oversized checkout metadata file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(Error::Invalid("Checkout metadata file exceeded its budget"));
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid("Checkout metadata must be UTF-8"))
}
fn link_target(base: &Path, value: &str) -> Result<PathBuf> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(Error::Invalid("Invalid Git metadata pointer"));
    }
    let target = Path::new(value);
    canonical(&if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    })
}
fn objects_root(common: &Path) -> Result<Directory> {
    let path = common.join("objects");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Invalid(
            "External or symlinked Git object roots need a separate explicit adapter",
        ));
    }
    let objects = directory(&path)?;
    if objects.path.parent() != Some(common) {
        return Err(Error::Invalid(
            "Git object root escapes its approved metadata directory",
        ));
    }
    Ok(objects)
}
fn reject_alternates(common: &Path) -> Result<()> {
    let path = common.join("objects/info/alternates");
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            if !regular(&path)?.trim().is_empty() {
                return Err(Error::Invalid(
                    "External Git object roots need a separate explicit adapter",
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
/// Git can discover a bare repository without a .git entry, including through
/// an ancestor. This adapter has no bare-repository authority contract. Treat
/// even symlinked/suspicious marker layouts as unsupported rather than silently
/// routing ordinary directory authority into repository metadata.
fn reject_bare_layout(path: &Path) -> Result<()> {
    let mut markers = 0;
    for name in ["HEAD", "objects", "refs"] {
        match fs::symlink_metadata(path.join(name)) {
            Ok(_) => markers += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if markers == 3 {
        return Err(Error::Invalid(
            "Bare repositories and nested bare directories require a separate explicit adapter",
        ));
    }
    Ok(())
}
impl CapturedCheckout {
    pub fn capture(
        root: &Path,
        settings_digest: &str,
        linked: Option<&LinkedRoots>,
    ) -> Result<Self> {
        if !digest(settings_digest) {
            return Err(Error::Invalid(
                "A lowercase SHA-256 settings digest is required",
            ));
        }
        let root = directory(root)?;
        let marker = root.path.join(".git");
        let (git, common, objects, repo_kind) = match fs::symlink_metadata(&marker) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if linked.is_some() {
                    return Err(Error::Invalid(
                        "Linked roots require a linked checkout .git file",
                    ));
                }
                for parent in root.path.ancestors() {
                    reject_bare_layout(parent)?;
                    if parent == root.path.as_path() {
                        continue;
                    }
                    match fs::symlink_metadata(parent.join(".git")) {
                        Ok(_) => {
                            return Err(Error::Invalid(
                                "Export the repository root, not an implicit nested checkout",
                            ));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                (None, None, None, RepoKind::Directory)
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                if linked.is_some() {
                    return Err(Error::Invalid(
                        "Explicit linked roots were supplied for a normal Git checkout",
                    ));
                }
                match fs::symlink_metadata(marker.join("commondir")) {
                    Ok(_) => {
                        return Err(Error::Invalid(
                            "Shared Git metadata requires explicit linked roots",
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                let git = directory(&marker)?;
                regular(&git.path.join("HEAD"))?;
                let objects = objects_root(&git.path)?;
                reject_alternates(&git.path)?;
                (Some(git), None, Some(objects), RepoKind::Git)
            }
            Ok(metadata) if metadata.is_file() => {
                let linked = linked.ok_or(Error::Invalid(
                    "Linked checkout requires explicit operator-approved git-dir and common-dir",
                ))?;
                let pointer = regular(&marker)?;
                let pointer = pointer
                    .trim()
                    .strip_prefix("gitdir: ")
                    .ok_or(Error::Invalid("Invalid checkout .git pointer"))?;
                let git = directory(&linked.git_dir)?;
                let common = directory(&linked.common_dir)?;
                if link_target(&root.path, pointer)? != git.path {
                    return Err(Error::Conflict(
                        "Checkout git-dir differs from the operator-approved root",
                    ));
                }
                if link_target(&git.path, &regular(&git.path.join("commondir"))?)? != common.path {
                    return Err(Error::Conflict(
                        "Checkout common-dir differs from the operator-approved root",
                    ));
                }
                if link_target(&git.path, &regular(&git.path.join("gitdir"))?)?
                    != canonical(&marker)?
                {
                    return Err(Error::Conflict(
                        "Git worktree backlink does not identify this checkout",
                    ));
                }
                if git.path.parent()
                    != Some(directory(&common.path.join("worktrees"))?.path.as_path())
                {
                    return Err(Error::Invalid("Unsupported Git worktree metadata layout"));
                }
                regular(&git.path.join("HEAD"))?;
                regular(&common.path.join("HEAD"))?;
                let objects = objects_root(&common.path)?;
                reject_alternates(&common.path)?;
                (
                    Some(git),
                    Some(common),
                    Some(objects),
                    RepoKind::LinkedWorktree,
                )
            }
            Ok(_) => return Err(Error::Invalid("Unsupported checkout .git entry")),
        };
        Ok(Self(Record {
            root,
            git,
            common,
            objects,
            repo_kind,
            settings_digest: settings_digest.into(),
        }))
    }
    pub fn revalidate(&self) -> Result<()> {
        self.revalidate_at(&self.0.root.path)
    }
    /// The native project may retain a symlink spelling. Re-resolve that exact
    /// original path instead of validating only the formerly canonical target.
    pub(crate) fn revalidate_at(&self, path: &Path) -> Result<()> {
        let linked = match (&self.0.common, &self.0.git) {
            (Some(common), Some(git)) => Some(LinkedRoots {
                git_dir: git.path.clone(),
                common_dir: common.path.clone(),
            }),
            (None, _) => None,
            _ => return Err(Error::Invalid("Invalid linked checkout record")),
        };
        let current = Self::capture(path, &self.0.settings_digest, linked.as_ref())?;
        if current.0 != self.0 {
            return Err(Error::Conflict(
                "Worker checkout identity changed; explicit rebind and regrant required",
            ));
        }
        Ok(())
    }
    pub(crate) fn validate_initialized_git_at(&self, path: &Path) -> Result<()> {
        if self.0.repo_kind != RepoKind::Directory {
            return Err(Error::Conflict(
                "Git init requires an originally ordinary directory",
            ));
        }
        let current = Self::capture(path, &self.0.settings_digest, None)?;
        if current.0.repo_kind != RepoKind::Git || current.0.root != self.0.root {
            return Err(Error::Conflict(
                "Git init changed the checkout root or selected external metadata",
            ));
        }
        Ok(())
    }
    /// A single Git-owned file, never a workspace filesystem root. Directory
    /// descriptors prevent a symlinked info/exclude from selecting host files.
    #[cfg(unix)]
    pub(crate) fn open_git_exclude(&self) -> Result<Option<fs::File>> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let Some(directory) = self.0.common.as_ref().or(self.0.git.as_ref()) else {
            return Ok(None);
        };
        let metadata = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&directory.path)?;
        let pinned = metadata.metadata()?;
        if !pinned.is_dir() || pinned.dev() != directory.device || pinned.ino() != directory.inode {
            return Err(Error::Conflict("Git metadata directory identity changed"));
        }
        let mkdir = unsafe { libc::mkdirat(metadata.as_raw_fd(), c"info".as_ptr(), 0o700) };
        if mkdir != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
        }
        let info = unsafe {
            libc::openat(
                metadata.as_raw_fd(),
                c"info".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if info < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let info = unsafe { fs::File::from_raw_fd(info) };
        let file = unsafe {
            libc::openat(
                info.as_raw_fd(),
                c"exclude".as_ptr(),
                libc::O_RDWR
                    | libc::O_APPEND
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        };
        if file < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { fs::File::from_raw_fd(file) };
        let attributes = file.metadata()?;
        if !attributes.is_file() || attributes.nlink() != 1 || attributes.len() > 1024 * 1024 {
            return Err(Error::Invalid(
                "Git exclude must be a bounded private regular file",
            ));
        }
        Ok(Some(file))
    }
    #[cfg(not(unix))]
    pub(crate) fn open_git_exclude(&self) -> Result<Option<fs::File>> {
        Err(Error::Invalid("Git metadata writes require macOS or Linux"))
    }
    pub(crate) fn canonical_root(&self) -> &Path {
        &self.0.root.path
    }
    /// Data only. Only a freshly authorized native checkout may turn these
    /// private canonical paths into typed sandbox metadata roots.
    pub(crate) fn captured_metadata_paths(&self) -> Vec<PathBuf> {
        if self.0.repo_kind != RepoKind::LinkedWorktree {
            return Vec::new();
        }
        let mut paths = self
            .0
            .git
            .iter()
            .chain(self.0.common.iter())
            .map(|v| v.path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }
    pub fn repo_kind(&self) -> RepoKind {
        self.0.repo_kind
    }
    pub fn settings_digest(&self) -> &str {
        &self.0.settings_digest
    }
    pub fn identity_digest(&self) -> Result<String> {
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&self.0)?)))
    }
    pub(crate) fn encoded(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.0)?)
    }
    /// Trusted operator-local persistence only. Decoding grants no authority;
    /// callers must revalidate identity and their separate policy/grants.
    pub(crate) fn decoded(raw: String) -> Result<Self> {
        Ok(Self(serde_json::from_str(&raw)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const LEGACY_JSON: &str = r#"{"root":{"path":"/fixture/root","device":7,"inode":11},"git":null,"common":null,"objects":null,"repo_kind":"directory","settings_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
    const LEGACY_DIGEST: &str = "5a7d00e437a59d8b1e24e001da7382167ebf8660b62152878b55fc082438a8eb";

    #[test]
    fn persisted_identity_encoding_and_digest_remain_byte_compatible() {
        let captured = CapturedCheckout(Record {
            root: Directory {
                path: "/fixture/root".into(),
                device: 7,
                inode: 11,
            },
            git: None,
            common: None,
            objects: None,
            repo_kind: RepoKind::Directory,
            settings_digest: "a".repeat(64),
        });
        assert_eq!(captured.encoded().unwrap(), LEGACY_JSON);
        assert_eq!(captured.identity_digest().unwrap(), LEGACY_DIGEST);
        let reopened = CapturedCheckout::decoded(LEGACY_JSON.to_owned()).unwrap();
        assert_eq!(reopened.encoded().unwrap(), LEGACY_JSON);
        assert_eq!(reopened.identity_digest().unwrap(), LEGACY_DIGEST);
        for (kind, wire) in [
            (RepoKind::Directory, "directory"),
            (RepoKind::Git, "git"),
            (RepoKind::LinkedWorktree, "linked_worktree"),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
        }
    }

    #[test]
    fn trusted_persistence_codec_still_rejects_unknown_record_and_directory_fields() {
        for raw in [
            LEGACY_JSON.replacen("{", "{\"authority\":true,", 1),
            LEGACY_JSON.replacen(
                "\"device\":7",
                "\"device\":7,\"extra_root\":\"/private\"",
                1,
            ),
        ] {
            assert!(matches!(
                CapturedCheckout::decoded(raw).err().unwrap(),
                Error::Json(_)
            ));
        }
    }
    #[test]
    fn actual_bare_repository_and_nested_directory_never_receive_ordinary_authority() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        let output = std::process::Command::new("git")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "init",
                "--bare",
                "--quiet",
            ])
            .arg(&bare)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Local bare fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let nested = bare.join("nested/deeper");
        fs::create_dir_all(&nested).unwrap();
        for path in [&bare, &nested] {
            assert!(matches!(
                CapturedCheckout::capture(path, &"a".repeat(64), None),
                Err(Error::Invalid(_))
            ));
        }
        let ordinary = dir.path().join("ordinary");
        fs::create_dir(&ordinary).unwrap();
        fs::write(ordinary.join("README"), "unrelated files").unwrap();
        fs::write(ordinary.join("HEAD"), "personal notes, not Git metadata").unwrap();
        let captured = CapturedCheckout::capture(&ordinary, &"a".repeat(64), None).unwrap();
        assert_eq!(captured.repo_kind(), RepoKind::Directory);
        captured.revalidate().unwrap();
    }
}
