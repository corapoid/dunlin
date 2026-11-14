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
    recipe_program: Option<std::path::PathBuf>,
    policy: dunlin_domain::vault::SecretPolicy,
}
impl ScopedFixture {
    async fn new() -> Self {
        Self::new_with_recipe(None).await
    }
    async fn new_with_recipe(script: Option<&str>) -> Self {
        let recipe = script.is_some();
        let executable = std::env::var_os("DUNLIN_RELAY_E2E_BIN")
            .expect("production --mcp-tools executable required");
        assert!(std::path::Path::new(&executable).is_file());
        let mut f = fixture(if recipe {
            "mcp-scoped-recipe-real"
        } else {
            "mcp-scoped-real"
        });
        let recipe_program = script.map(|script| {
            use std::os::unix::fs::PermissionsExt;
            let path = f.dir.path().join("trusted-recipe");
            let canary = f.dir.path().join("host-private-canary");
            std::fs::write(&canary, "host-private-canary-value").unwrap();
            let script = script.replace("@HOST_CANARY@", canary.to_str().unwrap());
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        });
        if recipe {
            let mut config = (*f.state.config).clone();
            config.sandbox = true;
            f.state.config = Arc::new(config);
            let script = f.dir.path().join("checkout/fake_acp.py");
            std::fs::write(&script, include_str!("fake_acp.py")).unwrap();
            Arc::get_mut(&mut f.state.interactive).unwrap().command = Some((
                "/usr/bin/python3".into(),
                vec![
                    script.to_str().unwrap().into(),
                    "mcp-scoped-recipe-real".into(),
                ],
            ));
        }
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
        let mut policy: dunlin_domain::vault::SecretPolicy=serde_json::from_value(json!({"version":1,"managed":true,"injection":{"type":"header","name":"Authorization","prefix":"Bearer "},"allowed_hosts":["api.example.com"],"path_prefix":"/items","allowed_methods":["GET"],"allowed_headers":[],"auto_approve":false,"expires_at":null,"project_ids":[f.project],"recipes":[],"operations":[]})).unwrap();
        if let Some(program) = &recipe_program {
            policy.recipes.push(dunlin_domain::vault::Recipe {
                id: "check".into(),
                secret_id: "local-api".into(),
                name: "check".into(),
                description: "Fixed local recipe".into(),
                program: program.to_str().unwrap().into(),
                args: vec!["{item}".into()],
                env_var: "DUNLIN_SECRET".into(),
                parameters: vec![dunlin_domain::vault::RecipeParameter {
                    name: "item".into(),
                    required: true,
                    max_len: 32,
                    allow_leading_dash: false,
                }],
                timeout_secs: 20,
                program_hash: Some(dunlin_vault::recipe::program_hash(program).unwrap()),
                created_at: 0,
            });
        }
        f.state
            .scoped_secrets
            .install(
                f.state.clone(),
                proof.clone(),
                crate::scoped_secrets::Install {
                    credential: "local-api".into(),
                    expected_revision: 0,
                    policy: policy.clone(),
                    value: Zeroizing::new(VALUE.into()),
                },
            )
            .await
            .unwrap();
        let mut definition = profiles::definition("Local scoped HTTP acceptance");
        definition.schema_version = 2;
        let tool = if recipe {
            dunlin_vault::scoped::tool::recipe()
        } else {
            dunlin_vault::scoped::tool::http()
        };
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
                    read_only: !recipe,
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
            recipe_program,
            policy,
        }
    }
    async fn approve(&self) {
        self.decide(true).await;
    }
    async fn decide(&self, approved: bool) -> (ahp_types::commands::DispatchActionParams, Value) {
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
        let action = permission_decision_action(&self.f, &self.session, approved).unwrap();
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
        (action, serde_json::to_value(receipt).unwrap())
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires production proxy and Linux bubblewrap"]
async fn production_browser_acceptance_runtime_local_scoped_recipe_complete() {
    let script = "#!/bin/sh\nset -eu\ntest \"$PWD\" = /workspace\ntest \"$HOME\" = /home/dunlin\ntest ! -e /root/.aws/credentials\ntest ! -e '@HOST_CANARY@'\nfor f in /proc/self/fd/*; do v=$(readlink \"$f\" || :); case \"$v\" in *memfd:dunlin-recipe*|*/trusted-recipe|*/checkout) exit 31;; esac; done\nprintf '%s recipe result' \"$DUNLIN_SECRET\"\nprintf '%s' \"$DUNLIN_SECRET\" >&2\nprintf '%s\\n' \"$1\" >> recipe-effect\n";
    let f = ScopedFixture::new_with_recipe(Some(script)).await;
    let (action, receipt) = f.decide(true).await;
    until(&f.f, || profiles::completed(&f.f, &f.session, 1)).await;
    assert_eq!(
        std::fs::read_to_string(f.f.dir.path().join("checkout/recipe-effect")).unwrap(),
        "approved\n"
    );
    let replay = {
        let store = f.f.state.store.lock().unwrap();
        let context = store
            .team_project_context(&f.f.workspace, &f.f.project, &f.f.auth.user.id)
            .unwrap();
        serde_json::to_value(
            store
                .interactive_journal()
                .dispatch_action(
                    &context,
                    &f.session.id,
                    &f.f.auth.device.id,
                    "local-scoped-approval",
                    &action,
                    dunlin_session_runtime::DispatchAdmission::default(),
                )
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(replay, receipt);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        std::fs::read_to_string(f.f.dir.path().join("checkout/recipe-effect")).unwrap(),
        "approved\n"
    );
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
    f.assert_private();
    stop(&f.f, &f.session).await;
    if std::env::var("DUNLIN_REMOTE_SESSION_BROWSER").as_deref() == Ok("1") {
        browser(f).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires production proxy and Linux bubblewrap"]
async fn production_browser_acceptance_runtime_local_scoped_recipe_pin_changed() {
    let f = ScopedFixture::new_with_recipe(Some("#!/bin/sh\ntouch recipe-effect\n")).await;
    std::fs::write(
        f.recipe_program.as_ref().unwrap(),
        "#!/bin/sh\ntouch escaped\n",
    )
    .unwrap();
    f.approve().await;
    f.interrupted().await;
    assert!(!f.f.dir.path().join("checkout/escaped").exists());
    assert!(!f.f.dir.path().join("checkout/recipe-effect").exists());
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires production proxy and Linux bubblewrap"]
async fn production_browser_acceptance_runtime_local_scoped_recipe_revoke_running() {
    let f=ScopedFixture::new_with_recipe(Some("#!/bin/sh\nsetsid /bin/sh -c 'while :; do printf x >> recipe-ticks; sleep 0.02; done' &\ntouch recipe-started\nsleep 20\n")).await;
    f.approve().await;
    until(&f.f, || {
        f.f.dir.path().join("checkout/recipe-started").exists()
            && f.f.dir.path().join("checkout/recipe-ticks").exists()
    })
    .await;
    f.f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_binding(&f.f.auth.user.id, &f.f.workspace, &f.f.project, 1)
        .unwrap();
    f.interrupted().await;
    let ticks = f.f.dir.path().join("checkout/recipe-ticks");
    let before = std::fs::metadata(&ticks).unwrap().len();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(std::fs::metadata(ticks).unwrap().len(), before);
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires production proxy and Linux bubblewrap"]
async fn production_browser_acceptance_runtime_local_scoped_recipe_denied() {
    let f = ScopedFixture::new_with_recipe(Some("#!/bin/sh\ntouch recipe-effect\n")).await;
    f.decide(false).await;
    until(&f.f, || profiles::completed(&f.f, &f.session, 1)).await;
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    assert_eq!(f.executor.calls.load(Ordering::Acquire), 0);
    assert!(!f.f.dir.path().join("checkout/recipe-effect").exists());
    f.assert_private();
    stop(&f.f, &f.session).await;
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
    let recipe = f.recipe_program.is_some();
    let input = json!({"origin":origin,"ownerCode":code,"workspace":f.f.workspace,"project":f.f.project,"profile":f.profile,"recipe":recipe,"policy":f.policy});
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
    let kind = if recipe { "recipe" } else { "http" };
    let mut images = vec![
        "desktop",
        "mobile",
        "retentionDesktop",
        "retentionMobile",
        "grantDesktop",
        "grantMobile",
    ];
    if recipe {
        images.extend(["builderDesktop", "builderMobile"]);
    }
    for name in images {
        println!(
            "LOCAL_SCOPED_UI_IMAGE_{kind}_{name} {}",
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
    assert_eq!(
        f.executor.calls.load(Ordering::Acquire),
        if recipe { 0 } else { 2 }
    );
    if recipe {
        assert_eq!(
            std::fs::read_to_string(f.f.dir.path().join("checkout/recipe-effect")).unwrap(),
            "approved\napproved\n"
        );
    }
    assert_eq!(f.secrets.deletes.load(Ordering::Acquire), 1);
    f.session = session;
    f.assert_private();
    stop(&f.f, &f.session).await;
}
