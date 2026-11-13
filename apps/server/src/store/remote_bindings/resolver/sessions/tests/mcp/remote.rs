use super::*;
use crate::interactive::mcp::authority::Authority;
use crate::remote_sessions::{Service, mcp::FixtureActor};

#[tokio::test]
async fn blocking_remote_guard_collects_fresh_describe_and_rejects_source_change() {
    let (fixture, service, actor, _, _) = live_fixture().await;
    let descriptor = Arc::new(std::sync::Mutex::new(fixture.data.descriptor.clone()));
    let (_registration, calls) = fixture
        .state
        .worker_channels
        .register_export_fixture(
            &fixture.state,
            &fixture.data.identity,
            &fixture.data.channel,
            descriptor.clone(),
        )
        .unwrap();
    for expected in [1, 2] {
        let authority = actor.authority.clone();
        let state = fixture.state.clone();
        tokio::task::spawn_blocking(move || {
            authority.check(
                &state,
                tokio::time::Instant::now() + std::time::Duration::from_secs(3),
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), expected);
    }
    descriptor.lock().unwrap().grant.permissions =
        vec![dunlin_relay_protocol::worker::ExportPermission::Read];
    let authority = actor.authority.clone();
    let state = fixture.state.clone();
    assert!(
        tokio::task::spawn_blocking(move || authority.check(
            &state,
            tokio::time::Instant::now() + std::time::Duration::from_secs(3)
        ))
        .await
        .unwrap()
        .is_err()
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 3);
    drop(actor);
    service.shutdown().await;
}

async fn live_fixture() -> (McpFixture, Arc<Service>, FixtureActor, String, Lease) {
    live_fixture_at(None).await
}
async fn live_fixture_at(
    db: Option<std::path::PathBuf>,
) -> (McpFixture, Arc<Service>, FixtureActor, String, Lease) {
    let mut fixture = McpFixture::new(Store::open_in_memory().unwrap());
    if let Some(db) = db {
        Arc::make_mut(&mut fixture.state.config).db = db;
    }
    live_from(fixture).await
}
pub(super) async fn live_from(
    fixture: McpFixture,
) -> (McpFixture, Arc<Service>, FixtureActor, String, Lease) {
    live_from_limit(fixture, 65536).await
}
pub(super) async fn live_from_limit(
    mut fixture: McpFixture,
    maximum: u32,
) -> (McpFixture, Arc<Service>, FixtureActor, String, Lease) {
    // Replace the synthetic producer with the actual service before admission.
    drop(fixture._producer.take());
    let service = Service::start(&fixture.state).await.unwrap();
    let (transport, registration) = fixture
        .state
        .worker_channels
        .register_broker_fixture(
            &fixture.state,
            &fixture.data.identity,
            &fixture.data.channel,
            "controller-node".into(),
            wire::broker::BrokerLimits {
                max_pending: 2,
                max_reply_bytes: maximum,
            },
        )
        .unwrap();
    fixture.transport = transport;
    fixture._registration = registration;
    let admitted = {
        let store = fixture.state.store.lock().unwrap();
        store
            .remote_binding_transaction(false, |store| {
                fixture.admit(store, fixture.prepare(store)?)
            })
            .unwrap()
    };
    let id = admitted.admission.session.id.clone();
    let lease = admitted.admission.lease.clone().unwrap();
    let actor = FixtureActor::attach(
        &fixture.state,
        &service,
        &admitted,
        fixture.transport.clone(),
    )
    .await
    .unwrap();
    (fixture, service, actor, id, lease)
}

#[tokio::test]
async fn remote_policy_is_read_before_store_and_retained_through_sql_cut() {
    let directory = tempfile::tempdir().unwrap();
    let (fixture, service, actor, _, _) =
        live_fixture_at(Some(directory.path().join("relay.db"))).await;
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .connection
        .execute(
            "UPDATE users SET role='member' WHERE id=?1",
            [&fixture.data.owner],
        )
        .unwrap();
    let settings = crate::management::root(&fixture.state.config).join("settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, br#"{"host_accounts":false}"#).unwrap();
    let cut = actor.authority.sql(&fixture.state).unwrap();
    assert!(fixture.state.management.try_lock().is_err());
    // Deliberate external fixture mutation bypasses the held management lock.
    // Existing SQL uses its captured policy; the next cut must fail closed.
    std::fs::write(&settings, b"malformed").unwrap();
    {
        let store = fixture.state.store.lock().unwrap();
        cut.context(&store, false).unwrap();
        let decision = dunlin_session_runtime::PermissionDecision {
            nonce: "checked-by-sdk".into(),
            device_id: fixture.data.device.clone(),
            approved: true,
            option_id: Some("allow_once".into()),
        };
        cut.permission_device(&store, &fixture.data.owner, &decision)
            .unwrap();
        assert!(
            cut.permission_device(&store, "foreign-owner", &decision)
                .is_err()
        );
        let (other_device, _) = store
            .create_device(&fixture.data.owner, "second-approval-device")
            .unwrap();
        let mut other_decision = decision.clone();
        other_decision.device_id = other_device.id.clone();
        cut.permission_device(&store, &fixture.data.owner, &other_decision)
            .unwrap();
        store.revoke_device(&other_device.id).unwrap();
        assert!(
            cut.permission_device(&store, &fixture.data.owner, &other_decision)
                .is_err()
        );
        store
            .remote_binding_transaction(false, |store| {
                cut.context(store, true).map_err(|_| denied())
            })
            .unwrap();
    }
    drop(cut);
    {
        let cut = actor.authority.sql(&fixture.state).unwrap();
        let store = fixture.state.store.lock().unwrap();
        assert!(cut.context(&store, false).is_err());
        let decision = dunlin_session_runtime::PermissionDecision {
            nonce: "checked-by-sdk".into(),
            device_id: fixture.data.device.clone(),
            approved: true,
            option_id: Some("allow_once".into()),
        };
        assert!(
            cut.permission_device(&store, &fixture.data.owner, &decision)
                .is_err()
        );
    }
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn accepted_turn_requires_both_live_clock_and_current_sdk_turn() {
    for expired in [false, true] {
        let (fixture, service, actor, id, lease) = live_fixture().await;
        let (command, accepted) = {
            let cut = actor.authority.sql(&fixture.state).unwrap();
            let store = fixture.state.store.lock().unwrap();
            let context = cut.context(&store, false).unwrap();
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
                store.interactive_journal().harness_ready_in_transaction(&context, &id, &lease, dunlin_session_runtime::HarnessInfo {
                    profile: Some(snapshot.open.profile.clone()), remote_session_id: receipt.fence.worker_session_id.clone(), binding_revision: fixture.data.request.binding.revision,
                    model: snapshot.open.definition.model.clone(), mode: snapshot.open.definition.mode.clone(), max_seconds: snapshot.open.budgets.max_seconds, max_output_bytes: snapshot.open.budgets.max_output_bytes,
                }).map_err(journal_error)?;
                store.connection.execute("UPDATE controller_remote_sessions SET phase='active',dispatched=1,receipt=?2 WHERE session=?1",params![id,serde_json::to_string(&receipt)?])?;
                Ok::<(), StoreError>(())
            }).unwrap();
            store
                .interactive_journal()
                .enqueue(&context, &id, "accepted-turn", "prompt")
                .unwrap();
            store.remote_binding_transaction(false, |store| {
                let turn = store.interactive_journal().claim_next_in_transaction(&context, &id, &lease).map_err(journal_error)?.unwrap();
                let mut command = wire::TurnCommand {
                    session: receipt.fence.clone(), identity: wire::TurnIdentity { command_id: "command".into(), turn_id: turn.id.clone(), fingerprint: "0".repeat(64) }, turn_index: 1, input: turn.prompt,
                    budgets: dunlin_relay_protocol::worker::ExecutionBudgets { max_seconds: 10, max_output_bytes: 1024, lease_duration_ms: 10000 },
                };
                command.identity.fingerprint = valid(wire::session_turn_fingerprint(&command))?;
                let accepted = wire::TurnReceipt { identity: command.identity.clone(), fence: wire::TurnFence { session: receipt.fence, turn_id: turn.id.clone(), turn_index: 1, turn_epoch: 1 }, budgets: command.budgets.clone() };
                valid(accepted.validate_for_command(&command))?;
                store.interactive_journal().transition_timed_in_transaction(&context, &id, &lease, &turn.id, dunlin_session_runtime::TurnPhase::Running, None).map_err(journal_error)?;
                store.connection.execute("INSERT INTO controller_remote_session_turns(session,turn,command_id,turn_index,command,receipt,phase) VALUES(?1,?2,'command',1,?3,?4,'accepted')",params![id,turn.id,serde_json::to_string(&command)?,serde_json::to_string(&accepted)?])?;
                Ok::<_, StoreError>((command, accepted))
            }).unwrap()
        };
        let request = wire::broker::BrokerScope::Turn {
            turn: accepted.fence.clone(),
        };
        // A valid SQL receipt cannot reconstruct the missing live clock.
        assert!(
            actor
                .authority
                .validate_request(request.clone(), &fixture.state)
                .await
                .is_err()
        );
        let started = tokio::time::Instant::now()
            - std::time::Duration::from_secs(if expired { 20 } else { 0 });
        actor.begin_turn(command, started);
        if expired {
            assert!(actor.authority.turn().is_err());
            assert!(
                actor
                    .authority
                    .validate_request(request, &fixture.state)
                    .await
                    .is_err()
            );
        } else {
            let turn = actor.authority.turn().unwrap();
            assert_eq!(turn.id, accepted.identity.turn_id);
            assert_eq!(turn.deadline, started + std::time::Duration::from_secs(10));
            actor
                .authority
                .validate_request(request.clone(), &fixture.state)
                .await
                .unwrap();
            let mut foreign = accepted.fence.clone();
            foreign.turn_epoch += 1;
            assert!(
                actor
                    .authority
                    .validate_request(
                        wire::broker::BrokerScope::Turn { turn: foreign },
                        &fixture.state
                    )
                    .await
                    .is_err()
            );
            {
                let cut = actor.authority.sql(&fixture.state).unwrap();
                let store = fixture.state.store.lock().unwrap();
                let context = cut.context(&store, false).unwrap();
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
            }
            assert!(
                actor
                    .authority
                    .validate_request(request, &fixture.state)
                    .await
                    .is_err()
            );
        }
        drop(actor);
        service.shutdown().await;
    }
}

#[tokio::test]
async fn remote_adapter_owns_policy_cut_through_checked_oauth_rotation() {
    let (fixture, service, actor, id, _) = live_fixture().await;
    let authority = &actor.authority;
    let cut = authority.sql(&fixture.state).unwrap();
    assert!(fixture.state.management.try_lock().is_err());
    {
        let store = fixture.state.store.lock().unwrap();
        assert!(authority.context(&fixture.state, &store).is_err());
        cut.context(&store, false).unwrap();
        assert!(cut.context(&store, true).is_err());
        let check = |store: &Store, _: &AuthContext| {
            cut.context(store, true)
                .map(|_| ())
                .map_err(|_| dunlin_mcp_client::oauth::persistence::PersistenceError::Unauthorized)
        };
        let journal = store.mcp_oauth_journal(&check);
        let active = journal.active(&fixture.context, &"b".repeat(64)).unwrap();
        let refresh = journal
            .begin_refresh(&fixture.context, &"b".repeat(64), &active)
            .unwrap();
        journal
            .prepare_write(&fixture.context, &"b".repeat(64), &refresh, &"e".repeat(64))
            .unwrap();
        journal
            .commit(&fixture.context, &"b".repeat(64), &refresh, &"e".repeat(64))
            .unwrap();
        assert_eq!(
            journal
                .active(&fixture.context, &"b".repeat(64))
                .unwrap()
                .generation,
            active.generation + 1
        );
        assert_eq!(authority.pins().session, id);
    }
    drop(cut);
    assert!(fixture.state.management.try_lock().is_ok());
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn remote_adapter_fences_revocation_stop_foreign_state_and_weak_owner_drop() {
    for changed in ["device", "account", "journal", "stop", "state", "owner"] {
        let (fixture, service, actor, id, _) = live_fixture().await;
        let authority = actor.authority.clone();
        match changed {
            "device" => fixture
                .state
                .store
                .lock()
                .unwrap()
                .revoke_device(&fixture.data.device)
                .unwrap(),
            "account" => {
                fixture
                    .state
                    .store
                    .lock()
                    .unwrap()
                    .disable(&fixture.context)
                    .unwrap();
            }
            "journal" => {
                fixture.state.store.lock().unwrap().connection.execute("UPDATE controller_remote_sessions SET journal_epoch=journal_epoch+1 WHERE session=?1", [&id]).unwrap();
            }
            "stop" => authority.interrupt(),
            "state" => {
                let other = AppState::new(
                    Store::open_in_memory().unwrap(),
                    (*fixture.state.config).clone(),
                );
                assert!(authority.sql(&other).is_err());
                drop(actor);
                service.shutdown().await;
                continue;
            }
            "owner" => {
                drop(actor);
                service.shutdown().await;
                let weak = Arc::downgrade(&service);
                drop(service);
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while weak.upgrade().is_some() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert!(authority.sql(&fixture.state).is_err());
                assert!(authority.turn().is_err());
                continue;
            }
            _ => unreachable!(),
        }
        match authority.sql(&fixture.state) {
            Ok(cut) => assert!(
                cut.context(&fixture.state.store.lock().unwrap(), false)
                    .is_err(),
                "{changed}"
            ),
            Err(_) => assert_eq!(changed, "stop"),
        }
        drop(actor);
        service.shutdown().await;
    }
}

#[tokio::test]
async fn remote_catalog_request_requires_native_receipt_and_check_stops_before_io() {
    let (fixture, service, actor, id, _) = live_fixture().await;
    let snapshot = fixture
        .state
        .store
        .lock()
        .unwrap()
        .remote_session_snapshot(&id)
        .unwrap();
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
        actor
            .authority
            .validate_request(request.clone(), &fixture.state)
            .await
            .is_err()
    );
    fixture.state.store.lock().unwrap().connection.execute("UPDATE controller_remote_sessions SET phase='opening',dispatched=1,receipt=?2 WHERE session=?1",params![id,serde_json::to_string(&receipt).unwrap()]).unwrap();
    actor
        .authority
        .validate_request(request.clone(), &fixture.state)
        .await
        .unwrap();
    assert!(actor.authority.turn().is_err());
    let mut foreign = receipt.fence;
    foreign.session_epoch += 1;
    assert!(
        actor
            .authority
            .validate_request(
                wire::broker::BrokerScope::Catalog { session: foreign },
                &fixture.state
            )
            .await
            .is_err()
    );
    actor.authority.interrupt();
    let authority = actor.authority.clone();
    let state = fixture.state.clone();
    assert!(
        tokio::task::spawn_blocking(move || authority.check(
            &state,
            tokio::time::Instant::now() + std::time::Duration::from_secs(10)
        ))
        .await
        .unwrap()
        .is_err()
    );
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn pending_mcp_can_queue_before_factory_but_cannot_dispatch_or_ignore_revocation() {
    let (fixture, service, actor, id, _) = live_fixture().await;
    {
        let store = fixture.state.store.lock().unwrap();
        assert!(service.queue_allowed(&fixture.state, &store, &id, &fixture.data.owner));
        assert!(!service.approvals_allowed(&fixture.state, &store, &id, &fixture.data.owner));
        let snapshot = store.remote_session_snapshot(&id).unwrap();
        assert_eq!(snapshot.phase, "pending");
        assert!(!snapshot.dispatched && snapshot.receipt.is_none());
    }
    assert!(
        actor.dispatch_open().await.is_err(),
        "queue proof must never install dispatch authority"
    );
    {
        let store = fixture.state.store.lock().unwrap();
        store.disable(&fixture.context).unwrap();
        assert!(!service.queue_allowed(&fixture.state, &store, &id, &fixture.data.owner));
        assert!(!store.remote_session_snapshot(&id).unwrap().dispatched);
    }
    drop(actor);
    service.shutdown().await;
}
