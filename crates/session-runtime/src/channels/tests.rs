use super::*;
use crate::tests::{context, setup};

fn history_turn(
    j: &Journal<'_>,
    a: &AccessContext,
    id: &str,
    lease: &Lease,
    key: &str,
    text: &str,
) -> String {
    let turn = j.enqueue(a, id, key, key).unwrap();
    j.claim_next(a, id, lease).unwrap();
    j.transition(a, id, lease, &turn, TurnPhase::Running)
        .unwrap();
    for chunk in text.as_bytes().chunks(32768) {
        j.output(a, id, lease, &turn, std::str::from_utf8(chunk).unwrap())
            .unwrap();
    }
    j.transition_timed(a, id, lease, &turn, TurnPhase::Completed, Some(1))
        .unwrap();
    turn
}

#[test]
fn explicit_chat_view_freezes_older_completed_turns_and_keeps_active_and_queue() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "history-worker").unwrap();
    let ids = (0..6)
        .map(|i| history_turn(&j, &a, &s.id, &lease, &format!("history-{i}"), "completed"))
        .collect::<Vec<_>>();
    let active = j.enqueue(&a, &s.id, "active", "active").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    j.transition(&a, &s.id, &lease, &active, TurnPhase::Running)
        .unwrap();
    j.output(&a, &s.id, &lease, &active, "active output")
        .unwrap();
    j.enqueue(&a, &s.id, "queued", "queued").unwrap();
    let channel = format!("ahp-chat:/{}", s.id);
    let subscriptions = [ChannelSubscription {
        context: &a,
        session: &s.id,
        chat: true,
    }];
    let full = j.channel_snapshots(&subscriptions).unwrap();
    let SnapshotState::Chat(chat) = &full.snapshots[0].state else {
        panic!("chat")
    };
    assert_eq!(chat.turns.len(), 6);
    let cut = j
        .channel_snapshots_with_views(
            &subscriptions,
            &std::collections::BTreeMap::from([(channel, 2)]),
        )
        .unwrap();
    let SnapshotState::Chat(chat) = &cut.snapshots[0].state else {
        panic!("chat")
    };
    assert_eq!(
        chat.turns.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ids[4..]
    );
    assert!(chat.active_turn.is_some());
    assert_eq!(chat.queued_messages.as_ref().unwrap().len(), 1);
    let frozen = &cut.histories[0];
    assert_eq!(frozen.history.cut, cut.cursor);
    assert_eq!(frozen.before, 4);
    // Future canonical activity does not alter this connection's historical cut.
    j.transition_timed(&a, &s.id, &lease, &active, TurnPhase::Completed, Some(2))
        .unwrap();
    let page = frozen.history.page(frozen.before).unwrap();
    assert_eq!(
        page.turns.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ids[..4]
    );
    assert!(page.next_before.is_none());
    assert!(!page.turns.iter().any(|t| t.id == active));
    assert_eq!(j.projected_chat(&a, &s.id).unwrap().turns.len(), 7);
}

