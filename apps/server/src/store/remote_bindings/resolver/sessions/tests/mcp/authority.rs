use super::*;
use crate::store::remote_bindings::resolver::sessions::authority::RemoteMcpScope;
use dunlin_mcp_client::oauth::persistence::PersistenceError;
use std::time::{Duration, Instant};

pub(super) fn setup(fixture: &McpFixture, store: &Store) -> (String, Lease, RemoteMcpScope) {
    let admission = store
        .remote_binding_transaction(false, |store| fixture.admit(store, fixture.prepare(store)?))
        .unwrap();
    let id = admission.admission.session.id;
    let lease = admission.admission.lease.unwrap();
    let scope = store
        .remote_binding_transaction(false, |store| {
            RemoteMcpScope::capture(
                store,
                &fixture.state.config,
                &id,
                &lease,
                fixture.transport.clone(),
                Instant::now() + Duration::from_secs(30),
                caller,
            )
        })
        .unwrap();
    (id, lease, scope)
}

#[test]
fn live_scope_operates_inside_refresh_transaction_without_recreating_admission() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (id, _, scope) = setup(&fixture, &store);
    assert_eq!(scope.pins().session, id);
    scope
        .context(&store, &fixture.state.config, caller)
        .unwrap();
    assert!(
        scope
            .context_in_transaction(&store, &fixture.state.config, caller)
            .is_err()
    );
    let check = |store: &Store, _: &AuthContext| {
        scope
            .context_in_transaction(store, &fixture.state.config, caller)
            .map(|_| ())
            .map_err(|_| PersistenceError::Unauthorized)
    };
    let journal = store.mcp_oauth_journal(&check);
    let active = journal.active(&fixture.context, &"b".repeat(64)).unwrap();
    let rotation = journal
        .begin_refresh(&fixture.context, &"b".repeat(64), &active)
        .unwrap();
    // New admission rejects refreshing; a live authority cut must let the
    // same journal finish its own rotation without nesting another transaction.
    assert!(
        store
            .remote_binding_transaction(false, |store| fixture.prepare(store))
            .is_err()
    );
    scope
        .context(&store, &fixture.state.config, caller)
        .unwrap();
    journal
        .prepare_write(
            &fixture.context,
            &"b".repeat(64),
            &rotation,
            &"e".repeat(64),
        )
        .unwrap();
    journal
        .commit(
            &fixture.context,
            &"b".repeat(64),
            &rotation,
            &"e".repeat(64),
        )
        .unwrap();
    assert_eq!(
        journal
            .active(&fixture.context, &"b".repeat(64))
            .unwrap()
            .generation,
        active.generation + 1
    );
    scope
        .context(&store, &fixture.state.config, caller)
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT count(*) FROM controller_remote_sessions",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn live_scope_rechecks_device_account_configuration_and_actor_callback() {
    for changed in ["device", "account", "config", "actor"] {
        let fixture = McpFixture::new(Store::open_in_memory().unwrap());
        let store = fixture.state.store.lock().unwrap();
        let (_, _, scope) = setup(&fixture, &store);
        let mut config = (*fixture.state.config).clone();
        match changed {
            "device" => store.revoke_device(&fixture.data.device).unwrap(),
            "account" => {
                store.disable(&fixture.context).unwrap();
            }
            "config" => config.mcp_oauth[0].client_id = "changed-client".into(),
            "actor" => {}
            _ => unreachable!(),
        }
        assert!(
            scope
                .context(&store, &config, |store, owner, device| {
                    if changed == "actor" {
                        return Err(denied());
                    }
                    caller(store, owner, device)
                })
                .is_err(),
            "{changed}"
        );
    }
}

