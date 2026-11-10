use std::fs;
use std::path::{Path, PathBuf};

use dunlin_domain::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::*;

use crate::{harness, logging, remote};

pub(crate) fn editor_language(path: &str) -> &'static str {
    dunlin_workspace::syntax::language(path).unwrap_or("plaintext")
}

pub(crate) fn data_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DUNLIN_DATA_DIR") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    let base = PathBuf::from(home).join("Library/Application Support");
    let dir = base.join("Dunlin");
    if !dir.exists() {
        let legacy = base.join("Kestrel");
        if legacy.is_dir() {
            return Some(migrate_data_dir(&legacy, &dir));
        }
    }
    Some(dir)
}

/// Move the pre-rename data directory in place so an existing install keeps
/// its projects, sessions and settings after upgrading to Dunlin.
pub(crate) fn migrate_data_dir(legacy: &Path, current: &Path) -> PathBuf {
    if let Err(error) = fs::rename(legacy, current) {
        logging::warn(&format!(
            "could not migrate {} to {}: {error}",
            legacy.display(),
            current.display()
        ));
        return legacy.to_path_buf();
    }

    for suffix in ["", "-wal", "-shm"] {
        let from = current.join(format!("kestrel.db{suffix}"));
        let to = current.join(format!("dunlin.db{suffix}"));
        if from.exists() && !to.exists() {
            let _ = fs::rename(from, to);
        }
    }
    logging::info("migrated local data from Kestrel to Dunlin");
    current.to_path_buf()
}

pub(crate) fn data_dir_display() -> String {
    let Some(path) = data_dir() else {
        return "unavailable".into();
    };
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    path.to_string_lossy().to_string()
}

/// Approximate on-disk size of the SQLite database and its WAL files.
pub(crate) fn storage_size() -> u64 {
    let Some(directory) = data_dir() else {
        return 0;
    };
    [
        "dunlin.db",
        "dunlin.db-wal",
        "dunlin.db-shm",
        "kestrel.db",
        "kestrel.db-wal",
        "kestrel.db-shm",
    ]
    .iter()
    .filter_map(|name| std::fs::metadata(directory.join(name)).ok())
    .map(|metadata| metadata.len())
    .sum()
}

/// Only recover stale selection metadata for an unused, unavailable local
/// harness. Never migrate an ACP context or skip real conversation history.
pub(crate) fn startup_session_recovery(
    sessions: &[SessionSummary],
    selected: usize,
    transcript: &[harness::ChatItem],
    preferred: &str,
    selected_available: bool,
    preferred_available: bool,
    remote: bool,
) -> Option<usize> {
    let current = sessions.get(selected)?;
    if remote
        || selected_available
        || !preferred_available
        || current.harness == preferred
        || current.acp_session_id.is_some()
        || transcript
            .iter()
            .any(|item| !matches!(item, harness::ChatItem::Error { .. }))
    {
        return None;
    }
    sessions
        .iter()
        .position(|session| session.harness == preferred && !session.origin.is_remote())
}

pub(crate) fn open_store() -> Result<dunlin_storage::Store, String> {
    let base = data_dir()
        .ok_or_else(|| "Local storage is unavailable. History will not be saved.".to_string())?;
    let database = if base.join("dunlin.db").exists() || !base.join("kestrel.db").exists() {
        base.join("dunlin.db")
    } else {
        base.join("kestrel.db")
    };
    match dunlin_storage::Store::open(database) {
        Ok(store) => Ok(store),
        Err(error) => {
            logging::error(&format!("could not open database: {error}"));
            eprintln!("[storage] could not open database: {error}");
            Err(format!(
                "Could not open the database: {error}. History will not be saved."
            ))
        }
    }
}

pub(crate) fn remote_session_summary(
    row: &remote::RemoteSessionInfo,
    project_id: &str,
    host_id: &str,
) -> SessionSummary {
    SessionSummary {
        workspace: None,
        id: row.id.clone(),
        project_id: project_id.to_string(),
        title: row.title.clone(),
        status: SessionStatus::Idle,
        harness: "Remote".into(),
        model: "Auto".into(),
        branch: None,
        updated_at: chrono::DateTime::from_timestamp(row.updated_at, 0)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        cost_usd: 0.0,
        duration_secs: 0,
        tokens: 0,
        summary: "Remote session on the relay.".into(),
        origin: SessionOrigin::Remote {
            host_id: host_id.to_string(),
        },
        acp_session_id: Some(row.id.clone()),
    }
}

pub(crate) fn format_session_date(value: &str) -> String {
    use chrono::{DateTime, Local};
    let Ok(parsed) = DateTime::parse_from_rfc3339(value) else {
        return value.trim().to_string();
    };
    let local = parsed.with_timezone(&Local);
    local.format("%Y-%m-%d %H:%M").to_string()
}

pub(crate) fn metadata_mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn available_font(requested: &str, fallback: &str, installed: &[String]) -> String {
    installed
        .iter()
        .find(|font| font.as_str() == requested)
        .map(String::as_str)
        .unwrap_or(fallback)
        .to_owned()
}

pub(crate) fn blank_session(
    project_id: &str,
    harness: &str,
    branch: Option<String>,
) -> SessionSummary {
    SessionSummary {
        workspace: None,
        id: format!("ses-{}", uuid::Uuid::now_v7()),
        project_id: project_id.to_string(),
        title: "New session".into(),
        status: SessionStatus::Idle,
        harness: harness.to_string(),
        model: "Auto".into(),
        branch,
        updated_at: chrono::Utc::now().to_rfc3339(),
        cost_usd: 0.0,
        duration_secs: 0,
        tokens: 0,
        summary: "No messages yet. Ask Dunlin to build, fix, or explain something.".into(),
        origin: SessionOrigin::Local,
        acp_session_id: None,
    }
}

