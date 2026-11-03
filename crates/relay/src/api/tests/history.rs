use super::{Fixture, router};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use dunlin_workspace::git::history;
use serde_json::{Value, json};
use std::{fs, process::Command};
use tower::ServiceExt;

impl Fixture {
    fn git_history_fixture(&self) -> (String, String) {
        self.history_git(&["init", "-qb", "main"]);
        fs::write(self.root.join("old name"), "base\n").unwrap();
        self.history_git(&["add", "--all"]);
        self.history_git(&["commit", "-qm", "first"]);
        let first = self.history_git(&["rev-parse", "HEAD"]).trim().into();
        self.history_git(&["mv", "old name", "new name"]);
        fs::write(self.root.join("binary"), [0, 255, 0]).unwrap();
        self.history_git(&["add", "--all"]);
        self.history_git(&["commit", "-qm", "rename and binary"]);
        (
            first,
            self.history_git(&["rev-parse", "HEAD"]).trim().into(),
        )
    }
    fn history_git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args([
                "-c",
                "user.name=Relay Test",
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
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    async fn history_get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::get(path);
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())),
        )
    }
}

#[tokio::test]
async fn history_branch_creation_is_scoped_authenticated_and_never_moves_head() {
    let f = Fixture::new();
    let (first, _) = f.git_history_fixture();
    let head = f.history_git(&["rev-parse", "HEAD"]).trim().to_string();
    let status = f.history_git(&["status", "--porcelain"]);
    let (_, page) = f
        .history_get(
            &format!("/v1/git/history?project={}", f.project),
            Some(&f.token),
        )
        .await;
    let scope = page["scope"].as_str().unwrap().to_string();
    let body =
        json!({"project": f.project, "scope": scope, "oid": first, "name": "feature/from-history"});
    assert_eq!(
        f.post("/v1/git/history/branch", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.post(
            "/v1/git/history/branch",
            Some(&f.ungranted_token),
            body.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.post("/v1/git/history/branch", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.history_git(&["rev-parse", "feature/from-history"]).trim(),
        first
    );
    assert_eq!(f.history_git(&["rev-parse", "HEAD"]).trim(), head);
    assert_eq!(f.history_git(&["status", "--porcelain"]), status);
    assert_eq!(
        f.post("/v1/git/history/branch", Some(&f.token), body)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let wrong_scope =
        json!({"project": f.project, "scope": "0".repeat(64), "oid": first, "name": "other"});
    assert_eq!(
        f.post("/v1/git/history/branch", Some(&f.token), wrong_scope)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let bad_name =
        json!({"project": f.project, "scope": "0".repeat(64), "oid": first, "name": "-bad"});
    assert_eq!(
        f.post("/v1/git/history/branch", Some(&f.token), bad_name)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn history_routes_require_authentication_and_project_grants_before_reading_git() {
    let f = Fixture::new();
    for endpoint in [
        "/v1/git/history",
        "/v1/git/history/commit",
        "/v1/git/history/patch",
        "/v1/git/overview",
    ] {
        let path = format!(
            "{endpoint}?project={}&scope={}&oid={}&parent=0&file=0",
            f.project,
            "0".repeat(64),
            "0".repeat(40)
        );
        // Valid per-route query shapes; unknown fields are rejected separately.
        let path = if endpoint.ends_with("/commit") {
            path.replace("&file=0", "")
        } else if endpoint.ends_with("/history") || endpoint.ends_with("/overview") {
            format!("{endpoint}?project={}", f.project)
        } else {
            path
        };
        assert_eq!(f.history_get(&path, None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            f.history_get(&path, Some("bad-token")).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.history_get(&path, Some(&f.ungranted_token)).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(!f.root.join(".git").exists());
    assert_eq!(
        f.history_get("/v1/git/history?project=..%2Foutside", Some(&f.token))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn overview_matches_local_stashes_worktrees_and_conflicts_without_mutation() {
    use dunlin_workspace::git::overview;
    let f = Fixture::new();
    f.git_history_fixture();
    fs::write(f.root.join("new name"), "stash\n").unwrap();
    f.history_git(&["stash", "push", "-qm", "saved for later"]);
    let linked = f.root.join("linked");
    f.history_git(&["worktree", "add", "--detach", linked.to_str().unwrap()]);
    f.history_git(&["checkout", "-qb", "side"]);
    fs::write(f.root.join("new name"), "side\n").unwrap();
    f.history_git(&["commit", "-am", "side"]);
    f.history_git(&["checkout", "main"]);
    fs::write(f.root.join("new name"), "main\n").unwrap();
    f.history_git(&["commit", "-am", "main"]);
    let merge = Command::new("git")
        .arg("-C")
        .arg(&f.root)
        .args(["-c", "rerere.enabled=false", "merge", "side"])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let head = fs::read(f.root.join(".git/HEAD")).unwrap();
    let reflog = fs::read(f.root.join(".git/logs/refs/stash")).unwrap();
    let file = fs::read(f.root.join("new name")).unwrap();
    let (_, graph) = f
        .history_get(
            &format!("/v1/git/history?project={}", f.project),
            Some(&f.token),
        )
        .await;
    let path = format!(
        "/v1/git/overview?project={}&scope={}",
        f.project,
        graph["scope"].as_str().unwrap()
    );
    let (status, response) = f.history_get(&path, Some(&f.token)).await;
    assert_eq!(status, StatusCode::OK);
    let local = overview::load(f.root.to_str().unwrap()).unwrap();
    assert_eq!(response["value"], serde_json::to_value(&local).unwrap());
    assert_eq!(response["scope"], graph["scope"]);
    assert_eq!(local.stashes.len(), 1);
    assert_eq!(local.worktrees.len(), 2);
    assert_eq!(local.operations, vec![overview::Operation::Merge]);
    assert_eq!(local.conflicts[0].stages, vec![1, 2, 3]);
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.root.join(".git/HEAD")).unwrap(), head);
    assert_eq!(
        fs::read(f.root.join(".git/logs/refs/stash")).unwrap(),
        reflog
    );
    assert_eq!(fs::read(f.root.join("new name")).unwrap(), file);
}

#[tokio::test]
async fn overview_supports_unborn_repositories_and_rejects_scope_and_path_overrides() {
    let f = Fixture::new();
    f.history_git(&["init", "-qb", "main"]);
    let path = format!("/v1/git/overview?project={}", f.project);
    let (status, response) = f.history_get(&path, Some(&f.token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["value"]["stashes"], json!([]));
    assert_eq!(response["value"]["worktrees"][0]["current"], true);
    for extra in ["path=%2Foutside", "root=%2Foutside", "limit=1", "scope=bad"] {
        assert_eq!(
            f.history_get(&format!("{path}&{extra}"), Some(&f.token))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.history_get(&format!("{path}&scope={}", "0".repeat(64)), Some(&f.token))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let held = f
        .state
        .history_reads
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    assert_eq!(
        f.history_get(&path, Some(&f.token)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(held);
}

#[tokio::test]
async fn remote_history_matches_local_root_rename_binary_and_preserves_dirty_state() {
    let f = Fixture::new();
    let (first, latest) = f.git_history_fixture();
    fs::write(f.root.join("new name"), "staged\n").unwrap();
    f.history_git(&["add", "new name"]);
    fs::write(f.root.join("new name"), "unstaged\n").unwrap();
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let head = fs::read(f.root.join(".git/HEAD")).unwrap();
    let refs = f.history_git(&["show-ref"]);
    let (status, page) = f
        .history_get(
            &format!("/v1/git/history?project={}&limit=2", f.project),
            Some(&f.token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["version"], history::PROTOCOL_VERSION);
    assert_eq!(
        page["value"],
        serde_json::to_value(history::load(f.root.to_str().unwrap(), 2).unwrap()).unwrap()
    );
    let scope = page["scope"].as_str().unwrap();
    assert_eq!(scope.len(), 64);
    assert!(!page.to_string().contains(f.root.to_str().unwrap()));
    for oid in [first, latest] {
        let local = history::details(f.root.to_str().unwrap(), &oid, 0).unwrap();
        let path = format!(
            "/v1/git/history/commit?project={}&scope={scope}&oid={oid}&parent=0",
            f.project
        );
        let (status, detail) = f.history_get(&path, Some(&f.token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["value"], serde_json::to_value(&local).unwrap());
        for (i, file) in local.files.iter().enumerate() {
            let path = format!(
                "/v1/git/history/patch?project={}&scope={scope}&oid={oid}&parent=0&file={i}",
                f.project
            );
            let (status, patch) = f.history_get(&path, Some(&f.token)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(patch["value"]["file"], serde_json::to_value(file).unwrap());
            assert_eq!(
                patch["value"]["patch"],
                history::file_patch(f.root.to_str().unwrap(), &oid, 0, file).unwrap()
            );
        }
    }
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.root.join(".git/HEAD")).unwrap(), head);
    assert_eq!(f.history_git(&["show-ref"]), refs);
    assert_eq!(
        fs::read_to_string(f.root.join("new name")).unwrap(),
        "unstaged\n"
    );
}

#[tokio::test]
async fn malformed_scope_paths_limits_and_file_indices_never_select_another_root() {
    let f = Fixture::new();
    let (_, oid) = f.git_history_fixture();
    let (_, page) = f
        .history_get(
            &format!("/v1/git/history?project={}", f.project),
            Some(&f.token),
        )
        .await;
    let scope = page["scope"].as_str().unwrap();
    for query in [
        "limit=0",
        "limit=2001",
        "root=%2Foutside",
        "revision=--all",
        "revision=HEAD%00",
    ] {
        assert_eq!(
            f.history_get(
                &format!("/v1/git/history?project={}&{query}", f.project),
                Some(&f.token)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let base = format!(
        "/v1/git/history/patch?project={}&scope={scope}&oid={oid}&parent=0",
        f.project
    );
    for extra in [
        "file=2000",
        "file=999",
        "file=0&path=..%2Foutside",
        "file=-1",
        "file=0&root=%2Foutside",
    ] {
        assert_eq!(
            f.history_get(&format!("{base}&{extra}"), Some(&f.token))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.history_get(
            &format!(
                "/v1/git/history/commit?project={}&scope={scope}&oid=HEAD",
                f.project
            ),
            Some(&f.token)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.history_get(
            &format!(
                "/v1/git/history?project={}&scope={}",
                f.project,
                "0".repeat(64)
            ),
            Some(&f.token)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (user, _) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let other = f
        .state
        .store
        .lock()
        .unwrap()
        .create_project(
            "same-root-another-grant",
            &format!("{}/.", f.root.display()),
        )
        .unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .grant(&other.id, &user.id)
        .unwrap();
    assert_eq!(
        f.history_get(
            &format!("/v1/git/history?project={}&scope={scope}", other.id),
            Some(&f.token)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn saturation_is_bounded_and_revoked_devices_cannot_receive_inflight_results() {
    let f = Fixture::new();
    f.git_history_fixture();
    let held = f
        .state
        .history_reads
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    assert_eq!(
        f.history_get(
            &format!("/v1/git/history?project={}", f.project),
            Some(&f.token)
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(held);
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let project = f.state.project_for_user(&user.id, &f.project).unwrap();
    let state = f.state.clone();
    let device_id = device.id.clone();
    let result = crate::api::history::read(
        f.state.clone(),
        user.id,
        device.id,
        project,
        None,
        move |_| {
            state
                .store
                .lock()
                .unwrap()
                .revoke_device(&device_id)
                .unwrap();
            Ok(json!("must not escape"))
        },
    )
    .await;
    assert_eq!(result.unwrap_err().status, StatusCode::UNAUTHORIZED);
}

#[cfg(unix)]
#[tokio::test]
async fn retargeted_project_alias_rejects_the_original_history_scope() {
    let f = Fixture::new();
    let other = Fixture::new();
    f.git_history_fixture();
    other.git_history_fixture();
    let alias = f.root.join("alias");
    std::os::unix::fs::symlink(&f.root, &alias).unwrap();
    let (user, _) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let project = f
        .state
        .store
        .lock()
        .unwrap()
        .create_project("alias", alias.to_str().unwrap())
        .unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .grant(&project.id, &user.id)
        .unwrap();
    let (_, first) = f
        .history_get(
            &format!("/v1/git/history?project={}", project.id),
            Some(&f.token),
        )
        .await;
    let scope = first["scope"].as_str().unwrap();
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&other.root, &alias).unwrap();
    assert_eq!(
        f.history_get(
            &format!("/v1/git/history?project={}&scope={scope}", project.id),
            Some(&f.token)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn cancelled_http_work_keeps_its_slot_until_the_blocking_read_finishes() {
    let f = Fixture::new();
    f.git_history_fixture();
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let project = f.state.project_for_user(&user.id, &f.project).unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let state = f.state.clone();
    let task = tokio::spawn(crate::api::history::read(
        state,
        user.id,
        device.id,
        project,
        None,
        move |_| {
            let _ = entered_tx.send(());
            release_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|_| "test worker was not released")?;
            Ok(())
        },
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(f.state.history_reads.available_permits(), 3);
    release_tx.send(()).unwrap();
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        f.state.history_reads.clone().acquire_many_owned(4),
    )
    .await
    .unwrap()
    .unwrap();
    drop(permit);
}

#[tokio::test]
async fn relay_merge_parent_selection_matches_each_local_comparison() {
    let f = Fixture::new();
    f.git_history_fixture();
    f.history_git(&["checkout", "-qb", "side"]);
    fs::write(f.root.join("side"), "side\n").unwrap();
    f.history_git(&["add", "side"]);
    f.history_git(&["commit", "-qm", "side"]);
    f.history_git(&["checkout", "main"]);
    fs::write(f.root.join("main"), "main\n").unwrap();
    f.history_git(&["add", "main"]);
    f.history_git(&["commit", "-qm", "main"]);
    f.history_git(&["merge", "--no-ff", "side", "-m", "merge"]);
    let oid = f.history_git(&["rev-parse", "HEAD"]).trim().to_string();
    let (_, page) = f
        .history_get(
            &format!("/v1/git/history?project={}", f.project),
            Some(&f.token),
        )
        .await;
    let scope = page["scope"].as_str().unwrap();
    for parent in [0, 1] {
        let path = format!(
            "/v1/git/history/commit?project={}&scope={scope}&oid={oid}&parent={parent}",
            f.project
        );
        let (status, response) = f.history_get(&path, Some(&f.token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response["value"],
            serde_json::to_value(history::details(f.root.to_str().unwrap(), &oid, parent).unwrap())
                .unwrap()
        );
        let path = format!(
            "/v1/git/history/patch?project={}&scope={scope}&oid={oid}&parent={parent}&file=0",
            f.project
        );
        let (status, patch) = f.history_get(&path, Some(&f.token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(patch["value"]["parent"], parent);
        assert_eq!(
            patch["value"]["file"]["path"],
            if parent == 0 { "side" } else { "main" }
        );
    }
}
