//! SQL/opaque-observation fixtures only. Real v4 evidence belongs to TLS tests.
use super::*;
use serde_json::json;
mod mcp;
struct Fixture {
    store: Store,
    owner: String,
    device: String,
    workspace: String,
    project: String,
    identity: WorkerIdentity,
    channel: ChannelLease,
    descriptor: dunlin_relay_protocol::worker::ExportDescriptor,
    request: RemoteSessionRequest,
}
fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
impl Fixture {
    fn new() -> Self {
        Self::with_store(Store::open_in_memory().unwrap())
    }
    fn with_store(store: Store) -> Self {
        let owner = store.create_user("session-owner", "admin").unwrap();
        let (device, _) = store.create_device(&owner.id, "session-device").unwrap();
        let workspace = store.create_workspace(&owner, "Sessions").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Remote session")
            .unwrap();
        let host = store.create_host(&owner.id, &workspace.id, serde_json::from_value(json!({"name":"Session host","node_id":"worker-node","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap()).unwrap();
        let credential = store.enroll_host(&host.enrollment.ticket, serde_json::from_value(json!({"protocol_version":1,"version":"fixture","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"linux_bubblewrap","max_parallel":1})).unwrap()).unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &host.host.id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker_token(&credential.credential)
            .unwrap();
        let channel = store
            .open_worker_channel(
                &identity,
                "controller-boot",
                "worker-boot",
                "session-channel",
                now(),
                15000,
            )
            .unwrap();
        let descriptor: dunlin_relay_protocol::worker::ExportDescriptor = serde_json::from_value(json!({"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node","workspace_id":workspace.id,"host_id":host.host.id,"enrollment_epoch":1,"credential_revision":1},"export":{"id":"session-export","name":"Session export","repo_kind":"git","checkout_identity_digest":"a".repeat(64),"settings_digest":"b".repeat(64),"revision":1,"active":true},"grant":{"principal":{"kind":"human","id":owner.id},"permissions":["read","execute"],"revision":1,"export_revision":1,"active":true}})).unwrap();
        let proof =
            ObservedExport::for_test(identity.clone(), channel.clone(), descriptor.clone(), now());
        let authority = store
            .capture_remote_binding_authority(&owner.id, &workspace.id, &project.id)
            .unwrap();
        let binding = store
            .set_remote_binding(
                &owner.id,
                &workspace.id,
                &project.id,
                None,
                None,
                &host.host.id,
                "session-export",
                1,
                &proof,
                "controller-boot",
                "controller-node",
                now(),
                &authority,
            )
            .unwrap();
        let definition = serde_json::from_value(json!({"schema_version":1,"name":"Persistent agent","description":"","harness":"opencode","model":"model","mode":"mode","instructions":null,"skills":[],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        let draft = store
            .create_agent(&owner.id, &workspace.id, &project.id, definition)
            .unwrap();
        let profile = store
            .publish_agent(&owner.id, &workspace.id, &project.id, &draft.id, 1)
            .unwrap();
        Self {
            store,
            owner: owner.id,
            device: device.id,
            workspace: workspace.id,
            project: project.id,
            identity,
            channel,
            descriptor,
            request: RemoteSessionRequest {
                binding: BindingRef {
                    id: binding.id,
                    revision: binding.revision,
                },
                profile: AgentProfileRef {
                    agent_id: profile.agent_id,
                    version: profile.version,
                    digest: profile.digest,
                },
            },
        }
    }
    fn prepare(&self) -> PreparedRemoteSession {
        self.store
            .remote_binding_transaction(false, |store| {
                store.prepare_remote_session_in_transaction(
                    &self.owner,
                    &self.device,
                    &self.workspace,
                    &self.project,
                    self.request.clone(),
                    "controller-node",
                )
            })
            .unwrap()
    }
    fn proof(&self) -> ObservedExport {
        ObservedExport::for_test(
            self.identity.clone(),
            self.channel.clone(),
            self.descriptor.clone(),
            now(),
        )
    }
    fn fence(&self) -> ChannelFence {
        ChannelFence {
            channel_id: self.channel.channel_id.clone(),
            controller_boot_id: self.channel.controller_boot_id.clone(),
            worker_boot_id: self.channel.worker_boot_id.clone(),
        }
    }
    fn counts(&self) -> Vec<i64> {
        [
            "interactive_sessions",
            "interactive_turns",
            "interactive_events",
            "interactive_session_admissions",
            "interactive_session_profiles",
            "controller_remote_sessions",
        ]
        .into_iter()
        .map(|table| {
            self.store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
        .collect()
    }
}
fn attest(
    store: &Store,
    owner: &str,
    device: &str,
    identity: &WorkerIdentity,
    lease: &ChannelLease,
    fence: &ChannelFence,
) -> Result<()> {
    assert!(!store.connection.is_autocommit());
    if !store
        .active_device_user(device)?
        .is_some_and(|user| user.id == owner)
        || identity.host_id() != lease.host_id
        || lease.channel_id != fence.channel_id
        || lease.worker_boot_id != fence.worker_boot_id
        || lease.controller_boot_id != fence.controller_boot_id
    {
        return Err(denied());
    }
    Ok(())
}
fn source(store: &Store, snapshot: &RemoteSessionSnapshot) -> Result<()> {
    assert!(!store.connection.is_autocommit());
    snapshot
        .open
        .channel
        .validate()
        .map_err(|_| session_conflict())?;
    Ok(()) // Controlled test callback, never real opaque transport evidence.
}
fn caller(store: &Store, owner: &str, device: &str) -> Result<()> {
    if !store
        .active_device_user(device)?
        .is_some_and(|user| user.id == owner)
    {
        return Err(denied());
    }
    Ok(())
}
impl Fixture {
    fn active(&self) -> (String, wire::SessionReceipt) {
        let prepared = self.prepare();
        let admitted = self
            .store
            .remote_binding_transaction(false, |store| {
                store.admit_remote_session_in_transaction(
                    prepared,
                    self.proof(),
                    self.fence(),
                    "admission-key-0001",
                    "runtime",
                    now(),
                    attest,
                )
            })
            .unwrap();
        let id = admitted.admission.session.id;
        let command = self
            .store
            .remote_binding_transaction(false, |store| {
                store.dispatch_remote_session_open_in_transaction(&id, self.proof(), now(), source)
            })
            .unwrap();
        let receipt = wire::SessionReceipt {
            identity: command.identity,
            fence: wire::SessionFence {
                channel: command.channel,
                controller_session_id: id.clone(),
                worker_session_id: "worker-session-fixture".into(),
                session_epoch: 1,
            },
            budgets: command.budgets,
        };
        self.store
            .remote_binding_transaction(false, |store| {
                store.accept_remote_session_in_transaction(&id, &receipt, true, source)
            })
            .unwrap();
        (id, receipt)
    }
    fn queued(&self, id: &str) -> wire::TurnCommand {
        let context = self
            .store
            .team_project_context(&self.workspace, &self.project, &self.owner)
            .unwrap();
        self.store
            .interactive_journal()
            .enqueue(&context, id, "first-prompt", "prompt")
            .unwrap();
        let command = self
            .store
            .remote_binding_transaction(false, |store| {
                store.claim_remote_session_turn_in_transaction(id, self.proof(), now(), source)
            })
            .unwrap()
            .unwrap();
        self.store
            .remote_binding_transaction(false, |store| {
                store.dispatch_remote_session_turn_in_transaction(
                    id,
                    &command.identity.turn_id,
                    self.proof(),
                    now(),
                    source,
                )
            })
            .unwrap()
    }
}
fn event(
    receipt: &wire::SessionReceipt,
    sequence: u64,
    previous: Option<&wire::SessionEvent>,
    kind: wire::SessionEventKind,
) -> wire::SessionEvent {
    let mut event = wire::SessionEvent {
        session: receipt.fence.clone(),
        sequence,
        previous: previous.map(|event| wire::StreamPosition {
            session: event.session.clone(),
            sequence: event.sequence,
            digest: event.digest.clone(),
        }),
        digest: "0".repeat(64),
        event: kind,
    };
    event.digest = wire::session_event_digest(&event).unwrap();
    event
}

#[test]
fn stream_gap_late_sql_failure_duplicates_and_ack_share_one_journal_projection_cut() {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let receipt = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: session.fence.clone(),
            turn_id: command.identity.turn_id.clone(),
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    let text = event(
        &session,
        1,
        None,
        wire::SessionEventKind::Text {
            turn: receipt.fence.clone(),
            kind: wire::TextKind::Text,
            text: "durable remote output".into(),
        },
    );
    let terminal = event(
        &session,
        2,
        Some(&text),
        wire::SessionEventKind::TurnCompleted {
            receipt: receipt.clone(),
            status: wire::TurnTerminalStatus::Completed,
        },
    );
    let gap = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &terminal, source, caller)
        })
        .unwrap();
    assert_eq!(
        gap,
        RemoteStreamCut::Gap {
            after_sequence: 0,
            after_digest: None
        }
    );
    fixture
        .store
        .connection
        .execute_batch("CREATE TABLE sibling(value INTEGER CHECK(value>0));")
        .unwrap();
    let baseline = fixture.counts();
    let result: Result<()> = fixture.store.remote_binding_transaction(false, |store| {
        store.record_remote_session_event_in_transaction(&id, &text, source, caller)?;
        store.record_remote_session_event_in_transaction(&id, &terminal, source, caller)?;
        store
            .connection
            .execute("INSERT INTO sibling VALUES(0)", [])?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(fixture.counts(), baseline);
    assert_eq!(
        fixture.store.remote_session_snapshot(&id).unwrap().sequence,
        0
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT count(*) FROM controller_remote_session_stream",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &text, source, caller)?;
            store.record_remote_session_event_in_transaction(&id, &terminal, source, caller)
        })
        .unwrap();
    // A stream chunk may precede the RPC acceptance. Its exact receipt is
    // retained; a later acceptance cannot restart a terminal Journal turn.
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.accept_remote_session_turn_in_transaction(&id, &receipt, source)
        })
        .unwrap();
    let before = fixture.counts();
    let duplicate = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &text, source, |_, _, _| {
                panic!("duplicate ACK must not require renewed execution authority")
            })
        })
        .unwrap();
    assert!(matches!(
        duplicate,
        RemoteStreamCut::Persisted {
            sequence: 2,
            duplicate: true,
            ..
        }
    ));
    assert_eq!(fixture.counts(), before);
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    assert_eq!(
        fixture
            .store
            .interactive_journal()
            .snapshot(&context, &id)
            .unwrap()
            .1[0]
            .phase,
        dunlin_session_runtime::TurnPhase::Completed
    );
    assert_eq!(
        fixture
            .store
            .interactive_journal()
            .replay(&context, &id, 0)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "agent_update")
            .count(),
        1
    );
    let ack = wire::SessionControllerMessage::AckStream {
        channel: fixture.fence(),
        session: session.fence.clone(),
        through_sequence: 2,
        event_digest: terminal.digest.clone(),
    };
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .confirm_remote_session_ack_in_transaction(&id, &ack, |_, _, _| Err(denied())))
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .remote_session_snapshot(&id)
            .unwrap()
            .confirmed_ack,
        0
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.confirm_remote_session_ack_in_transaction(
                &id,
                &ack,
                |store, snapshot, request| {
                    source(store, snapshot)?;
                    assert_eq!(request, &ack);
                    Ok(())
                },
            )
        })
        .unwrap();
    assert_eq!(
        fixture
            .store
            .remote_session_snapshot(&id)
            .unwrap()
            .confirmed_ack,
        2
    );
    assert!(fixture.store.connection.query_row("SELECT event IS NULL FROM controller_remote_session_stream WHERE session=?1 AND sequence=1",[&id],|r|r.get::<_,bool>(0)).unwrap());
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(&id, &text, source, caller))
            .is_ok()
    );
}

