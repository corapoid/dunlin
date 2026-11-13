//! Controlled OAuth/MCP HTTP provider for the real remote product pipeline.
use super::*;
use dunlin_mcp_client::oauth::AuthContext;
use dunlin_vault::store::{MemoryStore, SecretStore};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
mod forms;
pub(super) use forms::content as form_content;
type Secrets = MemoryStore;

pub(super) struct Provider {
    secrets: Arc<Secrets>,
    forms: Arc<forms::Forms>,
    origin: String,
    calls: Arc<AtomicUsize>,
    _reply_bytes: Arc<AtomicUsize>,
    lose: Arc<AtomicBool>,
    hold: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Provider {
    pub(super) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let reply_bytes = Arc::new(AtomicUsize::new(0));
        let lose = Arc::new(AtomicBool::new(false));
        let hold = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (url, count, lost, held, stopped) = (
            origin.clone(),
            calls.clone(),
            lose.clone(),
            hold.clone(),
            stop.clone(),
        );
        let forms = Arc::new(forms::Forms::default());
        let form_flow = forms.clone();
        let result_size = reply_bytes.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    headers.push_str(&line);
                    assert!(headers.len() < 16384);
                    if line == "\r\n" {
                        break;
                    }
                }
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length < 32768);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let first = headers.lines().next().unwrap();
                let (status, extra, value) = if first == "POST /mcp HTTP/1.1" {
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    if headers.lines().any(|line| {
                        line.eq_ignore_ascii_case("Authorization: Bearer pipeline-private-access")
                    }) {
                        let result = match request["method"].as_str().unwrap() {
                            "server/discover" => {
                                json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
                            }
                            "tools/list" => {
                                json!({"resultType":"complete","tools":[{"name":"lookup","inputSchema":{"type":"object"}}],"ttlMs":0,"cacheScope":"private"})
                            }
                            "tools/call" => {
                                assert_eq!(request["params"]["name"], "lookup");
                                assert_eq!(
                                    request["params"]["arguments"],
                                    json!({"query":"native fixture"})
                                );
                                if request["params"].get("requestState").is_none() {
                                    count.fetch_add(1, Ordering::Release);
                                }
                                if let Some(reply) = form_flow.reply(&request) {
                                    reply
                                } else {
                                    let (lock, changed) = held.as_ref();
                                    let mut wait = lock.lock().unwrap();
                                    while *wait && !stopped.load(Ordering::Acquire) {
                                        wait = changed.wait(wait).unwrap();
                                    }
                                    if lost.load(Ordering::Acquire) {
                                        continue;
                                    }
                                    json!({"resultType":"complete","content":[{"type":"text","text":if result_size.load(Ordering::Acquire)==0 {"native broker result".into()} else {"x".repeat(result_size.load(Ordering::Acquire))}}],"ttlMs":0,"cacheScope":"private"})
                                }
                            }
                            _ => panic!("unexpected MCP method"),
                        };
                        (
                            200,
                            String::new(),
                            json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                        )
                    } else {
                        assert_eq!(request["method"], "server/discover");
                        (
                            401,
                            format!(
                                "WWW-Authenticate: Bearer resource_metadata=\"{url}/resource-metadata\"\r\n"
                            ),
                            json!({}),
                        )
                    }
                } else if first == "GET /resource-metadata HTTP/1.1" {
                    (
                        200,
                        String::new(),
                        json!({"resource":format!("{url}/mcp"),"authorization_servers":[format!("{url}/issuer")],"scopes_supported":["files:read"]}),
                    )
                } else if first == "GET /.well-known/oauth-authorization-server/issuer HTTP/1.1" {
                    (
                        200,
                        String::new(),
                        json!({"issuer":format!("{url}/issuer"),"authorization_endpoint":format!("{url}/authorize"),"token_endpoint":format!("{url}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true}),
                    )
                } else if first == "POST /token HTTP/1.1" {
                    let form = url::form_urlencoded::parse(&body)
                        .into_owned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    assert_eq!(form["grant_type"], "authorization_code");
                    assert!(form.contains_key("code_verifier"));
                    (
                        200,
                        String::new(),
                        json!({"access_token":"pipeline-private-access","refresh_token":"pipeline-private-refresh","token_type":"Bearer","expires_in":3600,"scope":"files:read"}),
                    )
                } else {
                    panic!("unexpected HTTP request");
                };
                let body = value.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            secrets: Arc::new(Secrets::default()),
            forms,
            origin,
            calls,
            _reply_bytes: reply_bytes,
            lose,
            hold,
            stop,
            thread: Some(thread),
        }
    }
    pub(super) fn enable_forms(&self, mode: usize) {
        self.forms.mode.store(mode, Ordering::Release);
    }
    pub(super) fn continuations(&self) -> usize {
        self.forms.continuations.load(Ordering::Acquire)
    }
    pub(super) fn release_hook(&self) -> Arc<dyn Fn() + Send + Sync> {
        let hold = self.hold.clone();
        Arc::new(move || {
            *hold.0.lock().unwrap() = false;
            hold.1.notify_all();
        })
    }
    pub(super) fn release(&self) {
        *self.hold.0.lock().unwrap() = false;
        self.hold.1.notify_all();
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.release();
        self.thread.take().unwrap().join().unwrap();
    }
}

