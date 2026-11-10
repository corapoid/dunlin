use super::*;

#[tokio::test]
async fn presence_requires_authentication_and_ignores_caller_identity() {
    let fixture = Fixture::new();
    let (status, _) = fixture.post("/v1/presence", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, snapshot) = fixture
        .post(
            "/v1/presence",
            Some(&fixture.token),
            json!({"user_id":"forged", "name":"forged"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["users"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["users"][0]["name"], "member");
    assert_eq!(snapshot["self_user_id"], snapshot["users"][0]["id"]);
    assert!(snapshot["users"][0].get("device_id").is_none());
    assert!(snapshot["users"][0].get("projects").is_none());
}

#[tokio::test]
async fn presence_counts_devices_per_user_and_removes_revoked_devices() {
    let fixture = Fixture::new();
    let (second, second_token) = {
        let store = fixture.state.store.lock().unwrap();
        let (user, _) = store.authenticate(&fixture.token).unwrap().unwrap();
        store.create_device(&user.id, "another Mac").unwrap()
    };
    // Merely pairing a device or accessing another API does not mark it online.
    fixture.get_json("/v1/projects", Some(&second_token)).await;
    let (_, snapshot) = fixture
        .post("/v1/presence", Some(&fixture.token), json!({}))
        .await;
    assert_eq!(snapshot["users"][0]["devices"], 1);
    fixture
        .post("/v1/presence", Some(&second_token), json!({}))
        .await;
    let (_, snapshot) = fixture
        .post("/v1/presence", Some(&fixture.ungranted_token), json!({}))
        .await;
    let users = snapshot["users"].as_array().unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(
        users.iter().find(|user| user["name"] == "member").unwrap()["devices"],
        2
    );
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .revoke_device(&second.id)
        .unwrap();
    let (_, snapshot) = fixture
        .post("/v1/presence", Some(&fixture.token), json!({}))
        .await;
    assert_eq!(
        snapshot["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["name"] == "member")
            .unwrap()["devices"],
        1
    );
    let (status, _) = fixture
        .post("/v1/presence", Some(&second_token), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn presence_is_local_to_the_relay_and_does_not_return_offline_accounts() {
    let first = Fixture::new();
    let second = Fixture::new();
    first
        .post("/v1/presence", Some(&first.ungranted_token), json!({}))
        .await;
    let (_, snapshot) = second
        .post("/v1/presence", Some(&second.token), json!({}))
        .await;
    assert_eq!(snapshot["users"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["users"][0]["name"], "member");
}

#[tokio::test]
async fn presence_retains_the_administrator_only_host_account_boundary() {
    let fixture = Fixture::new();
    fixture
        .post("/v1/presence", Some(&fixture.token), json!({}))
        .await;
    let admin_token = {
        let store = fixture.state.store.lock().unwrap();
        let admin = store.create_user("admin", "admin").unwrap();
        store.create_device(&admin.id, "admin Mac").unwrap().1
    };
    let settings = crate::management::root(&fixture.state.config);
    fs::create_dir_all(&settings).unwrap();
    // An unreadable policy already fails closed elsewhere in the relay.
    fs::write(settings.join("settings.json"), "corrupt policy").unwrap();
    let (status, _) = fixture
        .post("/v1/presence", Some(&fixture.token), json!({}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, snapshot) = fixture
        .post("/v1/presence", Some(&admin_token), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["users"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["users"][0]["name"], "admin");
}

#[tokio::test]
async fn presence_responses_cannot_be_cached() {
    let fixture = Fixture::new();
    let response = router(fixture.state.clone())
        .oneshot(
            Request::post("/v1/presence")
                .header("Authorization", format!("Bearer {}", fixture.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["Cache-Control"], "no-store");
}
