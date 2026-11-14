//! Actual local actor, native checkout authority and production stdio MCP proxy.
//! The keyring and HTTP executor are controlled; no external account is used.
use super::*;
use dunlin_vault::{
    http::{HttpExecutor, HttpOutput, HttpPlan},
    scoped::Binding as SecretBinding,
    store::{MemoryStore, SecretStore},
};
use std::sync::atomic::AtomicUsize;
use zeroize::Zeroizing;
const VALUE: &str = "local-runtime-private-value";
const PROVIDER: &str = "dunlin-secret-local";
type Hook = Mutex<Option<Box<dyn FnOnce() + Send>>>;
#[derive(Default)]
struct Secrets {
    memory: MemoryStore,
    reads: AtomicUsize,
    deletes: AtomicUsize,
    hook: Hook,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> std::result::Result<Zeroizing<String>, String> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        let result = self.memory.get(id)?;
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook();
        }
        Ok(result)
    }
    fn set(&self, id: &str, value: &str) -> std::result::Result<(), String> {
        self.memory.set(id, value)
    }
    fn delete(&self, id: &str) -> std::result::Result<(), String> {
        self.memory.delete(id)?;
        self.deletes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn get_policy(&self, _: &str) -> std::result::Result<Zeroizing<String>, String> {
        panic!("envelope only")
    }
    fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
        panic!("envelope only")
    }
    fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
        panic!("no implicit deletion")
    }
}
#[derive(Default)]
struct Executor {
    calls: AtomicUsize,
    hook: Hook,
}
impl HttpExecutor for Executor {
    fn execute_guarded(
        &self,
        plan: &HttpPlan,
        access: &dunlin_vault::access::LiveAccess,
    ) -> std::result::Result<HttpOutput, String> {
        if !access.allowed() {
            return Err("fixture authority denied".into());
        }
        self.execute(plan)
    }
    fn execute(&self, plan: &HttpPlan) -> std::result::Result<HttpOutput, String> {
        assert_eq!(plan.method, "GET");
        assert_eq!(plan.url, "https://api.example.com/items");
        assert!(plan.headers.iter().any(
            |(k, v)| k.eq_ignore_ascii_case("authorization") && v == &format!("Bearer {VALUE}")
        ));
        self.calls.fetch_add(1, Ordering::AcqRel);
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook();
        }
        Ok(HttpOutput {
            status: 200,
            content_type: Some("text/plain".into()),
            body: format!("{VALUE} local result"),
            truncated: false,
        })
    }
}
struct ScopedFixture {
    f: Fixture,
    secrets: Arc<Secrets>,
    executor: Arc<Executor>,
    session: Session,
    native: String,
    profile: ProfileRef,
}
impl ScopedFixture {
    async fn new() -> Self {
        let executable = std::env::var_os("DUNLIN_RELAY_E2E_BIN")
            .expect("production --mcp-tools executable required");
        assert!(std::path::Path::new(&executable).is_file());
        let mut f = fixture("mcp-scoped-real");
        let secrets = Arc::new(Secrets::default());
        let executor = Arc::new(Executor::default());
        f.state.scoped_secrets = Arc::new(crate::scoped_secrets::Service::testing_executor(
            secrets.clone(),
            executor.clone(),
        ));
        let (proof, native) = {
            let store = f.state.store.lock().unwrap();
            let cookie = store
                .create_browser_session(&f.auth.device.id, 600)
                .unwrap();
            let proof = store
                .scoped_secret_proof(
                    crate::mcp_oauth::BrowserProof {
                        user: f.auth.user.id.clone(),
                        device: f.auth.device.id.clone(),
                        session_hash: crate::auth::hash_token(&cookie),
                    },
                    SecretBinding {
                        workspace: f.workspace.clone(),
                        project: f.project.clone(),
                        host: "local".into(),
                    },
                )
                .unwrap();
            let native = store
                .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
                .unwrap()
                .unwrap()
                .native_project_id;
            (proof, native)
        };
        let policy=serde_json::from_value(json!({"version":1,"managed":true,"injection":{"type":"header","name":"Authorization","prefix":"Bearer "},"allowed_hosts":["api.example.com"],"path_prefix":"/items","allowed_methods":["GET"],"allowed_headers":[],"auto_approve":false,"expires_at":null,"project_ids":[f.project],"recipes":[],"operations":[]})).unwrap();
        f.state
            .scoped_secrets
            .install(
                f.state.clone(),
                proof.clone(),
                crate::scoped_secrets::Install {
                    credential: "local-api".into(),
                    expected_revision: 0,
                    policy,
                    value: Zeroizing::new(VALUE.into()),
                },
            )
            .await
            .unwrap();
        let mut definition = profiles::definition("Local scoped HTTP acceptance");
        definition.schema_version = 2;
        let tool = dunlin_vault::scoped::tool::http();
        definition.mcp.push(dunlin_domain::agent::AgentMcpTool {
            provider_id: PROVIDER.into(),
            name: tool.name.clone(),
            definition_digest: tool.definition_digest().unwrap(),
        });
        let published = profiles::publish(&f, definition);
        let profile = profiles::reference(&published);
        let grant = f
            .state
            .store
            .lock()
            .unwrap()
            .scoped_secret_set_grant(
                &proof,
                "local-api",
                &crate::store::scoped_secrets::GrantRequest {
                    expected_revision: 0,
                    credential_revision: 1,
                    principal_kind: crate::store::scoped_secrets::Principal::Human,
                    principal_id: f.auth.user.id.clone(),
                    profile: profile.clone(),
                    provider_id: PROVIDER.into(),
                    read_only: true,
                },
                &|_| Ok(()),
            )
            .unwrap();
        let session = Service::create_request_with_key(
            f.state.clone(),
            AuthUser {
                user: f.auth.user.clone(),
                device: f.auth.device.clone(),
            },
            f.workspace.clone(),
            f.project.clone(),
            CreateRequest::Profile {
                profile: profile.clone(),
                mcp_connections: vec![crate::interactive::mcp::Selection {
                    provider_id: PROVIDER.into(),
                    connection_id: grant.id,
                    revision: grant.revision,
                }],
                target: None,
            },
            Some("local-scoped-session".into()),
        )
        .unwrap()
        .session;
        Self {
            f,
            secrets,
            executor,
            session,
            native,
            profile,
        }
    }
    async fn approve(&self) {
        enqueue(
            &self.f,
            &self.session,
            "local-http",
            "Use local scoped HTTP",
        );
        until(&self.f, || {
            permission_action(&self.f, &self.session).is_some()
        })
        .await;
        assert_eq!(self.secrets.reads.load(Ordering::Acquire), 0);
        assert_eq!(self.executor.calls.load(Ordering::Acquire), 0);
        let action = permission_action(&self.f, &self.session).unwrap();
        let store = self.f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&self.f.workspace, &self.f.project, &self.f.auth.user.id)
            .unwrap();
        let receipt = store
            .interactive_journal()
            .dispatch_action(
                &context,
                &self.session.id,
                &self.f.auth.device.id,
                "local-scoped-approval",
                &action,
                dunlin_session_runtime::DispatchAdmission::default(),
            )
            .unwrap();
        assert!(receipt.rejection_reason.is_none(), "{receipt:?}");
    }
    fn assert_private(&self) {
        let store = self.f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&self.f.workspace, &self.f.project, &self.f.auth.user.id)
            .unwrap();
        let chat = serde_json::to_string(
            &store
                .interactive_journal()
                .projected_chat(&context, &self.session.id)
                .unwrap(),
        )
        .unwrap();
        assert!(!chat.contains(VALUE));
        assert!(!chat.contains("scoped-secret:"));
        for name in ["received-prompts", "received-mcp-result"] {
            let path = self.f.dir.path().join("checkout").join(name);
            match std::fs::read_to_string(path) {
                Ok(body) => {
                    assert!(!body.contains(VALUE));
                    assert!(!body.contains("scoped-secret:"));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    async fn interrupted(&self) {
        until(&self.f, || {
            phase(&self.f, &self.session) == SessionPhase::Interrupted
        })
        .await;
        until(&self.f, || {
            !self
                .f
                .state
                .interactive
                .actors
                .lock()
                .unwrap()
                .contains_key(&self.session.id)
        })
        .await;
        let store = self.f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&self.f.workspace, &self.f.project, &self.f.auth.user.id)
            .unwrap();
        let (_, turns) = store
            .interactive_journal()
            .snapshot(&context, &self.session.id)
            .unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].phase, TurnPhase::Unknown);
        assert!(
            !self
                .f
                .dir
                .path()
                .join("checkout/received-mcp-result")
                .exists()
        );
        drop(store);
        self.assert_private();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires built production stdio MCP proxy"]
async fn production_browser_acceptance_runtime_local_scoped_http_complete() {
    let f = ScopedFixture::new().await;
    f.approve().await;
    until(&f.f, || profiles::completed(&f.f, &f.session, 1)).await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
    let reply =
        std::fs::read_to_string(f.f.dir.path().join("checkout/received-mcp-result")).unwrap();
    assert!(reply.contains("[REDACTED] local result"));
    f.assert_private();
    stop(&f.f, &f.session).await;
    if std::env::var("DUNLIN_REMOTE_SESSION_BROWSER").as_deref() == Ok("1") {
        browser(f).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires built production stdio MCP proxy"]
async fn production_browser_acceptance_runtime_local_scoped_http_native_revoke_after_read() {
    let f = ScopedFixture::new().await;
    let state = f.f.state.clone();
    let native = f.native.clone();
    let user = f.f.auth.user.id.clone();
    *f.secrets.hook.lock().unwrap() = Some(Box::new(move || {
        state
            .store
            .lock()
            .unwrap()
            .revoke_grant(&native, &user)
            .unwrap()
    }));
    f.approve().await;
    f.interrupted().await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires built production stdio MCP proxy"]
async fn production_browser_acceptance_runtime_local_scoped_http_binding_revoke_during_http() {
    let f = ScopedFixture::new().await;
    let state = f.f.state.clone();
    let user = f.f.auth.user.id.clone();
    let workspace = f.f.workspace.clone();
    let project = f.f.project.clone();
    *f.executor.hook.lock().unwrap() = Some(Box::new(move || {
        state
            .store
            .lock()
            .unwrap()
            .revoke_local_binding(&user, &workspace, &project, 1)
            .unwrap();
    }));
    f.approve().await;
    f.interrupted().await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 1);
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn browser(mut f: ScopedFixture) {
    use tokio::io::AsyncWriteExt;
    let mut config = (*f.f.state.config).clone();
    config.web_root = Some(
        std::env::var_os("DUNLIN_WEB_E2E_ROOT")
            .expect("web build required")
            .into(),
    );
    f.f.state.config = Arc::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::api::router(f.f.state.clone());
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let code =
        f.f.state
            .store
            .lock()
            .unwrap()
            .create_pairing_code(&f.f.auth.user.id, 600)
            .unwrap()
            .0;
    let input = json!({"origin":origin,"ownerCode":code,"workspace":f.f.workspace,"project":f.f.project,"profile":f.profile});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/local-scoped-session.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{input}\n").as_bytes())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
        .await
        .expect("local Chromium deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "passed");
    for name in ["desktop", "mobile", "retentionDesktop", "retentionMobile"] {
        println!(
            "LOCAL_SCOPED_UI_IMAGE_{name} {}",
            result[name].as_str().unwrap()
        );
    }
    let session = {
        let store = f.f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.f.workspace, &f.f.project, &f.f.auth.user.id)
            .unwrap();
        store
            .interactive_journal()
            .snapshot(&context, result["session"]["id"].as_str().unwrap())
            .unwrap()
            .0
    };
    until(&f.f, || profiles::completed(&f.f, &session, 1)).await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 2);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 2);
    assert_eq!(f.secrets.deletes.load(Ordering::Acquire), 1);
    f.session = session;
    f.assert_private();
    stop(&f.f, &f.session).await;
}
