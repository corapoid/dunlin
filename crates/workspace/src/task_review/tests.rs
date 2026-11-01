use super::*;
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap()
                .status
                .success()
        );
    };
    git(&["init", "--template=", "-qb", "main"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    fs::write(dir.path().join("file"), "base\n").unwrap();
    git(&["add", "file"]);
    git(&["commit", "-qm", "base"]);
    dir
}
#[test]
fn dirty_baseline_is_not_misattributed_and_restore_preserves_index() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    fs::write(dir.path().join("file"), "user changes\n").unwrap();
    let checkpoint = capture(root, "Before task").unwrap();
    assert!(changes(&checkpoint).unwrap().is_empty());
    fs::write(dir.path().join("file"), "after task\n").unwrap();
    let changed = changes(&checkpoint).unwrap();
    assert_eq!(
        changed[0].before.as_ref().unwrap().text().unwrap(),
        "user changes\n"
    );
    let index = fs::read(dir.path().join(".git/index")).unwrap();
    restore(prepare_restore(&checkpoint, "file").unwrap()).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("file")).unwrap(),
        "user changes\n"
    );
    assert_eq!(fs::read(dir.path().join(".git/index")).unwrap(), index);
}
#[test]
fn stale_restore_is_rejected_and_new_files_are_deleted_only_explicitly() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let checkpoint = capture(root, "Before task").unwrap();
    fs::write(dir.path().join("new"), "new").unwrap();
    let plan = prepare_restore(&checkpoint, "new").unwrap();
    fs::write(dir.path().join("new"), "later user edit").unwrap();
    assert!(restore(plan).is_err());
    assert!(dir.path().join("new").exists());
    restore(prepare_restore(&checkpoint, "new").unwrap()).unwrap();
    assert!(!dir.path().join("new").exists());
}

#[test]
fn malformed_checkpoints_and_expired_restore_plans_never_mutate() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let mut point = capture(root, "Before task").unwrap();
    let mut plan = prepare_restore(&point, "file").unwrap();
    plan.prepared = std::time::Instant::now() - Duration::from_secs(301);
    assert!(restore(plan).is_err());
    point.head = "--invalid-option".into();
    assert!(changes(&point).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("file")).unwrap(),
        "base\n"
    );
}

#[cfg(unix)]
#[test]
fn clean_git_files_retain_private_permissions_in_the_checkpoint() {
    use std::os::unix::fs::PermissionsExt;
    let dir = repo();
    let file = dir.path().join("file");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let point = capture(dir.path().to_str().unwrap(), "Private file permissions").unwrap();
    fs::write(&file, "later edit\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    restore(prepare_restore(&point, "file").unwrap()).unwrap();
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "base\n");
}
#[cfg(unix)]
#[test]
fn binary_changes_are_preserved_and_symlinks_cannot_escape() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    fs::write(dir.path().join("binary"), [0, 255, 2]).unwrap();
    let checkpoint = capture(root, "Binary overlay").unwrap();
    fs::write(dir.path().join("binary"), [0, 1]).unwrap();
    restore(prepare_restore(&checkpoint, "binary").unwrap()).unwrap();
    assert_eq!(fs::read(dir.path().join("binary")).unwrap(), [0, 255, 2]);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
    assert!(prepare_restore(&checkpoint, "escape/file").is_err());
    assert!(prepare_restore(&checkpoint, ".git/config").is_err());
}