#[test]
fn unknown_keeps_reservation_blocks_next_claim_and_only_exact_stop_releases_it() {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    let queued = fixture
        .store
        .interactive_journal()
        .enqueue(&context, &id, "second-prompt", "not dispatched")
        .unwrap();
    let receipt = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets,
        fence: wire::TurnFence {
            session: session.fence.clone(),
            turn_id: command.identity.turn_id,
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    let unknown = event(
        &session,
        1,
        None,
        wire::SessionEventKind::TurnCompleted {
            receipt,
            status: wire::TurnTerminalStatus::Unknown,
        },
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &unknown, source, caller)
        })
        .unwrap();
    let snapshot = fixture.store.remote_session_snapshot(&id).unwrap();
    assert_eq!(snapshot.phase, "interrupted");
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .claim_remote_session_turn_in_transaction(&id, fixture.proof(), now(), source))
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT phase FROM interactive_turns WHERE id=?1",
                [queued],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "queued"
    );
    let stopped = event(
        &session,
        2,
        Some(&unknown),
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(
                    &id,
                    &stopped,
                    |_, _| Err(denied()),
                    caller
                ))
            .is_err()
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &stopped, source, |_, _, _| {
                panic!("negative stop must survive revoked caller authority")
            })
        })
        .unwrap();
    assert_eq!(
        fixture.store.remote_session_snapshot(&id).unwrap().phase,
        "stopped"
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

const DROP_SESSION_SCHEMA: &str = "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; PRAGMA user_version=35;";
#[test]
fn migration_36_preserves_existing_v3_command_and_project_authority_without_session_admission() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let fixture = Fixture::with_store(Store::open(&path).unwrap());
    let request = crate::store::remote_bindings::resolver::execution::RemoteExecutionRequest {
        intent_key: "legacy-execution-0001".into(),
        binding: fixture.request.binding.clone(),
        profile: fixture.request.profile.clone(),
        input: "retained original intent".into(),
        budgets: dunlin_relay_protocol::worker::ExecutionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            lease_duration_ms: 10000,
        },
    };
    let prepared = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.prepare_remote_execution_in_transaction(
                &fixture.owner,
                &fixture.device,
                &fixture.workspace,
                &fixture.project,
                request,
                "controller-node",
            )
        })
        .unwrap();
    let accepted = fixture
        .store
        .admit_remote_execution(prepared, fixture.proof(), fixture.fence(), now(), attest)
        .unwrap();
    let original = fixture
        .store
        .remote_execution_snapshot(&accepted.identity.attempt_id, |_, _| Ok(()))
        .unwrap()
        .command;
    let before_binding = fixture
        .store
        .remote_bindings(&fixture.owner, &fixture.workspace, &fixture.project)
        .unwrap();
    // The existing v3 row and team authority are historical schema35 data.
    fixture
        .store
        .connection
        .execute_batch(DROP_SESSION_SCHEMA)
        .unwrap();
    let owner = fixture.owner.clone();
    let workspace = fixture.workspace.clone();
    let project = fixture.project.clone();
    drop(fixture);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    assert_eq!(
        store
            .remote_execution_snapshot(&accepted.identity.attempt_id, |_, _| Ok(()))
            .unwrap()
            .command,
        original
    );
    assert_eq!(
        store.remote_bindings(&owner, &workspace, &project).unwrap(),
        before_binding
    );
    for table in [
        "controller_remote_sessions",
        "controller_remote_session_turns",
        "controller_remote_session_stream",
    ] {
        assert_eq!(
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn migration_36_last_table_collision_rolls_back_both_prior_tables_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let fixture = Fixture::with_store(Store::open(&path).unwrap());
    fixture
        .store
        .connection
        .execute_batch(DROP_SESSION_SCHEMA)
        .unwrap();
    fixture.store.connection.execute_batch("CREATE TABLE controller_remote_session_stream(sentinel TEXT);INSERT INTO controller_remote_session_stream VALUES('preserve');").unwrap();
    drop(fixture);
    assert!(Store::open(&path).is_err());
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        35
    );
    assert_eq!(raw.query_row("SELECT count(*) FROM sqlite_master WHERE name IN('controller_remote_sessions','controller_remote_session_turns')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        raw.query_row(
            "SELECT sentinel FROM controller_remote_session_stream",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preserve"
    );
    raw.execute_batch("DROP TABLE controller_remote_session_stream;")
        .unwrap();
    drop(raw);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}

#[test]
fn recovery_quarantines_sent_turn_and_stop_reconcile_releases_only_after_source_proof_without_gap_ack()
 {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    let second = fixture
        .store
        .interactive_journal()
        .enqueue(&context, &id, "second-prompt", "keep queued")
        .unwrap();
    assert_eq!(fixture.store.recover_remote_sessions().unwrap(), 1);
    assert_eq!(fixture.store.recover_remote_sessions().unwrap(), 0);
    let snapshot = fixture.store.remote_session_snapshot(&id).unwrap();
    assert_eq!(snapshot.phase, "interrupted");
    assert!(snapshot.closing);
    let turns = fixture
        .store
        .interactive_journal()
        .snapshot(&context, &id)
        .unwrap()
        .1;
    assert_eq!(
        turns
            .iter()
            .find(|turn| turn.id == command.identity.turn_id)
            .unwrap()
            .phase,
        dunlin_session_runtime::TurnPhase::Unknown
    );
    assert_eq!(
        turns.iter().find(|turn| turn.id == second).unwrap().phase,
        dunlin_session_runtime::TurnPhase::Queued
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .remote_session_queue_allowed_in_transaction(&id, &fixture.owner, source))
            .is_err()
    );
    let record = wire::SessionRecord {
        receipt: session,
        phase: wire::SessionPhase::Stopped,
        active_turn: None,
        stop: Some(wire::SessionStop {
            reason: wire::SessionStopReason::Unknown,
            final_sequence: 2,
            event_digest: "f".repeat(64),
        }),
    };
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .reconcile_remote_session_stop_in_transaction(
                    &id,
                    &record,
                    |_, _, _| Err(denied())
                ))
            .is_err()
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.reconcile_remote_session_stop_in_transaction(
                &id,
                &record,
                |store, snapshot, _| source(store, snapshot),
            )
        })
        .unwrap();
    let snapshot = fixture.store.remote_session_snapshot(&id).unwrap();
    assert_eq!(snapshot.phase, "stopped");
    assert_eq!(snapshot.sequence, 0);
    assert_eq!(snapshot.ack_sequence, 0);
    assert_eq!(snapshot.confirmed_ack, 0);
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let mut changed = record.clone();
    changed.stop.as_mut().unwrap().event_digest = "a".repeat(64);
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .reconcile_remote_session_stop_in_transaction(
                    &id,
                    &changed,
                    |store, snapshot, _| source(store, snapshot)
                ))
            .is_err()
    );
    assert_eq!(
        fixture.store.remote_session_cleanup_ids().unwrap(),
        vec![id]
    );
}

