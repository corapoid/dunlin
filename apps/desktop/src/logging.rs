//! Local diagnostics log.
//!
//! Dunlin writes lifecycle, harness and error messages to
//! `~/Library/Logs/Dunlin/dunlin.log`. Prompt text and tool payloads are
//! deliberately excluded; set `DUNLIN_TRACE_HARNESS=1` for raw events on
//! stderr when debugging locally.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dunlin_agent::{HarnessEvent, HarnessUpdate};

static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub(crate) fn log_path() -> PathBuf {
    if let Some(path) = std::env::var_os("DUNLIN_LOG_FILE") {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Logs/Dunlin/dunlin.log"))
        .unwrap_or_else(|| PathBuf::from("dunlin.log"))
}

pub(crate) fn init() {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(&path)
        .map(|metadata| metadata.len() > 4 * 1024 * 1024)
        .unwrap_or(false)
    {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
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
pub(crate) fn harness_event(event: &HarnessEvent) {
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
        HarnessEvent::Update { session_id, update } => {
            format!("harness update · {session_id} · {}", update_kind(update))
        }
        HarnessEvent::PermissionRequested {
            session_id,
            title,
            options,
            ..
        } => format!(
            "permission requested · {session_id} · {title} · {} options",
            options.len()
        ),
        HarnessEvent::PromptFinished {
            session_id,
            stop_reason,
        } => format!("prompt finished · {session_id} · {stop_reason}"),
        HarnessEvent::Error { message } => format!("harness error · {message}"),
        HarnessEvent::Exited { reason } => format!("harness exited · {reason}"),
    };
    info(&summary);
}

fn update_kind(update: &HarnessUpdate) -> &'static str {
    match update {
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
    let Some(lock) = LOG.get() else {
        return;
    };
    let Ok(mut guard) = lock.lock() else {
        return;
    };
    let Some(file) = guard.as_mut() else {
        return;
    };
    let _ = writeln!(file, "[{}] {level} {message}", now_ms());
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
}
