//! Thin wrapper around the system `git` CLI.
//!
//! Dunlin deliberately shells out instead of linking libgit2: the CLI is
//! already the source of truth for credentials, hooks and worktree config, and
//! it keeps the binary small. All calls run off the UI thread.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
mod activity;
pub mod history;
pub use activity::{
    BranchActivity, RemoteState, RemoteStatus, branch_activity, check_remote_status,
};

/// Read-only root validation for explicit local imports; never expands a chosen
/// subdirectory to an unselected ancestor, fetches or runs repository scripts.
pub fn is_repository_root(path: &Path) -> bool {
    let Some(path_str) = path.to_str() else {
        return false;
    };
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    path.is_dir()
        && activity::git(path_str, &["rev-parse", "--show-toplevel"])
            .ok()
            .and_then(|root| std::fs::canonicalize(root.trim()).ok())
            .as_ref()
            == Some(&canonical)
}

#[cfg(test)]
mod branch_tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitChange {
    pub path: String,
    pub status: char,
    pub additions: u32,
    pub deletions: u32,
    pub staged: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub changes: Vec<GitChange>,
    pub error: Option<String>,
    #[serde(default)]
    pub not_repository: bool,
}

/// Read branch, working-tree status and line counts for `path`.
pub fn load(path: &str) -> GitStatus {
    if !Path::new(path).is_dir() {
        return GitStatus {
            error: Some("Folder is not available".into()),
            ..Default::default()
        };
    }

    let branch = git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "no branch".into());

    let porcelain = match git(path, &["status", "--porcelain=v1", "--untracked-files=all"]) {
        Ok(output) => output,
        Err(error) => {
            let not_repository = error.contains("not a git repository");
            return GitStatus {
                branch,
                changes: Vec::new(),
                error: (!not_repository).then_some(error),
                not_repository,
            };
        }
    };

    let mut changes = porcelain
        .lines()
        .flat_map(parse_changes)
        .collect::<Vec<_>>();

    // Count each side separately so a file with both staged and unstaged edits
    // gets accurate numbers in both source-control rows.
    let staged_counts =
        parse_numstat(&git(path, &["diff", "--cached", "--numstat"]).unwrap_or_default());
    let worktree_counts = parse_numstat(&git(path, &["diff", "--numstat"]).unwrap_or_default());

    for change in &mut changes {
        let counts = if change.staged {
            &staged_counts
        } else {
            &worktree_counts
        };
        if let Some((additions, deletions)) = counts.get(&change.path) {
            change.additions = *additions;
            change.deletions = *deletions;
        } else if change.status == 'U' {
            change.additions = untracked_lines(path, &change.path);
        }
    }

    GitStatus {
        branch,
        changes,
        error: None,
        not_repository: false,
    }
}

pub fn initialize(path: &str) -> Result<(), String> {
    git(path, &["init"]).map(|_| ())
}

pub fn stage_all(path: &str) -> Result<(), String> {
    git(path, &["add", "-A"]).map(|_| ())
}

/// Stage one path, including deletions and renames.
pub fn stage_file(path: &str, file: &str) -> Result<(), String> {
    git(path, &["add", "-A", "--", file]).map(|_| ())
}

/// Move one path back out of the index without touching the working tree.
pub fn unstage_file(path: &str, file: &str) -> Result<(), String> {
    match git(path, &["restore", "--staged", "--", file]) {
        Ok(_) => Ok(()),
        // A repository without a commit has no HEAD to restore from, so drop
        // the entry from the index directly.
        Err(error)
            if error.contains("could not resolve") || error.contains("ambiguous argument") =>
        {
            git(path, &["rm", "--cached", "-q", "--", file]).map(|_| ())
        }
        Err(error) => Err(error),
    }
}