#[test]
fn late_failure_rolls_back_journal_profile_admission_and_remote_open_outbox() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let baseline = fixture.counts();
    fixture
        .store
        .connection
        .execute_batch("CREATE TABLE sibling(value INTEGER CHECK(value>0));")
        .unwrap();
    let result: Result<()> = fixture.store.remote_binding_transaction(false, |store| {
        let admitted = store.admit_remote_session_in_transaction(
            prepared,
            fixture.proof(),
            fixture.fence(),
            "admission-key-0001",
            "runtime",
            now(),
            attest,
        )?;
        assert!(admitted.admission.lease.is_some());
        assert!(admitted.command.is_some());
        store
            .connection
            .execute("INSERT INTO sibling VALUES(0)", [])?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(fixture.counts(), baseline);
    assert_eq!(
        fixture
            .store
            .connection
            .query_row("SELECT count(*) FROM interactive_ahp_actions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn identical_admission_is_historical_without_second_lease_command_or_capacity() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let first = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "admission-key-0001",
                "runtime",
                now(),
                attest,
            )
        })
        .unwrap();
    let command = first.command.unwrap();
    assert!(first.admission.lease.is_some());
    let baseline = fixture.counts();
    let prepared = fixture.prepare();
    let again = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "admission-key-0001",
                "other-runtime",
                now(),
                attest,
            )
        })
        .unwrap();
    assert_eq!(again.admission.session.id, first.admission.session.id);
    assert!(again.admission.lease.is_none());
    assert!(again.command.is_none());
    assert_eq!(fixture.counts(), baseline);
    let snapshot = fixture
        .store
        .remote_session_snapshot(&first.admission.session.id)
        .unwrap();
    assert_eq!(snapshot.open, command);
    assert_eq!(snapshot.lease.instance, "runtime");
    let prepared = fixture.prepare();
    let receipt = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.lookup_remote_session_admission_in_transaction(
                &prepared,
                "admission-key-0001",
                |store, owner, device| {
                    assert_eq!(store.active_device_user(device)?.unwrap().id, owner);
                    Ok(())
                },
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(receipt.id, first.admission.session.id);
    assert_eq!(fixture.counts(), baseline);
    let prepared = fixture.prepare();
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store.admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "admission-key-0002",
                "runtime",
                now(),
                attest
            ))
            .is_err()
    );
    assert_eq!(fixture.counts(), baseline);
}

