//! Local diagnostics log.
//!
//! Dunlin writes lifecycle, harness and error messages to
//! `~/Library/Logs/Dunlin/dunlin.log`. Prompt text and tool payloads are
//! deliberately excluded. Adapter stderr and protocol metadata are included;
//! credentials are redacted. Set `DUNLIN_TRACE_HARNESS=1` for raw events on
//! stderr when debugging locally.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dunlin_agent::{HarnessEvent, HarnessUpdate};

static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static SECRETS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
const LOG_LIMIT: u64 = 4 * 1024 * 1024;

pub(crate) fn register_secrets(environment: impl IntoIterator<Item = (String, String)>) {
    let mut secrets = SECRETS.get_or_init(Default::default).lock().unwrap();
    for (key, value) in environment {
        let key = key.to_ascii_uppercase();
        if ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"]
            .iter()
            .any(|part| key.contains(part))
            && !value.is_empty()
            && !secrets.contains(&value)
        {
            secrets.push(value);
        }
    }
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
}

fn redact_with(message: &str, secrets: &[String]) -> String {
    let mut text = message.to_owned();
    for secret in secrets {
        text = text.replace(secret, "[REDACTED]");
    }
    static PATTERNS: OnceLock<[regex::Regex; 4]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        regex::Regex::new(r"(?i)(bearer\s+)[a-z0-9._~+/=-]+").unwrap(),
        regex::Regex::new(r#"(?i)((?:api[_-]?key|(?:access[_-]?|refresh[_-]?)?token|password|secret|authorization)[\"']?\s*[:=]\s*[\"']?)[^\s\"',;}]+"#).unwrap(),
        regex::Regex::new(r#"(?i)(--(?:api-key|token|password|secret|authorization)\s+[\"']?)[^\s\"']+"#).unwrap(),
        regex::Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{16,}|AIza[A-Za-z0-9_-]{20,})\b").unwrap(),
    ]);
    for (index, pattern) in patterns.iter().enumerate() {
        text = pattern
            .replace_all(
                &text,
                if index < 3 {
                    "${1}[REDACTED]"
                } else {
                    "[REDACTED]"
                },
            )
            .into_owned();
    }
    text
}

fn redact(message: &str) -> String {
    let secrets = SECRETS.get_or_init(Default::default).lock().unwrap();
    redact_with(message, &secrets)
}

/// Share one sink across all local adapters, including settings connection tests.
pub(crate) fn diagnostic_sink(label: String) -> dunlin_agent::DiagnosticSink {
    std::sync::Arc::new(move |direction, line| {
        let detail = diagnostic_detail(direction, line);
        if direction == "Error" {
            error(&format!("ACP · {label} · {direction} · {detail}"));
        } else {
            info(&format!("ACP · {label} · {direction} · {detail}"));
        }
    })
}

fn diagnostic_detail(direction: &str, line: &str) -> String {
    if direction.eq_ignore_ascii_case("stderr") || direction == "Error" {
        return line.to_owned();
    }
    // Never persist prompts, tool arguments/results or MCP environments.
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(value) => serde_json::json!({
            "id": value.get("id"),
            "method": value.get("method"),
            "error": value.get("error").map(|error| serde_json::json!({
                "code": error.get("code"), "message": error.get("message")
            })),
        })
        .to_string(),
        Err(_) => "[non-protocol output omitted]".into(),
    }
}

pub(crate) fn log_path() -> PathBuf {
    if let Some(path) = std::env::var_os("DUNLIN_LOG_FILE") {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Logs/Dunlin/dunlin.log"))
        .unwrap_or_else(|| PathBuf::from("dunlin.log"))
}

pub(crate) fn init() {
    register_secrets(std::env::vars());
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(&path)
        .map(|metadata| metadata.len() > LOG_LIMIT)
        .unwrap_or(false)
    {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).ok();
    if file.is_some() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
    }
    let _ = LOG.set(Mutex::new(file));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write("PANIC", &info.to_string());
        default_hook(info);
    }));
    info("Dunlin starting");
}

pub(crate) fn info(message: &str) {
    write("INFO", message);
}

pub(crate) fn warn(message: &str) {
    write("WARN", message);
}

