//! Provider usage probes.
//!
//! Dunlin keeps the UI responsive by running every probe on the background
//! executor. Codex exposes a local JSON-RPC app-server with real rate-limit
//! windows; Claude Code keeps its OAuth token in the macOS keychain but only
//! grants usage scopes after a re-login, so Dunlin reports that state instead
//! of inventing percentages. ACP context usage is not a subscription allowance.

pub mod billing;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::DateTime;

use dunlin_domain::{ProjectTone, ProviderUsage, UsageSource, UsageWindow};
use dunlin_http as ureq;
use serde_json::{Value, json};

/// Probe every provider that can be read locally.
pub fn probe_all() -> Vec<ProviderUsage> {
    vec![probe_claude(), probe_codex()]
}

/// Probe only the active harness. OpenCode provider limits are location-scoped
/// and are read by the client through OpenCode's public API, not other CLIs.
pub fn probe_harness(harness: &str) -> ProviderUsage {
    match harness {
        "Claude Code" => probe_claude(),
        "OpenAI Codex" => probe_codex(),
        _ => unavailable(
            harness,
            "Limits unavailable",
            "This harness does not expose account usage limits.",
            ProjectTone::Teal,
        ),
    }
}

fn probe_codex() -> ProviderUsage {
    let Some(binary) = codex_binary() else {
        return unavailable(
            "Codex",
            "not detected",
            "Install the Codex CLI to read live subscription windows.",
            ProjectTone::Blue,
        );
    };
    match codex_rate_limits(&binary) {
        Ok(limits) => limits.into_usage(),
        Err(error) => unavailable("Codex", "unavailable", &error, ProjectTone::Blue),
    }
}

fn probe_claude() -> ProviderUsage {
    let credentials = match claude_credentials() {
        Ok(credentials) => credentials,
        Err(error) => {
            return unavailable("Claude Code", "not signed in", &error, ProjectTone::Violet);
        }
    };
    if !credentials
        .scopes
        .iter()
        .any(|scope| scope == "user:profile")
    {
        return unavailable(
            "Claude Code",
            "re-auth needed",
            "Run `claude /login` to grant `user:profile`, then refresh usage.",
            ProjectTone::Violet,
        );
    }
    if credentials
        .expires_at
        .is_some_and(|expires| expires <= now_ms() + 30_000)
    {
        return unavailable(
            "Claude Code",
            "re-auth needed",
            "The Claude Code OAuth token expired. Run `claude /login` and refresh usage.",
            ProjectTone::Violet,
        );
    }
    match claude_usage(&credentials.access_token) {
        Ok(usage) => usage,
        Err(error) => unavailable("Claude Code", "unavailable", &error, ProjectTone::Violet),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaudeCredentials {
    access_token: String,
    scopes: Vec<String>,
    expires_at: Option<i64>,
}

fn parse_claude_credentials(raw: &str) -> Result<ClaudeCredentials, String> {
    let value: Value = serde_json::from_str(raw.trim())
        .map_err(|_| "Claude Code credentials are not readable".to_string())?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or_else(|| "Claude Code credentials have no OAuth section".to_string())?;
    let access_token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "Claude Code OAuth access token is missing".to_string())?
        .to_string();
    Ok(ClaudeCredentials {
        access_token,
        scopes: oauth
            .get("scopes")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        expires_at: oauth.get("expiresAt").and_then(Value::as_i64),
    })
}

fn claude_usage(token: &str) -> Result<ProviderUsage, String> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build()
        .get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("anthropic-version", "2023-06-01")
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(401 | 403, _) => {
                "Claude Code rejected the OAuth token; run `claude /login` and refresh usage."
                    .to_string()
            }
            ureq::Error::Status(code, _) => {
                format!("Claude usage request failed (HTTP {code}); no estimate was invented.")
            }
            ureq::Error::Transport(_) => {
                "Could not reach Claude usage (network error or timeout).".to_string()
            }
        })?;
    let value: Value = response
        .into_json()
        .map_err(|_| "Claude usage returned an invalid response".to_string())?;
    parse_claude_usage(&value)
}

