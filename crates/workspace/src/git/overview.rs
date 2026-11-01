//! Bounded repository inventory. Reads Git metadata only, never stash contents,
//! sibling worktree files, hooks, filters, or operation instructions.
use super::{activity::git_limited, history::repository_root};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const MAX_ENTRIES: usize = 2000;
const OUTPUT_LIMIT: u64 = 512 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overview {
    pub stashes: Vec<Stash>,
    pub worktrees: Vec<Worktree>,
    pub operations: Vec<Operation>,
    pub conflicts: Vec<Conflict>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stash {
    pub oid: String,
    pub selector: String,
    pub timestamp: i64,
    pub subject: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    /// Metadata from `worktree list`, not a path the caller may operate on.
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub current: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Merge,
    Rebase,
    ApplyMailbox,
    CherryPick,
    Revert,
    Sequencer,
}
impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::ApplyMailbox => "Apply mailbox (git am)",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
            Self::Sequencer => "Sequencer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: String,
    /// Present index stages: 1 = base, 2 = ours, 3 = theirs. No file reads.
    pub stages: Vec<u8>,
}
impl Conflict {
    pub fn label(&self) -> &'static str {
        match self.stages.as_slice() {
            [1, 2, 3] => "Both modified",
            [2, 3] => "Both added",
            [1, 2] => "Deleted by them",
            [1, 3] => "Deleted by us",
            [1] => "Both deleted",
            [2] => "Added by us",
            [3] => "Added by them",
            _ => "Unmerged",
        }
    }
}

fn invalid() -> String {
    "Invalid or oversized Git repository inventory".into()
}
fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
fn text(value: &str) -> bool {
    !value.contains('\0')
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && text(value)
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The desktop also validates remote inventories; metadata paths never become
/// local filesystem requests. Invalid/oversized lists fail, not silently truncate.
pub fn validate(value: &Overview) -> Result<(), String> {
    if value.stashes.len() > MAX_ENTRIES
        || value.worktrees.is_empty()
        || value.worktrees.len() > MAX_ENTRIES
        || value.conflicts.len() > MAX_ENTRIES
        || value.operations.len() > 6
        || value
            .operations
            .iter()
            .enumerate()
            .any(|(i, op)| value.operations[..i].contains(op))
        || value.stashes.iter().enumerate().any(|(i, s)| {
            !oid(&s.oid) || s.selector != format!("stash@{{{i}}}") || !text(&s.subject)
        })
        || value.worktrees.iter().any(|w| {
            !Path::new(&w.path).is_absolute()
                || !text(&w.path)
                || w.head.as_deref().is_some_and(|h| !oid(h))
                || w.branch
                    .as_deref()
                    .is_some_and(|b| !b.starts_with("refs/heads/") || !text(b))
                || w.locked.as_deref().is_some_and(|s| !text(s))
                || w.prunable.as_deref().is_some_and(|s| !text(s))
                || (w.detached && w.branch.is_some())
                || (!w.bare && w.head.is_none())
        })
        || value.worktrees.iter().filter(|w| w.current).count() != 1
        || value.conflicts.iter().any(|c| {
            !relative(&c.path)
                || c.stages.is_empty()
                || c.stages.iter().any(|s| !(1..=3).contains(s))
                || c.stages.windows(2).any(|w| w[0] >= w[1])
        })
    {
        return Err(invalid());
    }
    if serde_json::to_vec(value).map_err(|_| invalid())?.len() > 2 * 1024 * 1024 {
        return Err(invalid());
    }
    Ok(())
}

fn stashes(raw: &str) -> Result<Vec<Stash>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let fields: Vec<_> = raw
        .strip_suffix('\0')
        .ok_or_else(invalid)?
        .split('\0')
        .collect();
    let (records, remainder) = fields.as_chunks::<4>();
    if !remainder.is_empty() || records.len() > MAX_ENTRIES {
        return Err(invalid());
    }
    records
        .iter()
        .map(|f| {
            Ok(Stash {
                oid: f[0].into(),
                selector: f[1].into(),
                timestamp: f[2].parse().map_err(|_| invalid())?,
                subject: f[3].into(),
            })
        })
        .collect()
}

