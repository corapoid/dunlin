//! Real TLS HTTP effects behind the real runner bridge. Only the HTTP dialer and
//! keyring are test substitutes: the dialer pins our generated certificate and
//! loopback fixture; production public-DNS restrictions are never relaxed.
use super::*;
use dunlin_vault::{
    http::{HttpExecutor, HttpOutput, HttpPlan},
    store::{MemoryStore, SecretStore},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener as Listener, TcpStream as Stream},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
};
const VALUE: &str = "bridge-scoped-private-value";
const PROVIDER: &str = "dunlin-secret-bridge";
#[derive(Default)]
struct Secrets {
    memory: MemoryStore,
    reads: AtomicUsize,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        self.memory.get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.memory.set(id, value)
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        panic!("no implicit secret deletion")
    }
    fn get_policy(&self, _: &str) -> Result<Zeroizing<String>, String> {
        panic!("protected envelope only")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("protected envelope only")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("protected envelope only")
    }
}
struct Dialer {
    address: SocketAddr,
    config: Arc<rustls::ClientConfig>,
}
impl HttpExecutor for Dialer {
    fn execute(&self, plan: &HttpPlan) -> Result<HttpOutput, String> {
        self.run(plan, None)
    }
    fn execute_guarded(
        &self,
        plan: &HttpPlan,
        access: &dunlin_vault::access::LiveAccess,
    ) -> Result<HttpOutput, String> {
        self.run(plan, Some(access))
    }
}
impl Dialer {
    fn run(
        &self,
        plan: &HttpPlan,
        access: Option<&dunlin_vault::access::LiveAccess>,
    ) -> Result<HttpOutput, String> {
        if access.is_some_and(|access| !access.allowed()) {
            return Err("fixture authority denied".into());
        }
        assert_eq!(plan.method, "GET");
        assert_eq!(plan.url, "https://worker.tail123.ts.net/items");
        assert!(plan.body.is_none());
        let socket = Stream::connect_timeout(&self.address, Duration::from_secs(3))
            .map_err(|_| "fixture connect")?;
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let guard = if let Some(access) = access {
            let live = access.clone();
            let socket = socket.try_clone().unwrap();
            Some(dunlin_vault::http::Cancellation::new(
                move || live.allowed(),
                move || {
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                },
            )?)
        } else {
            None
        };
        let result = (|| {
            let client = rustls::ClientConnection::new(
                self.config.clone(),
                rustls::pki_types::ServerName::try_from("worker.tail123.ts.net").unwrap(),
            )
            .unwrap();
            let mut stream = rustls::StreamOwned::new(client, socket);
            let mut request = String::from(
                "GET /items HTTP/1.1\r\nHost: worker.tail123.ts.net\r\nConnection: close\r\n",
            );
            for (name, value) in &plan.headers {
                request.push_str(&format!("{name}: {value}\r\n"));
            }
            request.push_str("\r\n");
            stream
                .write_all(request.as_bytes())
                .map_err(|_| "fixture write")?;
            stream.flush().map_err(|_| "fixture flush")?;
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader
                .read_line(&mut first)
                .map_err(|_| "fixture lost response")?;
            if first != "HTTP/1.1 200 OK\r\n" {
                return Err("fixture lost response".into());
            }
            let mut length = None;
            let mut content_type = None;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).map_err(|_| "fixture header")?;
                if n == 0 {
                    return Err("fixture EOF".into());
                }
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').ok_or("fixture header")?;
                if name.eq_ignore_ascii_case("content-length") {
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| "fixture length")?,
                    );
                }
                if name.eq_ignore_ascii_case("content-type") {
                    content_type = Some(value.trim().into());
                }
            }
            let length = length.ok_or("fixture length")?;
            assert!(length < 4096);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).map_err(|_| "fixture body")?;
            Ok(HttpOutput {
                status: 200,
                content_type,
                body: String::from_utf8(body).unwrap(),
                truncated: false,
            })
        })();
        if guard.is_some_and(|guard| !guard.finish()) {
            return Err("fixture HTTP revoked".into());
        }
        result
    }
}
pub(super) struct Endpoint {
    secrets: Arc<Secrets>,
    calls: Arc<AtomicUsize>,
    hold: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Endpoint {
    pub(super) fn new(state: &mut AppState, mode: &str) -> Self {
        let listener = Listener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(CERT.to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(KEY.to_vec()),
                ),
            )
            .unwrap();
        let client = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots())
            .with_no_client_auth();
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let peer_closed = closed.clone();
        let kill = mode == "scoped-revoke";
        let hold = Arc::new((
            Mutex::new(mode == "scoped-revoke"),
            std::sync::Condvar::new(),
        ));
        let (count, stopped, held) = (calls.clone(), stop.clone(), hold.clone());
        let server = Arc::new(server);
        let lost = mode == "scoped-lost";
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let (socket, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let peer = socket.try_clone().unwrap();
                let mut stream = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(server.clone()).unwrap(),
                    socket,
                );
                let mut headers = String::new();
                let mut reader = BufReader::new(&mut stream);
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    headers.push_str(&line);
                    assert!(headers.len() < 8192);
                    if line == "\r\n" {
                        break;
                    }
                }
                assert!(headers.starts_with("GET /items HTTP/1.1\r\n"));
                assert!(
                    headers
                        .lines()
                        .any(|l| l.eq_ignore_ascii_case(&format!("Authorization: Bearer {VALUE}")))
                );
                count.fetch_add(1, Ordering::AcqRel);
                if kill {
                    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut byte = [0; 1];
                    let result = peer.peek(&mut byte);
                    if matches!(result, Ok(0))
                        || matches!(result,Err(ref e) if e.kind()==std::io::ErrorKind::ConnectionReset)
                    {
                        peer_closed.store(true, Ordering::Release);
                        continue;
                    }
                    break;
                }
                let guard = held.0.lock().unwrap();
                let (_guard, timeout) = held
                    .1
                    .wait_timeout_while(guard, Duration::from_secs(5), |v| {
                        *v && !stopped.load(Ordering::Acquire)
                    })
                    .unwrap();
                assert!(!timeout.timed_out(), "fixture release deadline");
                if lost {
                    continue;
                }
                let body = format!("{VALUE} scoped broker result");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        let secrets = Arc::new(Secrets::default());
        state.scoped_secrets = Arc::new(crate::scoped_secrets::Service::testing_executor(
            secrets.clone(),
            Arc::new(Dialer {
                address,
                config: Arc::new(client),
            }),
        ));
        Self {
            secrets,
            calls,
            hold,
            stop,
            closed,
            thread: Some(thread),
        }
    }
    pub(super) fn tool() -> dunlin_domain::agent::AgentMcpTool {
        let tool = dunlin_vault::scoped::tool::http();
        dunlin_domain::agent::AgentMcpTool {
            provider_id: PROVIDER.into(),
            name: tool.name.clone(),
            definition_digest: tool.definition_digest().unwrap(),
        }
    }
    pub(super) async fn install(
        &self,
        f: &Fixture,
        cookie: &str,
        project: &str,
        profile: &dunlin_domain::agent::AgentProfileRef,
    ) -> crate::interactive::mcp::Selection {
        let base = format!(
            "/v1/web/workspaces/{}/projects/{project}/hosts/{}/scoped-secrets/bridge",
            f.workspace, f.host
        );
        let policy = json!({"version":1,"managed":true,"injection":{"type":"header","name":"Authorization","prefix":"Bearer "},"allowed_hosts":["worker.tail123.ts.net"],"path_prefix":"/items","allowed_methods":["GET"],"allowed_headers":[],"auto_approve":false,"expires_at":null,"project_ids":[project],"recipes":[],"operations":[]});
        let (status, result) = fixture_http(
            &f.state,
            cookie,
            &base,
            json!({"expected_revision":0,"value":VALUE,"policy":policy}),
        )
        .await;
        assert_eq!(status, 200, "{result}");
        let (status,grant)=fixture_http(&f.state,cookie,&format!("{base}/grants"),json!({"expected_revision":0,"credential_revision":1,"principal_kind":"human","principal_id":f.owner,"profile":profile,"provider_id":PROVIDER,"read_only":true})).await;
        assert_eq!(status, 200, "{grant}");
        assert_eq!(self.secrets.reads.load(Ordering::Acquire), 0);
        crate::interactive::mcp::Selection {
            provider_id: PROVIDER.into(),
            connection_id: grant["id"].as_str().unwrap().into(),
            revision: grant["revision"].as_u64().unwrap(),
        }
    }
    fn release(&self) {
        *self.hold.0.lock().unwrap() = false;
        self.hold.1.notify_all();
    }
    pub(super) fn assert_before_approval(&self, previous: usize, denied: bool) {
        let previous = if denied { 0 } else { previous };
        assert_eq!(self.calls.load(Ordering::Acquire), previous);
        assert_eq!(self.secrets.reads.load(Ordering::Acquire), previous);
    }
    fn assert_private(
        &self,
        f: &Fixture,
        id: &str,
        state_dir: &std::path::Path,
        root: &std::path::Path,
    ) {
        let store = f.state.store.lock().unwrap();
        let snapshot = store.remote_session_snapshot(id).unwrap();
        let context = store
            .team_project_context(&f.workspace, &snapshot.project, &f.owner)
            .unwrap();
        let chat = serde_json::to_string(
            &store
                .interactive_journal()
                .projected_chat(&context, id)
                .unwrap(),
        )
        .unwrap();
        assert!(!chat.contains(VALUE));
        // The worker's durable open command and its journal must never contain
        // the keyring envelope reference or the plaintext credential.
        for path in [
            state_dir.join("exports.sqlite3"),
            state_dir.join("exports.sqlite3-wal"),
            root.join("session-calls"),
        ] {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e)
                    if path.ends_with("exports.sqlite3-wal")
                        && e.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(e) => panic!("cannot inspect worker artifact {}: {e}", path.display()),
            };
            for needle in [VALUE.as_bytes(), b"scoped-secret:"] {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle),
                    "secret in worker artifact {}",
                    path.display()
                );
            }
        }
    }
    pub(super) fn assert_browser(
        &self,
        f: &Fixture,
        id: &str,
        state_dir: &std::path::Path,
        root: &std::path::Path,
    ) {
        self.assert_before_approval(3, false);
        self.assert_private(f, id, state_dir, root);
    }
    pub(super) fn assert_success(
        &self,
        f: &Fixture,
        id: &str,
        state_dir: &std::path::Path,
        root: &std::path::Path,
        denied: bool,
    ) {
        self.assert_before_approval(if denied { 0 } else { 2 }, denied);
        self.assert_private(f, id, state_dir, root);
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.release();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

pub(super) struct Failure<'a> {
    pub endpoint: &'a Endpoint,
    pub fixture: &'a Fixture,
    pub cookie: &'a str,
    pub project: &'a str,
    pub selection: &'a crate::interactive::mcp::Selection,
    pub id: &'a str,
    pub path: &'a str,
    pub payload: &'a Value,
    pub root: &'a std::path::Path,
    pub state_dir: &'a std::path::Path,
    pub faults: &'a crate::worker_client::session_faults::Guard,
    pub revoked: bool,
}
impl Failure<'_> {
    pub(super) async fn verify(&self, socket: &mut SessionSocket, confirmation: Value) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.endpoint.calls.load(Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual scoped TLS HTTP deadline");
        if self.revoked {
            let path = format!(
                "/v1/web/workspaces/{}/projects/{}/hosts/{}/scoped-secrets/bridge/grants/{}/revoke",
                self.fixture.workspace,
                self.project,
                self.fixture.host,
                self.selection.connection_id
            );
            let (status, result) = fixture_http(
                &self.fixture.state,
                self.cookie,
                &path,
                json!({"expected_revision":self.selection.revision}),
            )
            .await;
            assert_eq!(status, 200, "{result}");
            tokio::time::timeout(Duration::from_secs(2), async {
                while !self.endpoint.closed.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("revocation must close the actual TLS socket before fixture response/release");
            self.endpoint.release();
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let done = {
                    let store = self.fixture.state.store.lock().unwrap();
                    let s = store.remote_session_snapshot(self.id).unwrap();
                    s.phase == "stopped"
                        && s.sequence == 2
                        && s.confirmed_ack == 2
                        && s.ack_sequence == 2
                };
                if done {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("scoped unknown outcome stop/ACK");
        {
            let store = self.fixture.state.store.lock().unwrap();
            let context = store
                .team_project_context(&self.fixture.workspace, self.project, &self.fixture.owner)
                .unwrap();
            let (_, turns) = store
                .interactive_journal()
                .snapshot(&context, self.id)
                .unwrap();
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].phase, dunlin_session_runtime::TurnPhase::Unknown);
            let chat = serde_json::to_string(
                &store
                    .interactive_journal()
                    .projected_chat(&context, self.id)
                    .unwrap(),
            )
            .unwrap();
            assert!(!chat.contains("remembered:journal-context"));
            assert!(!chat.contains("scoped broker result"));
        }
        session_dispatch(socket, "remote-browser-1", confirmation).await;
        let (status, replay) = session_http(
            &self.fixture.state,
            self.cookie,
            self.path,
            self.payload.clone(),
        )
        .await;
        if self.revoked {
            assert_eq!(status, 409, "{replay}");
        } else {
            assert_eq!(status, 202, "{replay}");
            assert_eq!(replay["admission"]["replayed"], true);
            assert_eq!(replay["session"]["id"], self.id);
        }
        let held = fs::metadata(self.root.join("session-held-effect"))
            .unwrap()
            .len();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            held,
            fs::metadata(self.root.join("session-held-effect"))
                .unwrap()
                .len()
        );
        assert_eq!(self.endpoint.calls.load(Ordering::Acquire), 1);
        assert_eq!(self.endpoint.secrets.reads.load(Ordering::Acquire), 1);
        assert!(!self.root.join("effects").exists());
        self.endpoint
            .assert_private(self.fixture, self.id, self.state_dir, self.root);
        let db = rusqlite::Connection::open(self.state_dir.join("exports.sqlite3")).unwrap();
        let counts:(i64,i64,i64,i64)=db.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT min(ack_sequence) FROM worker_sessions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(counts, (1, 1, 0, 2));
        let calls = fs::read_to_string(self.root.join("session-calls")).unwrap();
        let calls: Vec<Value> = calls
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for method in ["initialize", "session/new", "session/prompt"] {
            assert_eq!(calls.iter().filter(|v| v["method"] == method).count(), 1);
        }
        self.faults.assert_single_turn_no_replay();
    }
}
