use super::*;
use crate::tests::{context, setup};
use ahp_types::commands::DispatchActionParams;
fn params(id: &str, seq: i64, action: serde_json::Value) -> DispatchActionParams {
    serde_json::from_value(
        json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":seq,"action":action}),
    )
    .unwrap()
}
fn queued(id: &str, text: &str) -> serde_json::Value {
    json!({"type":"chat/pendingMessageSet","kind":"queued","id":id,"message":{"text":text,"origin":{"kind":"user"}}})
}
fn send(
    j: &Journal<'_>,
    a: &AccessContext,
    s: &str,
    seq: i64,
    action: serde_json::Value,
) -> ahp_types::actions::ActionEnvelope {
    j.dispatch_action(
        a,
        s,
        "device",
        "client",
        &params(s, seq, action),
        DispatchAdmission::default(),
    )
    .unwrap()
}
#[test]
fn queued_upsert_remove_and_reorder_match_durable_fifo_and_reducers() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    for (seq, id) in [(1, "a"), (2, "b"), (3, "c")] {
        assert!(
            send(&j, &a, &s.id, seq, queued(id, id))
                .rejection_reason
                .is_none()
        );
    }
    assert!(
        send(&j, &a, &s.id, 4, queued("b", "updated"))
            .rejection_reason
            .is_none()
    );
    send(
        &j,
        &a,
        &s.id,
        5,
        json!({"type":"chat/queuedMessagesReordered","order":["unknown","c"]}),
    );
    send(
        &j,
        &a,
        &s.id,
        6,
        json!({"type":"chat/pendingMessageRemoved","kind":"queued","id":"a"}),
    );
    send(&j, &a, &s.id, 7, queued("d", "last"));
    let chat = j.projected_chat(&a, &s.id).unwrap();
    let ids: Vec<_> = chat
        .queued_messages
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(ids, vec!["c", "b", "d"]);
    let first = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
    assert_eq!(first.prompt, "c");
    j.transition(&a, &s.id, &lease, &first.id, TurnPhase::Running)
        .unwrap();
    j.transition(&a, &s.id, &lease, &first.id, TurnPhase::Completed)
        .unwrap();
    let second = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
    assert_eq!(second.prompt, "updated");
    assert!(
        send(&j, &a, &s.id, 8, queued("b", "too late"))
            .rejection_reason
            .is_some()
    );
    assert!(
        send(
            &j,
            &a,
            &s.id,
            9,
            json!({"type":"chat/pendingMessageRemoved","kind":"queued","id":"b"})
        )
        .rejection_reason
        .is_some()
    );
}
#[test]
fn cancel_sequences_once_preserves_raw_evidence_and_blocks_next_until_settlement() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &s.id, "first", "first").unwrap();
    j.enqueue(&a, &s.id, "next", "next").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    j.transition(&a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    let p = params(
        &s.id,
        1,
        json!({"type":"chat/turnCancelled","turnId":t,"duration":99999}),
    );
    let e = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission {
                rejection: None,
                elapsed_ms: Some(42),
            },
        )
        .unwrap();
    assert!(e.rejection_reason.is_none());
    if let StateAction::ChatTurnCancelled(a) = &e.action {
        assert_eq!(a.duration, 42);
    } else {
        panic!()
    }
    assert!(j.cancellation_requested(&a, &s.id, &lease, &t).unwrap());
    assert!(j.projected_chat(&a, &s.id).unwrap().active_turn.is_none());
    assert!(j.claim_next(&a, &s.id, &lease).is_err());
    let before = j.projected_chat(&a, &s.id).unwrap();
    j.output(&a, &s.id, &lease, &t, "late evidence").unwrap();
    j.agent_update(
        &a,
        &s.id,
        &lease,
        &t,
        json!({"sessionUpdate":"unknown_future_update","secret":"raw-only"}),
    )
    .unwrap();
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    assert_eq!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission {
                rejection: Some("stopped"),
                elapsed_ms: None
            }
        )
        .unwrap(),
        e
    );
    j.settle_cancellation(&a, &s.id, &lease, &t, "agent_response:cancelled", 44)
        .unwrap();
    assert!(!j.cancellation_requested(&a, &s.id, &lease, &t).unwrap());
    j.settle_cancellation(&a, &s.id, &lease, &t, "duplicate", 44)
        .unwrap();
    j.output(&a, &s.id, &lease, &t, "after settle raw-only")
        .unwrap();
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    assert!(j.claim_next(&a, &s.id, &lease).unwrap().is_some());
    let log = j.projected_actions(&a, &s.id, 0).unwrap();
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e.action, StateAction::ChatTurnCancelled(_)))
            .count(),
        1
    );
}
#[test]
fn completion_cancel_race_is_atomic_and_recovery_is_unconfirmed() {
    for complete_first in [false, true] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let s = j.create(&a, "workspace", "project").unwrap();
        let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
        let t = j.enqueue(&a, &s.id, "first", "first").unwrap();
        j.claim_next(&a, &s.id, &lease).unwrap();
        j.transition(&a, &s.id, &lease, &t, TurnPhase::Running)
            .unwrap();
        if complete_first {
            j.transition_timed(&a, &s.id, &lease, &t, TurnPhase::Completed, Some(3))
                .unwrap();
        }
        let e = send(
            &j,
            &a,
            &s.id,
            1,
            json!({"type":"chat/turnCancelled","turnId":t,"duration":0}),
        );
        if complete_first {
            assert!(e.rejection_reason.is_some());
        } else {
            assert!(e.rejection_reason.is_none());
            j.transition_timed(&a, &s.id, &lease, &t, TurnPhase::Completed, Some(4))
                .unwrap();
            assert_eq!(
                c.query_row(
                    "SELECT phase FROM interactive_turns WHERE id=?1",
                    [&t],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "cancelled"
            );
            assert_eq!(j.projected_chat(&a, &s.id).unwrap().turns.len(), 1);
        }
    }
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &s.id, "first", "first").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    send(
        &j,
        &a,
        &s.id,
        1,
        json!({"type":"chat/turnCancelled","turnId":t,"duration":0}),
    );
    j.recover().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM interactive_cancellation_intents",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unconfirmed"
    );
    let summary = j.channel_summary(&a, &s.id).unwrap();
    assert_eq!(summary.status, 2);
    assert_eq!(
        summary.meta.as_ref().unwrap()["dunlin.cancellationUnconfirmed"],
        json!(true)
    );
    assert_eq!(j.projected_chat(&a, &s.id).unwrap().turns.len(), 1);
    assert!(
        j.settle_cancellation(&a, &s.id, &lease, &t, "late old actor", 4)
            .is_err()
    );
}
#[test]
fn controls_rollback_all_mutations_when_final_ledger_insert_fails() {
    for control in ["upsert", "remove", "reorder", "cancel"] {
        let c = Connection::open_in_memory().unwrap();
        setup(&c);
        let a = context();
        let j = Journal::new(&c);
        let s = j.create(&a, "workspace", "project").unwrap();
        let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
        send(&j, &a, &s.id, 1, queued("a", "first"));
        send(&j, &a, &s.id, 2, queued("b", "second"));
        let action = match control {
            "upsert" => queued("a", "modified"),
            "remove" => json!({"type":"chat/pendingMessageRemoved","kind":"queued","id":"a"}),
            "reorder" => json!({"type":"chat/queuedMessagesReordered","order":["b","a"]}),
            _ => {
                let t = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
                json!({"type":"chat/turnCancelled","turnId":t.id,"duration":99})
            }
        };
        let before = j.projected_chat(&a, &s.id).unwrap();
        let source = j.session(&s.id).unwrap().sequence;
        let clock = j.channel_clock().unwrap().sequence;
        let sql_before: Vec<(String, String, i64)> = c
            .prepare("SELECT prompt,phase,position FROM interactive_turns ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        c.execute_batch("CREATE TRIGGER fail_control BEFORE INSERT ON interactive_client_dispatch WHEN NEW.client_seq=3 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(
            j.dispatch_action(
                &a,
                &s.id,
                "device",
                "client",
                &params(&s.id, 3, action.clone()),
                DispatchAdmission::default()
            )
            .is_err()
        );
        assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
        assert_eq!(j.session(&s.id).unwrap().sequence, source);
        assert_eq!(j.channel_clock().unwrap().sequence, clock);
        let sql_after: Vec<(String, String, i64)> = c
            .prepare("SELECT prompt,phase,position FROM interactive_turns ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(sql_before, sql_after);
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM interactive_cancellation_intents",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        c.execute_batch("DROP TRIGGER fail_control").unwrap();
        assert!(send(&j, &a, &s.id, 3, action).rejection_reason.is_none());
    }
}
#[test]
fn cancel_reopen_dedupe_scope_and_recovery_never_claim_a_second_turn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &s.id, "first", "first").unwrap();
    j.enqueue(&a, &s.id, "second", "second").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    let p = params(
        &s.id,
        1,
        json!({"type":"chat/turnCancelled","turnId":t,"duration":123}),
    );
    let e = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default(),
        )
        .unwrap();
    let mut stranger = a.clone();
    stranger.membership.user_id = "stranger".into();
    assert!(
        j.dispatch_action(
            &stranger,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission::default()
        )
        .is_err()
    );
    let mut revoked = a.clone();
    revoked.membership.active = false;
    assert!(
        j.cancellation_requested(&revoked, &s.id, &lease, &t)
            .is_err()
    );
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    assert!(j.claim_next(&a, &s.id, &lease).is_err());
    assert_eq!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission {
                rejection: Some("unavailable"),
                elapsed_ms: Some(999)
            }
        )
        .unwrap(),
        e
    );
    j.recover().unwrap();
    assert!(j.claim_next(&a, &s.id, &lease).is_err());
    assert_eq!(
        j.dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &p,
            DispatchAdmission {
                rejection: Some("unavailable"),
                elapsed_ms: None
            }
        )
        .unwrap(),
        e
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_turns WHERE phase='queued'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM interactive_cancellation_intents",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unconfirmed"
    );
}
#[test]
fn cancellation_ack_uses_recovery_reserve_after_normal_budget_is_exhausted() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &s.id, "first", "first").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    send(
        &j,
        &a,
        &s.id,
        1,
        json!({"type":"chat/turnCancelled","turnId":t,"duration":0}),
    );
    c.execute(
        "UPDATE interactive_sessions SET bytes=?2 WHERE id=?1",
        params![s.id, MAX_BYTES - 4096],
    )
    .unwrap();
    c.execute(
        "UPDATE interactive_ahp_chats SET action_count=4064 WHERE session=?1",
        [&s.id],
    )
    .unwrap();
    j.settle_cancellation(&a, &s.id, &lease, &t, "agent_response:cancelled", 42)
        .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM interactive_cancellation_intents",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "confirmed"
    );
}
#[test]
fn canonical_reorder_preserves_missing_original_order_and_stale_controls_are_noops() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.acquire(&a, &s.id, 0, "worker").unwrap();
    let before = j.projected_chat(&a, &s.id).unwrap();
    let removed = send(
        &j,
        &a,
        &s.id,
        1,
        json!({"type":"chat/pendingMessageRemoved","kind":"queued","id":"unknown"}),
    );
    assert!(removed.rejection_reason.is_none());
    let reordered = send(
        &j,
        &a,
        &s.id,
        2,
        json!({"type":"chat/queuedMessagesReordered","order":["unknown"]}),
    );
    assert!(reordered.rejection_reason.is_none());
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    for (seq, id) in [(3, "z"), (4, "b"), (5, "a")] {
        send(&j, &a, &s.id, seq, queued(id, id));
    }
    let echo = send(
        &j,
        &a,
        &s.id,
        6,
        json!({"type":"chat/queuedMessagesReordered","order":["unknown","z"]}),
    );
    if let StateAction::ChatQueuedMessagesReordered(a) = echo.action {
        assert_eq!(a.order, vec!["z", "b", "a"]);
    } else {
        panic!()
    }
    let chat = j.projected_chat(&a, &s.id).unwrap();
    let ids: Vec<_> = chat
        .queued_messages
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(ids, vec!["z", "b", "a"]);
}

