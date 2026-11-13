//! Controlled registration/correlation tests, not private-TLS or process proof.
use super::*;
use crate::worker_channel::tests::{Fixture, fixture};
use serde_json::json;

fn pending_request(message: Request) -> ExecutionRequest {
    let (response, _receiver) = oneshot::channel();
    ExecutionRequest {
        message,
        deadline: tokio::time::Instant::now() + Duration::from_secs(5),
        response,
        _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
    }
}
fn terminal_record() -> worker::ExecutionRecord {
    serde_json::from_value(json!({
        "receipt":{"identity":wire_identity("a"),"lease_duration_ms":1000,
            "fence":{"channel":{"channel_id":"channel","controller_boot_id":"controller","worker_boot_id":"worker"},"attempt_id":"attempt","execution_epoch":1}},
        "status":"completed","outcome":{"outcome_id":"outcome","digest":"b".repeat(64),"status":"completed","text":"done"}
    })).unwrap()
}

#[test]
fn asynchronous_completion_preserves_renew_cancel_and_reconcile_waiters() {
    let record = terminal_record();
    record.validate().unwrap();
    let fence = record.receipt.fence.clone();
    let cases = [
        (
            Request::Renew {
                fence: fence.clone(),
                sequence: 2,
                lease_duration_ms: 1000,
            },
            Reply::Renewed {
                fence: fence.clone(),
                sequence: 2,
                lease_duration_ms: 1000,
            },
        ),
        (
            Request::Cancel {
                fence: fence.clone(),
            },
            Reply::CancelAck {
                fence: fence.clone(),
                disposition: worker::ExecutionCancelDisposition::AlreadyTerminal,
            },
        ),
        (
            Request::Reconcile {
                channel: fence.channel.clone(),
                identity: wire_identity("a"),
            },
            Reply::Reconciled {
                channel: fence.channel.clone(),
                identity: wire_identity("a"),
                record: Some(record.clone()),
            },
        ),
    ];
    for (request, response) in cases {
        let mut pending = BTreeMap::new();
        pending.insert(request_key(&request), pending_request(request.clone()));
        // Repeated terminal outbox receipts and standalone outcomes are events,
        // including duplicates while an unrelated RPC remains pending.
        for event in [
            Reply::Accepted {
                record: record.clone(),
            },
            Reply::Accepted {
                record: record.clone(),
            },
            Reply::Outcome {
                fence: fence.clone(),
                outcome: record.outcome.clone().unwrap(),
            },
        ] {
            assert!(take_pending_reply(&mut pending, &event).unwrap().is_none());
            assert_eq!(pending.len(), 1);
            assert_eq!(pending.values().next().unwrap().message, request);
        }
        let selected = take_pending_reply(&mut pending, &response)
            .unwrap()
            .unwrap();
        assert_eq!(selected.message, request);
        assert!(pending.is_empty());
    }
}

