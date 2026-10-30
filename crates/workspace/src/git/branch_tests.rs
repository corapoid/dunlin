use super::{branch_context, git, switch_branch, switch_branch_checked};
use std::{fs, path::PathBuf};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let counter = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dunlin-checkout-{}-{unique}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let repo = Self(root);
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Dunlin Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "core.hooksPath", ".git/no-hooks"]);
        repo.write("file.txt", "one\ntwo\n");
        repo.commit();
        repo
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        git(self.path(), args).unwrap()
    }
    fn write(&self, name: &str, text: &str) {
        fs::write(self.0.join(name), text).unwrap();
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap()
    }
    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "test fixture"]);
    }
    fn switch(&self, branch: &str) {
        switch_branch(self.path(), branch).unwrap();
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn confirmed_checkout_preserves_partial_staging_unstaged_and_untracked_work_without_a_stash() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    repo.switch("feature");
    repo.write("feature.txt", "feature\n");
    repo.commit();
    repo.switch("main");
    repo.write("file.txt", "one\nTWO\n");
    repo.git(&["add", "file.txt"]);
    repo.write("file.txt", "one\nTWO\nthree\n");
    repo.write("untracked.txt", "keep me\n");
    let expected = branch_context(repo.path()).unwrap();
    assert_eq!(
        (expected.staged, expected.unstaged, expected.untracked),
        (1, 1, 1)
    );
    let staged = repo.git(&["diff", "--cached", "--binary"]);
    let unstaged = repo.git(&["diff", "--binary"]);
    assert!(switch_branch_checked(repo.path(), "feature", &expected, false).is_err());
    assert_eq!(branch_context(repo.path()).unwrap(), expected);
    switch_branch_checked(repo.path(), "feature", &expected, true).unwrap();
    assert_eq!(branch_context(repo.path()).unwrap().current, "feature");
    assert_eq!(repo.read("file.txt"), "one\nTWO\nthree\n");
    assert_eq!(repo.git(&["show", ":file.txt"]), "one\nTWO\n");
    assert_eq!(repo.read("untracked.txt"), "keep me\n");
    assert_eq!(repo.git(&["diff", "--cached", "--binary"]), staged);
    assert_eq!(repo.git(&["diff", "--binary"]), unstaged);
    assert!(repo.git(&["stash", "list"]).is_empty());
}

#[test]
fn conflicting_tracked_and_untracked_files_abort_without_changing_head_or_content() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    repo.switch("feature");
    repo.write("file.txt", "feature version\n");
    repo.write("new.txt", "feature new\n");
    repo.commit();
    repo.switch("main");
    repo.write("file.txt", "local version\n");
    let expected = branch_context(repo.path()).unwrap();
    let staged = repo.git(&["show", ":file.txt"]);
    assert!(switch_branch_checked(repo.path(), "feature", &expected, true).is_err());
    assert_eq!(repo.read("file.txt"), "local version\n");
    assert_eq!(repo.git(&["show", ":file.txt"]), staged);
    assert_eq!(branch_context(repo.path()).unwrap(), expected);
    repo.write("file.txt", "one\ntwo\n");
    repo.write("new.txt", "untracked local\n");
    let expected = branch_context(repo.path()).unwrap();
    assert!(switch_branch_checked(repo.path(), "feature", &expected, true).is_err());
    assert_eq!(repo.read("new.txt"), "untracked local\n");
    assert_eq!(branch_context(repo.path()).unwrap(), expected);
}

#[test]
fn ignored_files_are_never_overwritten_even_when_status_looks_clean() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    repo.switch("feature");
    repo.write("settings.local", "tracked on feature\n");
    repo.commit();
    repo.switch("main");
    repo.write(".gitignore", "settings.local\n");
    repo.commit();
    repo.write("settings.local", "local private settings\n");
    let expected = branch_context(repo.path()).unwrap();
    assert!(!expected.dirty());
    assert!(switch_branch_checked(repo.path(), "feature", &expected, false).is_err());
    assert_eq!(repo.read("settings.local"), "local private settings\n");
    assert_eq!(branch_context(repo.path()).unwrap(), expected);
}

#[test]
fn stale_plans_flags_revisions_nested_roots_and_in_progress_operations_are_rejected() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    let expected = branch_context(repo.path()).unwrap();
    for branch in [
        "-f",
        "--discard-changes",
        "HEAD~1",
        "@{-1}",
        "not-a-local-branch",
    ] {
        assert!(switch_branch_checked(repo.path(), branch, &expected, true).is_err());
        assert_eq!(branch_context(repo.path()).unwrap(), expected);
    }
    repo.write("new.txt", "new\n");
    assert!(
        switch_branch_checked(repo.path(), "feature", &expected, true)
            .unwrap_err()
            .contains("repository changed")
    );
    let expected = branch_context(repo.path()).unwrap();
    repo.commit();
    assert!(switch_branch_checked(repo.path(), "feature", &expected, true).is_err());
    fs::create_dir(repo.0.join("nested")).unwrap();
    assert!(
        branch_context(repo.0.join("nested").to_str().unwrap())
            .unwrap_err()
            .contains("repository root")
    );
    repo.write(".git/MERGE_HEAD", "in progress");
    assert!(
        branch_context(repo.path())
            .unwrap_err()
            .contains("Git operation")
    );
    assert_eq!(
        repo.git(&["symbolic-ref", "--short", "HEAD"]).trim(),
        "main"
    );
}

#[test]
fn branches_used_by_another_worktree_are_not_forced_or_detached() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    let other = repo.0.join(".git/test-worktree");
    repo.git(&["worktree", "add", "-q", other.to_str().unwrap(), "feature"]);
    let expected = branch_context(repo.path()).unwrap();
    assert!(switch_branch_checked(repo.path(), "feature", &expected, false).is_err());
    assert_eq!(branch_context(repo.path()).unwrap(), expected);
    assert_eq!(repo.read("file.txt"), "one\ntwo\n");
}

#[cfg(unix)]
#[test]
fn a_retargeted_project_symlink_cannot_checkout_a_different_repository() {
    let first = Repo::new();
    let second = Repo::new();
    first.git(&["branch", "feature"]);
    second.git(&["branch", "feature"]);
    let link = first.0.join(".git/project-link");
    std::os::unix::fs::symlink(&first.0, &link).unwrap();
    let expected = branch_context(link.to_str().unwrap()).unwrap();
    assert_eq!(
        expected.root,
        first.0.canonicalize().unwrap().to_str().unwrap()
    );
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second.0, &link).unwrap();
    assert!(switch_branch_checked(link.to_str().unwrap(), "feature", &expected, false).is_err());
    assert_eq!(branch_context(first.path()).unwrap().current, "main");
    assert_eq!(branch_context(second.path()).unwrap().current, "main");
}

#[test]
fn ref_like_branch_names_cannot_resolve_to_a_different_local_branch() {
    let repo = Repo::new();
    repo.git(&["branch", "feature"]);
    repo.git(&["branch", "refs/heads/feature"]);
    let context = branch_context(repo.path()).unwrap();
    assert!(context.branches.iter().any(|b| b == "refs/heads/feature"));
    assert!(switch_branch_checked(repo.path(), "refs/heads/feature", &context, false).is_err());
    assert_eq!(branch_context(repo.path()).unwrap().current, "main");
}
