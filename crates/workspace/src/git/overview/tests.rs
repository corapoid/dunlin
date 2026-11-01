use super::*;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo =
            Self(std::env::temp_dir().join(format!("dunlin-inventory-{}", uuid::Uuid::now_v7())));
        fs::create_dir(&repo.0).unwrap();
        repo.git(&["init", "-qb", "main"]);
        repo
    }
    fn root(&self) -> &str {
        self.0.to_str().unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args([
                "-C",
                self.root(),
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "rerere.enabled=false",
            ])
            .args(args)
            .output()
            .unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn write(&self, file: &str, value: &str) {
        fs::write(self.0.join(file), value).unwrap();
    }
    fn commit(&self) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-qm", "fixture"]);
    }
    fn base(&self) {
        self.write("file", "base\n");
        self.commit();
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unborn_empty_and_dirty_stash_inventory_preserves_files_index_refs_and_reflog() {
    let repo = Repo::new();
    let empty = load(repo.root()).unwrap();
    assert!(empty.stashes.is_empty() && empty.conflicts.is_empty() && empty.operations.is_empty());
    assert_eq!(empty.worktrees.len(), 1);
    assert_eq!(
        empty.worktrees[0].branch.as_deref(),
        Some("refs/heads/main")
    );
    repo.base();
    for value in ["stash first\n", "stash second\n"] {
        repo.write("file", value);
        repo.git(&["stash", "push", "-qm", value.trim()]);
    }
    repo.write("file", "staged\n");
    repo.git(&["add", "file"]);
    repo.write("file", "unstaged\n");
    repo.write("untracked", "keep\n");
    let index = fs::read(repo.0.join(".git/index")).unwrap();
    let head = fs::read(repo.0.join(".git/HEAD")).unwrap();
    let reflog = fs::read(repo.0.join(".git/logs/refs/stash")).unwrap();
    let refs = repo.git(&["show-ref"]);
    let value = load(repo.root()).unwrap();
    assert_eq!(value.stashes.len(), 2);
    assert_eq!(value.stashes[0].selector, "stash@{0}");
    assert!(value.stashes[0].subject.contains("stash second"));
    assert!(value.stashes[1].subject.contains("stash first"));
    assert_eq!(fs::read(repo.0.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(repo.0.join(".git/HEAD")).unwrap(), head);
    assert_eq!(
        fs::read(repo.0.join(".git/logs/refs/stash")).unwrap(),
        reflog
    );
    assert_eq!(repo.git(&["show-ref"]), refs);
    assert_eq!(
        fs::read_to_string(repo.0.join("file")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("untracked")).unwrap(),
        "keep\n"
    );
}

#[test]
fn linked_detached_locked_and_prunable_worktrees_are_metadata_not_file_reads() {
    let repo = Repo::new();
    repo.base();
    let linked = repo.0.join("linked\nworktree");
    repo.git(&["worktree", "add", "-qb", "linked", linked.to_str().unwrap()]);
    repo.git(&[
        "worktree",
        "lock",
        "--reason",
        "keep\nthis",
        linked.to_str().unwrap(),
    ]);
    let detached = repo.0.join("detached");
    repo.git(&["worktree", "add", "--detach", detached.to_str().unwrap()]);
    let gone = repo.0.join("gone");
    repo.git(&["worktree", "add", "-qb", "gone", gone.to_str().unwrap()]);
    fs::remove_dir_all(&gone).unwrap();
    let value = load(linked.to_str().unwrap()).unwrap();
    assert_eq!(value.worktrees.len(), 4);
    let current = value.worktrees.iter().find(|w| w.current).unwrap();
    assert_eq!(current.branch.as_deref(), Some("refs/heads/linked"));
    assert_eq!(current.locked.as_deref(), Some("keep\nthis"));
    assert!(value.worktrees.iter().any(|w| w.detached));
    assert!(value.worktrees.iter().any(|w| w.prunable.is_some()));
    // Operation state is per worktree, not the main repository's state.
    repo.write(".git/MERGE_HEAD", "marker contents are not read");
    assert!(
        load(repo.root())
            .unwrap()
            .operations
            .contains(&Operation::Merge)
    );
    assert!(
        load(linked.to_str().unwrap())
            .unwrap()
            .operations
            .is_empty()
    );
    assert!(!gone.exists());
}

#[test]
fn actual_merge_rebase_and_cherry_pick_conflicts_are_reported_without_writes() {
    for command in ["merge", "rebase", "cherry-pick"] {
        let repo = Repo::new();
        repo.base();
        repo.git(&["checkout", "-qb", "side"]);
        repo.write("file", "side\n");
        repo.commit();
        let side = repo.git(&["rev-parse", "HEAD"]);
        repo.git(&["checkout", "main"]);
        repo.write("file", "main\n");
        repo.commit();
        let result = repo.run(&[
            command,
            if command == "cherry-pick" {
                side.trim()
            } else {
                "side"
            },
        ]);
        assert!(!result.status.success());
        let index = fs::read(repo.0.join(".git/index")).unwrap();
        let head = fs::read(repo.0.join(".git/HEAD")).unwrap();
        let file = fs::read(repo.0.join("file")).unwrap();
        let value = load(repo.root()).unwrap();
        assert!(value.operations.contains(&match command {
            "merge" => Operation::Merge,
            "rebase" => Operation::Rebase,
            _ => Operation::CherryPick,
        }));
        assert_eq!(
            value.conflicts,
            vec![Conflict {
                path: "file".into(),
                stages: vec![1, 2, 3]
            }]
        );
        assert_eq!(value.conflicts[0].label(), "Both modified");
        assert_eq!(fs::read(repo.0.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(repo.0.join(".git/HEAD")).unwrap(), head);
        assert_eq!(fs::read(repo.0.join("file")).unwrap(), file);
    }
}

#[test]
fn standalone_conflicts_and_resolved_operation_are_not_reported_as_clean() {
    let repo = Repo::new();
    repo.base();
    let object = repo.git(&["rev-parse", "HEAD:file"]);
    // NUL-delimited parser preserves whitespace and newlines in literal names.
    let raw = format!("100644 {} 2\todd\nname\tfile\0", object.trim());
    assert_eq!(conflicts(&raw).unwrap()[0].path, "odd\nname\tfile");
    repo.write(".git/CHERRY_PICK_HEAD", "marker");
    fs::create_dir(repo.0.join(".git/sequencer")).unwrap();
    let value = load(repo.root()).unwrap();
    assert!(value.conflicts.is_empty());
    assert_eq!(
        value.operations,
        vec![Operation::CherryPick, Operation::Sequencer]
    );
    fs::create_dir(repo.0.join(".git/rebase-apply")).unwrap();
    repo.write(".git/rebase-apply/applying", "marker");
    assert!(
        load(repo.root())
            .unwrap()
            .operations
            .contains(&Operation::ApplyMailbox)
    );
}

#[test]
fn malformed_and_oversized_inventory_is_rejected_without_truncation() {
    assert!(stashes("missing terminator").is_err());
    assert!(worktrees("worktree /repo\0HEAD bad\0", "/repo").is_err());
    assert!(conflicts("bad header\tfile\0").is_err());
    let repo = Repo::new();
    let valid = load(repo.root()).unwrap();
    let mut value = valid.clone();
    value.stashes = vec![
        Stash {
            oid: "a".repeat(40),
            selector: "stash@{0}".into(),
            timestamp: 0,
            subject: String::new()
        };
        MAX_ENTRIES + 1
    ];
    assert!(validate(&value).is_err());
    let mut value = valid.clone();
    value.conflicts = vec![Conflict {
        path: "../escape".into(),
        stages: vec![2],
    }];
    assert!(validate(&value).is_err());
    let mut value = valid.clone();
    value.worktrees[0].head = Some("bad".into());
    assert!(validate(&value).is_err());
    let mut value = valid;
    value.worktrees[0].locked = Some("x".repeat(2 * 1024 * 1024));
    assert!(validate(&value).is_err());
    fs::create_dir(repo.0.join("nested")).unwrap();
    assert!(load(repo.0.join("nested").to_str().unwrap()).is_err());
}

#[test]
fn stash_apply_conflicts_exist_without_an_operation_marker_and_are_never_called_clean() {
    let repo = Repo::new();
    repo.base();
    repo.write("file", "stashed\n");
    repo.git(&["stash", "push", "-qm", "saved"]);
    repo.write("file", "committed\n");
    repo.commit();
    assert!(!repo.run(&["stash", "apply"]).status.success());
    let index = fs::read(repo.0.join(".git/index")).unwrap();
    let value = load(repo.root()).unwrap();
    assert!(value.operations.is_empty());
    assert_eq!(value.stashes.len(), 1);
    assert_eq!(value.conflicts[0].stages, vec![1, 2, 3]);
    assert_eq!(fs::read(repo.0.join(".git/index")).unwrap(), index);
    for (stages, label) in [
        (vec![1, 2, 3], "Both modified"),
        (vec![2, 3], "Both added"),
        (vec![1, 2], "Deleted by them"),
        (vec![1, 3], "Deleted by us"),
        (vec![1], "Both deleted"),
        (vec![2], "Added by us"),
        (vec![3], "Added by them"),
    ] {
        assert_eq!(
            Conflict {
                path: "file".into(),
                stages
            }
            .label(),
            label
        );
    }
}

#[cfg(unix)]
#[test]
fn operation_symlinks_fail_closed_and_fsmonitor_is_not_executed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let repo = Repo::new();
    repo.base();
    let script = repo.0.join("monitor");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", repo.0.join("executed").display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    repo.git(&["config", "core.fsmonitor", script.to_str().unwrap()]);
    assert!(load(repo.root()).is_ok());
    assert!(!repo.0.join("executed").exists());
    symlink(repo.0.join("file"), repo.0.join(".git/MERGE_HEAD")).unwrap();
    assert!(load(repo.root()).is_err());
    assert_eq!(fs::read_to_string(repo.0.join("file")).unwrap(), "base\n");
}
