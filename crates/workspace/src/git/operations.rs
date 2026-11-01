//! Explicit two-phase Git mutations. Plans bind the exact root, common Git dir,
//! refs, index, dirty contents, operation metadata and worktree inventory. Execution
//! takes a common-dir Dunlin lock, revalidates and never retries or rolls back.
use super::{
    activity::git_limited,
    history::repository_root,
    overview::{self, Operation, Overview},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const LIMIT: u64 = 8 * 1024 * 1024;
const CONTENT_LIMIT: u64 = 64 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    StashPush {
        message: String,
        include_untracked: bool,
    },
    StashApply {
        oid: String,
    },
    StashPop {
        oid: String,
    },
    StashDrop {
        oid: String,
    },
    WorktreeAdd {
        name: String,
        revision: String,
    },
    WorktreeRemove {
        path: String,
    },
    WorktreePrune,
    Merge {
        revision: String,
    },
    Rebase {
        revision: String,
    },
    CherryPick {
        revision: String,
        mainline: Option<usize>,
    },
    Continue {
        operation: Operation,
    },
    Abort {
        operation: Operation,
    },
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::StashPush { .. } => "Stash changes",
            Self::StashApply { .. } => "Apply stash (restore index)",
            Self::StashPop { .. } => "Pop stash (drop only after success)",
            Self::StashDrop { .. } => "Drop stash",
            Self::WorktreeAdd { .. } => "Add detached worktree",
            Self::WorktreeRemove { .. } => "Remove clean worktree",
            Self::WorktreePrune => "Prune stale worktree metadata",
            Self::Merge { .. } => "Merge",
            Self::Rebase { .. } => "Rebase",
            Self::CherryPick { .. } => "Cherry-pick",
            Self::Continue { .. } => "Continue operation",
            Self::Abort { .. } => "Abort operation",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub root: String,
    pub common: String,
    pub fingerprint: String,
    pub action: Action,
    pub head: Option<String>,
    pub target: Option<String>,
    pub worktree_target: Option<String>,
    pub status: String,
    pub overview: Overview,
    pub warning: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub success: bool,
    pub message: String,
    pub overview: Overview,
}

