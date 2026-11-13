use super::*;
use serde_json::{Value, json};
fn channel() -> Value {
    json!({"channel_id":"channel-1","controller_boot_id":"controller-boot","worker_boot_id":"worker-boot"})
}
fn identity() -> Value {
    json!({"command_id":"command-1","attempt_id":"attempt-1","fingerprint":"a".repeat(64)})
}
fn fence() -> Value {
    json!({"channel":channel(),"attempt_id":"attempt-1","execution_epoch":1})
}
fn command() -> Value {
    json!({"type":"execute","command":{
        "channel":channel(),"identity":identity(),
        "descriptor":{
            "scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node",
                "workspace_id":"workspace-1","host_id":"host-1","enrollment_epoch":1,"credential_revision":1},
            "export":{"id":"export-1","name":"Export","repo_kind":"linked_worktree",
                "checkout_identity_digest":"b".repeat(64),"settings_digest":"c".repeat(64),"revision":1,"active":true},
            "grant":{"principal":{"kind":"human","id":"user-1"},"permissions":["read","execute"],
                "revision":1,"export_revision":1,"active":true}},
        "profile":{"agent_id":"agent-1","version":1,"digest":"d".repeat(64)},
        "definition":{"schema_version":1,"name":"Pinned agent","description":"",
            "harness":"opencode","model":"configured-model","mode":"configured-mode",
            "instructions":"literal instructions","skills":[],"max_seconds":120,"max_output_bytes":65536},
        "input":"explicit request","budgets":{"max_seconds":60,"max_output_bytes":32768,"lease_duration_ms":10000}
    }})
}
fn receipt() -> Value {
    json!({"fence":fence(),"identity":identity(),"lease_duration_ms":10000})
}
fn outcome() -> Value {
    json!({"outcome_id":"outcome-1","digest":"e".repeat(64),"status":"completed","text":"finished"})
}
fn controller(value: &Value) -> Result<ExecutionControllerMessage> {
    decode_execution_controller_frame(&value.to_string())
}
fn worker(value: &Value) -> Result<ExecutionWorkerMessage> {
    decode_execution_worker_frame(&value.to_string())
}
#[test]
fn mcp_profiles_require_broker_support_before_worker_execution() {
    let mut value = command();
    value["command"]["definition"]["schema_version"] = json!(2);
    value["command"]["definition"]["mcp"] =
        json!([{"provider_id":"work","name":"lookup","definition_digest":"a".repeat(64)}]);
    assert!(controller(&value).is_err());
}
#[test]
fn separate_v3_contract_roundtrips_without_accepting_execution_in_legacy_payload_parsers() {
    let value = command();
    let decoded = controller(&value).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    assert!(super::super::decode_controller_frame(&value.to_string()).is_err());
    let hello = json!({"type":"hello","protocol_version":5,"host_id":"host-1","workspace_id":"workspace-1",
        "enrollment_epoch":1,"credential_revision":1,"worker_boot_id":"worker-boot",
        "capabilities":{"protocol_version":1,"version":"fixture","os":"linux","architecture":"x86_64",
            "harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1}});
    let error = super::super::decode_frame(&hello.to_string()).unwrap_err();
    assert_eq!(
        error.code,
        super::super::WorkerErrorCode::UnsupportedProtocol
    );
    for value in [
        json!({"type":"accepted","record":{"receipt":receipt(),"status":"accepted","outcome":null}}),
        json!({"type":"renewed","fence":fence(),"sequence":1,"lease_duration_ms":1000}),
        json!({"type":"cancel_ack","fence":fence(),"disposition":"requested"}),
        json!({"type":"outcome","fence":fence(),"outcome":outcome()}),
    ] {
        assert_eq!(
            serde_json::to_value(worker(&value).unwrap()).unwrap(),
            value
        );
    }
}
#[test]
fn admission_rejects_service_or_read_only_grants_unpinned_profiles_and_budget_expansion() {
    for (pointer, invalid) in [
        ("/command/descriptor/grant/principal/kind", json!("service")),
        ("/command/descriptor/grant/permissions", json!(["read"])),
        ("/command/descriptor/grant/permissions", json!(["execute"])),
        ("/command/descriptor/grant/export_revision", json!(2)),
        ("/command/profile/version", json!(MAX_EXECUTION_INTEGER + 1)),
        ("/command/profile/digest", json!("D".repeat(64))),
        ("/command/identity/fingerprint", json!("x".repeat(64))),
        ("/command/definition/harness", json!("/bin/sh")),
        ("/command/budgets/max_seconds", json!(121)),
        ("/command/budgets/max_output_bytes", json!(65537)),
        ("/command/budgets/lease_duration_ms", json!(10001)),
        ("/command/input", json!(" \n\t")),
        ("/command/input", json!("nul\0byte")),
    ] {
        let mut value = command();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(controller(&value).is_err(), "Accepted invalid {pointer}");
    }
    let mut value = command();
    value["command"]["budgets"]["max_seconds"] = json!(1);
    value["command"]["budgets"]["lease_duration_ms"] = json!(1001);
    assert!(controller(&value).is_err());
}
#[test]
fn strict_shapes_reject_arbitrary_authority_execution_and_renewal_fields_at_every_boundary() {
    for pointer in [
        "/command",
        "/command/channel",
        "/command/identity",
        "/command/descriptor",
        "/command/descriptor/export",
        "/command/descriptor/grant",
        "/command/profile",
        "/command/definition",
        "/command/budgets",
    ] {
        let mut value = command();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("path".into(), json!("/unapproved"));
        assert!(
            controller(&value).is_err(),
            "Ignored an unknown field at {pointer}"
        );
    }
    let query = json!({"type":"reconcile","channel":channel(),"identity":identity()});
    assert!(controller(&query).is_ok());
    for field in [
        "input",
        "lease_duration_ms",
        "sequence",
        "argv",
        "verified",
        "credential",
    ] {
        let mut value = query.clone();
        value[field] = json!("injected");
        assert!(controller(&value).is_err());
    }
    let ack = json!({"type":"outcome_ack","channel":channel(),"identity":identity(),
        "fence":fence(),"outcome_id":"outcome-1","outcome_digest":"e".repeat(64)});
    assert!(controller(&ack).is_ok());
    let mut value = ack;
    value["lease_duration_ms"] = json!(1000);
    assert!(controller(&value).is_err());
}
#[test]
fn text_limits_count_utf8_bytes_and_encoded_frames_remain_bounded_after_escaping() {
    let mut value = command();
    value["command"]["input"] = json!("界".repeat(MAX_EXECUTION_TEXT_BYTES / 3));
    assert!(controller(&value).is_ok());
    value["command"]["input"] = json!("界".repeat(MAX_EXECUTION_TEXT_BYTES / 3 + 1));
    assert!(controller(&value).is_err());
    value["command"]["input"] = json!("\"".repeat(MAX_EXECUTION_TEXT_BYTES));
    value["command"]["definition"]["instructions"] = json!("\"".repeat(MAX_EXECUTION_TEXT_BYTES));
    assert!(value.to_string().len() > super::super::MAX_WORKER_FRAME_BYTES);
    assert!(controller(&value).is_err());
    let mut result = outcome();
    result["text"] = json!("界".repeat(MAX_EXECUTION_TEXT_BYTES / 3 + 1));
    assert!(worker(&json!({"type":"outcome","fence":fence(),"outcome":result})).is_err());
}
#[test]
fn receipt_epochs_safe_sequences_and_terminal_record_correspondence_are_checked() {
    let renew = json!({"type":"renew","fence":fence(),"sequence":1,"lease_duration_ms":1000});
    // Identical DTOs are valid: durable Store dedupe/monotonicity belongs later.
    assert_eq!(controller(&renew).unwrap(), controller(&renew).unwrap());
    for (pointer, invalid) in [
        ("/sequence", json!(0)),
        ("/sequence", json!(MAX_EXECUTION_INTEGER + 1)),
        ("/fence/execution_epoch", json!(0)),
        ("/fence/execution_epoch", json!(MAX_EXECUTION_INTEGER + 1)),
        ("/lease_duration_ms", json!(999)),
        ("/lease_duration_ms", json!(10001)),
    ] {
        let mut value = renew.clone();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(controller(&value).is_err());
    }
    for (status, output, valid) in [
        ("accepted", Value::Null, true),
        ("running", Value::Null, true),
        ("completed", outcome(), true),
        ("completed", Value::Null, false),
        ("failed", outcome(), false),
        ("running", outcome(), false),
    ] {
        assert_eq!(
            worker(&json!({"type":"accepted","record":{"receipt":receipt(),
            "status":status,"outcome":output}}))
            .is_ok(),
            valid
        );
    }
    let mut wrong = receipt();
    wrong["identity"]["attempt_id"] = json!("other-attempt");
    assert!(
        worker(&json!({"type":"accepted","record":{"receipt":wrong,
        "status":"accepted","outcome":null}}))
        .is_err()
    );
}
#[test]
fn reconciliation_and_outcome_ack_preserve_historical_fences_without_renewing() {
    let mut current = channel();
    current["channel_id"] = json!("fresh-channel");
    current["worker_boot_id"] = json!("fresh-worker-boot");
    let record = json!({"receipt":receipt(),"status":"completed","outcome":outcome()});
    let value =
        json!({"type":"reconciled","channel":current,"identity":identity(),"record":record});
    assert!(worker(&value).is_ok());
    let ack = json!({"type":"outcome_ack","channel":current,"identity":identity(),
        "fence":fence(),"outcome_id":"outcome-1","outcome_digest":"e".repeat(64)});
    assert!(controller(&ack).is_ok());
    let mut wrong = value;
    wrong["identity"]["command_id"] = json!("other-command");
    assert!(worker(&wrong).is_err());
    let mut wrong = ack;
    wrong["fence"]["attempt_id"] = json!("other-attempt");
    assert!(controller(&wrong).is_err());
    assert!(
        worker(&json!({"type":"reconciled","channel":current,"identity":identity(),"record":null}))
            .is_ok()
    );
}

#[test]
fn v3_rate_budget_is_shared_and_keeps_legacy_protocol_limits_unchanged() {
    assert_eq!(worker_rate_limit(1), 120);
    assert_eq!(worker_rate_limit(2), 120);
    assert_eq!(worker_rate_limit(3), MAX_EXECUTION_FRAMES_PER_MINUTE);
    assert_eq!(worker_rate_limit(4), 4096);
    assert_eq!(worker_rate_limit(5), 4096);
    assert_eq!(worker_rate_limit(6), 120);
    assert_eq!(MAX_EXECUTION_FRAMES_PER_MINUTE, 1024);
}

#[test]
fn durable_outcome_ack_reply_binds_current_channel_original_fence_identity_and_digest() {
    let mut current = channel();
    current["channel_id"] = json!("replacement-channel");
    let reply = json!({"type":"outcome_acknowledged","channel":current,"identity":identity(),"fence":fence(),"outcome_id":"outcome-1","outcome_digest":"e".repeat(64)});
    assert_eq!(
        serde_json::to_value(worker(&reply).unwrap()).unwrap(),
        reply
    );
    for (pointer, value) in [
        ("/identity/attempt_id", json!("different-attempt")),
        ("/outcome_digest", json!("E".repeat(64))),
        ("/fence/execution_epoch", json!(0)),
        ("/channel/worker_boot_id", json!("/invalid")),
    ] {
        let mut invalid = reply.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(worker(&invalid).is_err());
    }
    let mut invalid = reply;
    invalid["lease_duration_ms"] = json!(1000);
    assert!(worker(&invalid).is_err());
}

#[test]
fn ack_rejection_keeps_exact_negative_correlation_fields_and_closed_error_code() {
    let rejection = json!({"type":"outcome_ack_rejected","channel":channel(),"identity":identity(),"fence":fence(),
        "outcome_id":"outcome-1","outcome_digest":"e".repeat(64),"code":"unavailable"});
    assert_eq!(
        serde_json::to_value(worker(&rejection).unwrap()).unwrap(),
        rejection
    );
    let mut invalid = rejection.clone();
    invalid["code"] = json!("retry_and_renew");
    assert!(worker(&invalid).is_err());
    let mut invalid = rejection;
    invalid["fence"]["attempt_id"] = json!("different-attempt");
    assert!(worker(&invalid).is_err());
}
