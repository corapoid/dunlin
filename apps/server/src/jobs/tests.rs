use super::*;
use crate::{config::Config, store::Store};
use dunlin_domain::jobs::Profile;
use dunlin_mcp_client::linear::{Filter, Inbox, Issue, issue_digest};
use std::{fs, process::Command};
mod checkout;
mod status;

struct Fixture {
    dir: tempfile::TempDir,
    state: AppState,
    user: String,
    token: String,
    task: Task,
}
impl Fixture {
    fn new(behavior: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("source");
        fs::create_dir(&root).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "base",
            ],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        fs::write(root.join("unstaged.txt"), "not in approved HEAD").unwrap();
        let head = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let user = store.create_user("owner", "member").unwrap();
        let project = store
            .create_project("source", root.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let (device, token) = store.create_device(&user.id, "fixture").unwrap();
        let issue = Issue {
            id: "issue-1".into(),
            identifier: "APP-1".into(),
            title: "Fixture task".into(),
            description: "Do work".into(),
            url: "https://linear.app/team/issue/APP-1/fixture".into(),
            status: "Todo".into(),
            labels: vec!["dunlin".into()],
        };
        store
            .linear_save(
                &user.id,
                "fixture-credential",
                "revision",
                &Inbox {
                    filter: Some(Filter {
                        label: "dunlin".into(),
                        team: None,
                    }),
                    issues: vec![issue.clone()],
                    synced_at: Some(crate::store::now_secs()),
                    error: None,
                },
            )
            .unwrap();
        let script = dir.path().join("fake.py");
        fs::write(&script, include_str!("tests/fake_acp.py")).unwrap();
        let mut state = AppState::new(
            store,
            Config {
                db: dir.path().join("relay.db"),
                sandbox: false,
                ..Default::default()
            },
        );
        state.jobs = Arc::new(Service {
            command: Some((
                "/usr/bin/python3".into(),
                vec![
                    "-u".into(),
                    script.to_string_lossy().into_owned(),
                    behavior.into(),
                ],
            )),
            ..Default::default()
        });
        let native_grant = state
            .store
            .lock()
            .unwrap()
            .capture_native_grant(&project.id, &user.id)
            .unwrap();
        let source_checkout_stamp = {
            let candidate = state
                .store
                .lock()
                .unwrap()
                .native_checkout_candidate(&project.id, &user.id, MetadataPermission::Read)
                .unwrap();
            let validated = candidate.validate_filesystem().unwrap();
            state
                .store
                .lock()
                .unwrap()
                .authorize_native_checkout(validated)
                .unwrap()
                .stamp()
        };
        let task = Task {
            native_grant: Some(native_grant),
            source_checkout_stamp: Some(source_checkout_stamp),
            request: Enqueue {
                issue_id: issue.id.clone(),
                expected_issue_digest: issue_digest(&issue),
                project_id: project.id.clone(),
                idempotency_key: "approval-1".into(),
                profile: Profile {
                    harness: "opencode".into(),
                    model: "fixture-model".into(),
                    mode: "build".into(),
                    timeout_secs: 30,
                    max_tool_requests: 8,
                    allow_tools: true,
                    review_state: "In Review".into(),
                },
                approved: true,
            },
            issue,
            root: root.to_string_lossy().into_owned(),
            head,
            settings_digest: settings(&state, &project.id).unwrap().0,
            device: device.id,
            linear_revision: "revision".into(),
        };
        Self {
            dir,
            state,
            user: user.id,
            token,
            task,
        }
    }
    fn enqueue(&self) -> Record {
        let store = self.state.store.lock().unwrap();
        let job = store.enqueue(&self.user, &self.task).unwrap();
        store.job(&job.id).unwrap().unwrap()
    }
    fn claim(&self) -> Record {
        self.enqueue();
        self.state
            .store
            .lock()
            .unwrap()
            .claim_job()
            .unwrap()
            .unwrap()
    }
    fn checkout(&self, id: &str) -> PathBuf {
        crate::management::root(&self.state.config)
            .join("jobs")
            .join(id)
            .join("worktree")
    }
}