#[test]
fn autocommit_metadata_only_and_denied_v4_producer_never_create_a_session() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    assert!(
        fixture
            .store
            .admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "admission-key-0001",
                "runtime",
                now(),
                attest
            )
            .is_err()
    );
    let prepared = fixture.prepare();
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store.admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "admission-key-0001",
                "runtime",
                now(),
                |_, _, _, _, _, _| Err(denied())
            ))
            .is_err()
    );
    let prepared = fixture.prepare();
    let mut descriptor = fixture.descriptor.clone();
    descriptor.grant.permissions = vec![ExportPermission::Read];
    let proof = ObservedExport::for_test(
        fixture.identity.clone(),
        fixture.channel.clone(),
        descriptor,
        now(),
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store.admit_remote_session_in_transaction(
                prepared,
                proof,
                fixture.fence(),
                "admission-key-0001",
                "runtime",
                now(),
                attest
            ))
            .is_err()
    );
    assert_eq!(fixture.counts(), vec![0; 6]);
}

#[test]
fn migration_37_preserves_v36_open_bytes_and_distinguishes_only_proven_unsent_reservations() {
    for phase in ["pending", "active", "interrupted"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let fixture = Fixture::with_store(Store::open(&path).unwrap());
        let id = if phase == "pending" {
            let prepared = fixture.prepare();
            fixture
                .store
                .remote_binding_transaction(false, |store| {
                    store.admit_remote_session_in_transaction(
                        prepared,
                        fixture.proof(),
                        fixture.fence(),
                        "admission-key-0001",
                        "runtime",
                        now(),
                        attest,
                    )
                })
                .unwrap()
                .admission
                .session
                .id
        } else {
            let (id, _) = fixture.active();
            if phase == "interrupted" {
                fixture
                    .store
                    .remote_binding_transaction(false, |store| {
                        store.interrupt_remote_session_in_transaction(&id, source)
                    })
                    .unwrap();
            }
            id
        };
        let before = fixture.store.remote_session_snapshot(&id).unwrap().open;
        fixture.store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; ALTER TABLE controller_remote_sessions DROP COLUMN stop_record;ALTER TABLE controller_remote_sessions DROP COLUMN dispatched;PRAGMA user_version=36;").unwrap();
        drop(fixture);
        let store = Store::open(&path).unwrap();
        let snapshot = store.remote_session_snapshot(&id).unwrap();
        assert_eq!(snapshot.open, before);
        assert_eq!(snapshot.phase, phase);
        assert_eq!(snapshot.dispatched, phase != "pending");
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        store.recover_remote_sessions().unwrap();
        let snapshot = store.remote_session_snapshot(&id).unwrap();
        assert_eq!(
            snapshot.phase,
            if phase == "pending" {
                "stopped"
            } else {
                "interrupted"
            }
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            if phase == "pending" { 0 } else { 1 }
        );
    }
}

