use super::*;
use crate::tests::{context, setup};

fn invocation() -> McpInvocation {
    McpInvocation {
        provider_id: "work".into(),
        connection_id: "private-connection".into(),
        connection_revision: 1,
        tool_name: "update_issue".into(),
        definition_digest: "a".repeat(64),
        arguments: json!({"id":"ISSUE-1","title":"Exact approved title"}),
    }
}
fn challenge(request: &str) -> McpPermissionChallenge {
    McpPermissionChallenge {
        request_id: request.into(),
        invocation: invocation(),
        ttl_seconds: 60,
        starter_device: "starter".into(),
        binding_revision: 1,
        harness_digest: "harness-digest".into(),
    }
}
fn running(j: &Journal<'_>, a: &AccessContext) -> (Session, Lease, String) {
    let s = j.create(a, "workspace", "project").unwrap();
    let lease = j.acquire(a, &s.id, 0, "worker").unwrap();
    let turn = j.enqueue(a, &s.id, "first", "prompt").unwrap();
    j.claim_next(a, &s.id, &lease).unwrap();
    j.transition(a, &s.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    (s, lease, turn)
}
fn decide(
    j: &Journal<'_>,
    a: &AccessContext,
    id: &str,
    turn: &str,
    approval: &McpApproval,
    approved: bool,
) {
    let p=serde_json::from_value(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,
        "action":{"type":"chat/toolCallConfirmed","turnId":turn,"toolCallId":approval.tool,
        "approved":approved,"selectedOptionId":format!("permission:{}:{}",approval.nonce,if approved{0}else{1})}})).unwrap();
    let result = j
        .dispatch_action(
            a,
            id,
            "approving-device",
            &approval.nonce,
            &p,
            DispatchAdmission::default(),
        )
        .unwrap();
    assert!(
        result.rejection_reason.is_none(),
        "{:?}",
        result.rejection_reason
    );
}
fn duplicate(approval: &McpApproval) -> McpApproval {
    // Deliberately forge a second in-process handle to test the SQL barrier as
    // well as the public non-Clone/non-Deserialize interface.
    McpApproval {
        nonce: approval.nonce.clone(),
        tool: approval.tool.clone(),
        request: approval.request.clone(),
    }
}

