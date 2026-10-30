//! Account limits for the active environment/harness/provider, never ACP context
//! percentages or another CLI's account. No credentials are read here.
use dunlin_domain::{ProjectTone, ProviderUsage, UsageSource, UsageWindow};
use dunlin_ui::Context;
use serde_json::Value;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{app::DunlinApp, providers};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Scope {
    pub location: String,
    pub remote: bool,
    pub harness: String,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Default)]
pub(crate) struct Cache {
    entries: Vec<(Scope, Vec<ProviderUsage>, Instant)>,
}

impl Cache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
    pub(crate) fn get(&self, scope: &Scope) -> Option<Vec<ProviderUsage>> {
        self.entries
            .iter()
            .find(|(key, _, _)| key == scope)
            .map(|(_, rows, _)| rows.clone())
    }

    pub(crate) fn fresh(&self, scope: &Scope) -> bool {
        self.entries
            .iter()
            .any(|(key, _, at)| key == scope && at.elapsed() < Duration::from_secs(60))
    }

    pub(crate) fn record(&mut self, scope: Scope, mut rows: Vec<ProviderUsage>) {
        if rows.iter().all(|row| row.windows.is_empty())
            && rows.iter().any(|row| row.plan == "Usage request failed")
            && let Some(previous) = self.get(&scope)
            && previous.iter().any(|row| !row.windows.is_empty())
        {
            let reason = rows
                .first()
                .map(|row| row.detail.clone())
                .unwrap_or_default();
            rows = previous;
            for row in &mut rows {
                if !row.plan.ends_with(" · cached") {
                    row.plan.push_str(" · cached");
                }
                row.detail =
                    format!("Last available limits; refresh did not return current data. {reason}");
            }
        }
        self.entries.retain(|(key, _, _)| key != &scope);
        self.entries.push((scope, rows, Instant::now()));
        if self.entries.len() > 24 {
            self.entries.remove(0);
        }
    }
}

fn model_scope(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    if provider == Some(providers::CONSOLE) {
        model.map(str::to_owned)
    } else {
        None
    }
}

impl Scope {
    fn label(&self) -> String {
        if self.harness == "OpenCode" {
            self.provider
                .as_deref()
                .map(providers::provider_name)
                .unwrap_or("OpenCode")
                .into()
        } else {
            self.harness.clone()
        }
    }

    pub(crate) fn unavailable(&self, state: &str, detail: &str) -> ProviderUsage {
        ProviderUsage {
            provider: self.label(),
            plan: state.into(),
            accent: ProjectTone::Teal,
            windows: Vec::new(),
            detail: detail.into(),
        }
    }
}

/// Old relays return a list for all CLIs. Do not display their OpenCode estimate
/// or mistake a provider's account for the similarly named standalone harness.
pub(crate) fn select(scope: &Scope, usages: &[ProviderUsage]) -> Vec<ProviderUsage> {
    let expected = match scope.harness.as_str() {
        "Claude Code" => Some("Claude Code"),
        "OpenAI Codex" => Some("Codex"),
        _ => None,
    };
    let selected = usages.iter().find(|usage| {
        expected == Some(usage.provider.as_str())
            && !usage.windows.iter().any(|window| {
                matches!(window.source, UsageSource::Estimated | UsageSource::Session)
            })
    });
    vec![selected.cloned().unwrap_or_else(|| {
        scope.unavailable(
            "Limits unavailable",
            "Account limits are not exposed for this provider/harness in this environment.",
        )
    })]
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.)
}

fn reset_label(epoch: f64, now: f64) -> String {
    let minutes = ((epoch - now).max(0.) / 60_000.).ceil() as u64;
    if minutes == 0 {
        "now".into()
    } else if minutes >= 1440 {
        format!("{}d {}h", minutes / 1440, minutes % 1440 / 60)
    } else if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

fn window(label: String, percent: f64, reset: f64, now: f64) -> Option<UsageWindow> {
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) || !reset.is_finite() {
        return None;
    }
    Some(UsageWindow {
        label,
        used_percent: percent as f32,
        resets_in: reset_label(reset, now),
        source: UsageSource::Live,
    })
}

