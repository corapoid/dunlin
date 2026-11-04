use super::{AppState, Config, Store, router};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use tower::ServiceExt;

mod history;
mod operations;
mod terminal;
mod workbench;

struct Fixture {
    root: PathBuf,
    state: AppState,
    project: String,
    token: String,
    ungranted_token: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dunlin-relay-fs-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&root).unwrap();
        let store = Store::open_in_memory().unwrap();
        let user = store.create_user("member", "member").unwrap();
        let outsider = store.create_user("outsider", "member").unwrap();
        let project = store
            .create_project("files", root.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let (_, token) = store.create_device(&user.id, "test").unwrap();
        let (_, ungranted_token) = store.create_device(&outsider.id, "test").unwrap();
        Self {
            root,
            state: AppState::new(store, Config::default()),
            project: project.id,
            token,
            ungranted_token,
        }
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::post(path).header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
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

    async fn put(&self, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::put(path).header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
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

    async fn preview(&self, token: Option<&str>, path: &str) -> (StatusCode, Value) {
        let mut request = Request::get(format!(
            "/v1/fs/preview?project={}&path={path}",
            self.project
        ));
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn get_json(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::get(path);
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())),
        )
    }

    async fn branch_context(&self, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::get(format!("/v1/git/branch-context?project={}", self.project));
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn safe_branch_endpoints_require_grants_and_explicit_consent_without_legacy_fallbacks() {
    let f = Fixture::new();
    assert_eq!(f.branch_context(None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.branch_context(Some(&f.ungranted_token)).await.0,
        StatusCode::FORBIDDEN
    );
    let empty = json!({"root":"", "current":"main", "head":null, "branches":[], "status":"", "staged":0, "unstaged":0, "untracked":0});
    let body =
        json!({"project":f.project, "branch":"feature", "expected":empty, "keep_changes":true});
    assert_eq!(
        f.post("/v1/git/switch-safe", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.post("/v1/git/switch-safe", Some(&f.ungranted_token), body)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert!(!f.root.join(".git").exists());

    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&f.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    fs::write(f.root.join("file.txt"), "base\n").unwrap();
    git(&["add", "file.txt"]);
    git(&[
        "-c",
        "user.name=Relay Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=.git/no-hooks",
        "commit",
        "-qm",
        "fixture",
    ]);
    git(&["branch", "feature"]);
    fs::write(f.root.join("file.txt"), "local work\n").unwrap();
    let (status, context) = f.branch_context(Some(&f.token)).await;
    assert_eq!(status, StatusCode::OK);
    let body =
        json!({"project":f.project, "branch":"feature", "expected":context, "keep_changes":false});
    assert_eq!(
        f.post("/v1/git/switch-safe", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut body = body;
    body["keep_changes"] = json!(true);
    assert_eq!(
        f.post("/v1/git/switch-safe", Some(&f.token), body).await.0,
        StatusCode::OK
    );
    assert_eq!(
        fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "local work\n"
    );
    assert_eq!(
        f.branch_context(Some(&f.token)).await.1["current"],
        "feature"
    );
}

#[tokio::test]
async fn explorer_mutations_require_authentication_and_a_project_grant() {
    let f = Fixture::new();
    for endpoint in [
        "/v1/fs/mkdir",
        "/v1/fs/move",
        "/v1/fs/rename",
        "/v1/fs/delete",
        "/v1/git/init",
    ] {
        let body = json!({"project": f.project, "path": "new", "sources": [], "target": "", "name": "renamed"});
        assert_eq!(
            f.post(endpoint, None, body.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.post(endpoint, Some(&f.ungranted_token), body).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(!f.root.join("new").exists());
    assert!(!f.root.join(".git").exists());
    assert_eq!(
        f.preview(None, "image.png").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.preview(Some(&f.ungranted_token), "image.png").await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn preview_rename_and_delete_preserve_binary_data_scope_and_non_recursive_safety() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let f = Fixture::new();
    let bytes = b"\x89PNG\r\n\x1a\n\0\xffbinary";
    fs::write(f.root.join("image.png"), bytes).unwrap();
    fs::write(f.root.join("taken.png"), b"keep").unwrap();
    let (status, body) = f.preview(Some(&f.token), "image.png").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        STANDARD.decode(body["contents"].as_str().unwrap()).unwrap(),
        bytes
    );
    for path in [
        "../image.png",
        "%2Foutside.png",
        "image.zip",
        "image.avif",
        "image.mp4",
    ] {
        assert_eq!(
            f.preview(Some(&f.token), path).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    for (path, name) in [
        ("image.png", "taken.png"),
        ("image.png", "../outside.png"),
        ("../outside.png", "renamed.png"),
        (".", "root"),
    ] {
        assert_eq!(
            f.post(
                "/v1/fs/rename",
                Some(&f.token),
                json!({"project": f.project, "path": path, "name": name})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(fs::read(f.root.join("taken.png")).unwrap(), b"keep");
    assert_eq!(
        f.post(
            "/v1/fs/rename",
            Some(&f.token),
            json!({"project": f.project, "path": "image.png", "name": "renamed.png"})
        )
        .await
        .0,
        StatusCode::OK
    );
    fs::create_dir(f.root.join("folder")).unwrap();
    fs::write(f.root.join("folder/keep"), b"keep").unwrap();
    for path in ["folder", ".", "../outside"] {
        assert_eq!(
            f.post(
                "/v1/fs/delete",
                Some(&f.token),
                json!({"project": f.project, "path": path})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(f.root.join("folder/keep").exists());
    assert_eq!(
        f.post(
            "/v1/fs/delete",
            Some(&f.token),
            json!({"project": f.project, "path": "renamed.png"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!f.root.join("renamed.png").exists());
}

#[tokio::test]
async fn explorer_routes_create_move_and_initialize_only_inside_the_granted_project() {
    let f = Fixture::new();
    fs::write(f.root.join("file"), b"keep").unwrap();
    let mkdir = json!({"project": f.project, "path": "target"});
    assert_eq!(
        f.post("/v1/fs/mkdir", Some(&f.token), mkdir.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.post("/v1/fs/mkdir", Some(&f.token), mkdir).await.0,
        StatusCode::BAD_REQUEST
    );
    for source in ["../outside", "/outside", "target/../../outside"] {
        let body = json!({"project": f.project, "sources": ["file", source], "target": "target"});
        assert_eq!(
            f.post("/v1/fs/move", Some(&f.token), body).await.0,
            StatusCode::BAD_REQUEST
        );
        assert!(f.root.join("file").is_file());
    }
    let body = json!({"project": f.project, "sources": ["file"], "target": "target"});
    let (status, report) = f.post("/v1/fs/move", Some(&f.token), body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(report["error"].is_null());
    assert_eq!(report["completed"][0]["moved"], true);
    assert_eq!(fs::read(f.root.join("target/file")).unwrap(), b"keep");
    assert!(!f.root.join("file").exists());
    assert!(!f.root.join(".git").exists());
    assert_eq!(
        f.post(
            "/v1/git/init",
            Some(&f.token),
            json!({"project": f.project})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(f.root.join(".git").is_dir());
}

#[tokio::test]
async fn provider_usage_is_admin_only() {
    let f = Fixture::new();
    let request = Request::get("/v1/usage").header("Authorization", format!("Bearer {}", f.token));
    let response = router(f.state.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn billing_evidence_is_device_scoped_and_admin_only_to_aggregate() {
    let f = Fixture::new();
    let summary = dunlin_usage::billing::Summary {
        generated_ms: 1,
        totals: dunlin_usage::billing::Totals {
            events: 2,
            subagent_events: 1,
            ..Default::default()
        },
        by_attribution: vec![dunlin_usage::billing::Slice {
            label: "Subagents".into(),
            totals: dunlin_usage::billing::Totals {
                events: 1,
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let evidence = dunlin_usage::billing::Evidence::new("test-device", summary);
    assert_eq!(
        f.post("/v1/billing/evidence", Some(&f.token), json!(evidence))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.get_json("/v1/billing/evidence", Some(&f.token)).await.0,
        StatusCode::FORBIDDEN
    );
    let (admin, admin_token) = {
        let store = f.state.store.lock().unwrap();
        let admin = store.create_user("billing-admin", "admin").unwrap();
        store.grant(&f.project, &admin.id).unwrap();
        let (_, token) = store.create_device(&admin.id, "admin-test").unwrap();
        (admin, token)
    };
    assert_eq!(admin.name, "billing-admin");
    let (status, value) = f.get_json("/v1/billing/evidence", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value[0]["device_name"], "test");
    assert_eq!(
        value[0]["evidence"]["summary"]["totals"]["subagent_events"],
        1
    );
    let mut invalid = evidence;
    invalid.device.clear();
    assert_eq!(
        f.post("/v1/billing/evidence", Some(&f.token), json!(invalid))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[cfg(unix)]
#[tokio::test]
async fn attachment_upload_rejects_a_symlinked_namespace() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let f = Fixture::new();
    let outside = f.root.with_file_name(format!(
        "dunlin-relay-upload-outside-{}",
        uuid::Uuid::now_v7()
    ));
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.root.join(".dunlin")).unwrap();
    let body = json!({
        "project": f.project,
        "name": "escape.txt",
        "contents": STANDARD.encode(b"must not escape"),
    });
    let (status, _) = f.post("/v1/fs/upload", Some(&f.token), body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let _ = fs::remove_dir_all(&outside);
}
#[tokio::test]
async fn relay_project_settings_are_granted_readable_and_admin_only_writable() {
    let f = Fixture::new();
    let (admin, admin_token) = {
        let store = f.state.store.lock().unwrap();
        let admin = store.create_user("admin", "admin").unwrap();
        store.grant(&f.project, &admin.id).unwrap();
        let (_, token) = store.create_device(&admin.id, "admin-test").unwrap();
        (admin, token)
    };
    let path = format!("/v1/project/settings?project={}", f.project);
    assert_eq!(f.get_json(&path, None).await.0, StatusCode::UNAUTHORIZED);
    let (status, value) = f.get_json(&path, Some(&f.token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["environment"], json!({}));
    let body = json!({
        "project": f.project,
        "settings": {
            "harness": "OpenCode",
            "default_models": {"OpenCode": "provider/model"},
            "environment": {"LANG": "pl_PL.UTF-8"}
        }
    });
    assert_eq!(
        f.put("/v1/project/settings", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.put("/v1/project/settings", Some(&admin_token), body)
            .await
            .0,
        StatusCode::OK
    );
    let (_, value) = f.get_json(&path, Some(&f.token)).await;
    assert_eq!(value["default_models"]["OpenCode"], "provider/model");
    assert_eq!(value["environment"]["LANG"], "pl_PL.UTF-8");
    assert_eq!(admin.name, "admin");
}

#[tokio::test]
async fn relay_checks_record_real_exit_status_for_granted_users() {
    let f = Fixture::new();
    let body = json!({
        "project": f.project,
        "argv": ["/bin/sh", "-c", "printf out; printf err >&2; exit 7"]
    });
    assert_eq!(
        f.post("/v1/checks/run", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, report) = f.post("/v1/checks/run", Some(&f.token), body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["exit_code"], 7);
    assert_eq!(report["stdout"], "out");
    assert_eq!(report["stderr"], "err");
    assert_eq!(report["result"], "exited_nonzero");
    assert_eq!(
        f.post(
            "/v1/checks/cancel",
            Some(&f.token),
            json!({"project": f.project, "id": "unknown"})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[test]
fn audit_artifact_names_are_allowlisted_and_bounded() {
    assert!(super::safe_audit_run("run-0190"));
    assert!(!super::safe_audit_run("../run"));
    assert!(!super::safe_audit_run("run/one"));
    assert!(super::safe_audit_file("findings.json"));
    assert!(super::safe_audit_file("verification-src-auth.json"));
    assert!(!super::safe_audit_file("../findings.json"));
    assert!(!super::safe_audit_file("secret.txt"));
    assert!(!super::safe_audit_file("exports/../../secret"));
}

#[tokio::test]
async fn audit_artifacts_are_scoped_to_the_granted_project() {
    let fixture = Fixture::new();
    let run = fixture.root.join(".dunlin/audits/run-1");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("findings.json"), "[]").unwrap();
    fs::write(run.join("secret.txt"), "no").unwrap();

    let (status, body) = fixture
        .get_json(
            &format!("/v1/audits?project={}", fixture.project),
            Some(&fixture.token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["id"], "run-1");
    assert!(
        body[0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "findings.json")
    );
    assert!(
        !body[0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "secret.txt")
    );

    let (status, body) = fixture
        .get_json(
            &format!(
                "/v1/audits/file?project={}&run=run-1&name=findings.json",
                fixture.project
            ),
            Some(&fixture.token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["contents"], "[]");

    let (status, _) = fixture
        .get_json(&format!("/v1/audits?project={}", fixture.project), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = fixture
        .get_json(
            &format!("/v1/audits?project={}", fixture.project),
            Some(&fixture.ungranted_token),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = fixture
        .get_json(
            &format!(
                "/v1/audits/file?project={}&run=run-1&name=secret.txt",
                fixture.project
            ),
            Some(&fixture.token),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
