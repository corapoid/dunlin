use super::*;
use crate::worker_channel::tests::{Fixture, fixture};

fn enabled() -> Fixture {
    let mut fixture = fixture(true, true);
    let config = Arc::get_mut(&mut fixture.state.config)
        .unwrap()
        .worker_controller
        .as_mut()
        .unwrap();
    config.execution_enabled = true;
    config.persistent_sessions = true;
    fixture
}
fn lease(fixture: &Fixture, channel: &str) -> (WorkerIdentity, ChannelLease) {
    let store = fixture.state.store.lock().unwrap();
    let identity = store
        .authenticate_worker_token(&fixture.credential.credential)
        .unwrap();
    let lease = store
        .open_worker_channel(
            &identity,
            &fixture.state.worker_channels.boot_id,
            "session-worker-boot",
            channel,
            now_ms().unwrap(),
            15000,
        )
        .unwrap();
    (identity, lease)
}

#[test]
fn session_transport_proof_requires_transaction_current_slot_and_own_producer() {
    let fixture = enabled();
    let (sender, _events) = mpsc::channel(8);
    let producer = fixture
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
    let (identity, channel) = lease(&fixture, "session-channel-one");
    let (_registered, transport, _requests) = fixture
        .state
        .worker_channels
        .register_session_transport(&fixture.state, &identity, &channel, "n-controller".into())
        .unwrap();
    let store = fixture.state.store.lock().unwrap();
    assert!(
        transport
            .attest_in_transaction(&store, &identity, &channel, transport.fence())
            .is_err()
    );
    store
        .remote_binding_transaction(false, |store| {
            transport.attest_in_transaction(store, &identity, &channel, transport.fence())
        })
        .unwrap();
    assert!(
        fixture
            .state
            .worker_channels
            .execution_transport(identity.host_id())
            .is_err()
    );
    drop(store);
    let (next_identity, next_channel) = lease(&fixture, "session-channel-two");
    let (_next, next, _next_requests) = fixture
        .state
        .worker_channels
        .register_session_transport(
            &fixture.state,
            &next_identity,
            &next_channel,
            "n-controller".into(),
        )
        .unwrap();
    let store = fixture.state.store.lock().unwrap();
    assert!(
        store
            .remote_binding_transaction(false, |store| transport.attest_in_transaction(
                store,
                &identity,
                &channel,
                transport.fence()
            ))
            .is_err()
    );
    store
        .remote_binding_transaction(false, |store| {
            next.attest_in_transaction(store, &next_identity, &next_channel, next.fence())
        })
        .unwrap();
    drop(producer);
    assert!(
        store
            .remote_binding_transaction(false, |store| next.attest_in_transaction(
                store,
                &next_identity,
                &next_channel,
                next.fence()
            ))
            .is_err()
    );
    assert!(
        fixture
            .state
            .worker_channels
            .session_transport(next.host_id())
            .is_err()
    );
}

