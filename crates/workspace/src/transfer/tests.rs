use super::*;

struct Fixture {
    base: PathBuf,
    project: PathBuf,
    external: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("dunlin-transfer-{}", uuid::Uuid::now_v7()));
        let project = base.join("project");
        let external = base.join("external");
        fs::create_dir_all(project.join("target")).unwrap();
        fs::create_dir_all(&external).unwrap();
        Self {
            base,
            project,
            external,
        }
    }
    fn root(&self) -> &str {
        self.project.to_str().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn creates_folders_without_replacing_files_or_escaping_the_project() {
    let f = Fixture::new();
    assert!(create_directory(f.root(), "new").unwrap().is_dir);
    assert!(create_directory(f.root(), "new").is_err());
    assert!(create_directory(f.root(), "../outside").is_err());
    assert!(create_directory(f.root(), ".").is_err());
    assert!(create_directory(f.root(), ".git").is_err());
    fs::write(f.project.join("existing"), b"keep").unwrap();
    assert!(create_directory(f.root(), "existing").is_err());
    assert_eq!(fs::read(f.project.join("existing")).unwrap(), b"keep");
}

#[test]
fn moves_project_files_and_folders_without_copying_or_deleting_contents() {
    let f = Fixture::new();
    fs::write(f.project.join("file.txt"), b"content").unwrap();
    fs::create_dir_all(f.project.join("folder/sub")).unwrap();
    fs::write(f.project.join("folder/sub/code.rs"), b"fn main() {}").unwrap();
    let report = transfer(
        f.root(),
        &[f.project.join("file.txt"), f.project.join("folder")],
        "target",
    );
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(report.completed.len(), 2);
    assert!(report.completed.iter().all(|entry| entry.moved));
    assert!(!f.project.join("file.txt").exists());
    assert!(!f.project.join("folder").exists());
    assert_eq!(
        fs::read(f.project.join("target/file.txt")).unwrap(),
        b"content"
    );
    assert!(f.project.join("target/folder/sub/code.rs").is_file());
    let unchanged = transfer(f.root(), &[f.project.join("target/file.txt")], "target");
    assert!(unchanged.error.is_none());
    assert!(unchanged.completed.is_empty());
}

#[test]
fn imports_binary_files_and_folder_trees_while_keeping_external_originals() {
    let f = Fixture::new();
    fs::write(f.external.join("zażółć 'file'.bin"), [0, 1, 255]).unwrap();
    fs::create_dir_all(f.external.join("assets/empty")).unwrap();
    fs::write(f.external.join("assets/image.bin"), [0, 2, 3]).unwrap();
    let report = transfer(
        f.root(),
        &[
            f.external.join("zażółć 'file'.bin"),
            f.external.join("assets"),
        ],
        "target",
    );
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(report.completed.len(), 2);
    assert!(report.completed.iter().all(|entry| !entry.moved));
    assert_eq!(
        fs::read(f.project.join("target/zażółć 'file'.bin")).unwrap(),
        [0, 1, 255]
    );
    assert!(f.project.join("target/assets/empty").is_dir());
    assert!(f.external.join("assets/image.bin").is_file());
    assert!(f.external.join("zażółć 'file'.bin").is_file());
    assert!(
        fs::read_dir(f.project.join("target"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".dunlin-import-"))
    );
}

#[test]
fn preflights_the_entire_batch_before_moving_any_item() {
    let f = Fixture::new();
    fs::write(f.project.join("first"), b"first").unwrap();
    fs::write(f.project.join("second"), b"second").unwrap();
    fs::write(f.project.join("target/second"), b"existing").unwrap();
    let report = transfer(
        f.root(),
        &[f.project.join("first"), f.project.join("second")],
        "target",
    );
    assert!(report.error.is_some());
    assert!(report.completed.is_empty());
    assert!(f.project.join("first").is_file());
    assert!(!f.project.join("target/first").exists());
    assert_eq!(
        fs::read(f.project.join("target/second")).unwrap(),
        b"existing"
    );
    fs::write(f.external.join("first"), b"duplicate").unwrap();
    let duplicate = transfer(
        f.root(),
        &[f.project.join("first"), f.external.join("first")],
        "target",
    );
    assert!(duplicate.error.is_some());
    assert!(duplicate.completed.is_empty());
}

#[test]
fn atomic_moves_reject_a_target_created_after_preflight() {
    let f = Fixture::new();
    let source = f.project.join("file");
    let destination = f.project.join("target/file");
    fs::write(&source, b"source").unwrap();
    assert_eq!(
        plan(f.root(), std::slice::from_ref(&source), "target")
            .unwrap()
            .len(),
        1
    );
    fs::write(&destination, b"racing writer").unwrap();
    assert!(rename_new(&source, &destination).is_err());
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(destination).unwrap(), b"racing writer");
}

#[test]
fn rejects_cycles_project_root_git_metadata_and_oversized_imports() {
    let f = Fixture::new();
    fs::create_dir_all(f.project.join("folder/sub")).unwrap();
    for (source, target) in [
        (f.project.clone(), "target"),
        (f.project.join("folder"), "folder/sub"),
        (f.base.clone(), "target"),
    ] {
        let report = transfer(f.root(), &[source], target);
        assert!(report.error.is_some());
        assert!(report.completed.is_empty());
    }
    fs::create_dir(f.project.join(".git")).unwrap();
    assert!(
        transfer(f.root(), &[f.project.join(".git")], "target")
            .error
            .is_some()
    );
    assert!(
        transfer(f.root(), &[f.project.join("folder")], ".git")
            .error
            .is_some()
    );
    assert!(crate::fs::create_file(f.root(), ".git/entry").is_err());
    let huge = f.external.join("huge.bin");
    fs::File::create(&huge)
        .unwrap()
        .set_len(MAX_IMPORT_BYTES + 1)
        .unwrap();
    assert!(
        transfer(f.root(), std::slice::from_ref(&huge), "target")
            .error
            .is_some()
    );
    assert!(huge.is_file());
    assert!(!f.project.join("target/huge.bin").exists());
}

