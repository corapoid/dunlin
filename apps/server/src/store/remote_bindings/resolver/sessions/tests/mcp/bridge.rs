//! Actual host producer/broker/checked credentials/HTTP. Native session/turn
//! receipts and the source metadata responder remain explicit controlled fixtures.
use super::*;
use crate::remote_sessions::{Service, mcp::FixtureActor};
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
use wire::broker::{BrokerOperation, BrokerRequest, BrokerScope};
#[derive(Default)]
struct Secrets {
    values: MemoryStore,
    after_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> std::result::Result<zeroize::Zeroizing<String>, String> {
        let value = self.values.get(id);
        let hook = self.after_read.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        value
    }
    fn set(&self, id: &str, value: &str) -> std::result::Result<(), String> {
        self.values.set(id, value)
    }
    fn delete(&self, _: &str) -> std::result::Result<(), String> {
        panic!("no unfenced GC")
    }
    fn get_policy(&self, _: &str) -> std::result::Result<zeroize::Zeroizing<String>, String> {
        panic!("OAuth is private")
    }
    fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
        panic!("OAuth is private")
    }
    fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
        panic!("OAuth is private")
    }
}

struct Provider {
    secrets: Arc<Secrets>,
    origin: String,
    calls: Arc<AtomicUsize>,
    reply_bytes: Arc<AtomicUsize>,
    lose: Arc<AtomicBool>,
    hold: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Provider {
    fn new() -> Self {
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
                        line.eq_ignore_ascii_case("Authorization: Bearer bridge-private-access")
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
                                    json!({"query":"approved fixture"})
                                );
                                count.fetch_add(1, Ordering::Release);
                                let (lock, changed) = held.as_ref();
                                let mut wait = lock.lock().unwrap();
                                while *wait && !stopped.load(Ordering::Acquire) {
                                    wait = changed.wait(wait).unwrap();
                                }
                                if lost.load(Ordering::Acquire) {
                                    continue;
                                }
                                json!({"resultType":"complete","content":[{"type":"text","text":if result_size.load(Ordering::Acquire)==0 {"private bridge result".into()} else {"x".repeat(result_size.load(Ordering::Acquire))}}],"ttlMs":0,"cacheScope":"private"})
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
                        json!({"access_token":"bridge-private-access","refresh_token":"bridge-private-refresh","token_type":"Bearer","expires_in":3600,"scope":"files:read"}),
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
            origin,
            calls,
            reply_bytes,
            lose,
            hold,
            stop,
            thread: Some(thread),
        }
    }
    fn release(&self) {
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
fn with_credentials(provider: &Provider) -> McpFixture {
    let mut fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let mut config = (*fixture.state.config).clone();
    config.mcp_oauth[0].endpoint = format!("{}/mcp", provider.origin);
    config.mcp_oauth[0].issuer = format!("{}/issuer", provider.origin);
    config.mcp_oauth[0].scopes = vec!["files:read".into()];
    fixture.state.config = Arc::new(config);
    let secrets = provider.secrets.clone();
    fixture.state.mcp_oauth = Arc::new(crate::mcp_oauth::Service::testing(secrets.clone()));
    fixture.context.connection_id = uuid::Uuid::now_v7().to_string();
    let provider = &fixture.state.config.mcp_oauth[0];
    let flow = provider.flow(secrets.as_ref()).unwrap();
    let authorization = flow
        .begin(fixture.context.clone(), vec!["files:read".into()])
        .unwrap();
    let authorization = url::Url::parse(&authorization.url).unwrap();
    let state = authorization
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    let mut callback = url::Url::parse(&provider.redirect_uri).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", &state)
        .append_pair("code", "bridge-code")
        .append_pair("iss", &provider.issuer);
    let token = flow.finish(&fixture.context, callback.as_str()).unwrap();
    let store = fixture.state.store.lock().unwrap();
    store
        .mcp_oauth_prepare(
            &fixture.context,
            &flow.credential_binding(),
            "work",
            None,
            &|_, _| Ok(()),
        )
        .unwrap();
    flow.install_durable(&fixture.context, token, &*store, |id, value| {
        secrets
            .set(id, value)
            .map_err(|_| dunlin_mcp_client::oauth::persistence::PersistenceError::Storage)
    })
    .unwrap();
    let mut definition = store
        .agent_version(
            &fixture.data.owner,
            &fixture.data.workspace,
            &fixture.data.project,
            &fixture.data.request.profile.agent_id,
            fixture.data.request.profile.version,
        )
        .unwrap()
        .unwrap()
        .definition;
    let tool: dunlin_mcp_client::Tool =
        serde_json::from_value(json!({"name":"lookup","inputSchema":{"type":"object"}})).unwrap();
    definition.mcp[0].definition_digest = tool.definition_digest().unwrap();
    let draft = store
        .create_agent(
            &fixture.data.owner,
            &fixture.data.workspace,
            &fixture.data.project,
            definition,
        )
        .unwrap();
    let profile = store
        .publish_agent(
            &fixture.data.owner,
            &fixture.data.workspace,
            &fixture.data.project,
            &draft.id,
            1,
        )
        .unwrap();
    fixture.data.request.profile = AgentProfileRef {
        agent_id: profile.agent_id,
        version: profile.version,
        digest: profile.digest,
    };
    drop(store);
    fixture
}
async fn setup(
    provider: &Provider,
) -> (
    McpFixture,
    Arc<Service>,
    FixtureActor,
    String,
    Lease,
    Box<dyn Send>,
) {
    setup_limit(provider, 65536).await
}
async fn setup_limit(
    provider: &Provider,
    maximum: u32,
) -> (
    McpFixture,
    Arc<Service>,
    FixtureActor,
    String,
    Lease,
    Box<dyn Send>,
) {
    let fixture = with_credentials(provider);
    let (fixture, service, actor, id, lease) =
        super::remote::live_from_limit(fixture, maximum).await;
    let (metadata, _) = fixture
        .state
        .worker_channels
        .register_export_fixture(
            &fixture.state,
            &fixture.data.identity,
            &fixture.data.channel,
            Arc::new(Mutex::new(fixture.data.descriptor.clone())),
        )
        .unwrap();
    actor.install(&fixture.state).await.unwrap();
    (fixture, service, actor, id, lease, metadata)
}
fn session_receipt(fixture: &McpFixture, id: &str) -> wire::SessionReceipt {
    let store = fixture.state.store.lock().unwrap();
    let snapshot = store.remote_session_snapshot(id).unwrap();
    wire::SessionReceipt {
        identity: snapshot.open.identity.clone(),
        fence: wire::SessionFence {
            channel: fixture.transport.fence().clone(),
            controller_session_id: id.into(),
            worker_session_id: "native-bridge-session".into(),
            session_epoch: 1,
        },
        budgets: snapshot.open.budgets.clone(),
    }
}
fn activate(
    fixture: &McpFixture,
    actor: &FixtureActor,
    id: &str,
    lease: &Lease,
) -> wire::TurnReceipt {
    stage_turn(fixture, actor, id, lease, true)
}
fn stage_turn(
    fixture: &McpFixture,
    actor: &FixtureActor,
    id: &str,
    lease: &Lease,
    native_accepted: bool,
) -> wire::TurnReceipt {
    use crate::interactive::mcp::authority::Authority;
    let receipt = session_receipt(fixture, id);
    let cut = actor.authority.sql(&fixture.state).unwrap();
    let store = fixture.state.store.lock().unwrap();
    let context = cut.context(&store, false).unwrap();
    let snapshot = store.remote_session_snapshot(id).unwrap();
    store.remote_binding_transaction(false,|store| {
        store.interactive_journal().harness_ready_in_transaction(&context,id,lease,dunlin_session_runtime::HarnessInfo {profile:Some(snapshot.open.profile.clone()),remote_session_id:receipt.fence.worker_session_id.clone(),binding_revision:fixture.data.request.binding.revision,model:snapshot.open.definition.model.clone(),mode:snapshot.open.definition.mode.clone(),max_seconds:snapshot.open.budgets.max_seconds,max_output_bytes:snapshot.open.budgets.max_output_bytes}).map_err(journal_error)?;
        store.connection.execute("UPDATE controller_remote_sessions SET phase='active',dispatched=1,receipt=?2 WHERE session=?1",params![id,serde_json::to_string(&receipt)?])?; Ok::<_,StoreError>(())
    }).unwrap();
    store
        .interactive_journal()
        .enqueue(&context, id, "bridge-turn", "perform lookup")
        .unwrap();
    let (command,accepted)=store.remote_binding_transaction(false,|store| {
        let turn=store.interactive_journal().claim_next_in_transaction(&context,id,lease).map_err(journal_error)?.unwrap();
        let mut command=wire::TurnCommand {session:receipt.fence.clone(),identity:wire::TurnIdentity {command_id:"bridge-command".into(),turn_id:turn.id.clone(),fingerprint:"0".repeat(64)},turn_index:1,input:turn.prompt,budgets:dunlin_relay_protocol::worker::ExecutionBudgets {max_seconds:10,max_output_bytes:1024,lease_duration_ms:10000}};
        command.identity.fingerprint=valid(wire::session_turn_fingerprint(&command))?;
        let accepted=wire::TurnReceipt {identity:command.identity.clone(),fence:wire::TurnFence {session:receipt.fence,turn_id:turn.id.clone(),turn_index:1,turn_epoch:1},budgets:command.budgets.clone()};
        if native_accepted { store.interactive_journal().transition_timed_in_transaction(&context,id,lease,&turn.id,dunlin_session_runtime::TurnPhase::Running,None).map_err(journal_error)?; }
        store.connection.execute("INSERT INTO controller_remote_session_turns(session,turn,command_id,turn_index,command,receipt,phase) VALUES(?1,?2,'bridge-command',1,?3,?4,?5)",params![id,turn.id,serde_json::to_string(&command)?,if native_accepted { Some(serde_json::to_string(&accepted)?) } else { None },if native_accepted {"accepted"} else {"sent"}])?;
        Ok::<_,StoreError>((command,accepted))
    }).unwrap();
    actor.begin_turn(command, tokio::time::Instant::now());
    accepted
}
async fn catalog(fixture: &McpFixture, receipt: &wire::SessionReceipt) -> serde_json::Value {
    let payload = fixture
        .transport
        .broker_request_fixture(BrokerRequest {
            scope: BrokerScope::Catalog {
                session: receipt.fence.clone(),
            },
            request_id: "catalog".into(),
            operation: BrokerOperation::ToolsList { cursor: None },
        })
        .await
        .unwrap();
    assert!(
        !payload.contains("bridge-private") && !payload.contains(&fixture.context.connection_id)
    );
    serde_json::from_str(&payload).unwrap()
}
async fn approve(fixture: &McpFixture, id: &str, approved: bool) {
    approve_before(fixture, id, approved, || {}).await
}
async fn approve_before(fixture: &McpFixture, id: &str, approved: bool, before: impl FnOnce()) {
    let mut before = Some(before);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let found = {
            let store = fixture.state.store.lock().unwrap();
            let context = store
                .team_project_context(
                    &fixture.data.workspace,
                    &fixture.data.project,
                    &fixture.data.owner,
                )
                .unwrap();
            let chat = serde_json::to_value(
                store
                    .interactive_journal()
                    .projected_chat(&context, id)
                    .unwrap(),
            )
            .unwrap();
            let tool = chat["activeTurn"]["responseParts"]
                .as_array()
                .and_then(|parts| {
                    parts.iter().find(|part| {
                        part["kind"] == "toolCall"
                            && part["toolCall"]["status"] == "pending-confirmation"
                    })
                });
            tool.map(|part| {
                let tool=&part["toolCall"]; let option=tool["options"].as_array().unwrap().iter().find(|option|option["kind"]==if approved {"approve"}else{"deny"}).unwrap();
                let action=serde_json::from_value::<ahp_types::commands::DispatchActionParams>(json!({"channel":format!("ahp-chat:/{id}"),"clientSeq":1,"action":{"type":"chat/toolCallConfirmed","turnId":chat["activeTurn"]["id"],"toolCallId":tool["toolCallId"],"approved":approved,"selectedOptionId":option["id"]}})).unwrap();
                before.take().unwrap()();
                store.interactive_journal().dispatch_action(&context,id,&fixture.data.device,"bridge-choice",&action,dunlin_session_runtime::DispatchAdmission::default()).unwrap();
            }).is_some()
        };
        if found {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing SDK consent"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn private_host_bridge_uses_sdk_consent_and_never_replays_after_denial_disconnect_or_lost_reply()
 {
    for (allowed, disconnect, lose) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let provider = Provider::new();
        provider.lose.store(lose, Ordering::Release);
        let (fixture, service, actor, id, lease, _metadata) = setup(&provider).await;
        let accepted = activate(&fixture, &actor, &id, &lease);
        let receipt = session_receipt(&fixture, &id);
        let tools = catalog(&fixture, &receipt).await;
        let name = tools["tools"][0]["name"].as_str().unwrap().to_owned();
        let request = BrokerRequest {
            scope: BrokerScope::Turn {
                turn: accepted.fence,
            },
            request_id: "paid-call".into(),
            operation: BrokerOperation::ToolsCall {
                name,
                arguments_json: json!({"query":"approved fixture"}).to_string(),
            },
        };
        let transport = fixture.transport.clone();
        let work = request.clone();
        let task = tokio::spawn(async move { transport.broker_request_fixture(work).await });
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
        approve_before(&fixture, &id, allowed, || {
            if disconnect {
                let store = fixture.state.store.clone();
                let context = fixture.context.clone();
                *provider.secrets.after_read.lock().unwrap() = Some(Box::new(move || {
                    store.lock().unwrap().disable(&context).unwrap();
                }));
            }
        })
        .await;
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            provider.calls.load(Ordering::Acquire),
            usize::from(allowed && !disconnect)
        );
        if disconnect {
            assert_eq!(
                fixture
                    .state
                    .store
                    .lock()
                    .unwrap()
                    .mcp_oauth_connection(
                        &fixture.data.owner,
                        &fixture.data.workspace,
                        &fixture.context.connection_id
                    )
                    .unwrap()
                    .unwrap()
                    .revision,
                2
            );
        }
        if allowed && !disconnect && !lose {
            assert!(result.as_ref().unwrap().contains("private bridge result"));
        } else if !allowed {
            assert!(result.unwrap().contains("denied"));
        } else {
            assert!(result.is_err());
        }
        // Even a fresh controlled exchange cannot dispatch the SDK operation
        // twice. A live completed request may return its existing cached result.
        let _ = fixture.transport.broker_request_fixture(request).await;
        assert_eq!(
            provider.calls.load(Ordering::Acquire),
            usize::from(allowed && !disconnect)
        );
        drop(actor);
        service.shutdown().await;
    }
}

#[tokio::test]
async fn host_bridge_waits_for_sent_native_receipt_without_dispatching_a_tool() {
    let provider = Provider::new();
    let (fixture, service, actor, id, _, _metadata) = setup(&provider).await;
    let receipt = session_receipt(&fixture, &id);
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .connection
        .execute(
            "UPDATE controller_remote_sessions SET phase='opening',dispatched=1 WHERE session=?1",
            [&id],
        )
        .unwrap();
    let transport = fixture.transport.clone();
    let request = BrokerRequest {
        scope: BrokerScope::Catalog {
            session: receipt.fence.clone(),
        },
        request_id: "before-native-receipt".into(),
        operation: BrokerOperation::ToolsList { cursor: None },
    };
    let task = tokio::spawn(async move { transport.broker_request_fixture(request).await });
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!task.is_finished());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .connection
        .execute(
            "UPDATE controller_remote_sessions SET receipt=?2 WHERE session=?1",
            params![id, serde_json::to_string(&receipt).unwrap()],
        )
        .unwrap();
    actor.changed();
    let payload = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(payload.contains("tools"));
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn registered_host_bridge_does_not_keep_service_actor_or_runtime_capacity_alive() {
    let provider = Provider::new();
    let (fixture, service, actor, _, _, _metadata) = setup(&provider).await;
    fixture
        .state
        .remote_sessions
        .set(service.clone())
        .ok()
        .unwrap();
    let weak = Arc::downgrade(&service);
    let broker = actor.weak_broker();
    let resources = fixture.state.mcp_oauth.clone();
    assert_eq!(resources.runtime_slots.available_permits(), 7);
    actor.abandon();
    drop(fixture);
    drop(service);
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some()
            || broker.upgrade().is_some()
            || resources.runtime_slots.available_permits() != 8
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resources.runtime_slots.available_permits(), 8);
}

