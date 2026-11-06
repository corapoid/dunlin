use super::{Action, execute, preflight};
use crate::git::{
    blame,
    overview::{self, Operation},
};
use std::{fs, process::Command};
struct Repo {
    dir: tempfile::TempDir,
}
impl Repo {
    fn new() -> Self {
        let r = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        r.git(&["init", "--template=", "-qb", "main"]);
        fs::create_dir(r.dir.path().join(".git/hooks")).unwrap();
        r.git(&[
            "config",
            "core.hooksPath",
            r.dir.path().join(".git/hooks").to_str().unwrap(),
        ]);
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "core.fsmonitor", "false"]);
        r.git(&["config", "core.attributesFile", "/dev/null"]);
        r.git(&["config", "user.email", "test@example.invalid"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r.write("file", "one\ntwo\n");
        r.git(&["add", "file"]);
        r.git(&["commit", "-qm", "first"]);
        r
    }
    fn root(&self) -> &str {
        self.dir.path().to_str().unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-C", self.root()])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn write(&self, name: &str, value: &str) {
        fs::write(self.dir.path().join(name), value).unwrap();
    }
    fn apply(&self, action: Action) -> super::Outcome {
        let plan = preflight(self.root(), action).unwrap();
        execute(self.root(), &plan).unwrap()
    }
    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().into()
    }
}
#[test]
fn discard_restores_partial_staging_renames_and_preserves_ignored_files() {
    let r = Repo::new();
    r.write(".gitignore", "secret\n");
    r.git(&["add", ".gitignore"]);
    r.git(&["commit", "-qm", "ignore"]);
    r.write("secret", "keep");
    r.write("file", "staged");
    r.git(&["add", "file"]);
    r.write("file", "unstaged");
    r.write("added", "new");
    r.git(&["add", "added"]);
    r.write("loose", "loose");
    r.apply(Action::Discard {
        paths: vec!["file".into(), "added".into(), "loose".into()],
    });
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "one\ntwo\n"
    );
    assert!(!r.dir.path().join("added").exists());
    assert!(!r.dir.path().join("loose").exists());
    assert_eq!(
        fs::read_to_string(r.dir.path().join("secret")).unwrap(),
        "keep"
    );
    r.git(&["mv", "file", "renamed"]);
    r.apply(Action::Discard {
        paths: vec!["renamed".into()],
    });
    assert!(r.dir.path().join("file").exists());
    assert!(!r.dir.path().join("renamed").exists());
    assert_eq!(r.git(&["status", "--porcelain"]), "");
}

#[test]
fn unstage_all_keeps_working_files_and_handles_unborn_head() {
    let r = Repo::new();
    r.write("file", "working");
    r.git(&["add", "file"]);
    r.apply(Action::UnstageAll);
    assert_eq!(r.git(&["diff", "--cached"]), "");
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "working"
    );
    r.git(&["checkout", "--orphan", "empty"]);
    r.git(&["add", "file"]);
    r.apply(Action::UnstageAll);
    assert_eq!(r.git(&["ls-files"]), "");
    assert!(r.dir.path().join("file").exists());
    r.git(&["add", "file"]);
    r.apply(Action::Discard {
        paths: vec!["file".into()],
    });
    assert!(!r.dir.path().join("file").exists());
}

#[test]
fn discard_consent_rejects_content_changes_and_literal_pathspecs_do_not_expand() {
    let r = Repo::new();
    r.write("file", "approved");
    let plan = preflight(
        r.root(),
        Action::Discard {
            paths: vec!["file".into()],
        },
    )
    .unwrap();
    r.write("file", "changed after approval");
    assert!(execute(r.root(), &plan).is_err());
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "changed after approval"
    );
    r.write("*", "literal");
    r.apply(Action::Discard {
        paths: vec!["*".into()],
    });
    assert!(!r.dir.path().join("*").exists());
    assert!(r.dir.path().join("file").exists());
    for path in ["../file", ".git/config", "/file", "directory"] {
        assert!(
            preflight(
                r.root(),
                Action::Discard {
                    paths: vec![path.into()]
                }
            )
            .is_err()
        );
    }
}