#[test]
fn chat_view_adapts_to_byte_budgets_without_silently_truncating_default_snapshots() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "history-worker").unwrap();
    let text = "x".repeat(100 * 1024);
    let ids = (0..20)
        .map(|i| history_turn(&j, &a, &s.id, &lease, &format!("large-{i}"), &text))
        .collect::<Vec<_>>();
    let subscription = [ChannelSubscription {
        context: &a,
        session: &s.id,
        chat: true,
    }];
    assert!(
        matches!(j.channel_snapshots(&subscription), Err(Error::Limit(_))),
        "Absent view means all retained or explicit failure"
    );
    let cut = j
        .channel_snapshots_with_views(
            &subscription,
            &std::collections::BTreeMap::from([(format!("ahp-chat:/{}", s.id), 32)]),
        )
        .unwrap();
    assert!(serde_json::to_vec(&cut.snapshots).unwrap().len() < 901 * 1024);
    let SnapshotState::Chat(chat) = &cut.snapshots[0].state else {
        panic!("chat")
    };
    assert!(!chat.turns.is_empty() && chat.turns.len() < 20);
    let frozen = &cut.histories[0];
    let mut loaded = chat.turns.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let mut before = Some(frozen.before);
    while let Some(boundary) = before {
        let page = frozen.history.page(boundary).unwrap();
        assert!(page.turns.len() <= 32);
        assert!(serde_json::to_vec(&page.turns).unwrap().len() <= 513 * 1024);
        let mut older = page.turns.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        older.extend(loaded);
        loaded = older;
        before = page.next_before;
    }
    assert_eq!(loaded, ids);
    let count: i64 = c
        .query_row("SELECT count(*) FROM interactive_ahp_actions", [], |r| {
            r.get(0)
        })
        .unwrap();
    let seq = j.reserve_history_sequence(&a, &s.id).unwrap();
    assert!(seq > cut.cursor.sequence);
    assert_eq!(
        c.query_row::<i64, _, _>("SELECT count(*) FROM interactive_ahp_actions", [], |r| r
            .get(0))
            .unwrap(),
        count,
        "History read must not journal private page payloads"
    );
    let ChannelReplay::Actions { cursor, actions } = j
        .channel_replay(&subscription, cut.cursor.sequence)
        .unwrap()
    else {
        panic!("replay")
    };
    assert_eq!(cursor.sequence, seq);
    assert!(
        actions.is_empty(),
        "Other observers see a legal sequence gap, no history action"
    );
    assert!(
        j.projected_chat(&a, &s.id)
            .unwrap()
            .turns_next_cursor
            .is_none()
    );
}

#[test]
fn history_clock_overflow_and_invalid_authority_never_advance_the_clock() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let original = j.channel_clock().unwrap();
    let mut denied = a.clone();
    denied.membership.active = false;
    assert!(j.reserve_history_sequence(&denied, &s.id).is_err());
    assert_eq!(j.channel_clock().unwrap(), original);
    c.execute(
        "UPDATE interactive_ahp_clock SET sequence=9007199254740991",
        [],
    )
    .unwrap();
    assert!(matches!(
        j.reserve_history_sequence(&a, &s.id),
        Err(Error::Limit(_))
    ));
    assert_eq!(j.channel_clock().unwrap().sequence, 9_007_199_254_740_991);
}

