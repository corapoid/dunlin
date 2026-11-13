use super::*;
use crate::tests::{context, setup};
use ahp_types::actions::StateAction;
fn action(session: &str, seq: i64, id: &str) -> DispatchActionParams {
    serde_json::from_value(json!({"channel":format!("ahp-chat:/{session}"),"clientSeq":seq,"action":{"type":"chat/pendingMessageSet","kind":"queued","id":id,"message":{"text":"hello","origin":{"kind":"user"}}}})).unwrap()
}
#[test]
fn accepted_identity_message_mapping_retry_and_reopen_are_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let p = action(&s.id, 1, "client-message");
    let first = j
        .dispatch_text(&a, &s.id, "device", "client", &p, None)
        .unwrap();
    assert_eq!(first.action, p.action);
    assert_eq!(first.origin.as_ref().unwrap().client_seq, 1);
    assert_eq!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, Some("actor unavailable"))
            .unwrap(),
        first
    );
    let turn = j.claim_next(&a, &s.id, &lease).unwrap().unwrap();
    assert_ne!(turn.id, "client-message");
    let chat = j.projected_chat(&a, &s.id).unwrap();
    assert!(chat.queued_messages.as_ref().is_none_or(|q| q.is_empty()));
    let actions = j.projected_actions(&a, &s.id, 0).unwrap();
    let start = actions
        .iter()
        .find(|e| matches!(e.action, StateAction::ChatTurnStarted(_)))
        .unwrap();
    if let StateAction::ChatTurnStarted(start) = &start.action {
        assert_eq!(start.queued_message_id.as_deref(), Some("client-message"));
    }
    assert!(actions.iter().any(|e| e == &first));
    let mut mismatch = p.clone();
    mismatch.channel = "ahp-chat:/other".into();
    assert!(
        j.dispatch_text(&a, &s.id, "device", "client", &mismatch, None)
            .is_err()
    );
    let mut changed = p.clone();
    if let StateAction::ChatPendingMessageSet(m) = &mut changed.action {
        m.message.text = "changed".into();
    }
    assert!(matches!(
        j.dispatch_text(&a, &s.id, "device", "client", &changed, None),
        Err(Error::Conflict(_))
    ));
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c);
    assert_eq!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, Some("actor missing"))
            .unwrap(),
        first
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_turns", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn rejection_is_replayed_without_mutating_queue_or_chat_and_retry_keeps_outcome() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let before = j.projected_chat(&a, &s.id).unwrap();
    let seq = j.channel_clock().unwrap().sequence;
    let p = action(&s.id, 1, "rejected");
    let e = j
        .dispatch_text(&a, &s.id, "device", "client", &p, Some("actor unavailable"))
        .unwrap();
    assert_eq!(e.rejection_reason.as_deref(), Some("actor unavailable"));
    assert!(e.server_seq > seq);
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    assert_eq!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, None)
            .unwrap(),
        e
    );
    assert_eq!(
        j.projected_actions(&a, &s.id, seq).unwrap(),
        vec![e.clone()]
    );
    let ChannelReplay::Actions { actions, .. } = j
        .channel_replay(
            &[ChannelSubscription {
                context: &a,
                session: &s.id,
                chat: true,
            }],
            seq,
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(actions, vec![e]);
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_turns", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let mut revoked = a.clone();
    revoked.membership.active = false;
    assert!(
        j.dispatch_text(&revoked, &s.id, "device", "client", &p, None)
            .is_err()
    );
    let mut stranger = a.clone();
    stranger.membership.user_id = "other".into();
    assert!(
        j.dispatch_text(&stranger, &s.id, "device", "client", &p, None)
            .is_err()
    );
}
#[test]
fn projection_failure_rolls_back_queue_source_origin_and_ledger() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.acquire(&a, &s.id, 0, "worker").unwrap();
    c.execute("UPDATE interactive_ahp_chats SET action_count=4064", [])
        .unwrap();
    let before = j.session(&s.id).unwrap().sequence;
    let clock = j.channel_clock().unwrap();
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 1, "rollback"),
            None
        )
        .is_err()
    );
    assert_eq!(j.session(&s.id).unwrap().sequence, before);
    assert_eq!(j.channel_clock().unwrap().sequence, clock.sequence);
    for table in [
        "interactive_turns",
        "interactive_message_ids",
        "interactive_client_dispatch",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn unsupported_payload_and_full_queue_reject_but_queued_message_upserts() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.acquire(&a, &s.id, 0, "worker").unwrap();
    let mut p = action(&s.id, 1, "unsupported");
    if let StateAction::ChatPendingMessageSet(m) = &mut p.action {
        m.message.meta = Some(Default::default());
    }
    assert!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, None)
            .unwrap()
            .rejection_reason
            .is_some()
    );
    j.dispatch_text(
        &a,
        &s.id,
        "device",
        "client",
        &action(&s.id, 2, "one"),
        None,
    )
    .unwrap();
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 3, "one"),
            None
        )
        .unwrap()
        .rejection_reason
        .is_none()
    );
    for n in 0..31 {
        j.enqueue(&a, &s.id, &format!("key{n}"), "queued").unwrap();
    }
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 4, "full"),
            None
        )
        .unwrap()
        .rejection_reason
        .is_some()
    );
}
#[test]
fn ledger_insert_failure_rolls_back_an_already_projected_acceptance() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.acquire(&a, &s.id, 0, "worker").unwrap();
    c.execute_batch("CREATE TRIGGER reject_ledger BEFORE INSERT ON interactive_client_dispatch BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    let clock = j.channel_clock().unwrap().sequence;
    let source = j.session(&s.id).unwrap().sequence;
    let before = j.projected_chat(&a, &s.id).unwrap();
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 1, "atomic"),
            None
        )
        .is_err()
    );
    assert_eq!(j.channel_clock().unwrap().sequence, clock);
    assert_eq!(j.session(&s.id).unwrap().sequence, source);
    assert_eq!(j.projected_chat(&a, &s.id).unwrap(), before);
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_turns", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER reject_ledger").unwrap();
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 1, "atomic"),
            None
        )
        .unwrap()
        .rejection_reason
        .is_none()
    );
}
#[test]
fn ledger_identity_is_global_across_channels_and_capacity_never_evicts_acknowledgements() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let other = j.create(&a, "workspace", "project").unwrap();
    let p = action(&s.id, 1, "one");
    let first = j
        .dispatch_text(&a, &s.id, "device", "client", &p, Some("no actor"))
        .unwrap();
    assert!(matches!(
        j.dispatch_text(
            &a,
            &other.id,
            "device",
            "client",
            &action(&other.id, 1, "one"),
            Some("no actor")
        ),
        Err(Error::Conflict(_))
    ));
    c.execute_batch("WITH RECURSIVE n(i) AS (SELECT 2 UNION ALL SELECT i+1 FROM n WHERE i<1024) INSERT INTO interactive_client_dispatch SELECT 'device','reserved',i,'digest','{}' FROM n;").unwrap();
    assert!(matches!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 2, "new"),
            Some("no actor")
        ),
        Err(Error::Limit(_))
    ));
    assert_eq!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, None)
            .unwrap(),
        first
    );
}
#[test]
fn new_acceptance_revalidates_authority_but_old_ack_survives_actor_stop() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    assert!(j.authorize_dispatch(&a, &s.id, &lease).is_ok());
    let mut wrong_lease = lease.clone();
    wrong_lease.instance = "other-worker".into();
    assert!(j.authorize_dispatch(&a, &s.id, &wrong_lease).is_err());
    let p = action(&s.id, 1, "accepted");
    let accepted = j
        .dispatch_text(&a, &s.id, "device", "client", &p, None)
        .unwrap();
    let mut changed = a.clone();
    changed.membership.revision += 1;
    assert!(j.authorize_dispatch(&changed, &s.id, &lease).is_err());
    assert!(
        j.dispatch_text(
            &changed,
            &s.id,
            "device",
            "client",
            &action(&s.id, 2, "stale"),
            None
        )
        .is_err()
    );
    j.interrupt(&s.id, &lease, "stopped").unwrap();
    assert!(j.authorize_dispatch(&a, &s.id, &lease).is_err());
    assert_eq!(
        j.dispatch_text(&a, &s.id, "device", "client", &p, Some("actor stopped"))
            .unwrap(),
        accepted
    );
    assert!(
        j.dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 3, "after-stop"),
            None
        )
        .is_err()
    );
    let e = j
        .dispatch_text(
            &a,
            &s.id,
            "device",
            "client",
            &action(&s.id, 3, "after-stop"),
            Some("actor stopped"),
        )
        .unwrap();
    assert_eq!(e.rejection_reason.as_deref(), Some("actor stopped"));
}