#[test]
fn migration_37_column_collision_rolls_back_first_column_and_retains_prior_stop_column() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let fixture = Fixture::with_store(Store::open(&path).unwrap());
    fixture.store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; ALTER TABLE controller_remote_sessions DROP COLUMN stop_record;ALTER TABLE controller_remote_sessions DROP COLUMN dispatched;ALTER TABLE controller_remote_sessions ADD COLUMN stop_record TEXT;PRAGMA user_version=36;").unwrap();
    drop(fixture);
    assert!(Store::open(&path).is_err());
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        36
    );
    assert_eq!(raw.query_row("SELECT count(*) FROM pragma_table_info('controller_remote_sessions') WHERE name='dispatched'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(raw.query_row("SELECT count(*) FROM pragma_table_info('controller_remote_sessions') WHERE name='stop_record'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    raw.execute_batch("ALTER TABLE controller_remote_sessions DROP COLUMN stop_record;")
        .unwrap();
    drop(raw);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}

#[test]
fn ahp_cancellation_committed_after_claim_fences_submit_before_any_worker_frame() {
    let fixture = Fixture::new();
    let (id, _) = fixture.active();
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    fixture
        .store
        .interactive_journal()
        .enqueue(&context, &id, "first-prompt", "prompt")
        .unwrap();
    let command = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.claim_remote_session_turn_in_transaction(&id, fixture.proof(), now(), source)
        })
        .unwrap()
        .unwrap();
    let action=serde_json::from_value(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/turnCancelled","turnId":command.identity.turn_id,"duration":0}})).unwrap();
    let cancellation = fixture
        .store
        .interactive_journal()
        .dispatch_action(
            &context,
            &id,
            &fixture.device,
            "fixture-client",
            &action,
            dunlin_session_runtime::DispatchAdmission::default(),
        )
        .unwrap();
    assert!(cancellation.rejection_reason.is_none());
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .dispatch_remote_session_turn_in_transaction(
                    &id,
                    &command.identity.turn_id,
                    fixture.proof(),
                    now(),
                    source
                ))
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT phase FROM controller_remote_session_turns WHERE session=?1 AND turn=?2",
                params![id, command.identity.turn_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .remote_session_queue_allowed_in_transaction(&id, &fixture.owner, source))
            .is_err()
    );
    assert!(
        fixture
            .store
            .remote_session_activity(&id)
            .unwrap()
            .cancellation
    );
}

