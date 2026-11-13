use super::*;
use serde_json::{Value, json};
fn hello() -> Value {
    json!({"type":"hello","protocol_version":1,"host_id":"host-1","workspace_id":"workspace-1","enrollment_epoch":1,"credential_revision":1,"worker_boot_id":"worker-boot-1","capabilities":{"protocol_version":1,"version":"1.0.0-fixture","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":2}})
}
fn proof() -> ChannelProofResponse {
    ChannelProofResponse {
        channel_id: "channel-1".into(),
        controller_boot_id: "controller-boot-1".into(),
        worker_boot_id: "worker-boot-1".into(),
        nonce: "0123456789abcdef".repeat(4),
        host_id: "host-1".into(),
        enrollment_epoch: 1,
        credential_revision: 1,
    }
}
fn challenge() -> ControllerMessage {
    ControllerMessage::Challenge {
        channel_id: "channel-1".into(),
        controller_boot_id: "controller-boot-1".into(),
        nonce: proof().nonce,
        heartbeat_interval_ms: 1000,
        lease_ttl_ms: 5000,
    }
}
#[test]
fn hello_ready_heartbeat_and_controller_frames_roundtrip_as_metadata_only() {
    let h = hello();
    assert_eq!(
        serde_json::to_value(decode_frame(&h.to_string()).unwrap()).unwrap(),
        h
    );
    for message in [
        WorkerMessage::Ready {
            channel_id: "channel-1".into(),
            controller_boot_id: "controller-boot-1".into(),
            worker_boot_id: "worker-boot-1".into(),
            nonce: proof().nonce,
        },
        WorkerMessage::Heartbeat {
            channel_id: "channel-1".into(),
            lease_epoch: 1,
            sequence: 1,
        },
    ] {
        assert_eq!(
            decode_frame(&serde_json::to_string(&message).unwrap()).unwrap(),
            message
        );
    }
    for message in [
        challenge(),
        ControllerMessage::Accepted {
            channel_id: "channel-1".into(),
            controller_boot_id: "controller-boot-1".into(),
            worker_boot_id: "worker-boot-1".into(),
            lease_epoch: 1,
            expires_at_ms: 15000,
        },
        ControllerMessage::HeartbeatAck {
            channel_id: "channel-1".into(),
            lease_epoch: 1,
            sequence: 1,
            expires_at_ms: 20000,
        },
        ControllerMessage::Error {
            code: WorkerErrorCode::AuthorityRevoked,
        },
    ] {
        assert_eq!(
            decode_controller_frame(&serde_json::to_string(&message).unwrap()).unwrap(),
            message
        );
    }
    assert_eq!(WORKER_SUBPROTOCOL, "dunlin.worker.v1");
    assert!(decode_frame(r#"{"type":"execute","command":"/bin/sh"}"#).is_err());
    assert!(decode_controller_frame(r#"{"type":"dispatch","prompt":"unapproved"}"#).is_err());
}
#[test]
fn frame_byte_limits_unknown_fields_and_unsupported_versions_fail_before_use() {
    let frame = hello().to_string();
    let at_limit = format!(
        "{frame}{}",
        " ".repeat(MAX_WORKER_FRAME_BYTES - frame.len())
    );
    assert!(decode_frame(&at_limit).is_ok());
    let error = decode_frame(&(at_limit + " ")).unwrap_err();
    assert_eq!(error.path, "frame");
    assert!(error.message.contains("64 KiB"));
    assert!(
        decode_frame(&"界".repeat(MAX_WORKER_FRAME_BYTES / 3 + 1))
            .unwrap_err()
            .message
            .contains("64 KiB")
    );
    for field in ["verified", "execution_ready", "credential", "command"] {
        let mut value = hello();
        value[field] = json!(true);
        assert!(decode_frame(&value.to_string()).is_err());
        let mut value = hello();
        value["capabilities"][field] = json!(true);
        assert!(decode_frame(&value.to_string()).is_err());
    }
    let mut value = hello();
    value["protocol_version"] = json!(5);
    assert_eq!(
        decode_frame(&value.to_string()).unwrap_err().code,
        WorkerErrorCode::UnsupportedProtocol
    );
    let mut value = hello();
    value["capabilities"]["protocol_version"] = json!(2);
    assert!(decode_frame(&value.to_string()).is_err());
    let mut value = hello();
    value["capabilities"]["harnesses"] = json!(["/bin/sh"]);
    assert!(decode_frame(&value.to_string()).is_err());
    assert!(
        decode_frame(
            r#"{"type":"heartbeat","channel_id":"c","lease_epoch":1,"sequence":1,"sequence":2}"#
        )
        .is_err()
    );
    assert!(
        decode_controller_frame(r#"{"type":"error","code":"arbitrary_provider_error"}"#).is_err()
    );
}
#[test]
fn identifiers_nonces_and_signed_integer_limits_are_explicit() {
    for field in ["host_id", "workspace_id", "worker_boot_id"] {
        for invalid in [
            String::new(),
            "x".repeat(129),
            "bad/id".into(),
            "user@host".into(),
            "node\nnext".into(),
            "界".into(),
        ] {
            let mut value = hello();
            value[field] = json!(invalid);
            assert!(decode_frame(&value.to_string()).is_err(), "{field}");
        }
    }
    for value in [0, MAX_INTEGER + 1, u64::MAX] {
        let mut h = hello();
        h["enrollment_epoch"] = json!(value);
        assert!(decode_frame(&h.to_string()).is_err());
        assert!(
            WorkerMessage::Heartbeat {
                channel_id: "c".into(),
                lease_epoch: 1,
                sequence: value
            }
            .validate()
            .is_err()
        );
        assert!(
            ControllerMessage::HeartbeatAck {
                channel_id: "c".into(),
                lease_epoch: 1,
                sequence: 1,
                expires_at_ms: value
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        WorkerMessage::Heartbeat {
            channel_id: "c".into(),
            lease_epoch: MAX_INTEGER,
            sequence: MAX_INTEGER
        }
        .validate()
        .is_ok()
    );
    for value in [
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
    ] {
        let mut p = proof();
        p.nonce = value;
        assert!(p.validate().is_err());
    }
    for (interval, ttl) in [
        (999, 5000),
        (5001, 15000),
        (1000, 4999),
        (1000, 15001),
        (5000, 5000),
    ] {
        let mut value = serde_json::to_value(challenge()).unwrap();
        value["heartbeat_interval_ms"] = json!(interval);
        value["lease_ttl_ms"] = json!(ttl);
        assert!(decode_controller_frame(&value.to_string()).is_err());
    }
    for (interval, ttl) in [(1000, 5000), (5000, 5001), (5000, 15000)] {
        let mut value = serde_json::to_value(challenge()).unwrap();
        value["heartbeat_interval_ms"] = json!(interval);
        value["lease_ttl_ms"] = json!(ttl);
        assert!(decode_controller_frame(&value.to_string()).is_ok());
    }
}
#[test]
fn proof_frames_bind_all_identity_fields_but_cannot_assert_trusted_verification() {
    let response = proof();
    let encoded = serde_json::to_string(&response).unwrap();
    assert_eq!(decode_proof_response(&encoded).unwrap(), response);
    let request = ChannelProofRequest {
        channel_id: response.channel_id.clone(),
        controller_boot_id: response.controller_boot_id.clone(),
        nonce: response.nonce.clone(),
    };
    assert_eq!(
        decode_proof_request(&serde_json::to_string(&request).unwrap()).unwrap(),
        request
    );
    for field in [
        "channel_id",
        "controller_boot_id",
        "worker_boot_id",
        "nonce",
        "host_id",
        "enrollment_epoch",
        "credential_revision",
    ] {
        let mut value = serde_json::to_value(&response).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            decode_proof_response(&value.to_string()).is_err(),
            "{field}"
        );
    }
    let mut value = serde_json::to_value(&response).unwrap();
    value["network_verification"] = json!("verified");
    assert!(decode_proof_response(&value.to_string()).is_err());
    let mut value = serde_json::to_value(&request).unwrap();
    value["host_id"] = json!("host-1");
    assert!(decode_proof_request(&value.to_string()).is_err());
    assert!(decode_proof_response(&" ".repeat(MAX_WORKER_FRAME_BYTES + 1)).is_err());
}

#[test]
fn metadata_v2_is_bounded_path_free_and_requires_matching_active_grants() {
    let descriptor = json!({"scope":{"controller_node_id":"n-controller","worker_node_id":"n-worker","workspace_id":"w","host_id":"h","enrollment_epoch":1,"credential_revision":1},"export":{"id":"export-1","name":"Approved","repo_kind":"linked_worktree","checkout_identity_digest":"a".repeat(64),"settings_digest":"b".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":"u"},"permissions":["read","execute"],"revision":2,"export_revision":1,"active":true}});
    let valid = json!({"type":"export_described","request_id":"req","channel_id":"channel","lease_epoch":1,"descriptor":descriptor});
    assert!(decode_frame(&valid.to_string()).is_ok());
    for (section, field, value) in [
        ("scope", "worker_node_id", json!("n-controller")),
        ("export", "root", json!("/private/operator/root")),
        ("export", "active", json!(false)),
        ("grant", "export_revision", json!(2)),
        ("grant", "permissions", json!(["execute"])),
        ("grant", "permissions", json!(["read", "read"])),
        ("grant", "revision", json!(MAX_EXPORT_REVISION + 1)),
    ] {
        let mut invalid = valid.clone();
        invalid["descriptor"][section][field] = value;
        assert!(
            decode_frame(&invalid.to_string()).is_err(),
            "{section}.{field}"
        );
    }
    let request = ControllerMessage::DescribeExport {
        request_id: "req".into(),
        channel_id: "channel".into(),
        lease_epoch: 1,
        export_id: "export-1".into(),
        principal: ExportPrincipal {
            kind: PrincipalKind::Human,
            id: "u".into(),
        },
        expected_export_revision: Some(1),
    };
    assert_eq!(
        decode_controller_frame(&serde_json::to_string(&request).unwrap()).unwrap(),
        request
    );
    let mut modern = hello();
    modern["protocol_version"] = json!(WORKER_EXPORT_PROTOCOL_VERSION);
    assert!(decode_frame(&modern.to_string()).is_ok());
    assert!(decode_frame(&hello().to_string()).is_ok());
}
