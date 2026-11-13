use super::*;
use crate::tests::{context, setup};
use ahp_types::state::TurnState;
fn running(j: &Journal<'_>, a: &AccessContext) -> (Session, Lease, String) {
    let s = j.create(a, "workspace", "project").unwrap();
    let lease = j.acquire(a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(a, &s.id, "one", "user prompt").unwrap();
    j.claim_next(a, &s.id, &lease).unwrap();
    j.transition(a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    (s, lease, t)
}
#[test]
fn official_reducer_roundtrip_and_reconnect_reproduce_the_durable_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let s = j.create(&a, "workspace", "project").unwrap();
    let baseline = j.projected_chat(&a, &s.id).unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &s.id, "one", "hello").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    j.transition(&a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    j.agent_update(
        &a,
        &s.id,
        &lease,
        &t,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}),
    )
    .unwrap();
    j.agent_update(
        &a,
        &s.id,
        &lease,
        &t,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" world"}}),
    )
    .unwrap();
    j.agent_update(
        &a,
        &s.id,
        &lease,
        &t,
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"reason"}}),
    )
    .unwrap();
    j.output(&a, &s.id, &lease, &t, "after reasoning").unwrap();
    j.transition_timed(&a, &s.id, &lease, &t, TurnPhase::Completed, Some(17))
        .unwrap();
    let log = j.projected_actions(&a, &s.id, 0).unwrap();
    let mut connected = baseline.clone();
    let mut disconnected = baseline;
    for envelope in &log {
        let wire = serde_json::to_vec(envelope).unwrap();
        let parsed: ActionEnvelope = serde_json::from_slice(&wire).unwrap();
        assert_eq!(parsed.channel, format!("ahp-chat:/{}", s.id));
        assert_eq!(
            apply_action_to_chat(&mut connected, &parsed.action),
            ReduceOutcome::Applied
        );
    }
    for envelope in &log[..3] {
        assert_eq!(
            apply_action_to_chat(&mut disconnected, &envelope.action),
            ReduceOutcome::Applied
        );
    }
    let cursor = log[2].server_seq;
    assert_eq!(connected, j.projected_chat(&a, &s.id).unwrap());
    assert!(connected.active_turn.is_none());
    assert_eq!(connected.turns[0].duration, Some(17));
    assert_eq!(connected.turns[0].state, TurnState::Complete);
    match &connected.turns[0].response_parts[..] {
        [
            ResponsePart::Markdown(first),
            ResponsePart::Reasoning(reason),
            ResponsePart::Markdown(last),
        ] => {
            assert_eq!(first.content, "Hello world");
            assert_eq!(reason.content, "reason");
            assert_eq!(last.content, "after reasoning");
        }
        _ => panic!("Expected ordered text/reasoning/text parts"),
    }
    assert!(log.windows(2).all(|p| p[0].server_seq < p[1].server_seq));
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    for envelope in j.projected_actions(&a, &s.id, cursor).unwrap() {
        assert_eq!(
            apply_action_to_chat(&mut disconnected, &envelope.action),
            ReduceOutcome::Applied
        );
    }
    assert_eq!(disconnected, connected);
}
#[test]
fn failed_projection_rolls_back_source_queue_clock_and_idempotency_together() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let s = j.create(&a, "workspace", "project").unwrap();
    c.execute_batch("CREATE TRIGGER reject_action BEFORE INSERT ON interactive_ahp_actions BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(j.enqueue(&a, &s.id, "key", "one prompt").is_err());
    assert!(j.snapshot(&a, &s.id).unwrap().1.is_empty());
    assert_eq!(j.sequence(&a, &s.id).unwrap(), 1);
    assert!(j.projected_actions(&a, &s.id, 0).unwrap().is_empty());
    assert!(
        j.projected_chat(&a, &s.id)
            .unwrap()
            .queued_messages
            .is_none()
    );
    c.execute_batch("DROP TRIGGER reject_action").unwrap();
    let t = j.enqueue(&a, &s.id, "key", "one prompt").unwrap();
    assert_eq!(j.enqueue(&a, &s.id, "key", "one prompt").unwrap(), t);
    let log = j.projected_actions(&a, &s.id, 0).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].server_seq, 1);
}
#[test]
fn interruption_never_projects_unknown_work_as_complete_or_replays_a_prompt() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, lease, t) = running(&j, &a);
    j.output(&a, &s.id, &lease, &t, "partial").unwrap();
    j.interrupt(&s.id, &lease, "fixture_stop").unwrap();
    let state = j.projected_chat(&a, &s.id).unwrap();
    assert_eq!(state.turns[0].state, TurnState::Error);
    assert!(state.active_turn.is_none());
    let log = j.projected_actions(&a, &s.id, 0).unwrap();
    let last = serde_json::to_value(&log.last().unwrap().action).unwrap();
    assert_eq!(last["type"], "chat/error");
    assert_eq!(last["part"]["error"]["errorType"], "unknown_outcome");
    assert_eq!(last["part"]["resumable"], false);
    assert_eq!(last["_meta"]["dunlin.durationUnknown"], true);
    let before = log.len();
    assert_eq!(j.recover().unwrap(), 0);
    assert_eq!(j.projected_actions(&a, &s.id, 0).unwrap().len(), before);
    let mut revoked = a.clone();
    revoked.project_grant.as_mut().unwrap().permissions.clear();
    assert!(j.projected_chat(&revoked, &s.id).is_err());
    assert!(j.projected_actions(&revoked, &s.id, 0).is_err());
}
#[test]
fn tool_reports_preserve_failure_and_never_invent_a_human_approval() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, lease, t) = running(&j, &a);
    j.agent_update(&a,&s.id,&lease,&t,json!({"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Read","kind":"read","status":"pending","rawInput":{"path":"file"}})).unwrap();
    j.agent_update(&a,&s.id,&lease,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"pending","rawInput":{"path":"updated"}})).unwrap();
    j.agent_update(
        &a,
        &s.id,
        &lease,
        &t,
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"in_progress"}),
    )
    .unwrap();
    j.agent_update(&a,&s.id,&lease,&t,json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"failed","content":[{"type":"content","content":{"type":"text","text":"read failed"}},{"type":"diff","path":"file","oldText":"a","newText":"b"}]})).unwrap();
    let state = serde_json::to_value(j.projected_chat(&a, &s.id).unwrap()).unwrap();
    let tool = &state["activeTurn"]["responseParts"][0]["toolCall"];
    assert_eq!(tool["status"], "completed");
    assert_eq!(tool["success"], false);
    assert_eq!(tool["confirmed"], "not-needed");
    assert_eq!(tool["toolInput"], r#"{"path":"updated"}"#);
    assert!(tool.get("partialInput").is_none());
    assert_eq!(tool["content"][0]["text"], "read failed");
    assert_eq!(
        tool["structuredContent"]["dunlin.acpReport"]["content"][1]["type"],
        "diff"
    );
    assert!(
        j.agent_update(
            &a,
            &s.id,
            &lease,
            &t,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"missing","status":"completed"})
        )
        .is_err()
    );
}
#[test]
fn migration_preserves_unsupported_legacy_data_and_refuses_an_empty_projection_success() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, lease, t) = running(&j, &a);
    j.output(&a, &s.id, &lease, &t, "old retained text")
        .unwrap();
    c.execute_batch("DROP TABLE interactive_client_dispatch; DROP TABLE interactive_message_ids; DROP TABLE interactive_ahp_clients; DROP TABLE interactive_ahp_sessions; DROP TABLE interactive_ahp_actions; DROP TABLE interactive_ahp_chats; DROP TABLE interactive_ahp_clock;").unwrap();
    let seq = j.sequence(&a, &s.id).unwrap() + 1;
    let unsupported=json!({"turn_id":t,"update":{"sessionUpdate":"future_update","private":"retained original"}}).to_string();
    c.execute(
        "INSERT INTO interactive_events VALUES(?1,?2,'agent_update',?3)",
        params![s.id, sql_integer(seq).unwrap(), unsupported],
    )
    .unwrap();
    c.execute(
        "UPDATE interactive_sessions SET seq=?2 WHERE id=?1",
        params![s.id, sql_integer(seq).unwrap()],
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    tx.execute_batch(PROJECTION_SCHEMA).unwrap();
    tx.execute_batch(crate::CHANNEL_SCHEMA).unwrap();
    j.backfill_projection().unwrap();
    tx.commit().unwrap();
    assert!(j.projected_chat(&a, &s.id).is_err());
    assert!(j.projected_actions(&a, &s.id, 0).is_err());
    let raw: String = c
        .query_row(
            "SELECT payload FROM interactive_events WHERE session=?1 AND seq=?2",
            params![s.id, sql_integer(seq).unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, unsupported);
    assert_eq!(j.snapshot(&a, &s.id).unwrap().1[0].prompt, "user prompt");
    // Keep the worker's lease alive only in this fixture; migration never runs it.
    assert_eq!(lease.epoch, 1);
}

#[test]
fn projected_actions_have_one_global_order_and_private_reads_remain_scoped() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let first = j.create(&a, "workspace", "project").unwrap();
    let (second, lease, turn) = running(&j, &a);
    j.enqueue(&a, &first.id, "one", "first session").unwrap();
    j.output(&a, &second.id, &lease, &turn, "two actions from one chunk")
        .unwrap();
    let first_log = j.projected_actions(&a, &first.id, 0).unwrap();
    let second_log = j.projected_actions(&a, &second.id, 0).unwrap();
    assert!(second_log[1].server_seq < first_log[0].server_seq);
    assert!(second_log[2].server_seq > first_log[0].server_seq);
    assert_eq!(second_log[3].server_seq, second_log[2].server_seq + 1);
    let mut stranger = a.clone();
    stranger.membership.user_id = "other".into();
    stranger.project_grant.as_mut().unwrap().user_id = "other".into();
    assert!(j.projected_chat(&stranger, &first.id).is_err());
    assert!(j.projected_actions(&stranger, &second.id, 0).is_err());
}

