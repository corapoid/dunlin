use super::*;
use crate::worker::{decode_frame, session::tests::open as legacy_open};
use serde_json::json;
fn limits() -> BrokerLimits {
    BrokerLimits {
        max_pending: 8,
        max_reply_bytes: MAX_REPLY_BYTES as u32,
    }
}
fn command() -> OpenBrokerSessionCommand {
    let mut open = legacy_open();
    open.definition.schema_version = 2;
    open.definition
        .mcp
        .push(dunlin_domain::agent::AgentMcpTool {
            provider_id: "work".into(),
            name: "lookup".into(),
            definition_digest: "a".repeat(64),
        });
    let mut value = OpenBrokerSessionCommand {
        protocol_version: 5,
        open,
        broker_limits: limits(),
    };
    value.open.identity.fingerprint = broker_session_open_fingerprint(&value).unwrap();
    value
}
#[test]
fn hello5_requires_explicit_version_and_bounded_offer_and_is_never_legacy_hello() {
    let hello=BrokerHello{protocol_version:5,host_id:"host".into(),workspace_id:"workspace".into(),enrollment_epoch:1,credential_revision:1,worker_boot_id:"worker".into(),capabilities:serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap(),broker_limits:limits()};
    let encoded = hello.encode().unwrap();
    assert_eq!(decode_broker_hello(encoded.as_bytes()).unwrap(), hello);
    assert!(decode_frame(&encoded).is_err());
    for version in [1, 2, 3, 4, 6] {
        let mut bad = hello.clone();
        bad.protocol_version = version;
        assert!(bad.encode().is_err());
    }
    for pending in [0, 9] {
        let mut bad = hello.clone();
        bad.broker_limits.max_pending = pending;
        assert!(bad.validate().is_err());
    }
    for size in [0, MAX_REPLY_BYTES as u32 + 1] {
        let mut bad = hello.clone();
        bad.broker_limits.max_reply_bytes = size;
        assert!(bad.validate().is_err());
    }
    let mut bad = serde_json::from_str::<Value>(&encoded).unwrap();
    bad["hello"]["access_token"] = json!("not authority");
    assert!(decode_broker_hello(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = serde_json::from_str::<Value>(&encoded).unwrap();
    bad["hello"]
        .as_object_mut()
        .unwrap()
        .remove("broker_limits");
    assert!(decode_broker_hello(&serde_json::to_vec(&bad).unwrap()).is_err());
    assert!(decode_broker_hello(&vec![b' '; MAX_WORKER_FRAME_BYTES + 1]).is_err());
}
#[test]
fn broker_open_pins_entire_payload_and_offer_and_cannot_enter_v4_admission() {
    let command = command();
    assert_eq!(
        command.open.identity.fingerprint,
        "7dbc95a9788cfff5a334f54f644b33e508a3cbc545b7367ba2f1cab97c6272a0"
    );
    command
        .validate_for_channel(&command.open.channel, &limits())
        .unwrap();
    assert!(command.open.validate().is_err());
    assert!(session_open_fingerprint(&command.open).is_err());
    let message = BrokerSessionControllerMessage::OpenBrokerSession {
        command: Box::new(command.clone()),
    };
    let encoded = serde_json::to_string(&message).unwrap();
    assert_eq!(
        decode_broker_session_controller_frame(
            encoded.as_bytes(),
            &command.open.channel,
            &limits()
        )
        .unwrap(),
        message
    );
    assert!(decode_session_controller_frame(&encoded).is_err());
    for field in [
        "channel",
        "worker_boot",
        "controller_boot",
        "session",
        "intent",
        "profile",
        "definition",
        "budget",
        "limits",
    ] {
        let mut bad = command.clone();
        match field {
            "channel" => bad.open.channel.channel_id = "other".into(),
            "worker_boot" => bad.open.channel.worker_boot_id = "other".into(),
            "controller_boot" => bad.open.channel.controller_boot_id = "other".into(),
            "session" => bad.open.controller_session_id = "other".into(),
            "intent" => bad.open.identity.intent_id = "other".into(),
            "profile" => bad.open.profile.version += 1,
            "definition" => bad.open.definition.instructions = Some("other".into()),
            "budget" => bad.open.budgets.max_turns -= 1,
            _ => bad.broker_limits.max_pending -= 1,
        }
        assert_ne!(
            broker_session_open_fingerprint(&bad).unwrap(),
            command.open.identity.fingerprint,
            "{field}"
        );
        assert!(
            bad.validate_for_channel(&command.open.channel, &limits())
                .is_err(),
            "{field}"
        );
    }
    let mut offered = limits();
    offered.max_pending = 1;
    assert!(
        command
            .validate_for_channel(&command.open.channel, &offered)
            .is_err()
    );
    let mut bad = command.clone();
    bad.open.definition.mcp.clear();
    assert!(bad.validate().is_err());
    let mut bad = command.clone();
    bad.protocol_version = 4;
    assert!(bad.validate().is_err());
    let mut bad = command.clone();
    bad.open
        .descriptor
        .grant
        .permissions
        .retain(|p| *p != ExportPermission::Execute);
    assert!(bad.validate().is_err());
}
#[test]
fn broker_receipt_correlation_checks_v5_hash_before_accepting_historical_metadata() {
    let command = command();
    let receipt = SessionReceipt {
        identity: command.open.identity.clone(),
        fence: SessionFence {
            channel: command.open.channel.clone(),
            controller_session_id: command.open.controller_session_id.clone(),
            worker_session_id: "worker-session".into(),
            session_epoch: 1,
        },
        budgets: command.open.budgets.clone(),
    };
    command.validate_receipt(&receipt).unwrap();
    assert!(receipt.validate_for_open(&command.open).is_err());
    let mut changed = command.clone();
    changed.open.definition.instructions = Some("changed".into());
    assert!(changed.validate_receipt(&receipt).is_err());
    changed.open.identity.fingerprint = broker_session_open_fingerprint(&changed).unwrap();
    assert!(changed.validate_receipt(&receipt).is_err());
    let mut changed = receipt.clone();
    changed.fence.channel.worker_boot_id = "newboot".into();
    assert!(command.validate_receipt(&changed).is_err());
}
