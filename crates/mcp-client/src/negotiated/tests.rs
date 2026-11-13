use super::*;
use crate::tests::{Mock, Reply, Request};

#[test]
fn application_guard_fences_cached_catalog_and_cannot_revive_after_regrant() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = crate::DispatchGuard::new(Arc::new({
        let allowed = allowed.clone();
        move || allowed.load(Ordering::SeqCst)
    }));
    let cached = Reply::result(
        "tools/list",
        json!({"resultType":"complete","tools":[tool()],"ttlMs":60000,"cacheScope":"private"}),
    );
    let mock = Mock::new(vec![discover(), cached]);
    let mut client = Client::connect_authorized(&mock.url, "token", guard).unwrap();
    client.tools().unwrap();
    allowed.store(false, Ordering::SeqCst);
    assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
    assert!(client.discovery().is_none());
    allowed.store(true, Ordering::SeqCst);
    assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
    assert_eq!(
        client.call(&tool(), json!({})).unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn application_guard_applies_to_initial_probe_and_legacy_dispatch_and_cleanup() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mock = Mock::new(vec![]);
    let guard = crate::DispatchGuard::new(Arc::new(|| false));
    assert!(matches!(
        Client::connect_authorized(&mock.url, "token", guard),
        Err(Error::Unauthorized)
    ));
    assert!(mock.finish().is_empty());
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = crate::DispatchGuard::new(Arc::new({
        let allowed = allowed.clone();
        move || allowed.load(Ordering::SeqCst)
    }));
    let mock = Mock::new(vec![
        Reply::result("server/discover", Value::Null)
            .status(400)
            .raw("legacy"),
        Reply::initialize().session(),
        Reply::notification(),
    ]);
    let mut client = Client::connect_authorized(&mock.url, "token", guard).unwrap();
    allowed.store(false, Ordering::SeqCst);
    assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
    assert_eq!(client.close().unwrap_err(), Error::Unauthorized);
    assert_eq!(mock.finish().len(), 3);
}

pub(super) fn discover() -> Reply {
    Reply::result(
        "server/discover",
        json!({"resultType":"complete","supportedVersions":[metadata::VERSION],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"untrusted","version":"1"}},"ttlMs":0,"cacheScope":"private"}),
    )
}

#[test]
fn revocation_during_http_discards_modern_results_and_stops_legacy_initialization() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for legacy in [false, true] {
        let allowed = Arc::new(AtomicBool::new(true));
        let guard = crate::DispatchGuard::new(Arc::new({
            let allowed = allowed.clone();
            move || allowed.load(Ordering::SeqCst)
        }));
        let revoke = {
            let allowed = allowed.clone();
            move || allowed.store(false, Ordering::SeqCst)
        };
        let replies = if legacy {
            vec![
                Reply::result("server/discover", Value::Null)
                    .status(400)
                    .raw("legacy"),
                Reply::initialize().before_reply(revoke),
            ]
        } else {
            vec![discover(), tools(&[tool()]).before_reply(revoke)]
        };
        let mock = Mock::new(replies);
        if legacy {
            assert!(matches!(
                Client::connect_authorized(&mock.url, "token", guard),
                Err(Error::Unauthorized)
            ));
        } else {
            let mut client = Client::connect_authorized(&mock.url, "token", guard).unwrap();
            assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
            allowed.store(true, Ordering::SeqCst);
            assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
        }
        assert_eq!(mock.finish().len(), 2);
    }
}
pub(super) fn tool() -> Tool {
    Tool {
        output_schema: None,
        name: "lookup".into(),
        input_schema: json!({"type":"object","properties":{"tenant":{"type":"string","x-mcp-header":"Tenant"}}}),
    }
}
pub(super) fn tools(tools: &[Tool]) -> Reply {
    Reply::result(
        "tools/list",
        json!({"resultType":"complete","tools":tools,"ttlMs":0,"cacheScope":"private"}),
    )
}
pub(super) fn success() -> Reply {
    Reply::result(
        "tools/call",
        json!({"resultType":"complete","content":[{"type":"text","text":"ok"}]}),
    )
}

