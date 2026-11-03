//! Read-only import of OpenCode's local session database.
//!
//! Only `session`, `message` and `part` rows are read. Credentials, blobs,
//! snapshots and the writable OpenCode state are never touched.

use super::adapters::Candidate;
use crate::harness::{ChatItem, TurnTiming};
use dunlin_agent::{TokenUsage, ToolCallView};
use dunlin_domain::{ToolKind, ToolStatus};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MAX_SESSIONS: usize = 256;
const MAX_MESSAGES_PER_SESSION: usize = 30_000;
const MAX_PARTS_PER_SESSION: usize = 60_000;

pub(super) fn load(path: &Path) -> Result<(Vec<Candidate>, Vec<String>), String> {
    load_inner(path, None)
}

pub(super) fn load_selected(
    path: &Path,
    ids: &HashSet<String>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    load_inner(path, Some(ids))
}

fn load_inner(
    path: &Path,
    filter: Option<&HashSet<String>>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "OpenCode database is locked or unavailable".to_string())?;
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
    if connection
        .prepare("SELECT 1 FROM session_v2 LIMIT 1")
        .is_ok()
        && let Ok((candidates, warnings)) = load_v2(&connection, filter)
        && !candidates.is_empty()
    {
        return Ok((candidates, warnings));
    }
    load_v1(&connection, filter)
}

