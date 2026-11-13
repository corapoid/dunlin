use super::*;
use crate::tests::{context, setup};
fn challenge(request: i64) -> PermissionChallenge {
    PermissionChallenge {
        request_id: json!(request),
        tool_call: json!({"toolCallId":"tool","title":"Read file","kind":"read","rawInput":{"path":"file"}}),
        options: json!([{"optionId":"yes","kind":"allow_once","name":"Allow once"},{"optionId":"always","kind":"allow_always","name":"Always"}]),
        ttl_seconds: 60,
        starter_device: "starter".into(),
        binding_revision: 1,
        harness_digest: "digest".into(),
    }
}
pub(in crate::permissions) fn running(
    j: &Journal<'_>,
    a: &AccessContext,
) -> (Session, Lease, String) {
    let s = j.create(a, "workspace", "project").unwrap();
    let lease = j.acquire(a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(a, &s.id, "first", "prompt").unwrap();
    j.claim_next(a, &s.id, &lease).unwrap();
    j.transition(a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    (s, lease, t)
}
fn decision(id: &str, turn: &str, nonce: &str, seq: i64, approved: bool) -> DispatchActionParams {
    serde_json::from_value(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":seq,"action":{"type":"chat/toolCallConfirmed","turnId":turn,"toolCallId":"tool","approved":approved,"selectedOptionId":format!("permission:{nonce}:{}",if approved{"0"}else{"deny"})}})).unwrap()
}
#[test]
fn one_use_decision_is_durable_consumed_once_and_preserves_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let n = j
        .register_permission(&a, &s.id, &l, &t, challenge(1))
        .unwrap();
    assert_eq!(
        j.permission_status(&a, &s.id, &l, &n).unwrap(),
        PermissionStatus::Pending
    );
    let tool = j.permission_tool(&s.id, &t, "tool").unwrap().unwrap();
    assert_eq!(tool["status"], "pending-confirmation");
    assert_eq!(tool["options"].as_array().unwrap().len(), 2);
    assert!(tool["options"].to_string().find("always").is_none());
    let p = decision(&s.id, &t, &n, 1, true);
    let echo = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default(),
        )
        .unwrap();
    assert!(echo.rejection_reason.is_none());
    assert!(
        j.dispatch_action(
            &a,
            &s.id,
            "device2",
            "other",
            &decision(&s.id, &t, &n, 1, false),
            DispatchAdmission::default()
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    assert_eq!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default()
        )
        .unwrap(),
        echo
    );
    let d = j.consume_permission(&a, &s.id, &l, &n).unwrap();
    assert!(d.approved);
    assert_eq!(d.option_id.as_deref(), Some("yes"));
    assert!(j.consume_permission(&a, &s.id, &l, &n).is_err());
    j.validate_permission_delivery(&a, &s.id, &l, &n).unwrap();
    j.permission_delivered(&a, &s.id, &l, &n).unwrap();
    j.agent_update(&a,&s.id,&l,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed","title":"Read"})).unwrap();
    let tool = j.permission_tool(&s.id, &t, "tool").unwrap().unwrap();
    assert_eq!(tool["confirmed"], "user-action");
    assert_eq!(
        tool["_meta"]["dunlin.confirmationEvidence"],
        "human-one-use"
    );
}
#[test]
fn nonce_supersession_expiry_and_denial_do_not_enable_late_execution() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let old = j
        .register_permission(&a, &s.id, &l, &t, challenge(1))
        .unwrap();
    let n = j
        .register_permission(&a, &s.id, &l, &t, challenge(2))
        .unwrap();
    assert_ne!(old, n);
    j.invalidate_permission(&a, &s.id, &l, &old, "superseded")
        .unwrap();
    assert_eq!(
        j.permission_tool(&s.id, &t, "tool").unwrap().unwrap()["status"],
        "pending-confirmation"
    );
    assert!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &decision(&s.id, &t, &old, 1, true),
            DispatchAdmission::default()
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
    let echo = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &decision(&s.id, &t, &n, 2, false),
            DispatchAdmission::default(),
        )
        .unwrap();
    assert!(echo.rejection_reason.is_none());
    let d = j.consume_permission(&a, &s.id, &l, &n).unwrap();
    assert!(!d.approved);
    assert_eq!(d.option_id, None);
    j.permission_delivered(&a, &s.id, &l, &n).unwrap();
    let before = j.permission_chat(&s.id).unwrap();
    j.agent_update(&a,&s.id,&l,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed","title":"ignored"})).unwrap();
    assert_eq!(j.permission_chat(&s.id).unwrap(), before);
}
#[test]
fn expiry_authority_change_and_cancel_invalidate_permission_before_delivery() {
    for mode in ["expiry", "authority", "cancel"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let (s, l, t) = running(&j, &a);
        let n = j
            .register_permission(&a, &s.id, &l, &t, challenge(1))
            .unwrap();
        let p = decision(&s.id, &t, &n, 1, true);
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default(),
        )
        .unwrap();
        match mode {
            "expiry" => {
                c.execute(
                    "UPDATE interactive_permission_challenges SET expires=unixepoch()-1",
                    [],
                )
                .unwrap();
                assert_eq!(
                    j.permission_status(&a, &s.id, &l, &n).unwrap(),
                    PermissionStatus::Invalid
                );
                assert!(j.consume_permission(&a, &s.id, &l, &n).is_err());
                j.invalidate_permission(&a, &s.id, &l, &n, "expired")
                    .unwrap();
            }
            "authority" => {
                let mut changed = a.clone();
                changed.membership.revision += 1;
                assert!(j.permission_status(&changed, &s.id, &l, &n).is_err());
                assert!(j.consume_permission(&changed, &s.id, &l, &n).is_err());
            }
            _ => {
                let p=serde_json::from_value(json!({"channel":format!("ahp-chat:/{}",s.id),"clientSeq":2,"action":{"type":"chat/turnCancelled","turnId":t,"duration":0}})).unwrap();
                j.dispatch_action(
                    &a,
                    &s.id,
                    "device",
                    "client",
                    &p,
                    DispatchAdmission::default(),
                )
                .unwrap();
                assert_eq!(
                    j.permission_status(&a, &s.id, &l, &n).unwrap(),
                    PermissionStatus::Invalid
                );
                assert!(j.consume_permission(&a, &s.id, &l, &n).is_err());
            }
        }
    }
}
#[test]
fn consumed_undelivered_is_unknown_on_restart_and_never_reconsumed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let n = j
        .register_permission(&a, &s.id, &l, &t, challenge(1))
        .unwrap();
    let p = decision(&s.id, &t, &n, 1, true);
    let echo = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default(),
        )
        .unwrap();
    j.consume_permission(&a, &s.id, &l, &n).unwrap();
    assert!(
        j.register_permission(&a, &s.id, &l, &t, challenge(2))
            .is_err()
    );
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    j.recover().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM interactive_permission_challenges",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert!(j.consume_permission(&a, &s.id, &l, &n).is_err());
    assert!(j.permission_delivered(&a, &s.id, &l, &n).is_err());
    let s = j.channel_summary(&a, &s.id).unwrap();
    assert_eq!(
        s.meta.as_ref().unwrap()["dunlin.permissionUnconfirmed"],
        json!(true)
    );
    assert_eq!(s.status, 2);
    assert_eq!(
        j.dispatch_action(
            &a,
            s.resource.strip_prefix("ahp-session:/").unwrap(),
            "device",
            "client",
            &p,
            DispatchAdmission::default()
        )
        .unwrap(),
        echo
    );
}
#[test]
fn decision_and_registration_failures_roll_back_projection_source_and_state() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    let before = j.permission_chat(&s.id).unwrap();
    let clock = j.channel_clock().unwrap().sequence;
    c.execute_batch("CREATE TRIGGER fail_permission BEFORE INSERT ON interactive_ahp_actions BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        j.register_permission(&a, &s.id, &l, &t, challenge(1))
            .is_err()
    );
    assert_eq!(j.permission_chat(&s.id).unwrap(), before);
    assert_eq!(j.channel_clock().unwrap().sequence, clock);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_permission_challenges",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER fail_permission").unwrap();
    let n = j
        .register_permission(&a, &s.id, &l, &t, challenge(1))
        .unwrap();
    let before = j.permission_chat(&s.id).unwrap();
    c.execute_batch("CREATE TRIGGER fail_decision BEFORE INSERT ON interactive_client_dispatch BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &decision(&s.id, &t, &n, 1, true),
            DispatchAdmission::default()
        )
        .is_err()
    );
    assert_eq!(
        j.permission_status(&a, &s.id, &l, &n).unwrap(),
        PermissionStatus::Pending
    );
    assert_eq!(j.permission_chat(&s.id).unwrap(), before);
}
#[test]
fn malformed_challenges_permanent_options_and_scope_are_fail_closed() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let (s, l, t) = running(&j, &a);
    for mode in ["size", "duplicate", "ttl", "terminal", "option"] {
        let mut p = challenge(1);
        match mode {
            "size" => p.tool_call["rawInput"] = json!("a".repeat(32768)),
            "duplicate" => {
                p.options = json!([{"optionId":"a","kind":"allow_once","name":"a"},{"optionId":"a","kind":"reject_once","name":"b"}])
            }
            "ttl" => p.ttl_seconds = 301,
            "terminal" => p.tool_call["status"] = json!("completed"),
            _ => p.options = json!([{"optionId":"a","kind":"future_unsafe","name":"a"}]),
        }
        assert!(j.register_permission(&a, &s.id, &l, &t, p).is_err());
    }
    let mut stranger = a.clone();
    stranger.membership.user_id = "other".into();
    assert!(
        j.register_permission(&stranger, &s.id, &l, &t, challenge(1))
            .is_err()
    );
    let n = j
        .register_permission(&a, &s.id, &l, &t, challenge(1))
        .unwrap();
    let mut p = decision(&s.id, &t, &n, 1, true);
    if let StateAction::ChatToolCallConfirmed(m) = &mut p.action {
        m.edited_tool_input = Some("tampered".into());
    }
    assert!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default()
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
    assert_eq!(
        j.permission_status(&a, &s.id, &l, &n).unwrap(),
        PermissionStatus::Pending
    );
}
