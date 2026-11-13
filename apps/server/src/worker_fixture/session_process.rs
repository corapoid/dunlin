//! Actual SIGKILL/relaunch of both production services behind the private TLS
//! fixture. Only daemon identity/trust and the ACP peer are controlled here.
use super::session_vertical::{session_connect, session_dispatch, session_rpc};
use super::*;
use crate::worker_client::session_faults::{AckCut, AckCutGuard};
use crate::worker_exports::{
    CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind, WorkerStore,
    policy::PolicySnapshot,
};
use std::{
    fs,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    process::Stdio,
};

const HELPER: &str = "worker_fixture::session_process::remote_session_process_helper";

// Private test-only bootstrap: one bounded JSON value over stdin, never command
// arguments or ambient environment. This test is excluded from runtime prefixes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "invoked only as a child of the bounded process-relaunch fixture"]
async fn remote_session_process_helper() {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() <= 65536);
    let mut input: Value = serde_json::from_slice(&bytes).unwrap();
    let _cut = input["ack_cut"].as_str().map(|cut| {
        let cut = match cut {
            "controller_before_ack" => {
                assert_eq!(input["role"], "controller");
                AckCut::ControllerBeforeWire
            }
            "worker_after_ack" => {
                assert_eq!(input["role"], "worker");
                AckCut::WorkerAfterCommit
            }
            _ => panic!("invalid private fixture cut"),
        };
        AckCutGuard::install(input["host_id"].as_str().unwrap(), cut)
    });
    if input["role"] == "worker" {
        let config: crate::worker_client::WorkerConfig =
            serde_json::from_value(input["config"].take()).unwrap();
        let credential = Zeroizing::new(input["credential"].take().as_str().unwrap().to_owned());
        let transport = Transport::new(
            config.network.socket.clone(),
            config.network.node_id.clone(),
            config.network.serve_origin.clone(),
            config.network.listen,
        )
        .unwrap()
        .with_test_roots(roots());
        let _ = crate::worker_client::run_with_transport(config, credential, transport).await;
    } else {
        assert_eq!(input["role"], "controller");
        let listen: SocketAddr = input["listen"].as_str().unwrap().parse().unwrap();
        let db = PathBuf::from(input["db"].as_str().unwrap());
        let socket = PathBuf::from(input["socket"].as_str().unwrap());
        let store = Store::open(&db).unwrap();
        let mut state = AppState::new(
            store,
            Config {
                listen: listen.to_string(),
                db,
                worker_controller: Some(crate::worker_channel::ControllerConfig {
                    execution_enabled: true,
                    persistent_sessions: true,
                    mcp_broker: false,
                    socket: socket.clone(),
                    node_id: "n-controller".into(),
                    serve_origin: "https://controller.tail123.ts.net".into(),
                }),
                ..Default::default()
            },
        );
        Arc::get_mut(&mut state.worker_channels)
            .unwrap()
            .test_transport = Some(
            Transport::new(
                socket,
                "n-controller".into(),
                "https://controller.tail123.ts.net".into(),
                listen,
            )
            .unwrap()
            .with_test_roots(roots()),
        );
        // Same listener exclusivity, ordering and service constructor as main.
        // Startup never recreates a positive lease or restarts an old actor.
        let listener = TcpListener::bind(listen).await.unwrap();
        state
            .store
            .lock()
            .unwrap()
            .recover_interactive_sessions()
            .unwrap();
        let _services = crate::remote_services::Services::start(&state)
            .await
            .unwrap();
        println!(
            "{}",
            json!({"controller_ready":true,"boot":state.worker_channels.boot_id})
        );
        axum::serve(listener, crate::api::router(state))
            .await
            .unwrap();
    }
}