fn load_v2(
    connection: &Connection,
    filter: Option<&HashSet<String>>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let mut warnings = Vec::new();
    let mut session_statement = connection
        .prepare(
            "SELECT id, title, directory, model, time_updated
             FROM session_v2
             WHERE parent_id IS NULL
             ORDER BY time_updated DESC
             LIMIT ?1",
        )
        .map_err(|_| "OpenCode v2 session schema is not recognized".to_string())?;
    let mut sessions = session_statement
        .query_map([MAX_SESSIONS], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| "OpenCode v2 sessions could not be read".to_string())?;

    let mut candidates = Vec::new();
    let mut skipped = 0usize;
    while let Some(Ok((id, title, directory, model, updated))) = sessions.next() {
        if filter.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let mut message_statement = match connection.prepare(
            "SELECT type, data FROM session_message
             WHERE session_id = ?1
             ORDER BY seq
             LIMIT ?2",
        ) {
            Ok(statement) => statement,
            Err(_) => {
                warnings.push(format!(
                    "{}: OpenCode v2 messages could not be read",
                    title.as_deref().unwrap_or(&id)
                ));
                continue;
            }
        };
        let messages = match message_statement.query_map(
            rusqlite::params![id, MAX_MESSAGES_PER_SESSION as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ) {
            Ok(rows) => rows,
            Err(_) => {
                warnings.push(format!(
                    "{}: OpenCode v2 messages could not be read",
                    title.as_deref().unwrap_or(&id)
                ));
                continue;
            }
        };
        let mut transcript = Vec::new();
        let mut usage = UsageTotals::default();
        let mut turn = None;
        for (kind, data) in messages.flatten() {
            let Ok(data) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let at = ms(data.pointer("/time/created").and_then(Value::as_i64));
            match kind.as_str() {
                "user" => {
                    finish_turn(&mut transcript, &mut turn, &mut usage);
                    let mut user_index = None;
                    if let Some(text) = data["text"].as_str().filter(|text| !text.trim().is_empty())
                    {
                        let mut content = truncate(text, 32 * 1024);
                        if let Some(files) =
                            data["files"].as_array().filter(|files| !files.is_empty())
                        {
                            content.push_str(&format!("\n[{} attachment(s) omitted]", files.len()));
                        }
                        transcript.push(text_item("user", &content, at));
                        user_index = Some(transcript.len() - 1);
                    }
                    turn = Some(PendingTurn {
                        user_index,
                        started_at: at,
                        ..Default::default()
                    });
                }
                "assistant" => {
                    if let Some(tokens) = data.get("tokens")
                        && let Some(pending) = turn.as_mut()
                    {
                        pending.observe(tokens, message_finished_at(&data, at));
                    }
                    if let Some(content) = data["content"].as_array() {
                        for part in content {
                            match part["type"].as_str().unwrap_or_default() {
                                "text" => {
                                    if let Some(text) =
                                        part["text"].as_str().filter(|text| !text.trim().is_empty())
                                    {
                                        transcript.push(text_item("assistant", text, at));
                                    }
                                }
                                "reasoning" => {
                                    if let Some(text) =
                                        part["text"].as_str().filter(|text| !text.trim().is_empty())
                                    {
                                        transcript.push(reasoning_item(text, at));
                                    }
                                }
                                "tool" => {
                                    let name = part["name"].as_str().unwrap_or("tool");
                                    let id = part["id"]
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| format!("tool-{at}"));
                                    transcript.push(tool_call_item(id, name, &part["state"], at));
                                }
                                "file" => {
                                    let name = part["filename"]
                                        .as_str()
                                        .or_else(|| part["name"].as_str())
                                        .or_else(|| part["url"].as_str())
                                        .unwrap_or("attachment");
                                    transcript.push(text_item(
                                        "file",
                                        &format!("[File] {name}"),
                                        at,
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(error) = data.pointer("/error/message").and_then(Value::as_str) {
                        transcript.push(text_item("error", error, at));
                    }
                }
                _ => {}
            }
        }
        finish_turn(&mut transcript, &mut turn, &mut usage);
        if transcript.is_empty() {
            skipped += 1;
            continue;
        }
        let title = title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                transcript
                    .iter()
                    .find_map(|item| match item {
                        ChatItem::User { text, .. } => {
                            let first = text.lines().next().unwrap_or("").trim();
                            (!first.is_empty()).then(|| first.chars().take(80).collect::<String>())
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "Imported conversation".into())
            });
        candidates.push(Candidate {
            source_id: id,
            title,
            directory: (!directory.trim().is_empty()).then_some(directory),
            model: model_name(model.as_deref()),
            transcript,
            at_ms: ms(Some(updated)),
        });
    }
    if skipped > 0 {
        warnings.push(format!(
            "Skipped {skipped} OpenCode sessions without readable text or tool history"
        ));
    }
    Ok((candidates, warnings))
}

fn load_v1(
    connection: &Connection,
    filter: Option<&HashSet<String>>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let mut warnings = Vec::new();
    let mut session_statement = connection
        .prepare(
            "SELECT id, title, directory, model, time_updated
             FROM session
             WHERE parent_id IS NULL
             ORDER BY time_updated DESC
             LIMIT ?1",
        )
        .or_else(|_| {
            connection.prepare(
                "SELECT id, title, directory, model, time_updated
                 FROM session
                 ORDER BY time_updated DESC
                 LIMIT ?1",
            )
        })
        .map_err(|_| "OpenCode session schema is not recognized".to_string())?;
    let mut sessions = session_statement
        .query_map([MAX_SESSIONS], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| "OpenCode sessions could not be read".to_string())?;

    let mut candidates = Vec::new();
    while let Some(Ok((id, title, directory, model, updated))) = sessions.next() {
        if filter.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let mut messages = match connection.prepare(
            "SELECT id, data FROM message
             WHERE session_id = ?1
             ORDER BY time_created, id
             LIMIT ?2",
        ) {
            Ok(statement) => statement,
            Err(_) => {
                warnings.push(format!("{title}: OpenCode messages could not be read"));
                continue;
            }
        };
        let message_rows = match messages.query_map(
            rusqlite::params![id, MAX_MESSAGES_PER_SESSION as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ) {
            Ok(rows) => rows,
            Err(_) => {
                warnings.push(format!("{title}: OpenCode messages could not be read"));
                continue;
            }
        };

        let mut part_statement = match connection.prepare(
            "SELECT message_id, data FROM part
             WHERE session_id = ?1
             ORDER BY time_created, id
             LIMIT ?2",
        ) {
            Ok(statement) => statement,
            Err(_) => {
                warnings.push(format!("{title}: OpenCode parts could not be read"));
                continue;
            }
        };
        let part_rows = match part_statement
            .query_map(rusqlite::params![id, MAX_PARTS_PER_SESSION as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
            Ok(rows) => rows,
            Err(_) => {
                warnings.push(format!("{title}: OpenCode parts could not be read"));
                continue;
            }
        };
        let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
        for row in part_rows.flatten() {
            if let Ok(value) = serde_json::from_str::<Value>(&row.1) {
                parts.entry(row.0).or_default().push(value);
            }
        }

        let mut transcript = Vec::new();
        let mut usage = UsageTotals::default();
        let mut turn = None;
        for row in message_rows.flatten() {
            let Ok(data) = serde_json::from_str::<Value>(&row.1) else {
                continue;
            };
            let role = data["role"].as_str().unwrap_or("assistant");
            let at = ms(data.pointer("/time/created").and_then(Value::as_i64));
            if role == "user" {
                finish_turn(&mut transcript, &mut turn, &mut usage);
            }
            let before_len = transcript.len();
            let Some(message_parts) = parts.get(&row.0) else {
                continue;
            };
            for part in message_parts {
                let kind = part["type"].as_str().unwrap_or_default();
                match kind {
                    "text" => {
                        if let Some(text) =
                            part["text"].as_str().filter(|text| !text.trim().is_empty())
                        {
                            transcript.push(text_item(role, text, at));
                        }
                    }
                    "reasoning" => {
                        if let Some(text) =
                            part["text"].as_str().filter(|text| !text.trim().is_empty())
                        {
                            transcript.push(reasoning_item(text, at));
                        }
                    }
                    "tool" => {
                        let name = part["tool"].as_str().unwrap_or("tool");
                        let id = part["id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("tool-{at}"));
                        transcript.push(tool_call_item(id, name, &part["state"], at));
                    }
                    "file" => {
                        let name = part["filename"]
                            .as_str()
                            .or_else(|| part["name"].as_str())
                            .or_else(|| part["url"].as_str())
                            .unwrap_or("attachment");
                        transcript.push(text_item("file", &format!("[File] {name}"), at));
                    }
                    "compaction" => {
                        if let Some(text) =
                            part["text"].as_str().filter(|text| !text.trim().is_empty())
                        {
                            transcript.push(text_item("record", text, at));
                        }
                    }
                    _ => {}
                }
            }
            match role {
                "user" => {
                    let user_index = transcript[before_len..]
                        .iter()
                        .position(|item| matches!(item, ChatItem::User { .. }))
                        .map(|offset| before_len + offset);
                    turn = Some(PendingTurn {
                        user_index,
                        started_at: at,
                        ..Default::default()
                    });
                }
                "assistant" => {
                    if let Some(tokens) = data.get("tokens")
                        && let Some(pending) = turn.as_mut()
                    {
                        pending.observe(tokens, message_finished_at(&data, at));
                    }
                }
                _ => {}
            }
        }
        finish_turn(&mut transcript, &mut turn, &mut usage);
        if transcript.is_empty() {
            warnings.push(format!("{title}: no readable transcript"));
            continue;
        }
        candidates.push(Candidate {
            source_id: id,
            title,
            directory: (!directory.trim().is_empty()).then_some(directory),
            model: model_name(model.as_deref()),
            transcript,
            at_ms: ms(Some(updated)),
        });
    }
    Ok((candidates, warnings))
}

fn text_item(role: &str, text: &str, at: u64) -> ChatItem {
    if role == "user" {
        ChatItem::User {
            text: text.to_string(),
            timing: None,
            at_ms: at,
        }
    } else {
        ChatItem::AgentText {
            text: text.to_string(),
            streaming: false,
            message_id: None,
            at_ms: at,
        }
    }
}

fn reasoning_item(text: &str, at: u64) -> ChatItem {
    ChatItem::Thought {
        text: text.to_string(),
        streaming: false,
        message_id: None,
        at_ms: at,
    }
}

#[derive(Clone, Copy, Default)]
struct UsageTotals {
    input: u64,
    output: u64,
    reasoning: u64,
}

impl UsageTotals {
    fn add(&mut self, tokens: &Value) {
        let input = token_number(tokens.get("input"));
        let output = token_number(tokens.get("output"));
        let reasoning = token_number(tokens.get("reasoning"));
        if input == 0 && output == 0 && reasoning == 0 {
            self.input = self.input.saturating_add(token_number(tokens.get("total")));
            return;
        }
        self.input = self.input.saturating_add(input);
        self.output = self.output.saturating_add(output);
        self.reasoning = self.reasoning.saturating_add(reasoning);
    }

    fn add_totals(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
    }

    fn snapshot(self) -> TokenUsage {
        TokenUsage {
            total: self
                .input
                .saturating_add(self.output)
                .saturating_add(self.reasoning),
            input: self.input,
            output: self.output,
            reasoning: Some(self.reasoning),
        }
    }
}

fn token_number(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

#[derive(Default)]
struct PendingTurn {
    user_index: Option<usize>,
    started_at: u64,
    finished_at: u64,
    tokens: UsageTotals,
    samples: u64,
}

impl PendingTurn {
    fn observe(&mut self, tokens: &Value, finished_at: u64) {
        self.tokens.add(tokens);
        self.samples += 1;
        self.finished_at = self.finished_at.max(finished_at);
    }
}

fn finish_turn(
    transcript: &mut [ChatItem],
    turn: &mut Option<PendingTurn>,
    cumulative: &mut UsageTotals,
) {
    let Some(pending) = turn.take() else {
        return;
    };
    let Some(index) = pending.user_index else {
        return;
    };
    if pending.samples == 0 {
        return;
    }
    let Some(ChatItem::User { at_ms, timing, .. }) = transcript.get_mut(index) else {
        return;
    };
    let started = if pending.started_at > 0 {
        pending.started_at
    } else {
        *at_ms
    };
    let finished = (pending.finished_at > 0).then_some(pending.finished_at);
    let before = *cumulative;
    cumulative.add_totals(&pending.tokens);
    *timing = Some(TurnTiming {
        started_at_ms: started,
        finished_at_ms: finished,
        elapsed_ms: finished.and_then(|end| end.checked_sub(started)),
        usage: Some(cumulative.snapshot()),
        usage_before: Some(before.snapshot()),
    });
}

fn message_finished_at(data: &Value, fallback: u64) -> u64 {
    data.pointer("/time/completed")
        .and_then(Value::as_i64)
        .map(|value| ms(Some(value)))
        .unwrap_or(fallback)
}

fn tool_call_item(id: String, name: &str, state: &Value, at: u64) -> ChatItem {
    let title = state["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(name)
        .to_string();
    let mut detail = String::new();
    if let Some(input) = state.get("input").filter(|value| !value.is_null()) {
        detail.push_str(&input.to_string());
    }
    if let Some(content) = state["content"].as_array() {
        for item in content {
            if let Some(text) = item["text"].as_str().filter(|text| !text.trim().is_empty()) {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(text);
            }
        }
    } else if let Some(output) = state.get("output").filter(|value| !value.is_null()) {
        if !detail.is_empty() {
            detail.push('\n');
        }
        detail.push_str(&output.to_string());
    }
    let (additions, deletions, locations, diff) = tool_changes(state);
    ChatItem::Tool {
        view: ToolCallView {
            id,
            title,
            kind: tool_kind(name),
            status: match state["status"].as_str() {
                Some("running") => ToolStatus::Running,
                Some("pending") | Some("waiting") => ToolStatus::Waiting,
                Some("error") | Some("failed") => ToolStatus::Failed,
                _ => ToolStatus::Done,
            },
            detail: truncate(&detail, 4096),
            locations,
            additions,
            deletions,
            diff,
        },
        duration_ms: 0,
        at_ms: at,
    }
}

fn tool_changes(state: &Value) -> (u32, u32, Vec<String>, Option<String>) {
    let metadata = &state["metadata"];
    let mut additions = 0u32;
    let mut deletions = 0u32;
    let mut locations = Vec::new();
    let mut diff = None;
    let mut consume = |file: &Value| {
        let patch = file["patch"].as_str();
        let (patch_additions, patch_deletions) = patch.map(diff_counts).unwrap_or_default();
        additions = additions.saturating_add(
            file["additions"]
                .as_u64()
                .map(|value| value as u32)
                .unwrap_or(patch_additions),
        );
        deletions = deletions.saturating_add(
            file["deletions"]
                .as_u64()
                .map(|value| value as u32)
                .unwrap_or(patch_deletions),
        );
        if diff.is_none()
            && let Some(patch) = patch.filter(|patch| !patch.trim().is_empty())
        {
            diff = Some(truncate(patch, 16 * 1024));
        }
        let path = file["file"]
            .as_str()
            .or_else(|| file["filePath"].as_str())
            .or_else(|| file["relativePath"].as_str());
        if let Some(path) = path.filter(|path| !path.trim().is_empty())
            && !locations.iter().any(|known| known == path)
        {
            locations.push(path.to_string());
        }
    };

    let mut described = false;
    if let Some(files) = metadata["files"].as_array() {
        for file in files {
            consume(file);
            described = true;
        }
    }
    if !described && metadata.get("filediff").is_some() {
        consume(&metadata["filediff"]);
        described = true;
    }
    if !described
        && let Some(patch) = metadata["diff"]
            .as_str()
            .filter(|patch| !patch.trim().is_empty())
    {
        let (add, del) = diff_counts(patch);
        additions = additions.saturating_add(add);
        deletions = deletions.saturating_add(del);
        diff = Some(truncate(patch, 16 * 1024));
    }
    (additions, deletions, locations, diff)
}

fn diff_counts(patch: &str) -> (u32, u32) {
    let mut additions = 0u32;
    let mut deletions = 0u32;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions = additions.saturating_add(1);
        } else if line.starts_with('-') {
            deletions = deletions.saturating_add(1);
        }
    }
    (additions, deletions)
}

fn tool_kind(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "shell" | "bash" | "exec" => ToolKind::Shell,
        "read" | "view" => ToolKind::Read,
        "grep" | "glob" | "search" | "find" => ToolKind::Search,
        "edit" | "write" | "patch" | "apply_patch" => ToolKind::Edit,
        "webfetch" | "websearch" | "fetch" => ToolKind::Web,
        "test" => ToolKind::Test,
        "task" => ToolKind::Task,
        "plan" | "todowrite" | "todoread" => ToolKind::Plan,
        _ => ToolKind::Task,
    }
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated]", &value[..end])
}

fn model_name(model: Option<&str>) -> String {
    model
        .and_then(|model| serde_json::from_str::<Value>(model).ok())
        .and_then(|value| {
            value
                .get("id")
                .or_else(|| value.get("modelID"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Auto".into())
}

fn ms(value: Option<i64>) -> u64 {
    let value = value.unwrap_or(0);
    if value <= 0 {
        0
    } else if value < 10_000_000_000 {
        value as u64 * 1000
    } else {
        value as u64
    }
}

#[cfg(test)]
mod tests {
    use super::{load, load_selected};
    use crate::harness::ChatItem;
    use rusqlite::Connection;
    use std::collections::HashSet;

    #[test]
    fn reads_top_level_sessions_text_reasoning_and_tool_parts() {
        let path = std::env::temp_dir().join(format!(
            "dunlin-opencode-sessions-{}.db",
            uuid::Uuid::now_v7()
        ));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE session (
                    id TEXT PRIMARY KEY, title TEXT, directory TEXT, model TEXT,
                    parent_id TEXT, time_updated INTEGER
                );
                CREATE TABLE message (
                    id TEXT PRIMARY KEY, session_id TEXT, data TEXT, time_created INTEGER
                );
                CREATE TABLE part (
                    id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT,
                    data TEXT, time_created INTEGER
                );
                INSERT INTO session VALUES
                    ('s1', 'Visible', '/repo', '{"id":"gpt-5"}', NULL, 1700000000000),
                    ('s2', 'Subagent', '/repo', '{"id":"gpt-5"}', 's1', 1700000000001);
                INSERT INTO message VALUES
                    ('m1', 's1', '{"role":"user","time":{"created":1700000000000}}', 1),
                    ('m2', 's1', '{"role":"assistant","time":{"created":1700000001000,"completed":1700000002500},"tokens":{"input":40,"output":10,"reasoning":2,"cache":{"read":100}}}', 2),
                    ('m3', 's2', '{"role":"user"}', 3);
                INSERT INTO part VALUES
                    ('p1', 'm1', 's1', '{"type":"text","text":"hello"}', 1),
                    ('p2', 'm2', 's1', '{"type":"reasoning","text":"thought"}', 2),
                    ('p3', 'm2', 's1', '{"type":"tool","tool":"edit","state":{"status":"completed","input":{"path":"src/a.rs"},"metadata":{"files":[{"file":"src/a.rs","additions":2,"deletions":1,"patch":"--- a/src/a.rs\n+++ b/src/a.rs\n@@\n-old\n+new\n+extra"}]}}}', 3),
                    ('p4', 'm3', 's2', '{"type":"text","text":"hidden"}', 4);
                "#,
            )
            .unwrap();
        drop(connection);

        let (sessions, warnings) = load(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Visible");
        assert_eq!(sessions[0].directory.as_deref(), Some("/repo"));
        assert_eq!(sessions[0].model, "gpt-5");
        assert_eq!(sessions[0].transcript.len(), 3);
        let timing = sessions[0]
            .transcript
            .iter()
            .find_map(|item| match item {
                ChatItem::User { timing, .. } => *timing,
                _ => None,
            })
            .expect("user turn timing");
        assert_eq!(timing.usage.unwrap().total, 52);
        assert_eq!(timing.elapsed_ms, Some(2500));
        let tool = sessions[0]
            .transcript
            .iter()
            .find_map(|item| match item {
                ChatItem::Tool { view, .. } => Some(view),
                _ => None,
            })
            .expect("tool card");
        assert_eq!((tool.additions, tool.deletions), (2, 1));
        assert_eq!(tool.locations, ["src/a.rs"]);
        assert!(tool.diff.as_deref().unwrap().contains("+extra"));
        assert!(warnings.is_empty());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn reads_v2_sessions_with_embedded_content_parts() {
        let path =
            std::env::temp_dir().join(format!("dunlin-opencode-v2-{}.db", uuid::Uuid::now_v7()));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE session_v2 (
                    id TEXT PRIMARY KEY, title TEXT, directory TEXT, model TEXT,
                    parent_id TEXT, time_updated INTEGER
                );
                CREATE TABLE session_message (
                    id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                    data TEXT, time_created INTEGER
                );
                INSERT INTO session_v2 VALUES
                    ('s1', 'Current', '/repo', '{"id":"gpt-6"}', NULL, 1700000000000),
                    ('s2', 'Child', '/repo', '{"id":"gpt-6"}', 's1', 1700000000001);
                INSERT INTO session_message VALUES
                    ('m1', 's1', 'user', 1, '{"time":{"created":1700000000000},"text":"hello","files":[]}', 1),
                    ('m2', 's1', 'assistant', 2, '{"time":{"created":1700000001000,"completed":1700000003000},"tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":1000}},"content":[{"type":"reasoning","text":"think"},{"type":"text","text":"answer"},{"type":"tool","name":"edit","state":{"status":"completed","input":{"path":"src/b.rs"},"metadata":{"files":[{"file":"src/b.rs","additions":4,"deletions":3,"patch":"--- a/src/b.rs\n+++ b/src/b.rs\n@@\n-one\n+two\n+three"}]}}}]}', 2),
                    ('m3', 's2', 'user', 3, '{"time":{"created":1700000002000},"text":"hidden"}', 3);
                "#,
            )
            .unwrap();
        drop(connection);

        let (sessions, warnings) = load(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Current");
        assert_eq!(sessions[0].transcript.len(), 4);
        let timing = sessions[0]
            .transcript
            .iter()
            .find_map(|item| match item {
                ChatItem::User { timing, .. } => *timing,
                _ => None,
            })
            .expect("user turn timing");
        assert_eq!(timing.usage.unwrap().total, 125);
        let tool = sessions[0]
            .transcript
            .iter()
            .find_map(|item| match item {
                ChatItem::Tool { view, .. } => Some(view),
                _ => None,
            })
            .expect("tool card");
        assert_eq!((tool.additions, tool.deletions), (4, 3));
        assert_eq!(tool.locations, ["src/b.rs"]);
        assert!(warnings.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn selected_import_only_reads_requested_sessions() {
        let path =
            std::env::temp_dir().join(format!("dunlin-opencode-v2-{}.db", uuid::Uuid::now_v7()));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE session_v2 (
                    id TEXT PRIMARY KEY, title TEXT, directory TEXT, model TEXT,
                    parent_id TEXT, time_updated INTEGER
                );
                CREATE TABLE session_message (
                    id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                    data TEXT, time_created INTEGER
                );
                INSERT INTO session_v2 VALUES
                    ('s1', 'First', '/repo', '{"id":"gpt-6"}', NULL, 1700000000000),
                    ('s2', 'Second', '/repo', '{"id":"gpt-6"}', NULL, 1700000000001);
                INSERT INTO session_message VALUES
                    ('m1', 's1', 'user', 1, '{"time":{"created":1700000000000},"text":"one"}', 1),
                    ('m2', 's2', 'user', 2, '{"time":{"created":1700000000001},"text":"two"}', 2);
                "#,
            )
            .unwrap();
        drop(connection);

        let ids = HashSet::from(["s2".to_string()]);
        let (sessions, warnings) = load_selected(&path, &ids).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source_id, "s2");
        assert_eq!(sessions[0].title, "Second");
        assert!(warnings.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