#[test]
fn approval_deduplication_serial_claims_and_workload_are_host_scoped() {
    let f = Fixture::new("ok");
    let first = f.enqueue();
    let again = f.enqueue();
    assert_eq!(first.job.id, again.job.id);
    let store = f.state.store.lock().unwrap();
    let mut changed = f.task.clone();
    changed.request.profile.mode = "plan".into();
    assert!(store.enqueue(&f.user, &changed).is_err());
    changed = f.task.clone();
    changed.request.idempotency_key = "another".into();
    assert!(store.enqueue(&f.user, &changed).is_err());
    let peer = store.create_user("peer", "member").unwrap();
    store.grant(&f.task.request.project_id, &peer.id).unwrap();
    changed.issue.id = "issue-2".into();
    changed.request.issue_id = "issue-2".into();
    let second = store.enqueue(&peer.id, &changed).unwrap();
    let outsider = store.create_user("outsider", "member").unwrap();
    assert_eq!(store.job_workload(&outsider.id).unwrap().queued, 0);
    assert_eq!(store.job_workload(&peer.id).unwrap().queued, 2);
    assert_eq!(store.claim_job().unwrap().unwrap().job.id, first.job.id);
    assert!(store.claim_job().unwrap().is_none());
    store
        .finish_job(&first.job.id, Status::Review, "review")
        .unwrap();
    assert_eq!(store.claim_job().unwrap().unwrap().job.id, second.id);
}

#[test]
fn restart_blocks_dispatch_and_never_replays_ambiguous_execution_or_status() {
    let f = Fixture::new("ok");
    let record = f.claim();
    {
        let store = f.state.store.lock().unwrap();
        store
            .finish_job(&record.job.id, Status::Review, "review")
            .unwrap();
        assert!(store.claim_status_delivery().unwrap().is_some());
    }
    let mut next = f.task.clone();
    next.issue.id = "issue-2".into();
    next.request.idempotency_key = "second".into();
    {
        let store = f.state.store.lock().unwrap();
        store.enqueue(&f.user, &next).unwrap();
        store.claim_job().unwrap().unwrap();
    }
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    reopened.recover_jobs().unwrap();
    let queue = reopened.jobs().unwrap();
    assert!(queue.paused);
    assert_eq!(queue.jobs[0].sync_state, "unknown");
    assert_eq!(queue.jobs[1].status, Status::Unknown);
    reopened.pause_jobs(false).unwrap();
    assert!(reopened.claim_job().unwrap().is_none());
    reopened.reconcile_jobs().unwrap();
    assert_eq!(reopened.jobs().unwrap().jobs[1].status, Status::Cancelled);
    assert!(reopened.claim_status_delivery().unwrap().is_none());
    reopened.retry_status_delivery(&record.job.id).unwrap();
    assert!(reopened.claim_status_delivery().unwrap().is_some());
}

#[test]
fn cancellation_priority_outbox_retry_and_archiving_are_durable() {
    let f = Fixture::new("ok");
    let first = f.enqueue();
    let store = f.state.store.lock().unwrap();
    store.pause_jobs(true).unwrap();
    assert!(store.claim_job().unwrap().is_none());
    store.cancel_job(&first.job.id).unwrap();
    assert_eq!(
        store.job(&first.job.id).unwrap().unwrap().job.status,
        Status::Cancelled
    );
    store.archive_job(&first.job.id).unwrap();
    assert!(store.job(&first.job.id).unwrap().is_none());
    assert!(
        store.enqueue(&f.user, &f.task).is_err(),
        "Archived approvals cannot be replayed"
    );
    let mut fresh = f.task.clone();
    fresh.request.idempotency_key = "fresh-approval".into();
    let first = store.enqueue(&f.user, &fresh).unwrap();
    let mut task = f.task.clone();
    task.issue.id = "second".into();
    task.request.idempotency_key = "second".into();
    let second = store.enqueue(&f.user, &task).unwrap();
    store.prioritize_job(&second.id).unwrap();
    store.pause_jobs(false).unwrap();
    assert_eq!(store.claim_job().unwrap().unwrap().job.id, second.id);
    store
        .finish_job(&second.id, Status::Review, "report")
        .unwrap();
    store.archive_job(&second.id).unwrap();
    assert!(store.job(&second.id).unwrap().is_some());
    assert_eq!(
        store.claim_status_delivery().unwrap().unwrap().job.id,
        second.id
    );
    assert!(store.claim_status_delivery().unwrap().is_none());
    store
        .defer_status_delivery(&second.id, "offline before mutation")
        .unwrap();
    assert!(store.claim_status_delivery().unwrap().is_none());
    store.resolve_status_delivery(&second.id).unwrap();
    assert_eq!(
        store.job(&second.id).unwrap().unwrap().job.sync_state,
        "resolved"
    );
    store.archive_job(&second.id).unwrap();
    assert!(store.job(&second.id).unwrap().is_none());
    assert_eq!(store.claim_job().unwrap().unwrap().job.id, first.id);
    store.cancel_job(&first.id).unwrap();
    store
        .finish_job(&first.id, Status::Review, "late result")
        .unwrap();
    let cancelled = store.job(&first.id).unwrap().unwrap().job;
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.sync_state, "none");
}