pub(crate) fn read(root: &str, args: &[&str]) -> Result<String, String> {
    let filters = git_limited(
        root,
        &[
            "-c",
            "core.fsmonitor=false",
            "config",
            "--null",
            "--name-only",
            "--list",
        ],
        LIMIT,
    )?;
    let mut overrides = Vec::new();
    for key in filters.split_terminator('\0') {
        if let Some((prefix, kind)) = key.rsplit_once('.')
            && prefix.starts_with("filter.")
            && matches!(kind, "clean" | "smudge" | "process" | "required")
        {
            for name in ["clean", "smudge", "process"] {
                overrides.push(format!("{prefix}.{name}="));
            }
            overrides.push(format!("{prefix}.required=false"));
        }
    }
    overrides.sort();
    overrides.dedup();
    let mut options = vec![
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "diff.external=",
        "--literal-pathspecs",
    ];
    for value in &overrides {
        options.extend(["-c", value.as_str()]);
    }
    options.extend_from_slice(args);
    git_limited(root, &options, LIMIT)
}
pub(crate) fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
fn commit(root: &str, revision: &str) -> Result<String, String> {
    if revision.is_empty()
        || revision.len() > 512
        || revision.starts_with('-')
        || revision.chars().any(char::is_control)
    {
        return Err("Choose an explicit local commit, branch or tag".into());
    }
    let value = read(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?
    .trim()
    .to_string();
    if !oid(&value) {
        return Err("Invalid commit".into());
    }
    Ok(value)
}
pub(crate) fn directory(root: &str, flag: &str) -> Result<String, String> {
    let raw = read(root, &["rev-parse", "--path-format=absolute", flag])?;
    let path =
        fs::canonicalize(raw.trim_end_matches('\n')).map_err(|_| "Git directory is unavailable")?;
    path.to_str()
        .map(str::to_owned)
        .ok_or("Git directory is not UTF-8".into())
}
fn locks(root: &str) -> Result<(), String> {
    for dir in [
        directory(root, "--git-common-dir")?,
        directory(root, "--git-dir")?,
    ] {
        for file in [
            "index.lock",
            "HEAD.lock",
            "packed-refs.lock",
            "config.lock",
            "shallow.lock",
            "refs/stash.lock",
            "logs/refs/stash.lock",
        ] {
            if fs::symlink_metadata(Path::new(&dir).join(file)).is_ok() {
                return Err("Git is locked by another process. Finish it first.".into());
            }
        }
    }
    Ok(())
}
fn digest_path(hash: &mut Sha256, path: &Path, budget: &mut u64) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            hash.update(b"absent");
            Ok(())
        }
        Err(_) => Err("Cannot fingerprint repository state".into()),
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                hash.update(meta.permissions().mode().to_le_bytes());
            }
            if meta.file_type().is_symlink() {
                hash.update(b"symlink");
                hash.update(
                    fs::read_link(path)
                        .map_err(|_| "Cannot read symlink identity")?
                        .as_os_str()
                        .as_encoded_bytes(),
                );
                return Ok(());
            }
            if !meta.is_file() {
                return Err("Unsupported dirty path or submodule. Use your Git client.".into());
            }
            if meta.len() > *budget {
                return Err("Repository changes exceed the 64 MiB safety snapshot limit".into());
            }
            let mut bytes = Vec::new();
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            options
                .open(path)
                .map_err(|_| "Cannot read repository state without following a replaced symlink")?
                .take(*budget + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read repository state")?;
            if bytes.len() as u64 > *budget {
                return Err("Repository state grew beyond the safety limit".into());
            }
            *budget -= bytes.len() as u64;
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            Ok(())
        }
    }
}
fn hash_metadata(
    hash: &mut Sha256,
    dir: &Path,
    depth: usize,
    budget: &mut u64,
) -> Result<(), String> {
    if depth > 8 {
        return Err("Git operation metadata is too deep".into());
    }
    let meta = match fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            hash.update(b"absent");
            return Ok(());
        }
        Err(_) => return Err("Cannot read Git operation metadata".into()),
    };
    if meta.file_type().is_symlink() {
        return Err("Symlinked operation metadata is not supported".into());
    }
    if !meta.is_dir() {
        return digest_path(hash, dir, budget);
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|_| "Cannot read Git operation metadata")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Cannot read Git operation metadata")?;
    if entries.len() > 4096 {
        return Err("Too many Git operation metadata entries".into());
    }
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        hash.update(e.file_name().as_encoded_bytes());
        hash_metadata(hash, &e.path(), depth + 1, budget)?;
    }
    Ok(())
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn identity(hash: &mut Sha256, path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|_| "Repository identity is unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hash.update(metadata.dev().to_le_bytes());
        hash.update(metadata.ino().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hash.update(format!(
            "{:?}",
            metadata
                .created()
                .map_err(|_| "Repository creation identity is unavailable")?
        ));
    }
    Ok(())
}
fn snapshot(root: &str, inventory: &Overview) -> Result<(String, String), String> {
    let status = read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut hash = Sha256::new();
    hash.update(root);
    identity(&mut hash, Path::new(root))?;
    identity(&mut hash, Path::new(&directory(root, "--git-common-dir")?))?;
    hash.update(serde_json::to_vec(inventory).map_err(|_| "Cannot encode Git state")?);
    hash.update(read(root, &["show-ref", "--head"]).unwrap_or_default());
    hash.update(read(root, &["symbolic-ref", "-q", "HEAD"]).unwrap_or_default());
    hash.update(read(
        root,
        &["config", "--null", "--list", "--show-origin"],
    )?);
    hash.update(&status);
    hash.update(ignored(root)?);
    // Pin semantic index entries and flags, not the raw index's mtime/ctime
    // cache. Ordinary status/diff refreshes may rewrite only that cache between
    // approval and execution without changing anything the user authorized.
    hash.update(read(root, &["ls-files", "--stage", "-z"])?);
    hash.update(read(root, &["ls-files", "-v", "-z"])?);
    let mut budget = CONTENT_LIMIT;
    let dir = directory(root, "--git-dir")?;
    for name in [
        "HEAD",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "MERGE_MSG",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        hash.update(name);
        hash_metadata(&mut hash, &Path::new(&dir).join(name), 0, &mut budget)?;
    }
    // Include the actual contents of all dirty/untracked paths, not just their
    // porcelain flags. A second edit to an already modified file invalidates consent.
    let names = read(
        root,
        &[
            "ls-files",
            "-z",
            "--modified",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut paths = names.split_terminator('\0').collect::<Vec<_>>();
    paths.sort_unstable();
    paths.dedup();
    if paths.len() > 20_000 {
        return Err("Too many changed paths for safe confirmation".into());
    }
    for path in paths {
        if !safe_relative(path) {
            return Err("Invalid changed Git path".into());
        }
        let full = Path::new(root).join(path);
        let parent = full.parent().ok_or("Invalid dirty path")?;
        if let Ok(parent) = fs::canonicalize(parent)
            && !parent.starts_with(root)
        {
            return Err("Dirty path has an escaping symlink ancestor".into());
        }
        hash.update(path);
        digest_path(&mut hash, &full, &mut budget)?;
    }
    Ok((format!("{:x}", hash.finalize()), status))
}
fn clean(root: &str) -> Result<(), String> {
    if !read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err("This operation requires a clean index/worktree, including untracked files. Stash or move changes explicitly first.".into());
    }
    Ok(())
}
fn ignored(root: &str) -> Result<String, String> {
    read(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--ignored=matching",
        ],
    )
}
fn protect_ignored(root: &str, target: &str) -> Result<(), String> {
    protect_extra_paths(root, target, false)
}
fn protect_extra_paths(root: &str, target: &str, untracked: bool) -> Result<(), String> {
    let ignored = ignored(root)?;
    let mut ignored = ignored
        .split_terminator('\0')
        .filter_map(|r| {
            r.strip_prefix("!! ")
                .or_else(|| untracked.then(|| r.strip_prefix("?? ")).flatten())
        })
        .map(|p| p.trim_end_matches('/'))
        .collect::<Vec<_>>();
    if ignored.is_empty() {
        return Ok(());
    }
    ignored.sort_unstable();
    ignored.dedup();
    let set = ignored
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let overlaps = |path: &str| {
        if set.contains(path)
            || path
                .match_indices('/')
                .any(|(at, _)| set.contains(&path[..at]))
        {
            return true;
        }
        let prefix = format!("{path}/");
        let index = ignored.partition_point(|other| *other < prefix.as_str());
        ignored
            .get(index)
            .is_some_and(|other| other.starts_with(&prefix))
    };
    if untracked {
        let tree = read(root, &["ls-tree", "-r", "--name-only", "-z", target])?;
        if tree.split_terminator('\0').any(&overlaps) {
            return Err("Untracked/ignored content overlaps the restored tree. Move it out explicitly before continue/abort; no content was removed.".into());
        }
    }
    let ranges = [format!("HEAD..{target}"), format!("{target}..HEAD")];
    let mut histories = Vec::new();
    for range in &ranges {
        let count = read(root, &["rev-list", "--count", range])?;
        if count
            .trim()
            .parse::<usize>()
            .map_err(|_| "Invalid revision count")?
            > 2000
        {
            return Err("Ignored-file safety check is limited to 2,000 commits. Use your Git client for this range.".into());
        }
        histories.push(read(
            root,
            &[
                "log",
                "--no-show-signature",
                "--no-notes",
                "--format=",
                "--name-status",
                "-z",
                "--no-renames",
                "--diff-merges=first-parent",
                "--no-ext-diff",
                "--no-textconv",
                range,
            ],
        )?);
    }
    let diff = read(
        root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            target,
        ],
    )?;
    histories.push(diff);
    for raw in &histories {
        let mut fields = raw.split_terminator('\0');
        while let Some(status) = fields.next() {
            let status = status.trim_matches('\n');
            if status.is_empty() {
                continue;
            }
            if status.len() != 1
                || !matches!(status.as_bytes()[0], b'A' | b'D' | b'M' | b'T' | b'U')
            {
                return Err("Cannot validate ignored-file collision safety".into());
            }
            let path = fields.next().ok_or("Missing changed path")?;
            if overlaps(path) {
                return Err("An ignored/untracked file or directory overlaps paths touched by this operation. Move it explicitly first; nothing was overwritten.".into());
            }
        }
    }
    Ok(())
}
fn selected_stash<'a>(inventory: &'a Overview, id: &str) -> Result<&'a overview::Stash, String> {
    if !oid(id) {
        return Err("Choose a stash from the inventory".into());
    }
    let values = inventory
        .stashes
        .iter()
        .filter(|s| s.oid == id)
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err("The stash changed or its identity is ambiguous. Refresh state.".into());
    }
    Ok(values[0])
}
fn visible_index(root: &str) -> Result<(), String> {
    let flags = read(root, &["ls-files", "-v", "-z"])?;
    if flags.split_terminator('\0').any(|line| {
        line.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')
    }) {
        return Err("Remove assume-unchanged/skip-worktree flags or use your Git client. Hidden working-tree changes cannot be safely preflighted.".into());
    }
    Ok(())
}
fn tree_root(root: &str, path: &str, inventory: &Overview) -> Result<String, String> {
    let tree = inventory
        .worktrees
        .iter()
        .find(|t| t.path == path)
        .ok_or("The worktree is no longer registered")?;
    if tree.current || tree.bare || tree.locked.is_some() || tree.prunable.is_some() {
        return Err("Current, bare, locked or missing worktrees cannot be removed".into());
    }
    let canonical = repository_root(path, Some(path))?;
    if directory(&canonical, "--git-common-dir")? != directory(root, "--git-common-dir")? {
        return Err("Worktree identity changed".into());
    }
    visible_index(&canonical)?;
    clean(&canonical)?;
    locks(&canonical)?;
    if !ignored(&canonical)?.is_empty() {
        return Err("Move ignored files out of the worktree before removing it".into());
    }
    let inventory = overview::load(&canonical)?;
    if !inventory.operations.is_empty() || !inventory.conflicts.is_empty() {
        return Err("Finish operations in that worktree before removal".into());
    }
    if !read(&canonical, &["ls-files", "--stage", "-z"])?
        .split_terminator('\0')
        .all(|s| !s.starts_with("160000 "))
    {
        return Err("Worktrees containing submodules cannot be removed here".into());
    }
    Ok(canonical)
}
pub fn preflight(path: &str, action: Action) -> Result<Plan, String> {
    let root = repository_root(path, None)?;
    locks(&root)?;
    let common = directory(&root, "--git-common-dir")?;
    let inventory = overview::load(&root)?;
    let head = commit(&root, "HEAD").ok();
    let continuation = matches!(action, Action::Continue { .. } | Action::Abort { .. });
    if !matches!(
        action,
        Action::StashDrop { .. }
            | Action::WorktreeAdd { .. }
            | Action::WorktreeRemove { .. }
            | Action::WorktreePrune
    ) {
        visible_index(&root)?;
    }
    if !continuation && (!inventory.operations.is_empty() || !inventory.conflicts.is_empty()) {
        return Err("Finish or abort the existing Git operation/conflicts first".into());
    }
    let mut target = None;
    let mut worktree_target = None;
    match &action {
        Action::StashPush { message, .. } => {
            if message.len() > 1024 || message.chars().any(char::is_control) {
                return Err(
                    "Stash message must be at most 1,024 bytes with no control characters".into(),
                );
            }
            if head.is_none() {
                return Err("Create the first commit before stashing".into());
            }
        }
        Action::StashApply { oid } | Action::StashPop { oid } => {
            selected_stash(&inventory, oid)?;
            clean(&root)?;
            protect_ignored(&root, oid)?;
        }
        Action::StashDrop { oid } => {
            selected_stash(&inventory, oid)?;
        }
        Action::WorktreeAdd { name, revision } => {
            if name.is_empty()
                || name.len() > 80
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err("Use a simple worktree name (letters, digits, - or _)".into());
            }
            target = Some(commit(&root, revision)?);
            let base = Path::new(&common).join("dunlin-worktrees");
            if fs::symlink_metadata(&base).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
            {
                return Err("Managed worktree directory is unsafe".into());
            }
            let destination = base.join(name);
            if fs::symlink_metadata(&destination).is_ok() {
                return Err("Worktree destination already exists".into());
            }
            worktree_target = Some(destination.to_str().ok_or("Invalid worktree path")?.into());
        }
        Action::WorktreeRemove { path } => {
            worktree_target = Some(tree_root(&root, path, &inventory)?);
        }
        Action::WorktreePrune => {}
        Action::Merge { revision }
        | Action::Rebase { revision }
        | Action::CherryPick { revision, .. } => {
            clean(&root)?;
            if head.is_none() {
                return Err("Create the first commit first".into());
            }
            let resolved = commit(&root, revision)?;
            protect_ignored(&root, &resolved)?;
            if let Action::CherryPick { mainline, .. } = &action {
                let parents = read(&root, &["rev-list", "--parents", "-n1", &resolved])?
                    .split_whitespace()
                    .count()
                    .saturating_sub(1);
                if parents > 1 && mainline.is_none_or(|p| p == 0 || p > parents)
                    || parents <= 1 && mainline.is_some()
                {
                    return Err("Merge commits require an explicit valid mainline parent; ordinary commits do not".into());
                }
            }
            target = Some(resolved);
        }
        Action::Continue { operation } | Action::Abort { operation } => {
            if !matches!(
                operation,
                Operation::Merge | Operation::Rebase | Operation::CherryPick
            ) || !inventory.operations.contains(operation)
            {
                return Err("Choose the current merge, rebase or cherry-pick operation".into());
            }
            if matches!(action, Action::Continue { .. }) && !inventory.conflicts.is_empty() {
                return Err("Resolve and stage every conflict before continuing".into());
            }
            for reference in ["ORIG_HEAD", "MERGE_HEAD", "CHERRY_PICK_HEAD"] {
                if let Ok(target) = commit(&root, reference) {
                    protect_extra_paths(&root, &target, true)?;
                }
            }
        }
    }
    let (mut fingerprint, status) = snapshot(&root, &inventory)?;
    if let Some(path) = worktree_target
        .as_ref()
        .filter(|_| matches!(action, Action::WorktreeRemove { .. }))
    {
        let other = overview::load(path)?;
        let (tree, _) = snapshot(path, &other)?;
        fingerprint = format!("{fingerprint}:{tree}");
    }
    let warning=match action {Action::Abort{..}=>"Native abort may discard ALL staged/unstaged tracked edits made since this operation started, including unrelated edits and conflict resolutions. Back them up explicitly first. No automatic backup or stash is created.",Action::StashDrop{..}=>"Permanently removes the selected stash reference. There is no application Undo.",Action::WorktreeRemove{..}=>"Removes the selected clean worktree directory. Unsaved buffers in it must be closed first.",Action::Rebase{..}=>"Rewrites local commits. No fetch, auto-stash or force push is performed.",_=>"No force, reset, automatic stash, fetch or push is performed."}.to_string();
    repository_root(path, Some(&root))?;
    Ok(Plan {
        root,
        common,
        fingerprint,
        action,
        head,
        target,
        worktree_target,
        status,
        overview: inventory,
        warning,
    })
}