#[test]
fn live_scope_rejects_stale_journal_source_and_expired_deadline() {
    for changed in ["journal", "source", "deadline"] {
        let mut fixture = McpFixture::new(Store::open_in_memory().unwrap());
        let store = fixture.state.store.lock().unwrap();
        let (id, mut lease, scope) = setup(&fixture, &store);
        match changed {
            "journal" => {
                store.connection.execute("UPDATE controller_remote_sessions SET journal_epoch=journal_epoch+1 WHERE session=?1", [&id]).unwrap();
            }
            "source" => drop(fixture._producer.take()),
            "deadline" => {
                assert!(
                    store
                        .remote_binding_transaction(false, |store| RemoteMcpScope::capture(
                            store,
                            &fixture.state.config,
                            &id,
                            &lease,
                            fixture.transport.clone(),
                            Instant::now(),
                            caller
                        ))
                        .is_err()
                );
                lease.epoch += 1;
                assert!(
                    store
                        .remote_binding_transaction(false, |store| RemoteMcpScope::capture(
                            store,
                            &fixture.state.config,
                            &id,
                            &lease,
                            fixture.transport.clone(),
                            Instant::now() + Duration::from_secs(10),
                            caller
                        ))
                        .is_err()
                );
                continue;
            }
            _ => unreachable!(),
        }
        assert!(
            scope
                .context(&store, &fixture.state.config, caller)
                .is_err(),
            "{changed}"
        );
    }
}

#[test]
fn fresh_source_observation_must_match_the_frozen_export_and_grant() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (_, _, scope) = setup(&fixture, &store);
    store
        .remote_binding_transaction(false, |store| {
            scope.observe_in_transaction(
                store,
                &fixture.state.config,
                fixture.data.proof(),
                now(),
                caller,
            )
        })
        .unwrap();
    let mut descriptor = fixture.data.descriptor.clone();
    descriptor.grant.permissions = vec![dunlin_relay_protocol::worker::ExportPermission::Read];
    let changed = ObservedExport::for_test(
        fixture.data.identity.clone(),
        fixture.data.channel.clone(),
        descriptor,
        now(),
    );
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.observe_in_transaction(
                store,
                &fixture.state.config,
                changed,
                now(),
                caller
            ))
            .is_err()
    );
}

#[test]
fn transport_request_requires_actual_native_receipt_and_running_turn() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (id, _, scope) = setup(&fixture, &store);
    let snapshot = store.remote_session_snapshot(&id).unwrap();
    let receipt = wire::SessionReceipt {
        identity: snapshot.open.identity.clone(),
        fence: wire::SessionFence {
            channel: fixture.transport.fence().clone(),
            controller_session_id: id.clone(),
            worker_session_id: "native-session".into(),
            session_epoch: 1,
        },
        budgets: snapshot.open.budgets.clone(),
    };
    let request = wire::broker::BrokerScope::Catalog {
        session: receipt.fence.clone(),
    };
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.request_in_transaction(
                store,
                &fixture.state.config,
                &request,
                caller
            ))
            .is_err()
    );
    // Controlled SQL fixture representing a received native acceptance; this
    // is not a public API capable of manufacturing a source receipt.
    store.connection.execute("UPDATE controller_remote_sessions SET phase='opening',dispatched=1,receipt=?2 WHERE session=?1", params![id,serde_json::to_string(&receipt).unwrap()]).unwrap();
    store
        .remote_binding_transaction(false, |store| {
            scope.request_in_transaction(store, &fixture.state.config, &request, caller)
        })
        .unwrap();
    let turn = wire::broker::BrokerScope::Turn {
        turn: wire::TurnFence {
            session: receipt.fence.clone(),
            turn_id: "no-accepted-turn".into(),
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.request_in_transaction(
                store,
                &fixture.state.config,
                &turn,
                caller
            ))
            .is_err()
    );
    let mut foreign = receipt.fence;
    foreign.channel.worker_boot_id = "foreign-boot".into();
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.request_in_transaction(
                store,
                &fixture.state.config,
                &wire::broker::BrokerScope::Catalog { session: foreign },
                caller
            ))
            .is_err()
    );
}

