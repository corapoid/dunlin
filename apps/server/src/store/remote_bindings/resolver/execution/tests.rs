//! Controlled SQL/opaque-observation fixtures, NOT negotiated v3 WSS evidence.
use super::*;
use dunlin_relay_protocol::worker::{
    AttemptFence, ExecutionOutcome, ExecutionReceipt, ExecutionTerminalStatus,
};
use serde_json::json;
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    owner: String,
    device: String,
    workspace: String,
    project: String,
    identity: WorkerIdentity,
    lease: ChannelLease,
    descriptor: dunlin_relay_protocol::worker::ExportDescriptor,
    binding: BindingRef,
    profile: AgentProfileRef,
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
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
    assert_eq!(store.active_device_user(device)?.unwrap().id, owner);
    assert_eq!(identity.host_id(), lease.host_id);
    assert!(channel_matches(lease, fence));
    Ok(())
}
fn attest_record(
    store: &Store,
    command: &ExecutionCommand,
    record: &ExecutionRecord,
) -> Result<()> {
    assert!(!store.connection.is_autocommit());
    assert_eq!(command.identity, record.receipt.identity);
    Ok(())
}
fn attest_ack(
    store: &Store,
    command: &ExecutionCommand,
    identity: &WorkerIdentity,
    lease: &ChannelLease,
    channel: &ChannelFence,
) -> Result<()> {
    assert!(!store.connection.is_autocommit());
    assert_eq!(command.descriptor.scope.host_id, identity.host_id());
    assert!(channel_matches(lease, channel));
    Ok(())
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let owner = store.create_user("execution-owner", "admin").unwrap();
        let (device, _) = store.create_device(&owner.id, "execution-device").unwrap();
        let workspace = store.create_workspace(&owner, "Executions").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Remote target")
            .unwrap();
        let host=store.create_host(&owner.id,&workspace.id,serde_json::from_value(json!({"name":"Execution host","node_id":"worker-node",
            "serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":4,"harnesses":["claude"]}})).unwrap()).unwrap();
        let credential=store.enroll_host(&host.enrollment.ticket,serde_json::from_value(json!({"protocol_version":1,"version":"fixture",
            "os":"linux","architecture":"x86_64","harnesses":["claude"],"sandbox":"linux_bubblewrap","max_parallel":4})).unwrap()).unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &host.host.id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker(&credential.credential, &host.host.id, 1, 1)
            .unwrap();
        let lease = store
            .open_worker_channel(
                &identity,
                "controller-boot",
                "worker-boot",
                "channel-1",
                now(),
                15000,
            )
            .unwrap();
        let descriptor:dunlin_relay_protocol::worker::ExportDescriptor=serde_json::from_value(json!({"scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node",
            "workspace_id":workspace.id,"host_id":host.host.id,"enrollment_epoch":1,"credential_revision":1},
            "export":{"id":"export-1","name":"Private export","repo_kind":"git","checkout_identity_digest":"a".repeat(64),
                "settings_digest":"b".repeat(64),"revision":1,"active":true},
            "grant":{"principal":{"kind":"human","id":owner.id},"permissions":["read","execute"],"revision":1,"export_revision":1,"active":true}})).unwrap();
        let proof =
            ObservedExport::for_test(identity.clone(), lease.clone(), descriptor.clone(), now());
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
                "export-1",
                1,
                &proof,
                "controller-boot",
                "controller-node",
                now(),
                &authority,
            )
            .unwrap();
        let definition=serde_json::from_value(json!({"schema_version":1,"name":"Execution fixture","description":"","harness":"claude",
            "model":"configured-model","mode":"configured-mode","instructions":null,"skills":[],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        let draft = store
            .create_agent(&owner.id, &workspace.id, &project.id, definition)
            .unwrap();
        let published = store
            .publish_agent(&owner.id, &workspace.id, &project.id, &draft.id, 1)
            .unwrap();
        Self {
            dir,
            store,
            owner: owner.id,
            device: device.id,
            workspace: workspace.id,
            project: project.id,
            identity,
            lease,
            descriptor,
            binding: BindingRef {
                id: binding.id,
                revision: 1,
            },
            profile: AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            },
        }
    }
    fn request(&self, key: &str) -> RemoteExecutionRequest {
        RemoteExecutionRequest {
            intent_key: key.into(),
            binding: self.binding.clone(),
            profile: self.profile.clone(),
            input: "explicit private input".into(),
            budgets: ExecutionBudgets {
                max_seconds: 60,
                max_output_bytes: 32768,
                lease_duration_ms: 5000,
            },
        }
    }
    fn prepare(&self, store: &Store, key: &str) -> PreparedRemoteExecution {
        store
            .remote_binding_transaction(false, |store| {
                store.prepare_remote_execution_in_transaction(
                    &self.owner,
                    &self.device,
                    &self.workspace,
                    &self.project,
                    self.request(key),
                    "controller-node",
                )
            })
            .unwrap()
    }
    fn channel(&self) -> ChannelFence {
        ChannelFence {
            channel_id: self.lease.channel_id.clone(),
            controller_boot_id: self.lease.controller_boot_id.clone(),
            worker_boot_id: self.lease.worker_boot_id.clone(),
        }
    }
    fn proof(&self) -> ObservedExport {
        ObservedExport::for_test(
            self.identity.clone(),
            self.lease.clone(),
            self.descriptor.clone(),
            now(),
        )
    }
    fn admit(&self, key: &str) -> RemoteExecutionReservation {
        self.store
            .admit_remote_execution(
                self.prepare(&self.store, key),
                self.proof(),
                self.channel(),
                now(),
                attest,
            )
            .unwrap()
    }
    fn dispatch(&self, attempt: &str, key: &str) -> Result<Option<ExecutionCommand>> {
        self.store.dispatch_remote_execution(
            attempt,
            self.prepare(&self.store, key),
            self.proof(),
            self.channel(),
            now(),
            attest,
        )
    }
    fn count(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
fn accepted(command: &ExecutionCommand) -> ExecutionRecord {
    ExecutionRecord {
        receipt: ExecutionReceipt {
            fence: AttemptFence {
                channel: command.channel.clone(),
                attempt_id: command.identity.attempt_id.clone(),
                execution_epoch: 1,
            },
            identity: command.identity.clone(),
            lease_duration_ms: command.budgets.lease_duration_ms,
        },
        status: ExecutionStatus::Accepted,
        outcome: None,
    }
}
fn completed(command: &ExecutionCommand) -> ExecutionRecord {
    let mut record = accepted(command);
    let mut outcome = ExecutionOutcome {
        outcome_id: "outcome-1".into(),
        digest: String::new(),
        status: ExecutionTerminalStatus::Completed,
        text: "bounded result".into(),
    };
    outcome.digest = outcome_digest(&outcome).unwrap();
    record.status = ExecutionStatus::Completed;
    record.outcome = Some(outcome);
    record
}
#[test]
fn identical_intent_retains_one_command_and_lost_dispatch_ack_never_replays_positive_work() {
    let f = Fixture::new();
    let key = "intent-lost-ack-0001";
    let reservation = f.admit(key);
    let retry = f.admit(key);
    assert!(retry.replayed);
    assert_eq!(reservation.identity, retry.identity);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    assert_eq!(command.identity, reservation.identity);
    assert!(
        f.dispatch(&reservation.identity.attempt_id, key)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.count("controller_remote_executions"), 1);
    let mut request = f.request(key);
    request.input.push_str(" altered");
    let prepared = f
        .store
        .remote_binding_transaction(false, |store| {
            store.prepare_remote_execution_in_transaction(
                &f.owner,
                &f.device,
                &f.workspace,
                &f.project,
                request,
                "controller-node",
            )
        })
        .unwrap();
    assert!(
        f.store
            .admit_remote_execution(prepared, f.proof(), f.channel(), now(), attest)
            .is_err()
    );
    assert_eq!(f.count("controller_remote_executions"), 1);
}
#[test]
fn v2_observation_without_live_v3_attestation_and_late_callback_failure_cannot_reserve() {
    let f = Fixture::new();
    let mut changes = f.store.subscribe_catalog();
    let result = f.store.admit_remote_execution(
        f.prepare(&f.store, "intent-no-v3-0001"),
        f.proof(),
        f.channel(),
        now(),
        |store, _, _, _, _, _| {
            store.connection.execute(
                "UPDATE team_hosts SET drain=1 WHERE id=?1",
                [f.identity.host_id()],
            )?;
            Err(denied())
        },
    );
    assert!(result.is_err());
    assert_eq!(f.count("controller_remote_executions"), 0);
    assert!(
        !f.store
            .connection
            .query_row(
                "SELECT drain FROM team_hosts WHERE id=?1",
                [f.identity.host_id()],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert!(matches!(
        changes.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    let mut read = f.descriptor.clone();
    read.grant.permissions = vec![ExportPermission::Read];
    let proof = ObservedExport::for_test(f.identity.clone(), f.lease.clone(), read, now());
    assert!(
        f.store
            .admit_remote_execution(
                f.prepare(&f.store, "intent-read-only-0001"),
                proof,
                f.channel(),
                now(),
                attest
            )
            .is_err()
    );
}
#[test]
fn terminal_inbox_is_exact_fenced_digest_checked_and_applied_once_across_reopen() {
    let f = Fixture::new();
    let key = "intent-outcome-0001";
    let reservation = f.admit(key);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    let record = completed(&command);
    assert!(
        f.store
            .record_remote_execution(&command.identity.attempt_id, record.clone(), attest_record)
            .unwrap()
    );
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    assert!(
        !reopened
            .record_remote_execution(&command.identity.attempt_id, record.clone(), attest_record)
            .unwrap()
    );
    assert_eq!(f.count("controller_remote_execution_outcomes"), 1);
    let mut wrong = record.clone();
    wrong.receipt.fence.execution_epoch = 2;
    assert!(
        f.store
            .record_remote_execution(&command.identity.attempt_id, wrong, attest_record)
            .is_err()
    );
    let mut wrong = record.clone();
    wrong.outcome.as_mut().unwrap().text = "tampered".into();
    assert!(
        f.store
            .record_remote_execution(&command.identity.attempt_id, wrong, attest_record)
            .is_err()
    );
    let mut wrong = record;
    wrong.receipt.fence.channel.worker_boot_id = "other-worker".into();
    assert!(
        f.store
            .record_remote_execution(&command.identity.attempt_id, wrong, attest_record)
            .is_err()
    );
    assert_eq!(f.count("controller_remote_execution_outcomes"), 1);
}
#[test]
fn each_renewal_pins_actual_worker_execute_and_logical_epochs_and_exact_duplicate_sequence() {
    let mut f = Fixture::new();
    let key = "intent-renewal-0001";
    let reservation = f.admit(key);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            accepted(&command),
            attest_record,
        )
        .unwrap();
    let renew = |f: &Fixture, sequence, duration| {
        f.store.renew_remote_execution(
            &command.identity.attempt_id,
            f.prepare(&f.store, key),
            f.proof(),
            f.channel(),
            now(),
            sequence,
            duration,
            attest,
        )
    };
    assert_eq!(renew(&f, 1, 1000).unwrap(), renew(&f, 1, 1000).unwrap());
    assert!(renew(&f, 1, 2000).is_err());
    assert!(renew(&f, 3, 1000).is_err());
    f.descriptor.grant.revision = 3; // current active grant after revoke/regrant
    assert!(renew(&f, 2, 1000).is_err());
    f.descriptor.grant.revision = 1;
    f.store.connection.execute("UPDATE team_project_grants SET revision=revision+2 WHERE workspace_id=?1 AND project_id=?2 AND user_id=?3",params![f.workspace,f.project,f.owner]).unwrap();
    assert!(renew(&f, 2, 1000).is_err());
}
#[test]
fn recovery_reserves_unknown_and_new_channel_reconcile_neither_renews_nor_dispatches() {
    let f = Fixture::new();
    let key = "intent-recover-0001";
    let reservation = f.admit(key);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            accepted(&command),
            attest_record,
        )
        .unwrap();
    assert_eq!(f.store.recover_remote_executions().unwrap(), 1);
    assert!(
        f.dispatch(&command.identity.attempt_id, key)
            .unwrap()
            .is_none()
    );
    assert!(
        f.store
            .renew_remote_execution(
                &command.identity.attempt_id,
                f.prepare(&f.store, key),
                f.proof(),
                f.channel(),
                now(),
                1,
                1000,
                attest
            )
            .is_err()
    );
    let lease = f
        .store
        .open_worker_channel(
            &f.identity,
            "new-controller-boot",
            "new-worker-boot",
            "new-channel",
            now(),
            15000,
        )
        .unwrap();
    let channel = ChannelFence {
        channel_id: lease.channel_id.clone(),
        controller_boot_id: lease.controller_boot_id.clone(),
        worker_boot_id: lease.worker_boot_id.clone(),
    };
    let query = f
        .store
        .reconcile_remote_execution(
            &f.owner,
            &f.device,
            &command.identity.attempt_id,
            channel.clone(),
            &f.identity,
            &lease,
            now(),
            attest,
        )
        .unwrap();
    assert_eq!(
        query,
        ExecutionControllerMessage::Reconcile {
            channel: channel.clone(),
            identity: command.identity.clone()
        }
    );
    let record = completed(&command);
    f.store
        .record_remote_execution(&command.identity.attempt_id, record.clone(), attest_record)
        .unwrap();
    let ack = f
        .store
        .remote_execution_outcome_ack(
            &command.identity.attempt_id,
            channel,
            &f.identity,
            &lease,
            now(),
            attest_ack,
        )
        .unwrap();
    assert!(
        matches!(ack,ExecutionControllerMessage::OutcomeAck{fence,..} if fence==record.receipt.fence)
    );
}
#[test]
fn private_owner_device_scope_and_revocation_are_required_even_for_ledger_reads() {
    let f = Fixture::new();
    let reservation = f.admit("intent-owner-0001");
    let other = f
        .store
        .create_user("other-execution-owner", "admin")
        .unwrap();
    let (device, _) = f.store.create_device(&other.id, "other-browser").unwrap();
    let read = |owner: &str, device: &str| {
        f.store.remote_execution(
            owner,
            device,
            &reservation.identity.attempt_id,
            |_, _, _| Ok(()),
        )
    };
    assert!(read(&other.id, &device.id).is_err());
    assert!(read(&f.owner, &device.id).is_err());
    assert!(read(&f.owner, &f.device).is_ok());
    f.store
        .connection
        .execute("UPDATE devices SET revoked_at=1 WHERE id=?1", [&f.device])
        .unwrap();
    assert!(read(&f.owner, &f.device).is_err());
}

#[test]
fn two_independent_connections_racing_the_same_intent_commit_one_reservation() {
    let f = Fixture::new();
    let key = "intent-two-connections-0001";
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let prepared = f.prepare(&f.store, key);
        let proof = f.proof();
        let channel = f.channel();
        let path = f.dir.path().join("relay.db");
        let gate = gate.clone();
        workers.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            gate.wait();
            store
                .admit_remote_execution(prepared, proof, channel, now(), attest)
                .unwrap()
        }));
    }
    let first = workers.remove(0).join().unwrap();
    let second = workers.remove(0).join().unwrap();
    assert_eq!(first.identity, second.identity);
    assert_ne!(first.replayed, second.replayed);
    assert_eq!(f.count("controller_remote_executions"), 1);
}

