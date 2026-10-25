//! Thin wrapper around the system `git` CLI.
//!
//! Dunlin deliberately shells out instead of linking libgit2: the CLI is
//! already the source of truth for credentials, hooks and worktree config, and
//! it keeps the binary small. All calls run off the UI thread.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

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
            return GitStatus {
                branch,
                changes: Vec::new(),
                error: Some(error),
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
    }
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

/// A unified patch for one path, including an untracked file.
pub fn file_diff(path: &str, file: &str, staged: bool) -> Result<String, String> {
    const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;
    let args: &[&str] = if staged {
        &["diff", "--cached", "--no-color", "--", file]
    } else {
        &["diff", "--no-color", "--", file]
    };
    let patch = git(path, args)?;
    if patch.len() > MAX_PATCH_BYTES {
        return Err("This patch is too large to display".into());
    }
    if !patch.trim().is_empty() {
        return Ok(patch);
    }
    if git(path, &["ls-files", "--error-unmatch", "--", file]).is_ok() {
        return Ok(patch);
    }

    let full = Path::new(path).join(file);
    let metadata =
        std::fs::metadata(&full).map_err(|error| format!("Could not read {file}: {error}"))?;
    if metadata.len() > MAX_PATCH_BYTES as u64 {
        return Err("This file is too large to preview".into());
    }
    let bytes = std::fs::read(&full).map_err(|error| format!("Could not read {file}: {error}"))?;
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

/// Replace the last commit with the current index and a new message.
pub fn amend(path: &str, message: &str) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first".into());
    }
    git(path, &["commit", "--amend", "-m", message]).map(|_| ())
}

pub fn branches(path: &str) -> Result<Vec<String>, String> {
    let output = git(path, &["branch", "--format=%(refname:short)"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn switch_branch(path: &str, branch: &str) -> Result<(), String> {
    git(path, &["switch", branch]).map(|_| ())
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
}