fn queue_ids(chat: &ahp_types::state::ChatState) -> Vec<String> {
    chat.queued_messages
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|message| message.id.clone())
        .collect()
}

#[test]
fn partial_reorder_canonical_echo_matches_sql_live_sdk_and_snapshot_and_retry_is_immutable() {
    use ahp::reducers::{ReduceOutcome, apply_action_to_chat};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let mut live = j.projected_chat(&a, &s.id).unwrap();
    for (seq, id) in [(1, "z"), (2, "a"), (3, "m")] {
        let envelope = send(&j, &a, &s.id, seq, queued(id, id));
        assert_eq!(
            apply_action_to_chat(&mut live, &envelope.action),
            ReduceOutcome::Applied
        );
    }
    let cut = j.channel_clock().unwrap().sequence;
    let request = params(
        &s.id,
        4,
        json!({"type":"chat/queuedMessagesReordered","order":["m"]}),
    );
    let accepted = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &request,
            DispatchAdmission::default(),
        )
        .unwrap();
    assert!(accepted.rejection_reason.is_none());
    assert_eq!(accepted.origin.as_ref().unwrap().client_seq, 4);
    assert_eq!(
        serde_json::to_value(&accepted.action).unwrap(),
        json!({"type":"chat/queuedMessagesReordered","order":["m","z","a"]})
    );
    assert_eq!(
        apply_action_to_chat(&mut live, &accepted.action),
        ReduceOutcome::Applied
    );
    assert_eq!(queue_ids(&live), ["m", "z", "a"]);
    assert_eq!(live, j.projected_chat(&a, &s.id).unwrap());
    let snapshot = j.projected_snapshot(&a, &s.id).unwrap();
    let ahp_types::state::SnapshotState::Chat(snapshot) = snapshot.state else {
        panic!("Expected chat snapshot")
    };
    assert_eq!(*snapshot, live);
    let actions = j.projected_actions(&a, &s.id, cut).unwrap();
    assert_eq!(actions.as_slice(), std::slice::from_ref(&accepted));
    let queued_ids:Vec<String> = c.prepare("SELECT m.message_id FROM interactive_turns t JOIN interactive_message_ids m ON m.session=t.session AND m.turn_id=t.id WHERE t.session=?1 AND t.phase='queued' ORDER BY t.position").unwrap()
        .query_map([&s.id], |row| row.get(0)).unwrap().collect::<std::result::Result<_, _>>().unwrap();
    assert_eq!(queued_ids, ["m", "z", "a"]);
    let first = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
    assert_eq!(j.pending_message_id(&s.id, &first.id).unwrap(), "m");
    j.transition(&a, &s.id, &lease, &first.id, TurnPhase::Running)
        .unwrap();
    j.transition(&a, &s.id, &lease, &first.id, TurnPhase::Completed)
        .unwrap();
    send(&j, &a, &s.id, 5, queued("new", "new"));
    let before_retry = j.projected_chat(&a, &s.id).unwrap();
    assert_eq!(queue_ids(&before_retry), ["z", "a", "new"]);
    let clock = j.channel_clock().unwrap();
    drop(j);
    drop(c);
    let c = Connection::open(path).unwrap();
    let j = Journal::new(&c);
    let retry = j
        .dispatch_action(
            &a,
            &s.id,
            "device",
            "client",
            &request,
            DispatchAdmission {
                rejection: Some("actor unavailable"),
                elapsed_ms: None,
            },
        )
        .unwrap();
    assert_eq!(retry, accepted);
    assert_eq!(j.channel_clock().unwrap(), clock);
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before_retry);
    let next = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
    assert_eq!(j.pending_message_id(&s.id, &next.id).unwrap(), "z");
}

