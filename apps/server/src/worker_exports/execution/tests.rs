use super::*;
use crate::worker_exports::{
    CapturedCheckout, ExportPrincipal, ExportSummary, GrantSummary, policy::PolicySnapshot,
};
use dunlin_domain::agent::{AgentDefinition, AgentProfileRef};
use dunlin_relay_protocol::worker::{ExecutionBudgets, ExportDescriptor};
use std::sync::{Arc, Barrier};
const POLICY:&[u8]=br#"{"schema_version":1,"harnesses":["codex"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1}"#;
struct Fixture {
    directory: tempfile::TempDir,
    scope: ExportScope,
    export: ExportSummary,
    grant: GrantSummary,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let scope = ExportScope {
            controller_node_id: "controller".into(),
            worker_node_id: "worker".into(),
            workspace_id: "workspace".into(),
            host_id: "host".into(),
            enrollment_epoch: 1,
            credential_revision: 1,
        };
        let path = directory.path().canonicalize().unwrap().join("policy.json");
        std::fs::write(&path, POLICY).unwrap();
        let policy = PolicySnapshot::load(&path).unwrap();
        let root = directory.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let checkout = CapturedCheckout::capture(&root, policy.digest(), None).unwrap();
        let store = WorkerStore::open(&directory.path().join("state"), scope.clone()).unwrap();
        let export = store
            .register_captured_with_policy(&scope, "Execution", &checkout, Some(&policy))
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
    fn command(&self, id: &str, channel: ChannelFence) -> ExecutionCommand {
        let definition = AgentDefinition {
            schema_version: 1,
            name: "Fixture".into(),
            description: String::new(),
            harness: "codex".into(),
            model: "fixture-model".into(),
            mode: "default".into(),
            instructions: None,
            skills: vec![],
            mcp: vec![],
            max_seconds: 60,
            max_output_bytes: 65536,
        };
        let descriptor: ExportDescriptor = serde_json::from_value(
            serde_json::json!({"scope":self.scope,"export":self.export,"grant":self.grant}),
        )
        .unwrap();
        let mut command = ExecutionCommand {
            channel,
            identity: ExecutionIdentity {
                command_id: format!("command-{id}"),
                attempt_id: format!("attempt-{id}"),
                fingerprint: "0".repeat(64),
            },
            descriptor,
            profile: AgentProfileRef {
                agent_id: "agent".into(),
                version: 1,
                digest: crate::worker_execution::profile_digest(&definition).unwrap(),
            },
            definition,
            input: "Do one bounded turn".into(),
            budgets: ExecutionBudgets {
                max_seconds: 60,
                max_output_bytes: 65536,
                lease_duration_ms: 10000,
            },
        };
        command.identity.fingerprint =
            crate::worker_execution::command_fingerprint(&command).unwrap();
        command
    }
    fn validated(&self, store: &WorkerStore) -> ValidatedExport {
        store
            .candidate(
                &self.scope,
                &self.export.id,
                &principal(),
                ExportPermission::Execute,
            )
            .unwrap()
            .validate_filesystem()
            .unwrap()
    }
}
fn allow() -> Result<()> {
    Ok(())
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
        controller_boot_id: "controller-boot".into(),
        worker_boot_id: boot.into(),
    }
}
fn outcome(status: ExecutionTerminalStatus) -> ExecutionOutcome {
    let mut outcome = ExecutionOutcome {
        outcome_id: format!("outcome-{}", uuid::Uuid::now_v7()),
        digest: "0".repeat(64),
        status,
        text: "Retained private result".into(),
    };
    outcome.digest = crate::worker_execution::outcome_digest(&outcome).unwrap();
    outcome
}
fn admitted(
    f: &Fixture,
    store: &WorkerStore,
    ctx: &ExecutionChannel<'_>,
    command: &ExecutionCommand,
) -> ExecutionAdmission {
    store
        .admit_execution(ctx, command, f.validated(store))
        .unwrap()
}
#[test]
fn exact_retry_lost_receipt_and_start_intent_are_durable_not_new_process_permits() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot-a");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("one", wire.clone());
    let accepted = admitted(&f, &store, &ctx, &command);
    let lease = accepted.lease.unwrap();
    assert_eq!(lease.fence(), &accepted.record.receipt.fence);
    assert!(!lease.remaining().unwrap().is_zero());
    let retry = admitted(&f, &store, &ctx, &command);
    assert!(retry.lease.is_none());
    assert_eq!(accepted.record, retry.record);
    store.execution_start_intent(&ctx, &lease).unwrap();
    assert!(store.execution_start_intent(&ctx, &lease).is_err());
    store.execution_running(&ctx, &lease).unwrap();
    let mut changed = command.clone();
    changed.input = "Different body".into();
    changed.identity.fingerprint = crate::worker_execution::command_fingerprint(&changed).unwrap();
    assert!(
        store
            .admit_execution(&ctx, &changed, f.validated(&store))
            .is_err()
    );
    drop(store);
    let reopened = f.store();
    let report = reopened
        .reconcile_execution(&ctx, &command.identity)
        .unwrap()
        .unwrap();
    assert_eq!(report.receipt, accepted.record.receipt);
    assert_eq!(report.status, ExecutionStatus::Running);
    let result = outcome(ExecutionTerminalStatus::Completed);
    let terminal = reopened.finish_execution(&ctx, &lease, &result).unwrap();
    assert_eq!(terminal.status, ExecutionStatus::Completed);
    assert!(
        reopened
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Unknown))
            .is_err()
    );
    assert!(
        reopened
            .reconcile_execution(&ctx, &command.identity)
            .is_err()
    );
    assert!(reopened.execution_outbox(&ctx).unwrap().is_empty());
    assert!(
        reopened
            .admit_execution(&ctx, &command, f.validated(&reopened))
            .is_err()
    );
    // Injected trusted actor attestation only: this unit test proves ledger
    // delivery ordering, not process/namespace containment.
    reopened
        .confirm_execution_stopped(&f.scope, &command.identity, lease.fence(), |actual| {
            assert_eq!(actual, lease.fence());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        reopened
            .reconcile_execution(&ctx, &command.identity)
            .unwrap(),
        Some(terminal.clone())
    );
    assert!(admitted(&f, &reopened, &ctx, &command).lease.is_none());
    assert_eq!(reopened.execution_outbox(&ctx).unwrap(), vec![terminal]);
}
#[test]
fn reserved_outcome_failure_rolls_back_acceptance_and_epoch() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("rollback", wire.clone());
    store.connection.execute_batch("CREATE TRIGGER reject_reservation BEFORE INSERT ON worker_execution_outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        store
            .admit_execution(&ctx, &command, f.validated(&store))
            .is_err()
    );
    assert!(
        store
            .reconcile_execution(&ctx, &command.identity)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT next_epoch FROM worker_execution_meta", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_reservation;")
        .unwrap();
    assert_eq!(
        admitted(&f, &store, &ctx, &command)
            .record
            .receipt
            .fence
            .execution_epoch,
        1
    );
}
#[test]
fn sql_revoke_regrant_and_filesystem_changes_do_not_refresh_an_old_live_authority() {
    let f = Fixture::new();
    let store = f.store();
    let other = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("one", wire.clone());
    let validated = f.validated(&store);
    let revoked = other
        .set_grant(&f.scope, &f.export.id, &principal(), &[], false, Some(1))
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
    assert!(store.admit_execution(&ctx, &command, validated).is_err());
    let mut refreshed = command;
    refreshed.descriptor.grant = serde_json::from_value(
        serde_json::to_value(other.grants(&f.scope, &f.export.id).unwrap().remove(0)).unwrap(),
    )
    .unwrap();
    refreshed.identity.fingerprint =
        crate::worker_execution::command_fingerprint(&refreshed).unwrap();
    let lease = admitted(&f, &store, &ctx, &refreshed).lease.unwrap();
    std::fs::rename(
        f.directory.path().join("checkout"),
        f.directory.path().join("old-root"),
    )
    .unwrap();
    std::fs::create_dir(f.directory.path().join("checkout")).unwrap();
    assert!(lease.validate_filesystem().is_err());
    other
        .connection
        .execute(
            "UPDATE export_execution_policies SET digest=?1",
            ["0".repeat(64)],
        )
        .unwrap();
    assert!(store.validate_execution(&ctx, &lease).is_err());
    assert!(
        store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Completed))
            .is_err()
    );
    assert_eq!(
        store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Unknown))
            .unwrap()
            .status,
        ExecutionStatus::Unknown
    );
}
#[test]
fn renewal_exact_duplicates_never_extend_and_expired_clocks_cannot_be_restored() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("one", wire.clone());
    let mut lease = admitted(&f, &store, &ctx, &command).lease.unwrap();
    assert_eq!(
        store.renew_execution(&ctx, &mut lease, 1, 5000).unwrap(),
        Renewal::Fresh
    );
    let deadline = lease.deadline;
    assert_eq!(
        store.renew_execution(&ctx, &mut lease, 1, 5000).unwrap(),
        Renewal::Duplicate
    );
    assert_eq!(deadline, lease.deadline);
    assert!(store.renew_execution(&ctx, &mut lease, 1, 6000).is_err());
    assert!(store.renew_execution(&ctx, &mut lease, 3, 5000).is_err());
    lease.deadline = Instant::now() - Duration::from_millis(1);
    assert!(store.renew_execution(&ctx, &mut lease, 2, 5000).is_err());
    assert!(store.execution_start_intent(&ctx, &lease).is_err());
    assert!(admitted(&f, &store, &ctx, &command).lease.is_none());
}
#[test]
fn old_boot_recovery_unknown_is_atomic_and_never_replays_or_overwrites_terminal() {
    let f = Fixture::new();
    let store = f.store();
    let old = channel("old");
    let ctx = ExecutionChannel::verify(&f.scope, &old, &allow).unwrap();
    let command = f.command("one", old.clone());
    let lease = admitted(&f, &store, &ctx, &command).lease.unwrap();
    store.execution_start_intent(&ctx, &lease).unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_unknown BEFORE UPDATE ON worker_execution_outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store.recover_execution_boot(&f.scope, "new").is_err());
    assert_eq!(
        store
            .reconcile_execution(&ctx, &command.identity)
            .unwrap()
            .unwrap()
            .status,
        ExecutionStatus::Accepted
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_unknown;")
        .unwrap();
    drop(store);
    let store = f.store();
    assert_eq!(store.recover_execution_boot(&f.scope, "new").unwrap(), 1);
    assert_eq!(store.recover_execution_boot(&f.scope, "new").unwrap(), 0);
    let new = channel("new");
    let current = ExecutionChannel::verify(&f.scope, &new, &allow).unwrap();
    assert!(
        store
            .reconcile_execution(&current, &command.identity)
            .is_err()
    );
    assert!(store.execution_outbox(&current).unwrap().is_empty());
    assert!(
        store
            .admit_execution(&current, &command, f.validated(&store))
            .is_err()
    );
    // Internal SQL inspection supplies expected immutable identity for the stop
    // attestation/negative ACK; no terminal record was delivered to a peer.
    let report = store
        .execution_stored(&command.identity)
        .unwrap()
        .unwrap()
        .record;
    assert_eq!(report.receipt.fence.channel.worker_boot_id, "old");
    assert_eq!(report.status, ExecutionStatus::Unknown);
    assert!(
        store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Failed))
            .is_err()
    );
    let new_command = f.command("two", new.clone());
    assert!(
        store
            .admit_execution(&current, &new_command, f.validated(&store))
            .is_err()
    );
    assert!(
        store
            .confirm_execution_stopped(&f.scope, &command.identity, &report.receipt.fence, |_| Err(
                Error::Denied
            ))
            .is_err()
    );
    assert!(
        store
            .admit_execution(&current, &new_command, f.validated(&store))
            .is_err()
    );
    let historical = report.outcome.as_ref().unwrap();
    store
        .acknowledge_execution(
            &current,
            &command.identity,
            &report.receipt.fence,
            &historical.outcome_id,
            &historical.digest,
        )
        .unwrap();
    assert!(
        store
            .admit_execution(&current, &new_command, f.validated(&store))
            .is_err()
    );
    // This is an injected trusted actor attestation, not a kernel/process test.
    store
        .confirm_execution_stopped(
            &f.scope,
            &command.identity,
            &report.receipt.fence,
            |actual| {
                assert_eq!(actual, &report.receipt.fence);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        store
            .reconcile_execution(&current, &command.identity)
            .unwrap(),
        Some(report)
    );
    assert!(admitted(&f, &store, &current, &command).lease.is_none());
    assert!(admitted(&f, &store, &current, &new_command).lease.is_some());
}
#[test]
fn read_revocation_blocks_retained_output_but_exact_ack_releases_only_outbox() {
    let f = Fixture::new();
    let store = f.store();
    let other = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("one", wire.clone());
    let lease = admitted(&f, &store, &ctx, &command).lease.unwrap();
    let result = outcome(ExecutionTerminalStatus::Failed);
    store.finish_execution(&ctx, &lease, &result).unwrap();
    other
        .set_grant(&f.scope, &f.export.id, &principal(), &[], false, Some(1))
        .unwrap();
    assert!(store.reconcile_execution(&ctx, &command.identity).is_err());
    assert!(store.execution_outbox(&ctx).unwrap().is_empty());
    assert!(
        store
            .acknowledge_execution(
                &ctx,
                &command.identity,
                lease.fence(),
                &result.outcome_id,
                &"0".repeat(64)
            )
            .is_err()
    );
    store
        .acknowledge_execution(
            &ctx,
            &command.identity,
            lease.fence(),
            &result.outcome_id,
            &result.digest,
        )
        .unwrap();
    store
        .acknowledge_execution(
            &ctx,
            &command.identity,
            lease.fence(),
            &result.outcome_id,
            &result.digest,
        )
        .unwrap();
    assert!(store.execution_outbox(&ctx).unwrap().is_empty());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    other
        .set_grant(
            &f.scope,
            &f.export.id,
            &principal(),
            &[ExportPermission::Read],
            true,
            Some(2),
        )
        .unwrap();
    assert_eq!(
        store
            .reconcile_execution(&ctx, &command.identity)
            .unwrap()
            .unwrap()
            .outcome,
        Some(result)
    );
}
#[test]
fn two_connections_same_acceptance_have_one_launch_handle_and_active_capacity_is_global() {
    let f = Fixture::new();
    let wire = channel("boot");
    let command = f.command("race", wire.clone());
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = vec![];
    for _ in 0..2 {
        let path = f.directory.path().join("state");
        let scope = f.scope.clone();
        let command = command.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let store = WorkerStore::open_existing(&path, scope.clone()).unwrap();
            let ctx = ExecutionChannel::verify(&scope, &command.channel, &allow).unwrap();
            let candidate = store
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
            store
                .admit_execution(&ctx, &command, candidate)
                .unwrap()
                .lease
                .is_some()
        }));
    }
    let launches = workers
        .into_iter()
        .map(|w| usize::from(w.join().unwrap()))
        .sum::<usize>();
    assert_eq!(launches, 1);
    let store = f.store();
    let next = f.command("next", command.channel.clone());
    let ctx = ExecutionChannel::verify(&f.scope, &command.channel, &allow).unwrap();
    assert!(
        store
            .admit_execution(&ctx, &next, f.validated(&store))
            .is_err()
    );
}
#[test]
fn trusted_callback_loss_and_cancel_prevent_positive_completion_but_allow_unknown() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot");
    let live = AtomicBool::new(true);
    let guard = || {
        if live.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    };
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &guard).unwrap();
    let command = f.command("one", wire.clone());
    let lease = admitted(&f, &store, &ctx, &command).lease.unwrap();
    store.execution_start_intent(&ctx, &lease).unwrap();
    store.execution_running(&ctx, &lease).unwrap();
    store.cancel_execution(&ctx, lease.fence()).unwrap();
    assert!(
        store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Completed))
            .is_err()
    );
    live.store(false, Ordering::Release);
    assert!(store.validate_execution(&ctx, &lease).is_err());
    assert!(store.reconcile_execution(&ctx, &command.identity).is_err());
    assert_eq!(
        store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Unknown))
            .unwrap()
            .status,
        ExecutionStatus::Unknown
    );
}

