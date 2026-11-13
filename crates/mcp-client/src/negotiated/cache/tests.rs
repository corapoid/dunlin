use super::*;
use crate::{
    metadata,
    negotiated::{
        Change, Client, Outcome, SubscriptionFilter, Transport,
        tests::{discover, tool},
    },
    tests::{Mock, Reply},
};
use serde_json::json;
use std::collections::BTreeMap;

fn key(method: &str, params: Value) -> Key {
    Key::new(method, &params).unwrap().unwrap()
}
fn page(items: Value, ttl: u64, scope: &str, cursor: Option<&str>) -> Reply {
    let mut value = json!({"resultType":"complete","tools":items,"ttlMs":ttl,"cacheScope":scope});
    if let Some(cursor) = cursor {
        value["nextCursor"] = json!(cursor);
    }
    Reply::result("tools/list", value)
}
fn resource_discover() -> Reply {
    Reply::result(
        "server/discover",
        json!({"resultType":"complete","supportedVersions":[metadata::VERSION],"capabilities":{"resources":{},"tools":{}},"ttlMs":0,"cacheScope":"private"}),
    )
}
fn resource(text: &str, scope: &str) -> Reply {
    Reply::result(
        "resources/read",
        json!({"resultType":"complete","contents":[{"uri":"repo://a","text":text}],"ttlMs":60000,"cacheScope":scope}),
    )
}
fn value(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Complete(value) => value,
        _ => panic!("complete expected"),
    }
}

