use super::*;
use crate::{
    negotiated::{
        Client, Transport,
        tests::{discover, header, success, tool, tools},
    },
    tests::{Mock, Reply},
};

fn waiting(state: &str) -> Reply {
    Reply::result(
        "tools/call",
        json!({"resultType":"input_required","requestState":state}),
    )
}
fn pending(result: Result<Outcome, Error>) -> Pending {
    match result.unwrap() {
        Outcome::InputRequired(pending) => pending,
        Outcome::Complete(_) => panic!("expected suspended operation"),
    }
}
fn url_input(url: &str) -> Reply {
    Reply::result(
        "tools/call",
        json!({"resultType":"input_required","inputRequests":{"review":{"method":"elicitation/create","params":{"mode":"url","message":"Review third-party connection","url":url}}}}),
    )
}

fn form_input(omit_mode: bool) -> Reply {
    let mut params = json!({"mode":"form","message":"Review the requested profile","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2},"tags":{"type":"array","items":{"type":"string","enum":["a","b"]}}},"required":["name"]}});
    if omit_mode {
        params.as_object_mut().unwrap().remove("mode");
    }
    Reply::result(
        "tools/call",
        json!({"resultType":"input_required","requestState":"opaque-form-state","inputRequests":{"profile":{"method":"elicitation/create","params":params}}}),
    )
}
#[test]
fn form_mode_is_opt_in_and_typed_user_content_is_validated_before_continuation() {
    for omit_mode in [false, true] {
        let mock = Mock::new(vec![
            discover(),
            tools(&[tool()]),
            form_input(omit_mode),
            success(),
        ]);
        let mut client = Client::connect_with_input_support(
            &mock.url,
            "token",
            InputSupport {
                elicitation_form: true,
                ..Default::default()
            },
        )
        .unwrap();
        let catalog = client.tools().unwrap();
        let suspended = pending(client.begin_call(&catalog[0], json!({})));
        assert!(suspended.input_requests().is_empty());
        assert_eq!(
            suspended.form_requests()["profile"].message,
            "Review the requested profile"
        );
        let content = json!({"name":"User","tags":["b"]});
        assert!(matches!(
            client.continue_with_responses(
                suspended,
                BTreeMap::from([(
                    "profile".into(),
                    ElicitationResponse::Form {
                        action: ElicitationAction::Accept,
                        content: Some(content.clone())
                    }
                )])
            ),
            Ok(Outcome::Complete(_))
        ));
        let requests = mock.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests[3].body["params"]["inputResponses"]["profile"],
            json!({"action":"accept","content":content})
        );
        assert_eq!(
            requests[3].body["params"]["requestState"],
            "opaque-form-state"
        );
        for request in requests {
            assert_eq!(
                request.body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
                json!({"elicitation":{"form":{}}})
            );
        }
    }
    let mock = Mock::new(vec![discover(), tools(&[tool()]), form_input(true)]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    assert!(matches!(
        client.begin_call(&catalog[0], json!({})),
        Err(Error::Unsupported)
    ));
    assert_eq!(mock.finish().len(), 3);
}
#[test]
fn invalid_form_content_or_url_shaped_decisions_never_dispatch_a_continuation() {
    for response in [
        ElicitationResponse::Form {
            action: ElicitationAction::Accept,
            content: None,
        },
        ElicitationResponse::Form {
            action: ElicitationAction::Accept,
            content: Some(json!({"name":"x"})),
        },
        ElicitationResponse::Form {
            action: ElicitationAction::Accept,
            content: Some(json!({"name":"ok","extra":"private"})),
        },
        ElicitationResponse::Form {
            action: ElicitationAction::Decline,
            content: Some(json!({"name":"private"})),
        },
        ElicitationResponse::Url(ElicitationAction::Accept),
    ] {
        let mock = Mock::new(vec![discover(), tools(&[tool()]), form_input(false)]);
        let mut client = Client::connect_with_input_support(
            &mock.url,
            "token",
            InputSupport {
                elicitation_form: true,
                ..Default::default()
            },
        )
        .unwrap();
        let catalog = client.tools().unwrap();
        let suspended = pending(client.begin_call(&catalog[0], json!({})));
        assert!(
            client
                .continue_with_responses(suspended, BTreeMap::from([("profile".into(), response)]))
                .is_err()
        );
        assert_eq!(mock.finish().len(), 3);
    }
}
#[test]
fn forms_can_be_declined_or_cancelled_without_sending_content() {
    for action in [ElicitationAction::Decline, ElicitationAction::Cancel] {
        let mock = Mock::new(vec![
            discover(),
            tools(&[tool()]),
            form_input(false),
            success(),
        ]);
        let mut client = Client::connect_with_input_support(
            &mock.url,
            "token",
            InputSupport {
                elicitation_form: true,
                ..Default::default()
            },
        )
        .unwrap();
        let catalog = client.tools().unwrap();
        let suspended = pending(client.begin_call(&catalog[0], json!({})));
        assert!(matches!(
            client.continue_with_responses(
                suspended,
                BTreeMap::from([(
                    "profile".into(),
                    ElicitationResponse::Form {
                        action,
                        content: None
                    }
                )])
            ),
            Ok(Outcome::Complete(_))
        ));
        let requests = mock.finish();
        assert_eq!(
            requests[3].body["params"]["inputResponses"]["profile"],
            json!({"action":action})
        );
    }
}

#[test]
fn mixed_modes_need_exact_typed_responses_and_forms_cannot_bypass_revocation() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let params = json!({"mode":"form","message":"Review","requestedSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}});
    let mixed = Reply::result(
        "tools/call",
        json!({"resultType":"input_required","inputRequests":{
            "form":{"method":"elicitation/create","params":params},
            "url":{"method":"elicitation/create","params":{"mode":"url","message":"Review the host","url":"https://example.test/review"}}
        }}),
    );
    let mock = Mock::new(vec![discover(), tools(&[tool()]), mixed, success()]);
    let mut client = Client::connect_with_input_support(
        &mock.url,
        "token",
        InputSupport {
            elicitation_url: true,
            elicitation_form: true,
        },
    )
    .unwrap();
    let catalog = client.tools().unwrap();
    let suspended = pending(client.begin_call(&catalog[0], json!({})));
    assert_eq!(suspended.form_requests().len(), 1);
    assert_eq!(suspended.input_requests().len(), 1);
    let responses = BTreeMap::from([
        (
            "form".into(),
            ElicitationResponse::Form {
                action: ElicitationAction::Accept,
                content: Some(json!({"name":"Human"})),
            },
        ),
        (
            "url".into(),
            ElicitationResponse::Url(ElicitationAction::Decline),
        ),
    ]);
    assert!(matches!(
        client.continue_with_responses(suspended, responses),
        Ok(Outcome::Complete(_))
    ));
    let requests = mock.finish();
    assert_eq!(
        requests[3].body["params"]["inputResponses"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = crate::DispatchGuard::new(Arc::new({
        let allowed = allowed.clone();
        move || allowed.load(Ordering::SeqCst)
    }));
    let mock = Mock::new(vec![discover(), tools(&[tool()]), form_input(false)]);
    let mut client = Client::connect_authorized_with_input_support(
        &mock.url,
        "token",
        InputSupport {
            elicitation_form: true,
            ..Default::default()
        },
        guard,
    )
    .unwrap();
    let catalog = client.tools().unwrap();
    let suspended = pending(client.begin_call(&catalog[0], json!({})));
    allowed.store(false, Ordering::SeqCst);
    assert!(matches!(
        client.continue_with_responses(
            suspended,
            BTreeMap::from([(
                "profile".into(),
                ElicitationResponse::Form {
                    action: ElicitationAction::Accept,
                    content: Some(json!({"name":"Human"}))
                }
            )])
        ),
        Err(Error::Unauthorized)
    ));
    assert_eq!(mock.finish().len(), 3);
}

#[test]
fn opaque_state_is_echoed_only_for_the_exact_original_operation_with_new_rpc_id() {
    let state = "opaque:not-json\0世界";
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        waiting(state),
        success(),
        success(),
    ]);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    let suspended = pending(client.begin_call(&catalog[0], json!({"tenant":"first"})));
    assert!(suspended.input_requests().is_empty());
    client.call(&catalog[0], json!({"tenant":"other"})).unwrap();
    assert!(matches!(
        client.continue_request(suspended, BTreeMap::new()),
        Ok(Outcome::Complete(_))
    ));
    let requests = mock.finish();
    assert_eq!(requests.len(), 5);
    assert!(requests[3].body["params"].get("requestState").is_none());
    assert_eq!(requests[4].body["params"]["requestState"], state);
    assert_eq!(
        requests[4].body["params"]["arguments"],
        requests[2].body["params"]["arguments"]
    );
    assert_ne!(requests[4].body["id"], requests[2].body["id"]);
    assert!(requests[4].body["params"].get("inputResponses").is_none());
    assert_eq!(header(&requests[4], "Mcp-Param-Tenant"), Some("first"));
}

#[test]
fn explicit_url_input_declares_only_that_mode_and_sends_the_users_decision_without_state() {
    for action in [
        ElicitationAction::Accept,
        ElicitationAction::Decline,
        ElicitationAction::Cancel,
    ] {
        let mock = Mock::new(vec![
            discover(),
            tools(&[tool()]),
            url_input("https://third-party.example/review?task=1"),
            success(),
        ]);
        let mut client = Client::connect_with_input_support(
            &mock.url,
            "original-bearer",
            InputSupport {
                elicitation_form: false,
                elicitation_url: true,
            },
        )
        .unwrap();
        let catalog = client.tools().unwrap();
        let suspended = pending(client.begin_call(&catalog[0], json!({})));
        assert_eq!(
            suspended.input_requests()["review"].url,
            "https://third-party.example/review?task=1"
        );
        assert!(matches!(
            client.continue_request(suspended, BTreeMap::from([("review".into(), action)])),
            Ok(Outcome::Complete(_))
        ));
        let requests = mock.finish();
        assert_eq!(
            requests[3].body["params"]["inputResponses"]["review"],
            json!({"action":action})
        );
        assert!(requests[3].body["params"].get("requestState").is_none());
        for request in &requests {
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer original-bearer")
            );
            assert_eq!(
                request.body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
                json!({"elicitation":{"url":{}}})
            );
        }
    }
}

#[test]
fn pending_handle_cannot_cross_authenticated_clients_even_at_the_same_endpoint() {
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        waiting("opaque"),
        discover(),
    ]);
    let mut first = Client::connect(&mock.url, "first-user").unwrap();
    let catalog = first.tools().unwrap();
    let suspended = pending(first.begin_call(&catalog[0], json!({})));
    let mut second = Client::connect(&mock.url, "second-user").unwrap();
    assert!(matches!(
        second.continue_request(suspended, BTreeMap::new()),
        Err(Error::Protocol)
    ));
    assert_eq!(mock.finish().len(), 4);
}

