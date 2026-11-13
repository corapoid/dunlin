use super::*;
use crate::worker_exports::{
    CapturedCheckout, ExportSummary, GrantSummary, policy::PolicySnapshot,
};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use dunlin_relay_protocol::worker::{ChannelFence, ExecutionBudgets};
const POLICY:&[u8]=br#"{"schema_version":1,"harnesses":["codex"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1}"#;
struct Fixture {
    directory: tempfile::TempDir,
    scope: ExportScope,
    export: ExportSummary,
    grant: GrantSummary,
}
fn principal() -> ExportPrincipal {
    ExportPrincipal {
        kind: PrincipalKind::Human,
        id: "human".into(),
    }
}
fn channel(boot: &str) -> ChannelFence {
    ChannelFence {
        channel_id: format!("channel-{boot}"),
        controller_boot_id: "controller".into(),
        worker_boot_id: boot.into(),
    }
}
fn allow() -> Result<()> {
    Ok(())
}
fn broker_limits() -> wire::broker::BrokerLimits {
    wire::broker::BrokerLimits {
        max_pending: 8,
        max_reply_bytes: 256 * 1024,
    }
}
#[test]
fn broker_session_requires_live_offer_and_explicit_own_operator_policy() {
    let legacy = Fixture::new();
    let store = legacy.store();
    let ch = channel("boot");
    let offer = broker_limits();
    let context = ExecutionChannel::verify_broker(&legacy.scope, &ch, &allow, &offer).unwrap();
    assert!(
        store
            .open_broker_session(
                &context,
                &legacy.broker_open("no-policy"),
                legacy.validated(&store)
            )
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_sessions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let fixture = Fixture::broker();
    let store = fixture.store();
    let command = fixture.broker_open("accepted");
    let legacy_context = ExecutionChannel::verify(&fixture.scope, &ch, &allow).unwrap();
    assert!(
        store
            .open_broker_session(&legacy_context, &command, fixture.validated(&store))
            .is_err()
    );
    assert!(
        store
            .open_session(&legacy_context, &command.open, fixture.validated(&store))
            .is_err()
    );
    let context = ExecutionChannel::verify_broker(&fixture.scope, &ch, &allow, &offer).unwrap();
    let admission = store
        .open_broker_session(&context, &command, fixture.validated(&store))
        .unwrap();
    let lease = admission.lease.unwrap();
    command.validate_receipt(&admission.receipt).unwrap();
    store.validate_worker_session(&context, &lease).unwrap();
    assert!(
        store
            .validate_worker_session(&legacy_context, &lease)
            .is_err()
    );
    let mut small = offer.clone();
    small.max_pending = 1;
    let small_context =
        ExecutionChannel::verify_broker(&fixture.scope, &ch, &allow, &small).unwrap();
    assert!(
        store
            .validate_worker_session(&small_context, &lease)
            .is_err()
    );
    let raw: String = store
        .connection
        .query_row("SELECT pins FROM worker_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw).unwrap()["broker"]["max_pending"],
        8
    );
}
#[test]
fn broker_session_sql_rollback_reopen_and_changed_retry_never_mint_another_lease() {
    let fixture = Fixture::broker();
    let store = fixture.store();
    let ch = channel("boot");
    let offer = broker_limits();
    let context = ExecutionChannel::verify_broker(&fixture.scope, &ch, &allow, &offer).unwrap();
    let command = fixture.broker_open("rollback");
    store.connection.execute_batch("CREATE TRIGGER fail_broker_open BEFORE INSERT ON worker_sessions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        store
            .open_broker_session(&context, &command, fixture.validated(&store))
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT session_epoch FROM worker_session_meta", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_broker_open;")
        .unwrap();
    let original = store
        .open_broker_session(&context, &command, fixture.validated(&store))
        .unwrap();
    assert_eq!(original.receipt.fence.session_epoch, 1);
    assert!(original.lease.is_some());
    drop(store);
    let store = fixture.store();
    let retry = store
        .open_broker_session(&context, &command, fixture.validated(&store))
        .unwrap();
    assert_eq!(retry.receipt, original.receipt);
    assert!(retry.lease.is_none());
    let mut changed = command.clone();
    changed.broker_limits.max_pending = 1;
    changed.open.identity.fingerprint =
        wire::broker::broker_session_open_fingerprint(&changed).unwrap();
    assert!(
        store
            .open_broker_session(&context, &changed, fixture.validated(&store))
            .is_err()
    );
    let mut downgrade = command.open.clone();
    downgrade.definition.mcp.clear();
    downgrade.profile.digest =
        crate::worker_execution::profile_digest(&downgrade.definition).unwrap();
    downgrade.identity.fingerprint = wire::session_open_fingerprint(&downgrade).unwrap();
    assert!(
        store
            .open_session(&context, &downgrade, fixture.validated(&store))
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_sessions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let raw: String = store
        .connection
        .query_row("SELECT open_payload FROM worker_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw).unwrap()["protocol_version"],
        5
    );
}
#[test]
fn broker_lease_never_borrows_revoked_or_regranted_execute_authority() {
    let fixture = Fixture::broker();
    let store = fixture.store();
    let ch = channel("boot");
    let offer = broker_limits();
    let context = ExecutionChannel::verify_broker(&fixture.scope, &ch, &allow, &offer).unwrap();
    let command = fixture.broker_open("revoked");
    let lease = store
        .open_broker_session(&context, &command, fixture.validated(&store))
        .unwrap()
        .lease
        .unwrap();
    let revoked = store
        .set_grant(
            &fixture.scope,
            &fixture.export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            false,
            Some(fixture.grant.revision),
        )
        .unwrap();
    assert!(store.validate_worker_session(&context, &lease).is_err());
    store
        .set_grant(
            &fixture.scope,
            &fixture.export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            Some(revoked.revision),
        )
        .unwrap();
    assert!(store.validate_worker_session(&context, &lease).is_err());
}
#[test]
fn legacy_session_pins_and_lease_stay_legacy_even_on_broker_capable_channel() {
    let fixture = Fixture::broker();
    let store = fixture.store();
    let ch = channel("boot");
    let offer = broker_limits();
    let context = ExecutionChannel::verify_broker(&fixture.scope, &ch, &allow, &offer).unwrap();
    let command = fixture.open("legacy");
    let admission = store
        .open_session(&context, &command, fixture.validated(&store))
        .unwrap();
    assert!(admission.lease.as_ref().unwrap().broker.is_none());
    let raw: String = store
        .connection
        .query_row("SELECT pins FROM worker_sessions", [], |r| r.get(0))
        .unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap()
            .get("broker")
            .is_none()
    );
    let legacy_context = ExecutionChannel::verify(&fixture.scope, &ch, &allow).unwrap();
    store
        .validate_worker_session(&legacy_context, &admission.lease.unwrap())
        .unwrap();
}
impl Fixture {
    fn new() -> Self {
        Self::with_policy(POLICY)
    }
    fn with_policy(raw: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let scope = ExportScope {
            controller_node_id: "controller".into(),
            worker_node_id: "worker".into(),
            workspace_id: "workspace".into(),
            host_id: "host".into(),
            enrollment_epoch: 1,
            credential_revision: 1,
        };
        let path = directory.path().canonicalize().unwrap().join("policy");
        std::fs::write(&path, raw).unwrap();
        let policy = PolicySnapshot::load(&path).unwrap();
        let root = directory.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let checkout = CapturedCheckout::capture(&root, policy.digest(), None).unwrap();
        let store = WorkerStore::open(&directory.path().join("state"), scope.clone()).unwrap();
        let export = store
            .register_captured_with_policy(&scope, "Session", &checkout, Some(&policy))
            .unwrap();
        let grant = store
            .set_grant(
                &scope,
                &export.id,
                &principal(),
                &[ExportPermission::Read, ExportPermission::Execute],
                true,
                None,
            )
            .unwrap();
        Self {
            directory,
            scope,
            export,
            grant,
        }
    }
    fn store(&self) -> WorkerStore {
        WorkerStore::open_existing(&self.directory.path().join("state"), self.scope.clone())
            .unwrap()
    }
    fn broker() -> Self {
        let mut policy: serde_json::Value = serde_json::from_slice(POLICY).unwrap();
        policy["schema_version"] = serde_json::json!(2);
        policy["mcp_broker"] = serde_json::json!(true);
        Self::with_policy(&serde_json::to_vec(&policy).unwrap())
    }
    fn broker_open(&self, id: &str) -> wire::broker::OpenBrokerSessionCommand {
        let mut open = self.open(id);
        open.definition.schema_version = 2;
        open.definition
            .mcp
            .push(dunlin_domain::agent::AgentMcpTool {
                provider_id: "work".into(),
                name: "lookup".into(),
                definition_digest: "a".repeat(64),
            });
        open.profile.digest = crate::worker_execution::profile_digest(&open.definition).unwrap();
        let mut command = wire::broker::OpenBrokerSessionCommand {
            protocol_version: 5,
            open,
            broker_limits: broker_limits(),
        };
        command.open.identity.fingerprint =
            wire::broker::broker_session_open_fingerprint(&command).unwrap();
        command
    }
    fn open(&self, id: &str) -> wire::SessionOpenCommand {
        let definition = AgentDefinition {
            schema_version: 1,
            name: "Fixture".into(),
            description: String::new(),
            harness: "codex".into(),
            model: "fixture".into(),
            mode: "default".into(),
            instructions: None,
            skills: vec![],
            mcp: vec![],
            max_seconds: 60,
            max_output_bytes: 65536,
        };
        let mut command = wire::SessionOpenCommand {
            channel: channel("boot"),
            controller_session_id: format!("controller-session-{id}"),
            identity: wire::SessionIdentity {
                intent_id: format!("intent-{id}"),
                fingerprint: "0".repeat(64),
            },
            descriptor: serde_json::from_value(
                serde_json::json!({"scope":self.scope,"export":self.export,"grant":self.grant}),
            )
            .unwrap(),
            profile: AgentProfileRef {
                agent_id: "agent".into(),
                version: 1,
                digest: crate::worker_execution::profile_digest(&definition).unwrap(),
            },
            definition,
            budgets: wire::SessionBudgets {
                max_seconds: 60,
                max_output_bytes: 65536,
                idle_timeout_ms: 30000,
                lease_duration_ms: 10000,
                max_turns: 128,
                retained_bytes: 65536,
            },
        };
        command.identity.fingerprint = wire::session_open_fingerprint(&command).unwrap();
        command
    }
    fn validated(&self, s: &WorkerStore) -> ValidatedExport {
        s.candidate(
            &self.scope,
            &self.export.id,
            &principal(),
            ExportPermission::Execute,
        )
        .unwrap()
        .validate_filesystem()
        .unwrap()
    }
    fn admit(
        &self,
        s: &WorkerStore,
        c: &ExecutionChannel<'_>,
        open: &wire::SessionOpenCommand,
    ) -> SessionAdmission {
        s.open_session(c, open, self.validated(s)).unwrap()
    }
}
fn turn(session: &WorkerSessionLease, index: u64) -> wire::TurnCommand {
    let mut turn = wire::TurnCommand {
        session: session.fence().clone(),
        identity: wire::TurnIdentity {
            command_id: format!("command-{index}"),
            turn_id: format!("turn-{index}"),
            fingerprint: "0".repeat(64),
        },
        turn_index: index,
        input: format!("Turn {index}"),
        budgets: ExecutionBudgets {
            max_seconds: 30,
            max_output_bytes: 16384,
            lease_duration_ms: 10000,
        },
    };
    turn.identity.fingerprint = wire::session_turn_fingerprint(&turn).unwrap();
    turn
}
#[test]
fn two_turn_ledger_keeps_one_session_reservation_and_exact_retries_never_launch() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("one");
    let admitted = f.admit(&s, &ctx, &open);
    let mut lease = admitted.lease.unwrap();
    assert_eq!(f.admit(&s, &ctx, &open).receipt, admitted.receipt);
    assert!(f.admit(&s, &ctx, &open).lease.is_none());
    s.session_spawn_intent(&ctx, &lease).unwrap();
    assert!(s.session_spawn_intent(&ctx, &lease).is_err());
    s.session_ready(&ctx, &mut lease).unwrap();
    for index in 1..=2 {
        let command = turn(&lease, index);
        let accepted = s.admit_session_turn(&ctx, &mut lease, &command).unwrap();
        let mut active = accepted.lease.unwrap();
        assert!(
            s.admit_session_turn(&ctx, &mut lease, &command)
                .unwrap()
                .lease
                .is_none()
        );
        s.turn_write_intent(&ctx, &lease, &active).unwrap();
        assert!(s.turn_write_intent(&ctx, &lease, &active).is_err());
        s.append_session_text(&ctx, &mut lease, &active, wire::TextKind::Text, "answer")
            .unwrap();
        let terminal = s
            .complete_session_turn(
                &ctx,
                &mut lease,
                &mut active,
                wire::TurnTerminalStatus::Completed,
            )
            .unwrap();
        assert_eq!(terminal.status, wire::TurnStatus::Completed);
    }
    assert!(
        s.open_session(&ctx, &f.open("blocked"), f.validated(&s))
            .is_err()
    );
    let (cut, events, more) = s
        .replay_session_stream(&ctx, lease.fence(), 0, None, 8)
        .unwrap();
    assert_eq!(cut, 4);
    assert_eq!(events.len(), 4);
    assert!(!more);
    let stopped = s
        .finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
        .unwrap();
    assert_eq!(stopped.phase, wire::SessionPhase::Stopped);
    assert_eq!(
        s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
            .unwrap(),
        stopped
    );
    assert_eq!(
        s.connection
            .query_row(
                "SELECT reservation FROM worker_sessions WHERE id=?1",
                [&lease.fence().worker_session_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let next = f.admit(&s, &ctx, &f.open("next"));
    assert!(next.lease.is_some());
}

#[test]
fn unsolicited_outbox_skips_historical_boots_and_read_revocation_without_losing_negative_ack() {
    let f = Fixture::new();
    let store = f.store();
    let old_channel = channel("old-boot");
    let old_context = ExecutionChannel::verify(&f.scope, &old_channel, &allow).unwrap();
    let mut old_open = f.open("old-outbox");
    old_open.channel = old_channel.clone();
    old_open.identity.fingerprint = wire::session_open_fingerprint(&old_open).unwrap();
    let mut old = f.admit(&store, &old_context, &old_open).lease.unwrap();
    store.session_spawn_intent(&old_context, &old).unwrap();
    store.session_ready(&old_context, &mut old).unwrap();
    let old_turn = turn(&old, 1);
    let active = store
        .admit_session_turn(&old_context, &mut old, &old_turn)
        .unwrap()
        .lease
        .unwrap();
    store
        .turn_write_intent(&old_context, &old, &active)
        .unwrap();
    for _ in 0..8 {
        store
            .append_session_text(
                &old_context,
                &mut old,
                &active,
                wire::TextKind::Text,
                "historical-private",
            )
            .unwrap();
    }
    store
        .finish_stopped_session(&f.scope, &old_open.identity, old.fence(), |_| Ok(()))
        .unwrap();
    let current_channel = channel("boot");
    let current = ExecutionChannel::verify(&f.scope, &current_channel, &allow).unwrap();
    let mut session = f
        .admit(&store, &current, &f.open("current-outbox"))
        .lease
        .unwrap();
    store.session_spawn_intent(&current, &session).unwrap();
    store.session_ready(&current, &mut session).unwrap();
    let command = turn(&session, 1);
    let active = store
        .admit_session_turn(&current, &mut session, &command)
        .unwrap()
        .lease
        .unwrap();
    store
        .turn_write_intent(&current, &session, &active)
        .unwrap();
    let event = store
        .append_session_text(
            &current,
            &mut session,
            &active,
            wire::TextKind::Text,
            "current-private",
        )
        .unwrap();
    assert_eq!(store.session_outbox(&current).unwrap(), vec![event.clone()]);
    store
        .set_grant(
            &f.scope,
            &f.export.id,
            &principal(),
            &[],
            false,
            Some(f.grant.revision),
        )
        .unwrap();
    assert!(store.session_outbox(&current).unwrap().is_empty());
    store
        .acknowledge_session_stream(&current, session.fence(), event.sequence, &event.digest)
        .unwrap();
    assert!(store.connection.query_row(
        "SELECT payload IS NULL FROM worker_session_events WHERE session_id=?1 AND sequence=?2",
        params![event.session.worker_session_id, sql_revision(event.sequence).unwrap()],
        |row| row.get::<_, bool>(0),
    ).unwrap());
}
#[test]
fn reopened_retry_recovery_unknown_is_not_stop_and_requires_exact_attestation() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("reopen");
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let command = turn(&lease, 1);
    let active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    drop(s);
    let s = f.store();
    assert!(f.admit(&s, &ctx, &open).lease.is_none());
    assert_eq!(s.recover_session_boot(&f.scope, "new-boot").unwrap(), 1);
    let record = s
        .reconcile_worker_session(&ctx, &open.identity)
        .unwrap()
        .unwrap();
    assert_eq!(record.phase, wire::SessionPhase::Unknown);
    assert!(record.stop.is_none());
    assert!(record.active_turn.is_none());
    assert_eq!(
        s.reconcile_worker_turn(&ctx, lease.fence(), &command.identity)
            .unwrap()
            .unwrap()
            .status,
        wire::TurnStatus::Unknown
    );
    assert!(
        s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Err(
            Error::Denied
        ))
        .is_err()
    );
    assert!(
        s.open_session(&ctx, &f.open("no-replay"), f.validated(&s))
            .is_err()
    );
    let stopped = s
        .finish_stopped_session(&f.scope, &open.identity, lease.fence(), |actual| {
            assert_eq!(actual, lease.fence());
            Ok(())
        })
        .unwrap();
    assert_eq!(stopped.phase, wire::SessionPhase::Stopped);
}
#[test]
fn ack_prunes_only_payload_and_read_revocation_does_not_prevent_negative_ack() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("stream");
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let command = turn(&lease, 1);
    let mut active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    let text = s
        .append_session_text(&ctx, &mut lease, &active, wire::TextKind::Text, "private")
        .unwrap();
    let done = s
        .complete_session_turn(
            &ctx,
            &mut lease,
            &mut active,
            wire::TurnTerminalStatus::Completed,
        )
        .unwrap();
    let terminal = done.completion.unwrap();
    s.set_grant(
        &f.scope,
        &f.export.id,
        &principal(),
        &[],
        false,
        Some(f.grant.revision),
    )
    .unwrap();
    assert!(
        s.replay_session_stream(&ctx, lease.fence(), 0, None, 8)
            .is_err()
    );
    s.acknowledge_session_stream(
        &ctx,
        lease.fence(),
        terminal.final_sequence,
        &terminal.event_digest,
    )
    .unwrap();
    s.acknowledge_session_stream(&ctx, lease.fence(), text.sequence, &text.digest)
        .unwrap();
    assert!(
        s.acknowledge_session_stream(&ctx, lease.fence(), text.sequence, &"f".repeat(64))
            .is_err()
    );
    let revoked = s.grant(&f.export.id, &principal()).unwrap().unwrap();
    s.set_grant(
        &f.scope,
        &f.export.id,
        &principal(),
        &[ExportPermission::Read],
        true,
        Some(revoked.revision),
    )
    .unwrap();
    assert!(matches!(
        s.replay_session_stream(&ctx, lease.fence(), 0, None, 8),
        Err(Error::Conflict(_))
    ));
    let (_, events, _) = s
        .replay_session_stream(&ctx, lease.fence(), text.sequence, Some(&text.digest), 8)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        s.reconcile_worker_turn(&ctx, lease.fence(), &command.identity)
            .unwrap()
            .is_some()
    );
    assert!(s.validate_worker_session(&ctx, &lease).is_err());
}
#[test]
fn two_connections_share_one_acceptance_and_grant_aba_never_revives_handle() {
    let f = Fixture::new();
    let command = f.open("race");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let path = f.directory.path().join("state");
            let scope = f.scope.clone();
            let command = command.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let s = WorkerStore::open_existing(&path, scope.clone()).unwrap();
                let ctx = ExecutionChannel::verify(&scope, &command.channel, &allow).unwrap();
                let candidate = s
                    .candidate(
                        &scope,
                        &command.descriptor.export.id,
                        &principal(),
                        ExportPermission::Execute,
                    )
                    .unwrap()
                    .validate_filesystem()
                    .unwrap();
                barrier.wait();
                let result = s.open_session(&ctx, &command, candidate).unwrap();
                (result.receipt, result.lease.is_some())
            })
        })
        .collect::<Vec<_>>();
    let answers = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(answers[0].0, answers[1].0);
    assert_eq!(answers.iter().filter(|r| r.1).count(), 1);
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    s.finish_stopped_session(&f.scope, &command.identity, &answers[0].0.fence, |_| Ok(()))
        .unwrap();
    let next = f.open("next");
    let lease = f.admit(&s, &ctx, &next).lease.unwrap();
    let other = f.store();
    let revoked = other
        .set_grant(
            &f.scope,
            &f.export.id,
            &principal(),
            &[],
            false,
            Some(f.grant.revision),
        )
        .unwrap();
    other
        .set_grant(
            &f.scope,
            &f.export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            Some(revoked.revision),
        )
        .unwrap();
    assert!(s.validate_worker_session(&ctx, &lease).is_err());
    assert!(s.open_session(&ctx, &next, f.validated(&s)).is_err());
    // Negative exact-stop can still release the reservation after grant loss.
    s.finish_stopped_session(&f.scope, &next.identity, lease.fence(), |_| Ok(()))
        .unwrap();
}
#[test]
fn late_sql_failures_roll_back_identity_epoch_and_terminal_chain_together() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("rollback");
    s.connection.execute_batch("CREATE TRIGGER fail_open BEFORE INSERT ON worker_sessions BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s.open_session(&ctx, &open, f.validated(&s)).is_err());
    assert_eq!(
        s.connection
            .query_row("SELECT session_epoch FROM worker_session_meta", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    assert!(
        s.reconcile_worker_session(&ctx, &open.identity)
            .unwrap()
            .is_none()
    );
    s.connection
        .execute_batch("DROP TRIGGER fail_open;")
        .unwrap();
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let command = turn(&lease, 1);
    let active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    s.connection.execute_batch("CREATE TRIGGER fail_stop BEFORE UPDATE OF stop ON worker_sessions WHEN NEW.stop IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
            .is_err()
    );
    assert_eq!(
        s.connection
            .query_row("SELECT count(*) FROM worker_session_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.reconcile_worker_turn(&ctx, lease.fence(), &command.identity)
            .unwrap()
            .unwrap()
            .status,
        wire::TurnStatus::Running
    );
    s.connection
        .execute_batch("DROP TRIGGER fail_stop;")
        .unwrap();
    s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
        .unwrap();
    assert_eq!(
        s.connection
            .query_row("SELECT count(*) FROM worker_session_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn lifetime_event_capacity_reserves_unknown_and_stop_and_never_evicts_tombstones() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("quota");
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let command = turn(&lease, 1);
    let active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    // Compact acknowledged rows still count against the lifetime hash-chain cap.
    s.connection.execute("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4094) INSERT INTO worker_session_events(session_id,sequence,digest,payload,kind) SELECT ?1,x,?2,NULL,'text' FROM n",params![lease.fence().worker_session_id,"a".repeat(64)]).unwrap();
    s.connection
        .execute(
            "UPDATE worker_sessions SET tip_sequence=4094 WHERE id=?1",
            [&lease.fence().worker_session_id],
        )
        .unwrap();
    assert!(
        s.append_session_text(
            &ctx,
            &mut lease,
            &active,
            wire::TextKind::Text,
            "not retained"
        )
        .is_err()
    );
    assert!(s.validate_worker_session(&ctx, &lease).is_err());
    let record = s
        .finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
        .unwrap();
    assert_eq!(record.stop.as_ref().unwrap().final_sequence, 4096);
    assert_eq!(
        s.connection
            .query_row("SELECT count(*) FROM worker_session_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        4096
    );
    assert!(
        s.open_session(&ctx, &f.open("no-more"), f.validated(&s))
            .is_err()
    );
}
#[test]
fn duplicate_renewals_never_extend_clocks_and_close_targets_cannot_cancel_later_turn() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("renew");
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    assert_eq!(
        s.renew_worker_session(&ctx, &mut lease, 1, 10000).unwrap(),
        Renewal::Fresh
    );
    let deadline = lease.deadline;
    assert_eq!(
        s.renew_worker_session(&ctx, &mut lease, 1, 10000).unwrap(),
        Renewal::Duplicate
    );
    assert_eq!(lease.deadline, deadline);
    let command = turn(&lease, 1);
    let mut active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    s.complete_session_turn(
        &ctx,
        &mut lease,
        &mut active,
        wire::TurnTerminalStatus::Completed,
    )
    .unwrap();
    let next = turn(&lease, 2);
    let current = s
        .admit_session_turn(&ctx, &mut lease, &next)
        .unwrap()
        .lease
        .unwrap();
    assert!(
        !s.request_session_close(&ctx, lease.fence(), Some(active.fence()))
            .unwrap()
    );
    assert!(s.validate_worker_turn(&ctx, &lease, &current).is_ok());
    assert!(
        s.request_session_close(&ctx, lease.fence(), Some(current.fence()))
            .unwrap()
    );
    assert!(s.validate_worker_turn(&ctx, &lease, &current).is_err());
}
#[test]
fn replay_adaptively_bounds_escaped_json_bytes_without_changing_fixed_cut() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let mut open = f.open("escaped");
    open.budgets.retained_bytes = 262144;
    open.identity.fingerprint = wire::session_open_fingerprint(&open).unwrap();
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let mut command = turn(&lease, 1);
    command.budgets.max_output_bytes = 65536;
    command.identity.fingerprint = wire::session_turn_fingerprint(&command).unwrap();
    let active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    for _ in 0..8 {
        s.append_session_text(
            &ctx,
            &mut lease,
            &active,
            wire::TextKind::Text,
            &"\t".repeat(4096),
        )
        .unwrap();
    }
    let (cut, events, more) = s
        .replay_session_stream(&ctx, lease.fence(), 0, None, 8)
        .unwrap();
    assert_eq!(cut, 8);
    assert!(!events.is_empty());
    assert!(events.len() < 8);
    assert!(more);
    let page = wire::SessionWorkerMessage::StreamReplayed {
        channel: ch,
        session: lease.fence().clone(),
        after_sequence: 0,
        after_digest: None,
        limit: 8,
        through_sequence: cut,
        events,
        more,
    };
    page.validate().unwrap();
    assert!(serde_json::to_vec(&page).unwrap().len() <= 65536);
}
#[test]
fn delayed_commit_cannot_renew_a_session_after_its_old_deadline() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    let f = Fixture::new();
    let s = f.store();
    s.connection
        .execute_batch("PRAGMA journal_mode=DELETE;")
        .unwrap();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let mut lease = f.admit(&s, &ctx, &f.open("commit")).lease.unwrap();
    let (ready_send, ready_receive) = mpsc::channel();
    let (updated_send, updated_receive) = mpsc::channel::<Instant>();
    let path = f.directory.path().join("state/exports.sqlite3");
    let reader = std::thread::spawn(move || {
        let c = rusqlite::Connection::open(path).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let _: i64 = tx
            .query_row("SELECT count(*) FROM worker_sessions", [], |r| r.get(0))
            .unwrap();
        ready_send.send(()).unwrap();
        let deadline = updated_receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        std::thread::sleep(
            (deadline + Duration::from_millis(25)).saturating_duration_since(Instant::now()),
        );
        tx.commit().unwrap();
    });
    ready_receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let old = Instant::now() + Duration::from_millis(500);
    lease.deadline = old;
    let calls = AtomicUsize::new(0);
    let guard = || {
        if calls.fetch_add(1, Ordering::SeqCst) == 2 {
            updated_send.send(old).unwrap();
        }
        Ok(())
    };
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &guard).unwrap();
    assert!(s.renew_worker_session(&ctx, &mut lease, 1, 5000).is_err());
    reader.join().unwrap();
    assert_eq!(lease.deadline, old);
    assert!(lease.remaining().is_err());
}
#[test]
fn policy_corruption_blocks_positive_work_but_not_trusted_stopped_cleanup() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("policy");
    let lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.connection
        .execute(
            "UPDATE export_execution_policies SET raw=?1 WHERE export_id=?2",
            params![b"{}".as_slice(), f.export.id],
        )
        .unwrap();
    assert!(s.validate_worker_session(&ctx, &lease).is_err());
    s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
        .unwrap();
}
#[test]
fn stopped_same_channel_turn_retry_is_canonical_without_live_process_authority() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("stopped-retry");
    let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
    s.session_spawn_intent(&ctx, &lease).unwrap();
    s.session_ready(&ctx, &mut lease).unwrap();
    let command = turn(&lease, 1);
    let active = s
        .admit_session_turn(&ctx, &mut lease, &command)
        .unwrap()
        .lease
        .unwrap();
    s.turn_write_intent(&ctx, &lease, &active).unwrap();
    s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
        .unwrap();
    let canonical = s
        .reconcile_worker_turn(&ctx, lease.fence(), &command.identity)
        .unwrap()
        .unwrap();
    assert_eq!(
        s.lookup_session_turn(&ctx, &command).unwrap().unwrap(),
        canonical
    );
    let mut changed = command.clone();
    changed.input = "different body".into();
    changed.identity.fingerprint = wire::session_turn_fingerprint(&changed).unwrap();
    assert!(s.lookup_session_turn(&ctx, &changed).is_err());
    let replacement = channel("replacement");
    let replaced = ExecutionChannel::verify(&f.scope, &replacement, &allow).unwrap();
    assert!(s.lookup_session_turn(&replaced, &command).is_err());
    assert_eq!(
        s.reconcile_worker_turn(&replaced, lease.fence(), &command.identity)
            .unwrap()
            .unwrap(),
        canonical
    );
}
#[test]
fn uncertain_or_cancelled_turn_never_reopens_the_same_acp_context() {
    for terminal in [
        wire::TurnTerminalStatus::Unknown,
        wire::TurnTerminalStatus::Cancelled,
    ] {
        let f = Fixture::new();
        let s = f.store();
        let ch = channel("boot");
        let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
        let open = f.open("barrier");
        let mut lease = f.admit(&s, &ctx, &open).lease.unwrap();
        s.session_spawn_intent(&ctx, &lease).unwrap();
        s.session_ready(&ctx, &mut lease).unwrap();
        let command = turn(&lease, 1);
        let mut active = s
            .admit_session_turn(&ctx, &mut lease, &command)
            .unwrap()
            .lease
            .unwrap();
        s.turn_write_intent(&ctx, &lease, &active).unwrap();
        s.complete_session_turn(&ctx, &mut lease, &mut active, terminal)
            .unwrap();
        let next = turn(&lease, 2);
        assert!(s.admit_session_turn(&ctx, &mut lease, &next).is_err());
        assert!(s.renew_worker_session(&ctx, &mut lease, 1, 10000).is_err());
        assert!(
            s.open_session(&ctx, &f.open("blocked"), f.validated(&s))
                .is_err()
        );
        assert_eq!(
            s.reconcile_worker_session(&ctx, &open.identity)
                .unwrap()
                .unwrap()
                .phase,
            wire::SessionPhase::Unknown
        );
        s.finish_stopped_session(&f.scope, &open.identity, lease.fence(), |_| Ok(()))
            .unwrap();
    }
}