#[test]
fn removed_project_grant_revokes_execution_authority() {
    let f = Fixture::new("ok");
    let record = f.claim();
    assert!(authorized(&f.state, &record).is_ok());
    {
        let store = f.state.store.lock().unwrap();
        store
            .revoke_grant(&f.task.request.project_id, &f.user)
            .unwrap();
    }
    assert!(authorized(&f.state, &record).is_err());
}

#[test]
fn revoked_regranted_native_epoch_never_revives_approved_or_legacy_job() {
    let f = Fixture::new("ok");
    let mut record = f.claim();
    assert!(authorized(&f.state, &record).is_ok());
    {
        let store = f.state.store.lock().unwrap();
        store.revoke_grant(&record.job.project_id, &f.user).unwrap();
        store.grant(&record.job.project_id, &f.user).unwrap();
        assert!(
            store
                .project_for_user(&record.job.project_id, &f.user)
                .unwrap()
                .is_some()
        );
    }
    assert!(authorized(&f.state, &record).is_err());
    record.task.native_grant = None;
    assert!(authorized(&f.state, &record).is_err());
}

#[tokio::test]
async fn fake_agent_runs_serial_turns_in_independent_checkouts_with_exact_profile() {
    let mut f = Fixture::new("ok");
    for number in 0..2 {
        f.task.issue.id = format!("issue-{number}");
        f.task.request.idempotency_key = format!("approval-{number}");
        let record = f.claim();
        assert!(f.state.store.lock().unwrap().claim_job().unwrap().is_none());
        let report = runner::execute(&f.state, &record).await.unwrap();
        assert!(report.contains("fixture review"));
        let checkout = f.checkout(&record.job.id);
        assert!(checkout.join("result.txt").exists());
        assert!(!checkout.join("unstaged.txt").exists());
        assert!(!PathBuf::from(&f.task.root).join("result.txt").exists());
        let remotes = Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .arg("remote")
            .output()
            .unwrap();
        assert!(remotes.stdout.is_empty());
        assert!(checkout.join(".git").is_dir());
        f.state
            .store
            .lock()
            .unwrap()
            .finish_job(&record.job.id, Status::Review, &report)
            .unwrap();
    }
}

