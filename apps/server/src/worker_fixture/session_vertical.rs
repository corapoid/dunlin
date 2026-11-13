//! Real v4 TLS/WSS/sandbox transport and product REST/AHP/Journal acceptance.
//! Separate scenarios assert the source ledger and the controller projection.
use super::*;
use crate::worker_exports::{
    CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind, WorkerStore,
    policy::PolicySnapshot,
};
use dunlin_relay_protocol::worker::{
    self as worker,
    session::{self as wire, SessionControllerMessage as Request, SessionWorkerMessage as Reply},
};
use std::{fs, os::unix::fs::PermissionsExt};
mod mcp_browser;
mod mcp_failure;
mod mcp_form;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_session_transport() {
    session_transport(false, false, None).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_broker_transport() {
    session_transport(true, false, None).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_broker_client_timeout() {
    session_transport(true, true, None).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_native_mcp_proxy() {
    session_transport(true, false, Some("complete")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_native_mcp_revocation() {
    session_transport(true, false, Some("revoke")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_native_mcp_reply_quota() {
    session_transport(true, false, Some("quota")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_native_mcp_unix_disconnect() {
    session_transport(true, false, Some("disconnect")).await;
}
async fn open_request(
    transport: &crate::worker_channel::SessionTransport,
    open: &wire::SessionOpenCommand,
    broker: Option<&wire::broker::OpenBrokerSessionCommand>,
) -> Reply {
    match broker {
        Some(command) => transport
            .open_broker_session(command.clone())
            .await
            .unwrap(),
        None => transport
            .request(Request::OpenSession {
                command: Box::new(open.clone()),
            })
            .await
            .unwrap(),
    }
}
async fn session_transport(broker: bool, lost_reply: bool, native: Option<&str>) {
    let mut fixture = fixture_with_broker_transport(true, true, false, broker).await;
    let state_dir = fixture._dir.path().join("session-worker-state");
    let root = fixture._dir.path().join("session-checkout");
    fs::create_dir(&root).unwrap();
    fixture_git(&root, &["init", "-q"]);
    let scope = ExportScope {
        controller_node_id: fixture.client.controller.node_id.clone(),
        worker_node_id: fixture.client.network.node_id.clone(),
        workspace_id: fixture.workspace.clone(),
        host_id: fixture.host.clone(),
        enrollment_epoch: fixture.client.enrollment_epoch,
        credential_revision: fixture.client.credential_revision,
    };
    let store = WorkerStore::open(&state_dir, scope.clone()).unwrap();
    let policy_path = fixture._dir.path().join("session-policy.json");
    fs::write(&policy_path, serde_json::to_vec(&json!({"schema_version":1,"harnesses":["opencode"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1})).unwrap()).unwrap();
    if native.is_some() {
        let mut policy: Value = serde_json::from_slice(&fs::read(&policy_path).unwrap()).unwrap();
        policy["schema_version"] = json!(2);
        policy["mcp_broker"] = json!(true);
        fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
    }
    fs::set_permissions(&policy_path, fs::Permissions::from_mode(0o600)).unwrap();
    let policy = PolicySnapshot::load(&policy_path).unwrap();
    let checkout = CapturedCheckout::capture(&root, policy.digest(), None).unwrap();
    let export = store
        .register_captured_with_policy(&scope, "Session transport export", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &scope,
            &export.id,
            &ExportPrincipal {
                kind: PrincipalKind::Human,
                id: fixture.owner.clone(),
            },
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(store);
    let bin = state_dir.join("execution-runtime/server/tools/bin");
    crate::management::private_dir(&bin).unwrap();
    let program = bin.join("session-transport-agent");
    fs::write(
        &program,
        include_str!("../worker_execution/runtime/fixtures/session_agent.py"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.client.exports_state_dir = Some(state_dir.clone());
    fixture.client.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    fixture.client.execution = Some(serde_json::from_value(json!({"enabled":true,"persistent_sessions":true,"mcp_broker":broker,"harnesses":{"opencode":{"program":program,"args":[if native==Some("disconnect") {"native-mcp-disconnect"}else if native.is_some() {"native-mcp"}else{"persistent"}]}}})).unwrap());
    let (profile, definition) = {
        let store = fixture.state.store.lock().unwrap();
        let project = store
            .create_team_project(
                &fixture.owner,
                &fixture.workspace,
                "Session transport project",
            )
            .unwrap();
        let definition = serde_json::from_value(json!({"schema_version":1,"name":"Persistent transport agent","description":"","harness":"opencode","model":"fixture-model","mode":"fixture-mode","instructions":"literal-instructions","skills":[{"id":"fixture","version":"1","instructions":"literal-skill"}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        let mut definition: dunlin_domain::agent::AgentDefinition = definition;
        if native.is_some() {
            definition.schema_version = 2;
            definition.mcp.push(dunlin_domain::agent::AgentMcpTool {
                provider_id: "fixture".into(),
                name: "lookup".into(),
                definition_digest: "a".repeat(64),
            });
        }
        let draft = store
            .create_agent(&fixture.owner, &fixture.workspace, &project.id, definition)
            .unwrap();
        let published = store
            .publish_agent(
                &fixture.owner,
                &fixture.workspace,
                &project.id,
                &draft.id,
                1,
            )
            .unwrap();
        (
            dunlin_domain::agent::AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            },
            published.definition,
        )
    };
    let (sender, mut events) = tokio::sync::mpsc::channel(32);
    let producer = fixture
        .state
        .worker_channels
        .register_session_producer(sender)
        .unwrap();
    let config = fixture.client.clone();
    let credential = fixture.credential.clone();
    let dial = fixture.transport.clone();
    let mut task = tokio::spawn(async move {
        crate::worker_client::run_with_transport(config, Zeroizing::new(credential), dial).await
    });
    struct Cleanup(tokio::task::AbortHandle);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _cleanup = Cleanup(task.abort_handle());
    let transport = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(transport) = fixture.state.worker_channels.session_transport(&fixture.host) { break transport; }
            tokio::select! { result = &mut task => panic!("v4 worker stopped: {result:?}"), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
        }
    }).await.expect("v4 registration deadline");
    assert_eq!(transport.broker_offer().is_some(), broker);
    // A v4 transport never masquerades as a v3 one-turn transport.
    assert!(
        fixture
            .state
            .worker_channels
            .execution_transport(&fixture.host)
            .is_err()
    );
    let proof = fixture
        .state
        .worker_channels
        .describe_session_export(
            &transport,
            &export.id,
            worker::ExportPrincipal {
                kind: worker::PrincipalKind::Human,
                id: fixture.owner.clone(),
            },
            Some(export.revision),
        )
        .await
        .unwrap();
    {
        let store = fixture.state.store.lock().unwrap();
        store
            .remote_binding_transaction(false, |store| {
                transport.attest_in_transaction(
                    store,
                    proof.identity(),
                    proof.lease(),
                    transport.fence(),
                )
            })
            .unwrap();
    }
    let mut open = wire::SessionOpenCommand {
        channel: transport.fence().clone(),
        identity: wire::SessionIdentity {
            intent_id: "session-transport-intent".into(),
            fingerprint: "0".repeat(64),
        },
        controller_session_id: "session-transport-controller".into(),
        descriptor: proof.descriptor().clone(),
        profile,
        definition,
        budgets: wire::SessionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            idle_timeout_ms: 10000,
            lease_duration_ms: 10000,
            max_turns: 4,
            retained_bytes: 65536,
        },
    };
    let mut broker_open = native.map(|_| wire::broker::OpenBrokerSessionCommand {
        protocol_version: 5,
        open: open.clone(),
        broker_limits: wire::broker::BrokerLimits {
            max_pending: 2,
            max_reply_bytes: if native == Some("quota") { 8192 } else { 65536 },
        },
    });
    if let Some(command) = broker_open.as_mut() {
        command.open.identity.fingerprint =
            wire::broker::broker_session_open_fingerprint(command).unwrap();
        open = command.open.clone();
    } else {
        open.identity.fingerprint = wire::session_open_fingerprint(&open).unwrap();
    }
    let native_handler = native
        .map(|mode| super::broker_client::native(&transport, &open.controller_session_id, mode));
    let accepted = open_request(&transport, &open, broker_open.as_ref()).await;
    let Reply::SessionAccepted { receipt } = &accepted else {
        panic!("{accepted:?}")
    };
    let ready = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(ready.message, Reply::SessionReady { receipt: ref actual } if actual == receipt)
    );
    assert_eq!(ready.transport.fence(), transport.fence());
    if lost_reply {
        super::broker_client::lost_reply(&fixture, &transport, receipt).await;
        let stopped = tokio::time::timeout(Duration::from_secs(15), &mut task)
            .await
            .expect("uncertain MCP reply must stop real source runtime")
            .unwrap();
        assert_eq!(stopped, Err("worker_broker_expired"));
        let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
        let stopped: (String, i64) = database
            .query_row("SELECT phase,reservation FROM worker_sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(stopped, ("stopped".into(), 0));
        let calls = fs::read_to_string(root.join("session-calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .filter(|call| call["method"] == "session/new")
                .count(),
            1
        );
        assert!(!root.join("effects").exists());
        drop(producer);
        fixture.state.worker_channels.shutdown().await;
        return;
    }

    assert_eq!(
        open_request(&transport, &open, broker_open.as_ref()).await,
        accepted
    );
    let mut sequence = 0;
    let mut digest: Option<String> = None;
    for index in 1..=2 {
        let mut command = wire::TurnCommand {
            session: receipt.fence.clone(),
            identity: wire::TurnIdentity {
                command_id: format!("session-command-{index}"),
                turn_id: format!("session-turn-{index}"),
                fingerprint: "0".repeat(64),
            },
            turn_index: index,
            input: if index == 1 {
                "Remember nonce:network-context".into()
            } else {
                "Recall prior nonce".into()
            },
            budgets: worker::ExecutionBudgets {
                max_seconds: 10,
                max_output_bytes: 16384,
                lease_duration_ms: 10000,
            },
        };
        command.identity.fingerprint = wire::session_turn_fingerprint(&command).unwrap();
        let accepted_turn = transport
            .request(Request::SubmitTurn {
                command: Box::new(command.clone()),
            })
            .await
            .unwrap();
        if native == Some("quota") {
            let (_, handler) = native_handler.as_ref().unwrap();
            handler.wait_call().await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(15), &mut task)
                    .await
                    .unwrap()
                    .unwrap(),
                Err("invalid_worker_broker_reply")
            );
            let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
            let value: (String, i64) = database
                .query_row("SELECT phase,reservation FROM worker_sessions", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(value, ("stopped".into(), 0));
            assert_eq!(handler.calls(), 1);
            assert!(!root.join("effects").exists());
            let held = fs::metadata(root.join("session-held-effect"))
                .unwrap()
                .len();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                fs::metadata(root.join("session-held-effect"))
                    .unwrap()
                    .len(),
                held
            );
            drop(producer);
            fixture.state.worker_channels.shutdown().await;
            return;
        }
        if matches!(native, Some("revoke" | "disconnect")) {
            let (_, handler) = native_handler.as_ref().unwrap();
            handler.wait_call().await;
            if native == Some("revoke") {
                let store = WorkerStore::open_existing(&state_dir, scope.clone()).unwrap();
                store
                    .set_grant(
                        &scope,
                        &export.id,
                        &ExportPrincipal {
                            kind: PrincipalKind::Human,
                            id: fixture.owner.clone(),
                        },
                        &[ExportPermission::Read],
                        true,
                        Some(1),
                    )
                    .unwrap();
                drop(store);
            }
            handler.release();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let database =
                        rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
                    let value: (String, i64) = database
                        .query_row("SELECT phase,reservation FROM worker_sessions", [], |r| {
                            Ok((r.get(0)?, r.get(1)?))
                        })
                        .unwrap();
                    if value == ("stopped".into(), 0) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("native proxy failure must stop proxy and sandbox");
            assert_eq!(handler.calls(), 1);
            assert!(!root.join("effects").exists());
            let held = fs::metadata(root.join("session-held-effect"))
                .unwrap()
                .len();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                fs::metadata(root.join("session-held-effect"))
                    .unwrap()
                    .len(),
                held
            );
            drop(producer);
            fixture.state.worker_channels.shutdown().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
            return;
        }
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Reply::TurnAccepted { receipt } = &event.message {
                assert_eq!(
                    &accepted_turn,
                    &Reply::TurnAccepted {
                        receipt: receipt.clone()
                    }
                );
                continue;
            }
            let Reply::Event { event } = event.message else {
                panic!("unexpected session event")
            };
            event
                .validate_after(&receipt.fence, sequence, digest.as_deref())
                .unwrap();
            sequence = event.sequence;
            digest = Some(event.digest.clone());
            let terminal = match &event.event {
                wire::SessionEventKind::Text { text, .. } => {
                    assert_eq!(text, "remembered:network-context");
                    false
                }
                wire::SessionEventKind::TurnCompleted {
                    receipt: actual,
                    status,
                } => {
                    assert_eq!(actual.identity, command.identity);
                    assert_eq!(*status, wire::TurnTerminalStatus::Completed);
                    true
                }
                _ => panic!("unexpected stop during prompt"),
            };
            // This tests worker SQL ACK evidence. Controller Journal persistence
            // and its ordering before ACK are a separate integration layer.
            let ack = Request::AckStream {
                channel: transport.fence().clone(),
                session: receipt.fence.clone(),
                through_sequence: sequence,
                event_digest: event.digest,
            };
            assert!(matches!(
                transport.request(ack.clone()).await.unwrap(),
                Reply::StreamAcknowledged { .. }
            ));
            let store = fixture.state.store.lock().unwrap();
            store
                .remote_binding_transaction(false, |store| {
                    transport.attest_stream_ack_in_transaction(store, &ack)
                })
                .unwrap();
            if terminal {
                break;
            }
        }
        assert_eq!(
            transport
                .request(Request::SubmitTurn {
                    command: Box::new(command)
                })
                .await
                .unwrap(),
            accepted_turn
        );
    }
    assert_eq!(sequence, 4);
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\n");
    transport
        .request(Request::CloseSession {
            session: receipt.fence.clone(),
        })
        .await
        .unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Reply::Event { event } = stopped.message else {
        panic!("missing session stop")
    };
    event
        .validate_after(&receipt.fence, sequence, digest.as_deref())
        .unwrap();
    assert!(matches!(
        event.event,
        wire::SessionEventKind::SessionStopped { .. }
    ));
    transport
        .request(Request::AckStream {
            channel: transport.fence().clone(),
            session: receipt.fence.clone(),
            through_sequence: event.sequence,
            event_digest: event.digest,
        })
        .await
        .unwrap();
    let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
    let counts: (i64,i64,i64) = database.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions)", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts, (1, 2, 0));
    let before = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        before
    );
    if let Some((_, handler)) = &native_handler {
        assert_eq!(handler.calls(), 2);
    }
    if broker && native.is_none() {
        super::broker_client::interleaved(&fixture, &transport, receipt).await;
    }
    drop(producer);
    fixture.state.worker_channels.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_session_journal() {
    session_journal(None).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_pipeline() {
    session_journal(Some("complete")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_deny() {
    session_journal(Some("deny")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_lost_http_reply() {
    session_journal(Some("lost")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_revoke() {
    session_journal(Some("revoke")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_renew_terminal_race() {
    session_journal(Some("renew-race")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_forms() {
    session_journal(Some("forms")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_forms_decline() {
    session_journal(Some("forms-decline")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_forms_cancel() {
    session_journal(Some("forms-cancel")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_forms_revoke() {
    session_journal(Some("forms-revoke")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_urls() {
    session_journal(Some("urls")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_urls_decline() {
    session_journal(Some("urls-decline")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_urls_cancel() {
    session_journal(Some("urls-cancel")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_mcp_product_urls_revoke() {
    session_journal(Some("urls-revoke")).await;
}
async fn session_journal(mode: Option<&str>) {
    let mcp = mode.is_some();
    let denied = mode == Some("deny");
    let urls = mode.is_some_and(|m| m.starts_with("urls"));
    let forms = urls || mode.is_some_and(|m| m.starts_with("forms"));
    let no_effects = denied
        || matches!(
            mode,
            Some("forms-decline" | "forms-cancel" | "urls-decline" | "urls-cancel")
        );
    let form_action = if forms {
        Some(match mode.unwrap() {
            "forms" => "accept",
            "forms-decline" => "decline",
            "urls" => "url-accept",
            "urls-decline" => "url-decline",
            "urls-cancel" | "urls-revoke" => "url-cancel",
            _ => "cancel",
        })
    } else {
        None
    };
    let mut fixture = fixture_with_broker_transport(true, true, false, mcp).await;
    let provider = mcp.then(super::mcp_provider::Provider::new);
    if let Some(provider) = &provider {
        if forms {
            provider.enable_forms(match mode.unwrap() {
                "forms" | "forms-revoke" => 1,
                "forms-decline" => 2,
                "urls" | "urls-revoke" => 4,
                "urls-decline" => 5,
                "urls-cancel" => 6,
                _ => 3,
            });
        }
        if mode == Some("lost") {
            provider.lose_response();
        }
        if matches!(mode, Some("revoke" | "renew-race")) {
            provider.hold_response();
        }
    }
    let selections = provider
        .as_ref()
        .map(|provider| provider.install(&mut fixture.state, &fixture.owner, &fixture.workspace))
        .into_iter()
        .collect::<Vec<_>>();
    let state_dir = fixture._dir.path().join("session-worker-state");
    let root = fixture._dir.path().join("session-checkout");
    fs::create_dir(&root).unwrap();
    fixture_git(&root, &["init", "-q"]);
    let scope = ExportScope {
        controller_node_id: fixture.client.controller.node_id.clone(),
        worker_node_id: fixture.client.network.node_id.clone(),
        workspace_id: fixture.workspace.clone(),
        host_id: fixture.host.clone(),
        enrollment_epoch: fixture.client.enrollment_epoch,
        credential_revision: fixture.client.credential_revision,
    };
    let store = WorkerStore::open(&state_dir, scope.clone()).unwrap();
    let policy_path = fixture._dir.path().join("session-policy.json");
    fs::write(&policy_path, serde_json::to_vec(&json!({"schema_version":1,"harnesses":["opencode"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1})).unwrap()).unwrap();
    if mcp {
        let mut policy: Value = serde_json::from_slice(&fs::read(&policy_path).unwrap()).unwrap();
        policy["schema_version"] = json!(2);
        policy["mcp_broker"] = json!(true);
        fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
    }
    fs::set_permissions(&policy_path, fs::Permissions::from_mode(0o600)).unwrap();
    let policy = PolicySnapshot::load(&policy_path).unwrap();
    let checkout = CapturedCheckout::capture(&root, policy.digest(), None).unwrap();
    let export = store
        .register_captured_with_policy(&scope, "Session transport export", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &scope,
            &export.id,
            &ExportPrincipal {
                kind: PrincipalKind::Human,
                id: fixture.owner.clone(),
            },
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(store);
    let bin = state_dir.join("execution-runtime/server/tools/bin");
    crate::management::private_dir(&bin).unwrap();
    let program = bin.join("session-transport-agent");
    fs::write(
        &program,
        include_str!("../worker_execution/runtime/fixtures/session_agent.py"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.client.exports_state_dir = Some(state_dir.clone());
    fixture.client.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    fixture.client.execution = Some(serde_json::from_value(json!({"enabled":true,"persistent_sessions":true,"mcp_broker":mcp,"harnesses":{"opencode":{"program":program,"args":[if denied {"native-mcp-deny"}else if no_effects {"native-mcp-tool-failed"}else if mcp {"native-mcp"}else{"persistent"}]}}})).unwrap());
    let (profile, _definition, project) = {
        let store = fixture.state.store.lock().unwrap();
        let project = store
            .create_team_project(
                &fixture.owner,
                &fixture.workspace,
                "Session transport project",
            )
            .unwrap();
        let definition = serde_json::from_value(json!({"schema_version":1,"name":"Persistent transport agent","description":"","harness":"opencode","model":"fixture-model","mode":"fixture-mode","instructions":"literal-instructions","skills":[{"id":"fixture","version":"1","instructions":"literal-skill"}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        let mut definition: dunlin_domain::agent::AgentDefinition = definition;
        if mcp {
            definition.schema_version = 2;
            definition.mcp.push(super::mcp_provider::Provider::tool());
        }
        let draft = store
            .create_agent(&fixture.owner, &fixture.workspace, &project.id, definition)
            .unwrap();
        let published = store
            .publish_agent(
                &fixture.owner,
                &fixture.workspace,
                &project.id,
                &draft.id,
                1,
            )
            .unwrap();
        (
            dunlin_domain::agent::AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            },
            published.definition,
            project,
        )
    };

    let auth = {
        let store = fixture.state.store.lock().unwrap();
        let (device, _) = store
            .create_device(&fixture.owner, "journal-browser")
            .unwrap();
        crate::api::AuthUser {
            user: store.active_device_user(&device.id).unwrap().unwrap(),
            device,
        }
    };
    let faults = crate::worker_client::session_faults::Guard::install(&fixture.host);
    if mode == Some("renew-race") {
        faults.arm_renew_terminal_race(provider.as_ref().unwrap().release_hook());
    }
    let service = crate::remote_sessions::Service::start(&fixture.state)
        .await
        .unwrap();
    let config = fixture.client.clone();
    let credential = fixture.credential.clone();
    let dial = fixture.transport.clone();
    let mut task = tokio::spawn(async move {
        crate::worker_client::run_with_transport(config, Zeroizing::new(credential), dial).await
    });
    struct Cleanup(tokio::task::AbortHandle);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _cleanup = Cleanup(task.abort_handle());
    let transport = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(transport) = fixture.state.worker_channels.session_transport(&fixture.host) { break transport; }
            tokio::select! { result = &mut task => panic!("v4 worker stopped: {result:?}"), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
        }
    }).await.expect("v4 registration deadline");

    let proof = fixture
        .state
        .worker_channels
        .describe_session_export(
            &transport,
            &export.id,
            worker::ExportPrincipal {
                kind: worker::PrincipalKind::Human,
                id: fixture.owner.clone(),
            },
            Some(export.revision),
        )
        .await
        .unwrap();
    let binding = {
        let store = fixture.state.store.lock().unwrap();
        let authority = store
            .capture_remote_binding_authority(&fixture.owner, &fixture.workspace, &project.id)
            .unwrap();
        store
            .set_remote_binding(
                &fixture.owner,
                &fixture.workspace,
                &project.id,
                None,
                None,
                &fixture.host,
                &export.id,
                export.revision,
                &proof,
                &fixture.state.worker_channels.boot_id,
                &fixture.client.controller.node_id,
                fixture_now_ms(),
                &authority,
            )
            .unwrap()
    };
    let request = crate::store::remote_bindings::resolver::sessions::RemoteSessionRequest {
        binding: dunlin_domain::binding::BindingRef {
            id: binding.id,
            revision: binding.revision,
        },
        profile,
    };
    fixture
        .state
        .remote_sessions
        .set(service.clone())
        .ok()
        .unwrap();
    let cookie = {
        let store = fixture.state.store.lock().unwrap();
        format!(
            "dunlin_web_session={}",
            store.create_browser_session(&auth.device.id, 3600).unwrap()
        )
    };
    let path = format!(
        "/v1/web/workspaces/{}/projects/{}/interactive-sessions",
        fixture.workspace, project.id
    );
    let payload = json!({"profile":request.profile,"mcp_connections":selections,"target":{"kind":"remote","binding":request.binding}});
    let (status, accepted) = session_http(&fixture.state, &cookie, &path, payload.clone()).await;
    assert_eq!(status, 202, "{accepted}");
    assert_eq!(accepted["admission"]["replayed"], false);
    let id = accepted["session"]["id"].as_str().unwrap().to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::api::router(fixture.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let _server_cleanup = Cleanup(server.abort_handle());
    for index in 1..=2 {
        let mut socket = session_connect(&origin, &cookie).await;
        let initialized=session_rpc(&mut socket,1,"initialize",json!({"channel":"ahp-root://","clientId":format!("remote-browser-{index}"),"protocolVersions":["1.0.0"],"initialSubscriptions":["ahp-root://",format!("ahp-chat:/{id}"),format!("ahp-session:/{id}")]})).await;
        assert!(initialized.get("error").is_none(), "{initialized}");
        let dispatched=session_dispatch(&mut socket,&format!("remote-browser-{index}"),json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/pendingMessageSet","kind":"queued","id":format!("remote-prompt-{index}"),"message":{"text":if index==1 {"Remember nonce:journal-context"}else{"Recall prior nonce"},"origin":{"kind":"user"}}}})).await;
        assert!(dispatched.get("error").is_none(), "{dispatched}");
        if mcp {
            let tool = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let value = {
                        let store = fixture.state.store.lock().unwrap();
                        let context = store
                            .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
                            .unwrap();
                        let chat = serde_json::to_value(
                            store
                                .interactive_journal()
                                .projected_chat(&context, &id)
                                .unwrap(),
                        )
                        .unwrap();
                        chat["activeTurn"]["responseParts"]
                            .as_array()
                            .and_then(|parts| {
                                parts.iter().find(|part| {
                                    part["kind"] == "toolCall"
                                        && part["toolCall"]["status"] == "pending-confirmation"
                                })
                            })
                            .map(|part| {
                                (chat["activeTurn"]["id"].clone(), part["toolCall"].clone())
                            })
                    };
                    if let Some(value) = value {
                        break value;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("remote SDK consent deadline");
            let option = tool.1["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["kind"] == if denied { "deny" } else { "approve" })
                .unwrap();
            let confirmation = json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":2,"action":{"type":"chat/toolCallConfirmed","turnId":tool.0,"toolCallId":tool.1["toolCallId"],"approved":!denied,"selectedOptionId":option["id"]}});
            let approved = session_dispatch(
                &mut socket,
                &format!("remote-browser-{index}"),
                confirmation.clone(),
            )
            .await;
            assert!(approved.get("error").is_none(), "{approved}");
            if matches!(mode, Some("forms-revoke" | "urls-revoke")) {
                mcp_form::pending(&fixture, &id).await;
            }
            if forms && !matches!(mode, Some("forms-revoke" | "urls-revoke")) {
                mcp_form::respond(
                    &fixture,
                    &id,
                    &mut socket,
                    &format!("remote-browser-{index}"),
                    mode.unwrap(),
                )
                .await;
            }
            if matches!(
                mode,
                Some("lost" | "revoke" | "forms-revoke" | "urls-revoke")
            ) {
                mcp_failure::Case {
                    fixture: &fixture,
                    mode: mode.unwrap(),
                    id: &id,
                    path: &path,
                    payload: &payload,
                    cookie: &cookie,
                    root: &root,
                    state_dir: &state_dir,
                    provider: provider.as_ref().unwrap(),
                    selection: &selections[0],
                    faults: &faults,
                }
                .verify(&mut socket, confirmation)
                .await;
                if forms {
                    assert_eq!(provider.as_ref().unwrap().continuations(), 0);
                }
                drop(socket);
                fixture.state.worker_channels.shutdown().await;
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
                service.shutdown().await;
                return;
            }
        }
        drop(socket); // Browser ownership ends before the worker/Journal turn.
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                {
                    let store = fixture.state.store.lock().unwrap();
                    let context = store
                        .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
                        .unwrap();
                    let (session, turns) =
                        store.interactive_journal().snapshot(&context, &id).unwrap();
                    assert_ne!(
                        format!("{:?}", session.phase),
                        "Interrupted",
                        "controller interrupted before prompt completion"
                    );
                    if turns
                        .iter()
                        .filter(|turn| turn.phase == dunlin_session_runtime::TurnPhase::Completed)
                        .count()
                        == index
                    {
                        break;
                    }
                }
                assert!(!task.is_finished());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("Journal completion deadline");
        // The second turn arrives after an idle renewal, proving ownership
        // survives the first terminal and the browser request's return.
        if index == 1 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    if no_effects {
        assert!(!root.join("effects").exists());
    } else {
        assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\n");
    }
    let calls = fs::read_to_string(root.join("session-calls")).unwrap();
    let calls: Vec<Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "initialize")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/new")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        2
    );
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
            .unwrap();
        let chat = store
            .interactive_journal()
            .projected_chat(&context, &id)
            .unwrap();
        let body = serde_json::to_string(&chat).unwrap();
        assert_eq!(body.matches("remembered:journal-context").count(), 2);
    }
    let (status, replay) = session_http(&fixture.state, &cookie, &path, payload).await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["admission"]["replayed"], true);
    assert_eq!(replay["session"]["id"], id);
    let epoch = {
        let store = fixture.state.store.lock().unwrap();
        store.remote_session_snapshot(&id).unwrap().lease.epoch
    };
    let (status, stopped) = fixture_http(
        &fixture.state,
        &cookie,
        &format!("/v1/web/interactive-sessions/{id}/stop"),
        json!({"expected_epoch":epoch}),
    )
    .await;
    assert_eq!(status, 202, "{stopped}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let done = {
                let store = fixture.state.store.lock().unwrap();
                let snapshot = store.remote_session_snapshot(&id).unwrap();
                snapshot.phase == "stopped"
                    && snapshot.sequence == 5
                    && snapshot.confirmed_ack == snapshot.ack_sequence
            };
            if done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("source stop and durable ACK deadline");
    {
        let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
        let counts:(i64,i64,i64,i64)=database.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT min(ack_sequence) FROM worker_sessions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(counts, (1, 2, 0, 5));
    }
    let before = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        before
    );
    faults.assert_completed();
    if mode == Some("renew-race") {
        faults.assert_renew_terminal_race();
    }
    if !mcp && std::env::var("DUNLIN_REMOTE_SESSION_BROWSER").as_deref() == Ok("1") {
        session_browser(&fixture, &origin, &request, &project.id, &root, &state_dir).await;
    }
    if let Some(provider) = &provider {
        assert_eq!(provider.calls(), if denied { 0 } else { 2 });
        if forms {
            assert_eq!(provider.continuations(), if no_effects { 2 } else { 4 });
        }
        if (mode == Some("complete") || forms)
            && std::env::var("DUNLIN_REMOTE_SESSION_BROWSER").as_deref() == Ok("1")
        {
            mcp_browser::run(
                &fixture,
                &origin,
                &request,
                &project.id,
                &root,
                &state_dir,
                &selections,
                &cookie,
                form_action,
                no_effects,
            )
            .await;
            assert_eq!(provider.calls(), 3);
            if forms {
                assert_eq!(provider.continuations(), if no_effects { 3 } else { 6 });
            }
        }
    }
    fixture.state.worker_channels.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
    let (status,replayed)=session_http(&fixture.state,&cookie,&path,json!({"profile":request.profile,"mcp_connections":selections,"target":{"kind":"remote","binding":request.binding}})).await;
    assert_eq!(status, 202, "{replayed}");
    assert_eq!(replayed["admission"]["replayed"], true);
    assert_eq!(replayed["session"]["id"], id);
    service.shutdown().await;
}

async fn session_http(state: &AppState, cookie: &str, path: &str, body: Value) -> (u16, Value) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let response = crate::api::router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("Host", state.config.listen.as_str())
                .header("Origin", format!("http://{}", state.config.listen))
                .header("Cookie", cookie)
                .header("Content-Type", "application/json")
                .header("Idempotency-Key", "journal-session-intent-0001")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
pub(super) type SessionSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub(super) async fn session_connect(origin: &str, cookie: &str) -> SessionSocket {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = format!("{}/v1/web/ahp", origin.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
pub(super) async fn session_rpc(
    socket: &mut SessionSocket,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["id"] == id {
                        return value;
                    }
                    assert!(value["params"]["rejectionReason"].is_null(), "{value}");
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                frame => panic!("unexpected AHP frame: {frame:?}"),
            }
        }
    })
    .await
    .unwrap()
}

pub(super) async fn session_dispatch(
    socket: &mut SessionSocket,
    client: &str,
    params: Value,
) -> Value {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","method":"dispatchAction","params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert!(value.get("error").is_none(), "{value}");
                    if value["method"] == "action"
                        && value["params"]["origin"]["clientId"] == client
                        && value["params"]["origin"]["clientSeq"] == params["clientSeq"]
                    {
                        assert!(value["params"]["rejectionReason"].is_null(), "{value}");
                        let mut expected = params["action"].clone();
                        if expected["type"] == "chat/inputCompleted" {
                            expected = serde_json::to_value(serde_json::from_value::<ahp_types::actions::StateAction>(expected).unwrap()).unwrap();
                        }
                        if expected["type"] == "chat/toolCallConfirmed" {
                            assert!(expected["toolCallId"].as_str().unwrap().starts_with("dunlin:mcp:"));
                            let nonce = expected["selectedOptionId"].as_str().unwrap()
                                .strip_prefix("permission:").unwrap().rsplit_once(':').unwrap().0.to_owned();
                            uuid::Uuid::parse_str(&nonce).unwrap();
                            if params["action"]["approved"] == true {
                                expected["confirmed"] = json!("user-action");
                            } else {
                                expected["reason"] = json!("denied");
                            }
                            expected["_meta"] = json!({"dunlin.confirmationEvidence":"human-one-use",
                                "dunlin.humanApproved":params["action"]["approved"],"dunlin.permissionNonce":nonce});
                        }
                        assert_eq!(value["params"]["action"], expected);
                        return value;
                    }
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                frame => panic!("unexpected AHP dispatch frame: {frame:?}"),
            }
        }
    })
    .await
    .expect("durable AHP origin echo deadline")
}

async fn session_browser(
    fixture: &Fixture,
    origin: &str,
    request: &crate::store::remote_bindings::resolver::sessions::RemoteSessionRequest,
    project: &str,
    root: &std::path::Path,
    state_dir: &std::path::Path,
) {
    use tokio::io::AsyncWriteExt;
    let code = {
        let store = fixture.state.store.lock().unwrap();
        store.create_pairing_code(&fixture.owner, 900).unwrap().0
    };
    let input = json!({"origin":origin,"ownerCode":code,"workspace":fixture.workspace,"host":fixture.host,"project":project,"binding":request.binding,"profile":request.profile});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/remote-session.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(format!("{input}\n").as_bytes())
        .await
        .unwrap();
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
        .await
        .expect("Chromium Remote session deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "Chromium Remote session failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 65536);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "passed");
    let id = result["session_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let done = {
                let store = fixture.state.store.lock().unwrap();
                let snapshot = store.remote_session_snapshot(id).unwrap();
                snapshot.phase == "stopped" && snapshot.sequence == 7 && snapshot.confirmed_ack == 7
            };
            if done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Chromium session stop proof and worker ACK deadline");
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\nx\nx\nx\n");
    let database = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
    let counts:(i64,i64,i64,i64)=database.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT sum(ack_sequence) FROM worker_sessions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(counts, (2, 5, 0, 12));
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(&fixture.workspace, project, &fixture.owner)
            .unwrap();
        let (_, turns) = store.interactive_journal().snapshot(&context, id).unwrap();
        assert_eq!(
            turns
                .iter()
                .filter(|turn| turn.phase == dunlin_session_runtime::TurnPhase::Completed)
                .count(),
            2
        );
        assert_eq!(
            turns
                .iter()
                .filter(|turn| turn.phase == dunlin_session_runtime::TurnPhase::Unknown)
                .count(),
            1
        );
        let queued: Vec<_> = turns
            .iter()
            .filter(|turn| turn.phase == dunlin_session_runtime::TurnPhase::Queued)
            .map(|turn| turn.prompt.as_str())
            .collect();
        assert_eq!(queued, ["queued-second", "QUEUED_AFTER_CANCEL_EDITED"]);
    }
    let calls = fs::read_to_string(root.join("session-calls")).unwrap();
    let calls: Vec<Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "initialize")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/new")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        5
    );
    let before = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        before
    );
    eprintln!("remote-session production Chromium branch: PASS");
}
