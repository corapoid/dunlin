use super::*;
use crate::tests::{context, setup};
fn subscriptions<'a>(a: &'a AccessContext, ids: &'a [Session]) -> Vec<Subscription<'a>> {
    ids.iter()
        .map(|s| Subscription {
            context: a,
            session: &s.id,
        })
        .collect()
}
fn events(replay: StreamReplay) -> (StreamCursor, Vec<StreamEvent>) {
    match replay {
        StreamReplay::Events { cursor, events } => (cursor, events),
        _ => panic!("Expected complete replay"),
    }
}
#[test]
fn cross_session_snapshot_and_replay_are_one_order_and_never_repeat_idempotent_writes() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let ids = [
        j.create(&a, "workspace", "project").unwrap(),
        j.create(&a, "workspace", "project").unwrap(),
    ];
    let subs = subscriptions(&a, &ids);
    let snapshot = j.snapshot_stream(&subs).unwrap();
    assert_eq!(snapshot.cursor.sequence, 2);
    j.enqueue(&a, &ids[1].id, "b", "second session").unwrap();
    j.enqueue(&a, &ids[0].id, "a", "first session").unwrap();
    j.enqueue(&a, &ids[1].id, "b", "second session").unwrap();
    let (cursor, log) = events(j.replay_stream(&subs, &snapshot.cursor).unwrap());
    assert_eq!(cursor.sequence, 4);
    assert_eq!(
        log.iter()
            .map(|e| (&e.session, e.server_sequence, e.event.sequence))
            .collect::<Vec<_>>(),
        vec![(&ids[1].id, 3, 2), (&ids[0].id, 4, 2)]
    );
    assert!(
        events(j.replay_stream(&subs, &cursor).unwrap())
            .1
            .is_empty()
    );
    let cut = j.snapshot_stream(&subs).unwrap();
    assert_eq!(cut.cursor, cursor);
    assert_eq!(cut.sessions[0].1[0].prompt, "first session");
    assert_eq!(cut.sessions[1].1[0].prompt, "second session");
}
#[test]
fn cursor_does_not_grant_private_session_access_and_replay_filters_before_limits() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    c.execute("INSERT INTO users VALUES('other')", []).unwrap();
    let mut other = a.clone();
    other.membership.user_id = "other".into();
    other.project_grant.as_mut().unwrap().user_id = "other".into();
    let own = j.create(&a, "workspace", "project").unwrap();
    let secret = j.create(&other, "workspace", "project").unwrap();
    let ids = [own];
    let subs = subscriptions(&a, &ids);
    let cursor = j.snapshot_stream(&subs).unwrap().cursor;
    let lease = j.acquire(&other, &secret.id, 0, "other-worker").unwrap();
    let t = j
        .enqueue(&other, &secret.id, "one", "other user's prompt")
        .unwrap();
    j.claim_next(&other, &secret.id, &lease).unwrap();
    j.transition(&other, &secret.id, &lease, &t, TurnPhase::Running)
        .unwrap();
    for _ in 0..140 {
        j.output(&other, &secret.id, &lease, &t, "secret marker")
            .unwrap();
    }
    let (cursor, log) = events(j.replay_stream(&subs, &cursor).unwrap());
    assert!(log.is_empty());
    assert!(cursor.sequence > 140);
    assert!(
        j.replay_stream(
            &[Subscription {
                context: &a,
                session: &secret.id
            }],
            &cursor
        )
        .is_err()
    );
    let mut revoked = a.clone();
    revoked.project_grant.as_mut().unwrap().permissions.clear();
    assert!(
        j.replay_stream(
            &[Subscription {
                context: &revoked,
                session: &ids[0].id
            }],
            &cursor
        )
        .is_err()
    );
    assert!(
        j.snapshot_stream(&[Subscription {
            context: &revoked,
            session: &ids[0].id
        }])
        .is_err()
    );
}
#[test]
fn replay_is_complete_or_requires_a_snapshot_for_budget_and_cursor_mismatch() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let ids = [j.create(&a, "workspace", "project").unwrap()];
    let subs = subscriptions(&a, &ids);
    let cursor = j.snapshot_stream(&subs).unwrap().cursor;
    let lease = j.acquire(&a, &ids[0].id, 0, "worker").unwrap();
    let t = j.enqueue(&a, &ids[0].id, "one", "hello").unwrap();
    j.claim_next(&a, &ids[0].id, &lease).unwrap();
    j.transition(&a, &ids[0].id, &lease, &t, TurnPhase::Running)
        .unwrap();
    // Byte budget, still fewer than 128 events.
    for _ in 0..35 {
        j.output(&a, &ids[0].id, &lease, &t, &"x".repeat(32768))
            .unwrap();
    }
    assert!(matches!(
        j.replay_stream(&subs, &cursor).unwrap(),
        StreamReplay::SnapshotRequired
    ));
    let recent = j.snapshot_stream(&subs).unwrap().cursor;
    for _ in 0..129 {
        j.output(&a, &ids[0].id, &lease, &t, "small").unwrap();
    }
    assert!(matches!(
        j.replay_stream(&subs, &recent).unwrap(),
        StreamReplay::SnapshotRequired
    ));
    let current = j.snapshot_stream(&subs).unwrap().cursor;
    for invalid in [
        StreamCursor {
            stream: "another database".into(),
            sequence: current.sequence,
        },
        StreamCursor {
            stream: current.stream,
            sequence: current.sequence + 1,
        },
    ] {
        assert!(matches!(
            j.replay_stream(&subs, &invalid).unwrap(),
            StreamReplay::SnapshotRequired
        ));
    }
    assert!(
        j.snapshot_stream(&[
            Subscription {
                context: &a,
                session: &ids[0].id
            },
            Subscription {
                context: &a,
                session: &ids[0].id
            }
        ])
        .is_err()
    );
}
#[test]
fn stream_and_journal_rollback_together_and_committed_cursors_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let ids = [j.create(&a, "workspace", "project").unwrap()];
    let subs = subscriptions(&a, &ids);
    let before = j.snapshot_stream(&subs).unwrap();
    c.execute_batch("CREATE TRIGGER reject_stream BEFORE INSERT ON interactive_stream_events BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(j.enqueue(&a, &ids[0].id, "one", "must roll back").is_err());
    let after = j.snapshot_stream(&subs).unwrap();
    assert_eq!(after.cursor, before.cursor);
    assert!(after.sessions[0].1.is_empty());
    c.execute_batch("DROP TRIGGER reject_stream;").unwrap();
    j.enqueue(&a, &ids[0].id, "one", "committed").unwrap();
    drop(j);
    drop(c);
    let reopened = Connection::open(&path).unwrap();
    let j = Journal::new(&reopened);
    let (cursor, log) = events(j.replay_stream(&subs, &before.cursor).unwrap());
    assert_eq!(cursor.stream, before.cursor.stream);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].server_sequence, before.cursor.sequence + 1);
    assert_eq!(
        j.snapshot_stream(&subs).unwrap().sessions[0].1[0].prompt,
        "committed"
    );
}
#[test]
fn migration_backfill_is_a_snapshot_baseline_and_new_records_follow_it() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    c.execute_batch("DROP TRIGGER interactive_event_stream; DROP TABLE interactive_stream_events; DROP TABLE interactive_stream_clock;").unwrap();
    let j = Journal::new(&c);
    let a = context();
    let ids = [
        j.create(&a, "workspace", "project").unwrap(),
        j.create(&a, "workspace", "project").unwrap(),
    ];
    j.enqueue(&a, &ids[0].id, "one", "old prompt").unwrap();
    c.execute_batch(STREAM_SCHEMA).unwrap();
    let subs = subscriptions(&a, &ids);
    let baseline = j.snapshot_stream(&subs).unwrap().cursor;
    assert_eq!(baseline.sequence, 3);
    assert!(matches!(
        j.replay_stream(
            &subs,
            &StreamCursor {
                stream: baseline.stream.clone(),
                sequence: 0
            }
        )
        .unwrap(),
        StreamReplay::SnapshotRequired
    ));
    j.enqueue(&a, &ids[1].id, "two", "new prompt").unwrap();
    let (_, log) = events(j.replay_stream(&subs, &baseline).unwrap());
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].server_sequence, 4);
    // A deleted clock must fail the entire write, not silently omit the outbox.
    c.execute("DELETE FROM interactive_stream_clock", [])
        .unwrap();
    assert!(j.enqueue(&a, &ids[1].id, "three", "not committed").is_err());
    assert_eq!(j.snapshot(&a, &ids[1].id).unwrap().1.len(), 1);
}