fn opencode_limits(scope: &Scope, response: &Value, now: f64) -> ProviderUsage {
    let mut usage = scope.unavailable("Limits unavailable", "OpenCode has not exposed account limits for the selected provider. Context tokens are shown separately in the status bar.");
    if scope.provider.as_deref() == Some(providers::FREE) {
        usage.detail = "Free models · no free-tier account allowance is exposed. Paid Go and Console budgets do not apply to this filter.".into();
        return usage;
    }
    let Some(data) = response.get("data").filter(|data| data.is_object()) else {
        return usage;
    };
    match scope.provider.as_deref() {
        Some(providers::GO) => {
            if let Some(go) = data.get("go") {
                for (key, label) in [
                    ("rolling", "Rolling"),
                    ("weekly", "Weekly"),
                    ("monthly", "Monthly"),
                ] {
                    let meter = &go[key];
                    if let (Some(percent), Some(reset)) =
                        (meter["percent"].as_f64(), meter["resetsAt"].as_f64())
                        && let Some(window) = window(label.into(), percent, reset, now)
                    {
                        usage.windows.push(window);
                    }
                }
                if !usage.windows.is_empty() {
                    usage.plan = "Go".into();
                    usage.detail =
                        "Live Go allowances from this project's OpenCode Console workspace.".into();
                }
            }
        }
        Some(providers::CONSOLE) => {
            for budget in data["budgets"].as_array().into_iter().flatten() {
                // Restrictive model/vendor policies cannot be attributed without
                // the matching model. Never relabel an unrelated budget.
                let model = budget["models"].as_array().and_then(|models| {
                    models.iter().find(|model| {
                        model["providerID"].as_str() == Some(providers::CONSOLE)
                            && scope
                                .model
                                .as_deref()
                                .is_some_and(|id| model["modelID"].as_str() == Some(id))
                    })
                });
                if budget["coverage"].as_str() != Some("all") && model.is_none() {
                    continue;
                }
                if budget["vendors"]
                    .as_array()
                    .is_some_and(|vendors| !vendors.is_empty())
                    || budget.get("inputPriceAtLeast").is_some()
                {
                    continue;
                }
                let spent = if budget["per"].as_str() == Some("model") {
                    model.and_then(|model| model["spent"].as_f64())
                } else {
                    budget["spent"].as_f64()
                };
                if let (Some(limit), Some(spent), Some(reset)) =
                    (budget["limit"].as_f64(), spent, budget["resetsAt"].as_f64())
                {
                    if limit < 0. || spent < 0. {
                        continue;
                    }
                    let label = format!(
                        "{} budget · ${spent:.2} / ${limit:.2}",
                        if budget["shared"].as_bool() == Some(true) {
                            "Shared"
                        } else {
                            "Account"
                        }
                    );
                    // A $0 budget blocks spending, it is not a missing limit.
                    let percent = if limit == 0. {
                        100.
                    } else {
                        (spent / limit * 100.).min(100.)
                    };
                    if let Some(window) = window(label, percent, reset, now) {
                        usage.windows.push(window);
                    }
                }
            }
            if !usage.windows.is_empty() {
                usage.plan = "Console".into();
                usage.detail = "Live Console budgets applicable to this selection. Shared budgets include other workspace usage; they are not a subscription allowance.".into();
            }
        }
        _ => {}
    }
    usage
}

impl DunlinApp {
    pub(crate) fn usage_target(&self) -> Scope {
        let harness = self.session_harness();
        let remote = self.project().is_remote();
        let current = self
            .harness
            .models
            .as_ref()
            .map(|models| models.current.as_str())
            .or_else(|| (self.session().model != "Auto").then_some(self.session().model.as_str()));
        let provider = if harness == "OpenCode" {
            if remote {
                current.and_then(providers::provider_id)
            } else {
                self.selected_provider()
            }
            .map(str::to_owned)
        } else {
            None
        };
        let model = if harness == "OpenCode" {
            current
                .filter(|model| {
                    providers::provider_id(model)
                        == provider.as_deref().map(providers::account_provider)
                })
                .and_then(|model| {
                    model
                        .split_once('/')
                        .map(|(_, model)| model.split('#').next().unwrap_or(model).to_owned())
                })
        } else {
            None
        };
        let model = model_scope(provider.as_deref(), model.as_deref());
        Scope {
            location: self
                .project()
                .remote
                .as_ref()
                .map(|r| format!("{}:{}", r.host_id, r.project_id))
                .unwrap_or_else(|| self.project().path.clone()),
            remote,
            harness,
            provider,
            model,
        }
    }