#[test]
fn lost_reply_resolution_requires_persisted_receipt_current_authority_and_live_journal_lease() {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let resolve = || {
        fixture.store.remote_binding_transaction(false, |store| {
            store.resolve_remote_turn_receipt_in_transaction(&id, &command.identity, source)
        })
    };
    assert!(
        resolve().unwrap().is_none(),
        "a sent intent alone is not acceptance"
    );
    let receipt = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: session.fence.clone(),
            turn_id: command.identity.turn_id.clone(),
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    let text = event(
        &session,
        1,
        None,
        wire::SessionEventKind::Text {
            turn: receipt.fence.clone(),
            kind: wire::TextKind::Text,
            text: "proof before reply".into(),
        },
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &text, source, caller)
        })
        .unwrap();
    let counts = fixture.counts();
    let clock = fixture
        .store
        .interactive_journal()
        .channel_clock()
        .unwrap()
        .sequence;
    assert_eq!(resolve().unwrap(), Some(receipt));
    assert_eq!(counts, fixture.counts());
    assert_eq!(
        clock,
        fixture
            .store
            .interactive_journal()
            .channel_clock()
            .unwrap()
            .sequence
    );
    let mut wrong = command.identity.clone();
    wrong.fingerprint = "f".repeat(64);
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .resolve_remote_turn_receipt_in_transaction(&id, &wrong, source))
            .is_err()
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .resolve_remote_turn_receipt_in_transaction(&id, &command.identity, |_, _| Err(
                    denied()
                )))
            .is_err()
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.interrupt_remote_session_in_transaction(&id, source)
        })
        .unwrap();
    let counts = fixture.counts();
    assert!(resolve().is_err());
    assert_eq!(counts, fixture.counts());
}

