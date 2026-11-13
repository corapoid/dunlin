//! Service lifecycle/policy tests. They do not claim v3 network or ACP execution.
use super::*;
use serde_json::json;
struct Fixture {
    dir: tempfile::TempDir,
    state: AppState,
    owner: String,
    device: String,
    workspace: String,
    project: String,
    host: String,
}
fn fixture(enabled: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("controller.db")).unwrap();
    let owner = store
        .create_user("controller-service-owner", "admin")
        .unwrap();
    let (device, _) = store
        .create_device(&owner.id, "controller-browser")
        .unwrap();
    let workspace = store.create_workspace(&owner, "Controller").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Remote")
        .unwrap();
    let host=store.create_host(&owner.id,&workspace.id,serde_json::from_value(json!({"name":"Host","node_id":"worker-node","serve_origin":"https://worker.tail123.ts.net",
        "capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap()).unwrap();
    let state = AppState::new(
        store,
        Config {
            db: dir.path().join("controller.db"),
            worker_controller: Some(crate::worker_channel::ControllerConfig {
                execution_enabled: enabled,
                persistent_sessions: false,
                mcp_broker: false,
                socket: dir.path().join("tailscaled.sock"),
                node_id: "controller-node".into(),
                serve_origin: "https://controller.tail123.ts.net".into(),
            }),
            ..Default::default()
        },
    );
    Fixture {
        dir,
        state,
        owner: owner.id,
        device: device.id,
        workspace: workspace.id,
        project: project.id,
        host: host.host.id,
    }
}
fn phase(f: &Fixture) -> String {
    f.state
        .store
        .lock()
        .unwrap()
        .connection_for_tests_remote_execution_phase()
}
// The test sentinel is never decoded/dispatched: it exists solely to prove
// ordering of exclusive producer registration and global recovery's SQL cut.
fn sentinel(f: &Fixture) {
    let store = f.state.store.lock().unwrap();
    store
        .remote_execution_lifecycle_sentinel_for_test(
            &f.owner,
            &f.device,
            &f.workspace,
            &f.project,
            &f.host,
        )
        .unwrap();
}
#[tokio::test]
async fn disabled_service_never_registers_a_producer() {
    let f = fixture(false);
    assert!(matches!(
        Service::start(&f.state).await,
        Err(Error::Disabled)
    ));
    let (sender, _receive) = mpsc::channel(1);
    assert!(
        f.state
            .worker_channels
            .register_execution_producer(sender)
            .is_ok()
    );
}
#[tokio::test]
async fn second_service_cannot_recover_first_service_live_ledger_and_restart_never_replays() {
    let f = fixture(true);
    let first = Service::start(&f.state).await.unwrap();
    sentinel(&f);
    assert!(Service::start(&f.state).await.is_err());
    assert_eq!(phase(&f), "pending");
    first.shutdown().await;
    let next = Service::start(&f.state).await.unwrap();
    assert_eq!(phase(&f), "unknown");
    assert!(next.attempts.lock().unwrap().is_empty());
    assert_eq!(next.capacity.available_permits(), 8);
    next.shutdown().await;
}
#[tokio::test]
async fn service_drop_has_no_appstate_producer_task_arc_cycle() {
    let f = fixture(true);
    let service = Service::start(&f.state).await.unwrap();
    let weak = Arc::downgrade(&service);
    let stopped = service.context.stop.clone();
    drop(service);
    tokio::time::timeout(Duration::from_secs(1), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(stopped.load(Ordering::Acquire));
    let (sender, _receive) = mpsc::channel(1);
    assert!(
        f.state
            .worker_channels
            .register_execution_producer(sender)
            .is_ok()
    );
}
#[tokio::test]
async fn host_account_policy_is_captured_before_store_and_rechecks_fresh_user_role() {
    let f = fixture(true);
    let service = Service::start(&f.state).await.unwrap();
    let (member, device) = {
        let store = f.state.store.lock().unwrap();
        let user = store.create_user("controller-member", "member").unwrap();
        let (device, _) = store.create_device(&user.id, "member-browser").unwrap();
        (user.id, device.id)
    };
    let owner = member.clone();
    let browser = device.clone();
    service
        .context
        .authority(move |ctx, store, policy| ctx.caller(store, &owner, &browser, policy))
        .await
        .unwrap();
    std::fs::create_dir_all(crate::management::root(&f.state.config)).unwrap();
    std::fs::write(
        crate::management::root(&f.state.config).join("settings.json"),
        r#"{"host_accounts":true}"#,
    )
    .unwrap();
    assert!(
        service
            .context
            .authority(move |ctx, store, policy| ctx.caller(store, &member, &device, policy))
            .await
            .is_err()
    );
    service.shutdown().await;
}
#[cfg(unix)]
#[tokio::test]
async fn fifo_policy_without_a_writer_is_rejected_without_a_dangling_blocking_reader() {
    use std::os::unix::ffi::OsStrExt;
    let f = fixture(true);
    let service = Service::start(&f.state).await.unwrap();
    let root = crate::management::root(&f.state.config);
    std::fs::create_dir_all(&root).unwrap();
    let path = std::ffi::CString::new(root.join("settings.json").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let context = service.context.clone();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            context.authority(|_, _, policy| Ok(policy))
        )
        .await
        .unwrap()
        .unwrap()
    );
    service.shutdown().await;
    assert!(f.dir.path().exists());
}

#[tokio::test]
async fn cancelled_startup_holds_exclusive_producer_until_actual_recovery_finishes() {
    let f = fixture(true);
    sentinel(&f);
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (ready, locked) = std::sync::mpsc::channel();
    let store = f.state.store.clone();
    let holder = std::thread::spawn(move || {
        let _locked = store.lock().unwrap();
        ready.send(()).unwrap();
        // Dropping release during an assertion unwind also unlocks the Store.
        let _ = held.recv();
    });
    locked.recv().unwrap();
    let mut startup = Box::pin(Service::start(&f.state));
    // Poll through registration and spawn, stopping exactly at its blocked SQL
    // waiter. No sleep or scheduler guess is used to establish the cut.
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(startup.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(startup);
    assert!(matches!(
        Service::start(&f.state).await,
        Err(Error::Unavailable)
    ));
    drop(release);
    holder.join().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (sender, _receiver) = mpsc::channel(1);
            if let Ok(registration) = f.state.worker_channels.register_execution_producer(sender) {
                drop(registration);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual recovery must release producer after cancellation");
    assert_eq!(phase(&f), "unknown");
    let next = Service::start(&f.state).await.unwrap();
    next.shutdown().await;
}