struct Process {
    child: tokio::process::Child,
    control: OwnedFd,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
}
impl Process {
    async fn start(input: Value) -> Self {
        use tokio::io::AsyncBufReadExt;
        let bytes = serde_json::to_vec(&input).unwrap();
        assert!(bytes.len() <= 65536);
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                HELPER,
                "--ignored",
                "--nocapture",
                "--format",
                "terse",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        // Pin this exact owned child before releasing its bootstrap input.
        // STOP/CONT below never address a potentially reused numeric PID.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id().unwrap(), 0) };
        assert!(fd >= 0, "owned fixture child pidfd");
        let control = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&bytes).await.unwrap();
        drop(stdin);
        let lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            control,
            lines,
        }
    }
    async fn boot(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(line) = self.lines.next_line().await.unwrap() {
                assert!(line.len() <= 65536);
                // Default libtest can prefix a test's first stdout line. The
                // child uses terse format; tolerate only that known prefix too.
                let prefix = format!("test {HELPER} ... ");
                let raw = line.strip_prefix(&prefix).unwrap_or(&line);
                if let Ok(value) = serde_json::from_str::<Value>(raw)
                    && value["controller_ready"] == true
                {
                    return value["boot"].as_str().unwrap().to_owned();
                }
            }
            panic!(
                "controller exited before readiness: {:?}",
                self.child.wait().await
            );
        })
        .await
        .expect("controller process boot deadline")
    }
    async fn cut(&mut self, cut: AckCut) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(line) = self.lines.next_line().await.unwrap() {
                assert!(line.len() <= 65536);
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value["fixture_ack_cut"] == cut.label()
                {
                    return;
                }
            }
            panic!("owned fixture exited before exact ACK cut");
        })
        .await
        .expect("actual SQL/ACK cut marker");
    }
    fn signal(&self, signal: i32) {
        assert!(matches!(signal, libc::SIGSTOP | libc::SIGCONT));
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.control.as_raw_fd(),
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            },
            0,
            "signal exact owned fixture child"
        );
    }
    async fn kill(&mut self) {
        self.child.kill().await.unwrap(); // SIGKILL only this owned fixture child.
        let status = self.child.wait().await.unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
    }
}

struct Browser {
    origin: String,
    cookie: String,
}
impl Browser {
    async fn http(&self, method: &str, path: &str, body: Value, key: Option<&str>) -> (u16, Value) {
        tokio::time::timeout(
            Duration::from_secs(15),
            self.request(method, path, body, key),
        )
        .await
        .expect("bounded actual HTTP request")
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Value,
        key: Option<&str>,
    ) -> (u16, Value) {
        let host = self.origin.strip_prefix("http://").unwrap();
        let mut socket = TcpStream::connect(host).await.unwrap();
        let body = if method == "GET" {
            String::new()
        } else {
            body.to_string()
        };
        let admission = key
            .map(|k| format!("Idempotency-Key: {k}\r\n"))
            .unwrap_or_default();
        socket.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {}\r\nCookie: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{admission}Connection: close\r\n\r\n{body}",self.origin,self.cookie,body.len()).as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        socket
            .take(1024 * 1024)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        let boundary = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8_lossy(&bytes[..boundary]);
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let raw = &bytes[boundary + 4..];
        let mut body = Vec::new();
        if headers
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            let mut remaining = raw;
            loop {
                let end = remaining.windows(2).position(|v| v == b"\r\n").unwrap();
                let size = usize::from_str_radix(
                    std::str::from_utf8(&remaining[..end])
                        .unwrap()
                        .split(';')
                        .next()
                        .unwrap(),
                    16,
                )
                .unwrap();
                remaining = &remaining[end + 2..];
                if size == 0 {
                    break;
                }
                assert!(remaining.len() >= size + 2);
                body.extend_from_slice(&remaining[..size]);
                remaining = &remaining[size + 2..];
            }
        } else {
            body.extend_from_slice(raw);
        }
        (status, serde_json::from_slice(&body).unwrap())
    }
    async fn prompt(&self, id: &str, client: &str, text: &str) {
        let mut socket = session_connect(&self.origin, &self.cookie).await;
        let reply = session_rpc(&mut socket,1,"initialize",json!({"channel":"ahp-root://","clientId":client,"protocolVersions":["1.0.0"],"initialSubscriptions":["ahp-root://",format!("ahp-chat:/{id}"),format!("ahp-session:/{id}")]})).await;
        assert!(reply.get("error").is_none(), "{reply}");
        session_dispatch(&mut socket,client,json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/pendingMessageSet","kind":"queued","id":format!("prompt-{client}"),"message":{"text":text,"origin":{"kind":"user"}}}})).await;
    }
}