#[test]
fn arguments_and_outputs_use_pinned_schemas_and_invalid_input_sends_no_http() {
    let mut item = tool();
    item.input_schema = json!({"type":"object","properties":{"n":{"type":"integer","minimum":1}},"required":["n"],"additionalProperties":false});
    item.output_schema = Some(
        json!({"type":"object","properties":{"total":{"type":"integer"}},"required":["total"]}),
    );
    let mock = Mock::new(vec![
        discover(),
        tools(std::slice::from_ref(&item)),
        Reply::result(
            "tools/call",
            json!({"resultType":"complete","structuredContent":{"total":2}}),
        ),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    for arguments in [
        json!({}),
        json!({"n":"1"}),
        json!({"n":0}),
        json!({"n":1,"extra":true}),
    ] {
        assert_eq!(
            client.call(&catalog[0], arguments).unwrap_err(),
            Error::Arguments
        );
    }
    let mut forged = catalog[0].clone();
    forged.output_schema = None;
    assert_eq!(
        client.call(&forged, json!({"n":1})).unwrap_err(),
        Error::Unsupported
    );
    client.call(&catalog[0], json!({"n":1})).unwrap();
    assert_eq!(mock.finish().len(), 3);
}

#[test]
fn failed_output_validation_never_retries_and_unresolvable_schemas_are_not_exposed() {
    let mut item = tool();
    item.output_schema = Some(
        json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}),
    );
    for structured in [json!({"value":"private-provider-content"}), Value::Null] {
        let mock = Mock::new(vec![
            discover(),
            tools(std::slice::from_ref(&item)),
            Reply::result(
                "tools/call",
                json!({"resultType":"complete","structuredContent":structured}),
            ),
        ]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let catalog = client.tools().unwrap();
        assert_eq!(
            client.call(&catalog[0], json!({})).unwrap_err(),
            Error::Protocol
        );
        assert_eq!(mock.finish().len(), 3);
    }
    let mut external = tool();
    external.name = "external".into();
    external.input_schema = json!({"$ref":"file:///etc/passwd"});
    let mock = Mock::new(vec![discover(), tools(&[external.clone(), tool()])]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert_eq!(client.tools().unwrap().len(), 1);
    assert_eq!(
        client.call(&external, json!({})).unwrap_err(),
        Error::Unsupported
    );
    assert_eq!(mock.finish().len(), 2);
}
pub(super) fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[test]
fn modern_discovery_sse_and_tool_headers_have_no_initialize_or_protocol_session() {
    let mut item = tool();
    item.name = "lookup 世界".into();
    let mock = Mock::new(vec![
        discover().session(),
        tools(&[item]).sse(),
        success().sse(),
    ]);
    let mut client = Client::connect(&mock.url, "fixture-bearer").unwrap();
    assert_eq!(client.protocol_version(), Some(metadata::VERSION));
    assert_eq!(
        client.discovery().unwrap()["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "untrusted"
    );
    let catalog = client.tools().unwrap();
    let result = client
        .call(&catalog[0], json!({"tenant":" padded 世界\r\n"}))
        .unwrap();
    assert_eq!(result["content"][0]["text"], "ok");
    client.close().unwrap();
    client.close().unwrap();
    assert_eq!(client.protocol_version(), None);
    assert!(client.tools().is_err());
    let requests = mock.finish();
    assert_eq!(requests.len(), 3);
    for (index, request) in requests.iter().enumerate() {
        assert!(request.headers.starts_with("POST "));
        assert_eq!(header(request, "Mcp-Session-Id"), None);
        assert_eq!(
            header(request, "MCP-Protocol-Version"),
            Some(metadata::VERSION)
        );
        assert_eq!(
            header(request, "Mcp-Method"),
            request.body["method"].as_str()
        );
        assert_eq!(
            header(request, "Authorization"),
            Some("Bearer fixture-bearer")
        );
        assert_eq!(request.body["id"], index + 1);
        assert_eq!(
            request.body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            metadata::VERSION
        );
        assert_eq!(
            request.body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
    }
    assert_eq!(
        header(&requests[2], "Mcp-Name"),
        Some(metadata::header_value("lookup 世界").unwrap().as_str())
    );
    assert_eq!(
        header(&requests[2], "Mcp-Param-Tenant"),
        Some(metadata::header_value(" padded 世界\r\n").unwrap().as_str())
    );
}

#[test]
fn unrecognized_400_selects_the_legacy_adapter_once_and_preserves_session_semantics() {
    let mock = Mock::new(vec![
        Reply::result("server/discover", Value::Null)
            .status(400)
            .raw("legacy initialize required"),
        Reply::initialize().session(),
        Reply::notification(),
        Reply::result("tools/list", json!({"tools":[tool()]})),
        Reply::result("tools/call", json!({"content":[]})),
        Reply::result("DELETE", Value::Null).status(204),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    assert_eq!(client.protocol_version(), Some(crate::VERSION));
    assert!(client.discovery().is_none());
    let catalog = client.tools().unwrap();
    client.call(&catalog[0], json!({})).unwrap();
    client.close().unwrap();
    let requests = mock.finish();
    assert_eq!(requests.len(), 6);
    assert_eq!(requests[0].body["method"], "server/discover");
    assert_eq!(requests[1].body["method"], "initialize");
    for request in &requests[2..] {
        assert_eq!(header(request, "Mcp-Session-Id"), Some("fixture-session"));
        assert_eq!(
            header(request, "MCP-Protocol-Version"),
            Some(crate::VERSION)
        );
    }
    assert!(requests[5].headers.starts_with("DELETE "));
}

#[test]
fn recognized_modern_errors_and_authentication_never_trigger_downgrade() {
    for code in [-32022, -32021, -32020] {
        let mock = Mock::new(vec![
            Reply::result("server/discover", Value::Null)
                .status(400)
                .error(code),
        ]);
        assert!(matches!(Client::connect(&mock.url,"token"),Err(Error::Rpc(found)) if found==code));
        assert_eq!(mock.finish().len(), 1);
    }
    for status in [401, 403, 302, 405] {
        let mock = Mock::new(vec![
            Reply::result("server/discover", Value::Null).status(status),
        ]);
        let expected = if matches!(status, 401 | 403) {
            Error::Unauthorized
        } else {
            Error::Http(status)
        };
        assert!(matches!(Client::connect(&mock.url,"token"),Err(error) if error==expected));
        assert_eq!(mock.finish().len(), 1);
    }
    let mock = Mock::new(vec![
        Reply::result("server/discover", Value::Null)
            .status(404)
            .error(-32601),
    ]);
    assert!(matches!(
        Client::connect(&mock.url, "token"),
        Err(Error::Rpc(-32601))
    ));
    assert_eq!(mock.finish().len(), 1);
    let mock = Mock::new(vec![
        Reply::result("server/discover", Value::Null)
            .status(400)
            .raw(r#"{"jsonrpc":"2.0","id":999,"error":{"code":-32020}}"#),
    ]);
    assert!(matches!(
        Client::connect(&mock.url, "token"),
        Err(Error::Protocol)
    ));
    assert_eq!(mock.finish().len(), 1);
}

#[test]
fn invalid_header_tool_is_excluded_and_forged_schemas_cannot_change_projection() {
    let mut bad = tool();
    bad.name = "bad".into();
    bad.input_schema = json!({"properties":{"x":{"type":"number","x-mcp-header":"X"}}});
    let mock = Mock::new(vec![discover(), tools(&[bad.clone(), tool()]), success()]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(client.call(&bad, json!({})), Err(Error::Unsupported));
    let mut forged = catalog[0].clone();
    forged.input_schema = json!({});
    assert_eq!(client.call(&forged, json!({})), Err(Error::Unsupported));
    client.call(&catalog[0], json!({"tenant":"eu"})).unwrap();
    assert_eq!(mock.finish().len(), 3);
}

#[test]
fn catalog_refresh_is_atomic_and_failure_invalidates_the_previous_snapshot() {
    let mut page = json!({"resultType":"complete","tools":[tool()],"nextCursor":"next"});
    let mut second = tool();
    second.name = "second".into();
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        Reply::result("tools/list", page.clone()),
        Reply::result("tools/list", {
            page["tools"] = json!([second]);
            page
        }),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let old = client.tools().unwrap();
    assert!(matches!(client.tools(), Err(Error::Protocol)));
    assert_eq!(client.call(&old[0], json!({})), Err(Error::Unsupported));
    let requests = mock.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3].body["params"]["cursor"], "next");
}

#[test]
fn tool_errors_input_required_and_missing_result_types_are_not_success_or_retry() {
    for (reply, expected) in [
        (
            Reply::result(
                "tools/call",
                json!({"resultType":"input_required","requestState":"opaque"}),
            ),
            Error::Unsupported,
        ),
        (
            Reply::result("tools/call", json!({"content":[]})),
            Error::Protocol,
        ),
        (
            Reply::result(
                "tools/call",
                json!({"resultType":"complete","isError":true}),
            ),
            Error::ToolFailed,
        ),
        (
            Reply::result("tools/call", Value::Null)
                .status(400)
                .error(-32020),
            Error::Rpc(-32020),
        ),
    ] {
        let mock = Mock::new(vec![discover(), tools(&[tool()]), reply]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let catalog = client.tools().unwrap();
        assert_eq!(client.call(&catalog[0], json!({})), Err(expected));
        assert_eq!(mock.finish().len(), 3);
    }
}

#[test]
fn an_expired_total_budget_stops_before_another_network_operation() {
    let mock = Mock::new(vec![discover()]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    if let Some(Transport::Modern(modern)) = &mut client.transport {
        modern.started = Instant::now() - Duration::from_secs(91);
    } else {
        panic!("modern transport required");
    }
    assert!(matches!(client.tools(), Err(Error::Transport)));
    assert_eq!(mock.finish().len(), 1);
}