pub(crate) fn error(message: &str) {
    write("ERROR", message);
}

/// Log a harness event without prompt or tool payload text.
pub(crate) fn harness_event(context: &str, event: &HarnessEvent) {
    let summary = match event {
        HarnessEvent::Ready { agent, version } => {
            format!("harness ready · {agent} {version}")
        }
        HarnessEvent::SessionCreated { session_id } => {
            format!("harness session created · {session_id}")
        }
        HarnessEvent::SessionLoaded { session_id } => {
            format!("harness session loaded · {session_id}")
        }
        HarnessEvent::SessionLoadFailed {
            session_id,
            message,
        } => {
            format!("session load failed · {session_id} · {message}")
        }
        HarnessEvent::Models { session_id, .. } => format!("session models · {session_id}"),
        HarnessEvent::Modes { session_id, .. } => format!("session modes · {session_id}"),
        HarnessEvent::ConfigOptions { session_id, .. } => {
            format!("session configuration options · {session_id}")
        }
        HarnessEvent::ConfigOptionChangeFailed { message, .. } => {
            format!("configuration change failed · {message}")
        }
        HarnessEvent::ModeChangeFailed { message, .. } => format!("mode change failed · {message}"),
        HarnessEvent::ModelChangeFailed { message, .. } => {
            format!("model change failed · {message}")
        }
        HarnessEvent::Update { session_id, update } => {
            format!("harness update · {session_id} · {}", update_kind(update))
        }
        HarnessEvent::PermissionRequested {
            session_id,
            options,
            ..
        } => format!(
            "permission requested · {session_id} · {} options",
            options.len()
        ),
        HarnessEvent::PromptFinished {
            session_id,
            stop_reason,
            ..
        } => format!("prompt finished · {session_id} · {stop_reason}"),
        HarnessEvent::Error { message } => format!("harness error · {message}"),
        HarnessEvent::Exited { reason } => format!("harness exited · {reason}"),
    };
    let summary = format!("ACP · {context} · {summary}");
    if matches!(
        event,
        HarnessEvent::Error { .. }
            | HarnessEvent::SessionLoadFailed { .. }
            | HarnessEvent::ModeChangeFailed { .. }
            | HarnessEvent::ModelChangeFailed { .. }
            | HarnessEvent::ConfigOptionChangeFailed { .. }
    ) {
        error(&summary);
    } else {
        info(&summary);
    }
}

fn update_kind(update: &HarnessUpdate) -> &'static str {
    match update {
        HarnessUpdate::MessageChunk { thought: true, .. } => "thought",
        HarnessUpdate::MessageChunk { .. } => "message",
        HarnessUpdate::Child { .. } => "subagent",
        HarnessUpdate::UserMessage(_) => "user message",
        HarnessUpdate::Configuration { .. } => "configuration",
        HarnessUpdate::AgentMessage(_) => "message",
        HarnessUpdate::AgentThought(_) => "thought",
        HarnessUpdate::ToolCall(_) => "tool call",
        HarnessUpdate::ToolCallUpdate(_) => "tool update",
        HarnessUpdate::Plan(_) => "plan",
        HarnessUpdate::Usage { .. } => "usage",
        HarnessUpdate::AvailableCommands(_) => "commands",
    }
}

fn write(level: &str, message: &str) {
    let message = redact(message);
    let Some(lock) = LOG.get() else {
        return;
    };
    let Ok(mut guard) = lock.lock() else {
        return;
    };
    let Some(file) = guard.as_mut() else {
        return;
    };
    if file
        .metadata()
        .is_ok_and(|metadata| metadata.len() > LOG_LIMIT)
    {
        let path = log_path();
        if fs::rename(&path, path.with_extension("log.old")).is_ok() {
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            if let Ok(next) = options.open(path) {
                *file = next;
            }
        }
    }
    let _ = writeln!(file, "[{}] {level} {message}", now_ms());
}

/// Export only diagnostic logs, never the database, settings or credentials.
pub(crate) fn export_zip(destination: &std::path::Path) -> Result<(), String> {
    export_logs(&log_path(), destination)
}

