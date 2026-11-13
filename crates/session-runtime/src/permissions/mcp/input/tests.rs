use super::super::super::tests::running;
use super::*;
use crate::tests::{context, setup};

fn invocation() -> McpInvocation {
    McpInvocation {
        provider_id: "work".into(),
        connection_id: "connection".into(),
        connection_revision: 1,
        tool_name: "lookup".into(),
        definition_digest: "a".repeat(64),
        arguments: json!({"query":"fixture"}),
    }
}
fn permit(
    j: &Journal<'_>,
    a: &AccessContext,
    id: &str,
    l: &Lease,
    t: &str,
    key: &str,
) -> McpDispatchPermit {
    let approval = j
        .register_mcp_permission(
            a,
            id,
            l,
            t,
            McpPermissionChallenge {
                request_id: key.into(),
                invocation: invocation(),
                ttl_seconds: 60,
                starter_device: "starter".into(),
                binding_revision: 1,
                harness_digest: "digest".into(),
            },
        )
        .unwrap();
    let p=serde_json::from_value(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/toolCallConfirmed","turnId":t,"toolCallId":approval.tool,"approved":true,"selectedOptionId":format!("permission:{}:0",approval.nonce)}})).unwrap();
    j.dispatch_action(a, id, "device", key, &p, DispatchAdmission::default())
        .unwrap();
    j.consume_mcp_permission(a, id, l, approval, &invocation())
        .unwrap()
        .unwrap()
}
fn challenge() -> McpInputChallenge {
    McpInputChallenge{request:serde_json::from_value(json!({"id":"untrusted-placeholder","message":"MCP work / lookup: choose a value","questions":[{"kind":"text","id":"name","message":"Display name","required":true}]})).unwrap(),schema:json!({"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"]})}
}
fn scope<'a>(p: &'a McpDispatchPermit, i: &'a McpInvocation) -> McpInputScope<'a> {
    McpInputScope {
        permit: p,
        invocation: i,
    }
}
fn answer(
    session: &str,
    input: &str,
    response: &str,
    answers: Option<Value>,
) -> ahp_types::commands::DispatchActionParams {
    let mut action = json!({"type":"chat/inputCompleted","requestId":input,"response":response});
    if let Some(v) = answers {
        action["answers"] = v;
    }
    serde_json::from_value(
        json!({"channel":format!("ahp-chat:/{session}"),"clientSeq":2,"action":action}),
    )
    .unwrap()
}
fn submitted() -> Value {
    json!({"name":{"state":"submitted","value":{"kind":"text","value":"Alice"}}})
}
#[test]
fn mcp_form_input_is_atomic_durable_one_use_and_cannot_resume_after_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, l, t) = running(&j, &a);
    let p = permit(&j, &a, &s.id, &l, &t, "call");
    let i = invocation();
    let input = j
        .register_mcp_input(&a, &s.id, &l, scope(&p, &i), challenge())
        .unwrap();
    assert!(input.id().starts_with("dunlin:mcp-input:"));
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .unwrap()
            .is_none()
    );
    let mut response = answer(&s.id, input.id(), "accept", Some(submitted()));
    // The ordinary dispatcher cannot accept a form without an exact validator.
    assert!(
        j.dispatch_action(
            &a,
            &s.id,
            "responder",
            "browser",
            &response,
            DispatchAdmission::default()
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
    response.client_seq += 1;
    let validate = |schema: &Value, content: &Value| {
        assert_eq!(schema, &challenge().schema);
        assert_eq!(content, &json!({"name":"Alice"}));
        Ok(())
    };
    let echo = j
        .dispatch_action_with_mcp_input(
            &a,
            &s.id,
            "responder",
            "browser",
            &response,
            DispatchAdmission::default(),
            &validate,
        )
        .unwrap();
    assert!(echo.rejection_reason.is_none());
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    let replay = j
        .dispatch_action(
            &a,
            &s.id,
            "responder",
            "browser",
            &response,
            DispatchAdmission {
                rejection: Some("runtime offline"),
                elapsed_ms: None,
            },
        )
        .unwrap();
    assert_eq!(echo, replay);
    let d = j
        .take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
        .unwrap()
        .unwrap();
    assert_eq!(d.response, ChatInputResponseKind::Accept);
    assert_eq!(d.content, Some(json!({"name":"Alice"})));
    assert_eq!(d.device.device_id, "responder");
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .is_err()
    );
    let pending = j
        .register_mcp_input(&a, &s.id, &l, scope(&p, &i), challenge())
        .unwrap();
    j.recover().unwrap();
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &pending)
            .is_err()
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM interactive_mcp_inputs WHERE id=?1",
            [pending.id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invalid"
    );
}
#[test]
fn mcp_form_decline_cancel_and_competing_answers_never_send_content_or_change_the_winner() {
    for response in ["decline", "cancel"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let j = Journal::new(&c);
        let a = context();
        let (s, l, t) = running(&j, &a);
        let p = permit(&j, &a, &s.id, &l, &t, "call");
        let i = invocation();
        let input = j
            .register_mcp_input(&a, &s.id, &l, scope(&p, &i), challenge())
            .unwrap();
        let bad = answer(&s.id, input.id(), response, Some(submitted()));
        assert!(
            j.dispatch_action(
                &a,
                &s.id,
                "device",
                "browser",
                &bad,
                DispatchAdmission::default()
            )
            .unwrap()
            .rejection_reason
            .is_some()
        );
        let mut response = answer(&s.id, input.id(), response, None);
        response.client_seq += 1;
        assert!(
            j.dispatch_action(
                &a,
                &s.id,
                "device",
                "browser",
                &response,
                DispatchAdmission::default()
            )
            .unwrap()
            .rejection_reason
            .is_none()
        );
        let other = answer(&s.id, input.id(), "accept", Some(submitted()));
        assert!(
            j.dispatch_action(
                &a,
                &s.id,
                "other",
                "other",
                &other,
                DispatchAdmission::default()
            )
            .unwrap()
            .rejection_reason
            .is_some()
        );
        let d = j
            .take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .unwrap()
            .unwrap();
        assert!(d.content.is_none());
    }
}
#[test]
fn mcp_form_input_rejects_cross_invocation_schema_failures_drafts_and_cancellation() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, l, t) = running(&j, &a);
    let p = permit(&j, &a, &s.id, &l, &t, "call");
    let mut i = invocation();
    let input = j
        .register_mcp_input(&a, &s.id, &l, scope(&p, &i), challenge())
        .unwrap();
    i.connection_revision += 1;
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .is_err()
    );
    i.connection_revision -= 1;
    for (attempt, answers) in [
        json!({"name":{"state":"draft","value":{"kind":"text","value":"Alice"}}}),
        json!({"extra":{"state":"submitted","value":{"kind":"text","value":"Alice"}}}),
        json!({"name":{"state":"submitted","value":{"kind":"boolean","value":true}}}),
    ]
    .into_iter()
    .enumerate()
    {
        let action = answer(&s.id, input.id(), "accept", Some(answers));
        assert!(
            j.dispatch_action_with_mcp_input(
                &a,
                &s.id,
                "device",
                &format!("invalid-{attempt}"),
                &action,
                DispatchAdmission::default(),
                &|_, _| Ok(())
            )
            .unwrap()
            .rejection_reason
            .is_some()
        );
    }
    let action = answer(&s.id, input.id(), "accept", Some(submitted()));
    assert!(
        j.dispatch_action_with_mcp_input(
            &a,
            &s.id,
            "device",
            "browser",
            &action,
            DispatchAdmission::default(),
            &|_, _| Err(Error::Conflict("Schema rejected"))
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .unwrap()
            .is_none()
    );
    j.interrupt(&s.id, &l, "fixture stop").unwrap();
    assert!(
        j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .is_err()
    );
    let chat = serde_json::to_value(j.projected_chat(&a, &s.id).unwrap()).unwrap();
    assert!(chat.to_string().contains("cancel"));
}