#[test]
fn ignore_appends_literal_root_pattern_preserves_existing_and_checks_snapshot() {
    let r = Repo::new();
    r.write(".gitignore", "old");
    r.write("a[1]* #!.txt", "literal");
    r.apply(Action::Ignore {
        path: "a[1]* #!.txt".into(),
    });
    assert!(
        r.git(&["check-ignore", "--", "a[1]* #!.txt"])
            .contains("a[1]* #!.txt")
    );
    assert!(
        fs::read_to_string(r.dir.path().join(".gitignore"))
            .unwrap()
            .starts_with("old\n/")
    );
    let before = fs::read(r.dir.path().join(".gitignore")).unwrap();
    r.apply(Action::Ignore {
        path: "a[1]* #!.txt".into(),
    });
    assert_eq!(before, fs::read(r.dir.path().join(".gitignore")).unwrap());
    let plan = preflight(
        r.root(),
        Action::Ignore {
            path: "file".into(),
        },
    )
    .unwrap();
    r.write(".gitignore", "different\n");
    assert!(execute(r.root(), &plan).is_err());
    r.apply(Action::Ignore {
        path: "file".into(),
    });
    assert!(r.git(&["ls-files"]).contains("file"));
}

#[cfg(unix)]
#[test]
fn discard_and_ignore_reject_unsafe_parents_nested_repositories_and_hardlinks() {
    use std::os::unix::fs::symlink;
    let r = Repo::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "keep").unwrap();
    symlink(outside.path(), r.dir.path().join("link")).unwrap();
    assert!(
        preflight(
            r.root(),
            Action::Ignore {
                path: "link/file".into()
            }
        )
        .is_err()
    );
    symlink(outside.path().join("file"), r.dir.path().join(".gitignore")).unwrap();
    assert!(
        preflight(
            r.root(),
            Action::Ignore {
                path: "file".into()
            }
        )
        .is_err()
    );
    fs::remove_file(r.dir.path().join(".gitignore")).unwrap();
    fs::hard_link(outside.path().join("file"), r.dir.path().join(".gitignore")).unwrap();
    assert!(
        preflight(
            r.root(),
            Action::Ignore {
                path: "file".into()
            }
        )
        .is_err()
    );
    fs::remove_file(r.dir.path().join(".gitignore")).unwrap();
    r.apply(Action::Discard {
        paths: vec!["link".into()],
    });
    assert_eq!(
        fs::read_to_string(outside.path().join("file")).unwrap(),
        "keep"
    );
    fs::create_dir(r.dir.path().join("nested")).unwrap();
    r.git(&["-C", "nested", "init", "--template=", "-q"]);
    r.write("nested/local", "keep");
    assert!(
        preflight(
            r.root(),
            Action::Discard {
                paths: vec!["nested/".into()]
            }
        )
        .is_err()
    );
    assert!(
        preflight(
            r.root(),
            Action::Ignore {
                path: "nested/local".into()
            }
        )
        .is_err()
    );
}

#[test]
fn switch_preserves_or_discards_changes_only_after_explicit_fresh_plan() {
    let r = Repo::new();
    r.git(&["branch", "feature"]);
    r.write("file", "working");
    r.apply(Action::Switch {
        branch: "feature".into(),
        discard: false,
    });
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "working"
    );
    r.write("loose", "untracked");
    r.apply(Action::Switch {
        branch: "main".into(),
        discard: true,
    });
    assert_eq!(r.git(&["branch", "--show-current"]).trim(), "main");
    assert_eq!(r.git(&["status", "--porcelain"]), "");
    assert!(!r.dir.path().join("loose").exists());
    r.write("file", "keep");
    assert!(
        preflight(
            r.root(),
            Action::Switch {
                branch: "missing".into(),
                discard: true
            }
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "keep"
    );
}

