//! Explicit saved-file baselines: HEAD objects + exact dirty/untracked overlays.
//! Never attributes changes to an agent, mutates the index or silently resets Git.
use crate::git::{history, operations};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const TOTAL_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub base64: String,
    pub mode: u32,
}
impl Image {
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        STANDARD
            .decode(&self.base64)
            .map_err(|_| "Invalid checkpoint data".into())
    }
    pub fn text(&self) -> Option<String> {
        let bytes = self.bytes().ok()?;
        if bytes.contains(&0) {
            return None;
        }
        String::from_utf8(bytes).ok()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub root: String,
    pub head: String,
    pub created_ms: u64,
    pub label: String,
    identity: String,
    overlay: BTreeMap<String, Option<Image>>,
    modes: BTreeMap<String, u32>,
}
#[derive(Clone, Debug)]
pub struct Change {
    pub path: String,
    pub before: Option<Image>,
    pub after: Option<Image>,
}
pub struct Restore {
    pub checkpoint: Checkpoint,
    pub change: Change,
    prepared: std::time::Instant,
}
fn identity(root: &str) -> Result<String, String> {
    let common = operations::directory(root, "--git-common-dir")?;
    let identity = |path: &str| -> Result<String, String> {
        let meta = fs::metadata(path).map_err(|_| "Repository identity unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(format!("{}:{}", meta.dev(), meta.ino()))
        }
        #[cfg(not(unix))]
        {
            Ok(format!(
                "{:?}",
                meta.created()
                    .map_err(|_| "Repository identity unavailable")?
            ))
        }
    };
    Ok(format!(
        "{}:{common}:{}",
        identity(root)?,
        identity(&common)?
    ))
}
fn read(root: &str, args: &[&str]) -> Result<String, String> {
    let mut flags = vec!["--no-replace-objects"];
    flags.extend_from_slice(args);
    operations::read(root, &flags)
}
fn mode(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0o644
    }
}
fn modes(root: &str, started: std::time::Instant) -> Result<BTreeMap<String, u32>, String> {
    let names = read(root, &["ls-files", "-z"])?;
    let mut result = BTreeMap::new();
    for name in names.split_terminator('\0') {
        if result.len() >= 20_000 || started.elapsed() > Duration::from_secs(10) {
            return Err("File-mode inventory exceeded the checkpoint work limit (20,000 files / ten seconds)".into());
        }
        if !relative(name) {
            return Err("Invalid tracked checkpoint path".into());
        }
        let Ok(path) = path(root, name) else {
            continue;
        };
        if let Ok(meta) = fs::symlink_metadata(path)
            && meta.is_file()
        {
            result.insert(name.into(), mode(&meta));
        }
    }
    Ok(result)
}
fn head(root: &str) -> Result<String, String> {
    let value = read(root, &["rev-parse", "--verify", "HEAD"])?
        .trim()
        .to_owned();
    if !crate::git::operations::oid(&value) {
        return Err("Checkpoint needs a HEAD commit".into());
    }
    Ok(value)
}
fn current(checkpoint: &Checkpoint) -> Result<(), String> {
    if checkpoint.modes.len() > 20_000
        || checkpoint.modes.keys().any(|name| !relative(name))
        || !crate::git::operations::oid(&checkpoint.head)
        || checkpoint.overlay.len() > 2000
        || checkpoint.overlay.iter().any(|(name, image)| {
            !relative(name)
                || image
                    .as_ref()
                    .is_some_and(|i| i.base64.len() as u64 > 4 * FILE_LIMIT / 3 + 8)
        })
    {
        return Err("Invalid checkpoint metadata".into());
    }
    history::repository_root(&checkpoint.root, Some(&checkpoint.root))?;
    if identity(&checkpoint.root)? != checkpoint.identity {
        return Err("Checkpoint repository was replaced; restore is refused".into());
    }
    Ok(())
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|c| matches!(c,Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
}
fn path(root: &str, relative_path: &str) -> Result<PathBuf, String> {
    if !relative(relative_path) {
        return Err(
            "Checkpoint paths must stay outside Git metadata and inside the project".into(),
        );
    }
    let mut full = Path::new(root).to_owned();
    for component in Path::new(relative_path).components() {
        full.push(component);
        match fs::symlink_metadata(&full) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Checkpoint does not follow symlinks".into());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Checkpoint path unavailable".into()),
            _ => {}
        }
    }
    Ok(full)
}
pub(crate) fn image(root: &str, name: &str) -> Result<Option<Image>, String> {
    let full = path(root, name)?;
    let meta = match fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("File unavailable".into()),
    };
    if !meta.is_file() || meta.len() > FILE_LIMIT {
        return Err("Checkpoint supports regular files up to 2 MiB; submodules, symlinks and larger files need manual review".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut bytes = Vec::new();
    options
        .open(full)
        .map_err(|_| "Could not open checkpoint file")?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read checkpoint file")?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("Checkpoint file grew beyond its limit".into());
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    Ok(Some(Image {
        base64: STANDARD.encode(bytes),
        mode,
    }))
}
fn names(root: &str, base: &str) -> Result<BTreeSet<String>, String> {
    let changed = read(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-only",
            "-z",
            base,
            "--",
        ],
    )?;
    let untracked = read(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let names = changed
        .split_terminator('\0')
        .chain(untracked.split_terminator('\0'))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if names.len() > 2000 || names.iter().any(|p| !relative(p)) {
        return Err("Checkpoint review is limited to 2,000 safe changed paths".into());
    }
    Ok(names)
}
pub fn capture(root: &str, label: &str) -> Result<Checkpoint, String> {
    let started = std::time::Instant::now();
    if label.trim().is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
        return Err("Choose a checkpoint label up to 256 bytes".into());
    }
    let root = history::repository_root(root, None)?;
    let head = head(&root)?;
    let identity = identity(&root)?;
    let paths = names(&root, &head)?;
    let flags = read(&root, &["ls-files", "-v", "-z"])?;
    if flags.split_terminator('\0').any(|s| {
        s.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')
    }) {
        return Err("Remove hidden index flags before capturing a checkpoint".into());
    }
    let mut overlay = BTreeMap::new();
    let modes = modes(&root, started)?;
    let mut size = 0;
    for name in &paths {
        if started.elapsed() > Duration::from_secs(10) {
            return Err(
                "Checkpoint capture exceeded its work budget; no baseline was created".into(),
            );
        }
        let image = image(&root, name)?;
        size += image.as_ref().map_or(0, |i| i.base64.len());
        if size > TOTAL_LIMIT {
            return Err("Checkpoint overlay exceeds 16 MiB encoded size".into());
        }
        overlay.insert(name.clone(), image);
    }
    let checkpoint = Checkpoint {
        id: uuid::Uuid::now_v7().to_string(),
        root: root.clone(),
        head: head.clone(),
        created_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        label: label.into(),
        identity,
        overlay,
        modes,
    };
    current(&checkpoint)?;
    if self::head(&root)? != head || names(&root, &head)? != paths {
        return Err("Repository changed while capturing the baseline; capture again".into());
    }
    for (name, old) in &checkpoint.overlay {
        if &image(&root, name)? != old {
            return Err("File changed while capturing the baseline; capture again".into());
        }
    }
    if self::modes(&root, started)? != checkpoint.modes {
        return Err("File modes changed during capture; no baseline was created".into());
    }
    Ok(checkpoint)
}
fn baseline(checkpoint: &Checkpoint, name: &str) -> Result<Option<Image>, String> {
    if let Some(image) = checkpoint.overlay.get(name) {
        return Ok(image.clone());
    }
    let tree = read(
        &checkpoint.root,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "-z",
            &checkpoint.head,
            "--",
            name,
        ],
    )?;
    if tree.is_empty() {
        return Ok(None);
    }
    let (header, _) = tree.split_once('\t').ok_or("Invalid baseline tree entry")?;
    let fields = header.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 3
        || fields[1] != "blob"
        || !matches!(fields[0], "100644" | "100755")
        || !crate::git::operations::oid(fields[2])
    {
        return Err("Baseline path is not a regular file".into());
    }
    let mut command = Command::new("git");
    command
        .args([
            "-C",
            &checkpoint.root,
            "--no-replace-objects",
            "cat-file",
            "blob",
            fields[2],
        ])
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    let (status, bytes) =
        crate::git::activity::bounded_command(command, Duration::from_secs(2), FILE_LIMIT)?;
    if !status.success() {
        return Err("Baseline Git object unavailable; no automatic fetch or recovery".into());
    }
    Ok(Some(Image {
        base64: STANDARD.encode(bytes),
        mode: checkpoint
            .modes
            .get(name)
            .copied()
            .unwrap_or(if fields[0] == "100755" { 0o755 } else { 0o644 }),
    }))
}
pub fn changes(checkpoint: &Checkpoint) -> Result<Vec<Change>, String> {
    let started = std::time::Instant::now();
    current(checkpoint)?;
    let mut candidates = names(&checkpoint.root, &checkpoint.head)?;
    for (name, old) in &checkpoint.modes {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("Mode comparison exceeded the checkpoint work budget".into());
        }
        if let Ok(path) = path(&checkpoint.root, name)
            && let Ok(meta) = fs::symlink_metadata(path)
            && meta.is_file()
            && mode(&meta) != *old
        {
            candidates.insert(name.clone());
        }
    }
    candidates.extend(checkpoint.overlay.keys().cloned());
    if candidates.len() > 2000 {
        return Err("Too many changed files to review safely".into());
    }
    let mut changes = Vec::new();
    let mut size = 0;
    for name in candidates {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("Review exceeded its work budget; no incomplete change list is presented as complete".into());
        }
        let before = baseline(checkpoint, &name)?;
        let after = image(&checkpoint.root, &name)?;
        size += before.as_ref().map_or(0, |i| i.base64.len())
            + after.as_ref().map_or(0, |i| i.base64.len());
        if size > 2 * TOTAL_LIMIT {
            return Err("Review exceeds 32 MiB encoded data".into());
        }
        if before != after {
            changes.push(Change {
                path: name,
                before,
                after,
            });
        }
    }
    current(checkpoint)?;
    Ok(changes)
}
pub fn prepare_restore(checkpoint: &Checkpoint, name: &str) -> Result<Restore, String> {
    current(checkpoint)?;
    if !relative(name) {
        return Err("Invalid restore path".into());
    }
    Ok(Restore {
        checkpoint: checkpoint.clone(),
        change: Change {
            path: name.into(),
            before: baseline(checkpoint, name)?,
            after: image(&checkpoint.root, name)?,
        },
        prepared: std::time::Instant::now(),
    })
}
pub fn restore(plan: Restore) -> Result<(), String> {
    if plan.prepared.elapsed() > Duration::from_secs(300) {
        return Err("Restore preview expired; prepare a fresh preview".into());
    }
    let checkpoint = &plan.checkpoint;
    current(checkpoint)?;
    let common = operations::directory(&checkpoint.root, "--git-common-dir")?;
    let _lock = operations::lock(&common)?;
    if image(&checkpoint.root, &plan.change.path)? != plan.change.after {
        return Err("File changed after restore preview. No file was restored.".into());
    }
    let destination = path(&checkpoint.root, &plan.change.path)?;
    let parent = destination.parent().ok_or("Missing restore parent")?;
    if !parent.is_dir() {
        return Err(
            "Restore requires an existing parent directory; no folders are created implicitly"
                .into(),
        );
    }
    match &plan.change.before {
        Some(before) => {
            let bytes = before.bytes()?;
            if bytes.len() as u64 > FILE_LIMIT {
                return Err("Oversized checkpoint image".into());
            }
            let temp = parent.join(format!(".dunlin-restore-{}", uuid::Uuid::now_v7()));
            let result = (|| {
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options
                    .open(&temp)
                    .map_err(|_| "Could not create restore temporary file")?;
                file.write_all(&bytes)
                    .map_err(|_| "Could not write restored contents")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(fs::Permissions::from_mode(before.mode & 0o777))
                        .map_err(|_| "Could not restore file mode")?;
                }
                file.sync_all()
                    .map_err(|_| "Could not sync restored file")?;
                current(checkpoint)?;
                if image(&checkpoint.root, &plan.change.path)? != plan.change.after {
                    return Err("File changed before replacement; restore cancelled".into());
                }
                fs::rename(&temp, &destination)
                    .map_err(|_| "Could not atomically replace restored file")?;
                Ok::<_, String>(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            result?;
        }
        None => {
            fs::remove_file(&destination)
                .map_err(|_| "Could not remove the explicitly selected new file")?;
        }
    }
    Ok(())
}
pub fn fingerprint(checkpoint: &Checkpoint) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(checkpoint).unwrap_or_default())
    )
}

#[cfg(test)]
mod tests;