#[test]
fn cache_keys_separate_methods_parameters_and_exclude_mutations_and_mrtr() {
    let mut cache = Cache::default();
    let one = key("resources/read", json!({"uri":"repo://a"}));
    let result = json!({"resultType":"complete","ttlMs":1000,"cacheScope":"private","value":"a"});
    cache
        .put(one.clone(), &result, [0; 3], Instant::now())
        .unwrap();
    assert_eq!(cache.get(&one, [0; 3]), Some(result));
    assert_eq!(
        cache.get(&key("resources/read", json!({"uri":"repo://b"})), [0; 3]),
        None
    );
    assert_eq!(
        cache.get(&key("tools/list", json!({"uri":"repo://a"})), [0; 3]),
        None
    );
    for method in [
        "tools/call",
        "prompts/get",
        "subscriptions/listen",
        "initialize",
    ] {
        assert!(Key::new(method, &json!({})).unwrap().is_none());
    }
    for field in ["requestState", "inputResponses"] {
        assert!(
            Key::new("resources/read", &json!({field:null}))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn ttl_expiry_missing_negative_and_invalid_hints_are_handled_without_sleeping() {
    let mut cache = Cache::default();
    let item = key("tools/list", json!({}));
    let result = json!({"resultType":"complete","ttlMs":5000,"cacheScope":"private"});
    cache
        .put(
            item.clone(),
            &result,
            [0; 3],
            Instant::now() - Duration::from_secs(6),
        )
        .unwrap();
    assert!(cache.get(&item, [0; 3]).is_none());
    assert_eq!(cache.bytes, 0);
    for hints in [
        json!({"ttlMs":0,"cacheScope":"private"}),
        json!({"ttlMs":-1,"cacheScope":"private"}),
        json!({"cacheScope":"private"}),
        json!({"ttlMs":1000}),
    ] {
        let mut result = json!({"resultType":"complete"});
        result
            .as_object_mut()
            .unwrap()
            .extend(hints.as_object().unwrap().clone());
        cache
            .put(item.clone(), &result, [0; 3], Instant::now())
            .unwrap();
        assert!(cache.get(&item, [0; 3]).is_none());
    }
    for result in [
        json!({"resultType":"complete","ttlMs":1.5,"cacheScope":"private"}),
        json!({"resultType":"complete","ttlMs":5000,"cacheScope":"global"}),
    ] {
        assert_eq!(
            cache.put(item.clone(), &result, [0; 3], Instant::now()),
            Err(Error::Protocol)
        );
    }
    cache
        .put(item.clone(), &result, [0; 3], Instant::now())
        .unwrap();
    assert!(cache.get(&item, [1, 0, 0]).is_none());
}

#[test]
fn byte_and_entry_limits_evict_and_invalidation_releases_accounting() {
    let mut cache = Cache::default();
    for n in 0..70 {
        cache.put(key("resources/read",json!({"uri":format!("repo://{n}")})),&json!({"resultType":"complete","ttlMs":60000,"cacheScope":"private","text":"x".repeat(130000)}),[0;3],Instant::now()).unwrap();
        assert!(cache.bytes <= MAX_BYTES && cache.entries.len() <= MAX_ENTRIES);
    }
    assert!(
        cache
            .get(&key("resources/read", json!({"uri":"repo://0"})), [0; 3])
            .is_none()
    );
    assert!(
        cache
            .get(&key("resources/read", json!({"uri":"repo://69"})), [0; 3])
            .is_some()
    );
    cache.invalidate("resources/read");
    assert_eq!(cache.bytes, 0);
    assert!(cache.entries.is_empty());
}

#[test]
fn each_catalog_page_has_an_independent_ttl_and_cursor_key() {
    let mut second = tool();
    second.name = "second".into();
    let mock = Mock::new(vec![
        discover(),
        page(json!([tool()]), 60000, "private", Some("next")),
        page(json!([second.clone()]), 0, "private", None),
        page(json!([second]), 0, "private", None),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert_eq!(client.tools().unwrap().len(), 2);
    assert_eq!(client.tools().unwrap().len(), 2);
    let requests = mock.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3].body["params"]["cursor"], "next");
}

#[test]
fn resource_cache_hits_expire_on_access_and_never_cross_uri_parameters() {
    let mock = Mock::new(vec![
        resource_discover(),
        resource("first", "private"),
        resource("other", "private"),
        resource("new", "private"),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "first"
    );
    assert_eq!(
        value(client.read_resource("repo://b").unwrap())["contents"][0]["text"],
        "other"
    );
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "first"
    );
    if let Some(Transport::Modern(modern)) = &mut client.transport {
        modern
            .cache
            .entries
            .get_mut(&key("resources/read", json!({"uri":"repo://a"})))
            .unwrap()
            .received = Instant::now() - Duration::from_secs(61);
    }
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "new"
    );
    assert_eq!(mock.finish().len(), 4);
}

#[test]
fn public_and_private_results_are_both_isolated_between_authenticated_clients() {
    for scope in ["private", "public"] {
        let mock = Mock::new(vec![
            resource_discover(),
            resource("first-user", scope),
            resource_discover(),
            resource("second-user", scope),
        ]);
        let mut first = Client::connect(&mock.url, "first").unwrap();
        first.read_resource("repo://a").unwrap();
        let mut second = Client::connect(&mock.url, "second").unwrap();
        assert_eq!(
            value(second.read_resource("repo://a").unwrap())["contents"][0]["text"],
            "second-user"
        );
        assert_eq!(
            value(first.read_resource("repo://a").unwrap())["contents"][0]["text"],
            "first-user"
        );
        assert_eq!(mock.finish().len(), 4);
    }
}

#[test]
fn mrtr_complete_result_is_not_reused_for_a_new_resource_operation() {
    let mock = Mock::new(vec![
        resource_discover(),
        Reply::result(
            "resources/read",
            json!({"resultType":"input_required","requestState":"opaque","ttlMs":60000,"cacheScope":"private"}),
        ),
        resource("continued", "private"),
        resource("fresh", "private"),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let pending = match client.read_resource("repo://a").unwrap() {
        Outcome::InputRequired(pending) => pending,
        _ => panic!("pending expected"),
    };
    assert_eq!(
        value(client.continue_request(pending, BTreeMap::new()).unwrap())["contents"][0]["text"],
        "continued"
    );
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "fresh"
    );
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "fresh"
    );
    assert_eq!(mock.finish().len(), 4);
}

#[test]
fn tool_calls_are_never_cached_and_explicit_refresh_bypasses_catalog_cache() {
    let reply = || {
        Reply::result(
            "tools/call",
            json!({"resultType":"complete","content":[],"ttlMs":60000,"cacheScope":"public"}),
        )
    };
    let mock = Mock::new(vec![
        discover(),
        page(json!([tool()]), 60000, "private", None),
        reply(),
        reply(),
        page(json!([tool()]), 60000, "private", None),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    client.call(&catalog[0], json!({})).unwrap();
    client.call(&catalog[0], json!({})).unwrap();
    client.tools().unwrap();
    client.refresh_tools().unwrap();
    assert_eq!(mock.finish().len(), 5);
}

#[test]
fn resource_notification_invalidates_a_fresh_cached_result() {
    let id = 2;
    let meta = json!({"io.modelcontextprotocol/subscriptionId":id});
    let ack = json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{"_meta":meta,"notifications":{"resourceSubscriptions":["repo://a"]}}});
    let updated = json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"_meta":meta,"uri":"repo://a"}});
    let mock = Mock::new(vec![
        resource_discover(),
        Reply::result("subscriptions/listen", Value::Null)
            .sse()
            .raw(&format!("data: {ack}\n\ndata: {updated}\n\n")),
        resource("before", "private"),
        resource("after", "private"),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let mut stream = client
        .listen(SubscriptionFilter {
            resource_subscriptions: vec!["repo://a".into()],
            ..Default::default()
        })
        .unwrap();
    client.read_resource("repo://a").unwrap();
    client.read_resource("repo://a").unwrap();
    assert_eq!(
        stream.next_event(),
        Ok(Some(Change::Resource {
            uri: "repo://a".into()
        }))
    );
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "after"
    );
    assert_eq!(mock.finish().len(), 4);
}

