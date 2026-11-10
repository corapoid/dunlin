use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dunlin-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_version_do_not_require_gui_or_database() {
    assert!(cli(&["--help"]).status.success());
    assert!(cli(&["--version"]).status.success());
}

#[test]
fn invalid_options_fail_without_stdout_or_database_creation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let output = cli(&["project", "list", "--db", path.to_str().unwrap(), "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!path.exists());
    for args in [
        vec!["project", "list"],
        vec!["run", "--local", "--remote", "missing"],
        vec!["run", "--typo"],
        vec!["--help", "--typo"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn catalog_reads_existing_database_without_migration_or_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let store = dunlin_storage::Store::open(&path).unwrap();
    let version = store.schema_version().unwrap();
    drop(store);
    let before = std::fs::read(&path).unwrap();
    for command in ["project", "session"] {
        let output = cli(&[command, "list", "--db", path.to_str().unwrap(), "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!([])
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        dunlin_storage::Store::read_only(&path)
            .unwrap()
            .schema_version()
            .unwrap(),
        version
    );
}
