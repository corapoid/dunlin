use super::Fixture;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};
use tower::ServiceExt;
fn git(f: &Fixture, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-C", f.root.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
fn repo(f: &Fixture) {
    git(f, &["init", "--template=", "-qb", "main"]);
    fs::create_dir(f.root.join(".git/hooks")).unwrap();
    git(
        f,
        &[
            "config",
            "core.hooksPath",
            f.root.join(".git/hooks").to_str().unwrap(),
        ],
    );
    git(f, &["config", "user.name", "Relay Test"]);
    git(f, &["config", "core.fsmonitor", "false"]);
    git(f, &["config", "core.attributesFile", "/dev/null"]);
    git(f, &["config", "user.email", "t@example.invalid"]);
    git(f, &["config", "commit.gpgsign", "false"]);
    fs::write(f.root.join("file"), "one\ntwo\n").unwrap();
    git(f, &["add", "file"]);
    git(f, &["commit", "-qm", "first"]);
}
async fn prepare(f: &Fixture, action: Value) -> (StatusCode, Value) {
    f.post(
        "/v1/git/operation/prepare",
        Some(&f.token),
        json!({"project":f.project,"action":action}),
    )
    .await
}
async fn execute(f: &Fixture, id: &Value) -> (StatusCode, Value) {
    f.post(
        "/v1/git/operation/execute",
        Some(&f.token),
        json!({"project":f.project,"id":id}),
    )
    .await
}
#[tokio::test]
async fn git_mutations_require_auth_grants_and_reject_client_roots_commands_and_force() {
    let f = Fixture::new();
    repo(&f);
    let action = json!({"kind":"stash_push","message":"test","include_untracked":true});
    for path in ["/v1/git/operation/prepare", "/v1/git/operation/execute"] {
        assert_eq!(
            f.post(path, None, json!({})).await.0,
            StatusCode::UNAUTHORIZED
        );
        let body = if path.ends_with("prepare") {
            json!({"project":f.project,"action":action})
        } else {
            json!({"project":f.project,"id":"fake"})
        };
        assert_eq!(
            f.post(path, Some(&f.ungranted_token), body).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        prepare(
            &f,
            json!({"kind":"stash_push","message":"","include_untracked":true,"force":true})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.post(
            "/v1/git/operation/prepare",
            Some(&f.token),
            json!({"project":f.project,"action":action,"root":"/outside"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
#[tokio::test]
async fn prepared_stash_is_read_only_until_execute_and_token_is_one_use() {
    let f = Fixture::new();
    repo(&f);
    fs::write(f.root.join("file"), "dirty\n").unwrap();
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let (status, plan) = prepare(
        &f,
        json!({"kind":"stash_push","message":"relay","include_untracked":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(f.root.join("file")).unwrap(), "dirty\n");
    let (status, result) = execute(&f, &plan["id"]).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["success"], true);
    assert_eq!(
        fs::read_to_string(f.root.join("file")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    let oid = result["overview"]["stashes"][0]["oid"].clone();
    let (_, plan) = prepare(&f, json!({"kind":"stash_pop","oid":oid})).await;
    let (status, result) = execute(&f, &plan["id"]).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(fs::read_to_string(f.root.join("file")).unwrap(), "dirty\n");
}

#[tokio::test]
async fn source_control_actions_share_scoped_one_use_consent_and_stale_checks() {
    let f = Fixture::new();
    repo(&f);
    git(&f, &["branch", "feature"]);
    fs::write(f.root.join("file"), "staged\n").unwrap();
    git(&f, &["add", "file"]);
    let (status, plan) = prepare(&f, json!({"kind":"unstage_all"})).await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::OK);
    assert!(git(&f, &["diff", "--cached"]).is_empty());
    assert_eq!(fs::read_to_string(f.root.join("file")).unwrap(), "staged\n");
    let (_, plan) = prepare(&f, json!({"kind":"ignore", "path":"file"})).await;
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::OK);
    assert_eq!(
        fs::read_to_string(f.root.join(".gitignore")).unwrap(),
        "/file\n"
    );
    let (_, plan) = prepare(&f, json!({"kind":"discard", "paths":["file"]})).await;
    fs::write(f.root.join("file"), "newer\n").unwrap();
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    assert_eq!(fs::read_to_string(f.root.join("file")).unwrap(), "newer\n");
    let (status, plan) = prepare(
        &f,
        json!({"kind":"switch", "branch":"feature", "discard":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(git(&f, &["branch", "--show-current"]).trim(), "main");
    let (status, result) = execute(&f, &plan["id"]).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(git(&f, &["branch", "--show-current"]).trim(), "feature");
    assert_eq!(
        fs::read_to_string(f.root.join("file")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
}
#[tokio::test]
async fn stale_expired_and_other_device_confirmations_never_mutate() {
    let f = Fixture::new();
    repo(&f);
    fs::write(f.root.join("file"), "dirty\n").unwrap();
    let action = json!({"kind":"stash_push","message":"relay","include_untracked":false});
    let (_, plan) = prepare(&f, action.clone()).await;
    fs::write(f.root.join("file"), "newer\n").unwrap();
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    assert!(git(&f, &["stash", "list"]).is_empty());
    let (_, plan) = prepare(&f, action.clone()).await;
    f.state
        .git_plans
        .lock()
        .unwrap()
        .get_mut(plan["id"].as_str().unwrap())
        .unwrap()
        .expires = Instant::now() - Duration::from_secs(1);
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    let (_, plan) = prepare(&f, action).await;
    let other = {
        let store = f.state.store.lock().unwrap();
        let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
        store.create_device(&user.id, "other").unwrap().1
    };
    assert_eq!(
        f.post(
            "/v1/git/operation/execute",
            Some(&other),
            json!({"project":f.project,"id":plan["id"]})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fs::read_to_string(f.root.join("file")).unwrap(), "newer\n");
    let device = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap()
        .1;
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn blame_endpoint_is_authenticated_scoped_and_matches_local_objects() {
    let f = Fixture::new();
    repo(&f);
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(f.ungranted_token.as_str()), StatusCode::FORBIDDEN),
        (Some(f.token.as_str()), StatusCode::OK),
    ] {
        let mut request = Request::get(format!("/v1/git/blame?project={}&path=file", f.project));
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = super::router(f.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            let local =
                dunlin_workspace::git::blame::load(f.root.to_str().unwrap(), "file", None).unwrap();
            assert_eq!(value["value"], serde_json::to_value(local).unwrap());
            assert_eq!(value["scope"].as_str().unwrap().len(), 64);
        }
    }
}

#[tokio::test]
async fn deleting_a_linked_worktree_outside_the_granted_root_requires_its_own_grant() {
    let f = Fixture::new();
    repo(&f);
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("tree");
    git(
        &f,
        &[
            "worktree",
            "add",
            "--detach",
            target.to_str().unwrap(),
            "HEAD",
        ],
    );
    let inventory = dunlin_workspace::git::overview::load(f.root.to_str().unwrap()).unwrap();
    let path = inventory
        .worktrees
        .iter()
        .find(|t| !t.current)
        .unwrap()
        .path
        .clone();
    let action = json!({"kind":"worktree_remove","path":path});
    assert_eq!(prepare(&f, action.clone()).await.0, StatusCode::FORBIDDEN);
    assert!(target.join("file").exists());
    {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        let ancestor = store
            .create_project(
                "ancestor cannot adopt linked metadata",
                outside.path().to_str().unwrap(),
            )
            .unwrap();
        store.grant(&ancestor.id, &user.id).unwrap();
    }
    assert_eq!(
        prepare(&f, action.clone()).await.0,
        StatusCode::FORBIDDEN,
        "Ancestor native grant must not stand in for the target's own linked metadata authority"
    );
    {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        let project = store
            .create_project("authorized linked tree", &path)
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
    }
    assert_eq!(
        prepare(&f, action.clone()).await.0,
        StatusCode::FORBIDDEN,
        "Native grant alone must not approve linked Git metadata"
    );
    let linked_project = {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        store
            .projects_for_user(&user.id)
            .unwrap()
            .into_iter()
            .find(|project| project.path == path)
            .unwrap()
            .id
    };
    f.approve_linked_metadata(&linked_project, &target, true);
    let (status, plan) = prepare(&f, action).await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    let (status, result) = execute(&f, &plan["id"]).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["success"], true);
    assert!(!target.exists());
}

#[tokio::test]
async fn prepared_git_mutation_is_consumed_without_writes_after_grant_epoch_changes() {
    let f = Fixture::new();
    repo(&f);
    fs::write(f.root.join("file"), "epoch dirty\n").unwrap();
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let (status, plan) = prepare(
        &f,
        json!({"kind":"stash_push","message":"stale epoch","include_untracked":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        store.revoke_grant(&f.project, &user.id).unwrap();
        store.grant(&f.project, &user.id).unwrap();
    }
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::FORBIDDEN);
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    assert_eq!(
        fs::read_to_string(f.root.join("file")).unwrap(),
        "epoch dirty\n"
    );
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
    assert!(git(&f, &["stash", "list"]).is_empty());
    let (status, fresh) = prepare(
        &f,
        json!({"kind":"stash_push","message":"fresh epoch","include_untracked":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(execute(&f, &fresh["id"]).await.0, StatusCode::OK);
}

#[tokio::test]
async fn worktree_removal_pins_covering_grant_epoch_even_if_an_alternative_is_granted() {
    let f = Fixture::new();
    repo(&f);
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("tree");
    git(
        &f,
        &[
            "worktree",
            "add",
            "--detach",
            target.to_str().unwrap(),
            "HEAD",
        ],
    );
    let extra = {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        let project = store
            .create_project("linked epoch", target.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        project.id
    };
    f.approve_linked_metadata(&extra, &target, true);
    let (status, plan) = prepare(&f, json!({"kind":"worktree_remove","path":target})).await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    {
        let plans = f.state.git_plans.lock().unwrap();
        assert_eq!(
            plans[plan["id"].as_str().unwrap()]
                .worktree_grant
                .as_ref()
                .unwrap()
                .project
                .id,
            extra
        );
    }
    {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        store.revoke_grant(&extra, &user.id).unwrap();
        store.grant(&extra, &user.id).unwrap();
        let replacement = store
            .create_project("new covering grant", outside.path().to_str().unwrap())
            .unwrap();
        store.grant(&replacement.id, &user.id).unwrap();
    }
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::FORBIDDEN);
    assert_eq!(execute(&f, &plan["id"]).await.0, StatusCode::CONFLICT);
    assert!(target.join("file").exists());
    assert_eq!(
        dunlin_workspace::git::overview::load(f.root.to_str().unwrap())
            .unwrap()
            .worktrees
            .len(),
        2
    );
}

#[tokio::test]
async fn linked_prepared_mutation_requires_write_and_consumes_old_metadata_grant_epoch() {
    let f = Fixture::new();
    repo(&f);
    let directory = tempfile::tempdir().unwrap();
    let linked = directory.path().join("linked");
    git(
        &f,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    let project = {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        let project = store
            .create_project("linked mutation", linked.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        project.id
    };
    fs::write(linked.join("file"), "linked dirty\n").unwrap();
    f.approve_linked_metadata(&project, &linked, false);
    let body = json!({"project":project,"action":{"kind":"stash_push","message":"linked","include_untracked":false}});
    assert_eq!(
        f.post("/v1/git/operation/prepare", Some(&f.token), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    f.approve_linked_metadata(&project, &linked, true);
    let (status, plan) = f
        .post("/v1/git/operation/prepare", Some(&f.token), body.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    {
        let store = f.state.store.lock().unwrap();
        let user = store.authenticate(&f.token).unwrap().unwrap().0;
        store
            .revoke_local_metadata_grant(&project, &user.id, 2)
            .unwrap();
    }
    f.approve_linked_metadata(&project, &linked, true);
    let execute_body = json!({"project":project,"id":plan["id"]});
    assert_eq!(
        f.post(
            "/v1/git/operation/execute",
            Some(&f.token),
            execute_body.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.post("/v1/git/operation/execute", Some(&f.token), execute_body)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        fs::read_to_string(linked.join("file")).unwrap(),
        "linked dirty\n"
    );
    assert!(git(&f, &["stash", "list"]).is_empty());
    let (status, fresh) = f
        .post("/v1/git/operation/prepare", Some(&f.token), body)
        .await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    let (status, outcome) = f
        .post(
            "/v1/git/operation/execute",
            Some(&f.token),
            json!({"project":project,"id":fresh["id"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
}
