use super::{AppState, Config, Store, router};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use tower::ServiceExt;

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
                serde_json::from_slice(&bytes).unwrap()
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