#[tokio::test]
async fn cancelled_bridge_waiter_keeps_real_http_work_and_runtime_capacity_until_completion() {
    let provider = Provider::new();
    let (fixture, service, actor, id, lease, _metadata) = setup(&provider).await;
    let accepted = activate(&fixture, &actor, &id, &lease);
    let tools = catalog(&fixture, &session_receipt(&fixture, &id)).await;
    *provider.hold.0.lock().unwrap() = true;
    let request = BrokerRequest {
        scope: BrokerScope::Turn {
            turn: accepted.fence,
        },
        request_id: "detached-call".into(),
        operation: BrokerOperation::ToolsCall {
            name: tools["tools"][0]["name"].as_str().unwrap().into(),
            arguments_json: json!({"query":"approved fixture"}).to_string(),
        },
    };
    let transport = fixture.transport.clone();
    let task = tokio::spawn(async move { transport.broker_request_fixture(request).await });
    approve(&fixture, &id, true).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while provider.calls.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let broker = actor.weak_broker();
    let resources = fixture.state.mcp_oauth.clone();
    task.abort();
    let _ = task.await;
    drop(actor);
    assert!(broker.upgrade().is_some());
    assert_eq!(resources.runtime_slots.available_permits(), 7);
    assert_eq!(resources.slots.available_permits(), 1);
    provider.release();
    tokio::time::timeout(Duration::from_secs(3), async {
        while broker.upgrade().is_some()
            || resources.runtime_slots.available_permits() != 8
            || resources.slots.available_permits() != 2
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resources.runtime_slots.available_permits(), 8);
    assert_eq!(resources.slots.available_permits(), 2);
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    service.shutdown().await;
}

#[tokio::test]
async fn native_turn_receipt_must_arrive_before_any_sdk_consent_or_paid_http() {
    use crate::interactive::mcp::authority::Authority;
    let provider = Provider::new();
    let (fixture, service, actor, id, lease, _metadata) = setup(&provider).await;
    let accepted = stage_turn(&fixture, &actor, &id, &lease, false);
    let tools = catalog(&fixture, &session_receipt(&fixture, &id)).await;
    let request = BrokerRequest {
        scope: BrokerScope::Turn {
            turn: accepted.fence.clone(),
        },
        request_id: "before-turn-receipt".into(),
        operation: BrokerOperation::ToolsCall {
            name: tools["tools"][0]["name"].as_str().unwrap().into(),
            arguments_json: json!({"query":"approved fixture"}).to_string(),
        },
    };
    let transport = fixture.transport.clone();
    let task = tokio::spawn(async move { transport.broker_request_fixture(request).await });
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!task.is_finished());
    {
        let cut = actor.authority.sql(&fixture.state).unwrap();
        let store = fixture.state.store.lock().unwrap();
        let context = cut.context(&store, false).unwrap();
        let chat = serde_json::to_value(
            store
                .interactive_journal()
                .projected_chat(&context, &id)
                .unwrap(),
        )
        .unwrap();
        assert!(!chat.to_string().contains("pending-confirmation"));
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
        // Controlled native receipt import; a full WSS/native bridge is separate.
        store.remote_binding_transaction(false, |store| {
            store.interactive_journal().transition_timed_in_transaction(&context, &id, &lease, &accepted.identity.turn_id, dunlin_session_runtime::TurnPhase::Running, None).map_err(journal_error)?;
            store.connection.execute("UPDATE controller_remote_session_turns SET phase='accepted',receipt=?3 WHERE session=?1 AND turn=?2", params![id,accepted.identity.turn_id,serde_json::to_string(&accepted)?])?;
            Ok::<_, StoreError>(())
        }).unwrap();
    }
    actor.changed();
    approve(&fixture, &id, true).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .contains("private bridge result")
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn reply_above_session_limit_is_unknown_after_one_dispatch_without_partial_result() {
    let provider = Provider::new();
    provider.reply_bytes.store(40000, Ordering::Release);
    let (fixture, service, actor, id, lease, _metadata) = setup_limit(&provider, 32768).await;
    let accepted = activate(&fixture, &actor, &id, &lease);
    let tools = catalog(&fixture, &session_receipt(&fixture, &id)).await;
    let request = BrokerRequest {
        scope: BrokerScope::Turn {
            turn: accepted.fence,
        },
        request_id: "oversized-result".into(),
        operation: BrokerOperation::ToolsCall {
            name: tools["tools"][0]["name"].as_str().unwrap().into(),
            arguments_json: json!({"query":"approved fixture"}).to_string(),
        },
    };
    let transport = fixture.transport.clone();
    let task = tokio::spawn(async move { transport.broker_request_fixture(request).await });
    approve(&fixture, &id, true).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(
                &fixture.data.workspace,
                &fixture.data.project,
                &fixture.data.owner,
            )
            .unwrap();
        let events = store
            .interactive_journal()
            .replay(&context, &id, 0)
            .unwrap();
        assert!(
            events.iter().any(
                |event| event.payload["actions"]
                    .as_array()
                    .is_some_and(|actions| actions
                        .iter()
                        .any(|action| action["_meta"]["dunlin.mcpOutcome"] == "complete"))
            ),
            "SDK completion fits its own limit; the smaller session reply limit fences delivery"
        );
    }
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn invalid_alias_is_rejected_before_consent_and_does_not_stop_a_valid_followup() {
    let provider = Provider::new();
    let (fixture, service, actor, id, lease, _metadata) = setup(&provider).await;
    let accepted = activate(&fixture, &actor, &id, &lease);
    let tools = catalog(&fixture, &session_receipt(&fixture, &id)).await;
    let request = BrokerRequest {
        scope: BrokerScope::Turn {
            turn: accepted.fence,
        },
        request_id: "invalid-alias".into(),
        operation: BrokerOperation::ToolsCall {
            name: format!("mcp_{}", "0".repeat(32)),
            arguments_json: json!({"query":"approved fixture"}).to_string(),
        },
    };
    assert_eq!(
        fixture
            .transport
            .broker_request_fixture(request.clone())
            .await
            .unwrap_err(),
        wire::broker::BrokerError::InvalidRequest
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    assert!(actor.weak_broker().upgrade().unwrap().is_live());
    let mut valid = request;
    valid.request_id = "valid-followup".into();
    valid.operation = BrokerOperation::ToolsCall {
        name: tools["tools"][0]["name"].as_str().unwrap().into(),
        arguments_json: json!({"query":"approved fixture"}).to_string(),
    };
    let transport = fixture.transport.clone();
    let task = tokio::spawn(async move { transport.broker_request_fixture(valid).await });
    approve(&fixture, &id, true).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .contains("private bridge result")
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    drop(actor);
    service.shutdown().await;
}

#[tokio::test]
async fn controller_actor_scoped_cuts_import_native_receipts_and_drive_private_mcp_until_revoke() {
    let provider = Provider::new();
    let (fixture, service, actor, id, _, _metadata) = setup(&provider).await;
    let open = actor.dispatch_open().await.unwrap();
    assert!(matches!(&open, RemoteOpen::Broker(_)));
    assert!(actor.dispatch_open().await.is_err());
    let receipt = session_receipt(&fixture, &id);
    actor
        .persist(wire::SessionWorkerMessage::SessionAccepted {
            receipt: receipt.clone(),
        })
        .await
        .unwrap();
    actor
        .persist(wire::SessionWorkerMessage::SessionReady {
            receipt: receipt.clone(),
        })
        .await
        .unwrap();
    let tools = catalog(&fixture, &receipt).await;
    {
        let store = fixture.state.store.lock().unwrap();
        let context = store
            .team_project_context(
                &fixture.data.workspace,
                &fixture.data.project,
                &fixture.data.owner,
            )
            .unwrap();
        assert!(service.queue_allowed(&fixture.state, &store, &id, &fixture.data.owner));
        store
            .interactive_journal()
            .enqueue(&context, &id, "actor-native-turn", "perform lookup")
            .unwrap();
    }
    let command = actor.claim_submit().await.unwrap();
    let accepted = wire::TurnReceipt {
        identity: command.identity.clone(),
        budgets: command.budgets.clone(),
        fence: wire::TurnFence {
            session: receipt.fence.clone(),
            turn_id: command.identity.turn_id,
            turn_index: command.turn_index,
            turn_epoch: 1,
        },
    };
    actor
        .persist(wire::SessionWorkerMessage::TurnAccepted {
            receipt: accepted.clone(),
        })
        .await
        .unwrap();
    let request = BrokerRequest {
        scope: BrokerScope::Turn {
            turn: accepted.fence,
        },
        request_id: "actor-approved-call".into(),
        operation: BrokerOperation::ToolsCall {
            name: tools["tools"][0]["name"].as_str().unwrap().into(),
            arguments_json: json!({"query":"approved fixture"}).to_string(),
        },
    };
    let transport = fixture.transport.clone();
    let task = tokio::spawn(async move { transport.broker_request_fixture(request).await });
    approve(&fixture, &id, true).await;
    assert!(
        task.await
            .unwrap()
            .unwrap()
            .contains("private bridge result")
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    assert!(matches!(
        actor.renew().await.unwrap(),
        wire::SessionControllerMessage::RenewSession { sequence: 1, .. }
    ));
    fixture
        .state
        .store
        .lock()
        .unwrap()
        .disable(&fixture.context)
        .unwrap();
    assert!(actor.renew().await.is_err());
    assert!(
        actor
            .persist(wire::SessionWorkerMessage::SessionReady {
                receipt: receipt.clone()
            })
            .await
            .is_err()
    );
    {
        let store = fixture.state.store.lock().unwrap();
        assert!(!service.queue_allowed(&fixture.state, &store, &id, &fixture.data.owner));
        assert_eq!(store.remote_session_snapshot(&id).unwrap().phase, "active");
    }
    let stop = event(
        &receipt,
        1,
        None,
        wire::SessionEventKind::SessionStopped {
            reason: wire::SessionStopReason::Unknown,
        },
    );
    actor
        .persist(wire::SessionWorkerMessage::Event { event: stop })
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .store
            .lock()
            .unwrap()
            .remote_session_snapshot(&id)
            .unwrap()
            .phase,
        "stopped"
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    assert!(actor.renew().await.is_err());
    drop(actor);
    service.shutdown().await;
}