/// Read a bounded HEAD baseline for the editor overview. Never runs filters,
/// text conversion, external diff tools, or writes the index.
pub fn head_file(path: &str, file: &str) -> Result<Option<String>, String> {
    use std::{io::Read, process::Stdio};
    if Path::new(file).is_absolute()
        || Path::new(file).components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err("Baseline paths must stay inside the project".into());
    }
    if git(path, &["rev-parse", "--is-inside-work-tree"])?.trim() != "true" {
        return Ok(None);
    }
    if git(path, &["rev-parse", "--verify", "HEAD"]).is_err()
        || git(path, &["ls-tree", "--name-only", "HEAD", "--", file])?
            .trim()
            .is_empty()
    {
        return Ok(Some(String::new()));
    }
    let spec = format!("HEAD:./{file}");
    let mut child = Command::new("git")
        .args(["-C", path, "show", "--no-ext-diff", "--no-textconv", &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    const LIMIT: u64 = 2 * 1024 * 1024;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or("Missing baseline output")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > LIMIT {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Baseline exceeds the bounded text limit".into());
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("Could not read the HEAD baseline".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "Baseline is not UTF-8 text".into())
}

/// A unified patch for one path, including an untracked file.
pub fn file_diff(path: &str, file: &str, staged: bool) -> Result<String, String> {
    if Path::new(file).is_absolute()
        || Path::new(file).components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err("Diff paths must stay inside the project".into());
    }
    const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;
    let args: &[&str] = if staged {
        &[
            "diff",
            "--cached",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            file,
        ]
    } else {
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            file,
        ]
    };
    let patch = git(path, args)?;
    if patch.len() > MAX_PATCH_BYTES {
        return Err("This patch is too large to display".into());
    }
    if !patch.trim().is_empty() {
        return Ok(patch);
    }
    if staged || git(path, &["ls-files", "--error-unmatch", "--", file]).is_ok() {
        return Ok(patch);
    }

    let full = crate::fs::resolve(path, file)?;
    let metadata =
        std::fs::metadata(&full).map_err(|error| format!("Could not read {file}: {error}"))?;
    if metadata.len() > MAX_PATCH_BYTES as u64 {
        return Err("This file is too large to preview".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&full)
        .map_err(|e| e.to_string())?
        .take(MAX_PATCH_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PATCH_BYTES {
        return Err("This file grew beyond the diff preview limit".into());
    }
    if bytes.contains(&0) {
        return Err("Binary file — diff is not available".into());
    }
    let contents = String::from_utf8_lossy(&bytes);
    let lines = contents.lines().count();
    let mut patch = format!(
        "diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n@@ -0,0 +1,{lines} @@\n"
    );
    for line in contents.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

pub fn commit(path: &str, message: &str) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first".into());
    }
    git(path, &["commit", "-m", message]).map(|_| ())
}

/// Read-only preflight metadata, never a commit or stage operation.
pub fn ai_commit_context(path: &str) -> Result<String, String> {
    let root = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let top = git(path, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(top.trim()).map_err(|e| e.to_string())? != root {
        return Err("Open the repository root before using AI commit; a nested project must not authorize sibling changes".into());
    }
    for state in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
    ] {
        let state = git(path, &["rev-parse", "--git-path", state])?;
        let state = Path::new(path).join(state.trim());
        if state.exists() {
            return Err(
                "Finish the current Git merge/rebase/cherry-pick before using AI commit".into(),
            );
        }
    }
    if !git(
        path,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "--diff-filter=U",
        ],
    )?
    .trim()
    .is_empty()
    {
        return Err("Resolve merge conflicts before using AI commit".into());
    }
    let status = git(path, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if status.trim().is_empty() {
        return Err("There are no changes to commit".into());
    }
    if status.len() > 128 * 1024 {
        return Err("Too many changed paths for a bounded AI commit request".into());
    }
    Ok(serde_json::json!({"root": root, "head": git(path, &["rev-parse", "--verify", "HEAD"]).ok().map(|s| s.trim().to_string()), "branch": git(path, &["symbolic-ref", "--short", "HEAD"]).ok().map(|s| s.trim().to_string()), "status": status, "scope": "If the index is nonempty, preserve its staged scope and partial staging. Otherwise review and stage only explicit authorized paths."}).to_string())
}

/// Replace the last commit with the current index and a new message.
pub fn amend(path: &str, message: &str) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first".into());
    }
    git(path, &["commit", "--amend", "-m", message]).map(|_| ())
}