#[test]
fn cancellation_fences_renewals_and_negative_terminal_after_local_interrupt_preserves_stop_boundary()
 {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let receipt = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: session.fence.clone(),
            turn_id: command.identity.turn_id.clone(),
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    let text = event(
        &session,
        1,
        None,
        wire::SessionEventKind::Text {
            turn: receipt.fence.clone(),
            kind: wire::TextKind::Text,
            text: "before cancellation".into(),
        },
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &text, source, caller)
        })
        .unwrap();
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    let action=serde_json::from_value(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/turnCancelled","turnId":command.identity.turn_id,"duration":0}})).unwrap();
    assert!(
        fixture
            .store
            .interactive_journal()
            .dispatch_action(
                &context,
                &id,
                &fixture.device,
                "cancel-client",
                &action,
                dunlin_session_runtime::DispatchAdmission::default()
            )
            .unwrap()
            .rejection_reason
            .is_none()
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store.renew_remote_session_in_transaction(
                &id,
                fixture.proof(),
                now(),
                source
            ))
            .is_err()
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .renew_remote_session_turn_in_transaction(
                    &id,
                    &command.identity.turn_id,
                    fixture.proof(),
                    now(),
                    source
                ))
            .is_err()
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.interrupt_remote_session_in_transaction(&id, source)
        })
        .unwrap();
    let clock = fixture
        .store
        .interactive_journal()
        .channel_clock()
        .unwrap()
        .sequence;
    let unknown = event(
        &session,
        2,
        Some(&text),
        wire::SessionEventKind::TurnCompleted {
            receipt: receipt.clone(),
            status: wire::TurnTerminalStatus::Unknown,
        },
    );
    let mut wrong = receipt;
    wrong.identity.fingerprint = "f".repeat(64);
    let invalid = event(
        &session,
        2,
        Some(&text),
        wire::SessionEventKind::TurnCompleted {
            receipt: wrong,
            status: wire::TurnTerminalStatus::Unknown,
        },
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(
                    &id,
                    &invalid,
                    source,
                    |_, _, _| panic!("negative receipt must not renew caller authority")
                ))
            .is_err()
    );
    assert_eq!(
        fixture.store.remote_session_snapshot(&id).unwrap().sequence,
        1
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &unknown, source, |_, _, _| {
                panic!("negative terminal must not require a live caller")
            })
        })
        .unwrap();
    assert_eq!(
        clock,
        fixture
            .store
            .interactive_journal()
            .channel_clock()
            .unwrap()
            .sequence
    );
    assert_eq!(
        fixture.store.remote_session_stop_observation(&id).unwrap(),
        Some(false)
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let stopped = event(
        &session,
        3,
        Some(&unknown),
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &stopped, source, |_, _, _| {
                panic!("stop must not require a live caller")
            })
        })
        .unwrap();
    assert_eq!(
        fixture.store.remote_session_stop_observation(&id).unwrap(),
        Some(true)
    );
    assert_eq!(
        fixture
            .store
            .connection
            .query_row(
                "SELECT reservation FROM controller_remote_sessions WHERE session=?1",
                [&id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let snapshot = fixture.store.remote_session_snapshot(&id).unwrap();
    assert_eq!(snapshot.ack_sequence, 3);
    let (_, turns) = fixture
        .store
        .interactive_journal()
        .snapshot(&context, &id)
        .unwrap();
    assert_eq!(turns[0].phase, dunlin_session_runtime::TurnPhase::Unknown);
}

#[test]
fn mcp_session_profile_is_rejected_before_durable_admission() {
    let mut f = Fixture::new();
    let mut definition = f
        .store
        .agent_version(
            &f.owner,
            &f.workspace,
            &f.project,
            &f.request.profile.agent_id,
            f.request.profile.version,
        )
        .unwrap()
        .unwrap()
        .definition;
    definition.schema_version = 2;
    definition.mcp.push(dunlin_domain::agent::AgentMcpTool {
        provider_id: "work".into(),
        name: "lookup".into(),
        definition_digest: "a".repeat(64),
    });
    let draft = f
        .store
        .create_agent(&f.owner, &f.workspace, &f.project, definition)
        .unwrap();
    let version = f
        .store
        .publish_agent(
            &f.owner,
            &f.workspace,
            &f.project,
            &draft.id,
            draft.revision,
        )
        .unwrap();
    f.request.profile = AgentProfileRef {
        agent_id: version.agent_id,
        version: version.version,
        digest: version.digest,
    };
    let result = f.store.remote_binding_transaction(false, |store| {
        store.prepare_remote_session_in_transaction(
            &f.owner,
            &f.device,
            &f.workspace,
            &f.project,
            f.request.clone(),
            "controller-node",
        )
    });
    assert!(matches!(result,Err(StoreError::Limit(message)) if message.contains("MCP")));
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT count(*) FROM controller_remote_sessions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn reconciled_stop_drains_only_the_anchored_chain_without_projection_or_early_ack() {
    let fixture = Fixture::new();
    let (id, session) = fixture.active();
    let command = fixture.queued(&id);
    let receipt = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: session.fence.clone(),
            turn_id: command.identity.turn_id.clone(),
            turn_index: 1,
            turn_epoch: 1,
        },
    };
    let text = event(
        &session,
        1,
        None,
        wire::SessionEventKind::Text {
            turn: receipt.fence.clone(),
            kind: wire::TextKind::Text,
            text: "late text must never reach the user".into(),
        },
    );
    let terminal = event(
        &session,
        2,
        Some(&text),
        wire::SessionEventKind::TurnCompleted {
            receipt,
            status: wire::TurnTerminalStatus::Completed,
        },
    );
    let stopped = event(
        &session,
        3,
        Some(&terminal),
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Closed,
        },
    );
    let record = wire::SessionRecord {
        receipt: session.clone(),
        phase: wire::SessionPhase::Stopped,
        active_turn: None,
        stop: Some(wire::SessionStop {
            reason: wire::SessionStopReason::Closed,
            final_sequence: 3,
            event_digest: stopped.digest.clone(),
        }),
    };
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.reconcile_remote_session_stop_in_transaction(
                &id,
                &record,
                |store, snapshot, _| source(store, snapshot),
            )
        })
        .unwrap();
    assert_eq!(
        fixture.store.remote_session_cleanup_ids().unwrap(),
        vec![id.clone()]
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(
                    &id,
                    &text,
                    |_, _| Err(denied()),
                    caller
                ))
            .is_err()
    );
    for page in [&text, &terminal] {
        fixture
            .store
            .remote_binding_transaction(false, |store| {
                store.record_remote_session_event_in_transaction(&id, page, source, |_, _, _| {
                    panic!("late cleanup must not project or authorize execution")
                })
            })
            .unwrap();
        let snapshot = fixture.store.remote_session_snapshot(&id).unwrap();
        assert_eq!(snapshot.phase, "stopped");
        assert_eq!(snapshot.sequence, page.sequence);
        assert_eq!(
            snapshot.ack_sequence, 0,
            "unverified prefix must not be acknowledged"
        );
    }
    let wrong = event(
        &session,
        3,
        Some(&terminal),
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(&id, &wrong, source, caller))
            .is_err()
    );
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.record_remote_session_event_in_transaction(&id, &stopped, source, |_, _, _| {
                panic!("stop cleanup must not project")
            })
        })
        .unwrap();
    assert_eq!(
        fixture
            .store
            .remote_session_snapshot(&id)
            .unwrap()
            .ack_sequence,
        3
    );
    let extra = event(
        &session,
        4,
        Some(&stopped),
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Closed,
        },
    );
    assert!(
        fixture
            .store
            .remote_binding_transaction(false, |store| store
                .record_remote_session_event_in_transaction(&id, &extra, source, caller))
            .is_err()
    );
    let context = fixture
        .store
        .team_project_context(&fixture.workspace, &fixture.project, &fixture.owner)
        .unwrap();
    let (_, turns) = fixture
        .store
        .interactive_journal()
        .snapshot(&context, &id)
        .unwrap();
    assert_eq!(turns[0].phase, dunlin_session_runtime::TurnPhase::Unknown);
    let chat = fixture
        .store
        .interactive_journal()
        .projected_chat(&context, &id)
        .unwrap();
    assert!(
        !serde_json::to_string(&chat)
            .unwrap()
            .contains("late text must never reach the user")
    );
    assert!(fixture.store.connection.query_row("SELECT event IS NULL FROM controller_remote_session_stream WHERE session=?1 AND sequence=1",[&id],|row|row.get::<_,bool>(0)).unwrap());
    let ack = wire::SessionControllerMessage::AckStream {
        channel: fixture.fence(),
        session: session.fence,
        through_sequence: 3,
        event_digest: stopped.digest,
    };
    fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.confirm_remote_session_ack_in_transaction(&id, &ack, |store, snapshot, _| {
                source(store, snapshot)
            })
        })
        .unwrap();
    assert!(
        fixture
            .store
            .remote_session_cleanup_ids()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture
            .store
            .remote_session_snapshot(&id)
            .unwrap()
            .confirmed_ack,
        3
    );
}
