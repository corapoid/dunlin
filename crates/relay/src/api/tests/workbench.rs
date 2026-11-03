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
    let (status, body) = f
        .post(
            "/v1/github/load",
            Some(&f.token),
            json!({"project": f.project, "remote": "origin"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
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