#[test]
fn snapshot_budget_counts_serialized_prompts_and_clock_exhaustion_is_atomic() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let j = Journal::new(&c);
    let a = context();
    let ids = [
        j.create(&a, "workspace", "project").unwrap(),
        j.create(&a, "workspace", "project").unwrap(),
    ];
    let subs = subscriptions(&a, &ids);
    // Escaped control characters expand in JSON; raw-length checks alone do not suffice.
    for n in 0..8 {
        j.enqueue(
            &a,
            &ids[0].id,
            &format!("key-{n}"),
            &format!("x{}", "\u{0001}".repeat(32760)),
        )
        .unwrap();
    }
    assert!(j.snapshot_stream(&subs).is_err());
    c.execute(
        "UPDATE interactive_stream_clock SET sequence=9007199254740991",
        [],
    )
    .unwrap();
    assert!(
        j.enqueue(&a, &ids[1].id, "no-space", "not committed")
            .is_err()
    );
    assert!(j.snapshot(&a, &ids[1].id).unwrap().1.is_empty());
    let clock: u64 = c
        .query_row("SELECT sequence FROM interactive_stream_clock", [], |r| {
            nonnegative(r, 0)
        })
        .unwrap();
    assert_eq!(clock, 9_007_199_254_740_991);
}
