use super::*;
use base64::Engine as _;
fn target() -> RemoteRecipeTarget {
    RemoteRecipeTarget::new(
        binding(),
        "a".repeat(64),
        "b".repeat(64),
        "export".into(),
        1,
        "c".repeat(64),
    )
    .unwrap()
}
fn fixture() -> Fixture {
    let mut f = Fixture::with_policy("#!/bin/sh\nexit 0\n", true, |policy| {
        policy.injection = Injection::Basic {
            username: "account".into(),
        }
    });
    Arc::get_mut(&mut f.authority).unwrap().target = RecipeTarget::from_remote(target());
    f
}
struct Remote {
    calls: AtomicUsize,
    revoke: Option<Arc<AuthorityFixture>>,
    fail: bool,
}
impl RemoteRecipeExecutor for Remote {
    fn execute_guarded(
        &self,
        plan: &RemoteRecipePlan,
        access: LiveAccess,
    ) -> std::result::Result<RecipeOutput, String> {
        assert!(access.allowed());
        assert!(access.remaining() <= Duration::from_secs(10));
        assert!(plan.target == target());
        assert_eq!(plan.template.name, "check");
        assert_eq!(
            plan.template.render_args(&plan.params).unwrap(),
            vec!["approved"]
        );
        let debug = format!("{plan:?}");
        assert!(!debug.contains(SECRET));
        assert!(!debug.contains("approved"));
        assert!(!debug.contains("pinned-recipe"));
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.fail {
            return Err("private adapter failure".into());
        }
        if let Some(authority) = &self.revoke {
            authority.allowed.store(false, Ordering::Release);
        }
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("account:{}", plan.secret.as_str()));
        let result = plan.redact_output(RecipeOutput {
            exit_code: Some(0),
            signal: None,
            stdout: format!("Basic {encoded}"),
            stderr: plan.secret.to_string(),
            truncated: false,
            timed_out: false,
        })?;
        assert_eq!(result.stdout, "Basic [REDACTED]");
        assert_eq!(result.stderr, "[REDACTED]");
        Ok(result)
    }
}
fn remote() -> Remote {
    Remote {
        calls: AtomicUsize::new(0),
        revoke: None,
        fail: false,
    }
}
#[test]
fn remote_target_has_no_local_path_and_binds_every_live_stamp_to_a_separate_digest_domain() {
    let t = target();
    assert!(t.binding() == &binding());
    assert_eq!(t.channel_stamp(), "a".repeat(64));
    assert_eq!(t.turn_stamp(), "b".repeat(64));
    assert_eq!(t.export_id(), "export");
    assert_eq!(t.export_revision(), 1);
    assert_eq!(t.checkout_stamp(), "c".repeat(64));
    let remote = RecipeTarget::from_remote(t.clone());
    assert!(remote.local_cwd().is_none());
    assert!(remote.remote_scope().is_some());
    let local = RecipeTarget::new(binding(), "/workspace".into(), "c".repeat(64)).unwrap();
    let legacy=digest(&serde_json::to_vec(&serde_json::json!({"domain":"dunlin.scoped-recipe.v1","input":input(),"binding":binding(),"cwd":"/workspace","checkout_stamp":"c".repeat(64)})).unwrap());
    assert_eq!(local.input_digest(&input()).unwrap(), legacy);
    assert!(local.remote_scope().is_none());
    assert_ne!(remote.input_digest(&input()).unwrap(), legacy);
    for (channel, turn, export, revision, checkout) in [
        ("d".repeat(64), "b".repeat(64), "export", 1, "c".repeat(64)),
        ("a".repeat(64), "d".repeat(64), "export", 1, "c".repeat(64)),
        ("a".repeat(64), "b".repeat(64), "other", 1, "c".repeat(64)),
        ("a".repeat(64), "b".repeat(64), "export", 2, "c".repeat(64)),
        ("a".repeat(64), "b".repeat(64), "export", 1, "d".repeat(64)),
    ] {
        let changed = RecipeTarget::from_remote(
            RemoteRecipeTarget::new(binding(), channel, turn, export.into(), revision, checkout)
                .unwrap(),
        );
        assert_ne!(
            changed.input_digest(&input()).unwrap(),
            remote.input_digest(&input()).unwrap()
        );
    }
    let mut local_binding = binding();
    local_binding.host = "local".into();
    assert!(
        RemoteRecipeTarget::new(
            local_binding,
            "a".repeat(64),
            "b".repeat(64),
            "export".into(),
            1,
            "c".repeat(64)
        )
        .is_err()
    );
    for revision in [0, 9_007_199_254_740_992] {
        assert!(
            RemoteRecipeTarget::new(
                binding(),
                "a".repeat(64),
                "b".repeat(64),
                "export".into(),
                revision,
                "c".repeat(64)
            )
            .is_err()
        );
    }
}
#[test]
fn executor_kind_mismatch_cannot_fall_back_or_read_a_credential() {
    let local = Fixture::new("#!/bin/sh\nexit 0\n", true);
    let peer = remote();
    assert_eq!(
        local
            .credentials
            .recipe_remote(local.permit().unwrap(), input(), &peer),
        Err(Error::Denied)
    );
    assert_eq!(local.secrets.reads.load(Ordering::Acquire), 0);
    assert_eq!(peer.calls.load(Ordering::Acquire), 0);
    let f = fixture();
    let execute = executor();
    assert_eq!(
        f.credentials.recipe(f.permit().unwrap(), input(), &execute),
        Err(Error::Denied)
    );
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    assert_eq!(execute.calls.load(Ordering::Acquire), 0);
}
#[test]
fn remote_dispatch_is_one_use_guarded_and_redacted_before_and_after_the_adapter() {
    let f = fixture();
    // Only the runner's own image matters. The controller-local source path
    // need not exist after the pinned credential policy has been installed.
    std::fs::remove_file(f._program.path().join("pinned-recipe")).unwrap();
    let peer = remote();
    let output = f
        .credentials
        .recipe_remote(f.permit().unwrap(), input(), &peer)
        .unwrap();
    assert_eq!(output.stdout, "Basic [REDACTED]");
    assert_eq!(output.stderr, "[REDACTED]");
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(peer.calls.load(Ordering::Acquire), 1);
    assert!(matches!(f.permit(), Err(Error::Conflict)));
    assert_eq!(
        *f.authority.outcomes.lock().unwrap(),
        vec![Completion::Complete]
    );
}
#[test]
fn remote_revocation_after_keyring_read_never_dispatches_and_inflight_revoke_or_lost_ack_is_unknown()
 {
    let f = fixture();
    let authority = f.authority.clone();
    *f.secrets.after_read.lock().unwrap() = Some(Box::new(move || {
        authority.allowed.store(false, Ordering::Release)
    }));
    let peer = remote();
    assert_eq!(
        f.credentials
            .recipe_remote(f.permit().unwrap(), input(), &peer),
        Err(Error::Denied)
    );
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
    assert_eq!(peer.calls.load(Ordering::Acquire), 0);
    for (revoke, fail_ack, fail_adapter) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let f = fixture();
        f.authority.fail_ack.store(fail_ack, Ordering::Release);
        let peer = Remote {
            revoke: revoke.then(|| f.authority.clone()),
            fail: fail_adapter,
            ..remote()
        };
        assert_eq!(
            f.credentials
                .recipe_remote(f.permit().unwrap(), input(), &peer),
            Err(Error::OutcomeUnknown)
        );
        assert_eq!(peer.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.secrets.reads.load(Ordering::Acquire), 1);
        assert!(f.permit().is_err());
    }
}

