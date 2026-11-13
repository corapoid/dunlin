//! SQL lifecycle with the original live MCP scope, not reconstructed authority.
use super::*;
use crate::store::remote_bindings::resolver::sessions::{
    authority::RemoteMcpScope, runtime::RemoteSessionRuntime,
};

fn session(open: &RemoteOpen) -> wire::SessionReceipt {
    wire::SessionReceipt {
        identity: open.identity.clone(),
        fence: wire::SessionFence {
            channel: open.channel.clone(),
            controller_session_id: open.controller_session_id.clone(),
            worker_session_id: "native-runtime-fixture".into(),
            session_epoch: 1,
        },
        budgets: open.budgets.clone(),
    }
}
fn tx<T>(
    fixture: &McpFixture,
    store: &Store,
    scope: &RemoteMcpScope,
    operation: impl FnOnce(&RemoteSessionRuntime<'_>) -> Result<T>,
) -> Result<T> {
    store.remote_binding_transaction(false, |store| {
        operation(&scope.runtime(store, &fixture.state.config))
    })
}
fn active(
    fixture: &McpFixture,
    store: &Store,
    scope: &RemoteMcpScope,
    id: &str,
) -> wire::SessionReceipt {
    let open = tx(fixture, store, scope, |runtime| {
        runtime.dispatch_remote_session_open_in_transaction(id, fixture.data.proof(), now(), source)
    })
    .unwrap();
    assert!(matches!(&open, RemoteOpen::Broker(_)));
    let receipt = session(&open);
    tx(fixture, store, scope, |runtime| {
        runtime.accept_remote_session_in_transaction(id, &receipt, true, source)
    })
    .unwrap();
    receipt
}

#[test]
fn scoped_runtime_runs_native_lifecycle_and_projects_exact_ordered_events() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (id, _, scope) = super::authority::setup(&fixture, &store);
    let receipt = active(&fixture, &store, &scope, &id);
    assert!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .dispatch_remote_session_open_in_transaction(
                &id,
                fixture.data.proof(),
                now(),
                source
            ))
        .is_err()
    );
    tx(&fixture, &store, &scope, |runtime| {
        runtime.remote_session_queue_allowed_in_transaction(&id, &fixture.data.owner, source)
    })
    .unwrap();
    let context = store
        .team_project_context(
            &fixture.data.workspace,
            &fixture.data.project,
            &fixture.data.owner,
        )
        .unwrap();
    store
        .interactive_journal()
        .enqueue(&context, &id, "runtime-prompt", "look up")
        .unwrap();
    let command = tx(&fixture, &store, &scope, |runtime| {
        runtime.claim_remote_session_turn_in_transaction(&id, fixture.data.proof(), now(), source)
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .dispatch_remote_session_turn_in_transaction(
                &id,
                &command.identity.turn_id,
                fixture.data.proof(),
                now(),
                source
            ))
        .unwrap(),
        command
    );
    let accepted = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: receipt.fence.clone(),
            turn_id: command.identity.turn_id.clone(),
            turn_index: command.turn_index,
            turn_epoch: 1,
        },
    };
    tx(&fixture, &store, &scope, |runtime| {
        runtime.accept_remote_session_turn_in_transaction(&id, &accepted, source)
    })
    .unwrap();
    assert_eq!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .resolve_remote_turn_receipt_in_transaction(
                &id,
                &command.identity,
                source
            ))
        .unwrap(),
        Some(accepted.clone())
    );
    assert!(matches!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .renew_remote_session_in_transaction(
                &id,
                fixture.data.proof(),
                now(),
                source
            ))
        .unwrap(),
        wire::SessionControllerMessage::RenewSession { sequence: 1, .. }
    ));
    assert!(matches!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .renew_remote_session_turn_in_transaction(
                &id,
                &command.identity.turn_id,
                fixture.data.proof(),
                now(),
                source
            ))
        .unwrap(),
        Some(wire::SessionControllerMessage::RenewTurn { sequence: 1, .. })
    ));
    let text = event(
        &receipt,
        1,
        None,
        wire::SessionEventKind::Text {
            turn: accepted.fence.clone(),
            kind: wire::TextKind::Text,
            text: "scoped result".into(),
        },
    );
    let complete = event(
        &receipt,
        2,
        Some(&text),
        wire::SessionEventKind::TurnCompleted {
            receipt: accepted,
            status: wire::TurnTerminalStatus::Completed,
        },
    );
    for event in [&text, &complete] {
        assert!(matches!(
            tx(&fixture, &store, &scope, |runtime| runtime
                .record_remote_session_event_in_transaction(
                    &id, event, source, caller
                ))
            .unwrap(),
            RemoteStreamCut::Persisted {
                duplicate: false,
                ..
            }
        ));
    }
    assert!(matches!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .record_remote_session_event_in_transaction(
                &id, &complete, source, caller
            ))
        .unwrap(),
        RemoteStreamCut::Persisted {
            duplicate: true,
            ..
        }
    ));
    assert!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .claim_remote_session_turn_in_transaction(
                &id,
                fixture.data.proof(),
                now(),
                source
            ))
        .unwrap()
        .is_none()
    );
    assert_eq!(store.remote_session_snapshot(&id).unwrap().sequence, 2);
}