#[test]
fn multi_chat_view_history_turn_budget_fails_explicitly_at_the_boundary() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let first = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &first.id, 0, "worker").unwrap();
    history_turn(&j, &a, &first.id, &lease, "template", "small");
    let template = j.projected_chat(&a, &first.id).unwrap().turns[0].clone();
    let ids = (0..3)
        .map(|_| j.create(&a, "workspace", "project").unwrap().id)
        .collect::<Vec<_>>();
    for (i, id) in ids.iter().enumerate() {
        let mut chat = j.projected_chat(&a, id).unwrap();
        chat.turns = (0..if i < 2 { 256 } else { 1 })
            .map(|n| {
                let mut t = template.clone();
                t.id = format!("imported-{i}-{n}");
                t
            })
            .collect();
        c.execute(
            "UPDATE interactive_ahp_chats SET state=?2 WHERE session=?1",
            params![id, serde_json::to_string(&chat).unwrap()],
        )
        .unwrap();
    }
    let subscriptions = ids
        .iter()
        .map(|id| ChannelSubscription {
            context: &a,
            session: id,
            chat: true,
        })
        .collect::<Vec<_>>();
    let views = ids
        .iter()
        .map(|id| (format!("ahp-chat:/{id}"), 0))
        .collect();
    assert!(matches!(
        j.channel_snapshots_with_views(&subscriptions, &views),
        Err(Error::Limit(_))
    ));
}
#[test]
fn session_catalog_and_root_summary_track_the_chat_and_shared_action_order() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let cut = j
        .channel_snapshots(&[
            ChannelSubscription {
                context: &a,
                session: &s.id,
                chat: false,
            },
            ChannelSubscription {
                context: &a,
                session: &s.id,
                chat: true,
            },
        ])
        .unwrap();
    assert_eq!(cut.snapshots[0].from_seq, cut.snapshots[1].from_seq);
    let mut observer = match &cut.snapshots[0].state {
        SnapshotState::Session(s) => (**s).clone(),
        _ => panic!("session"),
    };
    let t = j.enqueue(&a, &s.id, "one", "prompt").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    j.transition(&a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    let summary = j.channel_summary(&a, &s.id).unwrap();
    let chat = j.projected_chat(&a, &s.id).unwrap();
    assert_eq!(summary.status & 8, 8);
    assert_eq!(summary.chats.as_ref().unwrap()[0].status, Some(chat.status));
    let replay = j
        .channel_replay(
            &[
                ChannelSubscription {
                    context: &a,
                    session: &s.id,
                    chat: false,
                },
                ChannelSubscription {
                    context: &a,
                    session: &s.id,
                    chat: true,
                },
            ],
            cut.cursor.sequence,
        )
        .unwrap();
    if let ChannelReplay::Actions { actions, .. } = replay {
        assert!(
            actions
                .windows(2)
                .all(|a| a[0].server_seq < a[1].server_seq)
        );
        for envelope in actions
            .iter()
            .filter(|a| a.channel.starts_with("ahp-session:/"))
        {
            assert_eq!(
                apply_action_to_session(&mut observer, &envelope.action),
                ReduceOutcome::Applied
            );
        }
        assert_eq!(observer.chats[0].status, chat.status);
        // Upstream 1.0.0 does not aggregate top-level activity bits in this reducer.
        // This gap must remain explicit in interoperability acceptance.
        assert_eq!(observer.status, 1);
        assert_eq!(j.channel_session(&a, &s.id).unwrap().status, 8);
    } else {
        panic!("Expected complete replay");
    }
}
#[test]
fn bounded_multi_channel_replay_and_grant_revalidation_require_resync_or_deny() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &s.id, 0, "worker").unwrap();
    let cursor = j.channel_clock().unwrap();
    let t = j.enqueue(&a, &s.id, "one", "prompt").unwrap();
    j.claim_next(&a, &s.id, &lease).unwrap();
    j.transition(&a, &s.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    for _ in 0..129 {
        j.output(&a, &s.id, &lease, &t, "text").unwrap();
    }
    let subs = [ChannelSubscription {
        context: &a,
        session: &s.id,
        chat: true,
    }];
    assert!(matches!(
        j.channel_replay(&subs, cursor.sequence).unwrap(),
        ChannelReplay::SnapshotRequired
    ));
    let mut revoked = a.clone();
    revoked.membership.active = false;
    assert!(
        j.channel_replay(
            &[ChannelSubscription {
                context: &revoked,
                session: &s.id,
                chat: true
            }],
            cursor.sequence
        )
        .is_err()
    );
    assert!(
        j.channel_snapshots(&[ChannelSubscription {
            context: &revoked,
            session: &s.id,
            chat: false
        }])
        .is_err()
    );
}
#[test]
fn invalidation_is_after_commit_and_failed_or_idempotent_writes_do_not_notify() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let count = std::cell::Cell::new(0);
    let j = Journal::new(&c).notifying(|| count.set(count.get() + 1));
    let s = j.create(&a, "workspace", "project").unwrap();
    let original = count.get();
    j.enqueue(&a, &s.id, "one", "prompt").unwrap();
    let queued = count.get();
    assert_eq!(queued, original + 1);
    j.enqueue(&a, &s.id, "one", "prompt").unwrap();
    assert_eq!(count.get(), queued);
    c.execute_batch("CREATE TRIGGER fail_projection BEFORE INSERT ON interactive_ahp_actions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(j.enqueue(&a, &s.id, "two", "failed prompt").is_err());
    assert_eq!(count.get(), queued);
    assert_eq!(j.snapshot(&a, &s.id).unwrap().1.len(), 1);
}

#[test]
fn migration_baseline_accepts_a_valid_terminal_journal_in_the_recovery_reserve() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    c.execute(
        "UPDATE interactive_ahp_chats SET action_count=4065 WHERE session=?1",
        [&s.id],
    )
    .unwrap();
    c.execute_batch("DROP TABLE interactive_ahp_clients; DROP TABLE interactive_ahp_sessions; ALTER TABLE interactive_ahp_actions DROP COLUMN channel;").unwrap();
    let tx = c.unchecked_transaction().unwrap();
    tx.execute_batch(CHANNEL_SCHEMA).unwrap();
    j.backfill_channels().unwrap();
    tx.commit().unwrap();
    assert!(j.channel_summary(&a, &s.id).is_ok());
    let count: i64 = c
        .query_row(
            "SELECT action_count FROM interactive_ahp_chats WHERE session=?1",
            [&s.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 4065);
}
