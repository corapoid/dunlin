use super::*;
use crate::tests::{context, setup};
use serde_json::json;
const KEY: &str = "admission-key-0001";
const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn admit(j: &Journal<'_>, context: &AccessContext, key: &str) -> Result<AdmissionResult> {
    j.create_admitted(context, "workspace", "project", "device", key, FP, "actor")
}
fn response(session: &Session) -> serde_json::Value {
    serde_json::to_value(session).unwrap()
}
#[test]
fn exact_admission_retry_reopens_with_original_response_and_never_revives_recovered_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    assert!(j.admission_lookup(&a, "device", KEY, FP).unwrap().is_none());
    let admitted = admit(&j, &a, KEY).unwrap();
    let lease = admitted.lease.unwrap();
    assert_eq!(lease.epoch, 1);
    assert_eq!(admitted.session.phase, SessionPhase::Active);
    let original = response(&admitted.session);
    assert!(original.get("runtime").is_none());
    assert!(original.get("authority").is_none());
    let turn = j
        .enqueue(&a, &admitted.session.id, "prompt", "uncertain work")
        .unwrap();
    j.claim_next(&a, &admitted.session.id, &lease).unwrap();
    j.transition(&a, &admitted.session.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    assert_eq!(j.recover().unwrap(), 1);
    let clock = j.channel_clock().unwrap();
    drop(j);
    drop(c);
    let c = Connection::open(path).unwrap();
    let j = Journal::new(&c);
    assert_eq!(
        response(&j.admission_lookup(&a, "device", KEY, FP).unwrap().unwrap()),
        original
    );
    // Runtime instance validation is only relevant to NEW admission, not retry.
    let retry = j
        .create_admitted(&a, "workspace", "project", "device", KEY, FP, "")
        .unwrap();
    assert!(retry.lease.is_none());
    assert_eq!(response(&retry.session), original);
    assert_eq!(j.channel_clock().unwrap(), clock);
    let actual = j.snapshot(&a, &admitted.session.id).unwrap();
    assert_eq!(actual.0.phase, SessionPhase::Interrupted);
    assert_eq!(actual.1[0].phase, TurnPhase::Unknown);
    assert!(j.claim_next(&a, &admitted.session.id, &lease).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_sessions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn admission_is_bound_to_fingerprint_owner_device_scope_and_current_human_authority() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let accepted = admit(&j, &a, KEY).unwrap();
    assert!(matches!(
        j.admission_lookup(&a, "device", KEY, &"a".repeat(64)),
        Err(Error::Conflict(_))
    ));
    let mut revoked = a.clone();
    revoked.membership.active = false;
    assert!(j.admission_lookup(&revoked, "device", KEY, FP).is_err());
    let mut service = a.clone();
    service.principal = PrincipalKind::Service;
    assert!(admit(&j, &service, KEY).is_err());
    let mut other_project = a.clone();
    other_project.project_grant.as_mut().unwrap().project_id = "other".into();
    assert!(matches!(
        j.admission_lookup(&other_project, "device", KEY, FP),
        Err(Error::Conflict(
            "Admission key was used with another request"
        ))
    ));
    assert!(matches!(
        j.create_admitted(
            &other_project,
            "workspace",
            "other",
            "device",
            KEY,
            FP,
            "actor"
        ),
        Err(Error::Conflict(_))
    ));
    let mut stranger = a.clone();
    stranger.membership.user_id = "other".into();
    stranger.project_grant.as_mut().unwrap().user_id = "other".into();
    assert!(j.admission_lookup(&stranger, "device", KEY, FP).is_err());
    assert!(
        j.admission_lookup(&stranger, "device", "different-key-0001", FP)
            .is_err()
    );
    let another_device = j
        .create_admitted(
            &a,
            "workspace",
            "project",
            "device-two",
            KEY,
            FP,
            "actor-two",
        )
        .unwrap();
    assert!(another_device.lease.is_some());
    assert_ne!(another_device.session.id, accepted.session.id);
    for key in [
        "short",
        "bad key with space",
        "0123456789abcdeé",
        "0123456789abcdef\n",
    ] {
        assert!(admit(&j, &a, key).is_err());
    }
    assert!(admit(&j, &a, &"a".repeat(129)).is_err());
    for fingerprint in ["A".repeat(64), "a".repeat(63), "x".repeat(64)] {
        assert!(j.admission_lookup(&a, "device", KEY, &fingerprint).is_err());
    }
}
#[test]
fn admission_late_ledger_or_acquire_failure_rolls_back_every_projection_and_notifies_only_commit() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let changed = std::cell::Cell::new(0);
    let j = Journal::new(&c).notifying(|| changed.set(changed.get() + 1));
    let before = j.channel_clock().unwrap();
    c.execute_batch("CREATE TRIGGER fail_admission BEFORE INSERT ON interactive_session_admissions BEGIN SELECT RAISE(ABORT,'admission injection'); END;").unwrap();
    assert!(admit(&j, &a, KEY).is_err());
    assert_eq!(changed.get(), 0);
    for table in [
        "interactive_sessions",
        "interactive_events",
        "interactive_ahp_chats",
        "interactive_ahp_sessions",
        "interactive_ahp_actions",
        "interactive_session_admissions",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "Rollback left rows in {table}"
        );
    }
    assert_eq!(j.channel_clock().unwrap(), before);
    c.execute_batch("DROP TRIGGER fail_admission;").unwrap();
    assert!(
        j.create_admitted(&a, "workspace", "project", "device", KEY, FP, "")
            .is_err()
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_sessions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(changed.get(), 0);
    admit(&j, &a, KEY).unwrap();
    assert_eq!(changed.get(), 1);
    admit(&j, &a, KEY).unwrap();
    assert_eq!(changed.get(), 1);
}
#[test]
fn admission_capacity_fails_closed_without_evicting_existing_accepted_identity() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let accepted = admit(&j, &a, KEY).unwrap();
    for index in 1..MAX_DEVICE_ADMISSIONS {
        let session = format!("fixture-{index}");
        c.execute("INSERT INTO interactive_sessions(id,workspace,project,owner,phase) VALUES(?1,'workspace','project','owner','pending')",[&session]).unwrap();
        c.execute("INSERT INTO interactive_session_admissions VALUES('device',?1,'owner','workspace','project',?2,?3,?4)",params![format!("fixture-key-{index:04}"),FP,session,json!({"id":session,"workspace":"workspace","project":"project","owner":"owner","phase":"pending","epoch":0,"sequence":0}).to_string()]).unwrap();
    }
    assert!(matches!(
        admit(&j, &a, "new-request-000001"),
        Err(Error::Limit("Session admission ledger capacity reached"))
    ));
    let retry = admit(&j, &a, KEY).unwrap();
    assert!(retry.lease.is_none());
    assert_eq!(response(&retry.session), response(&accepted.session));
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_session_admissions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        MAX_DEVICE_ADMISSIONS
    );
    for index in MAX_DEVICE_ADMISSIONS..MAX_ADMISSIONS {
        let session = format!("fixture-{index}");
        c.execute("INSERT INTO interactive_sessions(id,workspace,project,owner,phase) VALUES(?1,'workspace','project','owner','pending')",[&session]).unwrap();
        c.execute("INSERT INTO interactive_session_admissions VALUES(?1,?2,'owner','workspace','project',?3,?4,?5)",params![format!("quota-device-{}", index / MAX_DEVICE_ADMISSIONS),format!("fixture-key-{index:04}"),FP,session,json!({"id":session,"workspace":"workspace","project":"project","owner":"owner","phase":"pending","epoch":0,"sequence":0}).to_string()]).unwrap();
    }
    assert!(matches!(
        j.create_admitted(
            &a,
            "workspace",
            "project",
            "fresh-device",
            "new-request-000002",
            FP,
            "actor"
        ),
        Err(Error::Limit("Session admission ledger capacity reached"))
    ));
    let retry = admit(&j, &a, KEY).unwrap();
    assert!(retry.lease.is_none());
    assert_eq!(response(&retry.session), response(&accepted.session));
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_session_admissions",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        MAX_ADMISSIONS
    );
}
#[test]
fn racing_connections_share_one_atomic_admission_and_only_one_original_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    drop(c);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for index in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let c = Connection::open(path).unwrap();
            c.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
            let j = Journal::new(&c);
            let a = context();
            barrier.wait();
            j.create_admitted(
                &a,
                "workspace",
                "project",
                "device",
                KEY,
                FP,
                &format!("actor-{index}"),
            )
            .unwrap()
        }));
    }
    let first = threads.remove(0).join().unwrap();
    let second = threads.remove(0).join().unwrap();
    assert_eq!(response(&first.session), response(&second.session));
    assert_ne!(first.lease.is_some(), second.lease.is_some());
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM interactive_sessions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM interactive_session_admissions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn profile_pin_is_atomic_with_admission_and_survives_recovery_with_fresh_read_authority() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    c.execute_batch(ADMISSION_PROFILE_SCHEMA).unwrap();
    let a = context();
    let j = Journal::new(&c);
    let profile = dunlin_domain::agent::AgentProfileRef {
        agent_id: "agt-fixture".into(),
        version: 1,
        digest: "a".repeat(64),
    };
    c.execute_batch("CREATE TRIGGER fail_profile BEFORE INSERT ON interactive_session_profiles BEGIN SELECT RAISE(ABORT,'late profile failure'); END;").unwrap();
    assert!(
        j.create_admitted_profile(
            &a,
            "workspace",
            "project",
            "device",
            KEY,
            FP,
            "actor",
            Some(&profile)
        )
        .is_err()
    );
    for table in [
        "interactive_sessions",
        "interactive_session_admissions",
        "interactive_session_profiles",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    c.execute_batch("DROP TRIGGER fail_profile;").unwrap();
    let accepted = j
        .create_admitted_profile(
            &a,
            "workspace",
            "project",
            "device",
            KEY,
            FP,
            "actor",
            Some(&profile),
        )
        .unwrap();
    assert_eq!(
        j.session_profile(&a, &accepted.session.id).unwrap(),
        Some(profile.clone())
    );
    assert_eq!(j.recover().unwrap(), 1);
    let retry = j
        .create_admitted_profile(
            &a,
            "workspace",
            "project",
            "device",
            KEY,
            FP,
            "actor",
            Some(&profile),
        )
        .unwrap();
    assert!(retry.lease.is_none());
    assert_eq!(retry.session.id, accepted.session.id);
    assert_eq!(
        j.session_profile(&a, &retry.session.id).unwrap(),
        Some(profile)
    );
    let mut revoked = a;
    revoked.membership.active = false;
    assert!(j.session_profile(&revoked, &retry.session.id).is_err());
}