#[test]
fn host_mcp_binds_every_invocation_field_and_consumes_human_approval_once() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let approval = j
        .register_mcp_permission(&a, &s.id, &l, &t, challenge("call-1"))
        .unwrap();
    assert_eq!(
        j.mcp_permission_status(&a, &s.id, &l, &approval, &invocation())
            .unwrap(),
        PermissionStatus::Pending
    );
    for field in [
        "provider",
        "connection",
        "revision",
        "tool",
        "definition",
        "arguments",
    ] {
        let mut changed = invocation();
        match field {
            "provider" => changed.provider_id = "other".into(),
            "connection" => changed.connection_id = "other".into(),
            "revision" => changed.connection_revision = 2,
            "tool" => changed.tool_name = "delete_issue".into(),
            "definition" => changed.definition_digest = "b".repeat(64),
            _ => changed.arguments["title"] = json!("Unapproved replacement"),
        }
        assert!(
            j.mcp_permission_status(&a, &s.id, &l, &approval, &changed)
                .is_err(),
            "{field}"
        );
    }
    decide(&j, &a, &s.id, &t, &approval, true);
    assert!(
        j.consume_permission(&a, &s.id, &l, &approval.nonce)
            .is_err()
    );
    let replay = duplicate(&approval);
    let permit = j
        .consume_mcp_permission(&a, &s.id, &l, approval, &invocation())
        .unwrap()
        .unwrap();
    assert!(
        j.consume_mcp_permission(&a, &s.id, &l, replay, &invocation())
            .is_err()
    );
    let decision = j
        .validate_mcp_dispatch(&a, &s.id, &l, &permit, &invocation())
        .unwrap();
    assert_eq!(decision.device_id, "approving-device");
    assert!(
        j.validate_permission_delivery(&a, &s.id, &l, &permit.0.nonce)
            .is_err()
    );
    j.finish_mcp_dispatch(
        &a,
        &s.id,
        &l,
        permit,
        &invocation(),
        McpCompletion::Complete(json!({"content":[{"type":"text","text":"Done"}]})),
    )
    .unwrap();
    let tool = j
        .permission_tool(&s.id, &t, &decision_tool(&j, &s.id))
        .unwrap()
        .unwrap();
    assert_eq!(tool["status"], "completed");
    assert_eq!(tool["confirmed"], "user-action");
    assert_eq!(
        tool["_meta"]["dunlin.confirmationEvidence"],
        "human-one-use"
    );
    assert!(
        j.register_mcp_permission(&a, &s.id, &l, &t, challenge("call-1"))
            .is_err()
    );
}
fn decision_tool(j: &Journal<'_>, id: &str) -> String {
    j.connection
        .query_row(
            "SELECT tool_id FROM interactive_permission_challenges WHERE session=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn acp_cannot_register_report_or_complete_a_host_mcp_invocation() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let approval = j
        .register_mcp_permission(&a, &s.id, &l, &t, challenge("call-1"))
        .unwrap();
    let before = j.sequence(&a, &s.id).unwrap();
    for status in ["pending", "in_progress", "completed", "failed"] {
        assert!(j.agent_update(&a,&s.id,&l,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":approval.tool,"status":status,"rawInput":{"title":"Spoofed"}})).is_err());
        assert_eq!(j.sequence(&a, &s.id).unwrap(), before);
    }
    let legacy = |request| PermissionChallenge {
        request_id: request,
        tool_call: json!({"toolCallId":approval.tool,"title":"Forged","rawInput":{"title":"Other"}}),
        options: json!([]),
        ttl_seconds: 60,
        starter_device: "starter".into(),
        binding_revision: 1,
        harness_digest: "harness".into(),
    };
    assert!(
        j.register_permission(&a, &s.id, &l, &t, legacy(json!(7)))
            .is_err()
    );
    assert!(
        j.register_permission(&a, &s.id, &l, &t, legacy(approval.request.clone()))
            .is_err()
    );
    decide(&j, &a, &s.id, &t, &approval, true);
    let permit = j
        .consume_mcp_permission(&a, &s.id, &l, approval, &invocation())
        .unwrap()
        .unwrap();
    assert!(j.agent_update(&a,&s.id,&l,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":permit.0.tool,"status":"completed"})).is_err());
    j.validate_mcp_dispatch(&a, &s.id, &l, &permit, &invocation())
        .unwrap();
}

#[test]
fn mcp_denial_uncertainty_and_restart_never_make_an_old_request_dispatchable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let denied = j
        .register_mcp_permission(&a, &s.id, &l, &t, challenge("denied"))
        .unwrap();
    let denied_nonce = denied.nonce.clone();
    decide(&j, &a, &s.id, &t, &denied, false);
    assert!(
        j.consume_mcp_permission(&a, &s.id, &l, denied, &invocation())
            .unwrap()
            .is_none()
    );
    assert!(
        j.register_mcp_permission(&a, &s.id, &l, &t, challenge("denied"))
            .is_err()
    );
    let unknown = j
        .register_mcp_permission(&a, &s.id, &l, &t, challenge("uncertain"))
        .unwrap();
    let nonce = unknown.nonce.clone();
    decide(&j, &a, &s.id, &t, &unknown, true);
    let permit = j
        .consume_mcp_permission(&a, &s.id, &l, unknown, &invocation())
        .unwrap()
        .unwrap();
    j.finish_mcp_dispatch(&a, &s.id, &l, permit, &invocation(), McpCompletion::Unknown)
        .unwrap();
    let unfinished = j
        .register_mcp_permission(&a, &s.id, &l, &t, challenge("crashed"))
        .unwrap();
    let unfinished_nonce = unfinished.nonce.clone();
    decide(&j, &a, &s.id, &t, &unfinished, true);
    let stale_permit = j
        .consume_mcp_permission(&a, &s.id, &l, unfinished, &invocation())
        .unwrap()
        .unwrap();
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    for (request, nonce, expected) in [
        ("denied", denied_nonce, "invalid"),
        ("uncertain", nonce, "unknown"),
    ] {
        assert!(
            j.register_mcp_permission(&a, &s.id, &l, &t, challenge(request))
                .is_err()
        );
        assert!(j.consume_permission(&a, &s.id, &l, &nonce).is_err());
        let state: String = c
            .query_row(
                "SELECT state FROM interactive_permission_challenges WHERE nonce=?1",
                [nonce],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, expected);
    }
    assert_eq!(j.recover().unwrap(), 1);
    assert!(
        j.validate_mcp_dispatch(&a, &s.id, &l, &stale_permit, &invocation())
            .is_err()
    );
    let state: String = c
        .query_row(
            "SELECT state FROM interactive_permission_challenges WHERE nonce=?1",
            [unfinished_nonce],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unknown");
}

#[test]
fn mcp_permits_expire_and_stop_on_cancellation_and_authority_change() {
    for fault in ["expiry", "cancel", "authority", "lease"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let (s, l, t) = running(&j, &a);
        let approval = j
            .register_mcp_permission(&a, &s.id, &l, &t, challenge("call-1"))
            .unwrap();
        decide(&j, &a, &s.id, &t, &approval, true);
        let permit = j
            .consume_mcp_permission(&a, &s.id, &l, approval, &invocation())
            .unwrap()
            .unwrap();
        let mut changed = a.clone();
        let mut lease = l.clone();
        match fault {
            "expiry" => {
                c.execute("UPDATE interactive_permission_challenges SET expires=0", [])
                    .unwrap();
            }
            "cancel" => {
                c.execute(
                    "INSERT INTO interactive_cancellation_intents(session,turn_id,epoch,state) VALUES(?1,?2,?3,'requested')",
                    params![s.id,t,sql_integer(l.epoch).unwrap()],
                )
                .unwrap();
            }
            "authority" => changed.membership.revision += 1,
            _ => lease.epoch += 1,
        }
        assert!(
            j.validate_mcp_dispatch(&changed, &s.id, &lease, &permit, &invocation())
                .is_err(),
            "{fault}"
        );
    }
}

#[test]
fn mcp_admission_bounds_payloads_before_registration_or_large_serialization() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    for fault in [
        "bytes",
        "nodes",
        "depth",
        "revision",
        "digest",
        "provider",
        "arguments",
    ] {
        let mut challenge = challenge(fault);
        match fault {
            "bytes" => challenge.invocation.arguments = json!({"text":"x".repeat(8192)}),
            "nodes" => challenge.invocation.arguments = json!({"items":vec![0;1024]}),
            "depth" => {
                let mut value = json!(0);
                for _ in 0..34 {
                    value = json!([value]);
                }
                challenge.invocation.arguments = json!({"value":value});
            }
            "revision" => challenge.invocation.connection_revision = 0,
            "digest" => challenge.invocation.definition_digest = "invalid".into(),
            "provider" => challenge.invocation.provider_id = "../../other".into(),
            _ => challenge.invocation.arguments = json!(null),
        }
        assert!(
            j.register_mcp_permission(&a, &s.id, &l, &t, challenge)
                .is_err(),
            "{fault}"
        );
    }
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_permission_challenges",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn independent_mcp_registrations_race_for_one_durable_request_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    drop(j);
    drop(c);
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|threads| {
        let handles = (0..2)
            .map(|_| {
                threads.spawn(|| {
                    let c = Connection::open(&path).unwrap();
                    c.busy_timeout(std::time::Duration::from_secs(2)).unwrap();
                    barrier.wait();
                    Journal::new(&c)
                        .register_mcp_permission(&a, &s.id, &l, &t, challenge("same-request"))
                        .is_ok()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1);
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_permission_challenges",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn mcp_completion_is_atomic_and_crash_after_consumption_retains_unknown_outcome() {
    for fault in ["known-failure", "event-write-failure", "oversized-result"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let (s, l, t) = running(&j, &a);
        let approval = j
            .register_mcp_permission(&a, &s.id, &l, &t, challenge("call-1"))
            .unwrap();
        let nonce = approval.nonce.clone();
        decide(&j, &a, &s.id, &t, &approval, true);
        let permit = j
            .consume_mcp_permission(&a, &s.id, &l, approval, &invocation())
            .unwrap()
            .unwrap();
        if fault == "known-failure" {
            j.finish_mcp_dispatch(&a, &s.id, &l, permit, &invocation(), McpCompletion::Failed)
                .unwrap();
        } else {
            let outcome = if fault == "event-write-failure" {
                c.execute_batch("CREATE TRIGGER reject_mcp_result BEFORE INSERT ON interactive_events WHEN NEW.kind='permission_lifecycle' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
                McpCompletion::Complete(json!({"content":[]}))
            } else {
                McpCompletion::Complete(json!({"text":"x".repeat(48*1024)}))
            };
            assert!(
                j.finish_mcp_dispatch(&a, &s.id, &l, permit, &invocation(), outcome)
                    .is_err()
            );
            let state: String = c
                .query_row(
                    "SELECT state FROM interactive_permission_challenges WHERE nonce=?1",
                    [&nonce],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "consumed");
            c.execute_batch("DROP TRIGGER IF EXISTS reject_mcp_result")
                .unwrap();
            j.interrupt(&s.id, &l, "fixture_crash").unwrap();
        }
        let state: String = c
            .query_row(
                "SELECT state FROM interactive_permission_challenges WHERE nonce=?1",
                [&nonce],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            state,
            if fault == "known-failure" {
                "delivered"
            } else {
                "unknown"
            }
        );
    }
}