#[test]
fn changed_remote_binding_or_fence_never_obtains_a_permit_or_reads_keyring() {
    let f = fixture();
    for field in ["workspace", "project", "host"] {
        let mut b = binding();
        match field {
            "workspace" => b.workspace = "other".into(),
            "project" => b.project = "other".into(),
            _ => b.host = "other".into(),
        }
        let changed = RecipeTarget::from_remote(
            RemoteRecipeTarget::new(
                b,
                "a".repeat(64),
                "b".repeat(64),
                "export".into(),
                1,
                "c".repeat(64),
            )
            .unwrap(),
        );
        assert!(matches!(
            f.credentials.permit_recipe(
                f.authority.scope.clone(),
                f.authority.version.clone(),
                &input(),
                changed,
                Instant::now() + Duration::from_secs(10),
                f.authority.clone()
            ),
            Err(Error::Denied)
        ));
    }
    let changed = RecipeTarget::from_remote(
        RemoteRecipeTarget::new(
            binding(),
            "d".repeat(64),
            "b".repeat(64),
            "export".into(),
            1,
            "c".repeat(64),
        )
        .unwrap(),
    );
    assert!(matches!(
        f.credentials.permit_recipe(
            f.authority.scope.clone(),
            f.authority.version.clone(),
            &input(),
            changed,
            Instant::now() + Duration::from_secs(10),
            f.authority.clone()
        ),
        Err(Error::Denied)
    ));
    assert_eq!(f.secrets.reads.load(Ordering::Acquire), 0);
    assert!(f.authority.used.lock().unwrap().is_empty());
    assert!(f.permit().is_ok());
}