pub(crate) fn is_unused_draft(session: &SessionSummary) -> bool {
    session.title == "New session"
        && session.tokens == 0
        && session.cost_usd == 0.0
        && session.duration_secs == 0
        && matches!(
            session.status,
            SessionStatus::Idle | SessionStatus::Waiting | SessionStatus::Interrupted
        )
}

pub(crate) fn recovered_session_status(
    session: &SessionSummary,
    empty_history: bool,
) -> SessionStatus {
    if empty_history && is_unused_draft(session) {
        SessionStatus::Idle
    } else if matches!(
        session.status,
        SessionStatus::Running | SessionStatus::Waiting
    ) {
        SessionStatus::Interrupted
    } else {
        session.status
    }
}

pub(crate) fn git_branch(path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

pub(crate) fn tone_color(p: &Palette, tone: ProjectTone) -> Hsla {
    match tone {
        ProjectTone::Blue => p.blue,
        ProjectTone::Green => p.green,
        ProjectTone::Violet => p.violet,
        ProjectTone::Amber => p.amber,
        ProjectTone::Rose => p.red,
        ProjectTone::Teal => p.teal,
    }
}

pub(crate) fn tone_soft(p: &Palette, tone: ProjectTone) -> Hsla {
    match tone {
        ProjectTone::Blue => p.blue_soft,
        ProjectTone::Green => p.green_soft,
        ProjectTone::Violet => p.violet_soft,
        ProjectTone::Amber => p.amber_soft,
        ProjectTone::Rose => p.red_soft,
        ProjectTone::Teal => p.blue_soft,
    }
}

pub(crate) fn status_palette(p: &Palette, status: SessionStatus) -> (Hsla, Hsla) {
    match status {
        SessionStatus::Idle => (p.muted_fg, p.muted),
        SessionStatus::Running => (p.blue, p.blue_soft),
        SessionStatus::Waiting => (p.amber, p.amber_soft),
        SessionStatus::Done => (p.green, p.green_soft),
        SessionStatus::Failed => (p.red, p.red_soft),
        SessionStatus::Interrupted => (p.muted_fg, p.muted),
    }
}

pub(crate) fn agent_status_palette(p: &Palette, status: AgentStatus) -> (Hsla, Hsla) {
    match status {
        AgentStatus::Running => (p.blue, p.blue_soft),
        AgentStatus::Waiting => (p.amber, p.amber_soft),
        AgentStatus::Done => (p.green, p.green_soft),
        AgentStatus::Failed => (p.red, p.red_soft),
        AgentStatus::Stopped => (p.muted_fg, p.muted),
    }
}

pub(crate) fn tool_icon(kind: ToolKind) -> IconName {
    match kind {
        ToolKind::Read => IconName::FileText,
        ToolKind::Edit => IconName::PencilLine,
        ToolKind::Shell => IconName::SquareTerminal,
        ToolKind::Search => IconName::Search,
        ToolKind::Web => IconName::Globe,
        ToolKind::Task => IconName::ListTodo,
        ToolKind::Test => IconName::FlaskConical,
        ToolKind::Plan => IconName::Map,
    }
}

pub(crate) fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000_000 {
        format!("{:.2}T", tokens as f64 / 1_000_000_000_000.)
    } else if tokens >= 1_000_000_000 {
        format!("{:.2}B", tokens as f64 / 1_000_000_000.)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[test]
fn token_labels_scale_through_trillions() {
    for (tokens, label) in [
        (999, "999"),
        (1_000, "1.0k"),
        (1_000_000, "1.0M"),
        (19_963_900_000, "19.96B"),
        (1_000_000_000_000, "1.00T"),
    ] {
        assert_eq!(format_tokens(tokens), label);
    }
}

/// Pick the session to show after one has been removed at `removed`.
pub(crate) fn session_index_after_delete(
    removed: usize,
    selected: usize,
    len_after: usize,
) -> usize {
    if len_after == 0 {
        0
    } else if removed < selected {
        selected - 1
    } else if removed == selected {
        removed.min(len_after - 1)
    } else {
        selected
    }
}

/// Prefer a project-relative path in prompts; keep absolute paths outside it.
pub(crate) fn display_path(root: &str, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

/// Whether a new file sits inside `root`, resolving symlinks on its parent.
pub(crate) fn path_inside(root: &Path, path: &Path) -> bool {
    let parent = path.parent().unwrap_or(root);
    match (parent.canonicalize(), root.canonicalize()) {
        (Ok(parent), Ok(root)) => parent.starts_with(root),
        _ => false,
    }
}

pub(crate) fn context_status_label(usage: Option<harness::HarnessUsage>, tokens: u64) -> String {
    match usage {
        Some(usage) if usage.size > 0 => format!(
            "{} / {} context",
            format_tokens(usage.used),
            format_tokens(usage.size)
        ),
        Some(usage) => format!("{} tokens", format_tokens(usage.used)),
        None if tokens > 0 => format!("{} tokens", format_tokens(tokens)),
        None => "Context idle".into(),
    }
}

pub(crate) fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes < 1024usize.saturating_mul(1024 * 1024 * 1024) {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!(
            "{:.2} TB",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0)
        )
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    format_bytes(bytes.min(usize::MAX as u64) as usize)
}

pub(crate) fn format_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_dates_include_local_date_and_time_and_preserve_unknown_values() {
        assert_eq!(super::format_session_date("not a date"), "not a date");
        assert_eq!(
            super::format_session_date("2020-01-02T03:04:05Z"),
            chrono::DateTime::parse_from_rfc3339("2020-01-02T03:04:05Z")
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        );
        let now = chrono::Utc::now().to_rfc3339();
        assert_eq!(super::format_session_date(&now).len(), 16);
    }
}