#[test]
fn reorder_unknowns_noops_duplicates_and_bounds_preserve_authoritative_queue() {
    use ahp::reducers::{ReduceOutcome, apply_action_to_chat};
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.acquire(&a, &s.id, 0, "worker").unwrap();
    let mut live = j.projected_chat(&a, &s.id).unwrap();
    let empty = send(
        &j,
        &a,
        &s.id,
        1,
        json!({"type":"chat/queuedMessagesReordered","order":["unknown"]}),
    );
    assert!(empty.rejection_reason.is_none());
    assert_eq!(
        serde_json::to_value(&empty.action).unwrap(),
        json!({"type":"chat/queuedMessagesReordered","order":[]})
    );
    assert_eq!(
        apply_action_to_chat(&mut live, &empty.action),
        ReduceOutcome::NoOp
    );
    assert_eq!(live, j.projected_chat(&a, &s.id).unwrap());
    for (seq, id) in [(2, "z"), (3, "a"), (4, "m")] {
        send(&j, &a, &s.id, seq, queued(id, id));
    }
    let before = j.projected_chat(&a, &s.id).unwrap();
    for (seq, order) in [(5, json!(["unknown"])), (6, json!([]))] {
        let echo = send(
            &j,
            &a,
            &s.id,
            seq,
            json!({"type":"chat/queuedMessagesReordered","order":order}),
        );
        assert!(echo.rejection_reason.is_none());
        assert_eq!(
            serde_json::to_value(&echo.action).unwrap(),
            json!({"type":"chat/queuedMessagesReordered","order":["z","a","m"]})
        );
        assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    }
    for (seq, order) in [
        (7, json!(["m", "m"])),
        (
            8,
            json!(
                (0..33)
                    .map(|id| format!("unknown-{id}"))
                    .collect::<Vec<_>>()
            ),
        ),
    ] {
        let rejected = send(
            &j,
            &a,
            &s.id,
            seq,
            json!({"type":"chat/queuedMessagesReordered","order":order}),
        );
        assert!(rejected.rejection_reason.is_some());
        assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    }
}