    pub(crate) fn ensure_usage(&mut self, cx: &mut Context<Self>) {
        let scope = self.usage_target();
        if !self.usage_cache.fresh(&scope) && self.usage_request.as_ref() != Some(&scope) {
            self.refresh_usage(cx);
        }
    }

    pub(crate) fn active_usages(&self) -> Vec<ProviderUsage> {
        let scope = self.usage_target();
        if let Some(rows) = self.usage_cache.get(&scope) {
            return rows;
        }
        if self.usage_scope.as_ref() != Some(&scope) {
            return vec![scope.unavailable(
                "Loading…",
                "Reading account limits for this environment, harness and provider.",
            )];
        }
        if let Some(remote) = &self.project().remote {
            return select(
                &scope,
                self.remote_usages
                    .get(&remote.host_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            );
        }
        self.usages.clone()
    }
}

pub(crate) fn probe(scope: &Scope) -> Vec<ProviderUsage> {
    if scope.remote {
        return vec![scope.unavailable(
            "Limits unavailable",
            "Read account limits on the relay, never from this Mac's credentials.",
        )];
    }
    if scope.harness != "OpenCode" {
        return vec![dunlin_usage::probe_harness(&scope.harness)];
    }
    if scope.provider.as_deref() == Some(providers::FREE) {
        return vec![opencode_limits(scope, &Value::Null, now_ms())];
    }
    if !matches!(
        scope.provider.as_deref(),
        Some(providers::GO | providers::CONSOLE | providers::FREE)
    ) {
        return vec![scope.unavailable("Limits unavailable", "This OpenCode provider does not expose account limits. Its account is not the standalone Claude Code or Codex account.")];
    }
    let provider_id = scope.provider.as_deref().unwrap();
    let account_response = providers::api(
        &scope.location,
        &format!("provider/{}", providers::encode(provider_id)),
    );
    let response = match account_response {
        Ok(response) if response["data"].is_object() => response,
        _ => {
            return vec![scope.unavailable(
                "Usage request failed",
                "Could not verify the current OpenCode account integration. Refresh to retry.",
            )];
        }
    };
    let console_account = console_backed(provider_id, &response);
    if !console_account {
        return vec![scope.unavailable("Limits unavailable", "This provider is not using the OpenCode Console account integration. Console account limits would not describe its account.")];
    }
    match providers::api(&scope.location, "experimental/usage/limits") {
        Ok(response) => vec![opencode_limits(scope, &response, now_ms())],
        Err(_) => {
            let identity = providers::api(&scope.location, "info").unwrap_or(Value::Null);
            vec![limits_failure(scope, &identity)]
        }
    }
}

fn limits_failure(scope: &Scope, identity: &Value) -> ProviderUsage {
    if let Some(version) = identity["version"].as_str()
        && let Some((major, minor)) = version.trim_start_matches('v').split_once('.')
        && let (Ok(major), Some(minor)) = (
            major.parse::<u64>(),
            minor.split('.').next().and_then(|v| v.parse::<u64>().ok()),
        )
        && (major, minor) < (2, 2)
    {
        return scope.unavailable("Service update needed", &format!("Running OpenCode {version}; usage limits require 2.2+. After updating, finish active conversations, restart the OpenCode service and refresh. You can view usage in Console now."));
    }
    scope.unavailable("Usage request failed", "Could not read account usage from OpenCode. Refresh to retry, or view usage in Console. No percentage is estimated.")
}

fn console_backed(provider: &str, response: &Value) -> bool {
    let data = &response["data"];
    data["id"].as_str() == Some(provider)
        && data["integrationID"].as_str() == Some(providers::CONSOLE)
        && data["activation"].as_str() != Some("disabled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope(harness: &str, provider: Option<&str>) -> Scope {
        Scope {
            location: "/project".into(),
            remote: false,
            harness: harness.into(),
            provider: provider.map(str::to_owned),
            model: None,
        }
    }

    fn account(name: &str, source: UsageSource) -> ProviderUsage {
        ProviderUsage {
            provider: name.into(),
            plan: "Test".into(),
            accent: ProjectTone::Teal,
            detail: String::new(),
            windows: vec![UsageWindow {
                label: "Limit".into(),
                used_percent: 42.,
                resets_in: "1h".into(),
                source,
            }],
        }
    }

    #[test]
    fn legacy_relay_rows_are_filtered_by_harness_not_provider_brand() {
        let rows = vec![
            account("Claude Code", UsageSource::Live),
            account("Codex", UsageSource::Live),
            account("OpenCode", UsageSource::Estimated),
        ];
        assert_eq!(
            select(&scope("Claude Code", None), &rows)[0].provider,
            "Claude Code"
        );
        assert_eq!(
            select(&scope("OpenAI Codex", None), &rows)[0].provider,
            "Codex"
        );
        for provider in [providers::GO, providers::FREE, "openai", "anthropic"] {
            let usage = select(&scope("OpenCode", Some(provider)), &rows);
            assert!(usage[0].windows.is_empty());
            assert_eq!(usage[0].provider, providers::provider_name(provider));
        }
        assert!(
            select(&scope("Custom ACP", None), &rows)[0]
                .windows
                .is_empty()
        );
        assert!(
            select(
                &scope("OpenAI Codex", None),
                &[account("Codex", UsageSource::Session)]
            )[0]
            .windows
            .is_empty()
        );
    }

    #[test]
    fn free_and_unavailable_limits_never_borrow_paid_windows_or_estimate_a_percentage() {
        let response =
            json!({"data":{"go":{"weekly":{"percent":42,"resetsAt":3_600_000}},"budgets":[]}});
        let free = opencode_limits(&scope("OpenCode", Some(providers::FREE)), &response, 0.);
        assert!(free.windows.is_empty());
        assert_eq!(free.provider, "OpenCode Free");
        assert!(free.detail.contains("Free models"));
        for response in [
            json!({"data":null}),
            json!({}),
            json!({"data":{"budgets":[]}}),
        ] {
            assert!(
                opencode_limits(&scope("OpenCode", Some(providers::GO)), &response, 0.)
                    .windows
                    .is_empty()
            );
        }
    }

    #[test]
    fn go_uses_each_published_window_and_millisecond_resets() {
        let response = json!({"data":{"go":{
            "rolling":{"percent":12,"resetsAt":3_600_000},
            "weekly":{"percent":42,"resetsAt":86_400_000},
            "monthly":{"percent":100,"resetsAt":172_800_000}
        },"budgets":[{"limit":1,"spent":1}]}});
        let usage = opencode_limits(&scope("OpenCode", Some(providers::GO)), &response, 0.);
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].used_percent, 12.);
        assert_eq!(usage.windows[0].resets_in, "1h 0m");
        assert_eq!(usage.windows[1].resets_in, "1d 0h");
        assert!(
            usage
                .windows
                .iter()
                .all(|window| window.source == UsageSource::Live)
        );
        assert!(window("Invalid".into(), 150., 1., 0.).is_none());
    }

    #[test]
    fn console_model_budgets_require_the_selected_model_and_do_not_use_go_limits() {
        let mut target = scope("OpenCode", Some(providers::CONSOLE));
        target.model = Some("chosen".into());
        let response = json!({"data":{"go":{"weekly":{"percent":99,"resetsAt":1}},"budgets":[
            {"coverage":"models","per":"model","limit":10,"spent":9,"shared":false,"resetsAt":3_600_000,"models":[{"providerID":"opencode","modelID":"chosen","spent":2}]},
            {"coverage":"models","per":"set","limit":10,"spent":8,"shared":false,"resetsAt":3_600_000,"models":[{"providerID":"opencode-go","modelID":"chosen"}]},
            {"coverage":"all","per":"set","limit":0,"spent":0,"shared":true,"resetsAt":3_600_000,"models":[]},
            {"coverage":"all","per":"set","limit":10,"spent":9,"resetsAt":3_600_000,"vendors":["different-vendor"]}
        ]}});
        let usage = opencode_limits(&target, &response, 0.);
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_percent, 20.);
        assert!(usage.windows[0].label.contains("$2.00 / $10.00"));
        assert_eq!(usage.windows[1].used_percent, 100.);
        assert!(usage.windows[1].label.starts_with("Shared budget"));
        target.model = None;
        assert_eq!(opencode_limits(&target, &response, 0.).windows.len(), 1);
    }

    #[test]
    fn scopes_isolate_provider_harness_environment_and_model() {
        let target = scope("OpenCode", Some(providers::GO));
        assert!(target != scope("OpenCode", Some(providers::FREE)));
        assert!(target != scope("Claude Code", None));
        let mut other = target.clone();
        other.location = "/other".into();
        assert!(target != other);
        other = target.clone();
        other.remote = true;
        assert!(target != other);
        other = target.clone();
        other.model = Some("different".into());
        assert!(target != other);
    }

    #[test]
    fn go_limits_do_not_change_scope_while_session_models_reload() {
        for model in [None, Some("first"), Some("second")] {
            assert_eq!(model_scope(Some(providers::GO), model), None);
        }
        assert_eq!(
            model_scope(Some(providers::CONSOLE), Some("first")),
            Some("first".into())
        );
    }

    #[test]
    fn usage_cache_survives_switches_and_transient_errors_without_crossing_accounts() {
        let go = scope("OpenCode", Some(providers::GO));
        let free = scope("OpenCode", Some(providers::FREE));
        let mut cache = Cache::default();
        cache.record(go.clone(), vec![account("OpenCode Go", UsageSource::Live)]);
        assert!(cache.fresh(&go));
        assert!(cache.get(&free).is_none());
        let mut other = go.clone();
        other.location = "/other".into();
        assert!(cache.get(&other).is_none());
        cache.record(
            free.clone(),
            vec![free.unavailable("Limits unavailable", "No free allowance")],
        );
        assert_eq!(cache.get(&go).unwrap()[0].windows[0].used_percent, 42.);
        cache.record(
            go.clone(),
            vec![go.unavailable("Usage request failed", "Temporary failure")],
        );
        let cached = cache.get(&go).unwrap();
        assert_eq!(cached[0].windows[0].used_percent, 42.);
        assert!(cached[0].plan.ends_with("cached"));
        cache.record(go.clone(), vec![account("OpenCode Go", UsageSource::Live)]);
        assert!(!cache.get(&go).unwrap()[0].plan.contains("cached"));
        cache.record(
            go.clone(),
            vec![go.unavailable(
                "Limits unavailable",
                "This account no longer exposes limits",
            )],
        );
        assert!(cache.get(&go).unwrap()[0].windows.is_empty());
        cache.clear();
        assert!(cache.get(&go).is_none());
    }

    #[test]
    fn direct_opencode_providers_and_remote_harnesses_do_not_probe_local_cli_accounts() {
        for provider in ["openai", "anthropic", providers::FREE] {
            let usages = probe(&scope("OpenCode", Some(provider)));
            assert_eq!(usages.len(), 1);
            assert!(usages[0].windows.is_empty());
        }
        let mut remote = scope("Claude Code", None);
        remote.remote = true;
        let usage = probe(&remote);
        assert!(usage[0].detail.contains("relay"));
        assert!(usage[0].windows.is_empty());
        assert!(console_backed(
            providers::GO,
            &json!({"data":{"id":providers::GO,"integrationID":providers::CONSOLE}})
        ));
        assert!(!console_backed(
            providers::GO,
            &json!({"data":{"id":providers::GO,"integrationID":providers::GO}})
        ));
        assert!(!console_backed(providers::GO, &json!({"data":null})));
    }

    #[test]
    fn old_service_errors_explain_the_required_update_instead_of_hiding_the_cause() {
        let scope = scope("OpenCode", Some(providers::GO));
        let old = limits_failure(&scope, &json!({"version":"2.1.4"}));
        assert_eq!(old.plan, "Service update needed");
        assert!(old.detail.contains("2.2+"));
        assert!(old.detail.contains("finish active conversations"));
        assert!(old.windows.is_empty());
        let current = limits_failure(&scope, &json!({"version":"2.2.2"}));
        assert_eq!(current.plan, "Usage request failed");
        assert!(current.windows.is_empty());
    }
}
