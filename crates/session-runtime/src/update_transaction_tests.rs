//! A remote stream inbox and Journal projection must commit as one SQL cut.
use super::claim_tests::persisted_cut;
use super::*;
use serde_json::json;
use std::cell::Cell;

fn info() -> HarnessInfo {
    HarnessInfo {
        profile: None,
        remote_session_id: "verified-acp-session".into(),
        binding_revision: 1,
        model: "fixture-model".into(),
        mode: "fixture-mode".into(),
        max_seconds: 60,
        max_output_bytes: 65536,
    }
}

#[test]
fn inner_updates_require_transaction_and_keep_current_authority_and_lease_checks() {
    let connection = Connection::open_in_memory().unwrap();
    tests::setup(&connection);
    let context = tests::context();
    let journal = Journal::new(&connection);
    let session = journal.create(&context, "workspace", "project").unwrap();
    let turn = journal
        .enqueue(&context, &session.id, "turn", "prompt")
        .unwrap();
    let lease = journal
        .acquire(&context, &session.id, 0, "runtime")
        .unwrap();
    journal.claim_next(&context, &session.id, &lease).unwrap();
    journal
        .transition(&context, &session.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    let baseline = persisted_cut(&connection);
    assert!(
        journal
            .transition_timed_in_transaction(
                &context,
                &session.id,
                &lease,
                &turn,
                TurnPhase::Completed,
                None
            )
            .is_err()
    );
    assert!(
        journal
            .agent_update_in_transaction(&context, &session.id, &lease, &turn, json!({}))
            .is_err()
    );
    assert!(
        journal
            .harness_ready_in_transaction(&context, &session.id, &lease, info())
            .is_err()
    );
    assert!(
        journal
            .interrupt_in_transaction(&session.id, &lease, "test")
            .is_err()
    );
    let mut revoked = context.clone();
    revoked.project_grant = None;
    let mut stale = lease.clone();
    stale.epoch += 1;
    for (authority, lease) in [(&revoked, &lease), (&context, &stale)] {
        let transaction = connection.unchecked_transaction().unwrap();
        assert!(
            journal
                .transition_timed_in_transaction(
                    authority,
                    &session.id,
                    lease,
                    &turn,
                    TurnPhase::Completed,
                    None
                )
                .is_err()
        );
        assert!(
            journal
                .agent_update_in_transaction(authority, &session.id, lease, &turn, json!({}))
                .is_err()
        );
        assert!(
            journal
                .harness_ready_in_transaction(authority, &session.id, lease, info())
                .is_err()
        );
        transaction.rollback().unwrap();
    }
    let transaction = connection.unchecked_transaction().unwrap();
    assert!(
        journal
            .interrupt_in_transaction(&session.id, &stale, "stale")
            .is_err()
    );
    transaction.rollback().unwrap();
    assert_eq!(persisted_cut(&connection), baseline);
}

#[test]
fn late_inbox_failure_rolls_back_output_terminal_projection_and_watermark_before_notification() {
    let connection = Connection::open_in_memory().unwrap();
    tests::setup(&connection);
    connection
        .execute_batch("CREATE TABLE remote_inbox(sequence INTEGER PRIMARY KEY CHECK(sequence>0));")
        .unwrap();
    let context = tests::context();
    let journal = Journal::new(&connection);
    let session = journal.create(&context, "workspace", "project").unwrap();
    let turn = journal
        .enqueue(&context, &session.id, "turn", "prompt")
        .unwrap();
    let lease = journal
        .acquire(&context, &session.id, 0, "runtime")
        .unwrap();
    journal.claim_next(&context, &session.id, &lease).unwrap();
    journal
        .transition(&context, &session.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    let notifications = Cell::new(0);
    let journal =
        Journal::new(&connection).notifying(|| notifications.set(notifications.get() + 1));
    let baseline = persisted_cut(&connection);
    let update = json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"durable remote delta"}});
    {
        let transaction = connection.unchecked_transaction().unwrap();
        journal
            .agent_update_in_transaction(&context, &session.id, &lease, &turn, update.clone())
            .unwrap();
        journal
            .transition_timed_in_transaction(
                &context,
                &session.id,
                &lease,
                &turn,
                TurnPhase::Completed,
                None,
            )
            .unwrap();
        assert_ne!(persisted_cut(&connection), baseline);
        assert!(journal.notify_committed_update().is_err());
        assert!(
            transaction
                .execute("INSERT INTO remote_inbox VALUES(0)", [])
                .is_err()
        );
    }
    assert_eq!(persisted_cut(&connection), baseline);
    assert_eq!(notifications.get(), 0);
    let transaction = connection.unchecked_transaction().unwrap();
    journal
        .agent_update_in_transaction(&context, &session.id, &lease, &turn, update)
        .unwrap();
    journal
        .transition_timed_in_transaction(
            &context,
            &session.id,
            &lease,
            &turn,
            TurnPhase::Completed,
            None,
        )
        .unwrap();
    transaction
        .execute("INSERT INTO remote_inbox VALUES(1)", [])
        .unwrap();
    transaction.commit().unwrap();
    assert_eq!(notifications.get(), 0);
    journal.notify_committed_update().unwrap();
    assert_eq!(notifications.get(), 1);
    assert_eq!(
        journal.snapshot(&context, &session.id).unwrap().1[0].phase,
        TurnPhase::Completed
    );
    assert_eq!(
        journal
            .replay(&context, &session.id, 0)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "agent_update")
            .count(),
        1
    );
    assert!(
        serde_json::to_string(&journal.projected_chat(&context, &session.id).unwrap())
            .unwrap()
            .contains("durable remote delta")
    );
}

#[test]
fn ready_and_negative_interruption_roll_back_together_and_wrappers_notify_after_commit() {
    let connection = Connection::open_in_memory().unwrap();
    tests::setup(&connection);
    connection
        .execute_batch("CREATE TABLE remote_inbox(sequence INTEGER CHECK(sequence>0));")
        .unwrap();
    let context = tests::context();
    let notifications = Cell::new(0);
    let journal =
        Journal::new(&connection).notifying(|| notifications.set(notifications.get() + 1));
    let session = journal.create(&context, "workspace", "project").unwrap();
    let lease = journal
        .acquire(&context, &session.id, 0, "runtime")
        .unwrap();
    let baseline = persisted_cut(&connection);
    notifications.set(0);
    {
        let transaction = connection.unchecked_transaction().unwrap();
        journal
            .harness_ready_in_transaction(&context, &session.id, &lease, info())
            .unwrap();
        journal
            .interrupt_in_transaction(&session.id, &lease, "channel_lost")
            .unwrap();
        assert!(
            transaction
                .execute("INSERT INTO remote_inbox VALUES(0)", [])
                .is_err()
        );
    }
    assert_eq!(persisted_cut(&connection), baseline);
    assert_eq!(notifications.get(), 0);
    journal
        .harness_ready(&context, &session.id, &lease, info())
        .unwrap();
    assert_eq!(notifications.get(), 1);
    journal
        .interrupt(&session.id, &lease, "channel_lost")
        .unwrap();
    assert_eq!(notifications.get(), 2);
    assert_eq!(
        journal.snapshot(&context, &session.id).unwrap().0.phase,
        SessionPhase::Interrupted
    );
}