pub fn branches(path: &str) -> Result<Vec<String>, String> {
    let output = git(path, &["branch", "--format=%(refname)"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter_map(|branch| branch.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchContext {
    pub root: String,
    pub current: String,
    pub head: Option<String>,
    pub branches: Vec<String>,
    pub status: String,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}
impl BranchContext {
    pub fn dirty(&self) -> bool {
        !self.status.is_empty()
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.root == other.root
            && self.current == other.current
            && self.head == other.head
            && self.status == other.status
    }
}

/// Read-only, repository-root-scoped branch preflight. A project subfolder must
/// not silently authorize changing its siblings or interrupting another Git job.
pub fn branch_context(path: &str) -> Result<BranchContext, String> {
    let root = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let top = git(path, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(top.trim()).map_err(|e| e.to_string())? != root {
        return Err("Open the repository root before switching branches; a subfolder cannot authorize sibling changes".into());
    }
    let path = root
        .to_str()
        .ok_or("The repository root is not a UTF-8 path")?;
    for state in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
        "index.lock",
    ] {
        let state = git(path, &["rev-parse", "--git-path", state])?;
        if Path::new(path).join(state.trim()).exists() {
            return Err("Finish the current Git operation before switching branches".into());
        }
    }
    if !git(path, &["ls-files", "--unmerged"])?.is_empty() {
        return Err("Resolve Git conflicts before switching branches".into());
    }
    let status = git(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.len() > 256 * 1024 {
        return Err("Too many changed paths for a bounded branch switch confirmation".into());
    }
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    let mut entries = status.split('\0');
    while let Some(entry) = entries.next() {
        let bytes = entry.as_bytes();
        if bytes.len() < 3 {
            continue;
        }
        if bytes[0] == b'?' {
            untracked += 1;
            continue;
        }
        staged += usize::from(bytes[0] != b' ');
        unstaged += usize::from(bytes[1] != b' ');
        if matches!(bytes[0], b'R' | b'C') || matches!(bytes[1], b'R' | b'C') {
            entries.next();
        }
    }
    let branches = branches(path)?;
    if branches.len() > 4096 || branches.iter().map(String::len).sum::<usize>() > 512 * 1024 {
        return Err("Too many local branches for the bounded picker; use your Git client to narrow the repository refs".into());
    }
    Ok(BranchContext {
        root: path.into(),
        current: git(path, &["symbolic-ref", "--quiet", "HEAD"])
            .map(|s| {
                s.trim()
                    .strip_prefix("refs/heads/")
                    .unwrap_or(s.trim())
                    .to_string()
            })
            .unwrap_or_else(|_| "Detached HEAD".into()),
        head: git(path, &["rev-parse", "--verify", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string()),
        branches,
        status,
        staged,
        unstaged,
        untracked,
    })
}

/// Only local branches, with an explicit dirty-worktree consent and fresh scope.
/// Native Git must refuse overwrites (including ignored files). No merge,
/// discard, stash, force, remote guessing or recursive submodule checkout.
pub fn switch_branch_checked(
    path: &str,
    branch: &str,
    expected: &BranchContext,
    keep_changes: bool,
) -> Result<(), String> {
    if branch.starts_with('-') || branch.is_empty() {
        return Err("Choose an existing local branch".into());
    }
    git(path, &["check-ref-format", &format!("refs/heads/{branch}")])?;
    let current = branch_context(path)?;
    if !current.same_snapshot(expected) {
        return Err("The repository changed while the branch picker was open. Review the current changes and try again.".into());
    }
    if !current.branches.iter().any(|name| name == branch) {
        return Err("The selected local branch no longer exists".into());
    }
    if current.current == branch {
        return Ok(());
    }
    let literal = git(
        &current.root,
        &[
            "rev-parse",
            "--verify",
            "--symbolic-full-name",
            "--end-of-options",
            branch,
        ],
    )?;
    if literal.trim() != format!("refs/heads/{branch}") {
        return Err(
            "This branch name is ambiguous with another Git ref; no checkout was attempted".into(),
        );
    }
    if current.dirty() && !keep_changes {
        return Err(
            "Uncommitted changes need explicit Keep changes and switch confirmation".into(),
        );
    }
    git(
        &current.root,
        &[
            "switch",
            "--no-guess",
            "--no-overwrite-ignore",
            "--no-recurse-submodules",
            "--",
            branch,
        ],
    )
    .map(|_| ())
}

pub fn switch_branch(path: &str, branch: &str) -> Result<(), String> {
    let expected = branch_context(path)?;
    switch_branch_checked(path, branch, &expected, false)
}

pub fn pull(path: &str) -> Result<(), String> {
    git(path, &["pull", "--ff-only"]).map(|_| ())
}

pub fn push(path: &str) -> Result<(), String> {
    git(path, &["push"]).map(|_| ())
}

/// Restore one tracked file in the working tree, leaving the index alone.
pub fn discard_file(path: &str, file: &str) -> Result<(), String> {
    git(path, &["restore", "--worktree", "--", file]).map(|_| ())
}

fn git(path: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-c")
        .arg("core.quotePath=false")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("Could not run git: {error}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "git exited with an error".into()
        } else {
            stderr
        })
    }
}

fn parse_changes(line: &str) -> Vec<GitChange> {
    if line.len() < 4 {
        return Vec::new();
    }
    let bytes = line.as_bytes();
    let x = bytes[0] as char;
    let y = bytes[1] as char;
    if x == '!' && y == '!' {
        return Vec::new();
    }

    let raw_path = line[3..].trim();
    let path = raw_path
        .rsplit_once(" -> ")
        .map(|(_, new)| new)
        .unwrap_or(raw_path)
        .trim_matches('"')
        .to_string();
    if path.is_empty() {
        return Vec::new();
    }

    if x == '?' {
        return vec![GitChange {
            path,
            status: 'U',
            additions: 0,
            deletions: 0,
            staged: false,
        }];
    }

    let mut changes = Vec::new();
    if x != ' ' {
        changes.push(GitChange {
            path: path.clone(),
            status: x,
            additions: 0,
            deletions: 0,
            staged: true,
        });
    }
    if y != ' ' {
        changes.push(GitChange {
            path,
            status: y,
            additions: 0,
            deletions: 0,
            staged: false,
        });
    }
    changes
}

fn parse_numstat(output: &str) -> std::collections::HashMap<String, (u32, u32)> {
    let mut counts = std::collections::HashMap::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, '\t');
        let additions = parts.next().unwrap_or_default();
        let deletions = parts.next().unwrap_or_default();
        let path = parts.next().unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        let path = path
            .rsplit_once(" => ")
            .map(|(_, new)| new)
            .unwrap_or(path)
            .to_string();
        counts.insert(
            path,
            (
                additions.parse().unwrap_or(0),
                deletions.parse().unwrap_or(0),
            ),
        );
    }
    counts
}

fn untracked_lines(root: &str, relative: &str) -> u32 {
    let path = Path::new(root).join(relative);
    let Ok(metadata) = path.metadata() else {
        return 0;
    };
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return 0;
    }
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return 0;
    };
    if contents.is_empty() {
        0
    } else {
        contents.lines().count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_folder_is_not_an_error_and_initialization_is_explicit() {
        let root = std::env::temp_dir().join(format!("dunlin-git-empty-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&root).unwrap();
        let path = root.to_str().unwrap();
        let status = load(path);
        assert!(status.not_repository);
        assert!(status.error.is_none());
        assert!(!root.join(".git").exists());
        initialize(path).unwrap();
        assert!(!load(path).not_repository);
        assert!(load(path).changes.is_empty());
        assert!(git(path, &["rev-parse", "--verify", "HEAD"]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_modified_and_untracked_entries() {
        let modified = parse_changes(" M crates/agent/src/lib.rs").remove(0);
        assert_eq!(modified.path, "crates/agent/src/lib.rs");
        assert_eq!(modified.status, 'M');
        assert!(!modified.staged);

        let untracked = parse_changes("?? notes.md").remove(0);
        assert_eq!(untracked.path, "notes.md");
        assert_eq!(untracked.status, 'U');
        assert!(!untracked.staged);

        let staged = parse_changes("A  src/new.rs").remove(0);
        assert_eq!(staged.status, 'A');
        assert!(staged.staged);
    }

    #[test]
    fn splits_a_file_with_staged_and_unstaged_edits() {
        let changes = parse_changes("MM src/lib.rs");
        assert_eq!(changes.len(), 2);
        assert!(changes[0].staged);
        assert_eq!(changes[0].status, 'M');
        assert!(!changes[1].staged);
        assert_eq!(changes[1].status, 'M');
        assert_eq!(changes[0].path, changes[1].path);
    }

    #[test]
    fn parses_renames_to_the_new_path() {
        let change = parse_changes("R  old/name.rs -> new/name.rs").remove(0);
        assert_eq!(change.path, "new/name.rs");
        assert_eq!(change.status, 'R');
        assert!(change.staged);
    }

    #[test]
    fn parses_numstat_lines() {
        let counts = parse_numstat("12\t3\tsrc/main.rs\n-\t-\timage.png\n");
        assert_eq!(counts.get("src/main.rs"), Some(&(12, 3)));
        assert_eq!(counts.get("image.png"), Some(&(0, 0)));
    }

    #[test]
    fn reads_stages_and_commits_a_real_repository() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-git-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.to_string_lossy().to_string();

        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        };

        run(&["init", "-q"]);
        run(&["config", "user.name", "Dunlin Test"]);
        run(&["config", "user.email", "test@dunlin.invalid"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(std::path::Path::new(&root).join("hello.txt"), "one\ntwo\n").unwrap();

        let status = load(&root);
        assert_eq!(status.error, None);
        assert!(!status.branch.is_empty());
        assert_eq!(status.changes.len(), 1);
        assert_eq!(status.changes[0].path, "hello.txt");
        assert_eq!(status.changes[0].status, 'U');
        assert_eq!(status.changes[0].additions, 2);
        assert_eq!(status.changes[0].deletions, 0);

        let patch = file_diff(&root, "hello.txt", false).unwrap();
        assert!(patch.contains("new file mode 100644"), "{patch}");
        assert!(patch.contains("+one\n+two"), "{patch}");

        stage_file(&root, "hello.txt").unwrap();
        let status = load(&root);
        assert!(status.changes[0].staged);

        let patch = file_diff(&root, "hello.txt", true).unwrap();
        assert!(patch.contains("new file mode 100644"), "{patch}");

        unstage_file(&root, "hello.txt").unwrap();
        let status = load(&root);
        assert!(!status.changes[0].staged);

        stage_all(&root).unwrap();
        commit(&root, "initial commit").unwrap();

        let status = load(&root);
        assert_eq!(status.error, None);
        assert!(status.changes.is_empty());

        let default_branch = status.branch.clone();
        amend(&root, "amended initial commit").unwrap();
        assert!(
            git(&root, &["log", "-1", "--pretty=%s"])
                .unwrap()
                .contains("amended initial commit")
        );
        run(&["branch", "feature"]);
        switch_branch(&root, "feature").unwrap();
        assert_eq!(load(&root).branch, "feature");
        switch_branch(&root, &default_branch).unwrap();
        assert_eq!(load(&root).branch, default_branch);

        std::fs::write(std::path::Path::new(&root).join("ünïcode.txt"), "hi").unwrap();
        let status = load(&root);
        assert_eq!(status.changes.len(), 1);
        assert_eq!(status.changes[0].path, "ünïcode.txt");

        std::fs::write(
            std::path::Path::new(&root).join("hello.txt"),
            "one\ntwo\nthree\n",
        )
        .unwrap();
        let patch = file_diff(&root, "hello.txt", false).unwrap();
        assert!(patch.contains("+three"), "{patch}");

        stage_file(&root, "hello.txt").unwrap();
        let staged = file_diff(&root, "hello.txt", true).unwrap();
        assert!(staged.contains("+three"), "{staged}");
        let worktree = file_diff(&root, "hello.txt", false).unwrap();
        assert!(worktree.is_empty(), "{worktree}");

        std::fs::write(std::path::Path::new(&root).join("logo.bin"), [0, 1, 2]).unwrap();
        let error = file_diff(&root, "logo.bin", false).unwrap_err();
        assert!(error.contains("Binary"), "{error}");

        std::fs::write(
            std::path::Path::new(&root).join("hello.txt"),
            "one\ntwo\nthree\nfour\n",
        )
        .unwrap();
        let index = std::fs::read(std::path::Path::new(&root).join(".git/index")).unwrap();
        assert_eq!(
            head_file(&root, "hello.txt").unwrap().as_deref(),
            Some("one\ntwo\n")
        );
        assert_eq!(
            std::fs::read(std::path::Path::new(&root).join(".git/index")).unwrap(),
            index
        );
        let status = load(&root);
        let hello: Vec<_> = status
            .changes
            .iter()
            .filter(|change| change.path == "hello.txt")
            .collect();
        assert_eq!(hello.len(), 2);
        assert!(hello.iter().any(|change| change.staged));
        assert!(hello.iter().any(|change| !change.staged));

        discard_file(&root, "hello.txt").unwrap();
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&root).join("hello.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    struct TestRepository(std::path::PathBuf);
    impl TestRepository {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("dunlin-ai-git-{}-{unique}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            git(root.to_str().unwrap(), &["init", "-q"]).unwrap();
            Self(root)
        }
        fn root(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }
    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn editor_baselines_support_unborn_repos_nested_roots_and_confine_reads() {
        let repo = TestRepository::new();
        std::fs::create_dir(repo.0.join("nested")).unwrap();
        std::fs::write(repo.0.join("nested/file.txt"), "committed baseline\n").unwrap();
        assert_eq!(
            head_file(repo.root(), "nested/file.txt").unwrap(),
            Some(String::new())
        );
        stage_file(repo.root(), "nested/file.txt").unwrap();
        git(
            repo.root(),
            &[
                "-c",
                "user.name=Dunlin Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "test baseline",
            ],
        )
        .unwrap();
        std::fs::write(repo.0.join("nested/file.txt"), "working content\n").unwrap();
        assert_eq!(
            head_file(repo.0.join("nested").to_str().unwrap(), "file.txt")
                .unwrap()
                .as_deref(),
            Some("committed baseline\n")
        );
        assert_eq!(
            head_file(repo.root(), "new.txt").unwrap(),
            Some(String::new())
        );
        assert!(head_file(repo.root(), "../outside").is_err());
        assert!(head_file(repo.root(), "/outside").is_err());
    }

    #[test]
    fn ai_commit_preflight_is_read_only_and_preserves_the_index() {
        let repo = TestRepository::new();
        assert!(
            ai_commit_context(repo.root())
                .unwrap_err()
                .contains("no changes")
        );
        std::fs::write(repo.0.join("tracked.txt"), "reviewed\n").unwrap();
        stage_file(repo.root(), "tracked.txt").unwrap();
        let index = std::fs::read(repo.0.join(".git/index")).unwrap();
        let before = load(repo.root());
        let snapshot: serde_json::Value =
            serde_json::from_str(&ai_commit_context(repo.root()).unwrap()).unwrap();
        assert!(snapshot["head"].is_null());
        assert!(snapshot["status"].as_str().unwrap().contains("tracked.txt"));
        assert_eq!(load(repo.root()), before);
        assert_eq!(std::fs::read(repo.0.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(repo.0.join("tracked.txt")).unwrap(),
            "reviewed\n"
        );
    }
    #[test]
    fn ai_commit_refuses_nested_scope_and_in_progress_git_operations() {
        let repo = TestRepository::new();
        std::fs::write(repo.0.join("changed.txt"), "change\n").unwrap();
        std::fs::create_dir(repo.0.join("nested")).unwrap();
        assert!(
            ai_commit_context(repo.0.join("nested").to_str().unwrap())
                .unwrap_err()
                .contains("repository root")
        );
        std::fs::write(repo.0.join(".git/MERGE_HEAD"), "in-progress").unwrap();
        assert!(
            ai_commit_context(repo.root())
                .unwrap_err()
                .contains("merge/rebase")
        );
    }
    #[test]
    fn untracked_diff_is_scoped_and_never_fabricates_staged_content() {
        let repo = TestRepository::new();
        std::fs::write(repo.0.join("new.txt"), "new\n").unwrap();
        assert!(file_diff(repo.root(), "new.txt", true).unwrap().is_empty());
        assert!(
            file_diff(repo.root(), "new.txt", false)
                .unwrap()
                .contains("+new")
        );
        assert!(file_diff(repo.root(), "../outside.txt", false).is_err());
        assert!(file_diff(repo.root(), "/outside.txt", false).is_err());
        #[cfg(unix)]
        {
            let outside = TestRepository::new();
            std::fs::write(outside.0.join("secret.txt"), "not for this project").unwrap();
            std::os::unix::fs::symlink(outside.0.join("secret.txt"), repo.0.join("link.txt"))
                .unwrap();
            assert!(file_diff(repo.root(), "link.txt", false).is_err());
        }
    }
}