#[test]
fn imported_timestamps_are_marked_and_pre_migration_cursors_require_a_snapshot() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let (s, lease, t) = running(&j, &a);
    j.output(&a, &s.id, &lease, &t, "retained history").unwrap();
    j.transition(&a, &s.id, &lease, &t, TurnPhase::Completed)
        .unwrap();
    c.execute_batch("DROP TABLE interactive_client_dispatch; DROP TABLE interactive_message_ids; DROP TABLE interactive_ahp_clients; DROP TABLE interactive_ahp_sessions; DROP TABLE interactive_ahp_actions; DROP TABLE interactive_ahp_chats; DROP TABLE interactive_ahp_clock; UPDATE interactive_events SET payload=json_remove(payload,'$._recorded_at');").unwrap();
    let tx = c.unchecked_transaction().unwrap();
    tx.execute_batch(PROJECTION_SCHEMA).unwrap();
    tx.execute_batch(crate::CHANNEL_SCHEMA).unwrap();
    j.backfill_projection().unwrap();
    tx.execute_batch(crate::DISPATCH_SCHEMA).unwrap();
    j.backfill_dispatch().unwrap();
    tx.commit().unwrap();
    let state = j.projected_chat(&a, &s.id).unwrap();
    assert_eq!(state.meta.unwrap()["dunlin.timestampImported"], true);
    assert_eq!(state.turns[0].state, TurnState::Complete);
    assert!(j.projected_actions(&a, &s.id, 0).is_err());
    let snapshot = j.projected_snapshot(&a, &s.id).unwrap();
    assert!(snapshot.from_seq > 0);
    let wire = serde_json::to_vec(&snapshot).unwrap();
    let decoded: ahp_types::state::Snapshot = serde_json::from_slice(&wire).unwrap();
    assert_eq!(decoded, snapshot);
    assert!(
        j.projected_actions(&a, &s.id, snapshot.from_seq as u64)
            .unwrap()
            .is_empty()
    );
    assert!(
        j.projected_actions(&a, &s.id, snapshot.from_seq as u64 + 1)
            .is_err()
    );
}
