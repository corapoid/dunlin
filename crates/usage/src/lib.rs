//! Provider usage probes.
//!
//! Dunlin keeps the UI responsive by running every probe on the background
//! executor. Codex exposes a local JSON-RPC app-server with real rate-limit
//! windows; Claude Code keeps its OAuth token in the macOS keychain but only
//! grants usage scopes after a re-login, so Dunlin reports that state instead
//! of inventing percentages. OpenCode has no subscription API over ACP yet and
//! stays clearly marked as estimated.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dunlin_domain::{ProjectTone, ProviderUsage, UsageSource, UsageWindow};
use serde_json::{Value, json};

/// Probe every provider that can be read locally.
pub fn probe_all() -> Vec<ProviderUsage> {
    vec![probe_claude(), probe_codex(), estimated_opencode()]
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
    match claude_credentials() {
        Ok(scopes) if scopes.iter().any(|scope| scope == "user:profile") => unavailable(
            "Claude Code",
            "probe pending",
            "Claude Code granted usage scopes; the OAuth usage reader is not enabled yet.",
            ProjectTone::Violet,
        ),
        Ok(_) => unavailable(
            "Claude Code",
            "re-auth needed",
            "Run `claude /login` to grant `user:profile`, then refresh usage.",
            ProjectTone::Violet,
        ),
        Err(error) => unavailable("Claude Code", "not signed in", &error, ProjectTone::Violet),
    }
}

fn estimated_opencode() -> ProviderUsage {
    ProviderUsage {
        provider: "OpenCode".into(),
        plan: "Estimated".into(),
        accent: ProjectTone::Teal,
        windows: vec![UsageWindow {
            label: "Session".into(),
            used_percent: 19.0,
            resets_in: "estimated".into(),
            source: UsageSource::Estimated,
        }],
        detail: "Estimated · OpenCode does not expose subscription windows over ACP".into(),
    }
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

fn claude_credentials() -> Result<Vec<String>, String> {
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

    let value: Value = serde_json::from_str(raw.trim())
        .map_err(|_| "Claude Code credentials are not readable".to_string())?;
    Ok(value
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("scopes"))
        .and_then(Value::as_array)
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
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