#[tokio::test]
async fn unsolicited_event_and_wrong_renew_sequence_never_consume_pending_control() {
    let fence = wire::SessionFence {
        channel: ChannelFence {
            channel_id: "channel".into(),
            controller_boot_id: "controller".into(),
            worker_boot_id: "worker".into(),
        },
        controller_session_id: "controller-session".into(),
        worker_session_id: "worker-session".into(),
        session_epoch: 1,
    };
    let request = Request::RenewSession {
        session: fence.clone(),
        sequence: 2,
        lease_duration_ms: 1000,
    };
    let semaphore = Arc::new(Semaphore::new(1));
    let (sender, receiver) = oneshot::channel();
    let mut pending = BTreeMap::new();
    pending.insert(
        request_key(&request).unwrap(),
        SessionRequest {
            message: Control::Legacy(Box::new(request)),
            deadline: tokio::time::Instant::now() + SEND,
            response: sender,
            _permit: semaphore.clone().acquire_owned().await.unwrap(),
        },
    );
    let mut event = wire::SessionEvent {
        session: fence.clone(),
        sequence: 1,
        previous: None,
        digest: "0".repeat(64),
        event: wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    };
    event.digest = wire::session_event_digest(&event).unwrap();
    assert!(
        take_reply(&mut pending, &Reply::Event { event })
            .unwrap()
            .is_none()
    );
    assert!(
        take_reply(
            &mut pending,
            &Reply::SessionRenewed {
                session: fence.clone(),
                sequence: 1,
                lease_duration_ms: 1000
            }
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(pending.len(), 1);
    assert_eq!(semaphore.available_permits(), 0);
    let reply = Reply::SessionRenewed {
        session: fence,
        sequence: 2,
        lease_duration_ms: 1000,
    };
    let selected = take_reply(&mut pending, &reply).unwrap().unwrap();
    selected.response.send(Ok(reply.clone())).unwrap();
    assert_eq!(receiver.await.unwrap().unwrap(), reply);
    assert!(pending.is_empty());
    drop(selected._permit);
    assert_eq!(semaphore.available_permits(), 1);
}

pub(super) fn broker_command(
    transport: &SessionTransport,
) -> wire::broker::OpenBrokerSessionCommand {
    let host = transport.identity().host();
    let open = serde_json::from_value(serde_json::json!({
        "channel": transport.fence(), "controller_session_id":"browser-session",
        "identity":{"intent_id":"open-intent","fingerprint":"a".repeat(64)},
        "descriptor":{
            "scope":{"controller_node_id":"n-controller","worker_node_id":host.definition.node_id,
                "workspace_id":host.workspace_id,"host_id":host.id,
                "enrollment_epoch":host.enrollment_epoch,"credential_revision":host.credential_revision},
            "export":{"id":"export","name":"Export","repo_kind":"git",
                "checkout_identity_digest":"b".repeat(64),"settings_digest":"c".repeat(64),"revision":1,"active":true},
            "grant":{"principal":{"kind":"human","id":"owner"},"permissions":["read","execute"],
                "revision":1,"export_revision":1,"active":true}},
        "profile":{"agent_id":"agent","version":1,"digest":"d".repeat(64)},
        "definition":{"schema_version":2,"name":"Agent","description":"","harness":"opencode",
            "model":"model","mode":"mode","instructions":"instruction","skills":[],
            "max_seconds":60,"max_output_bytes":65536,
            "mcp":[{"provider_id":"work","name":"lookup","definition_digest":"a".repeat(64)}]},
        "budgets":{"max_seconds":60,"max_output_bytes":65536,"idle_timeout_ms":30000,
            "lease_duration_ms":10000,"max_turns":128,"retained_bytes":wire::MAX_SESSION_RETAINED_BYTES}
    })).unwrap();
    let mut command = wire::broker::OpenBrokerSessionCommand {
        protocol_version: 5,
        open,
        broker_limits: wire::broker::BrokerLimits {
            max_pending: 2,
            max_reply_bytes: 65536,
        },
    };
    command.open.identity.fingerprint =
        wire::broker::broker_session_open_fingerprint(&command).unwrap();
    command
}
fn receipt(command: &wire::broker::OpenBrokerSessionCommand) -> wire::SessionReceipt {
    wire::SessionReceipt {
        identity: command.open.identity.clone(),
        fence: wire::SessionFence {
            channel: command.open.channel.clone(),
            controller_session_id: command.open.controller_session_id.clone(),
            worker_session_id: "native-session".into(),
            session_epoch: 1,
        },
        budgets: command.open.budgets.clone(),
    }
}

#[tokio::test]
async fn broker_open_queue_keeps_v5_wire_and_correlates_exact_receipt_or_rejection() {
    let mut fixture = enabled();
    Arc::get_mut(&mut fixture.state.config)
        .unwrap()
        .worker_controller
        .as_mut()
        .unwrap()
        .mcp_broker = true;
    let (sender, _events) = mpsc::channel(8);
    let _producer = fixture
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
    let (identity, channel) = lease(&fixture, "broker-channel");
    let (_registration, transport, mut requests) = fixture
        .state
        .worker_channels
        .register_broker_session_transport(
            &fixture.state,
            &identity,
            &channel,
            "n-controller".into(),
            wire::broker::BrokerLimits {
                max_pending: 2,
                max_reply_bytes: 65536,
            },
        )
        .unwrap();
    let command = broker_command(&transport);
    let task_transport = transport.clone();
    let original = command.clone();
    let waiter = tokio::spawn(async move { task_transport.open_broker_session(original).await });
    let queued = requests.recv().await.unwrap();
    let encoded = queued.message.encode().unwrap();
    assert!(wire::decode_session_controller_frame(&encoded).is_err());
    assert_eq!(
        wire::broker::decode_broker_session_controller_frame(
            encoded.as_bytes(),
            transport.fence(),
            transport.broker_offer().unwrap(),
        )
        .unwrap(),
        wire::broker::BrokerSessionControllerMessage::OpenBrokerSession {
            command: Box::new(command.clone())
        }
    );
    let reply = Reply::SessionAccepted {
        receipt: receipt(&command),
    };
    let mut pending = BTreeMap::from([(queued.message.key().unwrap(), queued)]);
    assert!(
        take_reply(
            &mut pending,
            &Reply::SessionReady {
                receipt: receipt(&command)
            }
        )
        .unwrap()
        .is_none()
    );
    for field in ["boot", "identity", "budget", "session"] {
        let mut wrong = receipt(&command);
        match field {
            "boot" => wrong.fence.channel.worker_boot_id = "other".into(),
            "identity" => wrong.identity.intent_id = "other".into(),
            "budget" => wrong.budgets.max_turns -= 1,
            _ => wrong.fence.controller_session_id = "other".into(),
        }
        assert!(
            take_reply(&mut pending, &Reply::SessionAccepted { receipt: wrong })
                .unwrap()
                .is_none(),
            "{field}"
        );
    }
    let rejected = Reply::Rejected {
        channel: transport.fence().clone(),
        request: wire::SessionRequestRef::Open {
            identity: command.open.identity.clone(),
        },
        code: worker::ExecutionErrorCode::Unavailable,
    };
    assert!(Control::Broker(Box::new(command.clone())).accepts(&rejected));
    let mut wrong = rejected.clone();
    if let Reply::Rejected {
        request: wire::SessionRequestRef::Open { identity },
        ..
    } = &mut wrong
    {
        identity.fingerprint = "e".repeat(64);
    }
    assert!(take_reply(&mut pending, &wrong).unwrap().is_none());
    let selected = take_reply(&mut pending, &reply).unwrap().unwrap();
    selected.response.send(Ok(reply.clone())).unwrap();
    drop(selected._permit);
    assert_eq!(waiter.await.unwrap().unwrap(), reply);
    assert!(pending.is_empty());
    assert_eq!(transport.slot.permits.available_permits(), PENDING);
    // Payload modification with the old identity cannot correlate even rejection.
    let mut modified = command;
    modified.open.definition.instructions = Some("changed".into());
    assert!(!Control::Broker(Box::new(modified)).accepts(&rejected));
}

#[tokio::test]
async fn broker_open_rejects_legacy_channel_scope_changes_and_excess_budgets_before_enqueue() {
    let mut fixture = enabled();
    let (sender, _events) = mpsc::channel(8);
    let _producer = fixture
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
    let (identity, channel) = lease(&fixture, "legacy-channel");
    let (_registration, transport, mut requests) = fixture
        .state
        .worker_channels
        .register_session_transport(&fixture.state, &identity, &channel, "n-controller".into())
        .unwrap();
    let command = broker_command(&transport);
    assert!(
        transport
            .open_broker_session(command.clone())
            .await
            .is_err()
    );
    assert!(requests.try_recv().is_err());
    assert_eq!(transport.slot.permits.available_permits(), PENDING);
    Arc::make_mut(&mut fixture.state.config)
        .worker_controller
        .as_mut()
        .unwrap()
        .mcp_broker = true;
    let (identity, channel) = lease(&fixture, "upgraded-broker-channel");
    let (_broker_registration, transport, mut broker_requests) = fixture
        .state
        .worker_channels
        .register_broker_session_transport(
            &fixture.state,
            &identity,
            &channel,
            "n-controller".into(),
            command.broker_limits.clone(),
        )
        .unwrap();
    let command = broker_command(&transport);
    assert!(
        transport
            .check(&Control::Broker(Box::new(command.clone())))
            .is_ok()
    );
    let original = Control::Broker(Box::new(command.clone()));
    let offer = &command.broker_limits;
    assert!(
        original
            .validate_for_channel(transport.fence(), Some(offer))
            .is_ok()
    );
    for field in [
        "boot",
        "pending",
        "size",
        "host",
        "workspace",
        "node",
        "epoch",
        "revision",
        "payload",
    ] {
        let mut changed = command.clone();
        match field {
            "boot" => changed.open.channel.worker_boot_id = "other".into(),
            "pending" => changed.broker_limits.max_pending += 1,
            "size" => changed.broker_limits.max_reply_bytes += 1,
            "host" => changed.open.descriptor.scope.host_id = "other".into(),
            "workspace" => changed.open.descriptor.scope.workspace_id = "other".into(),
            "node" => changed.open.descriptor.scope.controller_node_id = "other".into(),
            "epoch" => changed.open.descriptor.scope.enrollment_epoch += 1,
            "revision" => changed.open.descriptor.scope.credential_revision += 1,
            _ => changed.open.definition.instructions = Some("changed".into()),
        }
        if field != "payload" {
            changed.open.identity.fingerprint =
                wire::broker::broker_session_open_fingerprint(&changed).unwrap();
        }
        // The control codec rejects channel/offer/payload tampering; the
        // transport additionally checks authenticated host scope at send time.
        let control = Control::Broker(Box::new(changed));
        if matches!(field, "host" | "workspace" | "node" | "epoch" | "revision") {
            assert!(transport.check(&control).is_err());
        } else {
            assert!(
                control
                    .validate_for_channel(transport.fence(), Some(offer))
                    .is_err(),
                "{field}"
            );
        }
        if let Control::Broker(command) = control {
            assert!(transport.open_broker_session(*command).await.is_err());
        }
        assert!(broker_requests.try_recv().is_err());
        assert_eq!(transport.slot.permits.available_permits(), PENDING);
    }
}
