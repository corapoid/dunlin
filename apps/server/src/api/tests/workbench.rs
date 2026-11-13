use super::{Fixture, router};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{fs, process::Command};
use tower::ServiceExt;

impl Fixture {
    fn git(&self, args: &[&str]) -> String {
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

    fn git_repo(&self) {
        self.git(&["init", "-qb", "main"]);
        fs::write(self.root.join("file.txt"), "base\n").unwrap();
        self.git(&["add", "file.txt"]);
        self.git(&["commit", "-qm", "base"]);
    }

    async fn get_with_token(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
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

    async fn post_large(
        &self,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::post(path).header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes)
                    .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
            },
        )
    }
}

#[tokio::test]
async fn remote_review_capture_changes_and_restore_are_authenticated_and_scoped() {
    let f = Fixture::new();
    f.git_repo();
    assert_eq!(
        f.post(
            "/v1/review/capture",
            None,
            json!({"project": f.project, "label": "before"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.post(
            "/v1/review/capture",
            Some(&f.ungranted_token),
            json!({"project": f.project, "label": "before"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, point) = f
        .post_large(
            "/v1/review/capture",
            Some(&f.token),
            json!({"project": f.project, "label": "before"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{point}");
    assert_eq!(point["label"], "before");
    assert!(point["root"].as_str().is_some());

    fs::write(f.root.join("file.txt"), "after\n").unwrap();
    let (status, changes) = f
        .post_large(
            "/v1/review/changes",
            Some(&f.token),
            json!({"project": f.project, "checkpoint": point}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{changes}");
    assert_eq!(changes.as_array().unwrap().len(), 1);
    assert_eq!(changes[0]["path"], "file.txt");
    let (status, preview) = f
        .post_large(
            "/v1/review/restore/prepare",
            Some(&f.token),
            json!({"project": f.project, "checkpoint": point, "path": "file.txt"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let (status, restored) = f
        .post_large(
            "/v1/review/restore/execute",
            Some(&f.token),
            json!({"project": f.project, "id": preview["id"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "base\n"
    );
    let (status, _) = f
        .post_large(
            "/v1/review/restore/execute",
            Some(&f.token),
            json!({"project": f.project, "id": preview["id"]}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn remote_comparison_prepares_creates_grants_and_exposes_only_worktree_projects() {
    let f = Fixture::new();
    f.git_repo();
    let (status, prepared) = f
        .post_large(
            "/v1/compare/prepare",
            Some(&f.token),
            json!({"project": f.project}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{prepared}");
    let id = prepared["id"].as_str().unwrap().to_string();
    let head = prepared["head"].as_str().unwrap().to_string();
    assert_eq!(prepared["paths"].as_array().unwrap().len(), 2);
    let (status, created) = f
        .post_large(
            "/v1/compare/create",
            Some(&f.token),
            json!({"project": f.project, "id": id}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(created["trees"]["error"].is_null(), "{created}");
    assert_eq!(created["trees"]["paths"].as_array().unwrap().len(), 2);
    assert_eq!(created["projects"].as_array().unwrap().len(), 2);
    assert_eq!(created["git_access"], "requires_operator_approval");
    let canonical_root = f.root.canonicalize().unwrap();
    let member = f
        .state
        .store
        .lock()
        .unwrap()
        .find_user_by_name("member")
        .unwrap()
        .unwrap();
    for (index, project) in created["projects"].as_array().unwrap().iter().enumerate() {
        let project_id = project["id"].as_str().unwrap();
        let path = project["path"].as_str().unwrap();
        assert!(
            std::path::Path::new(path).starts_with(&canonical_root),
            "{path} not under {}",
            canonical_root.display()
        );
        assert!(path.ends_with(if index == 0 { "-a" } else { "-b" }));
        assert!(
            f.state
                .store
                .lock()
                .unwrap()
                .project_for_user(project_id, &member.id)
                .unwrap()
                .is_some()
        );
    }
    let first = &created["projects"][0];
    let baseline = &created["trees"]["baselines"][0];
    fs::write(
        std::path::Path::new(first["path"].as_str().unwrap()).join("file.txt"),
        "comparison edit\n",
    )
    .unwrap();
    // A native grant on a newly derived project does not approve external Git
    // metadata or inherit the source project's operator/user metadata authority.
    let (status, denied) = f
        .post_large(
            "/v1/compare/changes",
            Some(&f.token),
            json!({"project":first["id"],"checkpoint":baseline}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let project_id = first["id"].as_str().unwrap();
    let checkout = first["path"].as_str().unwrap();
    let git_dir = std::process::Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["-C", checkout, "rev-parse", "--absolute-git-dir"])
        .output()
        .unwrap();
    assert!(git_dir.status.success());
    let git_dir = std::path::PathBuf::from(String::from_utf8(git_dir.stdout).unwrap().trim());
    let preparation = f
        .state
        .store
        .lock()
        .unwrap()
        .prepare_local_metadata(project_id)
        .unwrap();
    let prepared = preparation
        .capture(&git_dir, &canonical_root.join(".git"))
        .unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let approval = store.commit_local_metadata(prepared, None).unwrap();
        let native = store.capture_native_grant(project_id, &member.id).unwrap();
        store
            .set_local_metadata_grant(
                project_id,
                &member.id,
                approval.revision,
                native.revision,
                &[crate::store::local_metadata::MetadataPermission::Read],
                true,
                None,
                false,
            )
            .unwrap();
    }
    let (status, changes) = f
        .post_large(
            "/v1/compare/changes",
            Some(&f.token),
            json!({"project": first["id"], "checkpoint": baseline}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{changes}");
    assert_eq!(changes[0]["path"], "file.txt");
    let (status, _) = f
        .post_large(
            "/v1/compare/validate",
            Some(&f.token),
            json!({"project": first["id"], "head": head}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST); // changed tree is not launchable
}

#[tokio::test]
async fn relay_github_and_remote_context_keep_host_credentials_admin_scoped() {
    let f = Fixture::new();
    for endpoint in ["load", "runs", "jobs", "pull", "log", "prepare", "execute"] {
        let (status, _) = f
            .post(
                &format!("/v1/github/{endpoint}"),
                Some(&f.token),
                json!({"project": f.project}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(f.token.as_str()), StatusCode::FORBIDDEN),
        (Some(f.ungranted_token.as_str()), StatusCode::FORBIDDEN),
    ] {
        let (status, body) = f
            .post(
                "/v1/github/metadata",
                token,
                json!({"project": f.project, "remote": "origin"}),
            )
            .await;
        assert_eq!(status, expected, "{body}");
        assert!(!body.to_string().contains("do-not-leak"));
    }
    let mut settings = dunlin_domain::ProjectSettings::default();
    settings
        .environment
        .insert("SECRET_TOKEN".into(), "do-not-leak".into());
    f.state
        .store
        .lock()
        .unwrap()
        .save_project_settings(&f.project, &serde_json::to_string(&settings).unwrap())
        .unwrap();
    let (status, body) = f
        .get_with_token(
            &format!("/v1/context?project={}", f.project),
            Some(&f.token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["project"]["id"], f.project);
    assert_eq!(body["relay"]["harness"], "opencode acp");
    assert!(body["relay"]["sandbox"].is_boolean());
    assert_eq!(body["settings"]["environment"]["SECRET_TOKEN"], "[hidden]");
    assert!(!body.to_string().contains("do-not-leak"));
    assert!(
        body["context_boundary"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );
    assert_eq!(
        f.get_with_token(&format!("/v1/context?project={}", f.project), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn storage_is_project_granted_not_admin_only_and_includes_generated_folders() {
    let f = Fixture::new();
    fs::create_dir(f.root.join("node_modules")).unwrap();
    fs::write(f.root.join("node_modules/generated"), vec![1; 131_072]).unwrap();
    let path = format!("/v1/fs/storage?project={}", f.project);
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(f.ungranted_token.as_str()), StatusCode::FORBIDDEN),
        (Some(f.token.as_str()), StatusCode::OK),
    ] {
        let (status, body) = f.get_with_token(&path, token).await;
        assert_eq!(status, expected, "{body}");
        if status == StatusCode::OK {
            assert!(body.as_u64().unwrap() >= 131_072);
        }
    }
}

fn revoke_workbench_grant(f: &Fixture) {
    let store = f.state.store.lock().unwrap();
    let user = store.authenticate(&f.token).unwrap().unwrap().0;
    store.revoke_grant(&f.project, &user.id).unwrap();
    store.grant(&f.project, &user.id).unwrap();
}
fn other_workbench_device(f: &Fixture) -> String {
    let store = f.state.store.lock().unwrap();
    let user = store.authenticate(&f.token).unwrap().unwrap().0;
    store
        .create_device(&user.id, "other-workbench-device")
        .unwrap()
        .1
}

#[tokio::test]
async fn restore_plan_is_device_bound_foreign_requests_do_not_consume_it_and_aba_never_restores() {
    let f = Fixture::new();
    f.git_repo();
    let (_, point) = f
        .post_large(
            "/v1/review/capture",
            Some(&f.token),
            json!({"project":f.project,"label":"before"}),
        )
        .await;
    fs::write(f.root.join("file.txt"), "owner current content\n").unwrap();
    let (status, plan) = f
        .post_large(
            "/v1/review/restore/prepare",
            Some(&f.token),
            json!({"project":f.project,"checkpoint":point,"path":"file.txt"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    let other = other_workbench_device(&f);
    let body = json!({"project":f.project,"id":plan["id"]});
    for token in [&f.ungranted_token, &other] {
        assert_eq!(
            f.post("/v1/review/restore/execute", Some(token), body.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert!(
            f.state
                .review_plans
                .lock()
                .unwrap()
                .contains_key(plan["id"].as_str().unwrap())
        );
    }
    assert_eq!(
        f.post(
            "/v1/review/restore/execute",
            Some(&f.token),
            json!({"project":"different-project","id":plan["id"]})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    revoke_workbench_grant(&f);
    assert_eq!(
        f.post("/v1/review/restore/execute", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "owner current content\n"
    );
    assert_eq!(
        f.post("/v1/review/restore/execute", Some(&f.token), body)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn comparison_plan_preserves_owner_confirmation_until_consumed_and_never_revives_after_aba() {
    let f = Fixture::new();
    f.git_repo();
    let (status, plan) = f
        .post_large(
            "/v1/compare/prepare",
            Some(&f.token),
            json!({"project":f.project}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    let other = other_workbench_device(&f);
    let body = json!({"project":f.project,"id":plan["id"]});
    for token in [&f.ungranted_token, &other] {
        assert_eq!(
            f.post("/v1/compare/create", Some(token), body.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert!(
            f.state
                .compare_plans
                .lock()
                .unwrap()
                .contains_key(plan["id"].as_str().unwrap())
        );
    }
    revoke_workbench_grant(&f);
    assert_eq!(
        f.post("/v1/compare/create", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for path in plan["paths"].as_array().unwrap() {
        assert!(!std::path::Path::new(path.as_str().unwrap()).exists());
    }
    assert_eq!(
        f.git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert_eq!(
        f.post("/v1/compare/create", Some(&f.token), body).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn comparison_registration_revalidates_source_proof_and_never_grants_derived_paths_after_aba()
{
    let f = Fixture::new();
    f.git_repo();
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let scope = crate::api::native_io::NativeGitIo::capture(
        &f.state,
        &user.id,
        &device.id,
        &f.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await
    .unwrap();
    let path = f.root.join("derived-but-not-granted");
    fs::create_dir(&path).unwrap();
    let trees = dunlin_workspace::comparison::Trees {
        id: "fixture".into(),
        root: f.root.to_string_lossy().into_owned(),
        head: "fixture".into(),
        paths: vec![path.to_string_lossy().into_owned()],
        baselines: vec![],
        error: None,
    };
    let before = f
        .state
        .store
        .lock()
        .unwrap()
        .projects_for_user(&user.id)
        .unwrap()
        .len();
    revoke_workbench_grant(&f);
    let error = crate::api::register_comparison_projects(&f.state, &scope, &trees, "fixture")
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("do not automatically retry"));
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .projects_for_user(&user.id)
            .unwrap()
            .len(),
        before
    );
    assert!(
        path.exists(),
        "An uncertain operation must not delete possible user content"
    );
}

#[tokio::test]
async fn pending_workbench_proofs_do_not_keep_their_appstate_alive() {
    let f = Fixture::new();
    f.git_repo();
    let (_, point) = f
        .post_large(
            "/v1/review/capture",
            Some(&f.token),
            json!({"project":f.project,"label":"before"}),
        )
        .await;
    fs::write(f.root.join("file.txt"), "changed\n").unwrap();
    assert_eq!(
        f.post_large(
            "/v1/review/restore/prepare",
            Some(&f.token),
            json!({"project":f.project,"checkpoint":point,"path":"file.txt"})
        )
        .await
        .0,
        StatusCode::OK
    );
    // Comparison preflight requires a clean source.
    fs::write(f.root.join("file.txt"), "base\n").unwrap();
    assert_eq!(
        f.post_large(
            "/v1/compare/prepare",
            Some(&f.token),
            json!({"project":f.project})
        )
        .await
        .0,
        StatusCode::OK
    );
    let weak = std::sync::Arc::downgrade(&f.state.store);
    drop(f);
    assert!(
        weak.upgrade().is_none(),
        "Registry proofs must not own AppState/Arc cycles"
    );
}

#[cfg(unix)]
fn check_process_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
async fn holding_check(
    f: &Fixture,
    name: &str,
) -> (
    tokio::task::JoinHandle<axum::response::Response>,
    String,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    i32,
) {
    let request=Request::post("/v1/checks/run").header("content-type","application/json").header("authorization",format!("Bearer {}",f.token))
        .body(Body::from(json!({"project":f.project,"argv":["/bin/sh","-c",format!("echo $$ > {name}.pid; printf PRIVATE_CHECK_RESULT; sleep 30; printf forbidden > {name}.survived")]}).to_string())).unwrap();
    let state = f.state.clone();
    let task = tokio::spawn(async move { router(state).oneshot(request).await.unwrap() });
    let marker = f.root.join(format!("{name}.pid"));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fs::read_to_string(&marker)
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .is_none()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (id, cancel) = {
        let checks = f.state.checks.lock().unwrap();
        let (id, entry) = checks.iter().next().unwrap();
        (id.clone(), entry.cancel.clone())
    };
    let pid = fs::read_to_string(marker).unwrap().trim().parse().unwrap();
    (task, id, cancel, pid)
}

#[cfg(unix)]
#[tokio::test]
async fn active_checks_cannot_be_cancelled_by_another_principal_device_or_project() {
    let f = Fixture::new();
    let permits = f.state.history_reads.available_permits();
    let (task, id, cancel, pid) = holding_check(&f, "scoped").await;
    let other_root = f.root.join("another-check-project");
    fs::create_dir(&other_root).unwrap();
    let (peer, other_project) = {
        let store = f.state.store.lock().unwrap();
        let owner = store.authenticate(&f.token).unwrap().unwrap().0;
        let peer = store.create_user("check-peer", "member").unwrap();
        store.grant(&f.project, &peer.id).unwrap();
        let token = store.create_device(&peer.id, "peer").unwrap().1;
        let other = store
            .create_project("another-project", other_root.to_str().unwrap())
            .unwrap();
        assert_ne!(other.id, f.project);
        assert_ne!(other.path.as_str(), f.root.to_str().unwrap());
        store.grant(&other.id, &owner.id).unwrap();
        (token, other.id)
    };
    let other = other_workbench_device(&f);
    for (token, project) in [
        (&peer, &f.project),
        (&other, &f.project),
        (&f.token, &other_project),
    ] {
        assert_eq!(
            f.post(
                "/v1/checks/cancel",
                Some(token),
                json!({"project":project,"id":id})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(!cancel.load(std::sync::atomic::Ordering::Acquire));
    assert!(check_process_alive(pid));
    assert_eq!(f.state.history_reads.available_permits(), permits - 1);
    assert_eq!(
        f.post(
            "/v1/checks/cancel",
            Some(&f.token),
            json!({"project":f.project,"id":id})
        )
        .await
        .0,
        StatusCode::OK
    );
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(report["result"], "cancelled");
    assert_eq!(report["id"], id);
    assert!(!check_process_alive(pid));
    assert!(f.state.checks.lock().unwrap().is_empty());
    assert_eq!(f.state.history_reads.available_permits(), permits);
}

#[cfg(unix)]
#[tokio::test]
async fn checks_guardian_kills_real_process_after_grant_aba_or_device_revocation() {
    for device_revoke in [false, true] {
        let f = Fixture::new();
        let permits = f.state.history_reads.available_permits();
        let (task, _, _, pid) = holding_check(&f, "revoked").await;
        if device_revoke {
            let store = f.state.store.lock().unwrap();
            let device = store.authenticate(&f.token).unwrap().unwrap().1;
            store.revoke_device(&device.id).unwrap();
        } else {
            revoke_workbench_grant(&f);
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
            .unwrap();
        assert!(!body.contains("PRIVATE_CHECK_RESULT"));
        assert!(body.contains("do not automatically retry"));
        assert!(!check_process_alive(pid));
        assert!(!f.root.join("revoked.survived").exists());
        assert!(f.state.checks.lock().unwrap().is_empty());
        assert_eq!(f.state.history_reads.available_permits(), permits);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn dropped_check_request_cancels_process_and_worker_eventually_releases_registry_and_permit()
{
    let f = Fixture::new();
    let permits = f.state.history_reads.available_permits();
    let (task, _, cancel, pid) = holding_check(&f, "dropped").await;
    assert_eq!(f.state.history_reads.available_permits(), permits - 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !f.state.checks.lock().unwrap().is_empty()
            || f.state.history_reads.available_permits() != permits
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!check_process_alive(pid));
    assert!(!f.root.join("dropped.survived").exists());
}
