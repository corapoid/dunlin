//! Outer transaction ownership for atomic turn claim plus sibling admission.
use super::*;
use std::cell::Cell;

pub(super) fn persisted_cut(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "interactive_sessions",
        "interactive_turns",
        "interactive_events",
        "interactive_ahp_clock",
        "interactive_ahp_chats",
        "interactive_ahp_actions",
        "interactive_ahp_sessions",
    ]
    .into_iter()
    .map(|table| {
        let mut query = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<rusqlite::Result<Vec<Vec<rusqlite::types::Value>>>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn inner_claim_requires_transaction_and_preserves_lease_and_authority_checks() {
    let c = Connection::open_in_memory().unwrap();
    tests::setup(&c);
    let a = tests::context();
    let notifications = Cell::new(0);
    let j = Journal::new(&c).notifying(|| notifications.set(notifications.get() + 1));
    let session = j.create(&a, "workspace", "project").unwrap();
    j.enqueue(&a, &session.id, "key", "prompt").unwrap();
    let lease = j.acquire(&a, &session.id, 0, "runtime").unwrap();
    notifications.set(0);
    let baseline = persisted_cut(&c);
    assert!(matches!(
        j.claim_next_in_transaction(&a, &session.id, &lease),
        Err(Error::Conflict(
            "Turn claim requires an existing SQL transaction"
        ))
    ));
    for (context, lease) in [
        (
            a.clone(),
            Lease {
                epoch: lease.epoch + 1,
                instance: lease.instance.clone(),
            },
        ),
        (
            a.clone(),
            Lease {
                epoch: lease.epoch,
                instance: "other-runtime".into(),
            },
        ),
        (
            {
                let mut changed = a.clone();
                changed.membership.revision += 1;
                changed
            },
            lease.clone(),
        ),
        (
            {
                let mut revoked = a.clone();
                revoked.project_grant = None;
                revoked
            },
            lease.clone(),
        ),
    ] {
        let tx = c.unchecked_transaction().unwrap();
        assert!(
            j.claim_next_in_transaction(&context, &session.id, &lease)
                .is_err()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(persisted_cut(&c), baseline);
    assert_eq!(notifications.get(), 0);
}

#[test]
fn late_sibling_failure_rolls_back_claim_events_projection_and_sequences_without_notification() {
    let c = Connection::open_in_memory().unwrap();
    tests::setup(&c);
    c.execute_batch("CREATE TABLE sibling_admission(value INTEGER CHECK(value>0))")
        .unwrap();
    let a = tests::context();
    let j = Journal::new(&c);
    let session = j.create(&a, "workspace", "project").unwrap();
    let turn = j.enqueue(&a, &session.id, "key", "prompt").unwrap();
    let lease = j.acquire(&a, &session.id, 0, "runtime").unwrap();
    let notifications = Cell::new(0);
    let j = Journal::new(&c).notifying(|| notifications.set(notifications.get() + 1));
    let baseline = persisted_cut(&c);
    let tx = c.unchecked_transaction().unwrap();
    let claimed = j
        .claim_next_in_transaction(&a, &session.id, &lease)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, turn);
    assert_eq!(claimed.phase, TurnPhase::Dispatching);
    assert_ne!(persisted_cut(&c), baseline);
    assert!(j.notify_committed_claim().is_err());
    assert_eq!(notifications.get(), 0);
    assert!(
        tx.execute("INSERT INTO sibling_admission VALUES(0)", [])
            .is_err()
    );
    drop(tx);
    assert_eq!(persisted_cut(&c), baseline);
    assert_eq!(
        j.snapshot(&a, &session.id).unwrap().1[0].phase,
        TurnPhase::Queued
    );
    assert_eq!(notifications.get(), 0);
    // The rollback left no active turn or consumed queue item.
    assert_eq!(
        j.claim_next(&a, &session.id, &lease).unwrap().unwrap().id,
        turn
    );
    assert_eq!(notifications.get(), 1);
}

#[test]
fn outer_commit_notifies_once_explicitly_and_wrapper_empty_claim_does_not_notify() {
    let c = Connection::open_in_memory().unwrap();
    tests::setup(&c);
    let a = tests::context();
    let notifications = Cell::new(0);
    let j = Journal::new(&c).notifying(|| notifications.set(notifications.get() + 1));
    let session = j.create(&a, "workspace", "project").unwrap();
    let lease = j.acquire(&a, &session.id, 0, "runtime").unwrap();
    notifications.set(0);
    assert!(j.claim_next(&a, &session.id, &lease).unwrap().is_none());
    assert_eq!(notifications.get(), 0);
    j.enqueue(&a, &session.id, "key", "prompt").unwrap();
    notifications.set(0);
    let sequence = j.sequence(&a, &session.id).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    assert!(
        j.claim_next_in_transaction(&a, &session.id, &lease)
            .unwrap()
            .is_some()
    );
    assert_eq!(notifications.get(), 0);
    assert!(j.notify_committed_claim().is_err());
    tx.commit().unwrap();
    assert_eq!(notifications.get(), 0);
    j.notify_committed_claim().unwrap();
    assert_eq!(notifications.get(), 1);
    assert_eq!(j.sequence(&a, &session.id).unwrap(), sequence + 1);
    assert_eq!(
        j.snapshot(&a, &session.id).unwrap().1[0].phase,
        TurnPhase::Dispatching
    );
    let tx = c.unchecked_transaction().unwrap();
    assert!(
        j.claim_next_in_transaction(&a, &session.id, &lease)
            .is_err()
    );
    tx.rollback().unwrap();
    assert_eq!(notifications.get(), 1);
}