#[test]
fn read_revocation_keeps_only_exact_stopped_metadata_and_negative_ack_available() {
    let f = Fixture::new();
    let s = f.store();
    let ch = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &ch, &allow).unwrap();
    let open = f.open("read-revoked-stop");
    let admitted = f.admit(&s, &ctx, &open);
    s.set_grant(
        &f.scope,
        &f.export.id,
        &principal(),
        &[],
        false,
        Some(f.grant.revision),
    )
    .unwrap();
    assert!(
        s.reconcile_worker_session(&ctx, &open.identity).is_err(),
        "live state still requires Read"
    );
    s.finish_stopped_session(
        &f.scope,
        &open.identity,
        &admitted.receipt.fence,
        |_| Ok(()),
    )
    .unwrap();
    let record = s
        .reconcile_worker_session(&ctx, &open.identity)
        .unwrap()
        .unwrap();
    assert_eq!(record.phase, wire::SessionPhase::Stopped);
    assert!(record.active_turn.is_none());
    let stop = record.stop.unwrap();
    assert!(
        s.replay_session_stream(&ctx, &admitted.receipt.fence, 0, None, 8)
            .is_err()
    );
    assert!(s.session_outbox(&ctx).unwrap().is_empty());
    s.acknowledge_session_stream(
        &ctx,
        &admitted.receipt.fence,
        stop.final_sequence,
        &stop.event_digest,
    )
    .unwrap();
    let mut wrong = open.identity.clone();
    wrong.fingerprint = "f".repeat(64);
    assert!(s.reconcile_worker_session(&ctx, &wrong).is_err());
    let scope = ExportScope {
        host_id: "different-host".into(),
        ..f.scope.clone()
    };
    let foreign = ExecutionChannel::verify(&scope, &ch, &allow).unwrap();
    assert!(
        s.reconcile_worker_session(&foreign, &open.identity)
            .is_err()
    );
}
