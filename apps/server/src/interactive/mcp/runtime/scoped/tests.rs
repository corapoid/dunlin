use super::*;
use crate::interactive::mcp::{
    BoundSession, Selection,
    authority::{Authority, SessionPins, TurnScope},
};
use dunlin_session_runtime::{DispatchAdmission, TurnPhase};
use dunlin_vault::{
    http::{HttpExecutor, HttpOutput, HttpPlan},
    store::{MemoryStore, SecretStore},
};
use std::sync::atomic::AtomicUsize;
use zeroize::Zeroizing;

#[derive(Default)]
struct Secrets {
    memory: MemoryStore,
    reads: AtomicUsize,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> std::result::Result<Zeroizing<String>, String> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        let value = self.memory.get(id)?;
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook();
        }
        Ok(value)
    }
    fn set(&self, id: &str, value: &str) -> std::result::Result<(), String> {
        self.memory.set(id, value)
    }
    fn delete(&self, _: &str) -> std::result::Result<(), String> {
        panic!("no runtime deletion")
    }
    fn get_policy(&self, _: &str) -> std::result::Result<Zeroizing<String>, String> {
        panic!("protected envelope only")
    }
    fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
        panic!("protected envelope only")
    }
    fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
        panic!("no runtime deletion")
    }
}
#[derive(Default)]
struct Executor {
    calls: AtomicUsize,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl HttpExecutor for Executor {
    fn execute(&self, plan: &HttpPlan) -> std::result::Result<HttpOutput, String> {
        assert!(
            plan.headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization")
                    && v == "Bearer runtime-fixture-private-value")
        );
        self.calls.fetch_add(1, Ordering::AcqRel);
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook();
        }
        Ok(HttpOutput {
            status: 200,
            content_type: Some("text/plain".into()),
            body: "runtime-fixture-private-value public-result".into(),
            truncated: false,
        })
    }
}
struct Host {
    pins: SessionPins,
    turn: Mutex<String>,
    stop: AtomicBool,
}
impl Authority for Host {
    fn pins(&self) -> &SessionPins {
        &self.pins
    }
    fn check(&self, _: &AppState, deadline: Instant) -> Result<()> {
        if self.stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            Err("fixture_stopped")
        } else {
            Ok(())
        }
    }
    fn context(&self, _: &AppState, store: &Store) -> Result<AccessContext> {
        if self.stop.load(Ordering::Acquire) {
            return Err("fixture_stopped");
        }
        store
            .team_project_context(
                &self.pins.workspace,
                &self.pins.secret_target.project,
                &self.pins.owner,
            )
            .map_err(|_| "fixture_source_changed")
    }
    fn context_in_transaction(&self, state: &AppState, store: &Store) -> Result<AccessContext> {
        self.context(state, store)
    }
    fn turn(&self) -> Result<TurnScope> {
        Ok(TurnScope {
            id: self.turn.lock().unwrap().clone(),
            deadline: Instant::now() + Duration::from_secs(30),
        })
    }
    fn changed(&self, _: &Store) {}
    fn interrupt(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
struct Fixture {
    broker: Arc<Broker>,
    host: Arc<Host>,
    proof: crate::store::scoped_secrets::Proof,
    secrets: Arc<Secrets>,
    executor: Arc<Executor>,
    grant: Grant,
    params: Value,
}
impl Fixture {
    async fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let proof = crate::store::scoped_secrets::testing_scope(&store);
        let secrets = Arc::new(Secrets::default());
        let executor = Arc::new(Executor::default());
        let mut state = AppState::new(
            store,
            crate::config::Config {
                db: "/nonexistent/scoped-runtime-fixture/db".into(),
                ..Default::default()
            },
        );
        state.scoped_secrets = Arc::new(crate::scoped_secrets::Service::testing_executor(
            secrets.clone(),
            executor.clone(),
        ));
        let policy=serde_json::from_value(json!({"version":1,"managed":true,"injection":{"type":"header","name":"Authorization","prefix":"Bearer "},"allowed_hosts":["api.example.com"],"path_prefix":"/","allowed_methods":["GET"],"allowed_headers":[],"auto_approve":false,"expires_at":null,"project_ids":[proof.binding.project],"recipes":[],"operations":[]})).unwrap();
        state
            .scoped_secrets
            .install(
                state.clone(),
                proof.clone(),
                crate::scoped_secrets::Install {
                    credential: "api".into(),
                    expected_revision: 0,
                    policy,
                    value: Zeroizing::new("runtime-fixture-private-value".into()),
                },
            )
            .await
            .unwrap();
        let (grant, connections, pins, turn) = {
            let store = state.store.lock().unwrap();
            let definition:dunlin_domain::agent::AgentDefinition=serde_json::from_value(json!({"schema_version":2,"name":"HTTP consumer","harness":"claude","model":"fixture","mode":"execute","skills":[],"mcp":[{"provider_id":"dunlin-secret-api","name":"http","definition_digest":scoped::tool::http().definition_digest().unwrap()}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
            let draft = store
                .create_agent(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    definition.clone(),
                )
                .unwrap();
            let published = store
                .publish_agent(
                    &proof.browser.user,
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &draft.id,
                    1,
                )
                .unwrap();
            let profile = dunlin_domain::agent::AgentProfileRef {
                agent_id: published.agent_id,
                version: published.version,
                digest: published.digest,
            };
            let request = crate::store::scoped_secrets::GrantRequest {
                expected_revision: 0,
                credential_revision: 1,
                principal_kind: crate::store::scoped_secrets::Principal::Human,
                principal_id: proof.browser.user.clone(),
                profile: profile.clone(),
                provider_id: "dunlin-secret-api".into(),
                read_only: true,
            };
            let grant = store
                .scoped_secret_set_grant(&proof, "api", &request, &|_| Ok(()))
                .unwrap();
            let bound = BoundSession::capture(
                &state.config,
                &store,
                &proof.browser.user,
                &proof.binding.workspace,
                &definition,
                &[Selection {
                    provider_id: grant.provider_id.clone(),
                    connection_id: grant.id.clone(),
                    revision: grant.revision,
                }],
            )
            .unwrap();
            bound.validate_target(&proof.binding, &profile).unwrap();
            let encoded = bound.encode().unwrap();
            assert!(!encoded.contains("scoped-secret:"));
            assert!(!encoded.contains("runtime-fixture-private-value"));
            let restored = BoundSession::restore(
                &state.config,
                &store,
                &proof.browser.user,
                &proof.binding.workspace,
                &definition,
                &encoded,
            )
            .unwrap();
            let a = store
                .team_project_context(
                    &proof.binding.workspace,
                    &proof.binding.project,
                    &proof.browser.user,
                )
                .unwrap();
            let j = store.interactive_journal();
            let session = j
                .create(&a, &proof.binding.workspace, &proof.binding.project)
                .unwrap();
            let lease = j.acquire(&a, &session.id, 0, "fixture-worker").unwrap();
            let turn = j.enqueue(&a, &session.id, "prompt-1", "Use HTTP").unwrap();
            j.claim_next(&a, &session.id, &lease).unwrap();
            j.transition(&a, &session.id, &lease, &turn, TurnPhase::Running)
                .unwrap();
            let pins = SessionPins {
                session: session.id,
                workspace: proof.binding.workspace.clone(),
                owner: proof.browser.user.clone(),
                lease,
                starter_device: proof.browser.device.clone(),
                binding_revision: 1,
                profile_digest: profile.digest.clone(),
                secret_target: proof.binding.clone(),
                profile,
            };
            (grant, restored.into_connections(), pins, turn)
        };
        let host = Arc::new(Host {
            pins,
            turn: Mutex::new(turn),
            stop: AtomicBool::new(false),
        });
        let broker = Broker::prepare(
            state,
            host.clone(),
            connections,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();
        let tools = broker.request("list", "tools/list", json!({})).unwrap();
        assert_eq!(tools["tools"].as_array().unwrap().len(), 1);
        let params = json!({"name":tools["tools"][0]["name"],"arguments":{"method":"GET","url":"https://api.example.com/items"}});
        Self {
            broker,
            host,
            proof,
            secrets,
            executor,
            grant,
            params,
        }
    }
    fn decide(&self, approved: bool) -> bool {
        let store = self.broker.state.store.lock().unwrap();
        let a = store
            .team_project_context(
                &self.proof.binding.workspace,
                &self.proof.binding.project,
                &self.proof.browser.user,
            )
            .unwrap();
        let j = store.interactive_journal();
        let chat =
            serde_json::to_value(j.projected_chat(&a, &self.host.pins.session).unwrap()).unwrap();
        let Some(parts) = chat["activeTurn"]["responseParts"].as_array() else {
            return false;
        };
        let Some(tool) = parts
            .iter()
            .find(|p| p["kind"] == "toolCall" && p["toolCall"]["status"] == "pending-confirmation")
            .map(|p| &p["toolCall"])
        else {
            return false;
        };
        let option = tool["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["kind"] == if approved { "approve" } else { "deny" })
            .unwrap()["id"]
            .clone();
        let action=serde_json::from_value(json!({"channel":format!("ahp-chat:/{}",self.host.pins.session),"clientSeq":1,"action":{"type":"chat/toolCallConfirmed","turnId":*self.host.turn.lock().unwrap(),"toolCallId":tool["toolCallId"],"approved":approved,"selectedOptionId":option}})).unwrap();
        let result = j
            .dispatch_action(
                &a,
                &self.host.pins.session,
                &self.proof.browser.device,
                "approval-1",
                &action,
                DispatchAdmission::default(),
            )
            .unwrap();
        assert!(
            result.rejection_reason.is_none(),
            "{:?}",
            result.rejection_reason
        );
        true
    }
    async fn run(&self) -> Result<Value> {
        self.run_decision(true).await
    }
    async fn run_decision(&self, approved: bool) -> Result<Value> {
        let broker = self.broker.clone();
        let params = self.params.clone();
        let task =
            tokio::task::spawn_blocking(move || broker.request("http-1", "tools/call", params));
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if self.decide(approved) {
                break;
            }
            assert!(Instant::now() < until, "approval was not published");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task.await.unwrap()
    }
    fn revoke_hook(&self) -> Box<dyn FnOnce() + Send> {
        let state = self.broker.state.clone();
        let proof = self.proof.clone();
        let grant = self.grant.clone();
        Box::new(move || {
            state
                .store
                .lock()
                .unwrap()
                .scoped_secret_revoke_grant(&proof, "api", &grant.id, grant.revision, &|_| Ok(()))
                .unwrap();
        })
    }
}
#[tokio::test]
async fn scoped_http_runs_only_after_sdk_approval_redacts_result_and_reuses_receipt_without_replay()
{
    let f = Fixture::new().await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    let value = f.run().await.unwrap();
    assert!(!value.to_string().contains("runtime-fixture-private-value"));
    assert!(value.to_string().contains("public-result"));
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    assert_eq!(
        f.broker
            .request("http-1", "tools/call", f.params.clone())
            .unwrap(),
        value
    );
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    let restored = Broker::prepare(
        f.broker.state.clone(),
        f.host.clone(),
        f.broker.connections.clone(),
        Instant::now() + Duration::from_secs(30),
    )
    .await
    .unwrap();
    assert!(
        restored
            .request("http-1", "tools/call", f.params.clone())
            .is_err()
    );
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    let mut changed = f.params.clone();
    changed["arguments"]["url"] = json!("https://api.example.com/other");
    assert!(f.broker.request("http-1", "tools/call", changed).is_err());
}
#[tokio::test]
async fn scoped_http_grant_revocation_after_keyring_read_prevents_external_dispatch() {
    let f = Fixture::new().await;
    *f.secrets.hook.lock().unwrap() = Some(f.revoke_hook());
    assert!(f.run().await.is_err());
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}
#[tokio::test]
async fn scoped_http_grant_revocation_during_http_discards_response_and_stops_runtime() {
    let f = Fixture::new().await;
    *f.executor.hook.lock().unwrap() = Some(f.revoke_hook());
    assert!(f.run().await.is_err());
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    assert!(f.host.stop.load(Ordering::Acquire));
    assert!(
        f.broker
            .request("http-1", "tools/call", f.params.clone())
            .is_err()
    );
}
#[tokio::test]
async fn scoped_http_rejects_changed_target_profile_turn_and_revoked_grant_before_keyring() {
    for field in ["host", "profile", "turn", "grant"] {
        let mut f = Fixture::new().await;
        if field == "turn" {
            *f.host.turn.lock().unwrap() = "replacement-turn".into();
        } else if field == "grant" {
            (f.revoke_hook())();
        } else {
            let b = Arc::get_mut(&mut f.broker).unwrap();
            if field == "host" {
                b.pins.secret_target.host = "another-host".into();
            } else {
                b.pins.profile.agent_id = "another-profile".into();
            }
        }
        assert!(
            f.broker
                .request("invalid", "tools/call", f.params.clone())
                .is_err()
        );
        assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
        assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test]
async fn scoped_http_invalid_arguments_are_rejected_before_approval_or_keyring() {
    let f = Fixture::new().await;
    let mut params = f.params.clone();
    params["arguments"]["secret"] = json!("attacker-selected");
    assert!(
        f.broker
            .request("invalid-schema", "tools/call", params)
            .is_err()
    );
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}
#[tokio::test]
async fn scoped_http_revoked_approving_device_during_io_discards_response() {
    let f = Fixture::new().await;
    let state = f.broker.state.clone();
    let device = f.proof.browser.device.clone();
    *f.executor.hook.lock().unwrap() = Some(Box::new(move || {
        state.store.lock().unwrap().revoke_device(&device).unwrap();
    }));
    assert!(f.run().await.is_err());
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    assert!(f.host.stop.load(Ordering::Acquire));
}

#[tokio::test]
async fn scoped_http_denied_permission_never_reads_secret_or_dispatches() {
    let f = Fixture::new().await;
    let value = f.run_decision(false).await.unwrap();
    assert_eq!(value["isError"], true);
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}