fn parse_claude_usage(value: &Value) -> Result<ProviderUsage, String> {
    let mut windows = Vec::new();
    for (key, label) in [
        ("five_hour", "5 hour"),
        ("seven_day", "Weekly"),
        ("seven_day_opus", "Weekly Opus"),
        ("seven_day_sonnet", "Weekly Sonnet"),
    ] {
        let Some(window) = value.get(key) else {
            continue;
        };
        let Some(used) = window.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at = window
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp());
        windows.push(UsageWindow {
            label: label.into(),
            used_percent: used.clamp(0., 100.) as f32,
            resets_in: resets_in(resets_at),
            source: UsageSource::Live,
        });
    }
    if windows.is_empty() {
        return Err("Claude usage returned no limit windows".into());
    }
    Ok(ProviderUsage {
        provider: "Claude Code".into(),
        plan: "CLAUDE".into(),
        accent: ProjectTone::Violet,
        windows,
        detail: "Live from Claude Code OAuth usage · refreshed just now".into(),
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn unavailable(provider: &str, plan: &str, detail: &str, accent: ProjectTone) -> ProviderUsage {
    ProviderUsage {
        provider: provider.into(),
        plan: plan.into(),
        accent,
        windows: Vec::new(),
        detail: detail.into(),
    }
}

fn codex_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DUNLIN_CODEX_BIN")
        && Path::new(&path).is_file()
    {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = path_lookup("codex") {
        return Some(path);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    for candidate in [
        home.join(".local/bin/codex"),
        home.join(".cargo/bin/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Zed installs ACP adapters in a versioned registry. Prefer that copy when
    // the CLI is not on PATH so Codex usage still works out of the box.
    let base = home.join(
        "Library/Application Support/Zed/external_agents/registry/npx/codex-acp/node_modules/@openai",
    );
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("codex-darwin") {
            continue;
        }
        let vendor = entry.path().join("vendor");
        let Ok(platforms) = std::fs::read_dir(vendor) else {
            continue;
        };
        for platform in platforms.flatten() {
            let candidate = platform.path().join("bin/codex");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn path_lookup(executable: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

#[derive(Clone, Debug)]
struct CodexLimits {
    plan: Option<String>,
    primary: Option<CodexWindow>,
    secondary: Option<CodexWindow>,
}

#[derive(Clone, Debug)]
struct CodexWindow {
    used_percent: f32,
    duration_mins: Option<i64>,
    resets_at: Option<i64>,
}

impl CodexLimits {
    fn into_usage(self) -> ProviderUsage {
        let plan = self
            .plan
            .as_deref()
            .map(plan_label)
            .unwrap_or_else(|| "LIVE".into());
        let mut windows = Vec::new();
        if let Some(window) = self.primary {
            windows.push(window.into_usage("primary"));
        }
        if let Some(window) = self.secondary {
            windows.push(window.into_usage("secondary"));
        }
        ProviderUsage {
            provider: "Codex".into(),
            plan,
            accent: ProjectTone::Blue,
            windows,
            detail: "Live from the local Codex app-server · refreshed just now".into(),
        }
    }
}

impl CodexWindow {
    fn into_usage(self, fallback: &str) -> UsageWindow {
        UsageWindow {
            label: window_label(self.duration_mins, fallback),
            used_percent: self.used_percent,
            resets_in: resets_in(self.resets_at),
            source: UsageSource::Live,
        }
    }
}

fn codex_rate_limits(binary: &Path) -> Result<CodexLimits, String> {
    let mut child = Command::new(binary)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start Codex: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex stdin is unavailable".to_string())?;
    let initialize = json!({
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "dunlin", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    let read = json!({ "id": 2, "method": "account/rateLimits/read", "params": null });
    writeln!(stdin, "{initialize}").map_err(|error| error.to_string())?;
    writeln!(stdin, "{read}").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex stdout is unavailable".to_string())?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(6);
    let mut response = None;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(remaining) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_i64) == Some(2) {
                    response = Some(value);
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    drop(stdin);

    let response = response.ok_or_else(|| "Codex did not answer within 6 seconds".to_string())?;
    let result = response
        .get("result")
        .ok_or_else(|| "Codex returned no rate-limit result".to_string())?;
    let limits = result
        .get("rateLimits")
        .ok_or_else(|| "Codex returned no rate-limit windows".to_string())?;
    Ok(CodexLimits {
        plan: limits
            .get("planType")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary: parse_codex_window(limits.get("primary")),
        secondary: parse_codex_window(limits.get("secondary")),
    })
}

fn parse_codex_window(value: Option<&Value>) -> Option<CodexWindow> {
    let value = value?;
    Some(CodexWindow {
        used_percent: value.get("usedPercent")?.as_f64()? as f32,
        duration_mins: value.get("windowDurationMins").and_then(Value::as_i64),
        resets_at: value.get("resetsAt").and_then(Value::as_i64),
    })
}

fn claude_credentials() -> Result<ClaudeCredentials, String> {
    let raw = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .or_else(|| {
            let home = std::env::var_os("HOME")?;
            std::fs::read_to_string(PathBuf::from(home).join(".claude/.credentials.json")).ok()
        })
        .ok_or_else(|| "No Claude Code credentials were found".to_string())?;

    parse_claude_credentials(&raw)
}

fn plan_label(plan: &str) -> String {
    match plan {
        "prolite" => "PRO Lite".into(),
        "pro" => "PRO".into(),
        "plus" => "PLUS".into(),
        "free" => "FREE".into(),
        "go" => "GO".into(),
        "team" => "TEAM".into(),
        "business" => "BUSINESS".into(),
        "enterprise" => "ENTERPRISE".into(),
        other => other.to_uppercase(),
    }
}

fn window_label(duration_mins: Option<i64>, fallback: &str) -> String {
    match duration_mins {
        Some(300) => "5 hour".into(),
        Some(10_080) => "Weekly".into(),
        Some(mins) if mins % 1440 == 0 => format!("{} day", mins / 1440),
        Some(mins) if mins % 60 == 0 => format!("{} hour", mins / 60),
        Some(mins) => format!("{mins} min"),
        None => fallback.to_string(),
    }
}

fn resets_in(epoch_seconds: Option<i64>) -> String {
    let Some(reset) = epoch_seconds else {
        return "unknown".into();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    if reset <= now {
        return "now".into();
    }
    format_remaining(reset - now)
}

fn format_remaining(seconds: i64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_remaining, plan_label, window_label};
    use dunlin_domain::UsageSource;

    #[test]
    fn formats_rate_limit_windows() {
        assert_eq!(window_label(Some(300), "primary"), "5 hour");
        assert_eq!(window_label(Some(10_080), "primary"), "Weekly");
        assert_eq!(window_label(Some(1_440), "primary"), "1 day");
        assert_eq!(window_label(None, "secondary"), "secondary");
    }

    #[test]
    fn formats_plan_names() {
        assert_eq!(plan_label("prolite"), "PRO Lite");
        assert_eq!(plan_label("pro"), "PRO");
        assert_eq!(plan_label("custom"), "CUSTOM");
    }

    #[test]
    fn formats_remaining_time() {
        assert_eq!(format_remaining(90), "1m");
        assert_eq!(format_remaining(3_600 + 120), "1h 02m");
        assert_eq!(format_remaining(86_400 + 3_600), "1d 1h");
    }

    #[test]
    fn unknown_and_opencode_harnesses_have_no_invented_allowance() {
        for harness in ["OpenCode", "Custom ACP"] {
            let usage = super::probe_harness(harness);
            assert_eq!(usage.provider, harness);
            assert!(usage.windows.is_empty());
            assert_eq!(usage.plan, "Limits unavailable");
        }
    }

    #[test]
    fn claude_oauth_credentials_and_usage_are_parsed_without_inventing_windows() {
        let credentials = super::parse_claude_credentials(
            r#"{"claudeAiOauth":{"accessToken":"secret","expiresAt":9999999999999,"scopes":["user:profile","user:inference"]}}"#,
        )
        .unwrap();
        assert_eq!(credentials.access_token, "secret");
        assert!(credentials.scopes.iter().any(|s| s == "user:profile"));
        assert_eq!(credentials.expires_at, Some(9_999_999_999_999));
        assert!(super::parse_claude_credentials("{}").is_err());

        let usage = super::parse_claude_usage(&serde_json::json!({
            "five_hour": {"utilization": 12.5, "resets_at": "2030-01-01T00:00:00Z"},
            "seven_day": {"utilization": 44.0, "resets_at": "2030-01-02T00:00:00Z"},
            "seven_day_opus": {"utilization": 1.5, "resets_at": "2030-01-03T00:00:00Z"},
            "seven_day_sonnet": {"utilization": 99.0}
        }))
        .unwrap();
        assert_eq!(usage.provider, "Claude Code");
        assert_eq!(usage.windows.len(), 4);
        assert_eq!(usage.windows[0].label, "5 hour");
        assert_eq!(usage.windows[0].used_percent, 12.5);
        assert_eq!(usage.windows[0].source, UsageSource::Live);
        assert_eq!(usage.windows[3].label, "Weekly Sonnet");
        assert!(super::parse_claude_usage(&serde_json::json!({})).is_err());
    }

    #[test]
    fn codex_probe_reads_local_windows_when_available() {
        if super::codex_binary().is_none() {
            return;
        }
        let usage = super::probe_codex();
        assert_eq!(usage.provider, "Codex");
        assert!(
            usage
                .windows
                .iter()
                .any(|window| window.source == UsageSource::Live),
            "{}",
            usage.detail
        );
    }
}