#[tokio::test]
async fn reopened_relay_dispatches_queued_work_without_any_desktop_or_presence() {
    let f = Fixture::new("ok");
    f.enqueue();
    let mut second = f.task.clone();
    second.issue.id = "issue-2".into();
    second.request.idempotency_key = "approval-2".into();
    f.state
        .store
        .lock()
        .unwrap()
        .enqueue(&f.user, &second)
        .unwrap();
    let mut restarted = AppState::new(
        Store::open(&f.state.config.db).unwrap(),
        (*f.state.config).clone(),
    );
    restarted.jobs = f.state.jobs.clone();
    restarted.linear = Arc::new(crate::linear::Service::testing(
        Arc::new(dunlin_vault::store::MemoryStore::new()),
        "http://127.0.0.1:1/mcp".into(),
    ));
    start_background(restarted.clone()).unwrap();
    for _ in 0..200 {
        let jobs = restarted.store.lock().unwrap().jobs().unwrap().jobs;
        assert!(jobs.iter().filter(|job| job.status.active()).count() <= 1);
        if jobs.iter().all(|job| job.status == Status::Review) {
            return;
        }
        if jobs.iter().any(|job| {
            matches!(
                job.status,
                Status::Blocked | Status::Failed | Status::Unknown
            )
        }) {
            panic!("Unexpected background failure: {jobs:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Durable host queue did not finish without a desktop");
}

#[tokio::test]
async fn fake_agent_rejects_unapproved_model_mode_permissions_paths_and_tool_overruns() {
    for behavior in [
        "wrong_model",
        "mode_changes_model",
        "escape",
        "changed_model",
        "too_many_tools",
        "permission",
    ] {
        let mut f = Fixture::new(behavior);
        if behavior == "permission" {
            f.task.request.profile.allow_tools = false;
        }
        if behavior == "too_many_tools" {
            f.task.request.profile.max_tool_requests = 1;
        }
        let record = f.claim();
        let error = runner::execute(&f.state, &record).await.unwrap_err();
        assert!(
            matches!(error.0, Status::Blocked | Status::Failed),
            "{behavior}: {error:?}"
        );
        assert!(
            !f.checkout(&record.job.id)
                .join("prompt-after-rejection")
                .exists()
        );
    }
}

#[tokio::test]
async fn running_jobs_stop_on_cancel_revocation_and_timeout() {
    for reason in ["cancel", "device", "settings", "linear", "timeout"] {
        let f = Fixture::new("hang");
        let record = f.claim();
        let id = record.job.id.clone();
        let state = f.state.clone();
        let device = f.task.device.clone();
        let project = f.task.request.project_id.clone();
        let user = f.user.clone();
        let marker = f.checkout(&id).join("started");
        let control = async move {
            for _ in 0..100 {
                if marker.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(marker.exists());
            let store = state.store.lock().unwrap();
            match reason {
                "cancel" => store.cancel_job(&id).unwrap(),
                "device" => store.revoke_device(&device).unwrap(),
                "settings" => store
                    .save_project_settings(&project, "{\"harness\":\"changed\"}")
                    .unwrap(),
                "linear" => {
                    store.linear_remove(&user).unwrap();
                }
                _ => {}
            }
        };
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(if reason == "timeout" { 3 } else { 10 });
        let (result, _) = tokio::join!(runner::execute_until(&f.state, &record, deadline), control);
        let error = result.unwrap_err();
        assert_eq!(
            error.0,
            match reason {
                "cancel" => Status::Cancelled,
                "timeout" => Status::Failed,
                _ => Status::Blocked,
            }
        );
    }
}

#[tokio::test]
async fn aborting_runner_kills_its_process_group() {
    let f = Fixture::new("hang");
    let record = f.claim();
    let marker = f.checkout(&record.job.id).join("started");
    let state = f.state.clone();
    let task = tokio::spawn(async move { runner::execute(&state, &record).await });
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid: i32 = fs::read_to_string(marker).unwrap().parse().unwrap();
    task.abort();
    let _ = task.await;
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Runner process survived future cancellation");
}

#[tokio::test]
async fn queue_api_requires_device_project_and_owner_or_admin_authority() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new("ok");
    let record = f.enqueue();
    let peer_token = {
        let store = f.state.store.lock().unwrap();
        let peer = store.create_user("peer", "member").unwrap();
        store.grant(&f.task.request.project_id, &peer.id).unwrap();
        store.create_device(&peer.id, "peer").unwrap().1
    };
    for (path, token, body, expected) in [
        ("/v1/jobs".to_string(), None, None, StatusCode::UNAUTHORIZED),
        (
            format!("/v1/jobs/{}", record.job.id),
            Some(peer_token.as_str()),
            Some(serde_json::json!({"action":"cancel"})),
            StatusCode::FORBIDDEN,
        ),
        (
            "/v1/jobs/queue".to_string(),
            Some(f.token.as_str()),
            Some(serde_json::json!({"action":"pause"})),
            StatusCode::FORBIDDEN,
        ),
        (
            "/v1/jobs".to_string(),
            Some(f.token.as_str()),
            Some(serde_json::to_value(&f.task.request).unwrap()),
            StatusCode::OK,
        ),
    ] {
        let mut request = if body.is_some() {
            Request::post(&path).header("Content-Type", "application/json")
        } else {
            Request::get(&path)
        };
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = crate::api::router(f.state.clone())
            .oneshot(
                request
                    .body(
                        body.map(|body| Body::from(body.to_string()))
                            .unwrap_or_else(Body::empty),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(status, expected, "{}", String::from_utf8_lossy(&bytes));
    }
}
use std::path::PathBuf;

#[tokio::test]
async fn derived_checkout_epoch_revocation_stops_old_process_and_new_job_needs_fresh_checkout() {
    let mut f = Fixture::new("hang");
    let script = f.dir.path().join("fake.py");
    let original = include_str!("tests/fake_acp.py");
    let delayed = original.replace(
        "while True:\n                time.sleep(1)",
        "time.sleep(2)\n            open('late-derived-mutation', 'w').write('must not run')\n            while True:\n                time.sleep(1)",
    );
    assert_ne!(delayed, original);
    fs::write(&script, delayed).unwrap();
    let record = f.claim();
    let id = record.job.id.clone();
    let marker = f.checkout(&id).join("started");
    let state = f.state.clone();
    let owner = f.user.clone();
    let control = async move {
        let pid = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(pid) = fs::read_to_string(&marker)
                    .ok()
                    .and_then(|s| s.parse::<i32>().ok())
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("The real fixture process must reach its prompt");
        let store = state.store.lock().unwrap();
        let running = store.job(&id).unwrap().unwrap();
        let project = running.job.checkout_project.as_ref().unwrap().clone();
        let pin = running.checkout_grant.as_ref().unwrap().clone();
        assert_eq!(pin.project_id, project);
        assert_eq!(pin.user_id, owner);
        // No observer can see the intermediate denial. Presence-only checks
        // would accept this exact same project again and let the process run.
        store.revoke_grant(&project, &owner).unwrap();
        store.grant(&project, &owner).unwrap();
        assert!(store.project_for_user(&project, &owner).unwrap().is_some());
        assert!(store.revalidate_native_grant(&pin).is_err());
        (project, pin, pid)
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let (result, (derived, old_pin, pid)) =
        tokio::join!(runner::execute_until(&f.state, &record, deadline), control);
    let error = result.unwrap_err();
    assert_eq!(error.0, Status::Blocked);
    assert!(
        !f.checkout(&record.job.id)
            .join("late-derived-mutation")
            .exists()
    );
    #[cfg(unix)]
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "The stale process must be stopped"
    );
    {
        let store = f.state.store.lock().unwrap();
        let persisted = store.job(&record.job.id).unwrap().unwrap();
        assert_eq!(persisted.checkout_grant, Some(old_pin.clone()));
        assert_eq!(
            store
                .capture_native_grant(&derived, &f.user)
                .unwrap()
                .revision,
            old_pin.revision + 2
        );
        store
            .finish_job(&record.job.id, Status::Blocked, &error.1)
            .unwrap();
    }
    let reopened = Store::open(&f.state.config.db).unwrap();
    assert_eq!(
        reopened
            .job(&record.job.id)
            .unwrap()
            .unwrap()
            .checkout_grant,
        Some(old_pin)
    );
    drop(reopened);

    fs::write(&script, original).unwrap();
    let (program, mut args) = f.state.jobs.command.clone().unwrap();
    *args.last_mut().unwrap() = "ok".into();
    f.state.jobs = Arc::new(Service {
        command: Some((program, args)),
        ..Default::default()
    });
    f.task.issue.id = "fresh-derived-issue".into();
    f.task.request.issue_id = f.task.issue.id.clone();
    f.task.request.idempotency_key = "fresh-derived-approval".into();
    f.task.native_grant = Some(
        f.state
            .store
            .lock()
            .unwrap()
            .capture_native_grant(&f.task.request.project_id, &f.user)
            .unwrap(),
    );
    f.task.source_checkout_stamp = Some({
        let candidate = f
            .state
            .store
            .lock()
            .unwrap()
            .native_checkout_candidate(
                &f.task.request.project_id,
                &f.user,
                MetadataPermission::Read,
            )
            .unwrap();
        let validated = candidate.validate_filesystem().unwrap();
        f.state
            .store
            .lock()
            .unwrap()
            .authorize_native_checkout(validated)
            .unwrap()
            .stamp()
    });
    let fresh = f.claim();
    let report = runner::execute(&f.state, &fresh).await.unwrap();
    assert!(report.contains("fixture review"));
    assert!(f.checkout(&fresh.job.id).join("result.txt").exists());
    let store = f.state.store.lock().unwrap();
    let current = store.job(&fresh.job.id).unwrap().unwrap();
    let pin = current.checkout_grant.unwrap();
    assert_ne!(pin.project_id, derived);
    store.revalidate_native_grant(&pin).unwrap();
}

#[tokio::test]
async fn migration_preserves_legacy_checkout_without_inventing_a_grant_pin() {
    let f = Fixture::new("ok");
    let record = f.claim();
    let derived = {
        let store = f.state.store.lock().unwrap();
        let project = store
            .create_project(
                "legacy-checkout",
                f.checkout(&record.job.id).to_str().unwrap(),
            )
            .unwrap();
        store.grant(&project.id, &f.user).unwrap();
        store
            .job_running(&record.job.id, &project.id, "legacy-branch")
            .unwrap();
        // Reproduce the exact pre-migration 25 table: the source task stamp
        // already existed, while derived checkout authority was not captured.
        rusqlite::Connection::open(&f.state.config.db)
            .unwrap()
            .execute_batch(
                "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; ALTER TABLE relay_jobs DROP COLUMN checkout_grant; ALTER TABLE interactive_ahp_clients DROP COLUMN views; PRAGMA user_version=25;",
            )
            .unwrap();
        project.id
    };
    let restarted = AppState::new(
        Store::open(&f.state.config.db).unwrap(),
        (*f.state.config).clone(),
    );
    let old = restarted
        .store
        .lock()
        .unwrap()
        .job(&record.job.id)
        .unwrap()
        .unwrap();
    assert_eq!(old.job.checkout_project.as_deref(), Some(derived.as_str()));
    assert!(old.checkout_grant.is_none());
    let error = runner::execute_until(
        &restarted,
        &old,
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, Status::Blocked);
    assert!(error.1.contains("Legacy checkout"));
    assert!(!f.checkout(&record.job.id).exists());
    assert!(
        restarted
            .store
            .lock()
            .unwrap()
            .job(&record.job.id)
            .unwrap()
            .unwrap()
            .checkout_grant
            .is_none()
    );
}

#[test]
fn checkout_registration_rolls_back_status_project_branch_and_stamp_together() {
    let f = Fixture::new("ok");
    let record = f.claim();
    let task = serde_json::to_value(&record.task).unwrap();
    let project = {
        let store = f.state.store.lock().unwrap();
        let project = store
            .create_project(
                "atomic-checkout",
                f.checkout(&record.job.id).to_str().unwrap(),
            )
            .unwrap();
        store.grant(&project.id, &f.user).unwrap();
        project
    };
    let connection = rusqlite::Connection::open(&f.state.config.db).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_checkout BEFORE UPDATE OF checkout_grant ON relay_jobs BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    {
        let store = f.state.store.lock().unwrap();
        assert!(
            store
                .job_running(&record.job.id, &project.id, "atomic-branch")
                .is_err()
        );
        let unchanged = store.job(&record.job.id).unwrap().unwrap();
        assert_eq!(unchanged.job.status, Status::Preparing);
        assert!(unchanged.job.checkout_project.is_none());
        assert!(unchanged.job.branch.is_none());
        assert!(unchanged.checkout_grant.is_none());
    }
    connection
        .execute_batch("DROP TRIGGER fail_checkout")
        .unwrap();
    let store = f.state.store.lock().unwrap();
    store
        .job_running(&record.job.id, &project.id, "atomic-branch")
        .unwrap();
    let running = store.job(&record.job.id).unwrap().unwrap();
    assert_eq!(running.job.status, Status::Running);
    assert_eq!(
        running.job.checkout_project.as_deref(),
        Some(project.id.as_str())
    );
    assert_eq!(running.job.branch.as_deref(), Some("atomic-branch"));
    assert_eq!(
        running.checkout_grant.as_ref().unwrap(),
        &store.capture_native_grant(&project.id, &f.user).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&running.task).unwrap(),
        task,
        "Derived execution authority must not rewrite the approved Task/fingerprint"
    );
}
