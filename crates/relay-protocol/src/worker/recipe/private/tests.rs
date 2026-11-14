use super::*;
use crate::worker::{
    decode_frame,
    session::{SessionFence, broker, decode_session_controller_frame},
};
use serde_json::json;
fn broker_limits() -> BrokerLimits {
    BrokerLimits {
        max_pending: 4,
        max_reply_bytes: 256 * 1024,
    }
}
fn limits() -> RecipeLimits {
    RecipeLimits {
        max_pending: 2,
        max_secret_bytes: 16384,
        max_reply_bytes: 128 * 1024,
        max_timeout_ms: 120000,
    }
}
fn hello() -> RecipeHello {
    RecipeHello {protocol_version:6,host_id:"host".into(),workspace_id:"workspace".into(),enrollment_epoch:1,credential_revision:1,worker_boot_id:"worker-boot".into(),capabilities:serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap(),broker_limits:broker_limits(),recipe_limits:limits()}
}
fn open() -> OpenRecipeSessionCommand {
    let base=serde_json::from_value(json!({"channel":{"channel_id":"channel","controller_boot_id":"controller-boot","worker_boot_id":"worker-boot"},"controller_session_id":"browser-session","identity":{"intent_id":"intent","fingerprint":"a".repeat(64)},"descriptor":{"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node","workspace_id":"workspace","host_id":"host","enrollment_epoch":1,"credential_revision":1},"export":{"id":"export","name":"Export","repo_kind":"git","checkout_identity_digest":"b".repeat(64),"settings_digest":"c".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":"owner"},"permissions":["read","execute"],"revision":1,"export_revision":1,"active":true}},"profile":{"agent_id":"agent","version":1,"digest":"d".repeat(64)},"definition":{"schema_version":2,"name":"Agent","description":"","harness":"opencode","model":"model","mode":"mode","instructions":"instruction","skills":[],"mcp":[{"provider_id":"dunlin-secret-work","name":"recipe","definition_digest":"e".repeat(64)}],"max_seconds":60,"max_output_bytes":65536},"budgets":{"max_seconds":60,"max_output_bytes":65536,"idle_timeout_ms":30000,"lease_duration_ms":10000,"max_turns":128,"retained_bytes":8388608}})).unwrap();
    let mut command = OpenRecipeSessionCommand {
        protocol_version: 6,
        open: base,
        broker_limits: broker_limits(),
        recipe_limits: limits(),
    };
    command.open.identity.fingerprint = open_fingerprint(&command).unwrap();
    command
}
fn operation() -> RecipeOperation {
    let command = open();
    let mut op = RecipeOperation {
        protocol_version: 6,
        nonce: "a".repeat(64),
        turn: TurnFence {
            session: SessionFence {
                channel: command.open.channel.clone(),
                controller_session_id: command.open.controller_session_id,
                worker_session_id: "native-session".into(),
                session_epoch: 1,
            },
            turn_id: "turn".into(),
            turn_index: 1,
            turn_epoch: 1,
        },
        descriptor: command.open.descriptor,
        tool: command.open.definition.mcp[0].clone(),
        recipe: "deploy".into(),
        template_digest: "b".repeat(64),
        credential_stamp: "c".repeat(64),
        params: BTreeMap::from([("environment".into(), "private-approved-value".into())]),
        timeout_ms: 30000,
        max_reply_bytes: 65536,
        fingerprint: "d".repeat(64),
    };
    op.fingerprint = operation_fingerprint(&op).unwrap();
    op
}
#[test]
fn private_hello_requires_v6_and_old_codecs_cannot_negotiate_it_or_inject_a_value() {
    assert_eq!(WORKER_RECIPE_SUBPROTOCOL, "dunlin.worker.v6");
    let hello = hello();
    let raw = hello.encode().unwrap();
    assert_eq!(decode_hello(raw.as_bytes()).unwrap(), hello);
    assert!(decode_frame(&raw).is_err());
    assert!(broker::decode_broker_hello(raw.as_bytes()).is_err());
    for version in [1, 2, 3, 4, 5, 7] {
        let mut changed = hello.clone();
        changed.protocol_version = version;
        assert!(changed.encode().is_err());
    }
    for key in ["secret", "access_token", "value"] {
        let mut changed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        changed["hello"][key] = json!("must not enter metadata");
        assert!(decode_hello(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    assert!(decode_hello(&vec![b' '; MAX_WORKER_FRAME_BYTES + 1]).is_err());
}
#[test]
fn private_limits_require_explicit_bounded_selection_from_both_offers() {
    let valid = limits();
    valid.validate_selection(&valid, &broker_limits()).unwrap();
    for (key, value) in [
        ("max_pending", 0),
        ("max_pending", 5),
        ("max_secret_bytes", 0),
        ("max_secret_bytes", 16385),
        ("max_reply_bytes", 1023),
        ("max_reply_bytes", 262145),
        ("max_timeout_ms", 0),
        ("max_timeout_ms", 120001),
    ] {
        let mut changed = serde_json::to_value(&valid).unwrap();
        changed[key] = json!(value);
        assert!(
            serde_json::from_value::<RecipeLimits>(changed)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for key in [
        "max_pending",
        "max_secret_bytes",
        "max_reply_bytes",
        "max_timeout_ms",
    ] {
        let mut offer = serde_json::to_value(&valid).unwrap();
        offer[key] = json!(offer[key].as_u64().unwrap() - 1);
        let offer = serde_json::from_value(offer).unwrap();
        assert!(valid.validate_selection(&offer, &broker_limits()).is_err());
    }
    let mut broker = broker_limits();
    broker.max_reply_bytes = 65536;
    assert!(valid.validate_selection(&valid, &broker).is_err());
    broker = broker_limits();
    broker.max_pending = 1;
    assert!(valid.validate_selection(&valid, &broker).is_err());
}
#[test]
fn private_open_has_a_distinct_fingerprint_and_bounds_limits_to_the_live_channel() {
    let command = open();
    command
        .validate_for_channel(&command.open.channel, &broker_limits(), &limits())
        .unwrap();
    let envelope = PrivateControllerMetadata::OpenRecipeSession {
        command: Box::new(command.clone()),
    };
    let raw = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(decode_metadata(&raw).unwrap(), envelope);
    assert!(decode_session_controller_frame(std::str::from_utf8(&raw).unwrap()).is_err());
    assert!(
        broker::decode_broker_session_controller_frame(
            &raw,
            &command.open.channel,
            &broker_limits()
        )
        .is_err()
    );
    assert_ne!(
        open_fingerprint(&command).unwrap(),
        broker::broker_session_open_fingerprint(&command.broker_shape()).unwrap()
    );
    let receipt = SessionReceipt {
        identity: command.open.identity.clone(),
        fence: SessionFence {
            channel: command.open.channel.clone(),
            controller_session_id: command.open.controller_session_id.clone(),
            worker_session_id: "native".into(),
            session_epoch: 1,
        },
        budgets: command.open.budgets.clone(),
    };
    command.validate_receipt(&receipt).unwrap();
    let mut substituted = receipt.clone();
    substituted.fence.channel.worker_boot_id = "other".into();
    assert!(command.validate_receipt(&substituted).is_err());
    let mut changed = command.clone();
    changed.open.channel.worker_boot_id = "other".into();
    changed.open.identity.fingerprint = open_fingerprint(&changed).unwrap();
    assert!(
        changed
            .validate_for_channel(&command.open.channel, &broker_limits(), &limits())
            .is_err()
    );
    let mut changed = command.clone();
    changed.recipe_limits.max_pending -= 1;
    assert_ne!(
        open_fingerprint(&changed).unwrap(),
        command.open.identity.fingerprint
    );
    assert!(
        changed
            .validate_for_channel(&command.open.channel, &broker_limits(), &limits())
            .is_err()
    );
}
#[test]
fn operation_fingerprint_pins_nonce_identity_target_template_credential_params_and_budgets() {
    let op = operation();
    op.validate_for_turn(&op.turn, &op.descriptor, &open().open.definition, &limits())
        .unwrap();
    let raw = serde_json::to_vec(&PrivateControllerMetadata::PrepareRecipe {
        operation: Box::new(op.clone()),
    })
    .unwrap();
    assert!(decode_metadata(&raw).is_ok());
    let debug = format!("{op:?}");
    assert!(!debug.contains("private-approved-value"));
    assert!(!debug.contains(&op.credential_stamp));
    for field in [
        "nonce",
        "turn",
        "session",
        "boot",
        "export",
        "grant",
        "template",
        "credential",
        "provider",
        "tool_digest",
        "recipe",
        "params",
        "timeout",
        "reply",
    ] {
        let mut changed = op.clone();
        match field {
            "nonce" => changed.nonce = "e".repeat(64),
            "turn" => changed.turn.turn_epoch += 1,
            "session" => changed.turn.session.session_epoch += 1,
            "boot" => changed.turn.session.channel.worker_boot_id = "other".into(),
            "export" => {
                changed.descriptor.export.revision += 1;
                changed.descriptor.grant.export_revision += 1;
            }
            "grant" => changed.descriptor.grant.revision += 1,
            "template" => changed.template_digest = "e".repeat(64),
            "credential" => changed.credential_stamp = "e".repeat(64),
            "provider" => changed.tool.provider_id = "dunlin-secret-other".into(),
            "tool_digest" => changed.tool.definition_digest = "f".repeat(64),
            "recipe" => changed.recipe = "other".into(),
            "params" => {
                changed.params.insert("environment".into(), "other".into());
            }
            "timeout" => changed.timeout_ms -= 1,
            _ => changed.max_reply_bytes -= 1,
        }
        assert_ne!(
            operation_fingerprint(&changed).unwrap(),
            op.fingerprint,
            "{field}"
        );
        assert!(
            changed
                .validate_for_turn(&op.turn, &op.descriptor, &open().open.definition, &limits())
                .is_err(),
            "{field}"
        );
    }
    let mut other_pin = op.clone();
    other_pin.tool.provider_id = "dunlin-secret-other".into();
    other_pin.fingerprint = operation_fingerprint(&other_pin).unwrap();
    assert!(
        other_pin
            .validate_for_turn(&op.turn, &op.descriptor, &open().open.definition, &limits())
            .is_err()
    );
    let mut changed = op.clone();
    changed.turn.turn_epoch += 1;
    changed.fingerprint = operation_fingerprint(&changed).unwrap();
    assert!(
        changed
            .validate_for_turn(&op.turn, &op.descriptor, &open().open.definition, &limits())
            .is_err()
    );
}
#[test]
fn private_metadata_rejects_secrets_missing_execute_or_over_budget_params_without_dispatch() {
    let op = operation();
    let valid = serde_json::to_value(PrivateControllerMetadata::PrepareRecipe {
        operation: Box::new(op.clone()),
    })
    .unwrap();
    for field in ["value", "value_b64", "secret", "program", "cwd"] {
        let mut changed = valid.clone();
        changed["operation"][field] = json!("untrusted");
        assert!(decode_metadata(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let mut changed = op.clone();
    changed.descriptor.grant.permissions = vec![ExportPermission::Read];
    assert!(changed.validate().is_err());
    let mut changed = op.clone();
    changed.params.insert("extra".into(), "x".repeat(4096));
    changed.params.insert("more".into(), "x".repeat(4096));
    assert!(changed.validate().is_err());
    let mut changed = op.clone();
    changed
        .params
        .insert("environment".into(), "bad\0value".into());
    assert!(changed.validate().is_err());
    let mut smaller = limits();
    smaller.max_timeout_ms = op.timeout_ms - 1;
    assert!(
        op.validate_for_turn(&op.turn, &op.descriptor, &open().open.definition, &smaller)
            .is_err()
    );
    assert!(decode_metadata(&vec![b' '; MAX_WORKER_FRAME_BYTES + 1]).is_err());
}