#[test]
fn resource_and_prompt_continuations_preserve_their_own_method_and_parameters() {
    for (method, params) in [
        (
            "prompts/get",
            json!({"name":"review","arguments":{"branch":"main"}}),
        ),
        ("resources/read", json!({"uri":"repo://project/report"})),
    ] {
        let caps = Reply::result(
            "server/discover",
            json!({"resultType":"complete","supportedVersions":[crate::metadata::VERSION],"capabilities":{"resources":{},"prompts":{}}}),
        );
        let mock = Mock::new(vec![
            caps,
            Reply::result(
                method,
                json!({"resultType":"input_required","requestState":"opaque"}),
            ),
            Reply::result(method, json!({"resultType":"complete"})),
        ]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let suspended = pending(if method == "prompts/get" {
            client.get_prompt("review", json!({"branch":"main"}))
        } else {
            client.read_resource("repo://project/report")
        });
        assert!(matches!(
            client.continue_request(suspended, BTreeMap::new()),
            Ok(Outcome::Complete(_))
        ));
        let requests = mock.finish();
        for (key, value) in params.as_object().unwrap() {
            assert_eq!(requests[2].body["params"][key], *value);
        }
        assert_eq!(requests[2].body["method"], method);
        assert_eq!(requests[2].body["params"]["requestState"], "opaque");
    }
}

#[test]
fn pending_admission_is_bounded_before_network_and_released_by_drop() {
    let mut replies = vec![discover(), tools(&[tool()])];
    replies.extend((0..MAX_PENDING).map(|_| waiting("opaque")));
    replies.push(waiting("second-round"));
    replies.push(success());
    let mock = Mock::new(replies);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    let mut held = Vec::new();
    for _ in 0..MAX_PENDING {
        held.push(pending(client.begin_call(&catalog[0], json!({}))));
    }
    assert!(matches!(
        client.begin_call(&catalog[0], json!({})),
        Err(Error::Limit)
    ));
    // A continuation retains its slot even when every other slot is occupied.
    let resumed = pending(client.continue_request(held.pop().unwrap(), BTreeMap::new()));
    held.push(resumed);
    assert!(matches!(
        client.begin_call(&catalog[0], json!({})),
        Err(Error::Limit)
    ));
    drop(held.pop());
    client.call(&catalog[0], json!({})).unwrap();
    drop(held);
    assert_eq!(mock.finish().len(), MAX_PENDING + 4);
}

#[test]
fn each_round_replaces_state_and_input_keys_without_carrying_old_values_forward() {
    let mock = Mock::new(vec![
        discover(),
        tools(&[tool()]),
        waiting("old-opaque"),
        url_input("https://review.example/confirm"),
        success(),
    ]);
    let mut client = Client::connect_with_input_support(
        &mock.url,
        "token",
        InputSupport {
            elicitation_form: false,
            elicitation_url: true,
        },
    )
    .unwrap();
    let catalog = client.tools().unwrap();
    let first = pending(client.begin_call(&catalog[0], json!({"tenant":"fixed"})));
    let second = pending(client.continue_request(first, BTreeMap::new()));
    assert!(matches!(
        client.continue_request(
            second,
            BTreeMap::from([("review".into(), ElicitationAction::Accept)])
        ),
        Ok(Outcome::Complete(_))
    ));
    let requests = mock.finish();
    assert_eq!(requests[3].body["params"]["requestState"], "old-opaque");
    assert!(requests[4].body["params"].get("requestState").is_none());
    assert_eq!(
        requests[4].body["params"]["inputResponses"],
        json!({"review":{"action":"accept"}})
    );
}

#[test]
fn rounds_time_wrong_keys_and_stale_catalog_stop_before_continuation_network() {
    let mut replies = vec![discover(), tools(&[tool()])];
    replies.extend((0..MAX_ROUNDS).map(|_| waiting("opaque")));
    let mock = Mock::new(replies);
    let mut client = Client::connect(&mock.url, "token").unwrap();
    let catalog = client.tools().unwrap();
    let mut suspended = pending(client.begin_call(&catalog[0], json!({})));
    for _ in 1..MAX_ROUNDS {
        suspended = pending(client.continue_request(suspended, BTreeMap::new()));
    }
    assert!(matches!(
        client.continue_request(suspended, BTreeMap::new()),
        Err(Error::Limit)
    ));
    assert_eq!(mock.finish().len(), MAX_ROUNDS as usize + 2);

    for reason in ["expired", "wrong-key", "stale-catalog"] {
        let mock = Mock::new(vec![discover(), tools(&[tool()]), waiting("opaque")]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let catalog = client.tools().unwrap();
        let mut suspended = pending(client.begin_call(&catalog[0], json!({})));
        let mut responses = BTreeMap::new();
        match reason {
            "expired" => suspended.started = Instant::now() - Duration::from_secs(61),
            "wrong-key" => {
                responses.insert("unrequested".into(), ElicitationAction::Accept);
            }
            _ => {
                if let Some(Transport::Modern(modern)) = &mut client.transport {
                    modern.catalog.clear();
                }
            }
        }
        assert!(client.continue_request(suspended, responses).is_err());
        assert_eq!(mock.finish().len(), 3);
    }
}

#[test]
fn invalid_state_unadvertised_requests_and_unsafe_urls_are_not_executed_or_navigated() {
    for result in [
        json!({"resultType":"input_required"}),
        json!({"resultType":"input_required","requestState":{}}),
        json!({"resultType":"input_required","requestState":"x".repeat(MAX_STATE+1)}),
        json!({"resultType":"input_required","inputRequests":{"root":{"method":"roots/list"}}}),
        json!({"resultType":"input_required","inputRequests":{"sample":{"method":"sampling/createMessage"}}}),
    ] {
        let mock = Mock::new(vec![
            discover(),
            tools(&[tool()]),
            Reply::result("tools/call", result),
        ]);
        let mut client = Client::connect(&mock.url, "token").unwrap();
        let catalog = client.tools().unwrap();
        assert!(client.begin_call(&catalog[0], json!({})).is_err());
        assert_eq!(mock.finish().len(), 3);
    }
    for (url, enabled) in [
        ("https://example.com", false),
        ("javascript:alert(1)", true),
        ("file:///secret", true),
        ("http://example.com", true),
        ("https://user:secret@example.com", true),
        ("https://example.com/a b", true),
        ("https://example.com/\\other", true),
        ("https://example.com/\tpath", true),
    ] {
        let mock = Mock::new(vec![discover(), tools(&[tool()]), url_input(url)]);
        let mut client = Client::connect_with_input_support(
            &mock.url,
            "token",
            InputSupport {
                elicitation_form: false,
                elicitation_url: enabled,
            },
        )
        .unwrap();
        let catalog = client.tools().unwrap();
        assert!(client.begin_call(&catalog[0], json!({})).is_err());
        assert_eq!(mock.finish().len(), 3);
    }
}

#[test]
fn concurrent_reservations_never_exceed_the_limit_and_drop_releases_each_slot() {
    let identity = Arc::new(Identity::default());
    let barrier = Arc::new(std::sync::Barrier::new(17));
    let (sender, receiver) = std::sync::mpsc::channel();
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let identity = identity.clone();
            let barrier = barrier.clone();
            let sender = sender.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sender.send(Ticket::acquire(&identity)).ok().unwrap();
            })
        })
        .collect();
    drop(sender);
    barrier.wait();
    let results: Vec<_> = receiver.into_iter().collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        MAX_PENDING
    );
    assert_eq!(identity.pending.load(Ordering::Acquire), MAX_PENDING);
    drop(results);
    assert_eq!(identity.pending.load(Ordering::Acquire), 0);
    drop(Ticket::acquire(&identity).unwrap());
    assert_eq!(identity.pending.load(Ordering::Acquire), 0);
}