#[test]
fn deleted_submodules_and_project_subfolders_never_authorize_root_discard() {
    let r = Repo::new();
    let head = r.head();
    r.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000",
        &head,
        "module",
    ]);
    r.git(&["commit", "-qm", "gitlink"]);
    r.git(&["rm", "--cached", "module"]);
    assert!(
        preflight(
            r.root(),
            Action::Discard {
                paths: vec!["module".into()]
            }
        )
        .is_err()
    );
    fs::create_dir(r.dir.path().join("subfolder")).unwrap();
    r.write("file", "keep");
    assert!(
        preflight(
            r.dir.path().join("subfolder").to_str().unwrap(),
            Action::Discard {
                paths: vec!["file".into()]
            }
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "keep"
    );
}

#[test]
fn switch_rejects_checked_out_branch_and_ignored_collisions_before_discard() {
    let r = Repo::new();
    r.git(&["branch", "feature"]);
    let parent = tempfile::tempdir().unwrap();
    let tree = parent.path().join("tree");
    r.git(&["worktree", "add", tree.to_str().unwrap(), "feature"]);
    r.write("file", "keep");
    assert!(
        preflight(
            r.root(),
            Action::Switch {
                branch: "feature".into(),
                discard: true
            }
        )
        .is_err()
    );
    r.git(&["worktree", "remove", tree.to_str().unwrap()]);
    r.git(&["switch", "feature"]);
    r.write("secret", "target");
    r.git(&["add", "secret"]);
    r.git(&["commit", "-qm", "target"]);
    r.git(&["switch", "main"]);
    r.write(".gitignore", "secret\n");
    r.write("secret", "preserve ignored");
    assert!(
        preflight(
            r.root(),
            Action::Switch {
                branch: "feature".into(),
                discard: true
            }
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("secret")).unwrap(),
        "preserve ignored"
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "keep"
    );
}