impl Provider {
    pub(super) fn lose_response(&self) {
        self.lose.store(true, Ordering::Release);
    }
    pub(super) fn hold_response(&self) {
        *self.hold.0.lock().unwrap() = true;
    }
    pub(super) async fn wait_call(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.calls() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual HTTP tools/call deadline");
    }
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
    pub(super) fn tool() -> dunlin_domain::agent::AgentMcpTool {
        let tool: dunlin_mcp_client::Tool =
            serde_json::from_value(json!({"name":"lookup","inputSchema":{"type":"object"}}))
                .unwrap();
        dunlin_domain::agent::AgentMcpTool {
            provider_id: "work".into(),
            name: "lookup".into(),
            definition_digest: tool.definition_digest().unwrap(),
        }
    }
    pub(super) fn install(
        &self,
        state: &mut AppState,
        owner: &str,
        workspace: &str,
    ) -> crate::interactive::mcp::Selection {
        let mut config = (*state.config).clone();
        config.mcp_oauth = vec![
            serde_json::from_value(json!({
                "id":"work","name":"Pipeline fixture","endpoint":format!("{}/mcp",self.origin),
                "issuer":format!("{}/issuer",self.origin),"client_id":"dunlin",
                "redirect_uri":"https://relay.tail123.ts.net/v1/web/mcp/oauth/work/callback",
                "client_secret_ref":null,"scopes":["files:read"]
            }))
            .unwrap(),
        ];
        state.config = Arc::new(config);
        state.mcp_oauth = Arc::new(crate::mcp_oauth::Service::testing(self.secrets.clone()));
        let context = AuthContext {
            user_id: owner.into(),
            workspace_id: workspace.into(),
            connection_id: uuid::Uuid::now_v7().to_string(),
            connection_revision: 1,
        };
        let provider = &state.config.mcp_oauth[0];
        let flow = provider.flow(self.secrets.as_ref()).unwrap();
        let authorization = flow
            .begin(context.clone(), vec!["files:read".into()])
            .unwrap();
        let authorization = url::Url::parse(&authorization.url).unwrap();
        let nonce = authorization
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let mut callback = url::Url::parse(&provider.redirect_uri).unwrap();
        callback
            .query_pairs_mut()
            .append_pair("state", &nonce)
            .append_pair("code", "pipeline-code")
            .append_pair("iss", &provider.issuer);
        let token = flow.finish(&context, callback.as_str()).unwrap();
        let store = state.store.lock().unwrap();
        store
            .mcp_oauth_prepare(
                &context,
                &flow.credential_binding(),
                "work",
                None,
                &|_, _| Ok(()),
            )
            .unwrap();
        flow.install_durable(&context, token, &*store, |id, value| {
            self.secrets
                .set(id, value)
                .map_err(|_| dunlin_mcp_client::oauth::persistence::PersistenceError::Storage)
        })
        .unwrap();
        crate::interactive::mcp::Selection {
            provider_id: "work".into(),
            connection_id: context.connection_id,
            revision: 1,
        }
    }
}
