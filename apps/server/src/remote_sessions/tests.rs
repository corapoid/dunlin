use super::*;
fn state(enabled: bool, persistent: bool) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        Store::open(dir.path().join("controller.db")).unwrap(),
        crate::config::Config {
            db: dir.path().join("controller.db"),
            worker_controller: Some(crate::worker_channel::ControllerConfig {
                execution_enabled: enabled,
                persistent_sessions: persistent,
                mcp_broker: false,
                socket: dir.path().join("tailscaled.sock"),
                node_id: "controller-node".into(),
                serve_origin: "https://controller.tail123.ts.net".into(),
            }),
            ..Default::default()
        },
    );
    (dir, state)
}
#[tokio::test]
async fn session_producer_requires_both_explicit_flags() {
    for (enabled, persistent) in [(false, false), (true, false), (false, true)] {
        let (_dir, state) = state(enabled, persistent);
        assert!(matches!(Service::start(&state).await, Err(Error::Disabled)));
        let (sender, _receiver) = mpsc::channel(1);
        assert!(
            state
                .worker_channels
                .register_session_producer(sender)
                .is_ok()
        );
    }
}
#[tokio::test]
async fn exclusive_producer_and_drop_release_without_task_cycle() {
    let (_dir, state) = state(true, true);
    let service = Service::start(&state).await.unwrap();
    assert!(matches!(
        Service::start(&state).await,
        Err(Error::Unavailable)
    ));
    let weak = Arc::downgrade(&service);
    let context = service.context.clone();
    drop(service);
    tokio::time::timeout(Duration::from_secs(1), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(context.is_stopped());
    let next = Service::start(&state).await.unwrap();
    next.shutdown().await;
}

#[tokio::test]
async fn transcript_opt_out_never_registers_a_session_producer() {
    let (_dir, mut state) = state(true, true);
    Arc::make_mut(&mut state.config).store_transcripts = false;
    assert!(matches!(Service::start(&state).await, Err(Error::Disabled)));
    let (sender, _receiver) = mpsc::channel(1);
    assert!(
        state
            .worker_channels
            .register_session_producer(sender)
            .is_ok()
    );
}
