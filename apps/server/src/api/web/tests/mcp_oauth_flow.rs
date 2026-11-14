use super::*;
use crate::mcp_oauth::{
    Service,
    config::{Authentication, Provider},
};
use dunlin_mcp_client::oauth::{AuthContext, persistence::CredentialJournal};
use dunlin_vault::store::{MemoryStore, SecretStore};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use zeroize::Zeroizing;
type Hook = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct Secrets {
    values: MemoryStore,
    writes: Mutex<Vec<Zeroizing<String>>>,
    after_write: Mutex<Option<Hook>>,
    lose_write: AtomicBool,
    after_read: Mutex<Option<Hook>>,
    allow_delete: AtomicBool,
    lose_delete: AtomicBool,
    deletes: Mutex<Vec<String>>,
    deleted_values: Mutex<Vec<(String, Zeroizing<String>)>>,
    after_delete: Mutex<Option<Hook>>,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        let value = self.values.get(id);
        if let Some(hook) = self.after_read.lock().unwrap().take() {
            hook();
        }
        value
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.values.set(id, value)?;
        self.writes
            .lock()
            .unwrap()
            .push(Zeroizing::new(value.into()));
        let hook = self.after_write.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        if self.lose_write.swap(false, Ordering::SeqCst) {
            return Err("fixture-private-error-after-write".into());
        }
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        assert!(self.allow_delete.load(Ordering::SeqCst), "no implicit GC");
        let previous = self.values.get(id).ok();
        self.values.delete(id)?;
        if let Some(value) = previous {
            self.deleted_values
                .lock()
                .unwrap()
                .push((id.to_owned(), value));
        }
        self.deletes.lock().unwrap().push(id.to_owned());
        if let Some(hook) = self.after_delete.lock().unwrap().take() {
            hook();
        }
        if self.lose_delete.swap(false, Ordering::SeqCst) {
            return Err("fixture-private-delete-error-after-delete".into());
        }
        Ok(())
    }
    fn get_policy(&self, _: &str) -> Result<Zeroizing<String>, String> {
        panic!("OAuth is private")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("OAuth is not a vault tool")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("OAuth is private")
    }
}
struct ProviderServer {
    revocation_requests: Arc<AtomicUsize>,
    lose_revocation: Arc<AtomicBool>,
    on_revocation: Arc<Mutex<Option<Hook>>>,
    tool_calls: Arc<AtomicUsize>,
    allow_calls: Arc<AtomicBool>,
    lose_call: Arc<AtomicBool>,
    origin: String,
    stop: Arc<AtomicBool>,
    token_requests: Arc<AtomicUsize>,
    stall_token: Arc<AtomicBool>,
    closed_tokens: Arc<AtomicUsize>,
    mcp_requests: Arc<AtomicUsize>,
    expires_in: Arc<AtomicUsize>,
    on_catalog: Arc<Mutex<Option<Hook>>>,
    lose_refresh: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    on_token: Arc<Mutex<Option<Hook>>>,
    on_discovery: Arc<Mutex<Option<Hook>>>,
    stall_discovery: Arc<AtomicBool>,
    closed_discovery: Arc<AtomicUsize>,
    cimd_supported: Arc<AtomicBool>,
    client_ids: Arc<Mutex<Vec<String>>>,
}
impl ProviderServer {
    fn new() -> Self {
        let revocation_requests = Arc::new(AtomicUsize::new(0));
        let revoke_count = revocation_requests.clone();
        let lose_revocation = Arc::new(AtomicBool::new(false));
        let revoke_lost = lose_revocation.clone();
        let on_revocation: Arc<Mutex<Option<Hook>>> = Arc::default();
        let revoke_hook = on_revocation.clone();
        let tool_calls = Arc::new(AtomicUsize::new(0));
        let allow_calls = Arc::new(AtomicBool::new(false));
        let (calls, allowed) = (tool_calls.clone(), allow_calls.clone());
        let lose_call = Arc::new(AtomicBool::new(false));
        let lost_call = lose_call.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let token_requests = Arc::new(AtomicUsize::new(0));
        let stall_token = Arc::new(AtomicBool::new(false));
        let closed_tokens = Arc::new(AtomicUsize::new(0));
        let (stall, closed) = (stall_token.clone(), closed_tokens.clone());
        let mcp_requests = Arc::new(AtomicUsize::new(0));
        let expires_in = Arc::new(AtomicUsize::new(3600));
        let lose_refresh = Arc::new(AtomicBool::new(false));
        let on_catalog: Arc<Mutex<Option<Hook>>> = Arc::default();
        let (mcps, expiry, catalog_hook, lose) = (
            mcp_requests.clone(),
            expires_in.clone(),
            on_catalog.clone(),
            lose_refresh.clone(),
        );
        let on_token: Arc<Mutex<Option<Hook>>> = Arc::new(Mutex::new(None));
        let on_discovery: Arc<Mutex<Option<Hook>>> = Arc::default();
        let stall_discovery = Arc::new(AtomicBool::new(false));
        let closed_discovery = Arc::new(AtomicUsize::new(0));
        let (discovery_hook, discovery_stall, discovery_closed) = (
            on_discovery.clone(),
            stall_discovery.clone(),
            closed_discovery.clone(),
        );
        let cimd_supported = Arc::new(AtomicBool::new(false));
        let support = cimd_supported.clone();
        let client_ids = Arc::new(Mutex::new(Vec::new()));
        let ids = client_ids.clone();
        let (url, done, tokens, hook) = (
            origin.clone(),
            stop.clone(),
            token_requests.clone(),
            on_token.clone(),
        );
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let mut socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
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
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length < 32768);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let first = headers.lines().next().unwrap();
                let (status, extra, body) = if first == "POST /mcp HTTP/1.1" {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    if headers.lines().any(|line| {
                        line.eq_ignore_ascii_case("Authorization: Bearer fixture-private-access")
                    }) {
                        mcps.fetch_add(1, Ordering::SeqCst);
                        let result = match request["method"].as_str().unwrap() {
                            "server/discover" => {
                                json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
                            }
                            "tools/list" => {
                                if let Some(hook) = catalog_hook.lock().unwrap().take() {
                                    hook();
                                }
                                json!({"resultType":"complete","tools":[{"name":"lookup","inputSchema":{"type":"object"}}],"ttlMs":0,"cacheScope":"private"})
                            }
                            "tools/call" => {
                                assert!(
                                    allowed.load(Ordering::SeqCst),
                                    "catalog inspection must never call a tool"
                                );
                                assert_eq!(request["params"]["name"], "lookup");
                                assert_eq!(
                                    request["params"]["arguments"],
                                    json!({"query":"approved fixture"})
                                );
                                calls.fetch_add(1, Ordering::SeqCst);
                                if lost_call.load(Ordering::SeqCst) {
                                    continue;
                                }
                                json!({"resultType":"complete","content":[{"type":"text","text":"private fixture result"}],"ttlMs":0,"cacheScope":"private"})
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
                    if let Some(hook) = discovery_hook.lock().unwrap().take() {
                        hook();
                    }
                    if discovery_stall.swap(false, Ordering::SeqCst) {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let result = socket.read(&mut [0; 1]);
                        assert!(
                            matches!(result, Ok(0))
                                || matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset),
                            "discovery socket remained open: {result:?}"
                        );
                        discovery_closed.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    (
                        200,
                        String::new(),
                        json!({"issuer":format!("{url}/issuer"),"authorization_endpoint":format!("{url}/authorize"),"token_endpoint":format!("{url}/token"),"revocation_endpoint":format!("{url}/revoke"),"revocation_endpoint_auth_methods_supported":["none"],"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true,"client_id_metadata_document_supported":support.load(Ordering::SeqCst)}),
                    )
                } else if first == "POST /revoke HTTP/1.1" {
                    let form = url::form_urlencoded::parse(&body)
                        .into_owned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
                    assert!(form.contains_key("client_id"));
                    assert!(!form.contains_key("client_secret"));
                    match form["token_type_hint"].as_str() {
                        "refresh_token" => assert_eq!(form["token"], "fixture-private-refresh"),
                        "access_token" => assert_eq!(form["token"], "fixture-private-access"),
                        _ => panic!("unexpected revocation token hint"),
                    }
                    revoke_count.fetch_add(1, Ordering::SeqCst);
                    if let Some(hook) = revoke_hook.lock().unwrap().take() {
                        hook();
                    }
                    if revoke_lost.swap(false, Ordering::SeqCst) {
                        continue;
                    }
                    (200, String::new(), json!({}))
                } else if first == "POST /token HTTP/1.1" {
                    tokens.fetch_add(1, Ordering::SeqCst);
                    let form = url::form_urlencoded::parse(&body)
                        .into_owned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
                    assert!(!form.contains_key("client_secret"));
                    ids.lock().unwrap().push(form["client_id"].clone());
                    assert_eq!(form["resource"], format!("{url}/mcp"));
                    if form["grant_type"] == "refresh_token" {
                        assert_eq!(form["refresh_token"], "fixture-private-refresh");
                        if lose.load(Ordering::SeqCst) {
                            continue;
                        }
                    } else {
                        assert_eq!(form["grant_type"], "authorization_code");
                        assert_eq!(
                            form["redirect_uri"],
                            "https://build.tail123.ts.net/v1/web/mcp/oauth/fixture/callback"
                        );
                        assert_eq!(form["code_verifier"].len(), 43);
                    }
                    let callback = hook.lock().unwrap().take();
                    if let Some(callback) = callback {
                        callback();
                    }
                    if stall.swap(false, Ordering::SeqCst) {
                        // Do not release a response: the client must close I/O
                        // after the hook revokes its actual browser authority.
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let result = socket.read(&mut [0; 1]);
                        assert!(
                            matches!(result, Ok(0))
                                || matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset),
                            "AS socket remained open: {result:?}"
                        );
                        closed.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    (
                        200,
                        String::new(),
                        json!({"access_token":"fixture-private-access","refresh_token":"fixture-private-refresh","token_type":"Bearer","expires_in":expiry.load(Ordering::SeqCst),"scope":"files:read"}),
                    )
                } else {
                    panic!("Unexpected OAuth request: {first}")
                };
                let body = body.to_string();
                if let Err(error) = write!(
                    socket,
                    "HTTP/1.1 {status} Fixture\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ) {
                    assert!(
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                        ),
                        "unexpected fixture response error: {error}"
                    );
                }
            }
        });
        Self {
            revocation_requests,
            lose_revocation,
            on_revocation,
            tool_calls,
            allow_calls,
            lose_call,
            origin,
            stop,
            token_requests,
            stall_token,
            closed_tokens,
            mcp_requests,
            expires_in,
            on_catalog,
            lose_refresh,
            worker: Some(worker),
            on_token,
            on_discovery,
            stall_discovery,
            closed_discovery,
            cimd_supported,
            client_ids,
        }
    }
    fn provider(&self) -> Provider {
        Provider {
            id: "fixture".into(),
            name: "Fixture MCP".into(),
            endpoint: format!("{}/mcp", self.origin),
            issuer: format!("{}/issuer", self.origin),
            client_id: "fixture-client".into(),
            redirect_uri: "https://build.tail123.ts.net/v1/web/mcp/oauth/fixture/callback".into(),
            allowed_origins: vec![],
            scopes: vec!["files:read".into()],
            authentication: Authentication::Public,
            client_secret_ref: None,
        }
    }
}
impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
async fn setup() -> (
    AppState,
    ProviderServer,
    Arc<Secrets>,
    String,
    String,
    String,
) {
    let (mut state, _, _) = fixture();
    let provider = ProviderServer::new();
    let secrets = Arc::new(Secrets::default());
    state.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
    let mut config = (*state.config).clone();
    config.mcp_oauth = vec![provider.provider()];
    state.config = Arc::new(config);
    let (user, workspace) = {
        let store = state.store.lock().unwrap();
        let user = store.create_user("mcp-flow-owner", "admin").unwrap();
        let workspace = store.create_workspace(&user, "MCP OAuth").unwrap();
        (user.id, workspace.id)
    };
    let cookie = paired_cookie(&state, &user).await;
    (state, provider, secrets, user, workspace, cookie)
}

