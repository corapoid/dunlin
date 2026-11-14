use super::*;
use crate::scoped::tests::{TestJournal, binding, policy, scope};
use crate::store::{MemoryStore, SecretStore};
use dunlin_domain::vault::{Recipe, RecipeParameter};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize},
    },
};
const SECRET: &str = "recipe-private-token";
#[derive(Default)]
struct Secrets {
    values: MemoryStore,
    reads: AtomicUsize,
    after_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl SecretStore for Secrets {
    fn get(&self, id: &str) -> std::result::Result<Zeroizing<String>, String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let value = self.values.get(id);
        if let Some(hook) = self.after_read.lock().unwrap().take() {
            hook();
        }
        value
    }
    fn set(&self, id: &str, value: &str) -> std::result::Result<(), String> {
        self.values.set(id, value)
    }
    fn delete(&self, id: &str) -> std::result::Result<(), String> {
        self.values.delete(id)
    }
    fn get_policy(&self, _: &str) -> std::result::Result<Zeroizing<String>, String> {
        panic!("immutable envelope only")
    }
    fn set_policy(&self, _: &str, _: &str) -> std::result::Result<(), String> {
        panic!("immutable envelope only")
    }
    fn delete_policy(&self, _: &str) -> std::result::Result<(), String> {
        panic!("immutable envelope only")
    }
}
struct AuthorityFixture {
    scope: UseScope,
    version: Version,
    target: RecipeTarget,
    allowed: AtomicBool,
    deny_marker: AtomicBool,
    fail_ack: AtomicBool,
    used: Mutex<HashSet<String>>,
    outcomes: Mutex<Vec<Completion>>,
}
impl Authority for AuthorityFixture {
    fn check(&self, scope: &UseScope, version: &Version) -> Result<()> {
        if self.allowed.load(Ordering::Acquire) && *scope == self.scope && *version == self.version
        {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    }
    fn consume(&self, scope: &UseScope, version: &Version, digest: &str) -> Result<()> {
        self.check(scope, version)?;
        assert_eq!(digest, input_digest(&input(), &self.target).unwrap());
        if self.used.lock().unwrap().insert(scope.request.clone()) {
            Ok(())
        } else {
            Err(Error::Conflict)
        }
    }
    fn complete(&self, _: &UseScope, _: &Version, outcome: Completion) -> Result<()> {
        self.outcomes.lock().unwrap().push(outcome);
        if self.fail_ack.load(Ordering::Acquire) {
            Err(Error::Storage)
        } else {
            Ok(())
        }
    }
}
impl RecipeAuthority for AuthorityFixture {
    fn check_target(&self, _: &UseScope, _: &Version, target: &RecipeTarget) -> Result<()> {
        if *target != self.target
            || (self.deny_marker.load(Ordering::Acquire) && target.cwd.join("started").exists())
        {
            Err(Error::Denied)
        } else {
            Ok(())
        }
    }
}
struct Fixture {
    credentials: Credentials<TestJournal>,
    secrets: Arc<Secrets>,
    authority: Arc<AuthorityFixture>,
    _program: tempfile::TempDir,
    _cwd: tempfile::TempDir,
}
impl Fixture {
    fn new(script: &str, pin: bool) -> Self {
        Self::with_policy(script, pin, |_| {})
    }
    fn with_policy(script: &str, pin: bool, edit: impl FnOnce(&mut SecretPolicy)) -> Self {
        Self::with_secret(script, pin, edit, SECRET)
    }
    fn with_secret(
        script: &str,
        pin: bool,
        edit: impl FnOnce(&mut SecretPolicy),
        value: &str,
    ) -> Self {
        let program_dir = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let program = program_dir.path().join("pinned-recipe");
        std::fs::write(&program, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut policy = policy();
        policy.recipes.push(Recipe {
            id: "fixed".into(),
            secret_id: "host-selected".into(),
            name: "check".into(),
            description: String::new(),
            program: program.to_str().unwrap().into(),
            args: vec!["{item}".into()],
            env_var: "DUNLIN_SECRET".into(),
            parameters: vec![RecipeParameter {
                name: "item".into(),
                required: true,
                max_len: 64,
                allow_leading_dash: false,
            }],
            timeout_secs: 30,
            program_hash: pin.then(|| crate::recipe::program_hash(&program).unwrap()),
            created_at: now_secs(),
        });
        edit(&mut policy);
        let secrets = Arc::new(Secrets::default());
        let credentials = Credentials::new(Arc::new(TestJournal::default()), secrets.clone());
        let version = credentials
            .install(
                "host-selected",
                binding(),
                0,
                policy,
                Zeroizing::new(value.into()),
            )
            .unwrap();
        let target = RecipeTarget::new(
            binding(),
            std::fs::canonicalize(cwd.path()).unwrap(),
            "b".repeat(64),
        )
        .unwrap();
        let authority = Arc::new(AuthorityFixture {
            scope: scope(),
            version,
            target,
            allowed: AtomicBool::new(true),
            deny_marker: AtomicBool::new(false),
            fail_ack: AtomicBool::new(false),
            used: Mutex::default(),
            outcomes: Mutex::default(),
        });
        Self {
            credentials,
            secrets,
            authority,
            _program: program_dir,
            _cwd: cwd,
        }
    }
    fn permit(&self) -> Result<RecipePermit> {
        self.credentials.permit_recipe(
            self.authority.scope.clone(),
            self.authority.version.clone(),
            &input(),
            self.authority.target.clone(),
            Instant::now() + Duration::from_secs(10),
            self.authority.clone(),
        )
    }
}
fn input() -> RecipeInput {
    RecipeInput {
        secret: "host-selected".into(),
        recipe: "check".into(),
        params: BTreeMap::from([("item".into(), "approved".into())]),
    }
}
struct Executor {
    calls: AtomicUsize,
    revoke: Option<Arc<AuthorityFixture>>,
    fail: bool,
    oversized: bool,
}
impl RecipeExecutor for Executor {
    fn execute(&self, _: &RecipePlan) -> std::result::Result<RecipeOutput, String> {
        panic!("guarded dispatch required")
    }
    fn execute_guarded(
        &self,
        plan: &RecipePlan,
        access: Option<LiveAccess>,
    ) -> std::result::Result<RecipeOutput, String> {
        assert!(access.unwrap().allowed());
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(plan.secret.as_str(), SECRET);
        assert_eq!(plan.args, vec!["approved"]);
        assert!(plan.timeout <= Duration::from_secs(10));
        assert!(!format!("{plan:?}").contains(SECRET));
        if let Some(authority) = &self.revoke {
            authority.allowed.store(false, Ordering::Release);
        }
        if self.fail {
            return Err(format!("private error {SECRET}"));
        }
        Ok(RecipeOutput {
            exit_code: Some(0),
            signal: None,
            stdout: if self.oversized {
                "x".repeat(http::MAX_RESPONSE_BODY + 1)
            } else {
                format!("{SECRET} output")
            },
            stderr: SECRET.into(),
            truncated: false,
            timed_out: false,
        })
    }
}
fn executor() -> Executor {
    Executor {
        calls: AtomicUsize::new(0),
        revoke: None,
        fail: false,
        oversized: false,
    }
}
#[test]
fn target_input_and_read_only_are_fenced_and_consumed_recipe_cannot_replay() {
    let f = Fixture::new("#!/bin/sh\nexit 0\n", true);
    let execute = executor();
    let mut other = f.authority.target.clone();
    other.checkout_stamp = "c".repeat(64);
    assert!(matches!(
        f.credentials.permit_recipe(
            scope(),
            f.authority.version.clone(),
            &input(),
            other,
            Instant::now() + Duration::from_secs(10),
            f.authority.clone()
        ),
        Err(Error::Denied)
    ));
    let mut read_only = scope();
    read_only.read_only = true;
    assert!(matches!(
        f.credentials.permit_recipe(
            read_only,
            f.authority.version.clone(),
            &input(),
            f.authority.target.clone(),
            Instant::now() + Duration::from_secs(10),
            f.authority.clone()
        ),
        Err(Error::Denied)
    ));
    let permit = f.permit().unwrap();
    let mut changed = input();
    changed.params.insert("item".into(), "changed".into());
    assert_eq!(
        f.credentials.recipe(permit, changed, &execute),
        Err(Error::Denied)
    );
    assert_eq!(execute.calls.load(Ordering::Relaxed), 0);
    assert_eq!(f.secrets.reads.load(Ordering::Relaxed), 0);
    assert!(matches!(f.permit(), Err(Error::Conflict)));
    assert_eq!(
        *f.authority.outcomes.lock().unwrap(),
        vec![Completion::Failed]
    );
}
#[test]
fn revoke_after_keyring_read_and_during_execution_cannot_publish_outputs_or_revive_a_request() {
    for after_read in [true, false] {
        let f = Fixture::new("#!/bin/sh\nexit 0\n", true);
        let permit = f.permit().unwrap();
        let mut execute = executor();
        if after_read {
            let a = f.authority.clone();
            *f.secrets.after_read.lock().unwrap() =
                Some(Box::new(move || a.allowed.store(false, Ordering::Release)));
        } else {
            execute.revoke = Some(f.authority.clone());
        }
        assert_eq!(
            f.credentials.recipe(permit, input(), &execute),
            Err(if after_read {
                Error::Denied
            } else {
                Error::OutcomeUnknown
            })
        );
        assert_eq!(
            execute.calls.load(Ordering::Relaxed),
            usize::from(!after_read)
        );
        assert_eq!(
            *f.authority.outcomes.lock().unwrap(),
            vec![if after_read {
                Completion::Failed
            } else {
                Completion::Unknown
            }]
        );
        f.authority.allowed.store(true, Ordering::Release);
        assert!(matches!(f.permit(), Err(Error::Conflict)));
    }
}
#[test]
fn missing_pin_executor_failure_and_lost_completion_never_leak_secret_or_retry() {
    for fault in ["pin", "execute", "ack", "output"] {
        let f = Fixture::new("#!/bin/sh\nexit 0\n", fault != "pin");
        let permit = f.permit().unwrap();
        let mut execute = executor();
        execute.fail = fault == "execute";
        execute.oversized = fault == "output";
        f.authority
            .fail_ack
            .store(fault == "ack", Ordering::Release);
        let result = f.credentials.recipe(permit, input(), &execute);
        assert_eq!(
            result,
            Err(if fault == "pin" {
                Error::Denied
            } else {
                Error::OutcomeUnknown
            })
        );
        assert!(!format!("{result:?}").contains(SECRET));
        assert_eq!(
            execute.calls.load(Ordering::Relaxed),
            usize::from(fault != "pin")
        );
        assert!(matches!(f.permit(), Err(Error::Conflict)));
    }
}
#[cfg(unix)]
#[test]
fn real_fixed_recipe_uses_pinned_program_private_environment_and_redacts_both_streams() {
    let f = Fixture::new(
        "#!/bin/sh\nprintf '%s:%s' \"$DUNLIN_SECRET\" \"$1\"\nprintf '%s' \"$DUNLIN_SECRET\" >&2\n",
        true,
    );
    let result = f
        .credentials
        .recipe(
            f.permit().unwrap(),
            input(),
            &crate::recipe::CommandExecutor,
        )
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "[REDACTED]:approved");
    assert_eq!(result.stderr, "[REDACTED]");
    assert_eq!(
        *f.authority.outcomes.lock().unwrap(),
        vec![Completion::Complete]
    );
}
#[cfg(unix)]
#[test]
fn live_target_revocation_kills_a_real_recipe_group_and_records_unknown() {
    let f = Fixture::new("#!/bin/sh\ntouch started\nsleep 20\ntouch escaped\n", true);
    f.authority.deny_marker.store(true, Ordering::Release);
    let start = Instant::now();
    let result = f.credentials.recipe(
        f.permit().unwrap(),
        input(),
        &crate::recipe::CommandExecutor,
    );
    assert_eq!(result, Err(Error::OutcomeUnknown));
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(f.authority.target.cwd.join("started").exists());
    assert!(!f.authority.target.cwd.join("escaped").exists());
    assert_eq!(
        *f.authority.outcomes.lock().unwrap(),
        vec![Completion::Unknown]
    );
}

#[test]
fn unsupported_executor_cannot_fall_back_to_an_unguarded_spawn() {
    struct Unsupported;
    impl RecipeExecutor for Unsupported {
        fn execute(&self, _: &RecipePlan) -> std::result::Result<RecipeOutput, String> {
            panic!("must not dispatch")
        }
    }
    let f = Fixture::new("#!/bin/sh\nexit 0\n", true);
    assert_eq!(
        f.credentials
            .recipe(f.permit().unwrap(), input(), &Unsupported),
        Err(Error::OutcomeUnknown)
    );
    assert_eq!(
        *f.authority.outcomes.lock().unwrap(),
        vec![Completion::Unknown]
    );
}

#[test]
fn duplicate_recipe_names_cannot_choose_the_first_template() {
    let f = Fixture::with_policy("#!/bin/sh\nexit 0\n", true, |policy| {
        policy.recipes.push(policy.recipes[0].clone())
    });
    let execute = executor();
    assert_eq!(
        f.credentials.recipe(f.permit().unwrap(), input(), &execute),
        Err(Error::Denied)
    );
    assert_eq!(execute.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn redaction_growth_is_capped_in_both_streams_without_breaking_utf8() {
    struct Growth;
    impl RecipeExecutor for Growth {
        fn execute(&self, _: &RecipePlan) -> std::result::Result<RecipeOutput, String> {
            panic!("guard required")
        }
        fn execute_guarded(
            &self,
            plan: &RecipePlan,
            access: Option<LiveAccess>,
        ) -> std::result::Result<RecipeOutput, String> {
            assert!(access.unwrap().allowed());
            assert_eq!(plan.secret.as_str(), "x");
            Ok(RecipeOutput {
                exit_code: Some(0),
                signal: None,
                stdout: "x".repeat(http::MAX_RESPONSE_BODY),
                stderr: format!("x!{}", "é".repeat((http::MAX_RESPONSE_BODY - 2) / 2)),
                truncated: false,
                timed_out: false,
            })
        }
    }
    let f = Fixture::with_secret("#!/bin/sh\nexit 0\n", true, |_| {}, "x");
    let output = f
        .credentials
        .recipe(f.permit().unwrap(), input(), &Growth)
        .unwrap();
    assert!(output.truncated);
    assert!(output.stdout.len() <= http::MAX_RESPONSE_BODY);
    assert!(output.stderr.len() <= http::MAX_RESPONSE_BODY);
    assert!(!output.stdout.contains('x'));
    assert!(!output.stderr.contains('x'));
    assert!(output.stderr.contains('é'));
}

#[test]
fn basic_authorization_echo_is_redacted_from_recipe_output() {
    use base64::Engine as _;
    struct BasicEcho;
    impl RecipeExecutor for BasicEcho {
        fn execute(&self, _: &RecipePlan) -> std::result::Result<RecipeOutput, String> {
            panic!("guard required")
        }
        fn execute_guarded(
            &self,
            plan: &RecipePlan,
            access: Option<LiveAccess>,
        ) -> std::result::Result<RecipeOutput, String> {
            assert!(access.unwrap().allowed());
            let value = base64::engine::general_purpose::STANDARD
                .encode(format!("account:{}", plan.secret.as_str()));
            Ok(RecipeOutput {
                exit_code: Some(0),
                signal: None,
                stdout: format!("Basic {value}"),
                stderr: value,
                truncated: false,
                timed_out: false,
            })
        }
    }
    let f = Fixture::with_policy("#!/bin/sh\nexit 0\n", true, |policy| {
        policy.injection = Injection::Basic {
            username: "account".into(),
        }
    });
    let output = f
        .credentials
        .recipe(f.permit().unwrap(), input(), &BasicEcho)
        .unwrap();
    assert_eq!(output.stdout, "Basic [REDACTED]");
    assert_eq!(output.stderr, "[REDACTED]");
}