#[test]
fn caller_owned_admission_transaction_rolls_back_session_profile_receipt_and_notification() {
    use std::cell::Cell;
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    c.execute_batch(ADMISSION_PROFILE_SCHEMA).unwrap();
    let a = context();
    let notifications = Cell::new(0);
    let j = Journal::new(&c).notifying(|| notifications.set(notifications.get() + 1));
    let profile = dunlin_domain::agent::AgentProfileRef {
        agent_id: "agent".into(),
        version: 1,
        digest: "a".repeat(64),
    };
    assert!(
        j.create_admitted_profile_in_transaction(
            &a,
            "workspace",
            "project",
            "device",
            KEY,
            FP,
            "actor",
            Some(&profile)
        )
        .is_err()
    );
    {
        let tx = rusqlite::Transaction::new_unchecked(&c, rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let accepted = j
            .create_admitted_profile_in_transaction(
                &a,
                "workspace",
                "project",
                "device",
                KEY,
                FP,
                "actor",
                Some(&profile),
            )
            .unwrap();
        assert!(accepted.lease.is_some());
        assert!(j.notify_committed_admission().is_err());
        assert_eq!(notifications.get(), 0);
        tx.rollback().unwrap();
    }
    for table in [
        "interactive_sessions",
        "interactive_session_profiles",
        "interactive_session_admissions",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(notifications.get(), 0);
    let tx =
        rusqlite::Transaction::new_unchecked(&c, rusqlite::TransactionBehavior::Immediate).unwrap();
    let accepted = j
        .create_admitted_profile_in_transaction(
            &a,
            "workspace",
            "project",
            "device",
            KEY,
            FP,
            "actor",
            Some(&profile),
        )
        .unwrap();
    tx.commit().unwrap();
    j.notify_committed_admission().unwrap();
    assert_eq!(notifications.get(), 1);
    assert_eq!(
        j.session_profile(&a, &accepted.session.id).unwrap(),
        Some(profile)
    );
}

#[test]
fn checked_receipt_lookup_is_authorized_read_only_and_propagates_intent_failure() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    let a = context();
    let j = Journal::new(&c);
    let accepted = admit(&j, &a, KEY).unwrap();
    let clock = j.channel_clock().unwrap();
    let replay = j
        .admission_lookup_checked(&a, "device", KEY, |id, fingerprint| {
            assert_eq!(id, accepted.session.id);
            assert_eq!(fingerprint, FP);
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(response(&replay), response(&accepted.session));
    assert_eq!(j.channel_clock().unwrap(), clock);
    assert!(
        j.admission_lookup_checked(&a, "device", KEY, |_, _| {
            Err(Error::Conflict("host pins changed"))
        })
        .is_err()
    );
    let mut revoked = a.clone();
    revoked.membership.active = false;
    assert!(
        j.admission_lookup_checked(&revoked, "device", KEY, |_, _| {
            panic!("unauthorized receipt must not reach host callback")
        })
        .is_err()
    );
    assert!(
        j.admission_lookup_checked(&a, "device", "missing-key-0001", |_, _| {
            panic!("missing receipt must not reach host callback")
        })
        .unwrap()
        .is_none()
    );
}
