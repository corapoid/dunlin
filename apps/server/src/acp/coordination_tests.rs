use super::*;
use dunlin_coordination::service::{Service, request};
use dunlin_coordination::{Continuation, ContinuationStatus, RunStatus};

struct Fixture {
    _dir: tempfile::TempDir,
    state: AppState,
    user: User,
    device: Device,
    project: Project,
    tracker: Tracker,
    close: mpsc::Sender<()>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open_in_memory().unwrap();
        let user = store.create_user("user", "member").unwrap();
        let (device, _) = store.create_device(&user.id, "device").unwrap();
        let project = store
            .create_project("repo", dir.path().to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let mut state = AppState::new(
            store,
            crate::config::Config {
                db: dir.path().join("relay.db"),
                ..Default::default()
            },
        );
        state.coordination = Some(std::sync::Arc::new(
            Service::start(&dir.path().join("coordination")).unwrap(),
        ));
        let (close, _) = mpsc::channel(1);
        Self {
            _dir: dir,
            state,
            user,
            device,
            project,
            tracker: Tracker::default(),
            close,
        }
    }
    fn client(&mut self, value: Value) -> Result<Value, Value> {
        self.tracker
            .on_client_message(
                &self.state,
                &self.user,
                &self.device,
                &self.project,
                "connection",
                &self.close,
                &value,
            )
            .map(|v| serde_json::from_str(&v.unwrap()).unwrap())
    }
    fn agent(&mut self, value: Value) {
        self.tracker.on_agent_message(
            &self.state,
            &self.user,
            &self.device,
            &self.project,
            "connection",
            &self.close,
            &value,
        );
    }
    fn session(&mut self, id: u64, session: &str) -> String {
        let forwarded=self.client(json!({"jsonrpc":"2.0","id":id,"method":"session/new","params":{"cwd":self.project.path,"mcpServers":[]}})).unwrap();
        let server = forwarded["params"]["mcpServers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "dunlin-coordination")
            .unwrap();
        let token = server["env"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "DUNLIN_COORDINATION_TOKEN")
            .unwrap()["value"]
            .as_str()
            .unwrap()
            .to_owned();
        self.agent(json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session,"models":{"currentModelId":"model","availableModels":[]},"modes":{"currentModeId":"plan","availableModes":[]}}}));
        token
    }
    fn prompt(&mut self, id: u64, session: &str) -> Value {
        self.client(json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"Work"}]}})).unwrap()
    }
}

#[test]
fn stdio_capability_is_injected_bound_and_revoked_with_device_authorization() {
    let mut f = Fixture::new();
    let token = f.session(1, "session");
    f.prompt(2, "session");
    let service = f.state.coordination.as_ref().unwrap();
    let result = request(service.socket(), &token, "agents_list", json!({})).unwrap();
    assert_eq!(result["self_session"], "session");
    assert_eq!(result["agents"][0]["status"], "running");
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&f.device.id)
        .unwrap();
    assert!(request(service.socket(), &token, "agents_list", json!({})).is_err());
}

#[test]
fn remote_continuation_uses_only_approved_text_and_cannot_be_replayed() {
    let mut f = Fixture::new();
    let token = f.session(1, "c");
    f.session(2, "a");
    f.prompt(3, "a");
    f.prompt(4, "c");
    let service = f.state.coordination.as_ref().unwrap().clone();
    let proposed: Continuation = serde_json::from_value(
        request(
            service.socket(),
            &token,
            "continuations_create",
            json!({"prompt":"APPROVED task","idempotency_key":"once"}),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proposed.dependencies.len(), 1);
    f.agent(json!({"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}));
    f.agent(json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}));
    {
        let mut db = service.store.lock().unwrap();
        db.approve(
            &proposed.id,
            &proposed.execution,
            dunlin_coordination::now_ms() + 60_000,
        )
        .unwrap();
        db.mutate(|l| {
            l.continuations[0].executor_device = Some(f.device.id.clone());
            l.continuations[0].executor_connection = Some("connection".into());
            Ok(())
        })
        .unwrap();
    }
    let input = json!({"jsonrpc":"2.0","id":5,"method":"session/prompt","params":{"sessionId":"c","prompt":[{"type":"text","text":"UNAPPROVED OVERRIDE"}],"_meta":{"dunlin/continuation":proposed.id}}});
    let outgoing = f.client(input.clone()).unwrap();
    let text = outgoing["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("APPROVED task"));
    assert!(!text.contains("UNAPPROVED OVERRIDE"));
    f.tracker.confirm_coordination(&f.state, &input);
    assert_eq!(
        service.store.lock().unwrap().ledger().continuations[0].status,
        ContinuationStatus::Dispatched
    );
    f.agent(json!({"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn"}}));
    let mut replay = input;
    replay["id"] = json!(6);
    assert!(f.client(replay).is_err());
}

#[test]
fn disconnected_runtime_is_unknown_not_success_and_token_is_revoked() {
    let mut f = Fixture::new();
    let token = f.session(1, "c");
    f.prompt(2, "c");
    f.tracker.stop_coordination(&f.state);
    let service = f.state.coordination.as_ref().unwrap();
    assert_eq!(
        service.store.lock().unwrap().ledger().runs[0].status,
        RunStatus::Unknown
    );
    assert!(request(service.socket(), &token, "agents_list", json!({})).is_err());
}

#[test]
fn sandbox_exposes_exact_broker_sockets_not_the_ledger_or_host_temp_directory() {
    let socket = std::path::PathBuf::from("/tmp/private-coordinator/broker.sock");
    let executable = std::path::PathBuf::from("/opt/dunlin/dunlin-relay");
    let command = linux_harness_command(
        &crate::config::Config::default(),
        "mock",
        &[],
        "/repo",
        &[],
        std::path::Path::new("/agent-home"),
        &[socket.clone(), executable.clone()],
    );
    let args: Vec<_> = command
        .as_std()
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    for path in [socket, executable] {
        assert!(args.windows(3).any(|w| w[0] == "--ro-bind"
            && w[1] == path.to_string_lossy()
            && w[2] == path.to_string_lossy()));
    }
    assert!(
        !args
            .windows(3)
            .any(|w| w[0] == "--ro-bind" && w[1] == "/tmp")
    );
    let profile = sandbox_profile(
        "/repo",
        &[],
        &[],
        "mock",
        std::path::Path::new("/state/relay.db"),
    );
    assert!(profile.contains("/state/relay.coordination"));
}