#[test]
fn stash_push_apply_pop_drop_preserve_partial_staging_and_untracked() {
    let r = Repo::new();
    r.write("file", "staged\ntwo\n");
    r.git(&["add", "file"]);
    r.write("file", "unstaged\ntwo\n");
    r.write("new", "untracked");
    let index = r.git(&["show", ":file"]);
    assert!(
        r.apply(Action::StashPush {
            message: "fixture".into(),
            include_untracked: true
        })
        .success
    );
    assert_eq!(r.git(&["status", "--porcelain"]), "");
    let oid = overview::load(r.root()).unwrap().stashes[0].oid.clone();
    assert!(r.apply(Action::StashApply { oid: oid.clone() }).success);
    assert_eq!(r.git(&["show", ":file"]), index);
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "unstaged\ntwo\n"
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("new")).unwrap(),
        "untracked"
    );
    assert!(r.apply(Action::StashDrop { oid }).success);
    assert!(overview::load(r.root()).unwrap().stashes.is_empty());
    r.apply(Action::StashPush {
        message: "pop".into(),
        include_untracked: true,
    });
    let oid = overview::load(r.root()).unwrap().stashes[0].oid.clone();
    assert!(r.apply(Action::StashPop { oid }).success);
    assert!(overview::load(r.root()).unwrap().stashes.is_empty());
}
#[test]
fn stale_content_refs_targets_and_repository_locks_fail_before_mutation() {
    let r = Repo::new();
    r.write("file", "dirty");
    let action = Action::StashPush {
        message: "snapshot".into(),
        include_untracked: false,
    };
    let plan = preflight(r.root(), action.clone()).unwrap();
    r.write("file", "other");
    assert!(execute(r.root(), &plan).is_err());
    assert_eq!(
        fs::read_to_string(r.dir.path().join("file")).unwrap(),
        "other"
    );
    fs::write(r.dir.path().join(".git/index.lock"), "other-process").unwrap();
    assert!(preflight(r.root(), action).is_err());
}
#[test]
fn clean_merge_and_rebase_and_cherry_pick_use_pinned_local_objects() {
    let r = Repo::new();
    r.git(&["checkout", "-qb", "topic"]);
    r.write("other", "topic");
    r.git(&["add", "other"]);
    r.git(&["commit", "-qm", "topic"]);
    let topic = r.head();
    r.git(&["checkout", "-q", "main"]);
    assert!(
        r.apply(Action::CherryPick {
            revision: topic.clone(),
            mainline: None
        })
        .success
    );
    r.git(&["checkout", "-qb", "new-topic"]);
    r.write("another", "next");
    r.git(&["add", "another"]);
    r.git(&["commit", "-qm", "next"]);
    r.git(&["checkout", "-q", "main"]);
    assert!(
        r.apply(Action::Merge {
            revision: "new-topic".into()
        })
        .success
    );
    assert!(
        r.apply(Action::Rebase {
            revision: "topic".into()
        })
        .success
    );
    assert!(
        preflight(
            r.root(),
            Action::Merge {
                revision: "--all".into()
            }
        )
        .is_err()
    );
}
fn conflicting(r: &Repo) {
    r.git(&["checkout", "-qb", "topic"]);
    r.write("file", "theirs\n");
    r.git(&["commit", "-qam", "theirs"]);
    r.git(&["checkout", "-q", "main"]);
    r.write("file", "ours\n");
    r.git(&["commit", "-qam", "ours"]);
}
#[test]
fn merge_conflicts_can_be_staged_and_explicitly_continued() {
    let r = Repo::new();
    conflicting(&r);
    let result = r.apply(Action::Merge {
        revision: "topic".into(),
    });
    assert!(!result.success);
    assert!(!result.overview.conflicts.is_empty());
    assert!(
        preflight(
            r.root(),
            Action::Continue {
                operation: Operation::Merge
            }
        )
        .is_err()
    );
    r.write("file", "resolved\n");
    r.git(&["add", "file"]);
    assert!(
        r.apply(Action::Continue {
            operation: Operation::Merge
        })
        .success
    );
    assert!(overview::load(r.root()).unwrap().operations.is_empty());
}
#[test]
fn rebase_and_cherry_pick_conflicts_abort_without_losing_original_commits() {
    for operation in [Operation::Rebase, Operation::CherryPick] {
        let r = Repo::new();
        conflicting(&r);
        let head = r.head();
        let action = if operation == Operation::Rebase {
            Action::Rebase {
                revision: "topic".into(),
            }
        } else {
            Action::CherryPick {
                revision: "topic".into(),
                mainline: None,
            }
        };
        let result = r.apply(action);
        assert!(!result.success);
        assert!(result.overview.operations.contains(&operation));
        assert!(r.apply(Action::Abort { operation }).success);
        assert_eq!(r.head(), head);
        assert_eq!(
            fs::read_to_string(r.dir.path().join("file")).unwrap(),
            "ours\n"
        );
    }
}
#[test]
fn stash_pop_conflict_never_drops_the_stash() {
    let r = Repo::new();
    r.write("file", "stash\n");
    r.apply(Action::StashPush {
        message: "conflict".into(),
        include_untracked: false,
    });
    let oid = overview::load(r.root()).unwrap().stashes[0].oid.clone();
    r.write("file", "committed\n");
    r.git(&["commit", "-qam", "changed"]);
    let result = r.apply(Action::StashPop { oid: oid.clone() });
    assert!(!result.success);
    assert_eq!(overview::load(r.root()).unwrap().stashes[0].oid, oid);
}
#[test]
fn worktrees_add_remove_and_prune_never_force_dirty_or_locked_directories() {
    let r = Repo::new();
    assert!(
        r.apply(Action::WorktreeAdd {
            name: "fixture".into(),
            revision: "HEAD".into()
        })
        .success
    );
    let value = overview::load(r.root()).unwrap();
    let tree = value.worktrees.iter().find(|t| !t.current).unwrap();
    let path = tree.path.clone();
    fs::write(Path::new(&path).join("untracked"), "keep").unwrap();
    assert!(preflight(r.root(), Action::WorktreeRemove { path: path.clone() }).is_err());
    fs::remove_file(Path::new(&path).join("untracked")).unwrap();
    let tree_git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(["-C", &path])
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    };
    tree_git(&["update-index", "--assume-unchanged", "file"]);
    fs::write(Path::new(&path).join("file"), "hidden changes must survive").unwrap();
    assert!(preflight(r.root(), Action::WorktreeRemove { path: path.clone() }).is_err());
    assert_eq!(
        fs::read_to_string(Path::new(&path).join("file")).unwrap(),
        "hidden changes must survive"
    );
    tree_git(&["update-index", "--no-assume-unchanged", "file"]);
    fs::write(Path::new(&path).join("file"), "one\ntwo\n").unwrap();
    r.git(&["worktree", "lock", &path]);
    assert!(preflight(r.root(), Action::WorktreeRemove { path: path.clone() }).is_err());
    r.git(&["worktree", "unlock", &path]);
    assert!(
        r.apply(Action::WorktreeRemove { path: path.clone() })
            .success
    );
    assert!(!Path::new(&path).exists());
    assert!(r.apply(Action::WorktreePrune).success);
    assert!(
        preflight(
            r.root(),
            Action::WorktreeAdd {
                name: "../escape".into(),
                revision: "HEAD".into()
            }
        )
        .is_err()
    );
}
#[test]
fn subtask_worktree_creates_one_named_branch_and_refuses_duplicates() {
    let r = Repo::new();
    let plan = preflight(
        r.root(),
        Action::SubtaskAdd {
            name: "review-api".into(),
            revision: "HEAD".into(),
        },
    )
    .unwrap();
    let path = plan.worktree_target.clone().unwrap();
    assert!(execute(r.root(), &plan).unwrap().success);
    let tree = overview::load(r.root()).unwrap();
    let linked = tree
        .worktrees
        .iter()
        .find(|tree| tree.path == path)
        .unwrap();
    assert_eq!(
        linked.branch.as_deref(),
        Some("refs/heads/dunlin/subtask/review-api")
    );
    assert!(!linked.detached);
    assert!(Path::new(&path).join("file").is_file());
    assert!(
        preflight(
            r.root(),
            Action::SubtaskAdd {
                name: "review-api".into(),
                revision: "HEAD".into(),
            }
        )
        .is_err()
    );
    assert!(
        preflight(
            r.root(),
            Action::SubtaskAdd {
                name: "bad/name".into(),
                revision: "HEAD".into(),
            }
        )
        .is_err()
    );
    assert!(execute(r.root(), &plan).is_err());
}

