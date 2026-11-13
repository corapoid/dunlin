use super::*;
use crate::store::local_metadata::MetadataPermission;
struct Linked {
    _dir: tempfile::TempDir,
    state: AppState,
    owner: String,
    workspace: String,
    project: String,
    native: String,
    cookie: String,
    root: std::path::PathBuf,
    git: std::path::PathBuf,
    common: std::path::PathBuf,
}
impl Linked {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        let common = dir.path().join("repo.git");
        let git = common.join("worktrees/browser");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir_all(&git).unwrap();
        std::fs::create_dir_all(common.join("objects")).unwrap();
        std::fs::write(root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
        std::fs::write(git.join("commondir"), "../..").unwrap();
        std::fs::write(git.join("gitdir"), root.join(".git").to_str().unwrap()).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(common.join("HEAD"), "ref: refs/heads/main").unwrap();
        let (state, _, _) = fixture();
        let (owner, workspace, project, native, preparation) = {
            let s = state.store.lock().unwrap();
            let u = s.create_user("binding-metadata-admin", "admin").unwrap();
            let w = s.create_workspace(&u, "metadata binding").unwrap();
            let p = s.create_team_project(&u.id, &w.id, "logical").unwrap();
            let n = s.create_project("linked", root.to_str().unwrap()).unwrap();
            s.grant(&n.id, &u.id).unwrap();
            let prep = s.prepare_local_metadata(&n.id).unwrap();
            (u.id, w.id, p.id, n.id, prep)
        };
        let captured = preparation.capture(&git, &common).unwrap();
        state
            .store
            .lock()
            .unwrap()
            .commit_local_metadata(captured, None)
            .unwrap();
        let cookie = paired_cookie(&state, &owner).await;
        Self {
            _dir: dir,
            state,
            owner,
            workspace,
            project,
            native,
            cookie,
            root,
            git,
            common,
        }
    }
    fn path(&self) -> String {
        format!(
            "/v1/web/workspaces/{}/projects/{}/binding",
            self.workspace, self.project
        )
    }
    fn grant(&self, approval: u64, expected: Option<u64>) {
        self.state
            .store
            .lock()
            .unwrap()
            .set_local_metadata_grant(
                &self.native,
                &self.owner,
                approval,
                1,
                &[MetadataPermission::Read],
                true,
                expected,
                false,
            )
            .unwrap();
    }
    async fn request(&self, method: &str, body: Value) -> (StatusCode, Value) {
        team_request(&self.state, method, &self.path(), &self.cookie, body).await
    }
}
#[tokio::test]
async fn linked_binding_http_requires_explicit_approval_and_own_read_without_hiding_saved_config() {
    let f = Linked::new().await;
    assert_eq!(
        f.request(
            "PUT",
            json!({"native_project_id":f.native,"expected_revision":null})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let body = json!({"native_project_id":f.native,"expected_revision":null,"metadata_approval_revision":1});
    assert_eq!(
        f.request("PUT", body.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    f.grant(1, None);
    let (status, saved) = f.request("PUT", body).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["metadata_approval_revision"], 1);
    assert_eq!(saved["repo_kind"], "linked_worktree");
    assert!(saved.get("root").is_none());
    assert!(saved.get("identity").is_none());
    let (status, view) = f.request("GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["configuration_current"], true);
    assert_eq!(view["can_manage"], true);
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata_grant(&f.native, &f.owner, 1)
        .unwrap();
    let (status, view) = f.request("GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["binding"], saved);
    assert_eq!(view["configuration_current"], false);
    f.grant(1, Some(2));
    assert_eq!(
        f.request("GET", Value::Null).await.1["configuration_current"],
        true
    );
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata(&f.native, 1)
        .unwrap();
    let preparation = f
        .state
        .store
        .lock()
        .unwrap()
        .prepare_local_metadata(&f.native)
        .unwrap();
    let captured = preparation.capture(&f.git, &f.common).unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .commit_local_metadata(captured, Some(2))
        .unwrap();
    f.grant(3, Some(3));
    assert_eq!(
        f.request("GET", Value::Null).await.1["configuration_current"],
        false,
        "A newer approval must not silently refresh the saved pin"
    );
    assert_eq!(f.request("PUT",json!({"native_project_id":f.native,"expected_revision":1,"metadata_approval_revision":1})).await.0,StatusCode::FORBIDDEN);
    let(status,rebound)=f.request("PUT",json!({"native_project_id":f.native,"expected_revision":1,"metadata_approval_revision":3})).await;
    assert_eq!(status, StatusCode::OK, "{rebound}");
    assert_eq!(rebound["revision"], 2);
}
#[tokio::test]
async fn binding_revoke_needs_no_filesystem_slot_root_or_metadata_grant() {
    let f = Linked::new().await;
    f.grant(1, None);
    assert_eq!(
        f.request(
            "PUT",
            json!({"native_project_id":f.native,"metadata_approval_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata_grant(&f.native, &f.owner, 1)
        .unwrap();
    std::fs::rename(&f.root, f.root.with_extension("removed")).unwrap();
    let count = f.state.fs_reads.available_permits();
    let _permits = f
        .state
        .fs_reads
        .clone()
        .acquire_many_owned(count as u32)
        .await
        .unwrap();
    let (status, revoked) = team_request(
        &f.state,
        "POST",
        &format!("{}/revoke", f.path()),
        &f.cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert_eq!(revoked["active"], false);
    assert_eq!(revoked["revision"], 2);
}
#[tokio::test]
async fn linked_binding_rejects_unknown_fields_foreign_origin_and_revoked_cookie_without_save() {
    let f = Linked::new().await;
    f.grant(1, None);
    let rejected = super::super::super::router(f.state.clone()).oneshot(
        Request::put(f.path()).header("host", "build.tail123.ts.net")
            .header("origin", "https://build.tail123.ts.net").header("cookie", &f.cookie)
            .header("content-type", "application/json")
            .body(Body::from(json!({"native_project_id":f.native,"metadata_approval_revision":1,"path":"/hidden"}).to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = super::super::super::router(f.state.clone())
        .oneshot(
            Request::put(f.path())
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://foreign.example")
                .header("cookie", &f.cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"native_project_id":f.native,"metadata_approval_revision":1})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    {
        let s = f.state.store.lock().unwrap();
        for d in s.list_devices(&f.owner).unwrap() {
            s.revoke_device(&d.id).unwrap();
        }
    }
    assert_eq!(
        f.request(
            "PUT",
            json!({"native_project_id":f.native,"metadata_approval_revision":1})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .local_binding_for_user(&f.owner, &f.workspace, &f.project)
            .unwrap()
            .is_none()
    );
}
#[test]
fn local_binding_cookie_revoked_while_fs_worker_is_queued_cannot_commit() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let f = Linked::new().await;
            f.grant(1, None);
            let (entered, started) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            });
            started.await.unwrap();
            let state = f.state.clone();
            let path = f.path();
            let cookie = f.cookie.clone();
            let native = f.native.clone();
            let initial = f.state.fs_reads.available_permits();
            let task = tokio::spawn(async move {
                team_request(
                    &state,
                    "PUT",
                    &path,
                    &cookie,
                    json!({"native_project_id":native,"metadata_approval_revision":1}),
                )
                .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while f.state.fs_reads.available_permits() == initial {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            {
                let s = f.state.store.lock().unwrap();
                for d in s.list_devices(&f.owner).unwrap() {
                    s.revoke_device(&d.id).unwrap();
                }
            }
            release.send(()).unwrap();
            blocker.await.unwrap();
            let (status, value) = task.await.unwrap();
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{value}");
            assert!(
                f.state
                    .store
                    .lock()
                    .unwrap()
                    .local_binding_for_user(&f.owner, &f.workspace, &f.project)
                    .unwrap()
                    .is_none()
            );
        });
}

#[tokio::test]
async fn binding_reader_sees_saved_metadata_without_borrowing_admin_metadata_grant_or_edit_capability()
 {
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};
    let f = Linked::new().await;
    f.grant(1, None);
    assert_eq!(
        f.request(
            "PUT",
            json!({"native_project_id":f.native,"metadata_approval_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let reader = {
        let s = f.state.store.lock().unwrap();
        let reader = s.create_user("binding-viewer", "member").unwrap();
        s.set_team_member(
            &f.owner,
            &f.workspace,
            &reader.id,
            WorkspaceRole::Member,
            true,
            None,
        )
        .unwrap();
        s.set_team_project_grant(
            &f.owner,
            &f.workspace,
            &f.project,
            &reader.id,
            [Permission::ViewProject].into_iter().collect(),
            None,
        )
        .unwrap();
        s.grant(&f.native, &reader.id).unwrap();
        reader
    };
    let cookie = paired_cookie(&f.state, &reader.id).await;
    let (status, view) = team_request(&f.state, "GET", &f.path(), &cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["binding"]["metadata_approval_revision"], 1);
    assert_eq!(view["configuration_current"], false);
    assert_eq!(view["can_manage"], false);
    assert_eq!(team_request(&f.state,"PUT",&f.path(),&cookie,json!({"native_project_id":f.native,"metadata_approval_revision":1,"expected_revision":1})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(
        team_request(
            &f.state,
            "POST",
            &format!("{}/revoke", f.path()),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request("GET", Value::Null).await.1["binding"]["revision"],
        1
    );
}