async fn channel_ready(
    f: &Fixture,
    browser: &Browser,
    controller: &str,
    old_worker: Option<&str>,
) -> String {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let (status, value) = browser
                .http(
                    "GET",
                    &format!(
                        "/v1/web/workspaces/{}/hosts/{}/channel",
                        f.workspace, f.host
                    ),
                    Value::Null,
                    None,
                )
                .await;
            assert_eq!(status, 200, "{value}");
            let channel = &value["channel"];
            if channel["phase"] == "active"
                && channel["lease"]["last_sequence"]
                    .as_u64()
                    .is_some_and(|sequence| sequence >= 1)
                && channel["lease"]["controller_boot_id"] == controller
                && let Some(worker) = channel["lease"]["worker_boot_id"].as_str()
                && old_worker != Some(worker)
            {
                return worker.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("new opaque channel boot deadline")
}

async fn wait_until(mut observation: impl FnMut() -> bool, label: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !observation() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires provisioned real bubblewrap/pidfd worker runtime runner"]
async fn production_browser_acceptance_runtime_remote_session_process_restart() {
    for cut in [AckCut::ControllerBeforeWire, AckCut::WorkerAfterCommit] {
        process_crash_scenario(cut).await;
    }
}
async fn process_crash_scenario(cut: AckCut) {
    let mut fixture = fixture_with_storage(true, true, true).await;
    let state_dir = fixture._dir.path().join("session-worker-state");
    let root = fixture._dir.path().join("session-checkout");
    fs::create_dir(&root).unwrap();
    fixture_git(&root, &["init", "-q"]);
    let scope = ExportScope {
        controller_node_id: fixture.client.controller.node_id.clone(),
        worker_node_id: fixture.client.network.node_id.clone(),
        workspace_id: fixture.workspace.clone(),
        host_id: fixture.host.clone(),
        enrollment_epoch: fixture.client.enrollment_epoch,
        credential_revision: fixture.client.credential_revision,
    };
    let store = WorkerStore::open(&state_dir, scope.clone()).unwrap();
    let policy_path = fixture._dir.path().join("session-policy.json");
    fs::write(&policy_path, serde_json::to_vec(&json!({"schema_version":1,"harnesses":["opencode"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1})).unwrap()).unwrap();
    fs::set_permissions(&policy_path, fs::Permissions::from_mode(0o600)).unwrap();
    let policy = PolicySnapshot::load(&policy_path).unwrap();
    let checkout = CapturedCheckout::capture(&root, policy.digest(), None).unwrap();
    let export = store
        .register_captured_with_policy(&scope, "Session transport export", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &scope,
            &export.id,
            &ExportPrincipal {
                kind: PrincipalKind::Human,
                id: fixture.owner.clone(),
            },
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(store);
    let bin = state_dir.join("execution-runtime/server/tools/bin");
    crate::management::private_dir(&bin).unwrap();
    let program = bin.join("session-transport-agent");
    fs::write(
        &program,
        include_str!("../worker_execution/runtime/fixtures/session_agent.py"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.client.exports_state_dir = Some(state_dir.clone());
    fixture.client.capabilities.sandbox = dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap;
    fixture.client.execution = Some(serde_json::from_value(json!({"enabled":true,"persistent_sessions":true,"harnesses":{"opencode":{"program":program,"args":["persistent"]}}})).unwrap());
    let (profile, _definition, project) = {
        let store = fixture.state.store.lock().unwrap();
        let project = store
            .create_team_project(
                &fixture.owner,
                &fixture.workspace,
                "Session transport project",
            )
            .unwrap();
        let definition = serde_json::from_value(json!({"schema_version":1,"name":"Persistent transport agent","description":"","harness":"opencode","model":"fixture-model","mode":"fixture-mode","instructions":"literal-instructions","skills":[{"id":"fixture","version":"1","instructions":"literal-skill"}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        let draft = store
            .create_agent(&fixture.owner, &fixture.workspace, &project.id, definition)
            .unwrap();
        let published = store
            .publish_agent(
                &fixture.owner,
                &fixture.workspace,
                &project.id,
                &draft.id,
                1,
            )
            .unwrap();
        (
            dunlin_domain::agent::AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            },
            published.definition,
            project,
        )
    };

    let auth = {
        let store = fixture.state.store.lock().unwrap();
        let (device, _) = store
            .create_device(&fixture.owner, "journal-browser")
            .unwrap();
        crate::api::AuthUser {
            user: store.active_device_user(&device.id).unwrap().unwrap(),
            device,
        }
    };
    // Rebind the fixture's existing controller port in a separate OS process;
    // private daemon and TLS proxies remain alive in this parent test only.
    let mut server = fixture._tasks.pop().unwrap();
    server.0.abort();
    let _ = (&mut server.0).await;
    let browser = Browser {
        origin: format!("http://{}", fixture.state.config.listen),
        cookie: {
            let store = fixture.state.store.lock().unwrap();
            format!(
                "dunlin_web_session={}",
                store.create_browser_session(&auth.device.id, 3600).unwrap()
            )
        },
    };
    let controller_config = json!({"role":"controller","db":fixture.state.config.db,"listen":fixture.state.config.listen,"socket":fixture.state.config.worker_controller.as_ref().unwrap().socket});
    let config = &fixture.client;
    // Operator config types deliberately do not derive Serialize. Construct this
    // test-only bootstrap explicitly; its credential travels solely over stdin.
    let worker_config = json!({"role":"worker","credential":fixture.credential,"config":{
        "exports_state_dir":state_dir,"execution":{"enabled":true,"persistent_sessions":true,"harnesses":{"opencode":{"program":program,"args":["persistent"]}}},
        "network":{"socket":config.network.socket,"node_id":config.network.node_id,"serve_origin":config.network.serve_origin,"listen":config.network.listen},
        "controller":{"node_id":config.controller.node_id,"serve_origin":config.controller.serve_origin},
        "host_id":config.host_id,"workspace_id":config.workspace_id,"enrollment_epoch":config.enrollment_epoch,"credential_revision":config.credential_revision,"capabilities":config.capabilities
    }});
    let mut initial_controller = controller_config.clone();
    let mut initial_worker = worker_config.clone();
    let armed = match cut {
        AckCut::ControllerBeforeWire => &mut initial_controller,
        AckCut::WorkerAfterCommit => &mut initial_worker,
    };
    armed["ack_cut"] = json!(cut.label());
    armed["host_id"] = json!(fixture.host);
    let mut controller = Process::start(initial_controller).await;
    let first_controller = controller.boot().await;
    let mut worker = Process::start(initial_worker).await;
    let first_worker = channel_ready(&fixture, &browser, &first_controller, None).await;
    let project_path = format!(
        "/v1/web/workspaces/{}/projects/{}",
        fixture.workspace, project.id
    );
    let (status,binding) = browser.http("POST",&format!("{project_path}/remote-bindings"),json!({"host_id":fixture.host,"export_id":export.id,"expected_export_revision":export.revision}),None).await;
    assert_eq!(status, 200, "{binding}");
    let payload = json!({"profile":profile,"target":{"kind":"remote","binding":{"id":binding["id"],"revision":binding["revision"]}}});
    let path = format!("{project_path}/interactive-sessions");
    let (status, accepted) = browser
        .http(
            "POST",
            &path,
            payload.clone(),
            Some("process-crash-original-0001"),
        )
        .await;
    assert_eq!(status, 202, "{accepted}");
    let id = accepted["session"]["id"].as_str().unwrap().to_owned();
    browser
        .prompt(&id, "process-active", "Start crash fixture")
        .await;
    wait_until(
        || {
            let store = fixture.state.store.lock().unwrap();
            let context = store
                .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
                .unwrap();
            serde_json::to_string(
                &store
                    .interactive_journal()
                    .projected_chat(&context, &id)
                    .unwrap(),
            )
            .unwrap()
            .contains("active-crash")
        },
        "active source text committed before SIGKILL",
    )
    .await;
    browser
        .prompt(&id, "process-queued", "NEVER_REPLAY_OLD_QUEUED")
        .await;
    match cut {
        AckCut::ControllerBeforeWire => controller.cut(cut).await,
        AckCut::WorkerAfterCommit => worker.cut(cut).await,
    }
    {
        let store = fixture.state.store.lock().unwrap();
        let snapshot = store.remote_session_snapshot(&id).unwrap();
        assert_eq!(snapshot.sequence, 1);
        assert_eq!(snapshot.ack_sequence, 1);
        assert_eq!(snapshot.confirmed_ack, 0);
        let db = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
        let ack: i64 = db
            .query_row("SELECT ack_sequence FROM worker_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            ack,
            if cut == AckCut::ControllerBeforeWire {
                0
            } else {
                1
            }
        );
    }
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\n");
    // No graceful shutdown, local cancel or fabricated stop proof. Both
    // processes die while a real sandbox prompt and a queued prompt exist.
    worker.kill().await;
    controller.kill().await;
    let mut controller = Process::start(controller_config).await;
    let second_controller = controller.boot().await;
    assert_ne!(first_controller, second_controller);
    let mut worker = Process::start(worker_config.clone()).await;
    let second_worker =
        channel_ready(&fixture, &browser, &second_controller, Some(&first_worker)).await;
    assert_ne!(first_worker, second_worker);
    wait_until(
        || {
            let store = fixture.state.store.lock().unwrap();
            let snapshot = store.remote_session_snapshot(&id).unwrap();
            snapshot.phase == "stopped" && snapshot.sequence == 3 && snapshot.confirmed_ack == 3
        },
        "historical source stop persisted and ACKed after actual process restart",
    )
    .await;
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
            .unwrap();
        let (session, turns) = store.interactive_journal().snapshot(&context, &id).unwrap();
        assert_eq!(
            session.phase,
            dunlin_session_runtime::SessionPhase::Interrupted
        );
        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns
                .iter()
                .filter(|t| t.phase == dunlin_session_runtime::TurnPhase::Unknown)
                .count(),
            1
        );
        assert_eq!(
            turns
                .iter()
                .filter(|t| t.phase == dunlin_session_runtime::TurnPhase::Queued)
                .count(),
            1
        );
    }
    let (status, replayed) = browser
        .http(
            "POST",
            &path,
            payload.clone(),
            Some("process-crash-original-0001"),
        )
        .await;
    assert_eq!(status, 202, "{replayed}");
    assert_eq!(replayed["admission"]["replayed"], true);
    assert_eq!(replayed["session"]["id"], id);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\n");
    let held = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        held
    );
    let db = rusqlite::Connection::open(state_dir.join("exports.sqlite3")).unwrap();
    let counts = || {
        db.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT coalesce(sum(ack_sequence),0) FROM worker_sessions)",[],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap()
    };
    assert_eq!(counts(), (1, 1, 0, 3));
    // Only an explicit new intent starts a replacement ACP context.
    let (status, fresh) = browser
        .http(
            "POST",
            &path,
            payload.clone(),
            Some("process-recovery-fresh-0002"),
        )
        .await;
    assert_eq!(status, 202, "{fresh}");
    let fresh_id = fresh["session"]["id"].as_str().unwrap();
    assert_ne!(fresh_id, id);
    browser
        .prompt(
            fresh_id,
            "process-fresh",
            "Remember nonce:after-process-restart",
        )
        .await;
    wait_until(
        || {
            let store = fixture.state.store.lock().unwrap();
            let context = store
                .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
                .unwrap();
            store
                .interactive_journal()
                .snapshot(&context, fresh_id)
                .unwrap()
                .1
                .iter()
                .any(|t| t.phase == dunlin_session_runtime::TurnPhase::Completed)
        },
        "explicit fresh turn after restart",
    )
    .await;
    let epoch = fixture
        .state
        .store
        .lock()
        .unwrap()
        .remote_session_snapshot(fresh_id)
        .unwrap()
        .lease
        .epoch;
    let (status, reply) = browser
        .http(
            "POST",
            &format!("/v1/web/interactive-sessions/{fresh_id}/stop"),
            json!({"expected_epoch":epoch}),
            None,
        )
        .await;
    assert_eq!(status, 202, "{reply}");
    wait_until(
        || counts() == (2, 2, 0, 6),
        "both worker reservations released only with source stop",
    )
    .await;
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\n");
    let calls: Vec<Value> = fs::read_to_string(root.join("session-calls"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for method in ["initialize", "session/new", "session/prompt"] {
        assert_eq!(
            calls.iter().filter(|c| c["method"] == method).count(),
            2,
            "{method}"
        );
    }
    // A stalled controller is different from a clean disconnect: the WSS
    // socket stays open, while its entire runtime cannot renew either lease.
    let (status, stalled) = browser
        .http(
            "POST",
            &path,
            payload,
            Some("process-controller-stall-0003"),
        )
        .await;
    assert_eq!(status, 202, "{stalled}");
    let stalled_id = stalled["session"]["id"].as_str().unwrap();
    browser
        .prompt(stalled_id, "process-stall-active", "Start crash fixture")
        .await;
    wait_until(
        || {
            let store = fixture.state.store.lock().unwrap();
            let context = store
                .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
                .unwrap();
            serde_json::to_string(
                &store
                    .interactive_journal()
                    .projected_chat(&context, stalled_id)
                    .unwrap(),
            )
            .unwrap()
            .contains("active-crash")
        },
        "active prompt before whole controller runtime stall",
    )
    .await;
    browser
        .prompt(
            stalled_id,
            "process-stall-queued",
            "NEVER_REPLAY_AFTER_STALL",
        )
        .await;
    let paused_at = tokio::time::Instant::now();
    controller.signal(libc::SIGSTOP);
    let status = tokio::time::timeout(Duration::from_secs(25), worker.child.wait())
        .await
        .expect("worker independently detects missing controller lease ACK")
        .unwrap();
    assert!(
        status.success(),
        "worker helper must settle its real disconnect/guardian path"
    );
    // Exceed both configured ten-second leases even if the worker had already
    // observed a negative condition before its outer channel timer fired.
    tokio::time::sleep_until(paused_at + Duration::from_secs(11)).await;
    controller.signal(libc::SIGCONT);
    let mut worker = Process::start(worker_config).await;
    channel_ready(&fixture, &browser, &second_controller, Some(&second_worker)).await;
    wait_until(
        || {
            let store = fixture.state.store.lock().unwrap();
            let snapshot = store.remote_session_snapshot(stalled_id).unwrap();
            snapshot.phase == "stopped" && snapshot.sequence == 3 && snapshot.confirmed_ack == 3
        },
        "late controller callbacks cannot revive expired leases or queued prompts",
    )
    .await;
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(&fixture.workspace, &project.id, &fixture.owner)
            .unwrap();
        let (session, turns) = store
            .interactive_journal()
            .snapshot(&context, stalled_id)
            .unwrap();
        assert_eq!(
            session.phase,
            dunlin_session_runtime::SessionPhase::Interrupted
        );
        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns
                .iter()
                .filter(|t| t.phase == dunlin_session_runtime::TurnPhase::Unknown)
                .count(),
            1
        );
        assert_eq!(
            turns
                .iter()
                .filter(|t| t.phase == dunlin_session_runtime::TurnPhase::Queued)
                .count(),
            1
        );
    }
    assert_eq!(counts(), (3, 3, 0, 9));
    assert_eq!(fs::read(root.join("effects")).unwrap(), b"x\nx\nx\n");
    let held = fs::metadata(root.join("session-held-effect"))
        .unwrap()
        .len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fs::metadata(root.join("session-held-effect"))
            .unwrap()
            .len(),
        held
    );
    // Cleanup is also bounded and kills only this fixture's child processes.
    worker.kill().await;
    controller.kill().await;
}
