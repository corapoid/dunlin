use super::super::authority::TurnScope;
use super::*;
use std::sync::{Condvar, atomic::AtomicUsize};

struct Adapter {
    pins: [SessionPins; 2],
    choice: AtomicUsize,
    calls: AtomicUsize,
    hold: Option<Arc<(Mutex<bool>, Condvar)>>,
}
fn pins() -> SessionPins {
    SessionPins {
        secret_target: dunlin_vault::scoped::Binding {
            workspace: "workspace".into(),
            project: "project".into(),
            host: "host".into(),
        },
        profile: dunlin_domain::agent::AgentProfileRef {
            agent_id: "profile".into(),
            version: 1,
            digest: "a".repeat(64),
        },
        session: "session".into(),
        workspace: "workspace".into(),
        owner: "owner".into(),
        lease: Lease {
            epoch: 1,
            instance: "runtime".into(),
        },
        starter_device: "device".into(),
        binding_revision: 1,
        profile_digest: "a".repeat(64),
    }
}
impl Adapter {
    fn new(changed: SessionPins) -> Self {
        Self {
            pins: [pins(), changed],
            choice: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            hold: None,
        }
    }
}
impl Authority for Adapter {
    fn pins(&self) -> &SessionPins {
        &self.pins[self.choice.load(Ordering::Acquire)]
    }
    fn check(&self, _: &AppState, _: Instant) -> Result<()> {
        self.calls.fetch_add(1, Ordering::Release);
        let Some(hold) = &self.hold else {
            panic!("invalid scopes must fail before adapter I/O")
        };
        let (mutex, condition) = hold.as_ref();
        let released = mutex.lock().unwrap();
        let _ = condition
            .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
            .unwrap();
        Err("fixture_guard_refused")
    }
    fn context(&self, _: &AppState, _: &Store) -> Result<AccessContext> {
        panic!("invalid scope reached SQL authority")
    }
    fn context_in_transaction(&self, _: &AppState, _: &Store) -> Result<AccessContext> {
        panic!("invalid scope reached OAuth transaction")
    }
    fn turn(&self) -> Result<TurnScope> {
        panic!("invalid scope reached turn")
    }
    fn changed(&self, _: &Store) {
        panic!("invalid scope emitted change")
    }
    fn interrupt(&self) {
        panic!("invalid scope interrupted another actor")
    }
}
fn state() -> AppState {
    AppState::new(
        Store::open_in_memory().unwrap(),
        crate::config::Config {
            db: "/nonexistent/mcp-authority-fixture/relay.db".into(),
            ..Default::default()
        },
    )
}
fn connection() -> BoundConnection {
    BoundConnection {
        context: AuthContext {
            user_id: "owner".into(),
            workspace_id: "workspace".into(),
            connection_id: "connection".into(),
            connection_revision: 1,
        },
        backend: Backend::OAuth(Box::new(Provider {
            id: "fixture".into(),
            name: "Fixture".into(),
            endpoint: "http://127.0.0.1:1/mcp".into(),
            issuer: "http://127.0.0.1:1/issuer".into(),
            client_id: "fixture".into(),
            redirect_uri: "http://localhost/v1/web/mcp/oauth/fixture/callback".into(),
            allowed_origins: vec![],
            scopes: vec![],
            authentication: crate::mcp_oauth::config::Authentication::Public,
            client_secret_ref: None,
        })),
        tools: vec![],
    }
}
fn broker(state: &AppState, authority: Arc<Adapter>, lifetime: Instant) -> Broker {
    Broker {
        state: state.clone(),
        pins: authority.pins().clone(),
        authority,
        connections: vec![],
        _slot: state
            .mcp_oauth
            .runtime_slots
            .clone()
            .try_acquire_owned()
            .unwrap(),
        lifetime,
        stop: Arc::new(AtomicBool::new(false)),
        catalog: Mutex::new(vec![]),
        receipts: Mutex::new(BTreeMap::new()),
    }
}
#[test]
fn broker_freezes_all_session_pins_before_adapter_sql_or_oauth() {
    for field in [
        "session",
        "workspace",
        "owner",
        "epoch",
        "instance",
        "device",
        "binding",
        "profile",
        "target",
        "profile-id",
        "profile-version",
    ] {
        let state = state();
        let mut changed = pins();
        match field {
            "session" => changed.session = "another".into(),
            "workspace" => changed.workspace = "another".into(),
            "owner" => changed.owner = "another".into(),
            "epoch" => changed.lease.epoch += 1,
            "instance" => changed.lease.instance = "another".into(),
            "device" => changed.starter_device = "another".into(),
            "binding" => changed.binding_revision += 1,
            "target" => changed.secret_target.host = "another-host".into(),
            "profile-id" => changed.profile.agent_id = "another-profile".into(),
            "profile-version" => changed.profile.version += 1,
            _ => changed.profile_digest = "b".repeat(64),
        }
        let authority = Arc::new(Adapter::new(changed));
        let scope = broker(
            &state,
            authority.clone(),
            Instant::now() + Duration::from_secs(60),
        );
        authority.choice.store(1, Ordering::Release);
        assert_eq!(
            scope.check(scope.lifetime).unwrap_err(),
            "mcp_scope_stopped_or_changed",
            "{field}"
        );
        let store = state.store.lock().unwrap();
        assert!(
            scope
                .context(&store, false, scope.authority.sql(&state).unwrap().as_ref())
                .is_err()
        );
        assert!(
            scope
                .context(&store, true, scope.authority.sql(&state).unwrap().as_ref())
                .is_err()
        );
        assert_eq!(authority.calls.load(Ordering::Acquire), 0);
    }
}
#[test]
fn broker_enforces_own_deadlines_and_stop_without_trusting_adapter() {
    let state = state();
    let authority = Arc::new(Adapter::new(pins()));
    let scope = broker(
        &state,
        authority.clone(),
        Instant::now() + Duration::from_secs(60),
    );
    assert_eq!(
        scope
            .check(Instant::now() - Duration::from_millis(1))
            .unwrap_err(),
        "mcp_scope_expired"
    );
    scope.stop.store(true, Ordering::Release);
    assert_eq!(
        scope.check(scope.lifetime).unwrap_err(),
        "mcp_scope_stopped_or_changed"
    );
    let expired = broker(&state, authority, Instant::now() - Duration::from_millis(1));
    let store = state.store.lock().unwrap();
    assert!(
        expired
            .context(
                &store,
                false,
                expired.authority.sql(&state).unwrap().as_ref()
            )
            .is_err()
    );
    assert!(
        expired
            .context(
                &store,
                true,
                expired.authority.sql(&state).unwrap().as_ref()
            )
            .is_err()
    );
}
#[tokio::test]
async fn broker_rejects_foreign_connection_owner_or_workspace_before_any_io() {
    let state = state();
    let authority = Arc::new(Adapter::new(pins()));
    for field in ["owner", "workspace"] {
        let mut bound = connection();
        if field == "owner" {
            bound.context.user_id = "another".into();
        } else {
            bound.context.workspace_id = "another".into();
        }
        assert_eq!(
            Broker::prepare(
                state.clone(),
                authority.clone(),
                vec![bound],
                Instant::now() + Duration::from_secs(60)
            )
            .await
            .err()
            .unwrap(),
            "mcp_connection_scope_changed"
        );
        assert_eq!(state.mcp_oauth.runtime_slots.available_permits(), 8);
    }
}
#[tokio::test]
async fn cancelled_catalog_waiter_retains_capacity_until_actual_worker_exits() {
    let state = state();
    let hold = Arc::new((Mutex::new(false), Condvar::new()));
    let mut adapter = Adapter::new(pins());
    adapter.hold = Some(hold.clone());
    let authority = Arc::new(adapter);
    let preparing = tokio::spawn(Broker::prepare(
        state.clone(),
        authority.clone(),
        vec![connection()],
        Instant::now() + Duration::from_secs(60),
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    while authority.calls.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    preparing.abort();
    assert!(matches!(preparing.await,Err(error) if error.is_cancelled()));
    assert_eq!(
        state.mcp_oauth.runtime_slots.available_permits(),
        7,
        "detached catalog still owns its real capacity"
    );
    {
        let (mutex, condition) = hold.as_ref();
        *mutex.lock().unwrap() = true;
        condition.notify_all();
    }
    while state.mcp_oauth.runtime_slots.available_permits() != 8 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
