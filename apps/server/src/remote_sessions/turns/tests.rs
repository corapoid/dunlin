use super::*;

fn command(index: u64) -> wire::TurnCommand {
    let mut command = wire::TurnCommand {
        session: wire::SessionFence {
            channel: dunlin_relay_protocol::worker::ChannelFence {
                channel_id: "channel".into(),
                controller_boot_id: "controller".into(),
                worker_boot_id: "worker".into(),
            },
            controller_session_id: "session".into(),
            worker_session_id: "worker-session".into(),
            session_epoch: 1,
        },
        identity: wire::TurnIdentity {
            command_id: format!("command-{index}"),
            turn_id: format!("turn-{index}"),
            fingerprint: "0".repeat(64),
        },
        turn_index: index,
        input: "Work on this task".into(),
        budgets: dunlin_relay_protocol::worker::ExecutionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            lease_duration_ms: 10000,
        },
    };
    command.identity.fingerprint = wire::session_turn_fingerprint(&command).unwrap();
    command
}

#[test]
fn original_deadline_survives_waiting_and_duplicate_start() {
    let start = Instant::now();
    let mut clock = Clock::default();
    let command = command(1);
    clock
        .begin(command.clone(), start + Duration::from_secs(600), start)
        .unwrap();
    let deadline = start + Duration::from_secs(60);
    for elapsed in [1, 20, 59] {
        let now = start + Duration::from_secs(elapsed);
        assert!(
            clock
                .begin(command.clone(), now + Duration::from_secs(600), now)
                .is_err()
        );
        clock.reconcile(Some("turn-1")).unwrap();
        assert_eq!(clock.submit_deadline(&command, now).unwrap(), deadline);
    }
    assert!(clock.check(deadline).is_err());
    assert!(clock.submit_deadline(&command, deadline).is_err());
}

#[test]
fn session_deadline_and_exact_original_command_bound_submit() {
    let start = Instant::now();
    let mut clock = Clock::default();
    let original = command(1);
    let deadline = start + Duration::from_secs(15);
    clock.begin(original.clone(), deadline, start).unwrap();
    assert_eq!(clock.submit_deadline(&original, start).unwrap(), deadline);
    for changed in 0..5 {
        let mut command = original.clone();
        match changed {
            0 => command.session.session_epoch += 1,
            1 => command.identity.command_id.push_str("-other"),
            2 => command.input.push_str(" changed"),
            3 => command.budgets.max_seconds += 1,
            _ => command.turn_index += 1,
        }
        // Even a valid, newly hashed command cannot replace the claimed one.
        command.identity.fingerprint = wire::session_turn_fingerprint(&command).unwrap();
        assert!(clock.submit_deadline(&command, start).is_err());
    }
    assert!(clock.check(deadline).is_err());
}

#[test]
fn committed_idle_clears_turn_but_never_reconstructs_or_reuses_it() {
    let start = Instant::now();
    let session = start + Duration::from_secs(600);
    let mut clock = Clock::default();
    assert!(clock.reconcile(Some("turn-1")).is_err());
    clock.begin(command(1), session, start).unwrap();
    assert!(clock.reconcile(Some("turn-2")).is_err());
    assert!(clock.begin(command(2), session, start).is_err());
    clock.reconcile(None).unwrap();
    assert!(clock.submit_deadline(&command(1), start).is_err());
    assert!(clock.begin(command(1), session, start).is_err());
    let next = start + Duration::from_secs(30);
    clock.begin(command(2), session, next).unwrap();
    assert_eq!(
        clock.submit_deadline(&command(2), next).unwrap(),
        next + Duration::from_secs(60)
    );
}

#[test]
fn malformed_or_expired_claim_does_not_consume_the_turn_index() {
    let now = Instant::now();
    let session = now + Duration::from_secs(600);
    let mut clock = Clock::default();
    let mut altered = command(1);
    altered.input.push_str(" altered after hashing");
    assert!(clock.begin(altered, session, now).is_err());
    assert!(clock.begin(command(1), now, now).is_err());
    clock.begin(command(1), session, now).unwrap();
}