#[tokio::test]
async fn mcp_runtime_real_broker_requires_exact_human_approval_and_dedupes_replay() {
    mcp_broker_fixture(false).await;
}

#[tokio::test]
#[cfg(target_os = "linux")]
#[ignore = "requires production relay executable and Linux bwrap/LSM fixture"]
async fn production_browser_acceptance_runtime_mcp_stdio_broker() {
    assert!(
        std::path::Path::new(
            &std::env::var_os("DUNLIN_RELAY_E2E_BIN").expect("production executable required")
        )
        .is_file()
    );
    mcp_broker_fixture(true).await;
}

async fn mcp_broker_fixture(real_proxy: bool) {
    for (approved, disconnect, lose_response) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let (mut state, provider, _, user, workspace, cookie) = setup().await;
        provider.allow_calls.store(true, Ordering::SeqCst);
        provider.lose_call.store(lose_response, Ordering::SeqCst);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let mut config = (*state.config).clone();
        config.db = directory.path().join("relay.db");
        config.experimental_interactive_runtime = true;
        config.sandbox = real_proxy;
        state.config = Arc::new(config);
        let script = root.join("fake_acp.py");
        std::fs::write(
            &script,
            include_str!("../../../interactive/tests/fake_acp.py"),
        )
        .unwrap();
        Arc::get_mut(&mut state.interactive).unwrap().command = Some((
            "/usr/bin/python3".into(),
            vec![
                script.to_str().unwrap().into(),
                if real_proxy { "mcp-real" } else { "mcp" }.into(),
            ],
        ));
        let (started, flow_cookie) = begun(&state, &workspace, &cookie, json!({})).await;
        assert_eq!(
            callback(&state, &callback_url(&provider, &started), &flow_cookie)
                .await
                .status(),
            StatusCode::SEE_OTHER
        );
        let connection_id = started["connection_id"].as_str().unwrap().to_owned();
        let (auth, project, profile) = {
            let store = state.store.lock().unwrap();
            let device = store
                .browser_session_device(cookie.split_once('=').unwrap().1)
                .unwrap()
                .unwrap();
            let auth = crate::api::AuthUser {
                user: store.find_user(&user).unwrap().unwrap(),
                device,
            };
            let project = store
                .create_team_project(&user, &workspace, "MCP runtime")
                .unwrap();
            let native = store
                .create_project("MCP checkout", root.to_str().unwrap())
                .unwrap();
            store.grant(&native.id, &user).unwrap();
            store
                .set_local_binding(&user, &workspace, &project.id, &native.id, None)
                .unwrap();
            let tool = dunlin_mcp_client::Tool {
                name: "lookup".into(),
                input_schema: json!({"type":"object"}),
                output_schema: None,
            };
            let definition = dunlin_domain::agent::AgentDefinition {
                schema_version: 2,
                name: "MCP runtime".into(),
                description: String::new(),
                harness: "opencode".into(),
                model: "fixture-model".into(),
                mode: "plan".into(),
                instructions: None,
                skills: vec![],
                mcp: vec![dunlin_domain::agent::AgentMcpTool {
                    provider_id: "fixture".into(),
                    name: tool.name.clone(),
                    definition_digest: tool.definition_digest().unwrap(),
                }],
                max_seconds: 60,
                max_output_bytes: 1024 * 1024,
            };
            let draft = store
                .create_agent(&user, &workspace, &project.id, definition)
                .unwrap();
            let version = store
                .publish_agent(&user, &workspace, &project.id, &draft.id, draft.revision)
                .unwrap();
            (
                auth,
                project.id,
                crate::interactive::ProfileRef {
                    agent_id: version.agent_id,
                    version: version.version,
                    digest: version.digest,
                },
            )
        };
        let selections = vec![crate::interactive::mcp::Selection {
            provider_id: "fixture".into(),
            connection_id: connection_id.clone(),
            revision: 1,
        }];
        for mut invalid in [selections.clone(), selections.clone(), selections.clone()]
            .into_iter()
            .enumerate()
        {
            match invalid.0 {
                0 => invalid.1[0].revision = 2,
                1 => invalid.1[0].connection_id = "another-private-account".into(),
                _ => invalid.1[0].provider_id = "different-provider".into(),
            }
            assert!(
                crate::interactive::Service::create_request_with_key(
                    state.clone(),
                    crate::api::AuthUser {
                        user: auth.user.clone(),
                        device: auth.device.clone()
                    },
                    workspace.clone(),
                    project.clone(),
                    crate::interactive::CreateRequest::Profile {
                        profile: profile.clone(),
                        mcp_connections: invalid.1,
                        target: None
                    },
                    Some(format!("mcp-invalid-selection-{}", invalid.0))
                )
                .is_err()
            );
            assert!(!root.join("actor-starts").exists());
        }
        let acceptance = crate::interactive::Service::create_request_with_key(
            state.clone(),
            crate::api::AuthUser {
                user: auth.user.clone(),
                device: auth.device.clone(),
            },
            workspace.clone(),
            project.clone(),
            crate::interactive::CreateRequest::Profile {
                profile,
                mcp_connections: selections,
                target: None,
            },
            Some("mcp-real-broker-fixture".into()),
        )
        .unwrap();
        let session = acceptance.session;
        {
            let store = state.store.lock().unwrap();
            let context = store
                .team_project_context(&workspace, &project, &user)
                .unwrap();
            store
                .interactive_journal()
                .enqueue(
                    &context,
                    &session.id,
                    "mcp-prompt",
                    "perform fixture lookup",
                )
                .unwrap();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let action = loop {
            let action = {
                let store = state.store.lock().unwrap();
                let context = store
                    .team_project_context(&workspace, &project, &user)
                    .unwrap();
                let chat = serde_json::to_value(
                    store
                        .interactive_journal()
                        .projected_chat(&context, &session.id)
                        .unwrap(),
                )
                .unwrap();
                chat["activeTurn"]["responseParts"].as_array().and_then(|parts|parts.iter().find(|p|p["kind"]=="toolCall"&&p["toolCall"]["status"]=="pending-confirmation")).map(|part| {
                    let tool=&part["toolCall"];
                    let option=tool["options"].as_array().unwrap().iter().find(|o|o["kind"]==if approved{"approve"}else{"deny"}).unwrap();
                    serde_json::from_value::<ahp_types::commands::DispatchActionParams>(json!({"channel":format!("ahp-chat:/{}",session.id),"clientSeq":1,"action":{"type":"chat/toolCallConfirmed","turnId":chat["activeTurn"]["id"],"toolCallId":tool["toolCallId"],"approved":approved,"selectedOptionId":option["id"]}})).unwrap()
                })
            };
            if let Some(action) = action {
                break action;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "MCP permission did not appear"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        };
        assert_eq!(provider.tool_calls.load(Ordering::SeqCst), 0);
        {
            let store = state.store.lock().unwrap();
            let context = store
                .team_project_context(&workspace, &project, &user)
                .unwrap();
            let receipt = store
                .interactive_journal()
                .dispatch_action(
                    &context,
                    &session.id,
                    &auth.device.id,
                    "mcp-human-choice",
                    &action,
                    dunlin_session_runtime::DispatchAdmission::default(),
                )
                .unwrap();
            assert!(
                receipt.rejection_reason.is_none(),
                "{:?}",
                receipt.rejection_reason
            );
            if disconnect {
                assert_eq!(
                    store
                        .disable(&AuthContext {
                            user_id: user.clone(),
                            workspace_id: workspace.clone(),
                            connection_id: connection_id.clone(),
                            connection_revision: 1
                        })
                        .unwrap(),
                    2
                );
            }
        }
        loop {
            let complete = {
                let store = state.store.lock().unwrap();
                let context = store
                    .team_project_context(&workspace, &project, &user)
                    .unwrap();
                let snapshot = store
                    .interactive_journal()
                    .snapshot(&context, &session.id)
                    .unwrap();
                if disconnect || lose_response {
                    snapshot.0.phase == dunlin_session_runtime::SessionPhase::Interrupted
                } else {
                    snapshot
                        .1
                        .iter()
                        .any(|t| t.phase == dunlin_session_runtime::TurnPhase::Completed)
                }
            };
            if complete {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "MCP turn did not complete"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert_eq!(
            provider.tool_calls.load(Ordering::SeqCst),
            usize::from(approved && !disconnect)
        );
        if lose_response {
            let store = state.store.lock().unwrap();
            let context = store
                .team_project_context(&workspace, &project, &user)
                .unwrap();
            let events = store
                .interactive_journal()
                .replay(&context, &session.id, 0)
                .unwrap();
            assert!(
                events
                    .iter()
                    .any(|event| event.kind == "permission_lifecycle"
                        && event.payload["actions"].as_array().is_some_and(|actions| {
                            actions.iter().any(|action| {
                                action["_meta"]["dunlin.permissionUnconfirmed"] == true
                            })
                        }))
            );
        }
        if disconnect || lose_response {
            continue;
        }
        let result = std::fs::read_to_string(root.join("received-mcp-result")).unwrap();
        assert!(result.contains(if approved {
            "private fixture result"
        } else {
            "user denied"
        }));
        assert!(!result.contains("fixture-private-access"));
        assert!(!result.contains("fixture-private-refresh"));
        state
            .interactive
            .stop(&state, &auth, &session.id, session.epoch)
            .unwrap();
    }
}
async fn start(state: &AppState, workspace: &str, cookie: &str, body: Value) -> Response {
    super::super::super::router(state.clone())
        .oneshot(
            Request::post(format!(
                "/v1/web/workspaces/{workspace}/mcp-providers/fixture/authorize"
            ))
            .header("host", "build.tail123.ts.net")
            .header("origin", "https://build.tail123.ts.net")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn mcp_oauth_cimd_checks_advertised_support_and_uses_url_identity_through_callback_and_broker()
 {
    let (mut state, provider, secrets, user, workspace, cookie) = setup().await;
    let client_id = "https://client.example/oauth/client.json";
    let mut config = (*state.config).clone();
    config.mcp_oauth[0].authentication = Authentication::ClientMetadataDocument;
    config.mcp_oauth[0].client_id = client_id.into();
    state.config = Arc::new(config);
    assert_eq!(
        start(&state, &workspace, &cookie, json!({})).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    assert!(secrets.writes.lock().unwrap().is_empty());
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connections(&user, &workspace)
            .unwrap()
            .is_empty()
    );
    provider.cimd_supported.store(true, Ordering::SeqCst);
    let (started, flow_cookie) = begun(&state, &workspace, &cookie, json!({})).await;
    let authorize = url::Url::parse(started["authorization_url"].as_str().unwrap()).unwrap();
    let query = authorize
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(query["client_id"], client_id);
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(!query.contains_key("client_secret"));
    assert_eq!(
        callback(&state, &callback_url(&provider, &started), &flow_cookie)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        *provider.client_ids.lock().unwrap(),
        vec![client_id.to_owned()]
    );
    let id = started["connection_id"].as_str().unwrap();
    let inspect = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect");
    let result = team_request(
        &state,
        "POST",
        &inspect,
        &cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["tools"], json!(["lookup"]));
    let writes = secrets.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    let envelope: Value = serde_json::from_str(&writes[0]).unwrap();
    assert_eq!(envelope["user_id"], user);
    assert_eq!(envelope["connection_id"], id);
    assert!(!started.to_string().contains("fixture-private"));
    assert!(!result.1.to_string().contains("fixture-private"));
}

#[tokio::test]
async fn mcp_oauth_browser_stale_identity_cannot_read_start_or_disconnect() {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    let base = format!("/v1/web/workspaces/{workspace}");
    for (method, path) in [
        ("GET", format!("{base}/mcp-providers")),
        ("GET", format!("{base}/mcp-connections")),
        ("POST", format!("{base}/mcp-providers/fixture/authorize")),
    ] {
        for expected in [vec!["stale-user"], vec![user.as_str(), user.as_str()]] {
            let mut request = Request::builder()
                .method(method)
                .uri(&path)
                .header("host", "build.tail123.ts.net")
                .header("origin", "https://build.tail123.ts.net")
                .header("cookie", &cookie)
                .header("content-type", "application/json");
            for value in expected {
                request = request.header("x-dunlin-expected-user", value);
            }
            let response = super::super::super::router(state.clone())
                .oneshot(request.body(Body::from("{}")).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connections(&user, &workspace)
            .unwrap()
            .is_empty()
    );
    let (started, _) = begun(
        &state,
        &workspace,
        &cookie,
        json!({"display_name":"Konto służbowe"}),
    )
    .await;
    let id = started["connection_id"].as_str().unwrap();
    let path = format!("{base}/mcp-connections/{id}");
    for (method, suffix, payload) in [
        ("GET", "", "null"),
        ("POST", "/disconnect", "{\"expected_revision\":1}"),
        ("POST", "/inspect", "{\"expected_revision\":1}"),
    ] {
        let response = super::super::super::router(state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(format!("{path}{suffix}"))
                    .header("host", "build.tail123.ts.net")
                    .header("origin", "https://build.tail123.ts.net")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .header("x-dunlin-expected-user", "stale-user")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = super::super::super::router(state.clone())
        .oneshot(
            Request::get(path)
                .header("host", "build.tail123.ts.net")
                .header("cookie", cookie)
                .header("x-dunlin-expected-user", user)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let row: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(row["revision"], 1);
    assert_eq!(row["status"], "authorization_incomplete");
    assert_eq!(row["display_name"], "Konto służbowe");
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    assert!(secrets.writes.lock().unwrap().is_empty());
}
async fn begun(state: &AppState, workspace: &str, cookie: &str, body: Value) -> (Value, String) {
    let response = start(state, workspace, cookie, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let flow = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(flow.contains("HttpOnly; SameSite=Lax; Max-Age=300; Secure"));
    assert!(!flow.contains("dunlin_web_session"));
    let result =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    (result, flow.split(';').next().unwrap().to_owned())
}
fn callback_url(provider: &ProviderServer, started: &Value) -> String {
    let request = url::Url::parse(started["authorization_url"].as_str().unwrap()).unwrap();
    let state = request
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert!(request.query_pairs().any(|(k, _)| k == "code_challenge"));
    assert!(!request.query_pairs().any(|(k, _)| k == "code_verifier"));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", &state)
        .append_pair("code", "fixture-code")
        .append_pair("iss", &format!("{}/issuer", provider.origin))
        .finish();
    format!("/v1/web/mcp/oauth/fixture/callback?{query}")
}
async fn callback(state: &AppState, path: &str, flow: &str) -> Response {
    // No main Strict session cookie or same-origin header on AS navigation.
    super::super::super::router(state.clone())
        .oneshot(
            Request::get(path)
                .header("host", "build.tail123.ts.net")
                .header("cookie", flow)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn established(
    expiry: usize,
) -> (
    AppState,
    ProviderServer,
    Arc<Secrets>,
    String,
    String,
    String,
    String,
) {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    provider.expires_in.store(expiry, Ordering::SeqCst);
    let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
    let path = callback_url(&provider, &started);
    assert_eq!(
        callback(&state, &path, &flow).await.status(),
        StatusCode::SEE_OTHER
    );
    let id = started["connection_id"].as_str().unwrap().to_owned();
    (state, provider, secrets, user, workspace, cookie, id)
}

#[tokio::test]
async fn mcp_oauth_broker_catalog_is_personal_cas_checked_and_never_calls_tools() {
    let (state, provider, secrets, user, workspace, cookie, id) = established(3600).await;
    let path = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect");
    let other = state
        .store
        .lock()
        .unwrap()
        .create_user("catalog-other", "admin")
        .unwrap();
    let other_cookie = paired_cookie(&state, &other.id).await;
    for (who, revision, expected) in [
        (&other_cookie, 1, StatusCode::NOT_FOUND),
        (&cookie, 2, StatusCode::CONFLICT),
    ] {
        assert_eq!(
            team_request(
                &state,
                "POST",
                &path,
                who,
                json!({"expected_revision":revision})
            )
            .await
            .0,
            expected
        );
    }
    let csrf = super::super::super::router(state.clone())
        .oneshot(
            Request::post(&path)
                .header("host", "build.tail123.ts.net")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from("{\"expected_revision\":1}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(csrf.status(), StatusCode::FORBIDDEN);
    assert_eq!(provider.mcp_requests.load(Ordering::SeqCst), 0);
    let (status, body) = team_request(
        &state,
        "POST",
        &path,
        &cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"connection_id":id,"revision":1,"protocol_version":"2026-07-28","tools":["lookup"],"tool_pins":[{"provider_id":"fixture","name":"lookup","definition_digest":"11f2036a698b4a784c54872e1e1f1d19de3fd7a1fbcba2612ffb6eb8f9044587"}]})
    );
    assert_eq!(provider.mcp_requests.load(Ordering::SeqCst), 2);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(secrets.writes.lock().unwrap().len(), 1);
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connections(&user, &workspace)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn mcp_oauth_broker_refreshes_expiry_before_catalog_and_lost_refresh_never_retries() {
    for lost in [false, true] {
        let (state, provider, secrets, _, workspace, cookie, id) = established(1).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        provider.expires_in.store(3600, Ordering::SeqCst);
        provider.lose_refresh.store(lost, Ordering::SeqCst);
        let path = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect");
        let (status, _) = team_request(
            &state,
            "POST",
            &path,
            &cookie,
            json!({"expected_revision":1}),
        )
        .await;
        assert_eq!(
            status,
            if lost {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(provider.token_requests.load(Ordering::SeqCst), 2);
        assert_eq!(
            provider.mcp_requests.load(Ordering::SeqCst),
            if lost { 0 } else { 2 }
        );
        assert_eq!(
            secrets.writes.lock().unwrap().len(),
            if lost { 1 } else { 2 }
        );
        if lost {
            assert_eq!(
                team_request(
                    &state,
                    "POST",
                    &path,
                    &cookie,
                    json!({"expected_revision":1})
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            assert_eq!(provider.token_requests.load(Ordering::SeqCst), 2);
        }
    }
}

#[tokio::test]
async fn mcp_oauth_broker_discards_catalog_after_disconnect_logout_or_membership_aba() {
    use dunlin_domain::collaboration::WorkspaceRole;
    for fault in ["logout_read", "disconnect_catalog", "membership_aba"] {
        let (state, provider, secrets, user, workspace, cookie, id) = established(3600).await;
        let store = state.store.clone();
        let hook: Hook = if fault == "logout_read" {
            let device = store
                .lock()
                .unwrap()
                .browser_session_device(cookie.split_once('=').unwrap().1)
                .unwrap()
                .unwrap()
                .id;
            Box::new(move || {
                store
                    .try_lock()
                    .expect("no Store lock during keyring read")
                    .revoke_device(&device)
                    .unwrap();
            })
        } else if fault == "disconnect_catalog" {
            let context = AuthContext {
                user_id: user.clone(),
                workspace_id: workspace.clone(),
                connection_id: id.clone(),
                connection_revision: 1,
            };
            Box::new(move || {
                store
                    .try_lock()
                    .expect("no Store lock during catalog HTTP")
                    .disable(&context)
                    .unwrap();
            })
        } else {
            let other = {
                let s = store.lock().unwrap();
                let other = s.create_user("catalog-other-owner", "admin").unwrap();
                s.set_team_member(
                    &user,
                    &workspace,
                    &other.id,
                    WorkspaceRole::Owner,
                    true,
                    None,
                )
                .unwrap();
                other.id
            };
            let workspace = workspace.clone();
            let user = user.clone();
            Box::new(move || {
                let s = store.try_lock().expect("no Store lock during catalog HTTP");
                s.set_team_member(
                    &other,
                    &workspace,
                    &user,
                    WorkspaceRole::Owner,
                    false,
                    Some(1),
                )
                .unwrap();
                s.set_team_member(
                    &other,
                    &workspace,
                    &user,
                    WorkspaceRole::Owner,
                    true,
                    Some(2),
                )
                .unwrap();
            })
        };
        if fault == "logout_read" {
            *secrets.after_read.lock().unwrap() = Some(hook);
        } else {
            *provider.on_catalog.lock().unwrap() = Some(hook);
        }
        let path = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect");
        let (status, body) = team_request(
            &state,
            "POST",
            &path,
            &cookie,
            json!({"expected_revision":1}),
        )
        .await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::BAD_GATEWAY),
            "{status}: {body}"
        );
        assert!(!body.to_string().contains("lookup"));
        assert_eq!(
            provider.mcp_requests.load(Ordering::SeqCst),
            if fault == "logout_read" { 0 } else { 2 }
        );
        assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn mcp_oauth_browser_full_code_flow_keeps_tokens_private_and_wrong_callbacks_do_not_consume_it()
 {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    let catalog = team_request(
        &state,
        "GET",
        &format!("/v1/web/workspaces/{workspace}/mcp-providers"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(
        catalog.1,
        json!({"providers":[{"id":"fixture","name":"Fixture MCP"}]})
    );
    assert_eq!(
        start(
            &state,
            &workspace,
            &cookie,
            json!({"connection_id":"missing-connection","expected_revision":1})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
    assert_eq!(
        start(&state, &workspace, &cookie, json!({})).await.status(),
        StatusCode::CONFLICT
    );
    let connection = started["connection_id"].as_str().unwrap();
    let path = callback_url(&provider, &started);
    let wrong = callback(
        &state,
        &path,
        &format!("dunlin_mcp_oauth_flow={}", crate::auth::random_token()),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert_eq!(wrong.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(wrong.headers()[header::REFERRER_POLICY], "no-referrer");
    assert!(!wrong.headers().contains_key(header::SET_COOKIE));
    let wrong = callback(
        &state,
        "/v1/web/mcp/oauth/fixture/callback?state=wrong&code=fixture-code",
        &flow,
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert_eq!(wrong.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(wrong.headers()[header::REFERRER_POLICY], "no-referrer");
    assert!(!wrong.headers().contains_key(header::SET_COOKIE));
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    let response = callback(&state, &path, &flow).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    let status = team_request(
        &state,
        "GET",
        &format!("/v1/web/workspaces/{workspace}/mcp-connections/{connection}"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(
        status.1,
        json!({"id":connection,"revision":1,"status":"connected","provider_id":"fixture",
            "storage":{"pending_references":1,"retained_history":0,
                "uncertain_history":0,"available_slots":255,"cleanup_available":false}})
    );
    {
        let writes = secrets.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let record: Value = serde_json::from_str(&writes[0]).unwrap();
        assert_eq!(record["user_id"], user);
        assert_eq!(record["access_token"], "fixture-private-access");
        assert!(!started.to_string().contains("fixture-private"));
        assert!(!status.1.to_string().contains("fixture-private"));
    }
    assert_eq!(
        callback(&state, &path, &flow).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn mcp_oauth_browser_callback_rechecks_logout_disconnect_config_and_authority_after_provider_or_vault()
 {
    for fault in [
        "logout_before",
        "disconnect",
        "config",
        "issuer",
        "logout_token",
        "logout_write",
    ] {
        let (state, provider, secrets, user, workspace, cookie) = setup().await;
        let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
        let connection = started["connection_id"].as_str().unwrap();
        let mut path = callback_url(&provider, &started);
        let device = state
            .store
            .lock()
            .unwrap()
            .browser_session_device(cookie.split_once('=').unwrap().1)
            .unwrap()
            .unwrap()
            .id;
        let revoke = {
            let store = state.store.clone();
            let device = device.clone();
            move || store.lock().unwrap().revoke_device(&device).unwrap()
        };
        let mut changed = state.clone();
        match fault {
            "logout_before" => revoke(),
            "disconnect" => {
                assert_eq!(
                    team_request(
                        &state,
                        "POST",
                        &format!(
                            "/v1/web/workspaces/{workspace}/mcp-connections/{connection}/disconnect"
                        ),
                        &cookie,
                        json!({"expected_revision":1})
                    )
                    .await
                    .0,
                    StatusCode::OK
                );
            }
            "config" => {
                let mut config = (*state.config).clone();
                config.mcp_oauth[0].client_id = "another-client".into();
                changed.config = Arc::new(config);
            }
            "issuer" => {
                let mut url =
                    url::Url::parse(&format!("https://build.tail123.ts.net{path}")).unwrap();
                let mut pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
                pairs.iter_mut().find(|(k, _)| k == "iss").unwrap().1 =
                    "https://wrong.example".into();
                url.query_pairs_mut().clear().extend_pairs(pairs);
                path = format!("{}?{}", url.path(), url.query().unwrap());
            }
            "logout_token" => *provider.on_token.lock().unwrap() = Some(Box::new(revoke)),
            _ => *secrets.after_write.lock().unwrap() = Some(Box::new(revoke)),
        }
        let response = callback(&changed, &path, &flow).await;
        assert_eq!(
            response.status(),
            if fault == "config" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            },
            "{fault}"
        );
        assert_eq!(
            provider.token_requests.load(Ordering::SeqCst),
            usize::from(matches!(fault, "logout_token" | "logout_write")),
            "{fault}"
        );
        assert_eq!(
            secrets.writes.lock().unwrap().len(),
            usize::from(fault == "logout_write")
        );
        let status = state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connection(&user, &workspace, connection)
            .unwrap()
            .unwrap();
        assert_ne!(serde_json::to_value(status).unwrap()["status"], "connected");
    }
}
#[tokio::test]
async fn mcp_oauth_browser_service_restart_requires_new_state_and_fences_prior_revision() {
    let (mut state, provider, secrets, _, workspace, cookie) = setup().await;
    let (first, old_flow) = begun(&state, &workspace, &cookie, json!({})).await;
    let old_path = callback_url(&provider, &first);
    state.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
    assert_eq!(
        callback(&state, &old_path, &old_flow).await.status(),
        StatusCode::FORBIDDEN
    );
    let (second, flow) = begun(
        &state,
        &workspace,
        &cookie,
        json!({"connection_id":first["connection_id"],"expected_revision":1}),
    )
    .await;
    assert_eq!(second["revision"], 2);
    assert_eq!(
        callback(&state, &old_path, &flow).await.status(),
        StatusCode::FORBIDDEN
    );
    let path = callback_url(&provider, &second);
    assert_eq!(
        callback(&state, &path, &flow).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mcp_oauth_browser_pending_limits_expiry_and_worker_capacity_do_not_expose_credentials() {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    let a = state.mcp_oauth.slots.clone().try_acquire_owned().unwrap();
    let b = state.mcp_oauth.slots.clone().try_acquire_owned().unwrap();
    assert_eq!(
        start(&state, &workspace, &cookie, json!({})).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(a);
    drop(b);
    let (first, first_flow) = begun(&state, &workspace, &cookie, json!({})).await;
    for _ in 0..3 {
        let browser = paired_cookie(&state, &user).await;
        begun(&state, &workspace, &browser, json!({})).await;
    }
    let fifth = paired_cookie(&state, &user).await;
    assert_eq!(
        start(&state, &workspace, &fifth, json!({})).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let catalog = team_request(
        &state,
        "GET",
        &format!("/v1/web/workspaces/{workspace}/mcp-connections"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(catalog.1["connections"].as_array().unwrap().len(), 4);
    state.mcp_oauth.expire_pending();
    assert_eq!(
        callback(&state, &callback_url(&provider, &first), &first_flow)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    begun(&state, &workspace, &fifth, json!({})).await;
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    assert!(secrets.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_oauth_browser_https_loopback_also_sets_secure_flow_cookie() {
    let (mut state, _provider, _secrets, _user, workspace, cookie) = setup().await;
    let mut config = (*state.config).clone();
    config.mcp_oauth[0].redirect_uri = "https://localhost/v1/web/mcp/oauth/fixture/callback".into();
    state.config = Arc::new(config);
    let response = super::super::super::router(state.clone())
        .oneshot(
            Request::post(format!(
                "/v1/web/workspaces/{workspace}/mcp-providers/fixture/authorize"
            ))
            .header("host", "localhost")
            .header("origin", "https://localhost")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("; Secure")
    );
}

#[tokio::test]
async fn mcp_oauth_provider_revoke_is_personal_durable_and_covers_retired_envelopes_without_replay()
{
    for lost in [false, true] {
        let (state, provider, secrets, user, workspace, cookie, id) = established(1).await;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
        assert_eq!(
            team_request(
                &state,
                "POST",
                &format!("{base}/inspect"),
                &cookie,
                json!({"expected_revision":1})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(secrets.writes.lock().unwrap().len(), 2);
        provider.lose_revocation.store(lost, Ordering::SeqCst);
        let other = state
            .store
            .lock()
            .unwrap()
            .create_user("revoke-other", "admin")
            .unwrap();
        let other_cookie = paired_cookie(&state, &other.id).await;
        assert_eq!(
            team_request(
                &state,
                "POST",
                &format!("{base}/revoke"),
                &other_cookie,
                json!({"expected_revision":1})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            team_request(
                &state,
                "POST",
                &format!("{base}/revoke"),
                &cookie,
                json!({"expected_revision":2})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 0);
        let observer = state.clone();
        let observed_user = user.clone();
        let observed_workspace = workspace.clone();
        let observed_id = id.clone();
        *provider.on_revocation.lock().unwrap() = Some(Box::new(move || {
            let store = observer
                .store
                .try_lock()
                .expect("no Store lock across provider I/O");
            let row = store
                .mcp_oauth_connection(&observed_user, &observed_workspace, &observed_id)
                .unwrap()
                .unwrap();
            assert!(matches!(
                row.status,
                crate::store::mcp_oauth::ConnectionStatus::Disabled
            ));
            assert_eq!(row.revision, 2);
            let receipt = serde_json::to_value(row).unwrap();
            assert_eq!(receipt["revocation"]["state"], "unfinished");
        }));
        let (status, receipt) = team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        assert_eq!(receipt["revision"], 2);
        assert_eq!(receipt["confirmed"], if lost { 1 } else { 2 });
        assert_eq!(receipt["unconfirmed"], usize::from(lost));
        assert_eq!(
            provider.revocation_requests.load(Ordering::SeqCst),
            if lost { 3 } else { 4 }
        );
        let (status, row) = team_request(&state, "GET", &base, &cookie, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(row["status"], "disabled");
        assert_eq!(
            row["revocation"]["state"],
            if lost {
                "unconfirmed"
            } else {
                "confirmed_known_tokens"
            }
        );
        let mut restarted = state.clone();
        restarted.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
        assert_eq!(
            team_request(&restarted, "GET", &base, &cookie, json!({}))
                .await
                .1,
            row
        );
        assert_eq!(
            team_request(
                &restarted,
                "POST",
                &format!("{base}/revoke"),
                &cookie,
                json!({"expected_revision":1})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            provider.revocation_requests.load(Ordering::SeqCst),
            if lost { 3 } else { 4 }
        );
        for private in [
            "fixture-private-access",
            "fixture-private-refresh",
            "mcp-oauth:",
            "sha256",
            "binding",
        ] {
            assert!(!row.to_string().contains(private));
            assert!(!receipt.to_string().contains(private));
        }
        assert_eq!(provider.tool_calls.load(Ordering::SeqCst), 0);
        let context = AuthContext {
            user_id: user,
            workspace_id: workspace,
            connection_id: id,
            connection_revision: 2,
        };
        assert!(
            restarted
                .store
                .lock()
                .unwrap()
                .mcp_oauth_can_authorize(&context)
                .is_ok()
        );
        if lost {
            let (status, retry) = team_request(
                &restarted,
                "POST",
                &format!("{base}/revoke"),
                &cookie,
                json!({"expected_revision":2}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{retry}");
            assert_eq!(retry["revision"], 3);
            assert_eq!(retry["provider_revocation"], "confirmed_known_tokens");
            // The previous confirmed envelope is never sent again.
            assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 5);
        } else {
            assert_eq!(
                team_request(
                    &restarted,
                    "POST",
                    &format!("{base}/revoke"),
                    &cookie,
                    json!({"expected_revision":2})
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 4);
            let (status, started) = team_request(
                &restarted,
                "POST",
                &format!(
                    "/v1/web/workspaces/{}/mcp-providers/fixture/authorize",
                    context.workspace_id
                ),
                &cookie,
                json!({"connection_id":context.connection_id,"expected_revision":2}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{started}");
            assert_eq!(started["revision"], 3);
            assert_eq!(
                team_request(
                    &restarted,
                    "POST",
                    &format!("{base}/revoke"),
                    &cookie,
                    json!({"expected_revision":2})
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 4);
        }
    }
}

#[tokio::test]
async fn mcp_oauth_revoke_rechecks_browser_authority_between_refresh_and_access_revocation() {
    let (state, provider, secrets, user, workspace, cookie, id) = established(3600).await;
    let revoked = state.clone();
    let device = state
        .store
        .lock()
        .unwrap()
        .browser_session_device(cookie.split_once('=').unwrap().1)
        .unwrap()
        .unwrap()
        .id;
    *provider.on_revocation.lock().unwrap() = Some(Box::new(move || {
        revoked
            .store
            .lock()
            .unwrap()
            .revoke_device(&device)
            .unwrap();
    }));
    let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 1);
    let mut restarted = state.clone();
    restarted.mcp_oauth = Arc::new(Service::testing(secrets));
    let cookie = paired_cookie(&restarted, &user).await;
    let row = team_request(&restarted, "GET", &base, &cookie, json!({}))
        .await
        .1;
    assert_eq!(row["status"], "disabled");
    assert_eq!(row["revocation"]["state"], "unfinished");
    assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mcp_oauth_revoke_never_sends_tokens_to_changed_configuration_or_claims_lost_rotation_confirmed()
 {
    for fault in ["config", "rotation"] {
        let (mut state, provider, secrets, _, workspace, cookie, id) =
            established(if fault == "rotation" { 1 } else { 3600 }).await;
        let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
        if fault == "rotation" {
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            provider.lose_refresh.store(true, Ordering::SeqCst);
            assert_eq!(
                team_request(
                    &state,
                    "POST",
                    &format!("{base}/inspect"),
                    &cookie,
                    json!({"expected_revision":1})
                )
                .await
                .0,
                StatusCode::BAD_GATEWAY
            );
        } else {
            let mut config = (*state.config).clone();
            config.mcp_oauth[0].client_id = "changed-registration".into();
            state.config = Arc::new(config);
            *secrets.after_read.lock().unwrap() = Some(Box::new(|| {
                panic!("wrong configuration must not read a token envelope")
            }));
        }
        let (status, value) = team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["provider_revocation"], "unconfirmed");
        assert_eq!(value["confirmed"], usize::from(fault == "rotation"));
        assert_eq!(value["unconfirmed"], 1);
        assert_eq!(
            provider.revocation_requests.load(Ordering::SeqCst),
            if fault == "rotation" { 2 } else { 0 }
        );
        assert_eq!(
            provider.token_requests.load(Ordering::SeqCst),
            if fault == "rotation" { 2 } else { 1 }
        );
        let row = team_request(&state, "GET", &base, &cookie, json!({}))
            .await
            .1;
        assert_eq!(row["status"], "disabled");
        assert_eq!(row["revocation"]["state"], "unconfirmed");
        let (status, started) = team_request(
            &state,
            "POST",
            &format!("/v1/web/workspaces/{workspace}/mcp-providers/fixture/authorize"),
            &cookie,
            json!({"connection_id":id,"expected_revision":2}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{started}");
        assert_eq!(started["revision"], 3);
        let row = team_request(&state, "GET", &base, &cookie, json!({}))
            .await
            .1;
        assert_eq!(row["status"], "authorization_incomplete");
        assert_eq!(row["revision"], 3);
        assert_eq!(row["revocation"]["revision"], 2);
        assert_eq!(row["revocation"]["state"], "unconfirmed");
    }
}

#[tokio::test]
async fn mcp_oauth_cleanup_resweeps_late_writes_without_as_io_or_forgetting_references() {
    let (mut state, provider, secrets, _user, workspace, cookie, id) = established(1).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/inspect"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let reads = provider.mcp_requests.load(Ordering::SeqCst);
    let tokens = provider.token_requests.load(Ordering::SeqCst);
    let mut config = (*state.config).clone();
    config.mcp_oauth.clear();
    state.config = Arc::new(config);
    secrets.allow_delete.store(true, Ordering::SeqCst);
    let observer = state.clone();
    let late = secrets.clone();
    state
        .store
        .lock()
        .unwrap()
        .mcp_oauth_test_legacy_writer(&id);
    *secrets.after_delete.lock().unwrap() = Some(Box::new(move || {
        assert!(
            observer.store.try_lock().is_ok(),
            "Store lock held across secret delete"
        );
        let (reference, value) = late.deleted_values.lock().unwrap().last().unwrap().clone();
        // A late fenced writer can restore exactly the old envelope.
        late.values.set(&reference, &value).unwrap();
    }));
    let other = state
        .store
        .lock()
        .unwrap()
        .create_user("cleanup-other", "admin")
        .unwrap();
    let foreign = paired_cookie(&state, &other.id).await;
    let body = json!({"expected_revision":2,"expected_cleanup_revision":0});
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/cleanup"),
            &foreign,
            body.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, result) = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["revision"], 1);
    assert_eq!(result["delete_acknowledged"], 2);
    assert_eq!(result["status"], "sweep_complete");
    assert_eq!(secrets.deletes.lock().unwrap().len(), 2);
    assert_eq!(
        team_request(&state, "POST", &format!("{base}/cleanup"), &cookie, body)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let first = secrets.deletes.lock().unwrap()[0].clone();
    assert!(secrets.values.get(&first).is_ok());
    let (status, result) = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        json!({"expected_revision":2,"expected_cleanup_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["revision"], 2);
    assert_eq!(result["delete_acknowledged"], 2);
    for reference in secrets.deletes.lock().unwrap().iter() {
        assert!(secrets.values.get(reference).is_err());
    }
    assert_eq!(secrets.deletes.lock().unwrap().len(), 4);
    assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 4);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), tokens);
    assert_eq!(provider.mcp_requests.load(Ordering::SeqCst), reads);
    let row = team_request(&state, "GET", &base, &cookie, json!({}))
        .await
        .1;
    assert_eq!(row["cleanup"]["revision"], 2);
    assert_eq!(row["cleanup"]["status"], "sweep_complete");
    for private in [
        "fixture-private-access",
        "fixture-private-refresh",
        "mcp-oauth:",
        "binding",
        "sha256",
    ] {
        assert!(!row.to_string().contains(private));
    }
    assert_eq!(row["id"], id);
    assert_eq!(row["status"], "disabled");
}

#[tokio::test]
async fn mcp_oauth_cleanup_lost_delete_and_revoked_device_remain_explicitly_resweepable() {
    for logout in [false, true] {
        let (state, provider, secrets, user, workspace, cookie, id) = established(3600).await;
        let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
        assert_eq!(
            team_request(
                &state,
                "POST",
                &format!("{base}/revoke"),
                &cookie,
                json!({"expected_revision":1})
            )
            .await
            .0,
            StatusCode::OK
        );
        secrets.allow_delete.store(true, Ordering::SeqCst);
        if logout {
            let device = state
                .store
                .lock()
                .unwrap()
                .browser_session_device(cookie.split_once('=').unwrap().1)
                .unwrap()
                .unwrap()
                .id;
            let state = state.clone();
            *secrets.after_delete.lock().unwrap() = Some(Box::new(move || {
                state
                    .store
                    .try_lock()
                    .expect("no Store lock across delete")
                    .revoke_device(&device)
                    .unwrap();
            }));
        } else {
            secrets.lose_delete.store(true, Ordering::SeqCst);
        }
        let (status, result) = team_request(
            &state,
            "POST",
            &format!("{base}/cleanup"),
            &cookie,
            json!({"expected_revision":2,"expected_cleanup_revision":0}),
        )
        .await;
        assert_eq!(
            status,
            if logout {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::OK
            },
            "{result}"
        );
        if !logout {
            assert_eq!(result["status"], "unconfirmed");
            assert!(!result.to_string().contains("fixture-private-delete-error"));
        }
        let cookie = if logout {
            paired_cookie(&state, &user).await
        } else {
            cookie
        };
        let mut restarted = state.clone();
        restarted.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
        let row = team_request(&restarted, "GET", &base, &cookie, json!({}))
            .await
            .1;
        assert_eq!(
            row["cleanup"]["status"],
            if logout { "unfinished" } else { "unconfirmed" }
        );
        let (status, result) = team_request(
            &restarted,
            "POST",
            &format!("{base}/cleanup"),
            &cookie,
            json!({"expected_revision":2,"expected_cleanup_revision":1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["revision"], 2);
        assert_eq!(result["status"], "sweep_complete");
        assert_eq!(secrets.deletes.lock().unwrap().len(), 2);
        assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn mcp_oauth_cleanup_retired_values_does_not_touch_a_new_active_authorization() {
    let (state, provider, secrets, _, workspace, cookie, id) = established(3600).await;
    let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (started, flow) = begun(
        &state,
        &workspace,
        &cookie,
        json!({"connection_id":id,"expected_revision":2}),
    )
    .await;
    assert_eq!(started["revision"], 3);
    assert_eq!(
        callback(&state, &callback_url(&provider, &started), &flow)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    secrets.allow_delete.store(true, Ordering::SeqCst);
    let (status, result) = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        json!({"expected_revision":3,"expected_cleanup_revision":0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["delete_acknowledged"], 1);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/inspect"),
            &cookie,
            json!({"expected_revision":3})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(secrets.deletes.lock().unwrap().len(), 1);
    assert_eq!(secrets.writes.lock().unwrap().len(), 2);
    assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 2);
    let row = team_request(&state, "GET", &base, &cookie, json!({}))
        .await
        .1;
    assert_eq!(row["status"], "connected");
    assert_eq!(row["revision"], 3);
}

#[tokio::test]
async fn mcp_oauth_reconcile_excludes_live_worker_then_finishes_without_provider_or_secret_io() {
    let (state, provider, secrets, user, workspace, cookie, id) = established(3600).await;
    let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
    let device = state
        .store
        .lock()
        .unwrap()
        .browser_session_device(cookie.split_once('=').unwrap().1)
        .unwrap()
        .unwrap()
        .id;
    let active = state.clone();
    let active_cookie = cookie.clone();
    let active_base = base.clone();
    let runtime = tokio::runtime::Handle::current();
    *provider.on_revocation.lock().unwrap() = Some(Box::new(move || {
        // This runs inside the controlled AS's thread while the real blocking
        // revocation worker is waiting on HTTP. A replacement Service is not
        // evidence of worker death; the database lease must still exclude it.
        let mut replacement = active.clone();
        replacement.mcp_oauth = Arc::new(Service::testing(Arc::new(Secrets::default())));
        let result = runtime.block_on(team_request(
            &replacement,
            "POST",
            &format!("{active_base}/reconcile-revocation"),
            &active_cookie,
            json!({"expected_revision":2}),
        ));
        assert_eq!(result.0, StatusCode::CONFLICT);
        active.store.lock().unwrap().revoke_device(&device).unwrap();
    }));
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut restarted = state.clone();
    restarted.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
    let mut config = (*restarted.config).clone();
    config.mcp_oauth.clear();
    restarted.config = Arc::new(config);
    let fresh = paired_cookie(&restarted, &user).await;
    let foreign = restarted
        .store
        .lock()
        .unwrap()
        .create_user("reconcile-other", "admin")
        .unwrap();
    let foreign_cookie = paired_cookie(&restarted, &foreign.id).await;
    let path = format!("{base}/reconcile-revocation");
    assert_eq!(
        team_request(
            &restarted,
            "POST",
            &path,
            &cookie,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        team_request(
            &restarted,
            "POST",
            &path,
            &foreign_cookie,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        team_request(
            &restarted,
            "POST",
            &path,
            &fresh,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    *secrets.after_read.lock().unwrap() =
        Some(Box::new(|| panic!("reconcile must never read a secret")));
    let result = team_request(
        &restarted,
        "POST",
        &path,
        &fresh,
        json!({"expected_revision":2}),
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["revision"], 2);
    assert_eq!(result.1["provider_revocation"], "unconfirmed");
    assert_eq!(result.1["confirmed"], 0);
    assert_eq!(result.1["unconfirmed"], 1);
    assert!(secrets.after_read.lock().unwrap().is_some());
    assert_eq!(provider.revocation_requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        team_request(
            &restarted,
            "POST",
            &path,
            &fresh,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let row = team_request(&restarted, "GET", &base, &fresh, Value::Null)
        .await
        .1;
    assert_eq!(row["revocation"]["state"], "unconfirmed");
    assert_eq!(row["status"], "disabled");
    assert!(!row.to_string().contains("mcp-oauth:"));
    let context = AuthContext {
        user_id: user,
        workspace_id: workspace,
        connection_id: id,
        connection_revision: 2,
    };
    assert!(
        restarted
            .store
            .lock()
            .unwrap()
            .mcp_oauth_can_authorize(&context)
            .is_ok()
    );
}

#[tokio::test]
async fn mcp_oauth_writer_lease_survives_lost_callback_waiter_and_disconnect_fences_commit() {
    let (state, provider, secrets, _user, workspace, cookie) = setup().await;
    let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
    let id = started["connection_id"].as_str().unwrap().to_owned();
    let path = callback_url(&provider, &started);
    let entered = Arc::new(tokio::sync::Notify::new());
    let noticed = entered.clone();
    let (release, blocked) = std::sync::mpsc::channel();
    *secrets.after_write.lock().unwrap() = Some(Box::new(move || {
        noticed.notify_one();
        blocked
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    }));
    let writing = state.clone();
    let waiter = tokio::spawn(async move { callback(&writing, &path, &flow).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert!(
        state.store.try_lock().is_ok(),
        "no Store mutex across private I/O"
    );
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_writer_lease()
            .is_err()
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_writer_lease()
            .is_err()
    );
    let result = team_request(
        &state,
        "POST",
        &format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/disconnect"),
        &cookie,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(lease) = state.store.lock().unwrap().mcp_oauth_writer_lease() {
                drop(lease);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let row = team_request(
        &state,
        "GET",
        &format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}"),
        &cookie,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(row["revision"], 2);
    assert_eq!(row["status"], "disabled");
    assert_eq!(secrets.writes.lock().unwrap().len(), 1);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mcp_oauth_writer_admission_waits_for_a_short_reader_without_replaying_refresh() {
    let (state, provider, secrets, _user, workspace, cookie, id) = established(3600).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let noticed = entered.clone();
    let (release, blocked) = std::sync::mpsc::channel();
    *secrets.after_read.lock().unwrap() = Some(Box::new(move || {
        noticed.notify_one();
        blocked
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    }));
    let path = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect");
    let first_state = state.clone();
    let first_path = path.clone();
    let first_cookie = cookie.clone();
    let first = tokio::spawn(async move {
        team_request(
            &first_state,
            "POST",
            &first_path,
            &first_cookie,
            json!({"expected_revision":1}),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let second = tokio::spawn(async move {
        team_request(
            &state,
            "POST",
            &path,
            &cookie,
            json!({"expected_revision":1}),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !second.is_finished(),
        "a short lease conflict waits before credential I/O"
    );
    release.send(()).unwrap();
    assert_eq!(first.await.unwrap().0, StatusCode::OK);
    assert_eq!(second.await.unwrap().0, StatusCode::OK);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(secrets.writes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mcp_oauth_callback_waiting_for_writer_rechecks_revision_before_exchanging_code() {
    let (state, provider, _secrets, _user, workspace, cookie) = setup().await;
    let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
    let id = started["connection_id"].as_str().unwrap().to_owned();
    let path = callback_url(&provider, &started);
    let writer = state
        .store
        .lock()
        .unwrap()
        .mcp_oauth_writer_lease()
        .unwrap();
    let queued = state.clone();
    let waiting = tokio::spawn(async move { callback(&queued, &path, &flow).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.mcp_oauth.slots.available_permits() != 1 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!waiting.is_finished());
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/disconnect"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(writer);
    assert_eq!(waiting.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        provider.token_requests.load(Ordering::SeqCst),
        0,
        "a canceled authorization must not exchange after waiting for admission"
    );
}

#[tokio::test]
async fn mcp_oauth_write_completion_survives_disconnect_or_device_revocation_but_never_activates() {
    for fence in ["disconnect", "device"] {
        let (state, provider, secrets, user, workspace, cookie) = setup().await;
        let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
        let path = callback_url(&provider, &started);
        let id = started["connection_id"].as_str().unwrap().to_owned();
        let device = state
            .store
            .lock()
            .unwrap()
            .browser_session_device(cookie.split_once('=').unwrap().1)
            .unwrap()
            .unwrap()
            .id;
        let context = AuthContext {
            user_id: user.clone(),
            workspace_id: workspace.clone(),
            connection_id: id.clone(),
            connection_revision: 1,
        };
        let revoked = state.clone();
        let revoking = context.clone();
        *secrets.after_write.lock().unwrap() = Some(Box::new(move || {
            let store = revoked.store.lock().unwrap();
            assert_eq!(
                store.mcp_oauth_test_write_counts(&revoking.connection_id),
                (1, 0)
            );
            if fence == "disconnect" {
                store.disable(&revoking).unwrap();
            } else {
                store.revoke_device(&device).unwrap();
            }
        }));
        assert_eq!(
            callback(&state, &path, &flow).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            state.store.lock().unwrap().mcp_oauth_test_write_counts(&id),
            (1, 1)
        );
        let row = state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connection(&user, &workspace, &id)
            .unwrap()
            .unwrap();
        let public = serde_json::to_value(row).unwrap();
        assert_ne!(public["status"], "connected");
        assert!(!public.to_string().contains("write_operation_id"));
        assert!(!public.to_string().contains("mcp-oauth:"));
        let mut restarted = state.clone();
        restarted.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
        assert_eq!(
            restarted
                .store
                .lock()
                .unwrap()
                .mcp_oauth_test_write_counts(&id),
            (1, 1)
        );
        assert_eq!(secrets.writes.lock().unwrap().len(), 1);
        assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn mcp_oauth_write_error_after_effect_stays_claimed_unknown_without_implicit_retry() {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
    let path = callback_url(&provider, &started);
    let id = started["connection_id"].as_str().unwrap().to_owned();
    secrets.lose_write.store(true, Ordering::SeqCst);
    assert_eq!(
        callback(&state, &path, &flow).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        state.store.lock().unwrap().mcp_oauth_test_write_counts(&id),
        (1, 0)
    );
    assert_eq!(secrets.writes.lock().unwrap().len(), 1);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
    let mut restarted = state.clone();
    restarted.mcp_oauth = Arc::new(Service::testing(secrets.clone()));
    let row = restarted
        .store
        .lock()
        .unwrap()
        .mcp_oauth_connection(&user, &workspace, &id)
        .unwrap()
        .unwrap();
    assert_ne!(serde_json::to_value(row).unwrap()["status"], "connected");
    assert_eq!(
        restarted
            .store
            .lock()
            .unwrap()
            .mcp_oauth_test_write_counts(&id),
        (1, 0)
    );
    assert_eq!(
        callback(&restarted, &path, &flow).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(secrets.writes.lock().unwrap().len(), 1);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mcp_oauth_retention_rest_reclaims_slots_and_historical_sweeps_protect_new_authorization() {
    let (state, provider, secrets, _user, workspace, cookie, id) = established(3600).await;
    let base = format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}");
    assert_eq!(
        team_request(&state, "GET", &base, &cookie, Value::Null)
            .await
            .1["storage"]["available_slots"],
        255
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    secrets.allow_delete.store(true, Ordering::SeqCst);
    let first = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        json!({"expected_revision":2,"expected_cleanup_revision":0}),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1["reclaimed"], 1);
    assert_eq!(first.1["pruned"], 0);
    let row = team_request(&state, "GET", &base, &cookie, Value::Null)
        .await
        .1;
    assert_eq!(row["storage"]["available_slots"], 256);
    assert_eq!(row["storage"]["retained_history"], 1);
    assert_eq!(row["storage"]["uncertain_history"], 0);
    let (started, flow) = begun(
        &state,
        &workspace,
        &cookie,
        json!({"connection_id":id,"expected_revision":2}),
    )
    .await;
    assert_eq!(
        callback(&state, &callback_url(&provider, &started), &flow)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let current = started["revision"].as_u64().unwrap();
    let again = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        json!({"expected_revision":current,"expected_cleanup_revision":1}),
    )
    .await;
    assert_eq!(again.0, StatusCode::OK);
    assert_eq!(again.1["reclaimed"], 0);
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/inspect"),
            &cookie,
            json!({"expected_revision":current})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        team_request(
            &state,
            "POST",
            &format!("{base}/revoke"),
            &cookie,
            json!({"expected_revision":current})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        provider.revocation_requests.load(Ordering::SeqCst),
        4,
        "only new tokens are sent in the second AS operation"
    );
    let last = team_request(
        &state,
        "POST",
        &format!("{base}/cleanup"),
        &cookie,
        json!({"expected_revision":current+1,"expected_cleanup_revision":2}),
    )
    .await;
    assert_eq!(last.0, StatusCode::OK);
    assert_eq!(last.1["reclaimed"], 1);
    let row = team_request(&state, "GET", &base, &cookie, Value::Null)
        .await
        .1;
    assert_eq!(row["storage"]["available_slots"], 256);
    assert_eq!(row["storage"]["retained_history"], 2);
    assert_eq!(row["storage"]["uncertain_history"], 0);
    assert_eq!(secrets.writes.lock().unwrap().len(), 2);
    assert_eq!(secrets.deletes.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn mcp_oauth_positive_as_transport_closes_on_logout_without_install_or_refresh_replay() {
    for refresh in [false, true] {
        let (state, provider, secrets, user, workspace, cookie) = setup().await;
        provider.expires_in.store(1, Ordering::SeqCst);
        let (started, flow) = begun(&state, &workspace, &cookie, json!({})).await;
        let callback_path = callback_url(&provider, &started);
        let id = started["connection_id"].as_str().unwrap();
        if refresh {
            assert_eq!(
                callback(&state, &callback_path, &flow).await.status(),
                StatusCode::SEE_OTHER
            );
            tokio::time::sleep(Duration::from_millis(1100)).await;
        }
        let store = state.store.clone();
        let token = cookie.split_once('=').unwrap().1.to_owned();
        *provider.on_token.lock().unwrap() = Some(Box::new(move || {
            store
                .lock()
                .unwrap()
                .remove_browser_session(&token)
                .unwrap();
        }));
        provider.stall_token.store(true, Ordering::SeqCst);
        let response = if refresh {
            team_request(
                &state,
                "POST",
                &format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect"),
                &cookie,
                json!({"expected_revision":1}),
            )
            .await
            .0
        } else {
            callback(&state, &callback_path, &flow).await.status()
        };
        assert_eq!(
            response,
            if refresh {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while provider.closed_tokens.load(Ordering::SeqCst) == 0
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // The peer requires closure within one second without releasing any
        // response; the normal ten-second client timeout cannot pass this.
        assert_eq!(provider.closed_tokens.load(Ordering::SeqCst), 1);
        assert_eq!(secrets.writes.lock().unwrap().len(), usize::from(refresh));
        assert_eq!(
            provider.token_requests.load(Ordering::SeqCst),
            1 + usize::from(refresh)
        );
        assert_eq!(provider.mcp_requests.load(Ordering::SeqCst), 0);
        let replacement = paired_cookie(&state, &user).await;
        if refresh {
            assert_eq!(
                team_request(
                    &state,
                    "POST",
                    &format!("/v1/web/workspaces/{workspace}/mcp-connections/{id}/inspect"),
                    &replacement,
                    json!({"expected_revision":1})
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            let row = state
                .store
                .lock()
                .unwrap()
                .mcp_oauth_connection(&user, &workspace, id)
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(row).unwrap()["status"],
                "reauthorization_required"
            );
        } else {
            assert_eq!(
                callback(&state, &callback_path, &flow).await.status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            provider.token_requests.load(Ordering::SeqCst),
            1 + usize::from(refresh)
        );
    }
}

#[tokio::test]
async fn mcp_oauth_start_closes_discovery_after_logout_before_preparing_any_connection() {
    let (state, provider, secrets, user, workspace, cookie) = setup().await;
    let store = state.store.clone();
    let token = cookie.split_once('=').unwrap().1.to_owned();
    *provider.on_discovery.lock().unwrap() = Some(Box::new(move || {
        store
            .lock()
            .unwrap()
            .remove_browser_session(&token)
            .unwrap();
    }));
    provider.stall_discovery.store(true, Ordering::SeqCst);
    let response = start(&state, &workspace, &cookie, json!({})).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while provider.closed_discovery.load(Ordering::SeqCst) == 0
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(provider.closed_discovery.load(Ordering::SeqCst), 1);
    assert_eq!(provider.token_requests.load(Ordering::SeqCst), 0);
    assert!(secrets.writes.lock().unwrap().is_empty());
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .mcp_oauth_connections(&user, &workspace)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn mcp_oauth_pending_discovery_does_not_retain_its_own_service() {
    let (state, _provider, _secrets, _user, workspace, cookie) = setup().await;
    let service = Arc::downgrade(&state.mcp_oauth);
    let _pending = begun(&state, &workspace, &cookie, json!({})).await;
    drop(state);
    assert!(
        service.upgrade().is_none(),
        "Pending retained the application through its discovery guard"
    );
}
