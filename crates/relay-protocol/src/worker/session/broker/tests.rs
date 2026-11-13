use super::*;
use serde_json::json;

fn request() -> BrokerRequest {
    let channel = ChannelFence {
        channel_id: "channel".into(),
        controller_boot_id: "controller".into(),
        worker_boot_id: "worker".into(),
    };
    let session = SessionFence {
        channel,
        controller_session_id: "browser-session".into(),
        worker_session_id: "worker-session".into(),
        session_epoch: 1,
    };
    BrokerRequest {
        scope: BrokerScope::Turn {
            turn: TurnFence {
                session,
                turn_id: "turn".into(),
                turn_index: 1,
                turn_epoch: 1,
            },
        },
        request_id: "request".into(),
        operation: BrokerOperation::ToolsCall {
            name: format!("mcp_{}", "a".repeat(32)),
            arguments_json: r#"{"query":"value"}"#.into(),
        },
    }
}
#[test]
fn request_reference_binds_both_boots_entire_session_turn_and_exact_operation() {
    let original = request();
    assert_eq!(
        original.reference().unwrap().fingerprint,
        "b23ead9a1f4aebdc17af056bd1310b2445626c5cda7b7e10b5549a6f45326df1"
    );
    let chunk = chunk_reply(&original, r#"{"content":[]}"#)
        .unwrap()
        .remove(0);
    for field in [
        "channel",
        "controller",
        "worker",
        "controller_session",
        "worker_session",
        "session_epoch",
        "turn",
        "turn_index",
        "turn_epoch",
        "request",
        "name",
        "arguments",
    ] {
        let mut changed = original.clone();
        let BrokerScope::Turn { turn } = &mut changed.scope else {
            unreachable!()
        };
        match field {
            "channel" => turn.session.channel.channel_id = "another".into(),
            "controller" => turn.session.channel.controller_boot_id = "another".into(),
            "worker" => turn.session.channel.worker_boot_id = "another".into(),
            "controller_session" => turn.session.controller_session_id = "another".into(),
            "worker_session" => turn.session.worker_session_id = "another".into(),
            "session_epoch" => turn.session.session_epoch += 1,
            "turn" => turn.turn_id = "another".into(),
            "turn_index" => turn.turn_index += 1,
            "turn_epoch" => turn.turn_epoch += 1,
            "request" => changed.request_id = "another".into(),
            "name" => {
                let BrokerOperation::ToolsCall { name, .. } = &mut changed.operation else {
                    unreachable!()
                };
                *name = format!("mcp_{}", "b".repeat(32));
            }
            _ => {
                let BrokerOperation::ToolsCall { arguments_json, .. } = &mut changed.operation
                else {
                    unreachable!()
                };
                *arguments_json = r#"{"query":"different"}"#.into();
            }
        }
        changed.validate().unwrap();
        assert_ne!(
            changed.reference().unwrap(),
            original.reference().unwrap(),
            "{field}"
        );
        assert!(chunk.validate_for_request(&changed).is_err(), "{field}");
    }
    let mut channel = original.scope.session().channel.clone();
    channel.worker_boot_id = "newboot".into();
    assert!(original.validate_for_channel(&channel).is_err());
    let BrokerScope::Turn { turn } = &original.scope else {
        unreachable!()
    };
    original.validate_for_turn(turn).unwrap();
    let mut stale = turn.clone();
    stale.turn_epoch += 1;
    assert!(original.validate_for_turn(&stale).is_err());
}
#[test]
fn call_scope_alias_and_bounded_arguments_cannot_be_bypassed_by_metadata() {
    let original = request();
    for arguments_json in [
        "[]".into(),
        "null".into(),
        "not json".into(),
        json!({"text":"x".repeat(MAX_ARGUMENT_BYTES)}).to_string(),
        json!({"nodes":vec![0;1024]}).to_string(),
    ] {
        let mut bad = original.clone();
        let BrokerOperation::ToolsCall {
            arguments_json: arguments,
            ..
        } = &mut bad.operation
        else {
            unreachable!()
        };
        *arguments = arguments_json;
        assert!(bad.validate().is_err());
    }
    let mut deep = json!(0);
    for _ in 0..40 {
        deep = json!({"nested":deep});
    }
    let mut bad = original.clone();
    let BrokerOperation::ToolsCall { arguments_json, .. } = &mut bad.operation else {
        unreachable!()
    };
    *arguments_json = deep.to_string();
    assert!(bad.validate().is_err());
    let mut bad = original.clone();
    let BrokerOperation::ToolsCall { name, .. } = &mut bad.operation else {
        unreachable!()
    };
    *name = "lookup".into();
    assert!(bad.validate().is_err());
    let mut bad = original.clone();
    bad.scope = BrokerScope::Catalog {
        session: original.scope.session().clone(),
    };
    assert!(bad.validate().is_err());
    let message = BrokerWorkerMessage::McpRequest { request: original };
    let mut wire = serde_json::to_value(&message).unwrap();
    wire["request"]["access_token"] = json!("never accepted as authority");
    assert!(decode_broker_worker_frame(&serde_json::to_vec(&wire).unwrap()).is_err());
    let mut wire = serde_json::to_value(&message).unwrap();
    wire["request"]["scope"]["turn"]["session"]["credential"] = json!("forged");
    assert!(decode_broker_worker_frame(&serde_json::to_vec(&wire).unwrap()).is_err());
}
#[test]
fn catalog_request_is_session_scoped_and_v4_never_accepts_broker_frames() {
    let original = request();
    let mut list = BrokerRequest {
        scope: BrokerScope::Catalog {
            session: original.scope.session().clone(),
        },
        request_id: "catalog".into(),
        operation: BrokerOperation::ToolsList { cursor: None },
    };
    list.validate().unwrap();
    let message = BrokerWorkerMessage::McpRequest {
        request: list.clone(),
    };
    let bytes = serde_json::to_vec(&message).unwrap();
    assert_eq!(decode_broker_worker_frame(&bytes).unwrap(), message);
    assert!(decode_session_worker_frame(std::str::from_utf8(&bytes).unwrap()).is_err());
    let chunk = chunk_reply(&list, r#"{"tools":[]}"#).unwrap().remove(0);
    let reply = BrokerControllerMessage::McpReplyChunk { chunk };
    let bytes = serde_json::to_vec(&reply).unwrap();
    assert!(decode_session_controller_frame(std::str::from_utf8(&bytes).unwrap()).is_err());
    assert_eq!(
        decode_broker_controller_frame(&bytes, &list).unwrap(),
        reply
    );
    list.operation = BrokerOperation::ToolsList {
        cursor: Some("cursor\n".into()),
    };
    assert!(list.validate().is_err());
    list = request();
    list.operation = BrokerOperation::ToolsList { cursor: None };
    assert!(list.validate().is_err());
}
#[test]
fn bounded_unicode_reply_roundtrips_without_splitting_utf8_or_exceeding_frames() {
    let request = request();
    let payload = json!({"content":[{"type":"text","text":"ż🦆\n".repeat(20000)}]}).to_string();
    let chunks = chunk_reply(&request, &payload).unwrap();
    assert!(chunks.len() > 1);
    let mut assembly = ReplyAssembly::new(&request).unwrap();
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(chunk.data.len() <= MAX_REPLY_CHUNK_BYTES);
        let bytes = serde_json::to_vec(&BrokerControllerMessage::McpReplyChunk {
            chunk: chunk.clone(),
        })
        .unwrap();
        assert!(bytes.len() <= MAX_WORKER_FRAME_BYTES);
        decode_broker_controller_frame(&bytes, &request).unwrap();
        let result = assembly.push(chunk).unwrap();
        if index + 1 == chunks.len() {
            assert_eq!(result.as_deref(), Some(payload.as_str()));
        } else {
            assert!(result.is_none());
        }
    }
    assert!(assembly.push(&chunks[0]).is_err());
}
#[test]
fn reply_assembly_poisoning_prevents_reorder_replay_header_changes_and_corruption() {
    let request = request();
    let payload = json!({"text":"x".repeat(9000)}).to_string();
    let chunks = chunk_reply(&request, &payload).unwrap();
    for case in [
        "reorder",
        "duplicate",
        "scope",
        "fingerprint",
        "id",
        "header",
        "digest",
        "oversize",
    ] {
        let mut assembly = ReplyAssembly::new(&request).unwrap();
        let mut bad = chunks[0].clone();
        match case {
            "reorder" => bad = chunks[1].clone(),
            "duplicate" => {
                assembly.push(&chunks[0]).unwrap();
            }
            "scope" => {
                let BrokerScope::Turn { turn } = &mut bad.request.scope else {
                    unreachable!()
                };
                turn.turn_epoch += 1;
            }
            "fingerprint" => bad.request.fingerprint = "b".repeat(64),
            "id" => bad.request.request_id = "other".into(),
            "header" => {
                assembly.push(&chunks[0]).unwrap();
                bad = chunks[1].clone();
                bad.total_bytes += 1;
            }
            "digest" => {
                bad.data.replace_range(0..1, "z");
            }
            _ => bad.total_bytes = MAX_REPLY_BYTES as u32 + 1,
        }
        if case == "digest" {
            assert!(assembly.push(&bad).unwrap().is_none());
            assert!(assembly.push(&chunks[1]).unwrap().is_none());
            assert!(assembly.push(&chunks[2]).is_err());
        } else {
            assert!(assembly.push(&bad).is_err(), "{case}");
        }
        assert!(
            assembly.push(&chunks[0]).is_err(),
            "poison must latch: {case}"
        );
    }
    let mut missing = ReplyAssembly::new(&request).unwrap();
    for chunk in chunks.iter().take(chunks.len() - 1) {
        assert!(missing.push(chunk).unwrap().is_none());
    }
    assert!(chunk_reply(&request, &"x".repeat(MAX_REPLY_BYTES + 1)).is_err());
    assert!(chunk_reply(&request, "invalid JSON").is_err());
    let mut bad = chunks[0].clone();
    bad.chunks = 0;
    assert!(bad.validate().is_err());
    let mut bad = chunks[0].clone();
    bad.data = "x".repeat(MAX_REPLY_CHUNK_BYTES + 1);
    assert!(bad.validate().is_err());
    assert!(decode_broker_worker_frame(&vec![b' '; MAX_WORKER_FRAME_BYTES + 1]).is_err());
}
#[test]
fn rejections_require_the_complete_request_reference_even_for_unknown_outcome() {
    let request = request();
    let reply = BrokerControllerMessage::McpRejected {
        request: request.reference().unwrap(),
        code: BrokerError::OutcomeUnknown,
    };
    let bytes = serde_json::to_vec(&reply).unwrap();
    assert_eq!(
        decode_broker_controller_frame(&bytes, &request).unwrap(),
        reply
    );
    let mut changed = request.clone();
    changed.request_id = "another".into();
    assert!(decode_broker_controller_frame(&bytes, &changed).is_err());
}
