use super::*;
use serde_json::{Value, json};
#[test]
fn mcp_profiles_require_broker_support_before_session_open() {
    let mut value = open();
    value.definition.schema_version = 2;
    value
        .definition
        .mcp
        .push(dunlin_domain::agent::AgentMcpTool {
            provider_id: "work".into(),
            name: "lookup".into(),
            definition_digest: "a".repeat(64),
        });
    assert!(value.validate().is_err());
}

fn channel() -> ChannelFence {
    ChannelFence {
        channel_id: "channel".into(),
        controller_boot_id: "controller".into(),
        worker_boot_id: "worker".into(),
    }
}
fn session() -> SessionFence {
    SessionFence {
        channel: channel(),
        controller_session_id: "browser-session".into(),
        worker_session_id: "worker-session".into(),
        session_epoch: 1,
    }
}
fn budgets() -> SessionBudgets {
    SessionBudgets {
        max_seconds: 60,
        max_output_bytes: 65536,
        idle_timeout_ms: 30000,
        lease_duration_ms: 10000,
        max_turns: 128,
        retained_bytes: MAX_SESSION_RETAINED_BYTES,
    }
}
pub(super) fn open() -> SessionOpenCommand {
    serde_json::from_value(json!({"channel":channel(),"controller_session_id":"browser-session",
        "identity":{"intent_id":"open-intent","fingerprint":"a".repeat(64)},
        "descriptor":{"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node","workspace_id":"workspace","host_id":"host","enrollment_epoch":1,"credential_revision":1},
            "export":{"id":"export","name":"Export","repo_kind":"git","checkout_identity_digest":"b".repeat(64),"settings_digest":"c".repeat(64),"revision":1,"active":true},
            "grant":{"principal":{"kind":"human","id":"owner"},"permissions":["read","execute"],"revision":1,"export_revision":1,"active":true}},
        "profile":{"agent_id":"agent","version":1,"digest":"d".repeat(64)},
        "definition":{"schema_version":1,"name":"Agent","description":"","harness":"opencode","model":"model","mode":"mode","instructions":"instruction","skills":[],"max_seconds":60,"max_output_bytes":65536},
        "budgets":budgets()})).unwrap()
}
fn session_receipt() -> SessionReceipt {
    SessionReceipt {
        identity: open().identity,
        fence: session(),
        budgets: budgets(),
    }
}
fn turn() -> TurnCommand {
    TurnCommand {
        session: session(),
        identity: TurnIdentity {
            command_id: "command".into(),
            turn_id: "turn".into(),
            fingerprint: "a".repeat(64),
        },
        turn_index: 1,
        input: "hello".into(),
        budgets: ExecutionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            lease_duration_ms: 10000,
        },
    }
}
fn turn_receipt() -> TurnReceipt {
    let command = turn();
    TurnReceipt {
        identity: command.identity,
        fence: TurnFence {
            session: session(),
            turn_id: "turn".into(),
            turn_index: 1,
            turn_epoch: 1,
        },
        budgets: command.budgets,
    }
}
fn event() -> SessionEvent {
    SessionEvent {
        session: session(),
        sequence: 1,
        previous: None,
        digest: "e".repeat(64),
        event: SessionEventKind::Text {
            turn: turn_receipt().fence,
            kind: TextKind::Text,
            text: "hello".into(),
        },
    }
}
fn controller(value: &Value) -> Result<SessionControllerMessage> {
    decode_session_controller_frame(&value.to_string())
}
fn worker(value: &Value) -> Result<SessionWorkerMessage> {
    decode_session_worker_frame(&value.to_string())
}