#[test]
fn lifetime_capacity_survives_terminal_ack_and_exact_receipt_still_bypasses_quota() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let first = f.command("quota-0", wire.clone());
    let mut original = None;
    for index in 0..MAX_EXPORT_INBOX {
        let command = f.command(&format!("quota-{index}"), wire.clone());
        let admission = admitted(&f, &store, &ctx, &command);
        let lease = admission.lease.unwrap();
        let result = outcome(ExecutionTerminalStatus::Failed);
        let record = store.finish_execution(&ctx, &lease, &result).unwrap();
        // Pristine acceptance never yielded a start intent, so no process was
        // authorized and its slot can be released without a fabricated proof.
        store
            .acknowledge_execution(
                &ctx,
                &command.identity,
                lease.fence(),
                &result.outcome_id,
                &result.digest,
            )
            .unwrap();
        if index == 0 {
            original = Some(record);
        }
    }
    assert!(store.execution_outbox(&ctx).unwrap().is_empty());
    let duplicate = admitted(&f, &store, &ctx, &first);
    assert!(duplicate.lease.is_none());
    assert_eq!(Some(duplicate.record), original);
    let next = f.command("quota-next", wire.clone());
    assert!(
        store
            .admit_execution(&ctx, &next, f.validated(&store))
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        MAX_EXPORT_INBOX
    );
}
#[test]
fn export_rebind_and_profile_or_policy_mismatch_cannot_admit_or_renew_authority() {
    let f = Fixture::new();
    let store = f.store();
    let other = f.store();
    let wire = channel("boot");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("one", wire.clone());
    for variant in 0..3 {
        let mut changed = command.clone();
        if variant == 0 {
            changed.definition.harness = "claude".into();
            changed.profile.digest =
                crate::worker_execution::profile_digest(&changed.definition).unwrap();
        }
        if variant == 1 {
            changed.definition.max_output_bytes = 131072;
            changed.budgets.max_output_bytes = 131072;
            changed.profile.digest =
                crate::worker_execution::profile_digest(&changed.definition).unwrap();
        }
        if variant == 2 {
            changed.profile.digest = "0".repeat(64);
        }
        changed.identity.fingerprint =
            crate::worker_execution::command_fingerprint(&changed).unwrap();
        assert!(
            store
                .admit_execution(&ctx, &changed, f.validated(&store))
                .is_err()
        );
    }
    let validated = f.validated(&store);
    let policy = PolicySnapshot::load(
        &f.directory
            .path()
            .canonicalize()
            .unwrap()
            .join("policy.json"),
    )
    .unwrap();
    let checkout =
        CapturedCheckout::capture(&f.directory.path().join("checkout"), policy.digest(), None)
            .unwrap();
    other
        .rebind_captured_with_policy(
            &f.scope,
            &f.export.id,
            1,
            "Reapproved",
            &checkout,
            Some(&policy),
        )
        .unwrap();
    let revoked = other.grants(&f.scope, &f.export.id).unwrap().remove(0);
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
    assert!(store.admit_execution(&ctx, &command, validated).is_err());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn late_trusted_channel_loss_rolls_back_acceptance_and_reserved_outbox() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("boot");
    let calls = AtomicUsize::new(0);
    let guard = || {
        if calls.fetch_add(1, Ordering::SeqCst) < 2 {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    };
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &guard).unwrap();
    let command = f.command("one", wire.clone());
    assert!(
        store
            .admit_execution(&ctx, &command, f.validated(&store))
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM worker_execution_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn delayed_sqlite_commit_cannot_revive_an_expired_monotonic_lease() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    let f = Fixture::new();
    let store = f.store();
    store
        .connection
        .execute_batch("PRAGMA journal_mode=DELETE;")
        .unwrap();
    let wire = channel("boot");
    let initial = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("delayed-commit", wire.clone());
    let mut lease = admitted(&f, &store, &initial, &command).lease.unwrap();
    let (ready_send, ready_receive) = mpsc::channel();
    let (updated_send, updated_receive) = mpsc::channel::<Instant>();
    let database = f.directory.path().join("state/exports.sqlite3");
    let reader = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(database).unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        // DELETE journal permits the writer's RESERVED lock/UPDATE, while this
        // shared reader blocks its EXCLUSIVE lock at COMMIT.
        let _: i64 = tx
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| {
                r.get(0)
            })
            .unwrap();
        ready_send.send(()).unwrap();
        let old_deadline = updated_receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let until_expired =
            (old_deadline + Duration::from_millis(25)).saturating_duration_since(Instant::now());
        std::thread::sleep(until_expired);
        tx.commit().unwrap();
    });
    ready_receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let old_deadline = Instant::now() + Duration::from_millis(500);
    lease.deadline = old_deadline;
    let calls = AtomicUsize::new(0);
    let guard = || {
        // verify + current_execution then final validation, after the actual
        // SQL UPDATE and immediately before COMMIT. Channels establish order;
        // sleeping is solely for crossing the old lease's monotonic expiry.
        if calls.fetch_add(1, Ordering::SeqCst) == 2 {
            updated_send.send(old_deadline).unwrap();
        }
        Ok(())
    };
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &guard).unwrap();
    assert!(store.renew_execution(&ctx, &mut lease, 1, 5000).is_err());
    reader.join().unwrap();
    assert!(lease.remaining().is_err());
    assert_eq!(lease.deadline, old_deadline);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT renew_sequence FROM worker_execution_inbox WHERE attempt_id=?1",
                [&command.identity.attempt_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(store.execution_start_intent(&initial, &lease).is_err());
}