fn worktrees(raw: &str, root: &str) -> Result<Vec<Worktree>, String> {
    let mut result = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in raw.split_terminator('\0') {
        if field.is_empty() {
            if let Some(entry) = current.take() {
                result.push(entry);
            }
            if result.len() > MAX_ENTRIES {
                return Err(invalid());
            }
            continue;
        }
        if let Some(path) = field.strip_prefix("worktree ") {
            if current.is_some() {
                return Err(invalid());
            }
            current = Some(Worktree {
                path: path.into(),
                current: path == root,
                ..Default::default()
            });
            continue;
        }
        let entry = current.as_mut().ok_or_else(invalid)?;
        if let Some(head) = field.strip_prefix("HEAD ") {
            entry.head = Some(head.into());
        } else if let Some(branch) = field.strip_prefix("branch ") {
            entry.branch = Some(branch.into());
        } else if field == "bare" {
            entry.bare = true;
        } else if field == "detached" {
            entry.detached = true;
        } else if field == "locked" {
            entry.locked = Some(String::new());
        } else if let Some(reason) = field.strip_prefix("locked ") {
            entry.locked = Some(reason.into());
        } else if field == "prunable" {
            entry.prunable = Some(String::new());
        } else if let Some(reason) = field.strip_prefix("prunable ") {
            entry.prunable = Some(reason.into());
        } else {
            return Err(invalid());
        }
    }
    if current.is_some() {
        return Err(invalid());
    }
    Ok(result)
}

fn conflicts(raw: &str) -> Result<Vec<Conflict>, String> {
    if !raw.is_empty() && !raw.ends_with('\0') {
        return Err(invalid());
    }
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for record in raw.split_terminator('\0') {
        let (header, path) = record.split_once('\t').ok_or_else(invalid)?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 || !oid(fields[1]) {
            return Err(invalid());
        }
        let stage = fields[2].parse().map_err(|_| invalid())?;
        entries.entry(path.into()).or_default().push(stage);
        if entries.len() > MAX_ENTRIES {
            return Err(invalid());
        }
    }
    Ok(entries
        .into_iter()
        .map(|(path, stages)| Conflict { path, stages })
        .collect())
}

// Test marker existence without opening its contents or following symlinks.
fn marker(dir: &Path, name: &str, directory: bool) -> Result<bool, String> {
    match std::fs::symlink_metadata(dir.join(name)) {
        Ok(m) if if directory { m.is_dir() } else { m.is_file() } => Ok(true),
        Ok(_) => Err("Unexpected Git operation marker type".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Git operation state is unavailable".into()),
    }
}

pub fn load(configured_root: &str) -> Result<Overview, String> {
    let canonical = repository_root(configured_root, None)?;
    let root = canonical.as_str();
    let stash = git_limited(
        root,
        &[
            "stash",
            "list",
            "--no-show-signature",
            "-z",
            "-n2001",
            "--format=%H%x00%gd%x00%ct%x00%gs",
        ],
        OUTPUT_LIMIT,
    )?;
    let trees = git_limited(
        root,
        &["worktree", "list", "--porcelain", "-z"],
        OUTPUT_LIMIT,
    )?;
    let unmerged = git_limited(
        root,
        &["-c", "core.fsmonitor=false", "ls-files", "--unmerged", "-z"],
        OUTPUT_LIMIT,
    )?;
    let git_dir = git_limited(root, &["rev-parse", "--absolute-git-dir"], 65536)?;
    let dir = Path::new(git_dir.strip_suffix('\n').ok_or_else(invalid)?);
    let mut operations = Vec::new();
    for (name, operation, directory) in [
        ("MERGE_HEAD", Operation::Merge, false),
        ("CHERRY_PICK_HEAD", Operation::CherryPick, false),
        ("REVERT_HEAD", Operation::Revert, false),
        ("sequencer", Operation::Sequencer, true),
    ] {
        if marker(dir, name, directory)? {
            operations.push(operation);
        }
    }
    let merge = marker(dir, "rebase-merge", true)?;
    let apply = marker(dir, "rebase-apply", true)?;
    if merge || apply {
        operations.push(
            if apply && marker(&dir.join("rebase-apply"), "applying", false)? {
                Operation::ApplyMailbox
            } else {
                Operation::Rebase
            },
        );
    }
    let result = Overview {
        stashes: stashes(&stash)?,
        worktrees: worktrees(&trees, root)?,
        operations,
        conflicts: conflicts(&unmerged)?,
    };
    validate(&result)?;
    repository_root(configured_root, Some(root))?;
    Ok(result)
}

#[cfg(test)]
mod tests;