#[test]
fn rpc_replies_and_rejections_correlate_all_operations_without_consuming_other_waiters() {
    use SessionControllerMessage as C;
    use SessionWorkerMessage as W;
    let cases = vec![
        (
            C::OpenSession {
                command: Box::new(open()),
            },
            W::SessionAccepted {
                receipt: session_receipt(),
            },
        ),
        (
            C::SubmitTurn {
                command: Box::new(turn()),
            },
            W::TurnAccepted {
                receipt: turn_receipt(),
            },
        ),
        (
            C::RenewSession {
                session: session(),
                sequence: 1,
                lease_duration_ms: 1000,
            },
            W::SessionRenewed {
                session: session(),
                sequence: 1,
                lease_duration_ms: 1000,
            },
        ),
        (
            C::RenewTurn {
                turn: turn_receipt().fence,
                sequence: 1,
                lease_duration_ms: 1000,
            },
            W::TurnRenewed {
                turn: turn_receipt().fence,
                sequence: 1,
                lease_duration_ms: 1000,
            },
        ),
        (
            C::CancelTurn {
                turn: turn_receipt().fence,
            },
            W::CancelRequested {
                turn: turn_receipt().fence,
            },
        ),
        (
            C::CloseSession { session: session() },
            W::CloseRequested { session: session() },
        ),
        (
            C::ReconcileSession {
                channel: channel(),
                identity: open().identity,
            },
            W::SessionReconciled {
                channel: channel(),
                identity: open().identity,
                record: None,
            },
        ),
        (
            C::ReconcileTurn {
                channel: channel(),
                session: session(),
                identity: turn().identity,
            },
            W::TurnReconciled {
                channel: channel(),
                session: session(),
                identity: turn().identity,
                record: None,
            },
        ),
        (
            C::ReplayStream {
                channel: channel(),
                session: session(),
                after_sequence: 0,
                after_digest: None,
                limit: 8,
            },
            W::StreamReplayed {
                channel: channel(),
                session: session(),
                after_sequence: 0,
                after_digest: None,
                limit: 8,
                through_sequence: 0,
                events: vec![],
                more: false,
            },
        ),
        (
            C::AckStream {
                channel: channel(),
                session: session(),
                through_sequence: 1,
                event_digest: "e".repeat(64),
            },
            W::StreamAcknowledged {
                channel: channel(),
                session: session(),
                through_sequence: 1,
                event_digest: "e".repeat(64),
            },
        ),
    ];
    for (index, (request, reply)) in cases.iter().enumerate() {
        assert_eq!(request.channel(), &channel());
        reply.validate_for_request(request).unwrap();
        let rejected = W::Rejected {
            channel: channel(),
            request: request.reference(),
            code: ExecutionErrorCode::Unavailable,
        };
        rejected.validate_for_request(request).unwrap();
        let mut wrong_channel = channel();
        wrong_channel.worker_boot_id = "restarted-worker".into();
        assert!(
            W::Rejected {
                channel: wrong_channel,
                request: request.reference(),
                code: ExecutionErrorCode::Unavailable
            }
            .validate_for_request(request)
            .is_err()
        );
        for (other_index, (other_request, other_reply)) in cases.iter().enumerate() {
            if index == other_index {
                continue;
            }
            assert!(
                other_reply.validate_for_request(request).is_err(),
                "mismatched operation {index}/{other_index}"
            );
            assert!(
                rejected.validate_for_request(other_request).is_err(),
                "mismatched rejection {index}/{other_index}"
            );
        }
        assert!(
            W::SessionReady {
                receipt: session_receipt()
            }
            .validate_for_request(request)
            .is_err()
        );
        assert!(
            W::Event { event: event() }
                .validate_for_request(request)
                .is_err()
        );
        let mut changed = serde_json::to_value(request).unwrap();
        let pointer = match index {
            0 => "/command/identity/fingerprint",
            1 => "/command/identity/fingerprint",
            2 | 3 => "/sequence",
            4 => "/turn/turn_epoch",
            5 => "/session/session_epoch",
            6 | 7 => "/identity/fingerprint",
            8 => "/limit",
            9 => "/event_digest",
            _ => unreachable!(),
        };
        *changed.pointer_mut(pointer).unwrap() = if matches!(index, 2..=5 | 8) {
            json!(2)
        } else {
            json!("f".repeat(64))
        };
        let changed: C = serde_json::from_value(changed).unwrap();
        changed.validate().unwrap();
        assert!(reply.validate_for_request(&changed).is_err());
        assert!(rejected.validate_for_request(&changed).is_err());
    }
}

