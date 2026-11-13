//! End-to-end worker session lifecycle in a dedicated real containment process.
use super::*;
use wire::session::{
    self as session, SessionControllerMessage as SessionRequest,
    SessionWorkerMessage as SessionReply,
};

async fn session_record(
    runtime: &Arc<Runtime>,
    connection: &Connected,
    identity: &session::SessionIdentity,
) -> session::SessionRecord {
    match runtime
        .handle_session(
            connection,
            SessionRequest::ReconcileSession {
                channel: connection.fence.clone(),
                identity: identity.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap()
    {
        SessionReply::SessionReconciled {
            record: Some(record),
            ..
        } => record,
        other => panic!("unexpected session reconcile: {other:?}"),
    }
}

async fn wait_stopped(runtime: &Arc<Runtime>, spec: &Spec, fence: &session::SessionFence) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let database = rusqlite::Connection::open(spec.state.join("exports.sqlite3")).unwrap();
        let (phase, reservation): (String, i64) = database
            .query_row(
                "SELECT phase,reservation FROM worker_sessions WHERE id=?1",
                [&fence.worker_session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        if phase == "stopped" {
            assert_eq!(reservation, 0);
            break;
        }
        assert!(Instant::now() < deadline, "session stop not confirmed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    runtime.shutdown().await;
    fs::write(&spec.output, "session-ok").unwrap();
}

pub(super) async fn run(runtime: &Arc<Runtime>, connection: &Connected, spec: &Spec) {
    let mut command = session::SessionOpenCommand {
        channel: connection.fence.clone(),
        identity: session::SessionIdentity {
            intent_id: "persistent-open".into(),
            fingerprint: "0".repeat(64),
        },
        controller_session_id: "persistent-controller-session".into(),
        descriptor: spec.command.descriptor.clone(),
        profile: spec.command.profile.clone(),
        definition: spec.command.definition.clone(),
        budgets: session::SessionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            idle_timeout_ms: if spec.action == "session-expiry" {
                500
            } else {
                10000
            },
            lease_duration_ms: 10000,
            max_turns: 4,
            retained_bytes: 65536,
        },
    };
    command.identity.fingerprint = session::session_open_fingerprint(&command).unwrap();
    let opened = runtime
        .handle_session(
            connection,
            SessionRequest::OpenSession {
                command: Box::new(command.clone()),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let SessionReply::SessionAccepted { receipt } = opened.clone() else {
        panic!("{opened:?}")
    };
    if spec.action == "session-fault" && !spec.fault.ends_with("running") {
        wait_stopped(runtime, spec, &receipt.fence).await;
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let record = session_record(runtime, connection, &command.identity).await;
        if record.phase == session::SessionPhase::Idle {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "session did not become ready: {record:?}"
        );
        assert_ne!(
            record.phase,
            session::SessionPhase::Stopped,
            "session stopped during initialization"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        runtime
            .handle_session(
                connection,
                SessionRequest::OpenSession {
                    command: Box::new(command.clone())
                }
            )
            .await
            .unwrap()
            .unwrap(),
        opened
    );
    fs::write(&spec.ready, serde_json::to_vec(&receipt).unwrap()).unwrap();
    if spec.action == "session-fault" {
        let mut turn = session::TurnCommand {
            session: receipt.fence.clone(),
            identity: session::TurnIdentity {
                command_id: "fault-command".into(),
                turn_id: "fault-turn".into(),
                fingerprint: "0".repeat(64),
            },
            turn_index: 1,
            input: "Remember nonce:fault-context".into(),
            budgets: wire::ExecutionBudgets {
                max_seconds: 10,
                max_output_bytes: 16384,
                lease_duration_ms: 10000,
            },
        };
        turn.identity.fingerprint = session::session_turn_fingerprint(&turn).unwrap();
        runtime
            .handle_session(
                connection,
                SessionRequest::SubmitTurn {
                    command: Box::new(turn),
                },
            )
            .await
            .unwrap();
    } else if spec.action == "session-success" {
        for index in 1..=2 {
            let mut turn = session::TurnCommand {
                session: receipt.fence.clone(),
                identity: session::TurnIdentity {
                    command_id: format!("command-{index}"),
                    turn_id: format!("turn-{index}"),
                    fingerprint: "0".repeat(64),
                },
                turn_index: index,
                input: if index == 1 {
                    "Remember nonce:kernel-context".into()
                } else {
                    "Recall the previous nonce".into()
                },
                budgets: wire::ExecutionBudgets {
                    max_seconds: 10,
                    max_output_bytes: 16384,
                    lease_duration_ms: 10000,
                },
            };
            turn.identity.fingerprint = session::session_turn_fingerprint(&turn).unwrap();
            let accepted = runtime
                .handle_session(
                    connection,
                    SessionRequest::SubmitTurn {
                        command: Box::new(turn.clone()),
                    },
                )
                .await
                .unwrap()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let result = runtime
                    .handle_session(
                        connection,
                        SessionRequest::ReconcileTurn {
                            channel: connection.fence.clone(),
                            session: receipt.fence.clone(),
                            identity: turn.identity.clone(),
                        },
                    )
                    .await
                    .unwrap()
                    .unwrap();
                if let SessionReply::TurnReconciled {
                    record: Some(record),
                    ..
                } = result
                    && let Some(completion) = record.completion
                {
                    assert_eq!(completion.status, session::TurnTerminalStatus::Completed);
                    break;
                }
                assert!(Instant::now() < deadline, "turn completion timeout");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                runtime
                    .handle_session(
                        connection,
                        SessionRequest::SubmitTurn {
                            command: Box::new(turn)
                        }
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                accepted
            );
            let database = rusqlite::Connection::open(spec.state.join("exports.sqlite3")).unwrap();
            let reserved: i64 = database
                .query_row("SELECT sum(reservation) FROM worker_sessions", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(
                reserved, 1,
                "turn completion cannot release the process slot"
            );
        }
        let replay = runtime
            .handle_session(
                connection,
                SessionRequest::ReplayStream {
                    channel: connection.fence.clone(),
                    session: receipt.fence.clone(),
                    after_sequence: 0,
                    after_digest: None,
                    limit: 8,
                },
            )
            .await
            .unwrap()
            .unwrap();
        let SessionReply::StreamReplayed {
            events,
            through_sequence,
            more,
            ..
        } = replay
        else {
            panic!("invalid replay")
        };
        assert!(!more);
        assert_eq!(through_sequence, 4);
        for event in &events {
            if let session::SessionEventKind::Text { text, .. } = &event.event {
                assert_eq!(text, "remembered:kernel-context");
            }
        }
        runtime
            .handle_session(
                connection,
                SessionRequest::CloseSession {
                    session: receipt.fence.clone(),
                },
            )
            .await
            .unwrap();
    } else if spec.action == "session-disconnect" {
        runtime.disconnect(connection);
    } else if spec.action == "session-revoke" {
        let store = runtime.store().unwrap();
        store
            .set_grant(
                &scope(),
                &command.descriptor.export.id,
                &principal(),
                &[],
                false,
                Some(command.descriptor.grant.revision),
            )
            .unwrap();
    } else {
        assert_eq!(spec.action, "session-expiry");
    }
    wait_stopped(runtime, spec, &receipt.fence).await;
}

#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_session_faults() {
    for fault in [
        "main-bootstrap",
        "guard-bootstrap",
        "main-handoff",
        "guard-handoff",
        "main-running",
        "guard-running",
    ] {
        let mut fixture = Fixture::new("persistent", true, false);
        fs::write(
            &fixture.spec.program,
            include_str!("fixtures/session_agent.py"),
        )
        .unwrap();
        fixture.spec.action = "session-fault".into();
        fixture.spec.fault = fault.into();
        let mut child = fixture.launch();
        observe(|| {
            let database =
                rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3")).unwrap();
            database
                .query_row(
                    "SELECT phase='stopped' AND reservation=0 FROM worker_sessions",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false)
        })
        .await;
        observe(|| child.try_wait().unwrap().is_some()).await;
        let result = child.wait_with_output().unwrap();
        if fault.starts_with("main-") {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(result.status.signal(), Some(libc::SIGKILL));
        } else {
            assert!(
                result.status.success(),
                "{fault}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(
            fixture.root.join("effects").exists(),
            fault.ends_with("running")
        );
        let held = fixture.root.join("session-held-effect");
        if held.exists() {
            let before = fs::metadata(&held).unwrap().len();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                fs::metadata(&held).unwrap().len(),
                before,
                "{fault}: descendant retained writable FD"
            );
        }
        // SessionStopped is durable historical state, not permission to resume.
        let store = fixture.store();
        let wire_channel = fence("restarted-session-boot");
        let scope = scope();
        let guard = || Ok(());
        let context = crate::worker_exports::execution::ExecutionChannel::verify(
            &scope,
            &wire_channel,
            &guard,
        )
        .unwrap();
        let identity: session::SessionIdentity = {
            let database =
                rusqlite::Connection::open(fixture.spec.state.join("exports.sqlite3")).unwrap();
            let raw: String = database
                .query_row("SELECT receipt FROM worker_sessions", [], |r| r.get(0))
                .unwrap();
            serde_json::from_str::<session::SessionReceipt>(&raw)
                .unwrap()
                .identity
        };
        let record = store
            .reconcile_worker_session(&context, &identity)
            .unwrap()
            .unwrap();
        assert_eq!(record.phase, session::SessionPhase::Stopped);
        assert!(record.stop.is_some());
    }
}

#[tokio::test]
#[ignore]
async fn production_browser_acceptance_runtime_worker_session() {
    for action in [
        "session-success",
        "session-disconnect",
        "session-revoke",
        "session-expiry",
    ] {
        let mut fixture = Fixture::new("persistent", true, false);
        fs::write(
            &fixture.spec.program,
            include_str!("fixtures/session_agent.py"),
        )
        .unwrap();
        fixture.spec.action = action.into();
        let mut child = fixture.launch();
        observe(|| fixture.spec.output.exists() || child.try_wait().unwrap().is_some()).await;
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read_to_string(&fixture.spec.output).unwrap(),
            "session-ok"
        );
        let calls: Vec<serde_json::Value> = fs::read_to_string(fixture.root.join("session-calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for name in ["initialize", "session/new"] {
            assert_eq!(
                calls.iter().filter(|call| call["method"] == name).count(),
                1
            );
        }
        assert!(calls.iter().all(|call| call["pid"] == calls[0]["pid"]));
        if action == "session-success" {
            assert_eq!(fs::read(fixture.root.join("effects")).unwrap(), b"x\nx\n");
        } else {
            assert!(!fixture.root.join("effects").exists());
        }
        let held = fixture.root.join("session-held-effect");
        let before = fs::metadata(&held).unwrap().len();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            fs::metadata(held).unwrap().len(),
            before,
            "setsid descendant survived session stop"
        );
    }
}