#[cfg(unix)]
#[test]
fn rejects_symbolic_links_escaping_targets_and_dangling_collisions() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::write(f.external.join("secret"), b"outside").unwrap();
    symlink(&f.external, f.project.join("escape")).unwrap();
    assert!(
        transfer(f.root(), &[f.external.join("secret")], "escape")
            .error
            .is_some()
    );
    assert!(
        transfer(f.root(), &[f.project.join("escape")], "target")
            .error
            .is_some()
    );
    fs::create_dir(f.external.join("bundle")).unwrap();
    symlink(f.external.join("secret"), f.external.join("bundle/link")).unwrap();
    assert!(
        transfer(f.root(), &[f.external.join("bundle")], "target")
            .error
            .is_some()
    );
    assert!(!f.project.join("target/bundle").exists());
    symlink(f.external.join("missing"), f.project.join("target/secret")).unwrap();
    assert!(
        transfer(f.root(), &[f.external.join("secret")], "target")
            .error
            .is_some()
    );
    assert!(
        fs::symlink_metadata(f.project.join("target/secret"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::create_dir(f.project.join(".git")).unwrap();
    symlink(f.project.join(".git"), f.project.join("alias")).unwrap();
    assert!(create_directory(f.root(), "alias/unsafe").is_err());
    assert!(crate::fs::create_file(f.root(), "alias/unsafe").is_err());
}

#[test]
fn import_limits_cover_the_whole_selection_and_nested_folders() {
    let f = Fixture::new();
    let first = f.external.join("first");
    let second = f.external.join("second");
    for path in [&first, &second] {
        fs::File::create(path)
            .unwrap()
            .set_len(MAX_IMPORT_BYTES / 2 + 1)
            .unwrap();
    }
    let report = transfer(f.root(), &[first, second], "target");
    assert!(report.error.unwrap().contains("1 GiB"));
    assert!(report.completed.is_empty());
    assert!(!f.project.join("target/first").exists());
    let nested = f.external.join("nested");
    let mut deepest = nested.clone();
    for _ in 0..65 {
        deepest.push("x");
    }
    fs::create_dir_all(deepest).unwrap();
    assert!(
        transfer(f.root(), &[nested], "target")
            .error
            .unwrap()
            .contains("64 levels")
    );
}

#[test]
fn staged_import_collisions_leave_the_original_and_destination_untouched() {
    let f = Fixture::new();
    let source = f.external.join("file");
    let destination = f.project.join("target/file");
    fs::write(&source, b"external original").unwrap();
    let plans = plan(f.root(), std::slice::from_ref(&source), "target").unwrap();
    fs::write(&destination, b"racing writer").unwrap();
    assert!(copy_new(&plans[0].source, &plans[0].destination, &mut 0, &mut 0).is_err());
    assert_eq!(fs::read(source).unwrap(), b"external original");
    assert_eq!(fs::read(destination).unwrap(), b"racing writer");
    assert_eq!(fs::read_dir(f.project.join("target")).unwrap().count(), 1);
}

#[test]
fn rename_and_delete_are_scoped_exclusive_and_non_recursive() {
    let f = Fixture::new();
    fs::write(f.project.join("file.txt"), b"original").unwrap();
    fs::write(f.project.join("taken.txt"), b"existing").unwrap();
    assert!(rename_entry(f.root(), "file.txt", "taken.txt").is_err());
    assert_eq!(fs::read(f.project.join("taken.txt")).unwrap(), b"existing");
    for name in ["../escape", ".git", "", "/outside"] {
        assert!(rename_entry(f.root(), "file.txt", name).is_err());
    }
    let renamed = rename_entry(f.root(), "file.txt", "renamed.txt").unwrap();
    assert!(renamed.moved);
    assert!(!f.project.join("file.txt").exists());
    assert_eq!(
        fs::read(f.project.join("renamed.txt")).unwrap(),
        b"original"
    );
    assert!(delete_entry(f.root(), ".").is_err());
    assert!(delete_entry(f.root(), "../external").is_err());
    fs::create_dir(f.project.join(".git")).unwrap();
    assert!(delete_entry(f.root(), ".git").is_err());
    fs::create_dir(f.project.join("folder")).unwrap();
    fs::write(f.project.join("folder/keep"), b"keep").unwrap();
    assert!(delete_entry(f.root(), "folder").is_err());
    assert!(f.project.join("folder/keep").is_file());
    delete_entry(f.root(), "renamed.txt").unwrap();
    assert!(!f.project.join("renamed.txt").exists());
}

#[cfg(unix)]
#[test]
fn transfer_reports_preserve_the_configured_project_root_alias() {
    let f = Fixture::new();
    let alias = f.base.join("alias");
    std::os::unix::fs::symlink(&f.project, &alias).unwrap();
    fs::write(f.project.join("file"), b"keep").unwrap();
    let report = transfer(alias.to_str().unwrap(), &[alias.join("file")], "target");
    assert!(report.error.is_none());
    assert_eq!(Path::new(&report.completed[0].source), alias.join("file"));
    assert_eq!(
        Path::new(&report.completed[0].destination),
        alias.join("target/file")
    );
}