pub(crate) struct Lock {
    path: std::path::PathBuf,
    token: String,
}
impl Drop for Lock {
    fn drop(&mut self) {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut value = String::new();
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_file() && m.len() <= 64)
            && options
                .open(&self.path)
                .and_then(|f| f.take(65).read_to_string(&mut value))
                .is_ok()
            && value == self.token
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub(crate) fn lock(common: &str) -> Result<Lock, String> {
    let path = Path::new(common).join("dunlin-operation.lock");
    let token = uuid::Uuid::now_v7().to_string();
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_|"Another Dunlin Git operation holds this repository lock. A crashed job requires manual lock inspection.")?;
    file.write_all(token.as_bytes())
        .map_err(|_| "Cannot record repository lock")?;
    Ok(Lock { path, token })
}
/// A confirmed pair may add its first detached tree, but must not authorize any
/// unrelated source/index/ref/config change made by a hook or external process.
pub(crate) fn unchanged_except_added_worktrees(
    original: &Plan,
    added: &[&str],
) -> Result<(), String> {
    repository_root(&original.root, Some(&original.root))?;
    if directory(&original.root, "--git-common-dir")? != original.common {
        return Err("Comparison source Git directory changed".into());
    }
    let mut inventory = overview::load(&original.root)?;
    for path in added {
        let count = inventory.worktrees.len();
        inventory.worktrees.retain(|tree| tree.path != *path);
        if inventory.worktrees.len() + 1 != count {
            return Err("Comparison worktree is no longer uniquely registered".into());
        }
    }
    let (fingerprint, _) = snapshot(&original.root, &inventory)?;
    if fingerprint != original.fingerprint {
        return Err("Source changed while creating comparison worktrees. No further checkout or agent is authorized; created trees are retained.".into());
    }
    Ok(())
}

