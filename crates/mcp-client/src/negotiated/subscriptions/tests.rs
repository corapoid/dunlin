use super::*;
use crate::{
    negotiated::{
        Client,
        tests::{discover, header, success, tool, tools},
    },
    tests::{Mock, Reply},
};

fn filter() -> SubscriptionFilter {
    SubscriptionFilter {
        tools_list_changed: true,
        ..Default::default()
    }
}
fn notification(id: u64, method: &str, extra: Value) -> Value {
    let mut params = json!({"_meta":{SUB_ID:id}});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"jsonrpc":"2.0","method":method,"params":params})
}
fn ack(id: u64, filter: Value) -> Value {
    notification(
        id,
        "notifications/subscriptions/acknowledged",
        json!({"notifications":filter}),
    )
}
fn end(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","_meta":{SUB_ID:id}}})
}
fn stream(messages: Vec<Value>) -> Reply {
    let body = messages
        .into_iter()
        .map(|message| format!("data: {message}\r\n\r\n"))
        .collect::<String>();
    Reply::result("subscriptions/listen", Value::Null)
        .sse()
        .raw(&format!(": keepalive\r\n\r\n{body}"))
}

#[test]
fn post_subscription_acknowledges_subset_and_invalidates_tools_before_the_next_call() {
    // listen id3; refreshing the catalog id4 happens while the SSE body remains
    // independently owned, and consumes neither its notification nor closure.
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        stream(vec![
            ack(3, json!({"toolsListChanged":true})),
            notification(3, "notifications/tools/list_changed", json!({})),
            end(3),
        ]),
        tools(&[tool()]),
        tools(&[tool()]),
        success(),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let old = client.tools().unwrap();
    let mut requested = filter();
    requested.prompts_list_changed = true;
    let mut subscription = client.listen(requested).unwrap();
    assert!(subscription.acknowledged_filter().tools_list_changed);
    assert!(!subscription.acknowledged_filter().prompts_list_changed);
    assert_eq!(client.call(&old[0], json!({})), Err(Error::Unsupported));
    let refreshed = client.tools().unwrap();
    assert_eq!(subscription.next_event(), Ok(Some(Change::Tools)));
    assert_eq!(
        client.call(&refreshed[0], json!({})),
        Err(Error::Unsupported)
    );
    assert_eq!(subscription.next_event(), Ok(None));
    assert_eq!(subscription.next_event(), Ok(None));
    let catalog = client.tools().unwrap();
    client.call(&catalog[0], json!({})).unwrap();
    let requests = mock.finish();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        header(&requests[2], "Mcp-Method"),
        Some("subscriptions/listen")
    );
    assert_eq!(
        header(&requests[2], "MCP-Protocol-Version"),
        Some(metadata::VERSION)
    );
    assert_eq!(header(&requests[2], "Last-Event-ID"), None);
    assert!(requests.iter().all(|r| r.headers.starts_with("POST ")));
}