#[test]
fn pending_execute_still_rejects_malformed_acceptance_without_consuming_waiter() {
    let record = terminal_record();
    let command = serde_json::from_value(json!({
        "channel":record.receipt.fence.channel,"identity":wire_identity("a"),
        "descriptor":{"scope":{"controller_node_id":"controller","worker_node_id":"worker","workspace_id":"workspace","host_id":"host","enrollment_epoch":1,"credential_revision":1},
            "export":{"id":"export","name":"Export","repo_kind":"directory","checkout_identity_digest":"c".repeat(64),"settings_digest":"d".repeat(64),"revision":1,"active":true},
            "grant":{"principal":{"kind":"human","id":"human"},"permissions":["read","execute"],"revision":1,"export_revision":1,"active":true}},
        "profile":{"agent_id":"agent","version":1,"digest":"e".repeat(64)},
        "definition":{"schema_version":1,"name":"Agent","description":"","harness":"opencode","model":"fixture-model","mode":"plan","instructions":null,"skills":[],"max_seconds":10,"max_output_bytes":1024},
        "input":"hello","budgets":{"max_seconds":10,"max_output_bytes":1024,"lease_duration_ms":1000}
    })).unwrap();
    let request = Request::Execute {
        command: Box::new(command),
    };
    let mut pending = BTreeMap::new();
    pending.insert(request_key(&request), pending_request(request.clone()));
    let mut malformed = record.clone();
    malformed.receipt.identity.fingerprint = "f".repeat(64);
    assert!(take_pending_reply(&mut pending, &Reply::Accepted { record: malformed }).is_err());
    assert_eq!(pending.len(), 1);
    let mut malformed = record.clone();
    malformed.receipt.lease_duration_ms = 2000;
    assert!(take_pending_reply(&mut pending, &Reply::Accepted { record: malformed }).is_err());
    assert_eq!(pending.len(), 1);
    // Outcome cannot acknowledge Execute; only an exact acceptance can.
    assert!(
        take_pending_reply(
            &mut pending,
            &Reply::Outcome {
                fence: record.receipt.fence.clone(),
                outcome: record.outcome.clone().unwrap()
            }
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(pending.len(), 1);
    assert_eq!(
        take_pending_reply(&mut pending, &Reply::Accepted { record })
            .unwrap()
            .unwrap()
            .message,
        request
    );
    assert!(pending.is_empty());
}
fn enabled() -> Fixture {
    let mut f = fixture(true, true);
    Arc::get_mut(&mut f.state.config)
        .unwrap()
        .worker_controller
        .as_mut()
        .unwrap()
        .execution_enabled = true;
    f
}
fn identity_lease(f: &Fixture, channel: &str) -> (WorkerIdentity, ChannelLease) {
    let store = f.state.store.lock().unwrap();
    let identity = store
        .authenticate_worker_token(&f.credential.credential)
        .unwrap();
    let lease = store
        .open_worker_channel(
            &identity,
            &f.state.worker_channels.boot_id,
            "worker-boot",
            channel,
            now_ms().unwrap(),
            15000,
        )
        .unwrap();
    (identity, lease)
}
#[test]
fn opaque_v3_attestation_requires_fresh_transaction_and_dies_on_slot_or_producer_replacement() {
    let f = enabled();
    let (sender, _events) = mpsc::channel(8);
    let producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let (identity, lease) = identity_lease(&f, "channel-1");
    let (_registration, transport, _requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let store = f.state.store.lock().unwrap();
    assert!(
        transport
            .attest_in_transaction(&store, &identity, &lease, transport.fence())
            .is_err()
    );
    store
        .remote_binding_transaction(false, |store| {
            transport.attest_in_transaction(store, &identity, &lease, transport.fence())
        })
        .unwrap();
    drop(store);
    let (next_identity, next_lease) = identity_lease(&f, "channel-2");
    let (_next_registration, next, _next_requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &next_identity, &next_lease, "n-controller".into())
        .unwrap();
    let store = f.state.store.lock().unwrap();
    assert!(
        store
            .remote_binding_transaction(false, |store| transport.attest_in_transaction(
                store,
                &identity,
                &lease,
                transport.fence()
            ))
            .is_err()
    );
    store
        .remote_binding_transaction(false, |store| {
            next.attest_in_transaction(store, &next_identity, &next_lease, next.fence())
        })
        .unwrap();
    drop(producer);
    assert!(
        store
            .remote_binding_transaction(false, |store| next.attest_in_transaction(
                store,
                &next_identity,
                &next_lease,
                next.fence()
            ))
            .is_err()
    );
    assert!(
        f.state
            .worker_channels
            .execution_transport(next.host_id())
            .is_err()
    );
}
#[test]
fn metadata_or_unregistered_producer_never_manufactures_execution_transport() {
    let f = fixture(true, true);
    let (identity, lease) = identity_lease(&f, "metadata-channel");
    assert!(
        f.state
            .worker_channels
            .execution_transport(identity.host_id())
            .is_err()
    );
    assert!(
        f.state
            .worker_channels
            .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
            .is_err()
    );
    let (sender, _events) = mpsc::channel(8);
    let _producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    assert!(
        f.state
            .worker_channels
            .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
            .is_err()
    );
}
#[test]
fn reply_correlation_binds_renew_sequence_duration_and_query_identity_not_only_attempt_id() {
    let fence=serde_json::from_value(json!({"channel":{"channel_id":"channel","controller_boot_id":"controller","worker_boot_id":"worker"},"attempt_id":"attempt","execution_epoch":1})).unwrap();
    let request = Request::Renew {
        fence,
        sequence: 2,
        lease_duration_ms: 1000,
    };
    let Request::Renew { fence, .. } = &request else {
        unreachable!()
    };
    assert!(matches_reply(
        &request,
        &Reply::Renewed {
            fence: fence.clone(),
            sequence: 2,
            lease_duration_ms: 1000
        }
    ));
    assert!(!matches_reply(
        &request,
        &Reply::Renewed {
            fence: fence.clone(),
            sequence: 1,
            lease_duration_ms: 1000
        }
    ));
    assert!(!matches_reply(
        &request,
        &Reply::Renewed {
            fence: fence.clone(),
            sequence: 2,
            lease_duration_ms: 2000
        }
    ));
    let identity = wire_identity("a");
    let query = Request::Reconcile {
        channel: fence.channel.clone(),
        identity: identity.clone(),
    };
    assert!(matches_reply(
        &query,
        &Reply::Reconciled {
            channel: fence.channel.clone(),
            identity,
            record: None
        }
    ));
    assert!(!matches_reply(
        &query,
        &Reply::Reconciled {
            channel: fence.channel.clone(),
            identity: wire_identity("b"),
            record: None
        }
    ));
}
fn wire_identity(fingerprint: &str) -> worker::ExecutionIdentity {
    worker::ExecutionIdentity {
        command_id: "command".into(),
        attempt_id: "attempt".into(),
        fingerprint: fingerprint.repeat(64),
    }
}

#[tokio::test]
async fn execution_request_queue_and_permits_remain_bounded_while_the_socket_consumer_is_stalled() {
    let f = enabled();
    let (sender, _events) = mpsc::channel(8);
    let _producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let (identity, lease) = identity_lease(&f, "bounded-channel");
    let (_registration, transport, receiver) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let mut waiting = Vec::new();
    for i in 0..PENDING {
        let transport = transport.clone();
        waiting.push(tokio::spawn(async move {
            let mut identity = wire_identity("a");
            identity.attempt_id = format!("attempt-{i}");
            transport
                .request(Request::Reconcile {
                    channel: transport.fence().clone(),
                    identity,
                })
                .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while transport.slot.permits.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let result = transport
        .request(Request::Reconcile {
            channel: transport.fence().clone(),
            identity: wire_identity("b"),
        })
        .await;
    assert_eq!(result.unwrap_err(), ExecutionTransportError::Busy);
    for task in waiting {
        task.abort();
        let _ = task.await;
    }
    // Cancelling a waiter must not release the bound while its actual queued
    // command still exists. Receiver teardown drops those commands/permits.
    assert_eq!(transport.slot.permits.available_permits(), 0);
    drop(receiver);
    assert_eq!(transport.slot.permits.available_permits(), PENDING);
    assert!(
        transport
            .check(&Request::Reconcile {
                channel: transport.fence().clone(),
                identity: wire_identity("a")
            })
            .is_err()
    );
}

#[test]
fn legal_one_second_lease_describe_and_renew_traffic_fits_v3_while_legacy_rate_stays_closed() {
    let mut rate = Rate::new();
    // Three renewals/second, each with fresh Describe, plus 30 heartbeats/minute.
    for _ in 0..390 {
        rate.frame_for_protocol(WORKER_EXECUTION_PROTOCOL_VERSION)
            .unwrap();
    }
    for _ in 390..worker::MAX_EXECUTION_FRAMES_PER_MINUTE {
        rate.frame_for_protocol(WORKER_EXECUTION_PROTOCOL_VERSION)
            .unwrap();
    }
    assert_eq!(
        rate.frame_for_protocol(WORKER_EXECUTION_PROTOCOL_VERSION)
            .unwrap_err(),
        WorkerErrorCode::RateLimited
    );
    let mut legacy = Rate::new();
    for _ in 0..120 {
        legacy.frame().unwrap();
    }
    assert_eq!(legacy.frame().unwrap_err(), WorkerErrorCode::RateLimited);
}

#[tokio::test]
async fn execution_describe_uses_only_current_opaque_v3_slot_and_rechecks_replacement_after_reply()
{
    let f = enabled();
    let (send, _events) = mpsc::channel(8);
    let producer = f
        .state
        .worker_channels
        .register_execution_producer(send)
        .unwrap();
    let (identity, lease) = identity_lease(&f, "describe-v3");
    let (_metadata_registration, mut metadata) = f
        .state
        .worker_channels
        .register_exports(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let (_execution_registration, transport, _requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let principal = worker::ExportPrincipal {
        kind: worker::PrincipalKind::Human,
        id: "reader".into(),
    };
    let descriptor:worker::ExportDescriptor=serde_json::from_value(json!({"scope":{"controller_node_id":"n-controller","worker_node_id":"n-worker",
        "workspace_id":identity.workspace_id(),"host_id":identity.host_id(),"enrollment_epoch":1,"credential_revision":1},
        "export":{"id":"export-1","name":"Export","repo_kind":"directory","checkout_identity_digest":"a".repeat(64),"settings_digest":"b".repeat(64),"revision":1,"active":true},
        "grant":{"principal":principal,"permissions":["read"],"revision":1,"export_revision":1,"active":true}})).unwrap();
    let query = f.state.worker_channels.describe_execution_export(
        &transport,
        "export-1",
        principal.clone(),
        Some(1),
    );
    let response = async {
        let request = metadata.recv().await.unwrap();
        assert_eq!(request.principal, principal);
        assert_eq!(request.export_id, "export-1");
        let _ = request.response.send(Ok(descriptor.clone()));
    };
    let (observed, _) = tokio::join!(query, response);
    assert_eq!(observed.unwrap().descriptor(), &descriptor);
    let query = f.state.worker_channels.describe_execution_export(
        &transport,
        "export-1",
        principal,
        Some(1),
    );
    let response = async {
        let request = metadata.recv().await.unwrap();
        drop(producer);
        let _ = request.response.send(Ok(descriptor));
    };
    let (observed, _) = tokio::join!(query, response);
    assert!(observed.is_err());
    assert!(
        f.state
            .worker_channels
            .describe_execution_export(
                &transport,
                "export-1",
                worker::ExportPrincipal {
                    kind: worker::PrincipalKind::Human,
                    id: "reader".into()
                },
                Some(1)
            )
            .await
            .is_err()
    );
}

#[test]
fn only_exact_cached_worker_ack_can_attest_and_cache_never_survives_slot_replacement() {
    let f = enabled();
    let (sender, _events) = mpsc::channel(8);
    let producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let (identity, lease) = identity_lease(&f, "ack-v3");
    let (_registration, transport, _requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    let fence = worker::AttemptFence {
        channel: transport.fence().clone(),
        attempt_id: "attempt".into(),
        execution_epoch: 1,
    };
    let message = Request::OutcomeAck {
        channel: transport.fence().clone(),
        identity: wire_identity("a"),
        fence: fence.clone(),
        outcome_id: "outcome".into(),
        outcome_digest: "b".repeat(64),
    };
    let receipt = Reply::OutcomeAcknowledged {
        channel: transport.fence().clone(),
        identity: wire_identity("a"),
        fence,
        outcome_id: "outcome".into(),
        outcome_digest: "b".repeat(64),
    };
    let attest = |transport: &ExecutionTransport, message: &Request| {
        f.state
            .store
            .lock()
            .unwrap()
            .remote_binding_transaction(false, |store| {
                transport.attest_outcome_ack_in_transaction(store, message)
            })
    };
    // Merely serializing/sending the request cannot manufacture wire evidence.
    assert!(attest(&transport, &message).is_err());
    let mut wrong = receipt.clone();
    if let Reply::OutcomeAcknowledged { outcome_digest, .. } = &mut wrong {
        *outcome_digest = "c".repeat(64);
    }
    assert!(transport.remember_ack(&message, &wrong).is_err());
    transport.remember_ack(&message, &receipt).unwrap();
    assert!(attest(&transport, &message).is_ok());
    let mut wrong = message.clone();
    if let Request::OutcomeAck { outcome_digest, .. } = &mut wrong {
        *outcome_digest = "d".repeat(64);
    }
    assert!(attest(&transport, &wrong).is_err());
    let (identity, next_lease) = identity_lease(&f, "ack-new-slot");
    let (_next_registration, next, _next_requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &next_lease, "n-controller".into())
        .unwrap();
    assert!(attest(&transport, &message).is_err());
    assert!(attest(&next, &message).is_err());
    drop(producer);
    assert!(attest(&next, &message).is_err());
}

#[test]
fn concurrent_same_attempt_renew_and_ack_rejection_resolve_separate_correlator_entries() {
    let channel = worker::ChannelFence {
        channel_id: "channel".into(),
        controller_boot_id: "controller".into(),
        worker_boot_id: "worker".into(),
    };
    let fence = worker::AttemptFence {
        channel: channel.clone(),
        attempt_id: "attempt".into(),
        execution_epoch: 1,
    };
    let renew = Request::Renew {
        fence: fence.clone(),
        sequence: 2,
        lease_duration_ms: 1000,
    };
    let ack = Request::OutcomeAck {
        channel: channel.clone(),
        identity: wire_identity("a"),
        fence: fence.clone(),
        outcome_id: "outcome".into(),
        outcome_digest: "b".repeat(64),
    };
    let rejected = Reply::OutcomeAckRejected {
        channel,
        identity: wire_identity("a"),
        fence,
        outcome_id: "outcome".into(),
        outcome_digest: "b".repeat(64),
        code: worker::ExecutionErrorCode::Unavailable,
    };
    let mut pending = BTreeMap::new();
    pending.insert(request_key(&renew), renew.clone());
    pending.insert(request_key(&ack), ack.clone());
    assert_ne!(request_key(&renew), request_key(&ack));
    let selected = pending.remove(&reply_key(&rejected)).unwrap();
    assert_eq!(selected, ack);
    assert!(matches_reply(&selected, &rejected));
    assert!(!matches_reply(&renew, &rejected));
    assert_eq!(pending.get(&request_key(&renew)), Some(&renew));
    let f = enabled();
    let (sender, _events) = mpsc::channel(8);
    let _producer = f
        .state
        .worker_channels
        .register_execution_producer(sender)
        .unwrap();
    let (identity, lease) = identity_lease(&f, "no-reject-proof");
    let (_registration, transport, _requests) = f
        .state
        .worker_channels
        .register_execution_transport(&f.state, &identity, &lease, "n-controller".into())
        .unwrap();
    assert!(transport.remember_ack(&selected, &rejected).is_err());
    assert!(transport.slot.acknowledged.lock().unwrap().is_empty());
}
