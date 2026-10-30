use super::{LastTarget, ModelStep, Pending, check_auth, legacy_target};
fn target() -> LastTarget {
    LastTarget {
        harness: "OpenCode".into(),
        model: "opencode-go/model".into(),
        provider: Some("opencode-go".into()),
        remote: false,
    }
}
#[test]
fn exact_model_harness_and_billing_provider_must_all_match() {
    let target = target();
    assert!(target.matches("OpenCode", "opencode-go/model", Some("opencode-go")));
    assert!(!target.matches("OpenCode", "opencode-go/different", Some("opencode-go")));
    assert!(!target.matches(
        "OpenCode",
        "opencode-go/model",
        Some(crate::providers::FREE)
    ));
    assert!(!target.matches("Claude Code", "opencode-go/model", Some("opencode-go")));
}
#[test]
fn legacy_recents_preserve_wire_ids_without_guessing_paid_console_authorization() {
    let target = legacy_target(&crate::providers::RecentModel {
        scope: "local:OpenCode".into(),
        id: "opencode/big-pickle".into(),
    });
    assert_eq!(target.model, "opencode/big-pickle");
    assert_eq!(target.provider.as_deref(), Some(crate::providers::FREE));
    let remote = legacy_target(&crate::providers::RecentModel {
        scope: "remote:host:project:OpenCode".into(),
        id: "opencode-go/model".into(),
    });
    assert!(remote.remote);
    assert!(!check_auth(&remote, "/not-a-repository"));
}
#[test]
fn navigation_or_relocation_cannot_dispatch_a_pending_commit_in_a_different_scope() {
    let pending = Pending {
        target: target(),
        project: "one".into(),
        root: "/one".into(),
        session: "session".into(),
        prompt: "explicit commit request".into(),
        started: std::time::Instant::now(),
        model_requested: false,
    };
    assert!(pending.is_selected("one", "/one", "session"));
    assert!(!pending.is_selected("two", "/one", "session"));
    assert!(!pending.is_selected("one", "/relocated", "session"));
    assert!(!pending.is_selected("one", "/one", "other-session"));
}
#[test]
fn unknown_or_missing_auth_contracts_do_not_run_paid_probes_and_target_settings_roundtrip() {
    let mut target = target();
    target.harness = "Custom ACP".into();
    assert!(!check_auth(&target, "/not-a-repository"));
    target.harness = "OpenCode".into();
    target.model.clear();
    assert!(!check_auth(&target, "/not-a-repository"));
    let settings = crate::app::StoredSettings {
        last_ai_target: Some(target.clone()),
        ..Default::default()
    };
    let restored: crate::app::StoredSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(restored.last_ai_target, Some(target));
}

#[test]
fn mock_model_selection_waits_for_confirmation_and_never_accepts_a_replacement() {
    let mut pending = Pending {
        target: target(),
        project: "one".into(),
        root: "/one".into(),
        session: "commit".into(),
        prompt: "explicit commit request".into(),
        started: std::time::Instant::now(),
        model_requested: false,
    };
    assert_eq!(
        pending.model_step("provider/default", true),
        ModelStep::Select
    );
    pending.model_requested = true;
    assert_eq!(
        pending.model_step("provider/default", true),
        ModelStep::Wait
    );
    assert_eq!(
        pending.model_step("opencode-go/model", true),
        ModelStep::Ready
    );
    assert_eq!(
        pending.model_step("provider/replacement", false),
        ModelStep::Unavailable
    );
    assert_eq!(
        pending.model_step("opencode-go/model", false),
        ModelStep::Unavailable
    );
}

#[test]
fn legacy_settings_gain_the_skill_without_replacing_existing_preferences_or_empty_libraries() {
    let settings: crate::app::StoredSettings =
        serde_json::from_str(r#"{"default_harness":"Claude Code","autosave":true}"#).unwrap();
    assert_eq!(settings.default_harness, "Claude Code");
    assert!(settings.autosave);
    assert_eq!(settings.skills[0].id, crate::skills::COMMIT_SKILL);
    assert!(settings.last_ai_target.is_none());
    let settings: crate::app::StoredSettings = serde_json::from_str(r#"{"skills":[]}"#).unwrap();
    assert!(settings.skills.is_empty());
}
