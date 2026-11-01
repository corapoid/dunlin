use super::*;
use std::{fs, path::PathBuf, process::Command};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(
            std::env::temp_dir().join(format!("dunlin-commit-details-{}", uuid::Uuid::now_v7())),
        );
        fs::create_dir(&repo.0).unwrap();
        repo.git(&["init", "-qb", "main"]);
        repo
    }
    fn root(&self) -> &str {
        self.0.to_str().unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
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
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn write(&self, path: &str, text: impl AsRef<[u8]>) {
        fs::write(self.0.join(path), text).unwrap();
    }
    fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-qm", message]);
        self.git(&["rev-parse", "HEAD"]).trim().into()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn root_commit_compares_to_empty_tree_and_dirty_index_worktree_are_untouched() {
    let repo = Repo::new();
    repo.write("file.txt", "one\ntwo\n");
    let first = repo.commit("First commit\n\nBody preserved");
    repo.write("file.txt", "staged\n");
    repo.git(&["add", "file.txt"]);
    repo.write("file.txt", "unsaved elsewhere\n");
    repo.write("untracked", "untouched");
    let index = fs::read(repo.0.join(".git/index")).unwrap();
    let head = fs::read(repo.0.join(".git/HEAD")).unwrap();
    let refs = repo.git(&["show-ref"]);
    let detail = details(repo.root(), &first, 0).unwrap();
    assert!(detail.parent.is_none());
    assert_eq!(detail.files[0].status, "A");
    assert!(detail.message.contains("Body preserved"));
    let patch = file_patch(repo.root(), &first, 0, &detail.files[0]).unwrap();
    assert!(patch.contains("+one\n+two"));
    assert!(!patch.contains("unsaved elsewhere"));
    assert!(details(repo.root(), &first, 1).is_err());
    assert_eq!(fs::read(repo.0.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(repo.0.join(".git/HEAD")).unwrap(), head);
    assert_eq!(repo.git(&["show-ref"]), refs);
    assert_eq!(
        fs::read_to_string(repo.0.join("file.txt")).unwrap(),
        "unsaved elsewhere\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("untracked")).unwrap(),
        "untouched"
    );
}

#[test]
fn rename_delete_binary_and_literal_unusual_filenames_have_scoped_patches() {
    let repo = Repo::new();
    repo.write("old name.txt", "unchanged rename content\n");
    repo.write("gone.txt", "deleted\n");
    repo.commit("base");
    repo.git(&["mv", "old name.txt", "new name.txt"]);
    repo.git(&["rm", "gone.txt"]);
    repo.write(":(glob)*", "literal path\n");
    repo.write("line\nbreak.txt", "newline name\n");
    repo.write("binary.bin", [0, 255, 0, 4]);
    let oid = repo.commit("changes");
    let detail = details(repo.root(), &oid, 0).unwrap();
    assert_eq!(detail.files.len(), 5);
    let renamed = detail.files.iter().find(|f| f.old_path.is_some()).unwrap();
    assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
    assert_eq!(renamed.path, "new name.txt");
    let patch = file_patch(repo.root(), &oid, 0, renamed).unwrap();
    assert!(patch.contains("rename from old name.txt"));
    let literal = detail.files.iter().find(|f| f.path == ":(glob)*").unwrap();
    let patch = file_patch(repo.root(), &oid, 0, literal).unwrap();
    assert!(patch.contains("+literal path"));
    assert!(!patch.contains("deleted"));
    assert!(!patch.contains("newline name"));
    let binary = detail
        .files
        .iter()
        .find(|f| f.path == "binary.bin")
        .unwrap();
    assert!(
        file_patch(repo.root(), &oid, 0, binary)
            .unwrap()
            .contains("Binary files")
    );
    let deleted = detail.files.iter().find(|f| f.path == "gone.txt").unwrap();
    assert_eq!(deleted.status, "D");
    assert!(
        file_patch(repo.root(), &oid, 0, deleted)
            .unwrap()
            .contains("-deleted")
    );
}

#[test]
fn merges_compare_explicitly_against_each_parent_and_empty_commits_are_empty() {
    let repo = Repo::new();
    repo.write("base", "base\n");
    repo.commit("base");
    repo.git(&["checkout", "-qb", "side"]);
    repo.write("side", "side\n");
    let side = repo.commit("side");
    repo.git(&["checkout", "main"]);
    repo.write("main", "main\n");
    let main = repo.commit("main");
    repo.git(&["merge", "--no-ff", "side", "-m", "merge"]);
    let merge = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    let first = details(repo.root(), &merge, 0).unwrap();
    let second = details(repo.root(), &merge, 1).unwrap();
    assert_eq!(first.parent.as_deref(), Some(main.as_str()));
    assert_eq!(second.parent.as_deref(), Some(side.as_str()));
    assert_eq!(first.files[0].path, "side");
    assert_eq!(second.files[0].path, "main");
    repo.git(&["commit", "--allow-empty", "-qm", "empty"]);
    let empty = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    assert!(details(repo.root(), &empty, 0).unwrap().files.is_empty());
}

#[test]
fn external_drivers_are_not_executed_and_invalid_inputs_do_not_expand_scope() {
    let repo = Repo::new();
    repo.write("file", "one\n");
    repo.write(".gitattributes", "file diff=unsafe\n");
    repo.commit("base");
    repo.write("file", "two\n");
    let oid = repo.commit("change");
    let sentinel = repo.0.join("must-not-exist");
    let driver = format!("touch '{}'", sentinel.display());
    repo.git(&["config", "diff.unsafe.command", &driver]);
    repo.git(&["config", "diff.unsafe.textconv", &driver]);
    let detail = details(repo.root(), &oid, 0).unwrap();
    assert!(
        file_patch(repo.root(), &oid, 0, &detail.files[0])
            .unwrap()
            .contains("+two")
    );
    assert!(!sentinel.exists());
    for revision in ["HEAD", "--all", "../other", "abcdef", &"0".repeat(40)] {
        assert!(details(repo.root(), revision, 0).is_err());
    }
    for path in ["../file", "/file", ""] {
        assert!(
            file_patch(
                repo.root(),
                &oid,
                0,
                &CommitFile {
                    path: path.into(),
                    old_path: None,
                    status: "M".into()
                }
            )
            .is_err()
        );
    }
    fs::create_dir(repo.0.join("nested")).unwrap();
    assert!(details(repo.0.join("nested").to_str().unwrap(), &oid, 0).is_err());
    #[cfg(unix)]
    {
        let other = Repo::new();
        let alias = repo.0.join("alias");
        std::os::unix::fs::symlink(&other.0, &alias).unwrap();
        let pinned = repository_root(alias.to_str().unwrap(), None).unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&repo.0, &alias).unwrap();
        assert!(repository_root(alias.to_str().unwrap(), Some(&pinned)).is_err());
    }
}

#[test]
fn huge_patches_fail_without_hiding_the_commit_file_list() {
    let repo = Repo::new();
    repo.write("large", "x".repeat(MAX_PATCH_BYTES as usize + 1));
    let oid = repo.commit("large");
    let detail = details(repo.root(), &oid, 0).unwrap();
    assert_eq!(detail.files.len(), 1);
    assert!(file_patch(repo.root(), &oid, 0, &detail.files[0]).is_err());
    assert!(parse_files("M\0incomplete").is_err());
    assert!(parse_files("R100\0old\0").is_err());
    let many = (0..=MAX_FILES)
        .map(|index| format!("A\0file{index}\0"))
        .collect::<String>();
    assert!(parse_files(&many).is_err());
    assert!(parse_files("😀\0file\0").is_err());
}

#[cfg(unix)]
#[test]
fn symlink_and_executable_mode_diffs_read_only_git_objects_not_worktree_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let repo = Repo::new();
    let outside = Repo::new();
    outside.write("private", "private content must not be in the patch");
    repo.write("script", "echo example\n");
    symlink("missing", repo.0.join("link")).unwrap();
    repo.commit("base");
    fs::remove_file(repo.0.join("link")).unwrap();
    symlink(outside.0.join("private"), repo.0.join("link")).unwrap();
    fs::set_permissions(repo.0.join("script"), fs::Permissions::from_mode(0o755)).unwrap();
    let oid = repo.commit("modes and link");
    let detail = details(repo.root(), &oid, 0).unwrap();
    assert_eq!(detail.files.len(), 2);
    let link = detail
        .files
        .iter()
        .find(|file| file.path == "link")
        .unwrap();
    let patch = file_patch(repo.root(), &oid, 0, link).unwrap();
    assert!(patch.contains(outside.0.to_str().unwrap()));
    assert!(!patch.contains("private content must not be in the patch"));
    let script = detail
        .files
        .iter()
        .find(|file| file.path == "script")
        .unwrap();
    let patch = file_patch(repo.root(), &oid, 0, script).unwrap();
    assert!(patch.contains("old mode 100644"));
    assert!(patch.contains("new mode 100755"));
}