fn downgrade_to_policy_v2(store: &WorkerStore) {
    store.connection.execute_batch("DROP TABLE worker_session_events;DROP TABLE worker_session_turns;DROP TABLE worker_sessions;DROP TABLE worker_session_meta;DROP TABLE worker_execution_outbox;DROP TABLE worker_execution_inbox;DROP TABLE worker_execution_meta;PRAGMA user_version=2;").unwrap();
}
fn version(store: &WorkerStore) -> i64 {
    store
        .connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}
fn table_exists(connection: &rusqlite::Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [name],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn real_operator_upgrade_v2_preserves_policy_and_grants_while_peer_open_never_migrates() {
    let f = Fixture::new();
    let store = f.store();
    downgrade_to_policy_v2(&store);
    drop(store);
    let peer =
        WorkerStore::open_existing(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&peer), 2);
    assert!(!table_exists(&peer.connection, "worker_execution_meta"));
    assert!(!table_exists(&peer.connection, "worker_execution_inbox"));
    assert!(!table_exists(&peer.connection, "worker_execution_outbox"));
    let observed = peer
        .candidate(&f.scope, &f.export.id, &principal(), ExportPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let authority = peer.authorize_validated(&f.scope, observed).unwrap();
    assert_eq!(authority.export_stamp().export, f.export);
    assert_eq!(authority.grant_stamp().grant, f.grant);
    assert_eq!(version(&peer), 2);
    drop(peer);
    let operator = WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&operator), 4);
    assert_eq!(operator.list(&f.scope).unwrap(), vec![f.export.clone()]);
    assert_eq!(
        operator.grants(&f.scope, &f.export.id).unwrap(),
        vec![f.grant.clone()]
    );
    assert_eq!(
        operator
            .policy_snapshot(&f.export.id, &f.export.settings_digest)
            .unwrap()
            .unwrap()
            .digest(),
        f.export.settings_digest
    );
    for table in ["worker_execution_inbox", "worker_execution_outbox"] {
        assert_eq!(
            operator
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        operator
            .connection
            .query_row("SELECT next_epoch FROM worker_execution_meta", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(operator);
    assert_eq!(version(&f.store()), 4);
}
#[test]
fn last_v3_table_collision_rolls_back_first_tables_and_reopens_after_only_collision_removed() {
    let f = Fixture::new();
    let store = f.store();
    downgrade_to_policy_v2(&store);
    store.connection.execute_batch("CREATE TABLE worker_execution_outbox(marker TEXT);INSERT INTO worker_execution_outbox VALUES('original-collision');").unwrap();
    drop(store);
    assert!(WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).is_err());
    let peer =
        WorkerStore::open_existing(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&peer), 2);
    assert!(!table_exists(&peer.connection, "worker_execution_meta"));
    assert!(!table_exists(&peer.connection, "worker_execution_inbox"));
    assert_eq!(
        peer.connection
            .query_row("SELECT marker FROM worker_execution_outbox", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "original-collision"
    );
    assert_eq!(peer.list(&f.scope).unwrap(), vec![f.export.clone()]);
    assert_eq!(
        peer.grants(&f.scope, &f.export.id).unwrap(),
        vec![f.grant.clone()]
    );
    assert!(
        peer.policy_snapshot(&f.export.id, &f.export.settings_digest)
            .unwrap()
            .is_some()
    );
    // Earlier CREATE statements were rolled back; only the preexisting final
    // collision table must be removed before an explicit operator retry.
    peer.connection
        .execute_batch("DROP TABLE worker_execution_outbox;")
        .unwrap();
    drop(peer);
    let operator = WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&operator), 4);
    for table in [
        "worker_execution_meta",
        "worker_execution_inbox",
        "worker_execution_outbox",
    ] {
        assert!(table_exists(&operator.connection, table));
    }
    assert_eq!(operator.list(&f.scope).unwrap(), vec![f.export]);
    assert_eq!(
        operator
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn guardian_stop_finalization_after_revoke_is_atomic_without_reconstructing_a_lease() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("crashed-worker");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("guardian-stop", wire.clone());
    let accepted = admitted(&f, &store, &ctx, &command);
    let lease = accepted.lease.unwrap();
    store.execution_start_intent(&ctx, &lease).unwrap();
    store.execution_running(&ctx, &lease).unwrap();
    let fence = lease.fence().clone();
    drop(lease); // The main process's executable handle is unavailable.
    store
        .set_grant(&f.scope, &f.export.id, &principal(), &[], false, Some(1))
        .unwrap();
    let mut wrong = fence.clone();
    wrong.execution_epoch += 1;
    assert!(
        store
            .finish_stopped_execution(&f.scope, &command.identity, &wrong, |_| {
                panic!("A mismatched fence must fail before consulting a proof")
            })
            .is_err()
    );
    assert!(
        store
            .finish_stopped_execution(&f.scope, &command.identity, &fence, |_| Err(Error::Denied))
            .is_err()
    );
    // This controlled callback proves only SQL composition, not kernel death.
    store.connection.execute_batch("CREATE TRIGGER reject_guardian_release BEFORE UPDATE OF reservation ON worker_execution_inbox WHEN NEW.reservation=0 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        store
            .finish_stopped_execution(&f.scope, &command.identity, &fence, |_| Ok(()))
            .is_err()
    );
    let stored = store.execution_stored(&command.identity).unwrap().unwrap();
    assert_eq!(stored.phase, "running");
    assert!(stored.record.outcome.is_none());
    assert!(stored.reservation);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_guardian_release;")
        .unwrap();
    let result = store
        .finish_stopped_execution(&f.scope, &command.identity, &fence, |_| Ok(()))
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Unknown);
    assert_eq!(result.receipt, accepted.record.receipt);
    let outcome = result.outcome.as_ref().unwrap();
    assert_eq!(
        outcome.digest,
        crate::worker_execution::outcome_digest(outcome).unwrap()
    );
    assert!(
        !store
            .execution_stored(&command.identity)
            .unwrap()
            .unwrap()
            .reservation
    );
    assert!(
        store.execution_outbox(&ctx).unwrap().is_empty(),
        "Stop confirmation cannot restore revoked Read"
    );
    drop(store);
    let reopened = f.store();
    assert_eq!(
        reopened
            .finish_stopped_execution(&f.scope, &command.identity, &fence, |_| Ok(()))
            .unwrap(),
        result
    );
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT count(*) FROM worker_execution_inbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn guardian_stop_finalization_preserves_an_existing_completed_outcome() {
    let f = Fixture::new();
    let store = f.store();
    let wire = channel("worker");
    let ctx = ExecutionChannel::verify(&f.scope, &wire, &allow).unwrap();
    let command = f.command("completed-stop", wire.clone());
    let lease = admitted(&f, &store, &ctx, &command).lease.unwrap();
    store.execution_start_intent(&ctx, &lease).unwrap();
    store.execution_running(&ctx, &lease).unwrap();
    let complete = store
        .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Completed))
        .unwrap();
    assert!(
        store
            .execution_stored(&command.identity)
            .unwrap()
            .unwrap()
            .reservation
    );
    store
        .set_grant(&f.scope, &f.export.id, &principal(), &[], false, Some(1))
        .unwrap();
    let stopped = store
        .finish_stopped_execution(&f.scope, &command.identity, lease.fence(), |_| Ok(()))
        .unwrap();
    assert_eq!(stopped, complete);
    assert!(
        !store
            .execution_stored(&command.identity)
            .unwrap()
            .unwrap()
            .reservation
    );
}