#[test]
fn running_accepted_turn_is_correlated_and_terminal_sdk_turn_is_fenced() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let (id, lease, scope) = setup(&fixture, &store);
    let snapshot = store.remote_session_snapshot(&id).unwrap();
    let receipt = wire::SessionReceipt {
        identity: snapshot.open.identity.clone(),
        fence: wire::SessionFence {
            channel: fixture.transport.fence().clone(),
            controller_session_id: id.clone(),
            worker_session_id: "native-session".into(),
            session_epoch: 1,
        },
        budgets: snapshot.open.budgets.clone(),
    };
    store.remote_binding_transaction(false, |store| {
        let context = scope.context_in_transaction(store, &fixture.state.config, caller)?;
        store.interactive_journal().harness_ready_in_transaction(&context, &id, &lease, dunlin_session_runtime::HarnessInfo {
            profile: Some(snapshot.open.profile.clone()), remote_session_id: receipt.fence.worker_session_id.clone(),
            binding_revision: fixture.data.request.binding.revision, model: snapshot.open.definition.model.clone(), mode: snapshot.open.definition.mode.clone(),
            max_seconds: snapshot.open.budgets.max_seconds, max_output_bytes: snapshot.open.budgets.max_output_bytes,
        }).map_err(journal_error)?;
        store.connection.execute("UPDATE controller_remote_sessions SET phase='active',dispatched=1,receipt=?2 WHERE session=?1",params![id,serde_json::to_string(&receipt)?])?;
        Ok::<(), StoreError>(())
    }).unwrap();
    let context = scope
        .context(&store, &fixture.state.config, caller)
        .unwrap();
    store
        .interactive_journal()
        .enqueue(&context, &id, "accepted-turn", "prompt")
        .unwrap();
    let accepted = store.remote_binding_transaction(false, |store| {
        let turn = store.interactive_journal().claim_next_in_transaction(&context, &id, &lease).map_err(journal_error)?.unwrap();
        let mut command = wire::TurnCommand {
            session: receipt.fence.clone(), identity: wire::TurnIdentity { command_id: "command".into(), turn_id: turn.id.clone(), fingerprint: "0".repeat(64) },
            turn_index: 1, input: turn.prompt, budgets: dunlin_relay_protocol::worker::ExecutionBudgets { max_seconds: 10, max_output_bytes: 1024, lease_duration_ms: 10000 },
        };
        command.identity.fingerprint = valid(wire::session_turn_fingerprint(&command))?;
        let accepted = wire::TurnReceipt { identity: command.identity.clone(), fence: wire::TurnFence { session: receipt.fence.clone(), turn_id: turn.id.clone(), turn_index: 1, turn_epoch: 1 }, budgets: command.budgets.clone() };
        valid(accepted.validate_for_command(&command))?;
        store.interactive_journal().transition_timed_in_transaction(&context, &id, &lease, &turn.id, dunlin_session_runtime::TurnPhase::Running, None).map_err(journal_error)?;
        store.connection.execute("INSERT INTO controller_remote_session_turns(session,turn,command_id,turn_index,command,receipt,phase) VALUES(?1,?2,'command',1,?3,?4,'accepted')", params![id,turn.id,serde_json::to_string(&command)?,serde_json::to_string(&accepted)?])?;
        Ok::<wire::TurnReceipt, StoreError>(accepted)
    }).unwrap();
    let request = wire::broker::BrokerScope::Turn {
        turn: accepted.fence.clone(),
    };
    store
        .remote_binding_transaction(false, |store| {
            scope.request_in_transaction(store, &fixture.state.config, &request, caller)
        })
        .unwrap();
    let mut foreign = accepted.fence.clone();
    foreign.turn_epoch += 1;
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.request_in_transaction(
                store,
                &fixture.state.config,
                &wire::broker::BrokerScope::Turn { turn: foreign },
                caller
            ))
            .is_err()
    );
    store
        .interactive_journal()
        .transition_timed(
            &context,
            &id,
            &lease,
            &accepted.identity.turn_id,
            dunlin_session_runtime::TurnPhase::Completed,
            None,
        )
        .unwrap();
    assert!(
        store
            .remote_binding_transaction(false, |store| scope.request_in_transaction(
                store,
                &fixture.state.config,
                &request,
                caller
            ))
            .is_err()
    );
}
