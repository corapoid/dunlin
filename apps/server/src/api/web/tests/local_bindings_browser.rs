use super::*;
use crate::store::local_metadata::MetadataPermission;
use dunlin_domain::collaboration::{Permission, WorkspaceRole};
fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "-c",
            "user.name=Local binding fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Fixture Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run --browser-test local_bindings"]
async fn production_browser_acceptance_local_bindings() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let web_root =
        std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&web_root).join("index.html").is_file());
    let dir = tempfile::tempdir().unwrap();
    let ordinary = dir.path().join("ordinary");
    let source = dir.path().join("source");
    let linked = dir.path().join("linked");
    std::fs::create_dir(&ordinary).unwrap();
    std::fs::create_dir(&source).unwrap();
    for root in [&ordinary, &source] {
        git(root, &["init", "-q", "-b", "main"]);
    }
    std::fs::write(source.join("README"), "real linked checkout fixture\n").unwrap();
    git(&source, &["add", "README"]);
    git(&source, &["commit", "-qm", "fixture"]);
    git(
        &source,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    let git_dir = std::path::PathBuf::from(git(&linked, &["rev-parse", "--absolute-git-dir"]));
    let common = source.join(".git").canonicalize().unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let owner = store.create_user("local-binding-owner", "admin").unwrap();
    let viewer = store.create_user("local-binding-viewer", "member").unwrap();
    let workspace = store
        .create_workspace(&owner, "Local binding browser")
        .unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Binding project")
        .unwrap();
    store
        .set_team_member(
            &owner.id,
            &workspace.id,
            &viewer.id,
            WorkspaceRole::Member,
            true,
            None,
        )
        .unwrap();
    store
        .set_team_project_grant(
            &owner.id,
            &workspace.id,
            &project.id,
            &viewer.id,
            [Permission::ViewProject].into_iter().collect(),
            None,
        )
        .unwrap();
    let ordinary = store
        .create_project("Ordinary native", ordinary.to_str().unwrap())
        .unwrap();
    let linked_native = store
        .create_project("Linked native", linked.to_str().unwrap())
        .unwrap();
    for native in [&ordinary, &linked_native] {
        store.grant(&native.id, &owner.id).unwrap();
        store.grant(&native.id, &viewer.id).unwrap();
    }
    let prepared = store
        .prepare_local_metadata(&linked_native.id)
        .unwrap()
        .capture(&git_dir, &common)
        .unwrap();
    store.commit_local_metadata(prepared, None).unwrap();
    let mut grant_revision = store
        .set_local_metadata_grant(
            &linked_native.id,
            &owner.id,
            1,
            1,
            &[MetadataPermission::Read],
            true,
            None,
            false,
        )
        .unwrap()
        .revision;
    let owner_code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let viewer_code = store.create_pairing_code(&viewer.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            web_root: Some(web_root.into()),
            db: dir.path().join("relay.db"),
            ..Config::default()
        },
    );
    let (origin, _server) = super::live_transport::start(&state).await;
    let input = json!({"origin":origin,"ownerCode":owner_code,"viewerCode":viewer_code,"workspace":workspace.id,"project":project.id,"ordinary":ordinary.id,"linked":linked_native.id});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/local-bindings.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Node is required");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(Duration::from_secs(120), async {
        stdin
            .write_all(format!("{input}\n").as_bytes())
            .await
            .unwrap();
        let mut log = String::new();
        while let Some(line) = stdout.next_line().await.unwrap() {
            if !line.starts_with('{') {
                log.push_str(&line);
                log.push('\n');
                continue;
            }
            let command: Value = serde_json::from_str(&line).unwrap();
            let command = command["command"].as_str().unwrap();
            match command {
                "bump_binding" => {
                    let prep = {
                        let store = state.store.lock().unwrap();
                        let binding = store
                            .local_binding_for_user(&owner.id, &workspace.id, &project.id)
                            .unwrap()
                            .unwrap();
                        store
                            .prepare_local_binding(
                                &owner.id,
                                &workspace.id,
                                &project.id,
                                &linked_native.id,
                                Some(binding.revision),
                                Some(1),
                            )
                            .unwrap()
                    };
                    let prepared = prep.validate_filesystem().unwrap();
                    state
                        .store
                        .lock()
                        .unwrap()
                        .commit_local_binding_authorized(prepared, |_, _| {
                            Ok::<_, crate::store::StoreError>(())
                        })
                        .unwrap();
                }
                "reapprove" => {
                    let prep = {
                        let store = state.store.lock().unwrap();
                        store.revoke_local_metadata(&linked_native.id, 1).unwrap();
                        store.prepare_local_metadata(&linked_native.id).unwrap()
                    };
                    let prepared = prep.capture(&git_dir, &common).unwrap();
                    let store = state.store.lock().unwrap();
                    store.commit_local_metadata(prepared, Some(2)).unwrap();
                    grant_revision = store
                        .set_local_metadata_grant(
                            &linked_native.id,
                            &owner.id,
                            3,
                            1,
                            &[MetadataPermission::Read],
                            true,
                            Some(grant_revision),
                            false,
                        )
                        .unwrap()
                        .revision;
                }
                "revoke_read" => {
                    state
                        .store
                        .lock()
                        .unwrap()
                        .revoke_local_metadata_grant(&linked_native.id, &owner.id, grant_revision)
                        .unwrap();
                    grant_revision += 1;
                }
                "restore_read" => {
                    grant_revision = state
                        .store
                        .lock()
                        .unwrap()
                        .set_local_metadata_grant(
                            &linked_native.id,
                            &owner.id,
                            3,
                            1,
                            &[MetadataPermission::Read],
                            true,
                            Some(grant_revision),
                            false,
                        )
                        .unwrap()
                        .revision;
                }
                "revoke_device" => {
                    let store = state.store.lock().unwrap();
                    for device in store.list_devices(&owner.id).unwrap() {
                        store.revoke_device(&device.id).unwrap();
                    }
                }
                _ => panic!("Unknown local binding fixture command {command}"),
            }
            stdin
                .write_all(format!("{}\n", json!({"ok":command})).as_bytes())
                .await
                .unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().await.unwrap();
        assert!(
            output.status.success(),
            "Local binding browser failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("PASS Chromium local bindings"));
        println!("{log}");
    })
    .await
    .expect("Local binding browser deadline");
}