#[test]
fn wrong_ack_ids_expanded_filters_and_events_before_ack_fail_closed() {
    for messages in [
        vec![ack(999, json!({"toolsListChanged":true}))],
        vec![ack(2, json!({"resourcesListChanged":true}))],
        vec![notification(
            2,
            "notifications/tools/list_changed",
            json!({}),
        )],
        vec![end(2)],
    ] {
        let mock = Mock::new(vec![discover(), stream(messages)]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        assert!(client.listen(filter()).is_err());
        assert_eq!(mock.finish().len(), 2);
    }
}

#[test]
fn notifications_must_match_the_acknowledged_filter_and_subscription_id() {
    for message in [
        notification(99, "notifications/tools/list_changed", json!({})),
        notification(2, "notifications/prompts/list_changed", json!({})),
        notification(
            2,
            "notifications/resources/updated",
            json!({"uri":"repo://private"}),
        ),
        notification(2, "notifications/progress", json!({})),
        ack(2, json!({"toolsListChanged":true})),
        json!({"jsonrpc":"2.0","id":2,"method":"sampling/createMessage","params":{}}),
        end(999),
    ] {
        let mock = Mock::new(vec![
            discover(),
            stream(vec![ack(2, json!({"toolsListChanged":true})), message]),
        ]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let mut subscription = client.listen(filter()).unwrap();
        assert!(subscription.next_event().is_err());
        assert_eq!(subscription.next_event(), Ok(None));
        assert_eq!(mock.finish().len(), 2);
    }
}

#[test]
fn resource_notifications_match_exact_uris_and_preserve_client_scope() {
    let requested = SubscriptionFilter {
        resources_list_changed: true,
        prompts_list_changed: true,
        resource_subscriptions: vec!["repo://workspace/report".into()],
        ..Default::default()
    };
    let mock = Mock::new(vec![
        discover(),
        stream(vec![
            ack(2, serde_json::to_value(&requested).unwrap()),
            notification(
                2,
                "notifications/resources/updated",
                json!({"uri":"repo://workspace/report"}),
            ),
            notification(2, "notifications/prompts/list_changed", json!({})),
            notification(2, "notifications/resources/list_changed", json!({})),
            end(2),
        ]),
    ]);
    let mut client = Client::connect(&mock.url, "first-user").unwrap();
    let mut subscription = client.listen(requested).unwrap();
    assert_eq!(
        subscription.next_event(),
        Ok(Some(Change::Resource {
            uri: "repo://workspace/report".into()
        }))
    );
    assert_eq!(subscription.next_event(), Ok(Some(Change::Prompts)));
    assert_eq!(subscription.next_event(), Ok(Some(Change::Resources)));
    assert_eq!(subscription.next_event(), Ok(None));
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn stream_drop_or_unexpected_eof_invalidates_catalog_without_automatic_reconnect() {
    for abrupt in [false, true] {
        let mock = Mock::new(vec![
            discover(),
            stream(vec![ack(2, json!({"toolsListChanged":true}))]),
            tools(&[tool()]),
        ]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let mut subscription = client.listen(filter()).unwrap();
        let catalog = client.tools().unwrap();
        if abrupt {
            assert_eq!(subscription.next_event(), Err(Error::Transport));
        }
        drop(subscription);
        assert_eq!(client.call(&catalog[0], json!({})), Err(Error::Unsupported));
        assert_eq!(mock.finish().len(), 3);
    }
}

#[test]
fn subscription_budget_is_reserved_before_post_and_released_when_dropped() {
    let mut replies = vec![discover()];
    for id in 2..=6 {
        replies.push(stream(vec![ack(id, json!({"toolsListChanged":true}))]));
    }
    let mock = Mock::new(replies);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let mut held = Vec::new();
    for _ in 0..MAX_STREAMS {
        held.push(client.listen(filter()).unwrap());
    }
    assert!(matches!(client.listen(filter()), Err(Error::Limit)));
    drop(held.pop());
    held.push(client.listen(filter()).unwrap());
    client.close().unwrap();
    for subscription in &mut held {
        assert_eq!(subscription.next_event(), Err(Error::Protocol));
    }
    assert_eq!(mock.finish().len(), 6);
}

#[test]
fn sse_comments_multiline_data_and_strict_memory_bounds() {
    let bytes =
        b":keepalive\r\n\r\nid: unused\nevent: message\ndata: {\ndata: \"jsonrpc\":\"2.0\"}\n\n";
    let mut sse = Sse::new(Box::new(std::io::Cursor::new(bytes.to_vec())));
    assert_eq!(sse.message().unwrap(), Some(json!({"jsonrpc":"2.0"})));
    assert_eq!(sse.message().unwrap(), None);
    let mut sse = Sse::new(Box::new(std::io::Cursor::new(format!(
        "data: {}\n\n",
        "x".repeat(MAX_FRAME)
    ))));
    assert_eq!(sse.message(), Err(Error::Limit));
    let mut sse = Sse::new(Box::new(std::io::Cursor::new(
        ": heartbeat\n\n".repeat(MAX_BYTES / 10),
    )));
    assert_eq!(sse.message(), Err(Error::Limit));
    let mut sse = Sse::new(Box::new(std::io::Cursor::new(
        "data: {}\n\n".repeat(MAX_MESSAGES + 1),
    )));
    for _ in 0..MAX_MESSAGES {
        assert!(sse.message().unwrap().is_some());
    }
    assert_eq!(sse.message(), Err(Error::Limit));
}

#[test]
fn invalid_filters_and_legacy_subscriptions_do_not_open_a_stream() {
    let mock = Mock::new(vec![discover()]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert!(client.listen(SubscriptionFilter::default()).is_err());
    assert!(
        client
            .listen(SubscriptionFilter {
                resource_subscriptions: vec!["repo://a".into(), "repo://a".into()],
                ..Default::default()
            })
            .is_err()
    );
    assert!(matches!(
        client.listen(SubscriptionFilter {
            resource_subscriptions: vec!["repo://a".into(); 65],
            ..Default::default()
        }),
        Err(Error::Limit)
    ));
    assert_eq!(mock.finish().len(), 1);
    let mock = Mock::new(vec![
        Reply::result("server/discover", Value::Null)
            .status(400)
            .raw("legacy"),
        Reply::initialize(),
        Reply::notification(),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert!(matches!(client.listen(filter()), Err(Error::Unsupported)));
    assert_eq!(mock.finish().len(), 3);
}

#[test]
fn notifications_cannot_invalidate_another_authenticated_clients_catalog() {
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        discover(),
        tools(&[tool()]),
        stream(vec![
            ack(3, json!({"toolsListChanged":true})),
            notification(3, "notifications/tools/list_changed", json!({})),
        ]),
        tools(&[tool()]),
        success(),
    ]);
    let mut first = Client::connect(&mock.url, "first-user").unwrap();
    first.tools().unwrap();
    let mut second = Client::connect(&mock.url, "second-user").unwrap();
    let other = second.tools().unwrap();
    let mut subscription = first.listen(filter()).unwrap();
    let current = first.tools().unwrap();
    assert_eq!(subscription.next_event(), Ok(Some(Change::Tools)));
    assert_eq!(first.call(&current[0], json!({})), Err(Error::Unsupported));
    second.call(&other[0], json!({})).unwrap();
    let requests = mock.finish();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        header(&requests[6], "Authorization"),
        Some("Bearer second-user")
    );
}

#[test]
fn notification_during_page_fetch_prevents_publication_of_a_mixed_catalog() {
    use std::{sync::mpsc, time::Duration};
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let mut second = tool();
    second.name = "second".into();
    let mock = Mock::new(vec![
        discover(),
        stream(vec![
            ack(2, json!({"toolsListChanged":true})),
            notification(2, "notifications/tools/list_changed", json!({})),
        ]),
        Reply::result(
            "tools/list",
            json!({"resultType":"complete","tools":[tool()],"nextCursor":"second"}),
        ),
        tools(&[second]).before_reply(move || {
            entered_sender.send(()).unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let mut subscription = client.listen(filter()).unwrap();
    let reader = std::thread::spawn(move || {
        entered_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let event = subscription.next_event();
        release_sender.send(()).unwrap();
        event
    });
    assert!(matches!(client.tools(), Err(Error::Protocol)));
    assert_eq!(reader.join().unwrap(), Ok(Some(Change::Tools)));
    assert_eq!(client.call(&tool(), json!({})), Err(Error::Unsupported));
    assert_eq!(mock.finish().len(), 4);
}