#[test]
fn mcp_form_presentation_allows_expanded_enum_labels_but_keeps_the_wire_bound() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, l, t) = running(&j, &a);
    let p = permit(&j, &a, &s.id, &l, &t, "call");
    let i = invocation();
    let choices = (0..8)
        .map(|n| format!("choice-{n}-{}", "x".repeat(1480)))
        .collect::<Vec<_>>();
    let options = choices
        .iter()
        .map(|v| json!({"id":v,"label":v}))
        .collect::<Vec<_>>();
    let schema = json!({"type":"object","properties":{"value":{"type":"string","enum":choices}},"required":["value"]});
    assert!(schema.to_string().len() < 16 * 1024);
    let request=serde_json::from_value(json!({"id":"assigned-by-host","message":"m".repeat(8100),"questions":[{"kind":"single-select","id":"value","message":"Choose","required":true,"options":options}]})).unwrap();
    let expanded = McpInputChallenge { request, schema };
    assert!(serde_json::to_vec(&expanded.request).unwrap().len() > 24 * 1024);
    j.register_mcp_input(&a, &s.id, &l, scope(&p, &i), expanded)
        .unwrap();
    let projected = serde_json::to_vec(&j.projected_chat(&a, &s.id).unwrap()).unwrap();
    assert!(projected.len() > 24 * 1024 && projected.len() < 64 * 1024);
    let mut oversized = challenge();
    oversized.request.message = Some("m".repeat(49 * 1024));
    assert!(
        j.register_mcp_input(&a, &s.id, &l, scope(&p, &i), oversized)
            .is_err()
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_mcp_inputs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn mcp_url_decisions_never_carry_content_and_keep_one_use_authority_and_recovery() {
    for response in ["accept", "decline", "cancel"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let j = Journal::new(&c);
        let a = context();
        let (s, l, t) = running(&j, &a);
        let p = permit(&j, &a, &s.id, &l, &t, "url-call");
        let i = invocation();
        let url = || {
            McpInputChallenge { request:serde_json::from_value(json!({"id":"ignored","message":"MCP work / lookup: review domain","url":"https://review.example.test/connect?step=1"})).unwrap(), schema:Value::Null }
        };
        for bad_url in [
            "javascript:alert(1)",
            "http://review.example.test",
            "https://user:pass@review.example.test",
            "https://review.example.test/a b",
            "https://review.example.test/\\other",
        ] {
            let mut bad = url();
            bad.request.url = Some(bad_url.into());
            assert!(
                j.register_mcp_input(&a, &s.id, &l, scope(&p, &i), bad)
                    .is_err()
            );
        }
        let mut hybrid = url();
        hybrid.request.questions = Some(vec![]);
        assert!(
            j.register_mcp_input(&a, &s.id, &l, scope(&p, &i), hybrid)
                .is_err()
        );
        let mut hybrid = url();
        hybrid.schema = json!({"type":"object"});
        assert!(
            j.register_mcp_input(&a, &s.id, &l, scope(&p, &i), hybrid)
                .is_err()
        );
        let input = j
            .register_mcp_input(&a, &s.id, &l, scope(&p, &i), url())
            .unwrap();
        // Even an empty answers object is forbidden for every URL action.
        let bad = answer(&s.id, input.id(), response, Some(json!({})));
        assert!(
            j.dispatch_action(
                &a,
                &s.id,
                "responder",
                "browser",
                &bad,
                DispatchAdmission::default()
            )
            .unwrap()
            .rejection_reason
            .is_some()
        );
        let mut good = answer(&s.id, input.id(), response, None);
        good.client_seq += 1;
        let echo = j
            .dispatch_action(
                &a,
                &s.id,
                "responder",
                "browser",
                &good,
                DispatchAdmission::default(),
            )
            .unwrap();
        assert!(echo.rejection_reason.is_none());
        assert_eq!(
            j.dispatch_action(
                &a,
                &s.id,
                "responder",
                "browser",
                &good,
                DispatchAdmission::default()
            )
            .unwrap(),
            echo
        );
        let decision = j
            .take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
            .unwrap()
            .unwrap();
        assert!(decision.content.is_none());
        assert_eq!(decision.device.device_id, "responder");
        assert!(
            j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &input)
                .is_err()
        );
        let pending = j
            .register_mcp_input(&a, &s.id, &l, scope(&p, &i), url())
            .unwrap();
        j.recover().unwrap();
        assert!(
            j.take_mcp_input(&a, &s.id, &l, scope(&p, &i), &pending)
                .is_err()
        );
    }
}