fn export_logs(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    let parent = destination.parent().ok_or("Invalid export path")?;
    let mut output = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    {
        let mut archive = zip::ZipWriter::new(output.as_file_mut());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        let lock = LOG.get().map(|log| log.lock().unwrap());
        let mut count = 0;
        for (path, name) in [
            (source.with_extension("log.old"), "dunlin.log.old"),
            (source.to_path_buf(), "dunlin.log"),
        ] {
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            archive
                .start_file(name, options)
                .map_err(|error| error.to_string())?;
            archive
                .write_all(redact(&String::from_utf8_lossy(&bytes)).as_bytes())
                .map_err(|error| error.to_string())?;
            count += 1;
        }
        drop(lock);
        if count == 0 {
            return Err("No diagnostic logs are available yet.".into());
        }
        archive
            .start_file("README.txt", options)
            .map_err(|error| error.to_string())?;
        write!(archive, "Dunlin {} · {} {}\nExported at {} ms since Unix epoch.\nIncludes current and rotated diagnostics from all harnesses, adapter stderr and errors.\nProtocol prompt/tool payloads are omitted; known credentials are redacted.\nAdapter messages may contain local paths or personal data. Review before sharing.\n", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, now_ms()).map_err(|error| error.to_string())?;
        archive.finish().map_err(|error| error.to_string())?;
    }
    output
        .persist(destination)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// Short path shown in Settings.
pub(crate) fn display_path() -> String {
    let path = log_path();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::{display_path, log_path};

    #[test]
    fn log_path_is_stable() {
        assert!(log_path().to_string_lossy().ends_with("dunlin.log"));
        assert!(!display_path().is_empty());
    }

    #[test]
    fn redacts_environment_secrets_bearer_tokens_and_key_assignments() {
        let text = super::redact_with(
            "failure: my-private-value Authorization: Bearer abc.def.ghi api_key=secret123 sk-abcdefghijklmnopqrstuv --api-key cli-secret token=another-secret",
            &["my-private-value".into()],
        );
        for secret in [
            "my-private-value",
            "abc.def.ghi",
            "secret123",
            "sk-abcdefghijklmnopqrstuv",
            "cli-secret",
            "another-secret",
        ] {
            assert!(!text.contains(secret), "secret leaked: {text}");
        }
        assert!(text.contains("failure:"));
    }

    #[test]
    fn protocol_diagnostics_omit_prompts_tool_results_and_error_data() {
        let summary = super::diagnostic_detail(
            "Stdout",
            r#"{"id":2,"method":"session/prompt","params":{"prompt":"private prompt","env":{"TOKEN":"private token"}},"result":{"tool":"private result"},"error":{"code":-32000,"message":"Authentication failed","data":"private error payload"}}"#,
        );
        assert!(summary.contains("session/prompt"));
        assert!(summary.contains("Authentication failed"));
        assert!(!summary.contains("private"));
        assert_eq!(
            super::diagnostic_detail("Stderr", "adapter warning"),
            "adapter warning"
        );
    }

    #[test]
    fn zip_contains_current_and_rotated_logs_but_no_settings() {
        use std::io::Read;
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("dunlin.log");
        std::fs::write(
            &source,
            "Gemini CLI · ERROR · API key missing\napi_key=private-value\n",
        )
        .unwrap();
        std::fs::write(source.with_extension("log.old"), "OpenCode · ready\n").unwrap();
        std::fs::write(directory.path().join("settings.json"), "do not export").unwrap();
        let destination = directory.path().join("diagnostics.zip");
        super::export_logs(&source, &destination).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(destination).unwrap()).unwrap();
        assert_eq!(zip.len(), 3);
        let mut current = String::new();
        zip.by_name("dunlin.log")
            .unwrap()
            .read_to_string(&mut current)
            .unwrap();
        assert!(current.contains("API key missing"));
        assert!(!current.contains("private-value"));
        assert!(zip.by_name("dunlin.log.old").is_ok());
        assert!(zip.by_name("README.txt").is_ok());
        assert!(zip.by_name("settings.json").is_err());
    }

    #[test]
    fn export_without_logs_reports_an_error_and_leaves_no_archive() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("diagnostics.zip");
        assert!(super::export_logs(&directory.path().join("missing.log"), &destination).is_err());
        assert!(!destination.exists());
    }
}