#[test]
fn outbox_filters_before_paging_and_denied_rows_cannot_starve_a_readable_principal() {
    for same_fence in [false, true] {
        let f = Fixture::new();
        let store = f.store();
        let current = channel("current");
        let previous = if same_fence {
            current.clone()
        } else {
            channel("historical")
        };
        let old_ctx = ExecutionChannel::verify(&f.scope, &previous, &allow).unwrap();
        for index in 0..8 {
            let command = f.command(&format!("a-denied-{index}"), previous.clone());
            let lease = admitted(&f, &store, &old_ctx, &command).lease.unwrap();
            store
                .finish_execution(&old_ctx, &lease, &outcome(ExecutionTerminalStatus::Failed))
                .unwrap();
        }
        store
            .set_grant(&f.scope, &f.export.id, &principal(), &[], false, Some(1))
            .unwrap();
        let readable = ExportPrincipal {
            kind: PrincipalKind::Human,
            id: "readable-human".into(),
        };
        let grant = store
            .set_grant(
                &f.scope,
                &f.export.id,
                &readable,
                &[ExportPermission::Read, ExportPermission::Execute],
                true,
                None,
            )
            .unwrap();
        let ctx = ExecutionChannel::verify(&f.scope, &current, &allow).unwrap();
        let mut command = f.command("z-readable", current.clone());
        command.descriptor.grant =
            serde_json::from_value(serde_json::to_value(grant).unwrap()).unwrap();
        command.identity.fingerprint =
            crate::worker_execution::command_fingerprint(&command).unwrap();
        let validated = store
            .candidate(&f.scope, &f.export.id, &readable, ExportPermission::Execute)
            .unwrap()
            .validate_filesystem()
            .unwrap();
        let lease = store
            .admit_execution(&ctx, &command, validated)
            .unwrap()
            .lease
            .unwrap();
        let result = store
            .finish_execution(&ctx, &lease, &outcome(ExecutionTerminalStatus::Failed))
            .unwrap();
        assert_eq!(store.execution_outbox(&ctx).unwrap(), vec![result]);
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM worker_execution_outbox", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            9,
            "Filtering must not delete history or denied payloads"
        );
        let denied_identity = f.command("a-denied-0", previous.clone()).identity;
        assert!(store.reconcile_execution(&ctx, &denied_identity).is_err());
    }
}