#[test]
fn historical_intent_lookup_survives_lost_http_response_and_offline_recovery_without_positive_authority()
 {
    let f = Fixture::new();
    let key = "intent-offline-retry-0001";
    let first = f.admit(key);
    f.store.close_worker_channel(&f.identity, &f.lease).unwrap();
    f.store.recover_remote_executions().unwrap();
    let prepared = f.prepare(&f.store, key);
    let retry = f
        .store
        .remote_binding_transaction(false, |store| {
            store.lookup_remote_execution_intent_in_transaction(
                &prepared,
                |store, owner, device| {
                    assert!(!store.connection.is_autocommit());
                    assert_eq!(store.active_device_user(device)?.unwrap().id, owner);
                    Ok(())
                },
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(retry.identity, first.identity);
    assert_eq!(retry.phase, RemoteExecutionPhase::Unknown);
    assert!(retry.replayed);
    assert_eq!(f.count("controller_remote_executions"), 1);
    assert!(f.dispatch(&first.identity.attempt_id, key).is_err());
}

#[test]
fn cancellation_before_dispatch_suppresses_locally_without_fabricating_a_worker_epoch() {
    let f = Fixture::new();
    let key = "intent-cancel-pending-0001";
    let reservation = f.admit(key);
    assert!(
        f.store
            .cancel_remote_execution(
                &f.owner,
                &f.device,
                &reservation.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .unwrap()
            .is_none()
    );
    let summary = f
        .store
        .remote_execution(
            &f.owner,
            &f.device,
            &reservation.identity.attempt_id,
            |_, _, _| Ok(()),
        )
        .unwrap();
    assert_eq!(summary.phase, RemoteExecutionPhase::Terminal);
    assert!(summary.record.is_none());
    assert!(
        f.dispatch(&reservation.identity.attempt_id, key)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.count("controller_remote_execution_outcomes"), 0);
    let row = f
        .store
        .remote_execution_row(&reservation.identity.attempt_id)
        .unwrap()
        .unwrap();
    assert!(
        f.store
            .record_remote_execution(
                &reservation.identity.attempt_id,
                accepted(&row.command),
                attest_record
            )
            .is_err()
    );
}

#[test]
fn cancellation_after_dispatch_with_lost_acceptance_retains_unknown_and_late_receipt_never_enables_renewal()
 {
    let f = Fixture::new();
    let key = "intent-cancel-lost-0001";
    let reservation = f.admit(key);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    assert!(
        f.store
            .cancel_remote_execution(
                &f.owner,
                &f.device,
                &command.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .unwrap()
            .is_none()
    );
    let retry = f.admit(key);
    assert_eq!(retry.identity, command.identity);
    assert_eq!(retry.phase, RemoteExecutionPhase::Unknown);
    assert!(
        f.dispatch(&command.identity.attempt_id, key)
            .unwrap()
            .is_none()
    );
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            accepted(&command),
            attest_record,
        )
        .unwrap();
    let retry = f.admit(key);
    assert_eq!(retry.phase, RemoteExecutionPhase::Unknown);
    assert!(
        f.store
            .renew_remote_execution(
                &command.identity.attempt_id,
                f.prepare(&f.store, key),
                f.proof(),
                f.channel(),
                now(),
                1,
                1000,
                attest
            )
            .is_err()
    );
    let cancel = f
        .store
        .cancel_remote_execution_from_guard(&command.identity.attempt_id, |_, stored| {
            assert_eq!(stored.identity, command.identity);
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        cancel,
        ExecutionControllerMessage::Cancel {
            fence: accepted(&command).receipt.fence
        }
    );
}

#[test]
fn revoked_user_permissions_cannot_block_producer_cancel_or_durable_terminal_ack_cleanup() {
    let f = Fixture::new();
    let key = "intent-cleanup-revoked-0001";
    let reservation = f.admit(key);
    let command = f
        .dispatch(&reservation.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            accepted(&command),
            attest_record,
        )
        .unwrap();
    f.store
        .connection
        .execute("UPDATE devices SET revoked_at=1 WHERE id=?1", [&f.device])
        .unwrap();
    f.store.connection.execute("UPDATE team_project_grants SET permissions='[]',revision=revision+1 WHERE workspace_id=?1 AND project_id=?2 AND user_id=?3",params![f.workspace,f.project,f.owner]).unwrap();
    assert!(
        f.store
            .remote_execution(
                &f.owner,
                &f.device,
                &command.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .is_err()
    );
    assert!(
        f.store
            .cancel_remote_execution(
                &f.owner,
                &f.device,
                &command.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .is_err()
    );
    assert!(
        f.store
            .cancel_remote_execution_from_guard(&command.identity.attempt_id, |_, stored| {
                assert_eq!(stored.identity, command.identity);
                Ok(())
            })
            .unwrap()
            .is_some()
    );
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            completed(&command),
            attest_record,
        )
        .unwrap();
    let ack = f
        .store
        .remote_execution_outcome_ack(
            &command.identity.attempt_id,
            f.channel(),
            &f.identity,
            &f.lease,
            now(),
            attest_ack,
        )
        .unwrap();
    assert!(
        matches!(ack,ExecutionControllerMessage::OutcomeAck{identity,..} if identity==command.identity)
    );
    // A syntactically valid terminal row is insufficient without its durable
    // atomic inbox receipt: never ACK away the worker's last recoverable copy.
    f.store
        .connection
        .execute(
            "DELETE FROM controller_remote_execution_outcomes WHERE attempt_id=?1",
            [&command.identity.attempt_id],
        )
        .unwrap();
    assert!(
        f.store
            .remote_execution_outcome_ack(
                &command.identity.attempt_id,
                f.channel(),
                &f.identity,
                &f.lease,
                now(),
                attest_ack
            )
            .is_err()
    );
}

#[test]
fn migration_35_preserves_v34_local_remote_bindings_and_existing_logical_native_grants_without_attempts()
 {
    let f = Fixture::new();
    let path = f.dir.path().join("relay.db");
    let local = f.dir.path().join("ordinary-checkout");
    std::fs::create_dir(&local).unwrap();
    let native = f
        .store
        .create_project("ordinary", local.to_str().unwrap())
        .unwrap();
    f.store.grant(&native.id, &f.owner).unwrap();
    let local_binding = f
        .store
        .set_local_binding(&f.owner, &f.workspace, &f.project, &native.id, None)
        .unwrap();
    let remote = f
        .store
        .remote_binding_row(&f.workspace, &f.project, &f.binding.id)
        .unwrap()
        .unwrap();
    let native_grant = f.store.capture_native_grant(&native.id, &f.owner).unwrap();
    let logical = f
        .store
        .authorize_team_project(
            &f.owner,
            &f.workspace,
            &f.project,
            Permission::CreateSession,
        )
        .unwrap();
    let audit: i64 = f
        .store
        .connection
        .query_row("SELECT count(*) FROM team_audit", [], |r| r.get(0))
        .unwrap();
    f.store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; PRAGMA user_version=34;").unwrap();
    drop(f.store);
    let reopened = Store::open(path).unwrap();
    assert_eq!(
        reopened
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    assert_eq!(
        reopened
            .local_binding_for_user(&f.owner, &f.workspace, &f.project)
            .unwrap()
            .unwrap(),
        local_binding
    );
    assert_eq!(
        reopened
            .remote_binding_row(&f.workspace, &f.project, &f.binding.id)
            .unwrap()
            .unwrap(),
        remote
    );
    assert_eq!(
        reopened.capture_native_grant(&native.id, &f.owner).unwrap(),
        native_grant
    );
    assert_eq!(
        reopened
            .authorize_team_project(
                &f.owner,
                &f.workspace,
                &f.project,
                Permission::CreateSession
            )
            .unwrap(),
        logical
    );
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT count(*) FROM team_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        audit
    );
    for table in [
        "controller_remote_executions",
        "controller_remote_execution_outcomes",
    ] {
        assert_eq!(
            reopened
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn migration_35_second_table_collision_rolls_back_first_table_and_version_then_recovers_after_only_collision_removal()
 {
    let f = Fixture::new();
    let path = f.dir.path().join("relay.db");
    let remote = f
        .store
        .remote_binding_row(&f.workspace, &f.project, &f.binding.id)
        .unwrap()
        .unwrap();
    f.store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; PRAGMA user_version=34; CREATE TABLE controller_remote_execution_outcomes(collision_marker TEXT); INSERT INTO controller_remote_execution_outcomes VALUES('keep collision data');").unwrap();
    drop(f.store);
    assert!(Store::open(&path).is_err());
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        34
    );
    assert!(!raw.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_remote_executions')",[],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        raw.query_row(
            "SELECT collision_marker FROM controller_remote_execution_outcomes",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "keep collision data"
    );
    raw.execute_batch("DROP TABLE controller_remote_execution_outcomes;")
        .unwrap();
    drop(raw);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
    assert_eq!(
        reopened
            .remote_binding_row(&f.workspace, &f.project, &f.binding.id)
            .unwrap()
            .unwrap(),
        remote
    );
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT count(*) FROM controller_remote_executions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT count(*) FROM controller_remote_execution_outcomes",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn persisted_renewal_sequence_must_be_a_nonnegative_safe_sql_integer() {
    let f = Fixture::new();
    let reservation = f.admit("intent-sql-sequence-0001");
    assert_eq!(execution_sql_integer(0).unwrap(), 0);
    assert_eq!(
        execution_sql_integer(MAX_EXECUTION_INTEGER).unwrap(),
        9_007_199_254_740_991_i64
    );
    assert!(execution_sql_integer(MAX_EXECUTION_INTEGER + 1).is_err());
    assert!(execution_sql_integer(u64::MAX).is_err());
    // SQLite accepts signed i64 values above JavaScript's safe-integer range.
    // A persisted value must not bypass the wire's counter/fence bound.
    f.store.connection.execute("UPDATE controller_remote_executions SET last_renew_sequence=9007199254740992 WHERE attempt_id=?1",
        [&reservation.identity.attempt_id]).unwrap();
    assert!(
        f.store
            .remote_execution(
                &f.owner,
                &f.device,
                &reservation.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .is_err()
    );
    f.store
        .connection
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    f.store
        .connection
        .execute(
            "UPDATE controller_remote_executions SET last_renew_sequence=-1 WHERE attempt_id=?1",
            [&reservation.identity.attempt_id],
        )
        .unwrap();
    assert!(
        f.store
            .remote_execution(
                &f.owner,
                &f.device,
                &reservation.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .is_err()
    );
}

#[test]
fn durable_ack_pending_clears_only_after_mandatory_matching_receipt_attestation() {
    let f = Fixture::new();
    let key = "intent-durable-ack-0001";
    let reserved = f.admit(key);
    let command = f
        .dispatch(&reserved.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            completed(&command),
            attest_record,
        )
        .unwrap();
    let message = f
        .store
        .remote_execution_outcome_ack(
            &command.identity.attempt_id,
            f.channel(),
            &f.identity,
            &f.lease,
            now(),
            attest_ack,
        )
        .unwrap();
    let pending = || {
        f.store
            .connection
            .query_row(
                "SELECT ack_pending FROM controller_remote_execution_outcomes WHERE attempt_id=?1",
                [&command.identity.attempt_id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    };
    // Producing/writing an ACK request is not a durable worker receipt.
    assert!(pending());
    assert!(
        f.store
            .confirm_remote_execution_ack(&command.identity.attempt_id, &message, |_, _, _| Err(
                denied()
            ))
            .is_err()
    );
    assert!(pending());
    let mut wrong = message.clone();
    if let ExecutionControllerMessage::OutcomeAck { outcome_digest, .. } = &mut wrong {
        *outcome_digest = "f".repeat(64);
    }
    assert!(
        f.store
            .confirm_remote_execution_ack(&command.identity.attempt_id, &wrong, |_, _, _| Ok(()))
            .is_err()
    );
    assert!(pending());
    let attest = |store: &Store, stored: &ExecutionCommand, ack: &ExecutionControllerMessage| {
        assert!(!store.connection.is_autocommit());
        assert_eq!(stored.identity, command.identity);
        assert_eq!(ack, &message);
        Ok(())
    };
    assert!(
        f.store
            .confirm_remote_execution_ack(&command.identity.attempt_id, &message, attest)
            .unwrap()
    );
    assert!(!pending());
    assert!(
        !f.store
            .confirm_remote_execution_ack(&command.identity.attempt_id, &message, attest)
            .unwrap()
    );
    assert!(
        f.store
            .remote_execution_cleanup(8, None, |_, _| Ok(()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn lost_accepted_and_revoked_user_allow_only_producer_reconcile_terminal_inbox_and_ack() {
    let f = Fixture::new();
    let key = "intent-lost-revoked-0001";
    let reserved = f.admit(key);
    let command = f
        .dispatch(&reserved.identity.attempt_id, key)
        .unwrap()
        .unwrap();
    f.store
        .connection
        .execute("UPDATE devices SET revoked_at=1 WHERE id=?1", [&f.device])
        .unwrap();
    f.store.connection.execute("UPDATE team_project_grants SET permissions='[]',revision=revision+1 WHERE workspace_id=?1 AND project_id=?2 AND user_id=?3",params![f.workspace,f.project,f.owner]).unwrap();
    assert!(
        f.store
            .remote_execution(
                &f.owner,
                &f.device,
                &command.identity.attempt_id,
                |_, _, _| Ok(())
            )
            .is_err()
    );
    let query = f
        .store
        .reconcile_remote_execution_from_guard(
            &command.identity.attempt_id,
            f.channel(),
            &f.identity,
            &f.lease,
            now(),
            attest_ack,
        )
        .unwrap();
    assert_eq!(
        query,
        ExecutionControllerMessage::Reconcile {
            channel: f.channel(),
            identity: command.identity.clone()
        }
    );
    assert_eq!(f.count("controller_remote_executions"), 1);
    f.store
        .record_remote_execution(
            &command.identity.attempt_id,
            completed(&command),
            attest_record,
        )
        .unwrap();
    let ack = f
        .store
        .remote_execution_outcome_ack(
            &command.identity.attempt_id,
            f.channel(),
            &f.identity,
            &f.lease,
            now(),
            attest_ack,
        )
        .unwrap();
    assert!(
        matches!(ack,ExecutionControllerMessage::OutcomeAck{identity,..} if identity==command.identity)
    );
    assert_eq!(f.count("controller_remote_executions"), 1);
    assert_eq!(f.count("controller_remote_execution_outcomes"), 1);
}

#[test]
fn remote_execution_admission_can_share_outer_cut_and_rolls_back_after_late_failure() {
    let f = Fixture::new();
    let key = "intent-atomic-turn-0001";
    let callback = std::cell::Cell::new(false);
    assert!(
        f.store
            .admit_remote_execution_in_transaction(
                f.prepare(&f.store, key),
                f.proof(),
                f.channel(),
                now(),
                |_, _, _, _, _, _| {
                    callback.set(true);
                    Ok(())
                },
            )
            .is_err()
    );
    assert!(
        !callback.get(),
        "no admission authority outside transaction"
    );
    assert_eq!(f.count("controller_remote_executions"), 0);
    let prepared = f.prepare(&f.store, key);
    let result: Result<()> = f.store.remote_binding_transaction(false, |store| {
        store.admit_remote_execution_in_transaction(
            prepared,
            f.proof(),
            f.channel(),
            now(),
            attest,
        )?;
        assert_eq!(f.count("controller_remote_executions"), 1);
        // A consumer's later SQL failure must roll back the command as well.
        store
            .connection
            .execute("INSERT INTO missing_session_turn_mapping VALUES(1)", [])?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(f.count("controller_remote_executions"), 0);
    let prepared = f.prepare(&f.store, key);
    let committed = f
        .store
        .remote_binding_transaction(false, |store| {
            store.admit_remote_execution_in_transaction(
                prepared,
                f.proof(),
                f.channel(),
                now(),
                attest,
            )
        })
        .unwrap();
    assert!(!committed.replayed);
    assert_eq!(f.admit(key).identity, committed.identity);
    assert_eq!(f.count("controller_remote_executions"), 1);
}

#[test]
fn remote_execution_terminal_record_and_ack_intent_rollback_with_outer_projection_failure() {
    let f = Fixture::new();
    let key = "intent-atomic-result-01";
    let accepted = f.admit(key);
    let attempt = &accepted.identity.attempt_id;
    let command = f.dispatch(attempt, key).unwrap().unwrap();
    let record = completed(&command);
    let callback = std::cell::Cell::new(false);
    assert!(
        f.store
            .record_remote_execution_in_transaction(attempt, record.clone(), |_, _, _| {
                callback.set(true);
                Ok(())
            })
            .is_err()
    );
    assert!(!callback.get());
    let failed: Result<()> = f.store.remote_binding_transaction(false, |store| {
        assert!(store.record_remote_execution_in_transaction(
            attempt,
            record.clone(),
            attest_record
        )?);
        assert_eq!(f.count("controller_remote_execution_outcomes"), 1);
        store
            .connection
            .execute("INSERT INTO missing_transcript_projection VALUES(1)", [])?;
        Ok(())
    });
    assert!(failed.is_err());
    assert_eq!(f.count("controller_remote_execution_outcomes"), 0);
    let previous = f.store.remote_execution_row(attempt).unwrap().unwrap();
    assert_eq!(previous.phase, RemoteExecutionPhase::Dispatched);
    assert!(previous.record.is_none());
    f.store
        .remote_binding_transaction(false, |store| {
            store.record_remote_execution_in_transaction(attempt, record.clone(), attest_record)
        })
        .unwrap();
    let committed = f.store.remote_execution_row(attempt).unwrap().unwrap();
    assert_eq!(committed.phase, RemoteExecutionPhase::Terminal);
    assert_eq!(committed.record, Some(record));
    assert_eq!(f.count("controller_remote_execution_outcomes"), 1);
    let pending: i64 = f
        .store
        .connection
        .query_row(
            "SELECT ack_pending FROM controller_remote_execution_outcomes WHERE attempt_id=?1",
            [attempt],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending, 1,
        "consumer must commit transcript before sending ACK"
    );
}

#[test]
fn mcp_execution_profile_is_rejected_before_durable_admission() {
    let mut f = Fixture::new();
    let mut definition = f
        .store
        .agent_version(
            &f.owner,
            &f.workspace,
            &f.project,
            &f.profile.agent_id,
            f.profile.version,
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
    f.profile = AgentProfileRef {
        agent_id: version.agent_id,
        version: version.version,
        digest: version.digest,
    };
    let result = f.store.remote_binding_transaction(false, |store| {
        store.prepare_remote_execution_in_transaction(
            &f.owner,
            &f.device,
            &f.workspace,
            &f.project,
            f.request("mcp-not-supported"),
            "controller-node",
        )
    });
    assert!(matches!(result,Err(StoreError::Limit(message)) if message.contains("MCP")));
}