#[cfg(unix)]
#[test]
fn hook_failure_is_reported_without_force_or_automatic_rollback() {
    use std::os::unix::fs::PermissionsExt;
    let r = Repo::new();
    r.git(&["checkout", "-qb", "topic"]);
    r.write("other", "topic");
    r.git(&["add", "other"]);
    r.git(&["commit", "-qm", "topic"]);
    r.git(&["checkout", "-q", "main"]);
    r.write("another", "main");
    r.git(&["add", "another"]);
    r.git(&["commit", "-qm", "main"]);
    let hook = r.dir.path().join(".git/hooks/pre-merge-commit");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let head = r.head();
    let result = r.apply(Action::Merge {
        revision: "topic".into(),
    });
    assert!(!result.success);
    assert_eq!(r.head(), head);
    assert!(result.overview.operations.contains(&Operation::Merge));
}
#[test]
fn blame_reads_head_objects_not_dirty_worktree_or_symlink_targets() {
    let r = Repo::new();
    let head = r.head();
    r.write("file", "dirty\n");
    let value = blame::load(r.root(), "file", Some(&head)).unwrap();
    assert_eq!(value.lines.len(), 2);
    assert_eq!(value.lines[0].text, "one");
    assert_eq!(value.lines[0].author, "Test");
    assert!(blame::load(r.root(), "../outside", None).is_err());
    assert!(blame::load(r.root(), "file", Some(&"a".repeat(40))).is_err());
}
use std::path::Path;