#[test]
fn scoped_runtime_requires_original_proof_current_account_and_fresh_source_observation() {
    for changed in [
        "legacy",
        "other_session",
        "caller",
        "source",
        "account",
        "config",
    ] {
        let fixture = McpFixture::new(Store::open_in_memory().unwrap());
        let store = fixture.state.store.lock().unwrap();
        let (id, _, scope) = super::authority::setup(&fixture, &store);
        let mut config = (*fixture.state.config).clone();
        if changed == "account" {
            store.disable(&fixture.context).unwrap();
        }
        if changed == "config" {
            config.mcp_oauth[0].client_id = "replaced".into();
        }
        let result = store.remote_binding_transaction(false, |store| {
            let mut descriptor = fixture.data.descriptor.clone();
            if changed == "source" {
                descriptor.grant.revision += 1;
            }
            let proof = ObservedExport::for_test(
                fixture.data.identity.clone(),
                fixture.data.channel.clone(),
                descriptor,
                now(),
            );
            if changed == "legacy" {
                return store
                    .remote_session_runtime()
                    .dispatch_remote_session_open_in_transaction(&id, proof, now(), source);
            }
            scope
                .runtime(store, &config)
                .dispatch_remote_session_open_in_transaction(
                    if changed == "other_session" {
                        "other-session"
                    } else {
                        &id
                    },
                    proof,
                    now(),
                    |store, snapshot| {
                        if changed == "caller" {
                            Err(denied())
                        } else {
                            source(store, snapshot)
                        }
                    },
                )
        });
        assert!(result.is_err(), "{changed}");
        let snapshot = store.remote_session_snapshot(&id).unwrap();
        assert_eq!(snapshot.phase, "pending", "{changed}");
        assert!(!snapshot.dispatched, "{changed}");
    }
}

#[test]
fn scoped_receipt_failure_rolls_back_sdk_and_revocation_still_allows_negative_stop() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (id, _, scope) = super::authority::setup(&fixture, &store);
    let open = tx(&fixture, &store, &scope, |runtime| {
        runtime.dispatch_remote_session_open_in_transaction(
            &id,
            fixture.data.proof(),
            now(),
            source,
        )
    })
    .unwrap();
    let receipt = session(&open);
    let before: String = store
        .connection
        .query_row(
            "SELECT phase FROM interactive_sessions WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    store.connection.execute_batch("CREATE TEMP TRIGGER reject_mcp_receipt BEFORE UPDATE OF receipt ON controller_remote_sessions BEGIN SELECT RAISE(ABORT,'controlled receipt failure'); END;").unwrap();
    assert!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .accept_remote_session_in_transaction(
                &id, &receipt, true, source
            ))
        .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row::<String, _, _>(
                "SELECT phase FROM interactive_sessions WHERE id=?1",
                [&id],
                |r| r.get(0)
            )
            .unwrap(),
        before
    );
    assert!(
        store
            .remote_session_snapshot(&id)
            .unwrap()
            .receipt
            .is_none()
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_mcp_receipt;")
        .unwrap();
    store.disable(&fixture.context).unwrap();
    assert!(
        tx(&fixture, &store, &scope, |runtime| runtime
            .accept_remote_session_in_transaction(
                &id, &receipt, true, source
            ))
        .is_err()
    );
    store
        .remote_binding_transaction(false, |store| {
            store.capture_remote_session_receipt_in_transaction(&id, &receipt, source)?;
            store.interrupt_remote_session_in_transaction(&id, source)?;
            assert!(matches!(
                store.close_remote_session_in_transaction(&id, source)?,
                Some(wire::SessionControllerMessage::CloseSession { .. })
            ));
            Ok::<_, StoreError>(())
        })
        .unwrap();
    let stopped = event(
        &receipt,
        1,
        None,
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    );
    // No live account or positive scope is needed to consume a matching stop.
    store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &stopped, source, |_, _, _| {
                Err(denied())
            })
        })
        .unwrap();
    assert_eq!(store.remote_session_snapshot(&id).unwrap().phase, "stopped");
}