#[test]
fn conflicting_page_scopes_discard_all_cached_pages_and_require_a_fresh_start() {
    let mut second = tool();
    second.name = "second".into();
    let mock = Mock::new(vec![
        discover(),
        page(json!([tool()]), 60000, "private", Some("next")),
        page(json!([second]), 60000, "public", None),
        page(json!([tool()]), 60000, "private", None),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert!(matches!(client.tools(), Err(Error::Protocol)));
    assert_eq!(client.tools().unwrap().len(), 1);
    let requests = mock.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].body["params"].get("cursor").is_none());
}

#[test]
fn observed_authentication_failure_and_expired_client_cannot_serve_cached_reads() {
    let mock = Mock::new(vec![
        resource_discover(),
        resource("private", "private"),
        Reply::result("resources/read", Value::Null).status(401),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    client.read_resource("repo://a").unwrap();
    assert!(matches!(
        client.read_resource("repo://b"),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        client.read_resource("repo://a"),
        Err(Error::Unauthorized)
    ));
    assert_eq!(mock.finish().len(), 3);
    let mock = Mock::new(vec![resource_discover(), resource("private", "private")]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    client.read_resource("repo://a").unwrap();
    if let Some(Transport::Modern(modern)) = &mut client.transport {
        modern.started = Instant::now() - Duration::from_secs(91);
    }
    assert!(matches!(
        client.read_resource("repo://a"),
        Err(Error::Transport)
    ));
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn discovery_uses_its_own_key_and_invalid_shapes_are_not_retained() {
    let result = json!({"resultType":"complete","supportedVersions":[metadata::VERSION],"capabilities":{"tools":{}},"ttlMs":60000,"cacheScope":"private"});
    let mock = Mock::new(vec![Reply::result("server/discover", result.clone())]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert_eq!(client.discover().unwrap(), result);
    assert_eq!(mock.finish().len(), 1);
    let mock = Mock::new(vec![
        resource_discover(),
        resource("old", "private"),
        Reply::result(
            "server/discover",
            json!({"resultType":"complete","supportedVersions":[],"capabilities":{},"ttlMs":60000,"cacheScope":"private"}),
        ),
        resource_discover(),
        resource("new", "private"),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    client.read_resource("repo://a").unwrap();
    assert_eq!(client.discover(), Err(Error::Protocol));
    assert!(matches!(
        client.read_resource("repo://a"),
        Err(Error::Unsupported)
    ));
    client.discover().unwrap();
    assert_eq!(
        value(client.read_resource("repo://a").unwrap())["contents"][0]["text"],
        "new"
    );
    assert_eq!(mock.finish().len(), 5);
}

#[test]
fn subscription_authentication_failure_also_revokes_cached_resource_reads() {
    let mock = Mock::new(vec![
        resource_discover(),
        resource("private", "private"),
        Reply::result("subscriptions/listen", Value::Null).status(401),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    client.read_resource("repo://a").unwrap();
    assert!(matches!(
        client.listen(SubscriptionFilter {
            tools_list_changed: true,
            ..Default::default()
        }),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        client.read_resource("repo://a"),
        Err(Error::Unauthorized)
    ));
    assert_eq!(mock.finish().len(), 3);
}

#[test]
fn schema_mismatch_discards_cached_catalog_without_automatically_repeating_the_tool() {
    let mock = Mock::new(vec![
        discover(),
        page(json!([tool()]), 60000, "private", None),
        Reply::result("tools/call", Value::Null)
            .status(400)
            .error(-32020),
        page(json!([tool()]), 60000, "private", None),
        Reply::result("tools/call", json!({"resultType":"complete","content":[]})),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let old = client.tools().unwrap();
    assert_eq!(client.call(&old[0], json!({})), Err(Error::Rpc(-32020)));
    assert_eq!(client.call(&old[0], json!({})), Err(Error::Unsupported));
    let current = client.tools().unwrap();
    client.call(&current[0], json!({})).unwrap();
    assert_eq!(mock.finish().len(), 5);
}