fn downgrade_to_execution_v3(store: &WorkerStore) {
    store.connection.execute_batch("DROP TABLE worker_session_events; DROP TABLE worker_session_turns; DROP TABLE worker_sessions; DROP TABLE worker_session_meta; PRAGMA user_version=3;").unwrap();
}
#[test]
fn worker_session_operator_upgrade_v3_preserves_receipt_and_peer_never_migrates() {
    let f = Fixture::new();
    let store = f.store();
    let fence = channel("session-upgrade");
    let context = ExecutionChannel::verify(&f.scope, &fence, &allow).unwrap();
    let command = f.command("preserve-v3", fence.clone());
    let accepted = admitted(&f, &store, &context, &command);
    downgrade_to_execution_v3(&store);
    drop(store);
    let peer =
        WorkerStore::open_existing(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&peer), 3);
    for table in [
        "worker_session_meta",
        "worker_sessions",
        "worker_session_turns",
        "worker_session_events",
    ] {
        assert!(!table_exists(&peer.connection, table));
    }
    let retry = admitted(&f, &peer, &context, &command);
    assert!(retry.lease.is_none());
    assert_eq!(retry.record, accepted.record);
    assert_eq!(version(&peer), 3);
    drop(peer);
    let operator = WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&operator), 4);
    let retry = admitted(&f, &operator, &context, &command);
    assert!(retry.lease.is_none());
    assert_eq!(retry.record, accepted.record);
    let (epoch, reservation): (i64, i64) = operator.connection.query_row(
        "SELECT (SELECT next_epoch FROM worker_execution_meta), reservation FROM worker_execution_inbox WHERE attempt_id=?1",
        [&command.identity.attempt_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(epoch, 2);
    assert_eq!(reservation, 1, "migration is not a process stop proof");
    assert_eq!(operator.list(&f.scope).unwrap(), vec![f.export.clone()]);
    assert_eq!(
        operator.grants(&f.scope, &f.export.id).unwrap(),
        vec![f.grant.clone()]
    );
}
#[test]
fn worker_session_last_table_collision_rolls_back_upgrade_and_retains_v3_identity() {
    let f = Fixture::new();
    let store = f.store();
    let fence = channel("session-collision");
    let context = ExecutionChannel::verify(&f.scope, &fence, &allow).unwrap();
    let command = f.command("preserve-collision", fence.clone());
    let accepted = admitted(&f, &store, &context, &command);
    downgrade_to_execution_v3(&store);
    store.connection.execute_batch("CREATE TABLE worker_session_events(marker TEXT); INSERT INTO worker_session_events VALUES('original');").unwrap();
    drop(store);
    assert!(WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).is_err());
    let peer =
        WorkerStore::open_existing(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&peer), 3);
    for table in [
        "worker_session_meta",
        "worker_sessions",
        "worker_session_turns",
    ] {
        assert!(!table_exists(&peer.connection, table));
    }
    assert_eq!(
        peer.connection
            .query_row("SELECT marker FROM worker_session_events", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "original"
    );
    let retry = admitted(&f, &peer, &context, &command);
    assert!(retry.lease.is_none());
    assert_eq!(retry.record, accepted.record);
    peer.connection
        .execute_batch("DROP TABLE worker_session_events;")
        .unwrap();
    drop(peer);
    let operator = WorkerStore::open(&f.directory.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(version(&operator), 4);
    assert_eq!(
        admitted(&f, &operator, &context, &command).record,
        accepted.record
    );
}
