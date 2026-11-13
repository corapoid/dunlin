use super::*;
use std::fs;
fn git(root: &Path, args: &[&str]) -> String {
    let output = directory(root.to_str().unwrap())
        .args([
            "-c",
            "user.name=Selection fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn repo(root: &Path, text: &str) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "--template=", "-qb", "main"]);
    fs::write(root.join("file"), text).unwrap();
    git(root, &["add", "file"]);
    git(root, &["commit", "-qm", "fixture"]);
}
fn inject(command: &mut Command, other: &Path) {
    command
        .env("GIT_DIR", other.join(".git"))
        .env("GIT_WORK_TREE", other)
        .env("GIT_COMMON_DIR", other.join(".git"))
        .env("GIT_OBJECT_DIRECTORY", other.join(".git/objects"))
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            other.join(".git/objects"),
        )
        .env("GIT_INDEX_FILE", other.join(".git/index"))
        .env("GIT_NAMESPACE", "foreign")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "core.worktree")
        .env("GIT_CONFIG_VALUE_0", other)
        .env("GIT_CONFIG_KEY_1", "core.bare")
        .env("GIT_CONFIG_VALUE_1", "true")
        .env("GIT_CONFIG_PARAMETERS", "'core.worktree=/foreign'")
        .env("GIT_CONFIG_GLOBAL", other.join(".git/config"));
}
#[test]
fn actual_git_scrubs_injected_repository_paths_index_objects_and_config_without_global_env_changes()
{
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("selected");
    let other = dir.path().join("other");
    repo(&selected, "SELECTED\n");
    repo(&other, "FOREIGN\n");
    let other_index = fs::read(other.join(".git/index")).unwrap();
    let mut command = Command::new("git");
    inject(&mut command, &other);
    bind_worktree(&mut command, &selected);
    let output = command.args(["show", "HEAD:file"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "SELECTED\n");
    fs::write(selected.join("file"), "selected update\n").unwrap();
    let mut command = Command::new("git");
    inject(&mut command, &other);
    bind_worktree(&mut command, &selected);
    assert!(
        command
            .args(["add", "--", "file"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(other.join(".git/index")).unwrap(), other_index);
    assert_eq!(fs::read_to_string(other.join("file")).unwrap(), "FOREIGN\n");
}
#[test]
fn repository_core_worktree_cannot_redirect_production_reads_or_restores() {
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("selected");
    let other = dir.path().join("other");
    repo(&selected, "SELECTED\n");
    repo(&other, "FOREIGN\n");
    git(
        &selected,
        &["config", "core.worktree", other.to_str().unwrap()],
    );
    fs::write(selected.join("file"), "selected dirty\n").unwrap();
    fs::write(other.join("file"), "foreign must survive\n").unwrap();
    assert_eq!(
        crate::git::head_file(selected.to_str().unwrap(), "file")
            .unwrap()
            .unwrap(),
        "SELECTED\n"
    );
    crate::git::discard_file(selected.to_str().unwrap(), "file").unwrap();
    assert_eq!(
        fs::read_to_string(selected.join("file")).unwrap(),
        "SELECTED\n"
    );
    assert_eq!(
        fs::read_to_string(other.join("file")).unwrap(),
        "foreign must survive\n"
    );
    assert!(crate::git::is_repository_root(&selected));
    assert_eq!(
        crate::git::history::load(selected.to_str().unwrap(), 10)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn desktop_subdirectory_discovery_retains_real_ancestor_root_and_linked_checkout_identity() {
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("selected");
    repo(&selected, "SELECTED\n");
    let sub = selected.join("nested/folder");
    fs::create_dir_all(&sub).unwrap();
    let output = worktree(sub.to_str().unwrap())
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap(),
        fs::canonicalize(&selected).unwrap()
    );
    assert!(!crate::git::is_repository_root(&sub));
    let linked = dir.path().join("linked");
    git(
        &selected,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    let output = worktree(linked.to_str().unwrap())
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap(),
        fs::canonicalize(&linked).unwrap()
    );
}
#[test]
fn object_only_bare_and_clone_helpers_keep_their_intended_modes() {
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("selected");
    repo(&selected, "SELECTED\n");
    let bare = dir.path().join("bare.git");
    fs::create_dir(&bare).unwrap();
    git(&bare, &["init", "--bare", "--template=", "-q"]);
    let output = directory(bare.to_str().unwrap())
        .args(["rev-parse", "--is-bare-repository"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "true");
    let cloned = dir.path().join("cloned");
    git(
        dir.path(),
        &[
            "clone",
            "--template=",
            "-q",
            selected.to_str().unwrap(),
            cloned.to_str().unwrap(),
        ],
    );
    assert!(crate::git::is_repository_root(&cloned));
    let output = directory(selected.to_str().unwrap())
        .arg("--git-dir")
        .arg(selected.join(".git"))
        .args(["--bare", "show", "HEAD:file"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "SELECTED\n");
}
#[test]
fn scrub_preserves_explicit_author_and_normal_account_ssh_environment() {
    let mut command = Command::new("git");
    command
        .env("GIT_AUTHOR_NAME", "Approved author")
        .env("GIT_AUTHOR_EMAIL", "author@example.invalid")
        .env("GIT_COMMITTER_DATE", "2002-01-01T00:00:00Z")
        .env("SSH_AUTH_SOCK", "/fixture/agent.sock")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .env("GH_TOKEN", "account-token")
        .env("GIT_CONFIG_KEY_123", "core.worktree")
        .env("GIT_CONFIG_VALUE_123", "/foreign");
    scrub(&mut command);
    let values: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    for key in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_DATE",
        "SSH_AUTH_SOCK",
        "GIT_SSH_COMMAND",
        "GH_TOKEN",
    ] {
        assert!(values[key].is_some());
    }
    assert_eq!(values["GIT_CONFIG_KEY_123"], None);
    assert_eq!(values["GIT_CONFIG_VALUE_123"], None);
}

#[test]
fn explicit_new_and_repeated_initialization_preserve_the_selected_root_and_existing_identity() {
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("brand-new");
    fs::create_dir(&selected).unwrap();
    fs::write(selected.join("file"), "selected content\n").unwrap();
    assert!(!selected.join(".git").exists());
    #[cfg(unix)]
    let original = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&selected).unwrap();
        (metadata.dev(), metadata.ino())
    };
    crate::git::initialize(selected.to_str().unwrap()).unwrap();
    assert!(crate::git::is_repository_root(&selected));
    let head = fs::read(selected.join(".git/HEAD")).unwrap();
    #[cfg(unix)]
    let objects = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(selected.join(".git/objects")).unwrap();
        (metadata.dev(), metadata.ino())
    };
    crate::git::initialize(selected.to_str().unwrap()).unwrap();
    assert!(crate::git::is_repository_root(&selected));
    assert_eq!(fs::read(selected.join(".git/HEAD")).unwrap(), head);
    assert_eq!(
        fs::read_to_string(selected.join("file")).unwrap(),
        "selected content\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let root = fs::metadata(&selected).unwrap();
        let current = fs::metadata(selected.join(".git/objects")).unwrap();
        assert_eq!((root.dev(), root.ino()), original);
        assert_eq!((current.dev(), current.ino()), objects);
    }
}
