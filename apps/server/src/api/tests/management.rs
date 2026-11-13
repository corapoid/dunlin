use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

fn administrator() -> Fixture {
    let mut f = Fixture::new();
    let (_, token) = {
        let store = f.state.store.lock().unwrap();
        let user = store.create_user("admin", "admin").unwrap();
        store.create_device(&user.id, "management").unwrap()
    };
    f.token = token;
    let config = Config {
        db: f.root.join("relay.db"),
        harness: "/nonexistent/no-harness".into(),
        ..Config::default()
    };
    f.state.config = std::sync::Arc::new(config);
    f
}
fn manifest(name: &str, contents: &[u8]) -> Value {
    json!({"name":name,"history":false,"branch":null,"files":[{"path":"src/main.txt","size":contents.len(),"sha256":hex::encode(Sha256::digest(contents)),"executable":false}]})
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn legacy_agent_home_changes_only_after_explicit_setup_and_corruption_fails_closed() {
    let f = administrator();
    assert!(!crate::management::isolated_agents(&f.state.config));
    assert_eq!(
        f.put("/v1/server", Some(&f.token), json!({"name":"legacy"}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(!crate::management::isolated_agents(&f.state.config));
    crate::management::setup(&[
        "--db".into(),
        f.state.config.db.to_string_lossy().to_string(),
        "--name".into(),
        "managed".into(),
    ])
    .unwrap();
    assert!(crate::management::isolated_agents(&f.state.config));
    fs::write(
        crate::management::root(&f.state.config).join("settings.json"),
        "broken policy",
    )
    .unwrap();
    assert!(crate::management::isolated_agents(&f.state.config));
    assert!(crate::management::host_accounts(&f.state.config));
}

#[tokio::test]
async fn management_does_not_require_a_harness_and_writes_require_admin() {
    let f = administrator();
    let (status, server) = f.get_json("/v1/server", Some(&f.token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server["harnesses"].as_array().unwrap().len(), 3);
    assert_eq!(
        f.get_json("/v1/server", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.post(
            "/v1/server/actions",
            Some(&f.ungranted_token),
            json!({"action":"install","target":"opencode"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.post(
            "/v1/server/actions",
            Some(&f.token),
            json!({"action":"install","target":"opencode; touch /tmp/pwn"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.put(
            "/v1/server",
            Some(&f.token),
            json!({"name":"studio-linux","harness":"codex"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, server) = f.get_json("/v1/server", Some(&f.token)).await;
    assert_eq!(server["name"], "studio-linux");
    assert_eq!(server["default_harness"], "codex-acp");
}

#[tokio::test]
async fn remote_git_login_refuses_mixed_trust_users_before_starting_a_cli() {
    let f = administrator();
    let (status, body) = f
        .post(
            "/v1/server/actions",
            Some(&f.token),
            json!({"action":"login","target":"github"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("administrator-only")
    );
    assert!(!crate::management::host_accounts(&f.state.config));
}

#[tokio::test]
async fn tool_updates_and_agent_connections_are_mutually_exclusive() {
    let f = administrator();
    let permit = f.state.connection_permit().unwrap();
    assert_eq!(
        f.post(
            "/v1/server/actions",
            Some(&f.token),
            json!({"action":"install","target":"opencode"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    drop(permit);
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let owner = user.id.clone();
    let authority = crate::management::authority::AdminAuthority::capture(
        &f.state,
        &crate::api::AuthUser { user, device },
    )
    .unwrap();
    f.state.management.lock().unwrap().jobs.insert(
        "fixture".into(),
        crate::management::Running {
            owner,
            authority,
            require_admin_only_relay: false,
            created: std::time::Instant::now(),
            job: std::sync::Arc::new(std::sync::Mutex::new(dunlin_workspace::server::Job {
                id: "fixture".into(),
                kind: "install:opencode".into(),
                output: String::new(),
                finished: false,
                success: false,
            })),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
    );
    assert_eq!(
        f.state.connection_permit().unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.state.terminal_permit().unwrap_err().status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn transfer_is_verified_owner_bound_and_never_overwrites() {
    let f = administrator();
    let body = manifest("copied", b"hello");
    assert_eq!(
        f.post("/v1/server/copies", Some(&f.ungranted_token), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, begin) = f
        .post("/v1/server/copies", Some(&f.token), body.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    let id = begin["id"].as_str().unwrap();
    let upload = format!("/v1/server/copies/{id}/chunk");
    let finish = format!("/v1/server/copies/{id}/finish");
    let other_token = {
        let store = f.state.store.lock().unwrap();
        let user = store.create_user("other-admin", "admin").unwrap();
        store.create_device(&user.id, "other-device").unwrap().1
    };
    assert_eq!(
        f.post(
            &upload,
            Some(&other_token),
            json!({"path":"src/main.txt","offset":0,"contents":STANDARD.encode("hello")})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.post(
            &upload,
            Some(&f.token),
            json!({"path":"../outside","offset":0,"contents":""})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.post(
            &upload,
            Some(&f.token),
            json!({"path":"src/main.txt","offset":1,"contents":STANDARD.encode("hello")})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.post(
            &upload,
            Some(&f.token),
            json!({"path":"src/main.txt","offset":0,"contents":STANDARD.encode("hello")})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, project) = f.post(&finish, Some(&f.token), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{project}");
    let destination = PathBuf::from(project["path"].as_str().unwrap());
    assert_eq!(
        fs::read(destination.join("src/main.txt")).unwrap(),
        b"hello"
    );
    assert_eq!(
        f.post("/v1/server/copies", Some(&f.token), body).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.post(&finish, Some(&f.token), json!({})).await.0,
        StatusCode::NOT_FOUND
    );
    assert!(!f.root.join("outside").exists());
}

#[tokio::test]
async fn checksum_failure_never_registers_or_publishes_a_project() {
    let f = administrator();
    let (_, begin) = f
        .post(
            "/v1/server/copies",
            Some(&f.token),
            manifest("bad-copy", b"hello"),
        )
        .await;
    let id = begin["id"].as_str().unwrap();
    f.post(
        &format!("/v1/server/copies/{id}/chunk"),
        Some(&f.token),
        json!({"path":"src/main.txt","offset":0,"contents":STANDARD.encode("wrong")}),
    )
    .await;
    assert_eq!(
        f.post(
            &format!("/v1/server/copies/{id}/finish"),
            Some(&f.token),
            json!({})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(!f.root.join("server/projects/bad-copy").exists());
    assert!(
        !f.root
            .join(format!("server/projects/.transfer-{id}"))
            .exists()
    );
}

#[tokio::test]
async fn history_copy_preserves_branch_and_dirty_files_without_importing_hooks_or_remotes() {
    use dunlin_workspace::project_copy::{self, BUNDLE};
    let f = administrator();
    let source = f.root.join("local");
    fs::create_dir(&source).unwrap();
    project_copy::git(&source, &["init", "-b", "main"]).unwrap();
    fs::write(source.join("tracked"), "committed").unwrap();
    fs::write(source.join("deleted"), "committed").unwrap();
    project_copy::git(&source, &["add", "."]).unwrap();
    project_copy::git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-m",
            "initial",
        ],
    )
    .unwrap();
    project_copy::git(
        &source,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/private.git",
        ],
    )
    .unwrap();
    fs::write(source.join("tracked"), "local edit").unwrap();
    fs::remove_file(source.join("deleted")).unwrap();
    fs::write(source.join("untracked"), "new").unwrap();
    let snapshot =
        project_copy::snapshot(&source, "history".into(), true, false, &[], &f.root).unwrap();
    let (_, begin) = f
        .post(
            "/v1/server/copies",
            Some(&f.token),
            serde_json::to_value(&snapshot.manifest).unwrap(),
        )
        .await;
    let id = begin["id"].as_str().unwrap();
    for entry in &snapshot.manifest.files {
        let bytes = fs::read(if entry.path == BUNDLE {
            snapshot.bundle.clone().unwrap()
        } else {
            source.join(&entry.path)
        })
        .unwrap();
        assert_eq!(
            f.post(
                &format!("/v1/server/copies/{id}/chunk"),
                Some(&f.token),
                json!({"path":entry.path,"offset":0,"contents":STANDARD.encode(bytes)})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    let (status, project) = f
        .post(
            &format!("/v1/server/copies/{id}/finish"),
            Some(&f.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{project}");
    let destination = PathBuf::from(project["path"].as_str().unwrap());
    assert_eq!(
        project_copy::git(&destination, &["branch", "--show-current"])
            .unwrap()
            .trim(),
        "main"
    );
    assert_eq!(
        fs::read_to_string(destination.join("tracked")).unwrap(),
        "local edit"
    );
    assert!(!destination.join("deleted").exists());
    assert_eq!(
        fs::read_to_string(destination.join("untracked")).unwrap(),
        "new"
    );
    assert!(
        project_copy::git(&destination, &["remote"])
            .unwrap()
            .trim()
            .is_empty()
    );
    assert!(!destination.join(BUNDLE).exists());
}
