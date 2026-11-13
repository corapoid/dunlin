use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
async fn reject(socket: &mut SessionSocket, client: &str, params: Value) {
    socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","method":"dispatchAction","params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert!(value.get("error").is_none(), "{value}");
                    if value["method"] == "action"
                        && value["params"]["origin"]["clientId"] == client
                        && value["params"]["origin"]["clientSeq"] == params["clientSeq"]
                    {
                        assert!(
                            value["params"]["rejectionReason"]
                                .as_str()
                                .unwrap()
                                .contains("schema")
                        );
                        break;
                    }
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                frame => panic!("unexpected frame: {frame:?}"),
            }
        }
    })
    .await
    .expect("canonical invalid form rejection deadline");
}

pub(super) async fn respond(
    fixture: &Fixture,
    id: &str,
    socket: &mut SessionSocket,
    client: &str,
    mode: &str,
) {
    let action = match mode {
        "forms" | "urls" => "accept",
        "forms-decline" | "urls-decline" => "decline",
        "forms-cancel" | "urls-cancel" => "cancel",
        _ => panic!("unexpected form case"),
    };
    let rounds = if action == "accept" { 2 } else { 1 };
    for round in 0..rounds {
        let request = pending(fixture, id).await;
        let mut response =
            json!({"type":"chat/inputCompleted","requestId":request["id"],"response":action});
        assert!(
            request["id"]
                .as_str()
                .unwrap()
                .starts_with("dunlin:mcp-input:")
        );
        assert!(
            request["message"]
                .as_str()
                .unwrap()
                .contains("MCP work (http://127.0.0.1:")
        );
        assert!(!request.to_string().contains("private-mrtr-state"));
        let url = mode.starts_with("urls");
        if url {
            assert_eq!(
                request["url"],
                format!("https://review.example.test/connect?step={}", round + 1)
            );
            assert!(request.get("questions").is_none());
            assert!(!request.to_string().contains("private-url-state"));
        }
        if action == "accept" && !url {
            let content = if round == 0 {
                crate::worker_fixture::mcp_provider::form_content()
            } else {
                json!({"reviewed":true})
            };
            let mut answers = serde_json::Map::new();
            for q in request["questions"].as_array().unwrap() {
                let key = q["id"].as_str().unwrap();
                let kind = match q["kind"].as_str().unwrap() {
                    "text" => "text",
                    "integer" | "number" => "number",
                    "boolean" => "boolean",
                    "single-select" => "selected",
                    "multi-select" => "selected-many",
                    _ => panic!("unexpected AHP form kind"),
                };
                answers.insert(
                    key.into(),
                    json!({"state":"submitted","value":{"kind":kind,"value":content[key]}}),
                );
            }
            response["answers"] = json!(answers);
        }
        if url && round == 0 {
            let mut bad = response.clone();
            bad["answers"] = json!({});
            reject(
                socket,
                client,
                json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":3,"action":bad}),
            )
            .await;
        }
        if action == "accept" && !url && round == 0 {
            let mut bad = response.clone();
            bad["answers"]["count"]["value"]["value"] = json!(0.5);
            reject(
                socket,
                client,
                json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":3,"action":bad}),
            )
            .await;
        }
        let params = json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":if action=="accept" || url {4+round}else{3+round},"action":response});
        session_dispatch(socket, client, params.clone()).await;
        session_dispatch(socket, client, params).await; // Same origin, never another continuation.
    }
}

pub(super) async fn pending(fixture: &Fixture, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let request = {
                let store = fixture.state.store.lock().unwrap();
                let session = store.remote_session_snapshot(id).unwrap();
                let context = store
                    .team_project_context(&fixture.workspace, &session.project, &fixture.owner)
                    .unwrap();
                let chat = serde_json::to_value(
                    store
                        .interactive_journal()
                        .projected_chat(&context, id)
                        .unwrap(),
                )
                .unwrap();
                chat["activeTurn"]["responseParts"]
                    .as_array()
                    .and_then(|parts| {
                        parts
                            .iter()
                            .find(|p| p["kind"] == "inputRequest" && p["response"].is_null())
                    })
                    .map(|p| p["request"].clone())
            };
            if let Some(request) = request {
                break request;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("actual MCP form AHP deadline")
}
