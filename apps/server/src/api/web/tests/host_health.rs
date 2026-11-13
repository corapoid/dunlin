use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[tokio::test]
async fn host_health_live_rechecks_manager_authority_without_any_host_data_change() {
    let (state, _, _) = fixture();
    let (owner, manager, workspace, host) = {
        let store = state.store.lock().unwrap();
        let owner = store.create_user("health-owner", "admin").unwrap();
        let manager = store.create_user("health-manager", "member").unwrap();
        let workspace = store.create_workspace(&owner, "Health workspace").unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &manager.id,
                WorkspaceRole::Admin,
                true,
                None,
            )
            .unwrap();
        let host = store.create_host(&owner.id, &workspace.id,
            serde_json::from_value(json!({"name":"Health host","node_id":"health-node","serve_origin":"https://health.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":[]}})).unwrap()).unwrap();
        (owner, manager, workspace, host.host)
    };
    let cookie = paired_cookie(&state, &manager.id).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::api::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    struct Cleanup(tokio::task::AbortHandle);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _cleanup = Cleanup(server.abort_handle());
    let url = format!(
        "{}/v1/web/workspaces/{}/hosts/{}/health/live",
        origin.replace("http://", "ws://"),
        workspace.id,
        host.id
    );
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "dunlin.host-health.v1".parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response.headers()["sec-websocket-protocol"],
        "dunlin.host-health.v1"
    );
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(first) = first else {
        panic!("expected observation snapshot")
    };
    let first: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["protocol_version"], 1);
    assert_eq!(first["sequence"], 1);
    assert_eq!(first["health"]["host"]["id"], host.id);
    assert_eq!(first["health"]["host"]["workspace_id"], workspace.id);
    assert_eq!(first["health"]["observational"], true);
    assert!(first["health"]["connection"].is_null());
    assert!(first["health"].get("credential").is_none());
    assert!(first["health"]["host"].get("enrollment").is_none());
    // No host or runtime row changes: a push subscriber must still lose access
    // on membership revision, not wait for the next changed health payload.
    state
        .store
        .lock()
        .unwrap()
        .set_team_member(
            &owner.id,
            &workspace.id,
            &manager.id,
            WorkspaceRole::Viewer,
            true,
            Some(1),
        )
        .unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(next) = next else {
        panic!("expected unavailable marker before close")
    };
    let next: Value = serde_json::from_str(&next).unwrap();
    assert_eq!(next["type"], "unavailable");
    assert_eq!(next["stream_id"], first["stream_id"]);
    assert!(next.get("health").is_none());
    let closed = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    assert!(matches!(closed, None | Some(Ok(Message::Close(_)))));
}
