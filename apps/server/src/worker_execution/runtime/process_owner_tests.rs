//! Typed proof separation only; real namespace-stop evidence is covered by
//! kernel acceptance. Never-spawned constructors are trusted local actor paths.
use super::*;
fn execution() -> wire::AttemptFence {
    wire::AttemptFence {
        channel: wire::ChannelFence {
            channel_id: "channel-owner-test".into(),
            controller_boot_id: "controller-owner-test".into(),
            worker_boot_id: "worker-owner-test".into(),
        },
        attempt_id: "attempt-owner-test".into(),
        execution_epoch: 1,
    }
}
fn session() -> wire::session::SessionFence {
    wire::session::SessionFence {
        channel: execution().channel,
        controller_session_id: "controller-session-owner-test".into(),
        worker_session_id: "worker-session-owner-test".into(),
        session_epoch: 1,
    }
}
#[test]
fn stop_proof_does_not_cross_execution_session_or_epoch() {
    let execution_fence = execution();
    let session_fence = session();
    let old = StopProof::never_spawned(execution_fence.clone());
    assert!(old.attest(&execution_fence).is_ok());
    assert!(old.attest_session(&session_fence).is_err());
    let persistent = StopProof::never_spawned_session(session_fence.clone());
    assert!(persistent.attest_session(&session_fence).is_ok());
    assert!(persistent.attest(&execution_fence).is_err());
    let mut newer = session_fence.clone();
    newer.session_epoch += 1;
    assert!(persistent.attest_session(&newer).is_err());
    newer = session_fence;
    newer.channel.worker_boot_id = "different-worker-boot".into();
    assert!(persistent.attest_session(&newer).is_err());
}
#[test]
fn private_guardian_owner_is_exactly_one_closed_variant() {
    let original = GuardianOwner::Execution {
        identity: wire::ExecutionIdentity {
            command_id: "command-owner-test".into(),
            attempt_id: execution().attempt_id,
            fingerprint: "a".repeat(64),
        },
        fence: execution(),
    };
    let mut encoded = serde_json::to_value(&original).unwrap();
    assert!(serde_json::from_value::<GuardianOwner>(encoded.clone()).is_ok());
    encoded["session"] = serde_json::to_value(session()).unwrap();
    assert!(serde_json::from_value::<GuardianOwner>(encoded).is_err());
    let persistent = GuardianOwner::Session {
        identity: wire::session::SessionIdentity {
            intent_id: "open-owner-test".into(),
            fingerprint: "b".repeat(64),
        },
        fence: session(),
    };
    let decoded: GuardianOwner =
        serde_json::from_value(serde_json::to_value(&persistent).unwrap()).unwrap();
    assert!(decoded.process_fence() == persistent.process_fence());
    assert!(decoded.process_fence() != original.process_fence());
}
