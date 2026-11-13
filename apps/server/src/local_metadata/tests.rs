use super::*;
fn args(value: &[&str]) -> Vec<String> {
    value.iter().map(|v| (*v).into()).collect()
}
#[test]
fn parser_rejects_implicit_or_extra_authority_and_requires_explicit_write_ack() {
    let base = [
        "show",
        "--db",
        "/fixture/relay.db",
        "--project-id",
        "project",
    ];
    parse(&args(&base)).unwrap();
    for extra in [
        vec!["--user-id", "user"],
        vec!["--db", "/other.db"],
        vec!["extra"],
        vec!["--unknown", "value"],
    ] {
        let mut value = args(&base);
        value.extend(args(&extra));
        assert!(parse(&value).is_err());
    }
    assert!(parse(&args(&["show", "--project-id", "project"])).is_err());
    assert!(
        parse(&args(&[
            "show",
            "--db",
            "relative.db",
            "--project-id",
            "project"
        ]))
        .is_err()
    );
    let grant = [
        "grant",
        "--db",
        "/fixture/relay.db",
        "--project-id",
        "project",
        "--user-id",
        "user",
        "--expected-revision",
        "none",
        "--expected-approval-revision",
        "1",
        "--expected-native-grant-revision",
        "1",
        "--permissions",
        "read,write",
    ];
    assert!(parse(&args(&grant)).is_err());
    let mut acknowledged = args(&grant);
    acknowledged.push("--ack-repository-wide-metadata-write".into());
    parse(&acknowledged).unwrap();
    acknowledged.push("--ack-repository-wide-metadata-write".into());
    assert!(parse(&acknowledged).is_err());
    for invalid in ["0", "-1", "+1", "1.5", "9007199254740992"] {
        assert!(revision(invalid).is_err());
    }
}
#[cfg(unix)]
struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    git: PathBuf,
    common: PathBuf,
    store: Store,
    project: String,
    user: String,
}
#[cfg(unix)]
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        let common = dir.path().join("repo.git");
        let git = common.join("worktrees/checkout");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&git).unwrap();
        std::fs::create_dir(common.join("objects")).unwrap();
        std::fs::write(root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
        std::fs::write(git.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            git.join("gitdir"),
            format!("{}\n", root.join(".git").display()),
        )
        .unwrap();
        for path in [git.join("HEAD"), common.join("HEAD")] {
            std::fs::write(path, "ref: refs/heads/main\n").unwrap();
        }
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("metadata-cli-fixture", "member").unwrap();
        let project = store
            .create_project("Fixture", root.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        Self {
            dir,
            root,
            git,
            common,
            store,
            project: project.id,
            user: user.id,
        }
    }
    fn command(&self, action: &str, extra: &[&str]) -> Command {
        let mut flags = vec![
            action.to_owned(),
            "--db".into(),
            "/fixture/relay.db".into(),
            "--project-id".into(),
            self.project.clone(),
        ];
        flags.extend(args(extra));
        parse(&flags).unwrap()
    }
    fn approve(&self) -> Value {
        execute(
            &self.store,
            &self.command(
                "approve",
                &[
                    "--git-dir",
                    self.git.to_str().unwrap(),
                    "--common-dir",
                    self.common.to_str().unwrap(),
                    "--expected-revision",
                    "none",
                ],
            ),
        )
        .unwrap()
    }
}
#[cfg(unix)]
#[test]
fn actual_parser_and_command_executor_pin_approval_native_epoch_and_explicit_permissions() {
    let f = Fixture::new();
    let approved = f.approve();
    assert_eq!(approved["revision"], 1);
    assert!(
        f.store
            .local_metadata_grants(&f.project)
            .unwrap()
            .is_empty(),
        "Approval must not implicitly grant anyone"
    );
    let native = f
        .store
        .capture_native_grant(&f.project, &f.user)
        .unwrap()
        .revision
        .to_string();
    let grant = f.command(
        "grant",
        &[
            "--user-id",
            &f.user,
            "--expected-revision",
            "none",
            "--expected-approval-revision",
            "1",
            "--expected-native-grant-revision",
            &native,
            "--permissions",
            "read,write",
            "--ack-repository-wide-metadata-write",
        ],
    );
    let granted = execute(&f.store, &grant).unwrap();
    assert_eq!(granted["permissions"], json!(["read", "write"]));
    assert_eq!(granted["revision"], 1);
    let checked = execute(
        &f.store,
        &f.command("check", &["--user-id", &f.user, "--permission", "write"]),
    )
    .unwrap();
    assert_eq!(checked["execution_ready"], false);
    assert_eq!(checked["observed_state_only"], true);
    assert_eq!(checked["stamp"]["grant"]["user_id"], f.user);
    let serialized = serde_json::to_string(&checked).unwrap();
    for path in [&f.root, &f.git, &f.common] {
        assert!(!serialized.contains(path.to_str().unwrap()));
    }
    assert!(!serialized.contains("extra_root"));
    assert!(
        execute(&f.store, &grant).is_err(),
        "none must not silently refresh an existing grant revision"
    );
}
#[cfg(unix)]
#[test]
fn revoke_and_ungrant_do_not_need_checkout_or_current_native_authority() {
    let f = Fixture::new();
    f.approve();
    let native = f
        .store
        .capture_native_grant(&f.project, &f.user)
        .unwrap()
        .revision
        .to_string();
    execute(
        &f.store,
        &f.command(
            "grant",
            &[
                "--user-id",
                &f.user,
                "--expected-revision",
                "none",
                "--expected-approval-revision",
                "1",
                "--expected-native-grant-revision",
                &native,
                "--permissions",
                "read",
            ],
        ),
    )
    .unwrap();
    f.store.revoke_grant(&f.project, &f.user).unwrap();
    std::fs::rename(&f.root, f.dir.path().join("missing-checkout")).unwrap();
    assert_eq!(
        execute(
            &f.store,
            &f.command(
                "ungrant",
                &["--user-id", &f.user, "--expected-revision", "1"]
            )
        )
        .unwrap()["active"],
        false
    );
    assert_eq!(
        execute(
            &f.store,
            &f.command("revoke", &["--expected-revision", "1"])
        )
        .unwrap()["active"],
        false
    );
    assert!(
        execute(
            &f.store,
            &f.command("check", &["--user-id", &f.user, "--permission", "read"])
        )
        .is_err()
    );
}
#[test]
fn main_command_reader_requires_existing_database_and_never_initializes_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("missing.db");
    let flags = args(&[
        "show",
        "--db",
        db.to_str().unwrap(),
        "--project-id",
        "project",
    ]);
    assert!(run(&flags).is_err());
    assert!(!db.exists());
    let store = Store::open(&db).unwrap();
    let project = store
        .create_project("Fixture", dir.path().to_str().unwrap())
        .unwrap();
    drop(store);
    let output = run(&args(&[
        "show",
        "--db",
        db.to_str().unwrap(),
        "--project-id",
        &project.id,
    ]))
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["approval"], Value::Null);
    assert_eq!(value["grants"], json!([]));
    assert!(!output.contains(dir.path().to_str().unwrap()));
}
