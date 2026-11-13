use super::*;
use dunlin_domain::collaboration::{Permission, WorkspaceRole};
fn definition(name: &str) -> Value {
    json!({"schema_version":1,"name":name,"description":"Fixture","harness":"claude","model":"fixture-model","mode":"plan","instructions":"agent-private-instructions-marker","skills":[{"id":"review","version":"v1","instructions":"Review carefully"}],"max_seconds":60,"max_output_bytes":1048576})
}
#[tokio::test]
async fn agent_http_versions_catalog_cas_and_current_manage_authority_are_scoped() {
    let (state, _, _) = fixture();
    let (owner, editor, w, p) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("agent-owner", "admin").unwrap();
        let editor = store.create_user("agent-editor", "member").unwrap();
        let w = store.create_workspace(&owner, "Agents").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "Profiles")
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &w.id,
                &editor.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &w.id,
                &p.id,
                &editor.id,
                [
                    Permission::ViewProject,
                    Permission::EditWorkflow,
                    Permission::Publish,
                ]
                .into(),
                None,
            )
            .unwrap();
        (owner, editor, w, p)
    };
    let owner_cookie = paired_cookie(&state, &owner.id).await;
    let editor_cookie = paired_cookie(&state, &editor.id).await;
    let base = format!("/v1/web/workspaces/{}/projects/{}", w.id, p.id);
    let drafts = format!("{base}/agents");
    let profiles = format!("{base}/agent-profiles");
    let catalog = team_request(&state, "GET", &drafts, &editor_cookie, Value::Null).await;
    assert_eq!(catalog.0, StatusCode::OK);
    assert_eq!(catalog.1["can_edit"], false);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &drafts,
            &editor_cookie,
            definition("Denied")
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "Workflow publishing grants do not manage agents"
    );
    let (status, created) =
        team_request(&state, "POST", &drafts, &owner_cookie, definition("First")).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let path = format!("{drafts}/{id}");
    let publish = format!("{path}/publish");
    assert_eq!(
        team_request(&state, "GET", &profiles, &editor_cookie, Value::Null)
            .await
            .1["profiles"],
        json!([])
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish,
            &editor_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, published) = team_request(
        &state,
        "POST",
        &publish,
        &owner_cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(published["agent_id"], id);
    assert_eq!(published["digest"].as_str().unwrap().len(), 64);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish,
            &owner_cookie,
            json!({"expected_revision":1})
        )
        .await
        .1,
        published
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_project_grant(
            &owner.id,
            &w.id,
            &p.id,
            &editor.id,
            [Permission::ViewProject, Permission::ManageProject].into(),
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &drafts, &editor_cookie, Value::Null)
            .await
            .1["can_edit"],
        false,
        "A project grant cannot bypass the Member role ceiling"
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &w.id,
            &editor.id,
            WorkspaceRole::Admin,
            true,
            Some(1),
        )
        .unwrap();
    assert_eq!(
        team_request(&state, "GET", &drafts, &editor_cookie, Value::Null)
            .await
            .1["can_edit"],
        true
    );
    let request = json!({"expected_revision":1,"definition":definition("Second")});
    let (left, right) = tokio::join!(
        team_request(&state, "PUT", &path, &editor_cookie, request.clone()),
        team_request(&state, "PUT", &path, &editor_cookie, request)
    );
    assert!([left.0, right.0].contains(&StatusCode::OK));
    assert!([left.0, right.0].contains(&StatusCode::CONFLICT));
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish,
            &editor_cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        team_request(
            &state,
            "GET",
            &format!("{path}/versions/1"),
            &editor_cookie,
            Value::Null
        )
        .await
        .1,
        published
    );
    let catalog = team_request(&state, "GET", &profiles, &editor_cookie, Value::Null).await;
    assert_eq!(catalog.0, StatusCode::OK);
    assert_eq!(catalog.1["profiles"], json!([published]));
    assert!(catalog.1.get("configured_harness").is_some());
    let (status, second) = team_request(
        &state,
        "POST",
        &publish,
        &editor_cookie,
        json!({"expected_revision":2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["version"], 2);
    assert_eq!(
        team_request(&state, "GET", &profiles, &editor_cookie, Value::Null)
            .await
            .1["profiles"],
        json!([second])
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_project_grant(
            &owner.id,
            &w.id,
            &p.id,
            &editor.id,
            [Permission::ViewProject].into(),
            Some(2),
        )
        .unwrap();
    assert_eq!(
        team_request(
            &state,
            "POST",
            &publish,
            &editor_cookie,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "A duplicate publication needs fresh ManageProject"
    );
    assert_eq!(
        team_request(
            &state,
            "PUT",
            &path,
            &editor_cookie,
            json!({"expected_revision":2,"definition":definition("Denied after downgrade")})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let audit = state
        .store
        .lock()
        .unwrap()
        .team_audit(&owner.id, &w.id)
        .unwrap();
    let raw = serde_json::to_string(&audit).unwrap();
    assert!(!raw.contains("agent-private-instructions-marker"));
    assert_eq!(
        audit
            .iter()
            .filter(|entry| entry.action == "agent_published")
            .count(),
        2
    );
    state
        .store
        .lock()
        .unwrap()
        .set_team_project_grant(
            &owner.id,
            &w.id,
            &p.id,
            &editor.id,
            Default::default(),
            Some(3),
        )
        .unwrap();
    for route in [&drafts, &path, &profiles, &format!("{path}/versions/1")] {
        assert_eq!(
            team_request(&state, "GET", route, &editor_cookie, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        team_request(
            &state,
            "GET",
            "/v1/web/session",
            &editor_cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK,
        "Project revocation does not invalidate browser identity"
    );
}

#[tokio::test]
async fn agent_http_rejects_cross_origin_invalid_definition_and_revoked_cookie_without_side_effects()
 {
    let (state, _, _) = fixture();
    let (owner, w, p) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("agent-owner", "admin").unwrap();
        let w = store.create_workspace(&owner, "Agents").unwrap();
        let p = store
            .create_team_project(&owner.id, &w.id, "Profiles")
            .unwrap();
        (owner, w, p)
    };
    let cookie = paired_cookie(&state, &owner.id).await;
    let path = format!("/v1/web/workspaces/{}/projects/{}/agents", w.id, p.id);
    let response = crate::api::router(state.clone())
        .oneshot(
            Request::post(&path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://attacker.invalid")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(definition("Denied").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let mut invalid = definition("Invalid");
    invalid["harness"] = json!("sh arbitrary command");
    assert_eq!(
        team_request(&state, "POST", &path, &cookie, invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut unknown = definition("Unknown");
    unknown["working_directory"] = json!("/tmp");
    // Axum's extraction rejection is plain text, unlike our JSON API errors.
    let response = crate::api::router(state.clone())
        .oneshot(
            Request::post(&path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://build.tail123.ts.net")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(unknown.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("working_directory"));
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .agent_drafts(&owner.id, &w.id, &p.id)
            .unwrap()
            .is_empty()
    );
    let device = state
        .store
        .lock()
        .unwrap()
        .list_devices(&owner.id)
        .unwrap()
        .into_iter()
        .find(|device| device.user_id == owner.id)
        .unwrap();
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        team_request(&state, "POST", &path, &cookie, definition("Revoked"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .agent_drafts(&owner.id, &w.id, &p.id)
            .unwrap()
            .is_empty()
    );
}
