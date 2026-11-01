//! Immutable commit/tree comparisons, never the index or worktree. External diff
//! drivers, textconv, lazy fetch, pagers and repository scripts are not invoked.
use super::{Commit, repository_root};
use crate::git::activity::git_limited;
use serde::{Deserialize, Serialize};

const MAX_FILES: usize = 2000;
const MAX_PATCH_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFile {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetails {
    pub commit: Commit,
    pub message: String,
    /// None for a root commit; otherwise the explicit comparison parent.
    pub parent: Option<String>,
    pub files: Vec<CommitFile>,
}

fn metadata(root: &str, oid: &str) -> Result<(Commit, String), String> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Select a full commit ID from this repository".into());
    }
    let text = git_limited(
        root,
        &[
            "show",
            "--no-show-signature",
            "--no-notes",
            "--no-patch",
            "--format=%H%x00%P%x00%an%x00%ct%x00%s%x00%B",
            oid,
            "--",
        ],
        64 * 1024,
    )?;
    let fields: Vec<_> = text.splitn(6, '\0').collect();
    if fields.len() != 6 || fields[0] != oid.to_ascii_lowercase() {
        return Err("Could not read this commit's metadata".into());
    }
    Ok((
        Commit {
            oid: fields[0].into(),
            parents: fields[1].split_whitespace().map(str::to_owned).collect(),
            author: fields[2].into(),
            timestamp: fields[3].parse().map_err(|_| "Invalid commit timestamp")?,
            subject: fields[4].into(),
            refs: String::new(),
        },
        fields[5].trim_end_matches('\n').into(),
    ))
}

fn comparison_parent(commit: &Commit, parent: usize) -> Result<Option<&str>, String> {
    if commit.parents.is_empty() && parent == 0 {
        return Ok(None);
    }
    commit
        .parents
        .get(parent)
        .map(|oid| Some(oid.as_str()))
        .ok_or_else(|| "Select a parent of this commit".into())
}

fn parse_files(text: &str) -> Result<Vec<CommitFile>, String> {
    if !text.is_empty() && !text.ends_with('\0') {
        return Err("Incomplete commit file list".into());
    }
    let mut fields = text.split_terminator('\0');
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        if files.len() == MAX_FILES {
            return Err("This commit exceeds the 2,000-file preview limit".into());
        }
        let kind = status
            .as_bytes()
            .first()
            .copied()
            .ok_or("Invalid commit file status")?;
        if !matches!(kind, b'A' | b'M' | b'D' | b'R' | b'C' | b'T')
            || !status[1..].bytes().all(|c| c.is_ascii_digit())
        {
            return Err("Unsupported commit file status".into());
        }
        let first = fields.next().ok_or("Missing commit file path")?;
        let (old_path, path) = if matches!(kind, b'R' | b'C') {
            (
                Some(first.to_string()),
                fields
                    .next()
                    .ok_or("Missing rename destination")?
                    .to_string(),
            )
        } else {
            (None, first.to_string())
        };
        validate_path(&path)?;
        if let Some(old) = &old_path {
            validate_path(old)?;
        }
        files.push(CommitFile {
            status: status.into(),
            path,
            old_path,
        });
    }
    Ok(files)
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\0')
        || std::path::Path::new(path).is_absolute()
        || std::path::Path::new(path)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Commit paths must stay inside the repository".into());
    }
    Ok(())
}

/// First-parent comparison by default, explicit other parents for merges. Root
/// commits compare to the empty tree without writing an empty-tree object.
pub fn details(root: &str, oid: &str, parent: usize) -> Result<CommitDetails, String> {
    let root = repository_root(root, None)?;
    let (commit, message) = metadata(&root, oid)?;
    let base = comparison_parent(&commit, parent)?;
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--name-status",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--ignore-submodules=none",
        "--find-renames=50%",
        "-l200",
    ];
    if let Some(base) = base {
        args.push(base);
    }
    args.extend([oid, "--"]);
    let text = git_limited(&root, &args, 512 * 1024)?;
    let files = parse_files(&text)?;
    let parent = base.map(str::to_owned);
    Ok(CommitDetails {
        commit,
        message,
        parent,
        files,
    })
}

pub fn file_patch(
    root: &str,
    oid: &str,
    parent: usize,
    file: &CommitFile,
) -> Result<String, String> {
    validate_path(&file.path)?;
    if let Some(old) = &file.old_path {
        validate_path(old)?;
    }
    let root = repository_root(root, None)?;
    let (commit, _) = metadata(&root, oid)?;
    let base = comparison_parent(&commit, parent)?;
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "-p",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--ignore-submodules=none",
        "--submodule=short",
        "--find-renames=50%",
        "-l200",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--no-relative",
        "--unified=3",
    ];
    if let Some(base) = base {
        args.push(base);
    }
    args.extend([oid, "--", &file.path]);
    if let Some(old) = &file.old_path {
        args.push(old);
    }
    let patch = git_limited(&root, &args, MAX_PATCH_BYTES)
        .map_err(|e| format!("Commit patch unavailable (2 MiB limit, bounded Git timeout): {e}"))?;
    if patch.lines().count() > 20_000 {
        return Err("This patch exceeds the 20,000-line preview limit".into());
    }
    Ok(patch)
}

#[cfg(test)]
mod tests;