fn run(root: &str, args: &[&str]) -> Result<bool, String> {
    run_with_timeout(root, args, Duration::from_secs(60))
}
fn run_with_timeout(root: &str, args: &[&str], timeout: Duration) -> Result<bool, String> {
    let mut command = Command::new("git");
    command
        .args([
            "-C",
            root,
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "rebase.autoStash=false",
            "-c",
            "merge.autoStash=false",
        ])
        .args(args)
        .env_remove("GIT_LITERAL_PATHSPECS")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "false")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| "Could not launch Git")?;
    let stdout = child.stdout.take().ok_or("Missing Git output")?;
    let stderr = child.stderr.take().ok_or("Missing Git error output")?;
    // Always drain both streams so verbose hooks cannot deadlock Git. Retain no
    // raw hook output (which may contain credentials). Bound time and process group.
    let drain = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while stream.read(&mut buffer).is_ok_and(|n| n > 0) {}
        })
    };
    let out = drain(Box::new(stdout));
    let err = drain(Box::new(stderr));
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s.success()),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break Err("Git timed out or failed to wait; it may have changed files. Refresh state; no automatic rollback or retry was performed.".into());
            }
        }
    };
    // Hooks may leave children with inherited pipes. Do not wait for those to
    // close: reader tasks terminate when their bounded process group is gone.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    drop(out);
    drop(err);
    result
}
pub fn execute(path: &str, plan: &Plan) -> Result<Outcome, String> {
    repository_root(path, Some(&plan.root))?;
    if directory(&plan.root, "--git-common-dir")? != plan.common {
        return Err("Git directory changed. Review a fresh preflight.".into());
    }
    let _lock = lock(&plan.common)?;
    let fresh = preflight(path, plan.action.clone())?;
    if &fresh != plan {
        return Err("Repository/target/content changed after confirmation. No mutation was started; review a new preflight.".into());
    }
    let root = plan.root.as_str();
    let target = plan.target.as_deref().unwrap_or("");
    let success = match &plan.action {
        Action::StashPush {
            message,
            include_untracked,
        } => {
            let mut args = vec!["stash", "push", "--message", message];
            if *include_untracked {
                args.push("--include-untracked");
            }
            run(root, &args)?
        }
        Action::StashApply { oid } | Action::StashPop { oid } => {
            let applied = run(root, &["stash", "apply", "--index", oid])?;
            if applied && matches!(plan.action, Action::StashPop { .. }) {
                let current = overview::load(root)?;
                if current.stashes != plan.overview.stashes {
                    return Err("Stash applied, but stash refs changed concurrently. Nothing was dropped. Refresh state.".into());
                }
                run(
                    root,
                    &["stash", "drop", &selected_stash(&current, oid)?.selector],
                )?
            } else {
                applied
            }
        }
        Action::StashDrop { oid } => run(
            root,
            &[
                "stash",
                "drop",
                &selected_stash(&fresh.overview, oid)?.selector,
            ],
        )?,
        Action::WorktreeAdd { .. } => {
            let destination = plan
                .worktree_target
                .as_deref()
                .ok_or("Missing worktree destination")?;
            fs::create_dir_all(
                Path::new(destination)
                    .parent()
                    .ok_or("Invalid worktree directory")?,
            )
            .map_err(|_| "Cannot create managed worktree directory")?;
            run(
                root,
                &["worktree", "add", "--detach", "--", destination, target],
            )?
        }
        Action::WorktreeRemove { .. } => run(
            root,
            &[
                "worktree",
                "remove",
                "--",
                plan.worktree_target.as_deref().ok_or("Missing worktree")?,
            ],
        )?,
        Action::WorktreePrune => run(root, &["worktree", "prune", "--expire", "now"])?,
        Action::Merge { .. } => run(
            root,
            &[
                "merge",
                "--no-autostash",
                "--no-edit",
                "--no-overwrite-ignore",
                target,
            ],
        )?,
        Action::Rebase { .. } => run(
            root,
            &["rebase", "--no-autostash", "--no-update-refs", target],
        )?,
        Action::CherryPick { mainline, .. } => {
            let parent = mainline.map(|p| p.to_string());
            let mut args = vec!["cherry-pick"];
            if let Some(p) = &parent {
                args.extend(["--mainline", p]);
            }
            args.push(target);
            run(root, &args)?
        }
        Action::Continue { operation } | Action::Abort { operation } => {
            let verb = if matches!(plan.action, Action::Continue { .. }) {
                "--continue"
            } else {
                "--abort"
            };
            let command = match operation {
                Operation::Merge => "merge",
                Operation::Rebase => "rebase",
                Operation::CherryPick => "cherry-pick",
                _ => return Err("Unsupported operation".into()),
            };
            run(root, &[command, verb])?
        }
    };
    let overview = overview::load(root)?;
    let message=if success {"Git finished. Review the refreshed repository state."} else if !overview.conflicts.is_empty() {"Git stopped with conflicts. The operation/stash is preserved; resolve and stage conflicts, then explicitly continue or abort."} else {"Git stopped or a hook refused the operation. Files/refs may have changed. Review state; nothing was automatically retried or rolled back."}.into();
    Ok(Outcome {
        success,
        message,
        overview,
    })
}

#[cfg(test)]
mod tests;