#[test]
fn ignored_build_outputs_are_preserved_but_colliding_ignored_paths_block_mutations() {
    let r = Repo::new();
    r.write(".gitignore", "build/\nignored\n");
    r.git(&["add", ".gitignore"]);
    r.git(&["commit", "-qm", "ignore outputs"]);
    r.git(&["checkout", "-qb", "topic"]);
    r.write("other", "topic");
    r.git(&["add", "other"]);
    r.git(&["commit", "-qm", "topic"]);
    r.git(&["checkout", "-q", "main"]);
    fs::create_dir(r.dir.path().join("build")).unwrap();
    r.write("build/output", "keep");
    assert!(
        r.apply(Action::Merge {
            revision: "topic".into()
        })
        .success
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("build/output")).unwrap(),
        "keep"
    );
    r.git(&["checkout", "-qb", "collision"]);
    r.write("ignored", "tracked upstream");
    r.git(&["add", "-f", "ignored"]);
    r.git(&["commit", "-qm", "incoming ignored path"]);
    r.git(&["checkout", "-q", "main"]);
    r.write("ignored", "local keep");
    for action in [
        Action::Merge {
            revision: "collision".into(),
        },
        Action::Rebase {
            revision: "collision".into(),
        },
        Action::CherryPick {
            revision: "collision".into(),
            mainline: None,
        },
    ] {
        assert!(preflight(r.root(), action).is_err());
    }
    assert_eq!(
        fs::read_to_string(r.dir.path().join("ignored")).unwrap(),
        "local keep"
    );
}
#[test]
fn rebase_and_cherry_pick_continue_after_resolving_conflicts() {
    for operation in [Operation::Rebase, Operation::CherryPick] {
        let r = Repo::new();
        conflicting(&r);
        let action = if operation == Operation::Rebase {
            Action::Rebase {
                revision: "topic".into(),
            }
        } else {
            Action::CherryPick {
                revision: "topic".into(),
                mainline: None,
            }
        };
        assert!(!r.apply(action).success);
        r.write("file", "resolved\n");
        r.git(&["add", "file"]);
        let result = r.apply(Action::Continue { operation });
        assert!(result.success, "{}", result.message);
        assert!(result.overview.operations.is_empty());
    }
}
#[test]
fn stash_and_revision_changes_invalidate_consent_and_common_lock_serializes_worktrees() {
    let r = Repo::new();
    r.write("file", "dirty\n");
    r.apply(Action::StashPush {
        message: "first".into(),
        include_untracked: false,
    });
    let oid = overview::load(r.root()).unwrap().stashes[0].oid.clone();
    let plan = preflight(r.root(), Action::StashDrop { oid }).unwrap();
    r.write("file", "new stash\n");
    r.git(&["stash", "push", "-qm", "concurrent"]);
    assert!(execute(r.root(), &plan).is_err());
    assert_eq!(overview::load(r.root()).unwrap().stashes.len(), 2);
    r.git(&["branch", "topic"]);
    let plan = preflight(
        r.root(),
        Action::Merge {
            revision: "topic".into(),
        },
    )
    .unwrap();
    r.git(&["branch", "other"]);
    assert!(execute(r.root(), &plan).is_err());
    let lock = super::lock(&plan.common).unwrap();
    assert!(execute(r.root(), &plan).is_err());
    drop(lock);
    let plan = preflight(
        r.root(),
        Action::Merge {
            revision: "topic".into(),
        },
    )
    .unwrap();
    assert!(execute(r.root(), &plan).unwrap().success);
}
#[cfg(unix)]
#[test]
fn same_path_repository_replacement_and_escaping_dirty_ancestors_are_rejected() {
    let r = Repo::new();
    let plan = preflight(r.root(), Action::WorktreePrune).unwrap();
    let backup = tempfile::tempdir().unwrap();
    let moved = backup.path().join("original");
    fs::rename(r.dir.path(), &moved).unwrap();
    fs::create_dir(r.dir.path()).unwrap();
    let output = Command::new("cp")
        .args([
            "-R",
            moved.join(".git").to_str().unwrap(),
            r.dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::copy(moved.join("file"), r.dir.path().join("file")).unwrap();
    assert!(execute(r.root(), &plan).is_err());
    let r = Repo::new();
    fs::create_dir(r.dir.path().join("folder")).unwrap();
    r.write("folder/file", "base");
    r.git(&["add", "folder/file"]);
    r.git(&["commit", "-qm", "folder"]);
    fs::remove_dir_all(r.dir.path().join("folder")).unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("file"), "private").unwrap();
    std::os::unix::fs::symlink(external.path(), r.dir.path().join("folder")).unwrap();
    assert!(
        preflight(
            r.root(),
            Action::StashPush {
                message: "safe".into(),
                include_untracked: false
            }
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(external.path().join("file")).unwrap(),
        "private"
    );
}
#[test]
fn worktree_prune_only_removes_stale_metadata_and_removal_protects_ignored_content() {
    let r = Repo::new();
    r.apply(Action::WorktreeAdd {
        name: "stale".into(),
        revision: "HEAD".into(),
    });
    let inventory = overview::load(r.root()).unwrap();
    let path = inventory
        .worktrees
        .iter()
        .find(|t| !t.current)
        .unwrap()
        .path
        .clone();
    fs::write(Path::new(&path).join(".gitignore"), "ignored\n").unwrap();
    let out = Command::new("git")
        .args(["-C", &path, "add", ".gitignore"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = Command::new("git")
        .args(["-C", &path, "commit", "-qm", "ignore"])
        .output()
        .unwrap();
    assert!(out.status.success());
    fs::write(Path::new(&path).join("ignored"), "keep").unwrap();
    assert!(preflight(r.root(), Action::WorktreeRemove { path: path.clone() }).is_err());
    fs::remove_dir_all(&path).unwrap();
    assert!(r.apply(Action::WorktreePrune).success);
    assert_eq!(overview::load(r.root()).unwrap().worktrees.len(), 1);
}
#[test]
fn skip_worktree_or_assume_unchanged_cannot_hide_dirty_files_from_preflight() {
    let r = Repo::new();
    r.git(&["update-index", "--assume-unchanged", "file"]);
    r.write("file", "hidden\n");
    assert!(
        preflight(
            r.root(),
            Action::Merge {
                revision: "HEAD".into()
            }
        )
        .is_err()
    );
    r.git(&["update-index", "--no-assume-unchanged", "file"]);
    r.git(&["update-index", "--skip-worktree", "file"]);
    assert!(
        preflight(
            r.root(),
            Action::StashPush {
                message: "".into(),
                include_untracked: false
            }
        )
        .is_err()
    );
}
#[test]
fn cherry_pick_merge_requires_and_honors_explicit_mainline() {
    let r = Repo::new();
    r.git(&["checkout", "-qb", "topic"]);
    r.write("other", "topic");
    r.git(&["add", "other"]);
    r.git(&["commit", "-qm", "topic"]);
    r.git(&["checkout", "-q", "main"]);
    let base = r.head();
    r.git(&["merge", "--no-ff", "-qm", "merge", "topic"]);
    let merge = r.head();
    r.git(&["checkout", "-qb", "pick", &base]);
    assert!(
        preflight(
            r.root(),
            Action::CherryPick {
                revision: merge.clone(),
                mainline: None
            }
        )
        .is_err()
    );
    assert!(
        preflight(
            r.root(),
            Action::CherryPick {
                revision: merge.clone(),
                mainline: Some(3)
            }
        )
        .is_err()
    );
    assert!(
        r.apply(Action::CherryPick {
            revision: merge,
            mainline: Some(1)
        })
        .success
    );
    assert_eq!(
        fs::read_to_string(r.dir.path().join("other")).unwrap(),
        "topic"
    );
}

#[cfg(unix)]
#[test]
fn timed_out_hooks_are_stopped_without_retrying_or_inventing_success() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    let r = Repo::new();
    let hook = r.dir.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nsleep 30\nexit 0\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let head = r.head();
    let started = Instant::now();
    assert!(
        super::run_with_timeout(
            r.root(),
            &["commit", "--allow-empty", "-qm", "never"],
            Duration::from_millis(150)
        )
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(r.head(), head);
}

#[test]
fn preflight_does_not_execute_configured_content_filters_before_confirmation() {
    let r = Repo::new();
    r.write(".gitattributes", "file filter=spy\n");
    r.git(&["add", ".gitattributes"]);
    r.git(&["commit", "-qm", "attributes"]);
    let marker = r.dir.path().join(".git/filter-ran");
    let command = format!(
        "touch '{}'; cat",
        marker.to_string_lossy().replace('\'', "'\\''")
    );
    r.git(&["config", "filter.spy.clean", &command]);
    r.write("file", "dirty\n");
    let plan = preflight(
        r.root(),
        Action::StashPush {
            message: "explicit".into(),
            include_untracked: false,
        },
    )
    .unwrap();
    assert!(!marker.exists());
    assert!(execute(r.root(), &plan).unwrap().success);
    assert!(
        marker.exists(),
        "native filters run only after the explicit execution boundary"
    );
}

#[cfg(unix)]
#[test]
fn blame_is_immutable_across_worktree_mailmaps_external_ignore_files_and_symlinks() {
    let r = Repo::new();
    let external = tempfile::tempdir().unwrap();
    let mailmap = external.path().join("mailmap");
    fs::write(
        &mailmap,
        "Invented <invented@example.invalid> Test <test@example.invalid>\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&mailmap, r.dir.path().join(".mailmap")).unwrap();
    r.git(&["config", "mailmap.file", mailmap.to_str().unwrap()]);
    r.git(&[
        "config",
        "blame.ignoreRevsFile",
        external.path().join("missing").to_str().unwrap(),
    ]);
    assert!(blame::load(r.root(), "file", None).is_err());
    r.git(&["config", "--unset-all", "blame.ignoreRevsFile"]);
    let value = blame::load(r.root(), "file", None).unwrap();
    assert_eq!(value.lines[0].author, "Test");
    std::os::unix::fs::symlink(&mailmap, r.dir.path().join("link")).unwrap();
    r.git(&["add", "link"]);
    r.git(&["commit", "-qm", "symlink"]);
    assert!(blame::load(r.root(), "link", None).is_err());
}

#[test]
fn abort_cannot_overwrite_untracked_content_that_blocks_restoration() {
    let r = Repo::new();
    conflicting(&r);
    assert!(
        !r.apply(Action::Merge {
            revision: "topic".into()
        })
        .success
    );
    r.git(&["rm", "--cached", "-f", "file"]);
    let value = fs::read(r.dir.path().join("file")).unwrap();
    assert!(
        preflight(
            r.root(),
            Action::Abort {
                operation: Operation::Merge
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(r.dir.path().join("file")).unwrap(), value);
    let backup = tempfile::tempdir().unwrap();
    fs::rename(r.dir.path().join("file"), backup.path().join("retained")).unwrap();
    assert!(
        r.apply(Action::Abort {
            operation: Operation::Merge
        })
        .success
    );
    assert_eq!(fs::read(backup.path().join("retained")).unwrap(), value);
}

#[test]
fn benign_index_stat_cache_refresh_does_not_invalidate_semantic_consent() {
    let r = Repo::new();
    let plan = preflight(
        r.root(),
        Action::Merge {
            revision: "HEAD".into(),
        },
    )
    .unwrap();
    let index = fs::read(r.dir.path().join(".git/index")).unwrap();
    // APFS/Git stat-cache precision varies by host. Cross a whole second so a
    // same-content write reliably changes the cached mtime and refreshes the
    // index without changing its semantic entries.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    r.write("file", "one\ntwo\n");
    r.git(&["update-index", "--refresh"]);
    assert_ne!(fs::read(r.dir.path().join(".git/index")).unwrap(), index);
    assert!(execute(r.root(), &plan).unwrap().success);
}
