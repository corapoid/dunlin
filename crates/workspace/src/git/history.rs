//! Bounded local DAG; no network, checkout, stash or worktree writes.
use super::activity::git;
use serde::{Deserialize, Serialize};

mod details;
pub use details::{CommitDetails, CommitFile, details, file_patch};

/// Resolve the selected repository, never an unselected ancestor. Callers may
/// pin an earlier canonical root to reject retargeted folder aliases.
pub fn repository_root(root: &str, expected: Option<&str>) -> Result<String, String> {
    let canonical = std::fs::canonicalize(root).map_err(|_| "The project folder is unavailable")?;
    let canonical = canonical
        .to_str()
        .ok_or("The repository path is not UTF-8")?;
    if expected.is_some_and(|expected| expected != canonical) {
        return Err(
            "The project folder now points to another repository. Reopen Git history.".into(),
        );
    }
    if !super::is_repository_root(std::path::Path::new(canonical)) {
        return Err("Open the repository root to read Git history".into());
    }
    Ok(canonical.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: String,
}

/// Explicit read-only history protocol. A scope identifies a repository
/// snapshot, never an authorization grant or a caller-selected server path.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryResponse<T> {
    pub version: u32,
    pub scope: String,
    pub value: T,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitPatch {
    pub oid: String,
    pub parent: usize,
    pub file: CommitFile,
    pub patch: String,
}
pub fn load(root: &str, limit: usize) -> Result<Vec<Commit>, String> {
    load_from(root, "", limit)
}
pub fn load_from(root: &str, revision: &str, limit: usize) -> Result<Vec<Commit>, String> {
    let canonical = repository_root(root, None)?;
    let root = canonical.as_str();
    let revision = revision.trim();
    let resolved;
    let scope: Vec<&str> = if revision.is_empty() {
        vec!["--all", "HEAD"]
    } else {
        if revision.len() > 512 || revision.starts_with('-') {
            return Err("Enter a commit ID, branch or tag".into());
        }
        resolved = git(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{commit}}"),
            ],
        )?;
        vec![resolved.trim()]
    };
    let maximum = format!("--max-count={}", limit.clamp(1, 2000));
    let mut args = vec![
        "log",
        "--no-show-signature",
        "--no-notes",
        "--no-color",
        "--no-patch",
        "--topo-order",
        maximum.as_str(),
        "--format=%H%x00%P%x00%an%x00%ct%x00%s%x00%D",
    ];
    args.extend(scope);
    let text = super::activity::git_limited(root, &args, 2 * 1024 * 1024)?;
    text.lines()
        .map(|line| {
            let fields: Vec<_> = line.split('\0').collect();
            if fields.len() != 6 {
                return Err("Invalid Git history".into());
            }
            Ok(Commit {
                oid: fields[0].into(),
                parents: fields[1].split_whitespace().map(str::to_owned).collect(),
                author: fields[2].into(),
                timestamp: fields[3].parse().unwrap_or(0),
                subject: fields[4].into(),
                refs: fields[5].into(),
            })
        })
        .collect()
}

/// Create just one ref. Git's exclusive ref lock rejects existing names and
/// races. No checkout or index refresh, including when the worktree is dirty.
pub fn create_branch(root: &str, name: &str, oid: &str) -> Result<(), String> {
    let canonical = repository_root(root, None)?;
    let root = canonical.as_str();
    if name.is_empty()
        || name.len() > 240
        || name.starts_with('-')
        || name == "HEAD"
        || name.trim() != name
    {
        return Err("Invalid branch name".into());
    }
    let checked = git(root, &["check-ref-format", "--branch", name])
        .map_err(|_| "Invalid branch name".to_string())?;
    if checked.trim() != name {
        return Err("Enter an explicit branch name".into());
    }
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Select a commit from this repository".into());
    }
    git(root, &["cat-file", "-e", &format!("{oid}^{{commit}}")])?;
    let git_dir = git(root, &["rev-parse", "--absolute-git-dir"])?;
    let common_dir = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    for dir in [git_dir.trim(), common_dir.trim()] {
        for lock in [
            "index.lock",
            "HEAD.lock",
            "packed-refs.lock",
            "config.lock",
            "shallow.lock",
        ] {
            if std::path::Path::new(dir).join(lock).exists() {
                return Err("Repository is locked by another operation".into());
            }
        }
    }
    git(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "--no-track",
            name,
            oid,
        ],
    )
    .map(|_| ())
    .map_err(|_| {
        "Could not create branch: the name may already exist or the repository is locked".into()
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};
    #[test]
    fn creates_from_old_commit_without_touching_dirty_worktree_index_or_head() {
        let root = std::env::temp_dir().join(format!("dunlin-history-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&root).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        run(&["init", "-qb", "main"]);
        fs::write(root.join("file"), "one").unwrap();
        run(&["add", "file"]);
        let commit = || {
            run(&[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ]);
        };
        commit();
        let first = run(&["rev-parse", "HEAD"]).trim().to_string();
        fs::write(root.join("file"), "two").unwrap();
        run(&["add", "file"]);
        commit();
        fs::write(root.join("file"), "staged").unwrap();
        run(&["add", "file"]);
        fs::write(root.join("file"), "unstaged").unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let head = fs::read(root.join(".git/HEAD")).unwrap();
        let path = root.to_str().unwrap();
        assert_eq!(super::load(path, 100).unwrap().len(), 2);
        assert_eq!(super::load_from(path, &first, 100).unwrap().len(), 1);
        assert!(super::load_from(path, "--all", 100).is_err());
        super::create_branch(path, "feature/from-old", &first).unwrap();
        assert_eq!(run(&["rev-parse", "feature/from-old"]).trim(), first);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head);
        assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "unstaged");
        assert!(super::create_branch(path, "feature/from-old", &first).is_err());
        for name in ["-bad", "HEAD", "with space", "../bad"] {
            assert!(super::create_branch(path, name, &first).is_err());
        }
        fs::write(root.join(".git/index.lock"), "lock").unwrap();
        assert!(super::create_branch(path, "blocked", &first).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
