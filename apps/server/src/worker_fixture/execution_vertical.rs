//! Real controller Service -> private TLS/WSS -> worker Runtime -> bwrap/ACP.
//! The LocalAPI daemon fixture supplies controlled identity/network observations;
//! it does not replace the transport, worker ledger or process adapter.
use super::*;
use crate::{
    api::AuthUser,
    remote_execution::Service,
    store::remote_bindings::resolver::execution::RemoteExecutionRequest,
    worker_exports::{
        CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind,
        WorkerStore, policy::PolicySnapshot,
    },
};
use dunlin_domain::{agent::AgentProfileRef, binding::BindingRef};
use dunlin_relay_protocol::worker::{ExecutionBudgets, ExecutionStatus};
use std::{fs, os::unix::fs::PermissionsExt};

fn worker_counts(state: &std::path::Path) -> (i64, i64, i64) {
    let connection = rusqlite::Connection::open_with_flags(
        state.join("exports.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT (SELECT count(*) FROM worker_execution_inbox),
                    (SELECT count(*) FROM worker_execution_outbox),
                    (SELECT coalesce(sum(reservation),0) FROM worker_execution_inbox)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_execution_vertical() {
    let mut f = fixture_with_execution(true).await;
    let state_dir = f._dir.path().join("vertical-worker-state");
    let checkout_root = f._dir.path().join("vertical-checkout");
    fs::create_dir(&checkout_root).unwrap();
    fixture_git(&checkout_root, &["init", "-q"]);
    let scope = ExportScope {
        controller_node_id: f.client.controller.node_id.clone(),
        worker_node_id: f.client.network.node_id.clone(),
        workspace_id: f.workspace.clone(),
        host_id: f.host.clone(),
        enrollment_epoch: f.client.enrollment_epoch,
        credential_revision: f.client.credential_revision,
    };
    let worker_store = WorkerStore::open(&state_dir, scope.clone()).unwrap();
    let policy_path = f._dir.path().join("vertical-policy.json");
    fs::write(
        &policy_path,
        serde_json::to_vec(&json!({
            "schema_version":1,"harnesses":["opencode"],"sandbox_required":true,
            "credential_mode":"isolated","max_seconds":60,
            "max_output_bytes":65536,"max_parallel":1
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&policy_path, fs::Permissions::from_mode(0o600)).unwrap();
    let policy = PolicySnapshot::load(&policy_path).unwrap();
    let checkout = CapturedCheckout::capture(&checkout_root, policy.digest(), None).unwrap();
    let export = worker_store
        .register_captured_with_policy(&scope, "Vertical export", &checkout, Some(&policy))
        .unwrap();
    worker_store
        .set_grant(
            &scope,
            &export.id,
            &ExportPrincipal {
                kind: PrincipalKind::Human,
                id: f.owner.clone(),
            },
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(worker_store);

    let bin = state_dir.join("execution-runtime/server/tools/bin");
    crate::management::private_dir(&bin).unwrap();
    let program = bin.join("vertical-acp-agent");
    // Count real process starts as well as shared fixture's durable prompt
    // effect, so idempotency cannot hide a second agent that received no prompt.
    let agent = include_str!("../worker_execution/runtime/fixtures/acp_agent.py").replace(
        "for line in sys.stdin:",
        "with open('process-starts', 'ab', buffering=0) as starts:\n    starts.write(b'x\\n')\n\nfor line in sys.stdin:",
    );
    fs::write(&program, agent).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    f.client.exports_state_dir = Some(state_dir.clone());
    f.client.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    f.client.execution = Some(
        serde_json::from_value(json!({
            "enabled":true,"harnesses":{"opencode":{"program":program,"args":["complete"]}}
        }))
        .unwrap(),
    );

    let (auth, project, profile) = {
        let store = f.state.store.lock().unwrap();
        let (device, _) = store.create_device(&f.owner, "vertical-browser").unwrap();
        let user = store.active_device_user(&device.id).unwrap().unwrap();
        let project = store
            .create_team_project(&f.owner, &f.workspace, "Vertical remote project")
            .unwrap();
        let definition = serde_json::from_value(json!({
            "schema_version":1,"name":"Vertical agent","description":"",
            "harness":"opencode","model":"fixture-model","mode":"fixture-mode",
            "instructions":"literal-instructions",
            "skills":[{"id":"fixture","version":"1","instructions":"literal-skill"}],
            "max_seconds":60,"max_output_bytes":65536
        }))
        .unwrap();
        let draft = store
            .create_agent(&f.owner, &f.workspace, &project.id, definition)
            .unwrap();
        let published = store
            .publish_agent(&f.owner, &f.workspace, &project.id, &draft.id, 1)
            .unwrap();
        (
            AuthUser { user, device },
            project,
            AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            },
        )
    };
    let service = Service::start(&f.state).await.unwrap();
    let config = f.client.clone();
    let credential = f.credential.clone();
    let dial = f.transport.clone();
    let mut worker = tokio::spawn(async move {
        crate::worker_client::run_with_transport(config, Zeroizing::new(credential), dial).await
    });
    struct WorkerCleanup(tokio::task::AbortHandle);
    impl Drop for WorkerCleanup {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _worker_cleanup = WorkerCleanup(worker.abort_handle());
    let transport = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                result = &mut worker => {
                    let code = match result {
                        Ok(Err(code)) => code,
                        Ok(Ok(())) => "unexpected_clean_exit",
                        Err(_) => "worker_task_failed",
                    };
                    panic!("Vertical worker startup failed: {code}");
                },
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
            if let Ok(transport) = f.state.worker_channels.execution_transport(&f.host) {
                break transport;
            }
        }
    })
    .await
    .expect("Real v3 worker startup deadline");
    let proof = f
        .state
        .worker_channels
        .describe_execution_export(
            &transport,
            &export.id,
            dunlin_relay_protocol::worker::ExportPrincipal {
                kind: dunlin_relay_protocol::worker::PrincipalKind::Human,
                id: f.owner.clone(),
            },
            Some(export.revision),
        )
        .await
        .unwrap();
    let binding = {
        let store = f.state.store.lock().unwrap();
        let authority = store
            .capture_remote_binding_authority(&f.owner, &f.workspace, &project.id)
            .unwrap();
        store
            .set_remote_binding(
                &f.owner,
                &f.workspace,
                &project.id,
                None,
                None,
                &f.host,
                &export.id,
                export.revision,
                &proof,
                &f.state.worker_channels.boot_id,
                "n-controller",
                fixture_now_ms(),
                &authority,
            )
            .unwrap()
    };
    let request = RemoteExecutionRequest {
        intent_key: "vertical-intent-00000001".into(),
        binding: BindingRef {
            id: binding.id,
            revision: binding.revision,
        },
        profile,
        input: "Perform the controlled single effect".into(),
        budgets: ExecutionBudgets {
            max_seconds: 60,
            max_output_bytes: 65536,
            lease_duration_ms: 10000,
        },
    };
    let accepted = service
        .submit(&auth, &f.workspace, &project.id, request.clone())
        .await
        .unwrap();
    assert!(!accepted.replayed);
    let terminal = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let current = service
                .status(&auth, &accepted.identity.attempt_id)
                .await
                .unwrap();
            if current
                .record
                .as_ref()
                .is_some_and(|record| record.outcome.is_some())
            {
                break current;
            }
            assert!(
                !worker.is_finished(),
                "Worker exited before terminal delivery"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Real ACP terminal delivery deadline");
    let record = terminal.record.unwrap();
    assert_eq!(record.status, ExecutionStatus::Completed);
    assert_eq!(record.outcome.unwrap().text, "one confirmed fixture turn");
    assert_eq!(fs::read(checkout_root.join("effects")).unwrap(), b"x\n");
    assert_eq!(
        fs::read(checkout_root.join("process-starts")).unwrap(),
        b"x\n"
    );
    let prompt = fs::read_to_string(checkout_root.join("received-prompt")).unwrap();
    assert_eq!(
        prompt,
        "literal-instructions\n\n[Skill fixture@1]\nliteral-skill\n\n## User request\nPerform the controlled single effect"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(checkout_root.join("negotiated")).unwrap())
            .unwrap(),
        json!({"model":"fixture-model","mode":"fixture-mode"})
    );
    let replayed = service
        .submit(&auth, &f.workspace, &project.id, request)
        .await
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.identity, accepted.identity);
    assert_eq!(replayed.record.unwrap().status, ExecutionStatus::Completed);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let worker_done = worker_counts(&state_dir) == (1, 0, 0);
            let controller_done = f
                .state
                .store
                .lock()
                .unwrap()
                .remote_execution_cleanup(8, None, |_, _| Ok(()))
                .unwrap()
                .is_empty();
            if worker_done && controller_done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Durable worker ACK/outbox/reservation deadline");
    assert_eq!(worker_counts(&state_dir), (1, 0, 0));
    assert_eq!(fs::read(checkout_root.join("effects")).unwrap(), b"x\n");
    assert_eq!(
        fs::read(checkout_root.join("process-starts")).unwrap(),
        b"x\n"
    );
    service.shutdown().await;
    f.state.worker_channels.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(8), worker)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