#[test]
fn typed_roundtrips_keep_v3_codecs_separate_from_negotiated_v4_hello() {
    let request = SessionControllerMessage::OpenSession {
        command: Box::new(open()),
    };
    let raw = serde_json::to_string(&request).unwrap();
    assert_eq!(decode_session_controller_frame(&raw).unwrap(), request);
    assert!(super::super::decode_execution_controller_frame(&raw).is_err());
    assert!(super::super::decode_frame(&raw).is_err());
    let hello = json!({"type":"hello","protocol_version":4,"host_id":"host","workspace_id":"workspace",
        "enrollment_epoch":1,"credential_revision":1,"worker_boot_id":"worker",
        "capabilities":{"protocol_version":1,"version":"fixture","os":"linux","architecture":"x86_64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1}});
    assert!(super::super::decode_frame(&hello.to_string()).is_ok());
    let reply = SessionWorkerMessage::SessionAccepted {
        receipt: session_receipt(),
    };
    assert_eq!(
        decode_session_worker_frame(&serde_json::to_string(&reply).unwrap()).unwrap(),
        reply
    );
    session_receipt().validate_for_open(&open()).unwrap();
    turn_receipt().validate_for_command(&turn()).unwrap();
}

#[test]
fn strict_fields_human_grant_and_encoded_byte_limits_are_enforced() {
    let original = json!({"type":"open_session","command":open()});
    for pointer in [
        "/command/definition",
        "/command/descriptor",
        "/command/budgets",
    ] {
        let mut value = original.clone();
        value.pointer_mut(pointer).unwrap()["trusted_path"] = json!("/private");
        assert!(controller(&value).is_err());
    }
    let mut service = original.clone();
    service["command"]["descriptor"]["grant"]["principal"]["kind"] = json!("service");
    assert!(controller(&service).is_err());
    let mut huge = original;
    huge["command"]["definition"]["description"] = json!("x".repeat(MAX_WORKER_FRAME_BYTES));
    assert!(controller(&huge).is_err());
    let mut input = turn();
    input.input = "界".repeat(MAX_SESSION_INPUT_BYTES / 3);
    input.validate().unwrap();
    input.input.push('界');
    assert!(input.validate().is_err());
    let mut escaped = turn();
    escaped.input = format!("x{}", "\t".repeat(MAX_SESSION_INPUT_BYTES - 1));
    escaped.validate().unwrap();
    let mut raw = serde_json::to_string(&SessionControllerMessage::SubmitTurn {
        command: Box::new(escaped),
    })
    .unwrap();
    raw.push_str(&" ".repeat(MAX_WORKER_FRAME_BYTES));
    assert!(decode_session_controller_frame(&raw).is_err());
}

#[test]
fn conservative_budgets_and_safe_counters_reject_overflow() {
    let mut command = open();
    command.budgets.retained_bytes = MAX_SESSION_RETAINED_BYTES + 1;
    assert!(command.validate().is_err());
    command = open();
    command.budgets.max_turns = 129;
    assert!(command.validate().is_err());
    command = open();
    command.budgets.max_seconds = 61;
    assert!(command.validate().is_err());
    command = open();
    command.budgets.idle_timeout_ms = 30001;
    assert!(command.validate().is_err());
    command = open();
    command.budgets.max_seconds = 1;
    command.budgets.idle_timeout_ms = 1001;
    command.budgets.lease_duration_ms = 1000;
    assert!(command.validate().is_err());
    for sequence in [0, MAX_EXECUTION_INTEGER + 1, u64::MAX] {
        assert!(
            SessionControllerMessage::RenewSession {
                session: session(),
                sequence,
                lease_duration_ms: 1000
            }
            .validate()
            .is_err()
        );
    }
    for duration in [0, 999, 10001, u64::MAX] {
        assert!(
            SessionControllerMessage::RenewTurn {
                turn: turn_receipt().fence,
                sequence: 1,
                lease_duration_ms: duration
            }
            .validate()
            .is_err()
        );
    }
    let mut receipt = turn_receipt();
    receipt.fence.turn_epoch = u64::MAX;
    assert!(receipt.validate().is_err());
}

#[test]
fn exact_session_receipt_and_turn_pins_prevent_cross_session_or_epoch_use() {
    let mut receipt = session_receipt();
    receipt.budgets.lease_duration_ms = 9000;
    assert!(receipt.validate_for_open(&open()).is_err());
    let mut command = turn();
    command.session.worker_session_id = "other".into();
    assert!(command.validate_for_session(&session_receipt()).is_err());
    command = turn();
    command.session.session_epoch = 2;
    assert!(command.validate_for_session(&session_receipt()).is_err());
    command = turn();
    command.turn_index = 2;
    let mut receipt = session_receipt();
    receipt.budgets.max_turns = 1;
    assert!(command.validate_for_session(&receipt).is_err());
    let mut reply = turn_receipt();
    reply.identity.turn_id = "other".into();
    assert!(reply.validate().is_err());
    let mut reply = turn_receipt();
    reply.budgets.lease_duration_ms = 9000;
    assert!(reply.validate_for_command(&turn()).is_err());
}

#[test]
fn historical_fences_are_only_explicit_query_and_negative_ack_paths() {
    let mut new_channel = channel();
    new_channel.worker_boot_id = "new-boot".into();
    new_channel.channel_id = "new-channel".into();
    let old = turn();
    assert!(
        SessionControllerMessage::SubmitTurn {
            command: Box::new(old)
        }
        .validate_for_channel(&new_channel)
        .is_err()
    );
    assert!(
        SessionControllerMessage::RenewSession {
            session: session(),
            sequence: 1,
            lease_duration_ms: 1000
        }
        .validate_for_channel(&new_channel)
        .is_err()
    );
    assert!(
        SessionControllerMessage::RenewTurn {
            turn: turn_receipt().fence,
            sequence: 1,
            lease_duration_ms: 1000
        }
        .validate_for_channel(&new_channel)
        .is_err()
    );
    let ack = SessionControllerMessage::AckStream {
        channel: new_channel.clone(),
        session: session(),
        through_sequence: 1,
        event_digest: event().digest,
    };
    ack.validate_for_channel(&new_channel).unwrap();
    let query = SessionControllerMessage::ReconcileTurn {
        channel: new_channel.clone(),
        session: session(),
        identity: turn().identity,
    };
    query.validate_for_channel(&new_channel).unwrap();
    let reply = SessionWorkerMessage::TurnReconciled {
        channel: new_channel.clone(),
        session: session(),
        identity: turn().identity,
        record: Some(TurnRecord {
            receipt: turn_receipt(),
            status: TurnStatus::Running,
            completion: None,
        }),
    };
    reply.validate_for_channel(&new_channel).unwrap();
    assert!(
        SessionWorkerMessage::TurnAccepted {
            receipt: turn_receipt()
        }
        .validate_for_channel(&new_channel)
        .is_err()
    );
    let mut rejected = json!({"type":"rejected","channel":new_channel,"request":{"type":"renew_turn","turn":turn_receipt().fence,"sequence":1,"lease_duration_ms":1000},"code":"unavailable"});
    assert!(worker(&rejected).is_err());
    rejected["request"] = json!({"type":"ack","session":session(),"through_sequence":1,"event_digest":"e".repeat(64)});
    assert!(worker(&rejected).is_ok());
}

#[test]
fn output_chain_rejects_future_missing_and_wrong_session_references() {
    let first = event();
    first.validate().unwrap();
    let mut terminal = SessionEvent {
        session: session(),
        sequence: 2,
        previous: Some(StreamPosition {
            session: session(),
            sequence: 1,
            digest: first.digest,
        }),
        digest: "f".repeat(64),
        event: SessionEventKind::TurnCompleted {
            receipt: turn_receipt(),
            status: TurnTerminalStatus::Completed,
        },
    };
    terminal.validate().unwrap();
    terminal.previous.as_mut().unwrap().sequence = 2;
    assert!(terminal.validate().is_err());
    terminal.previous.as_mut().unwrap().sequence = 1;
    terminal
        .previous
        .as_mut()
        .unwrap()
        .session
        .worker_session_id = "other".into();
    assert!(terminal.validate().is_err());
    terminal.previous = None;
    assert!(terminal.validate().is_err());
    terminal = event();
    if let SessionEventKind::Text { turn, .. } = &mut terminal.event {
        turn.session.session_epoch = 2;
    }
    assert!(terminal.validate().is_err());
    let mut chunk = event();
    if let SessionEventKind::Text { text, .. } = &mut chunk.event {
        *text = "界".repeat(1365);
    }
    chunk.validate().unwrap();
    if let SessionEventKind::Text { text, .. } = &mut chunk.event {
        text.push('界');
    }
    assert!(chunk.validate().is_err());
}

#[test]
fn turn_completion_never_claims_process_stop_or_releases_session_reservation() {
    let complete = TurnRecord {
        receipt: turn_receipt(),
        status: TurnStatus::Completed,
        completion: Some(TurnCompletion {
            status: TurnTerminalStatus::Completed,
            final_sequence: 2,
            event_digest: "e".repeat(64),
        }),
    };
    complete.validate().unwrap();
    let terminal_event = SessionEvent {
        session: session(),
        sequence: 2,
        previous: Some(StreamPosition {
            session: session(),
            sequence: 1,
            digest: "a".repeat(64),
        }),
        digest: "e".repeat(64),
        event: SessionEventKind::TurnCompleted {
            receipt: turn_receipt(),
            status: TurnTerminalStatus::Completed,
        },
    };
    complete.validate_for_event(&terminal_event).unwrap();
    let mut future = complete.clone();
    future.completion.as_mut().unwrap().final_sequence = 3;
    assert!(future.validate_for_event(&terminal_event).is_err());
    let mut wrong = terminal_event.clone();
    wrong.session.worker_session_id = "other".into();
    assert!(complete.validate_for_event(&wrong).is_err());
    let session_record = SessionRecord {
        receipt: session_receipt(),
        phase: SessionPhase::Idle,
        active_turn: None,
        stop: None,
    };
    session_record.validate().unwrap(); // Still owns a live session; no stop record.
    let mut stopped = session_record;
    stopped.phase = SessionPhase::Stopped;
    assert!(stopped.validate().is_err());
    stopped.stop = Some(SessionStop {
        reason: SessionStopReason::Closed,
        final_sequence: 3,
        event_digest: "f".repeat(64),
    });
    stopped.validate().unwrap();
    stopped.active_turn = Some(turn_receipt());
    assert!(stopped.validate().is_err());
    let mut wrong = complete;
    wrong.status = TurnStatus::Running;
    assert!(wrong.validate().is_err());
    let terminal = json!({"type":"event","event":{"session":session(),"sequence":1,"previous":null,"digest":"e".repeat(64),
        "event":{"type":"turn_completed","receipt":turn_receipt(),"status":"completed","text":"duplicate full final text"}}});
    assert!(worker(&terminal).is_err());
}

#[test]
fn canonical_turn_hash_has_external_sha256_golden_and_excludes_only_self_fingerprint() {
    // SHA256 fixture independently calculated with Python hashlib over this
    // exact ordered UTF-8 JSON and the documented v4 turn domain plus NUL.
    assert_eq!(
        session_turn_fingerprint(&turn()).unwrap(),
        "ee69f0fa5ffd399f7187521b3126b3992f3657519421286f365eb078988bbd77"
    );
    let mut changed = turn();
    changed.identity.fingerprint = "b".repeat(64);
    assert_eq!(
        session_turn_fingerprint(&changed).unwrap(),
        session_turn_fingerprint(&turn()).unwrap()
    );
    changed = turn();
    changed.input = "different".into();
    assert_ne!(
        session_turn_fingerprint(&changed).unwrap(),
        session_turn_fingerprint(&turn()).unwrap()
    );
    changed = turn();
    changed.session.channel.worker_boot_id = "different".into();
    assert_ne!(
        session_turn_fingerprint(&changed).unwrap(),
        session_turn_fingerprint(&turn()).unwrap()
    );
    changed = turn();
    changed.session.session_epoch = 2;
    assert_ne!(
        session_turn_fingerprint(&changed).unwrap(),
        session_turn_fingerprint(&turn()).unwrap()
    );
    changed = turn();
    changed.identity.command_id = "different".into();
    assert_ne!(
        session_turn_fingerprint(&changed).unwrap(),
        session_turn_fingerprint(&turn()).unwrap()
    );
    changed = turn();
    changed.identity.fingerprint = "INVALID".into();
    assert!(session_turn_fingerprint(&changed).is_err());
}

#[test]
fn open_and_event_hashes_cover_immutable_pins_and_chain_without_self_digest() {
    let original = open();
    let hash = session_open_fingerprint(&original).unwrap();
    let mut changed = original.clone();
    changed.identity.fingerprint = "b".repeat(64);
    assert_eq!(session_open_fingerprint(&changed).unwrap(), hash);
    let mut changed = original.clone();
    changed.profile.version = 2;
    assert_ne!(session_open_fingerprint(&changed).unwrap(), hash);
    let mut changed = original.clone();
    changed.descriptor.scope.credential_revision = 2;
    assert_ne!(session_open_fingerprint(&changed).unwrap(), hash);
    let mut changed = original.clone();
    changed.definition.instructions = Some("other instructions".into());
    assert_ne!(session_open_fingerprint(&changed).unwrap(), hash);
    let mut changed = original;
    changed.budgets.max_turns = 127;
    assert_ne!(session_open_fingerprint(&changed).unwrap(), hash);
    let original = event();
    let hash = session_event_digest(&original).unwrap();
    let mut changed = original.clone();
    changed.digest = "a".repeat(64);
    assert_eq!(session_event_digest(&changed).unwrap(), hash);
    let mut changed = original.clone();
    if let SessionEventKind::Text { text, .. } = &mut changed.event {
        *text = "different".into();
    }
    assert_ne!(session_event_digest(&changed).unwrap(), hash);
    let mut changed = original;
    changed.sequence = 2;
    changed.previous = Some(StreamPosition {
        session: session(),
        sequence: 1,
        digest: "e".repeat(64),
    });
    let before = session_event_digest(&changed).unwrap();
    changed.previous.as_mut().unwrap().digest = "f".repeat(64);
    assert_ne!(session_event_digest(&changed).unwrap(), before);
}

fn replay_page() -> (
    SessionControllerMessage,
    SessionWorkerMessage,
    Vec<SessionEvent>,
) {
    let mut current = channel();
    current.channel_id = "replacement-channel".into();
    current.worker_boot_id = "replacement-worker".into();
    let mut first = event();
    first.digest = session_event_digest(&first).unwrap();
    let mut second = event();
    second.sequence = 2;
    second.previous = Some(StreamPosition {
        session: session(),
        sequence: 1,
        digest: first.digest.clone(),
    });
    second.digest = session_event_digest(&second).unwrap();
    let events = vec![first, second];
    let request = SessionControllerMessage::ReplayStream {
        channel: current.clone(),
        session: session(),
        after_sequence: 0,
        after_digest: None,
        limit: 2,
    };
    let reply = SessionWorkerMessage::StreamReplayed {
        channel: current,
        session: session(),
        after_sequence: 0,
        after_digest: None,
        limit: 2,
        through_sequence: 3,
        events: events.clone(),
        more: true,
    };
    (request, reply, events)
}

#[test]
fn historical_replay_preserves_original_hashes_without_enabling_positive_old_channel_events() {
    let (request, reply, original) = replay_page();
    let SessionControllerMessage::ReplayStream {
        channel: current, ..
    } = &request
    else {
        unreachable!()
    };
    request.validate_for_channel(current).unwrap();
    reply.validate_for_channel(current).unwrap();
    reply.validate_for_request(&request).unwrap();
    let bytes = serde_json::to_string(&reply).unwrap();
    let decoded = decode_session_worker_frame(&bytes).unwrap();
    assert_eq!(decoded, reply);
    let SessionWorkerMessage::StreamReplayed { events, .. } = decoded else {
        unreachable!()
    };
    assert_eq!(events, original);
    for event in events {
        assert_eq!(event.digest, session_event_digest(&event).unwrap());
        assert!(
            SessionWorkerMessage::Event { event }
                .validate_for_channel(current)
                .is_err()
        );
    }
    let SessionControllerMessage::ReplayStream {
        channel, session, ..
    } = request
    else {
        unreachable!()
    };
    SessionWorkerMessage::Rejected {
        channel: channel.clone(),
        request: SessionRequestRef::Replay {
            session: session.clone(),
            after_sequence: 0,
            after_digest: None,
            limit: 2,
        },
        code: ExecutionErrorCode::Unavailable,
    }
    .validate_for_channel(&channel)
    .unwrap();
    assert!(
        SessionControllerMessage::CloseSession { session }
            .validate_for_channel(&channel)
            .is_err()
    );
}

#[test]
fn replay_rejects_wrong_anchors_sessions_hashes_missing_history_and_correlation() {
    let (request, reply, events) = replay_page();
    let value = serde_json::to_value(&reply).unwrap();
    let mut anchored = value.clone();
    anchored["after_sequence"] = json!(1);
    anchored["after_digest"] = json!(events[0].digest);
    anchored["events"] = json!([events[1]]);
    anchored["through_sequence"] = json!(2);
    anchored["more"] = json!(false);
    worker(&anchored).unwrap();
    anchored["after_digest"] = json!("f".repeat(64));
    assert!(worker(&anchored).is_err());
    for bad in [
        {
            let mut v = value.clone();
            v["after_sequence"] = json!(1);
            v["after_digest"] = json!(events[0].digest);
            v
        },
        {
            let mut v = value.clone();
            v["after_digest"] = json!("a".repeat(64));
            v
        },
        {
            let mut v = value.clone();
            v["events"][0]["sequence"] = json!(2);
            v
        },
        {
            let mut v = value.clone();
            v["events"][1]["session"]["session_epoch"] = json!(2);
            v
        },
        {
            let mut v = value.clone();
            v["events"][1]["previous"]["digest"] = json!("f".repeat(64));
            v
        },
        {
            let mut v = value.clone();
            v["events"][0]["digest"] = json!("f".repeat(64));
            v
        },
        {
            let mut v = value.clone();
            v["events"] = json!([]);
            v
        },
        {
            let mut v = value.clone();
            v["more"] = json!(false);
            v
        },
        {
            let mut v = value.clone();
            v["limit"] = json!(1);
            v
        },
        {
            let mut v = value.clone();
            v["through_sequence"] = json!(1);
            v
        },
        {
            let mut v = value.clone();
            v["through_sequence"] = json!(MAX_EXECUTION_INTEGER + 1);
            v
        },
    ] {
        assert!(worker(&bad).is_err(), "accepted malformed replay: {bad}");
    }
    let query = serde_json::to_value(&request).unwrap();
    for bad in [
        {
            let mut v = query.clone();
            v["limit"] = json!(0);
            v
        },
        {
            let mut v = query.clone();
            v["limit"] = json!(9);
            v
        },
        {
            let mut v = query.clone();
            v["after_sequence"] = json!(1);
            v
        },
        {
            let mut v = query.clone();
            v["after_sequence"] = json!(MAX_EXECUTION_INTEGER + 1);
            v["after_digest"] = json!("a".repeat(64));
            v
        },
    ] {
        assert!(controller(&bad).is_err());
    }
    let mut wrong = query.clone();
    wrong["limit"] = json!(3);
    assert!(
        reply
            .validate_for_request(&controller(&wrong).unwrap())
            .is_err()
    );
    wrong = query.clone();
    wrong["channel"]["controller_boot_id"] = json!("other-boot");
    assert!(
        reply
            .validate_for_request(&controller(&wrong).unwrap())
            .is_err()
    );
    wrong = query;
    wrong["session"]["worker_session_id"] = json!("other-session");
    assert!(
        reply
            .validate_for_request(&controller(&wrong).unwrap())
            .is_err()
    );
}

#[test]
fn replay_empty_end_and_byte_bounded_partial_pages_are_explicit() {
    let (_, reply, events) = replay_page();
    let mut value = serde_json::to_value(reply).unwrap();
    value["after_sequence"] = json!(2);
    value["after_digest"] = json!(events[1].digest);
    value["through_sequence"] = json!(2);
    value["events"] = json!([]);
    value["more"] = json!(false);
    worker(&value).unwrap();
    value["through_sequence"] = json!(3);
    assert!(worker(&value).is_err()); // Missing retained history is not EOF.
    value["more"] = json!(true);
    assert!(worker(&value).is_err()); // Nor an empty page to loop over forever.
    value["after_sequence"] = json!(0);
    value["after_digest"] = Value::Null;
    value["through_sequence"] = json!(0);
    value["more"] = json!(false);
    worker(&value).unwrap();
    // A producer may shorten a page for the frame budget, but cannot drop its
    // watermark or claim EOF while a later durable event remains.
    value["events"] = json!([events[0]]);
    value["through_sequence"] = json!(2);
    value["more"] = json!(true);
    worker(&value).unwrap();
    let mut previous = None;
    let mut large = Vec::new();
    for sequence in 1..=8 {
        let mut e = event();
        e.sequence = sequence;
        e.previous = previous;
        let SessionEventKind::Text { text, .. } = &mut e.event else {
            unreachable!()
        };
        *text = "\t".repeat(MAX_SESSION_CHUNK_BYTES);
        e.digest = session_event_digest(&e).unwrap();
        previous = Some(StreamPosition {
            session: e.session.clone(),
            sequence,
            digest: e.digest.clone(),
        });
        large.push(e);
    }
    value["events"] = json!(large);
    value["limit"] = json!(8);
    value["through_sequence"] = json!(8);
    value["more"] = json!(false);
    assert!(worker(&value).is_err()); // Aggregate escaped frame still <=64KiB.
}
