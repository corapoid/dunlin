//! Read-only import of OpenCode's local session database.
//!
//! Only V1/V2 session, message and part rows are read. Credentials, blobs,
//! snapshots and the writable OpenCode state are never touched.

use super::adapters::Candidate;
use crate::harness::{ChatItem, TurnTiming};
use dunlin_agent::{TokenUsage, ToolCallView};
use dunlin_domain::{ToolKind, ToolStatus};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MAX_MESSAGES_PER_SESSION: usize = 30_000;
const MAX_PARTS_PER_SESSION: usize = 60_000;

#[cfg(test)]
pub(super) fn load(path: &Path) -> Result<(Vec<Candidate>, Vec<String>), String> {
    load_inner(path, None)
}

#[cfg(test)]
pub(super) fn load_selected(
    path: &Path,
    ids: &HashSet<String>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    load_inner(path, Some(ids))
}

#[cfg(test)]
fn load_inner(
    path: &Path,
    filter: Option<&HashSet<String>>,
) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let mut candidates = Vec::new();
    let warnings = visit(
        path,
        filter,
        &mut |candidate| {
            candidates.push(candidate);
            Ok(())
        },
        &mut |_, _| {},
    )?;
    Ok((candidates, warnings))
}

pub(super) fn visit(
    path: &Path,
    filter: Option<&HashSet<String>>,
    candidate: &mut dyn FnMut(Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<String>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "OpenCode database is locked or unavailable".to_string())?;
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
    let v2 = connection
        .prepare("SELECT 1 FROM session_v2 LIMIT 0")
        .is_ok();
    let v1 = connection.prepare("SELECT 1 FROM session LIMIT 0").is_ok();
    let mut emitted = HashSet::new();
    let mut warnings = Vec::new();
    let mut unreadable = HashSet::new();
    if v2 {
        let mut visitor_error = None;
        let result = load_v2(
            &connection,
            filter,
            &mut |row| {
                emitted.insert(row.source_id.clone());
                candidate(row).inspect_err(|error| visitor_error = Some(error.clone()))
            },
            progress,
        );
        if let Some(error) = visitor_error {
            return Err(error);
        }
        match result {
            Ok((notes, empty)) => {
                warnings.extend(notes);
                unreadable = empty;
            }
            Err(error) if v1 => warnings.push(error),
            Err(error) => return Err(error),
        }
    }
    // Migrated databases retain V1 histories, sometimes with an empty V2 shell
    // under the same ID. Prefer readable V2, then fall back per session, not DB.
    if v1 {
        let mut missing = HashSet::new();
        let mut statement = connection
            .prepare("SELECT id FROM session")
            .map_err(|e| e.to_string())?;
        for id in statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
        {
            let id = id.map_err(|e| e.to_string())?;
            if !emitted.contains(&id) && filter.is_none_or(|ids| ids.contains(&id)) {
                missing.insert(id);
            }
        }
        let mut visitor_error = None;
        let result = load_v1(
            &connection,
            Some(&missing),
            &mut |row| {
                unreadable.remove(&row.source_id);
                candidate(row).inspect_err(|error| visitor_error = Some(error.clone()))
            },
            progress,
        );
        if let Some(error) = visitor_error {
            return Err(error);
        }
        match result {
            Ok(notes) => warnings.extend(notes),
            Err(error) if !emitted.is_empty() => warnings.push(error),
            Err(error) => return Err(error),
        }
    }
    if !v1 && !v2 {
        return Err("OpenCode session schema is not recognized".into());
    }
    if !unreadable.is_empty() {
        warnings.push(format!(
            "{} empty OpenCode sessions have no readable history in either format",
            unreadable.len()
        ));
    }
    Ok(warnings)
}

fn load_v2(
    connection: &Connection,
    filter: Option<&HashSet<String>>,
    candidate: &mut dyn FnMut(Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(Vec<String>, HashSet<String>), String> {
    let mut warnings = Vec::new();
    let mut session_statement = connection
        .prepare(
            "SELECT id, title, directory, model, time_updated
             FROM session_v2
             WHERE parent_id IS NULL
             ORDER BY time_updated DESC",
        )
        .map_err(|_| "OpenCode v2 session schema is not recognized".to_string())?;
    let sessions = session_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| "OpenCode v2 sessions could not be read".to_string())?
        .filter(|row| {
            row.as_ref()
                .map_or(true, |row| filter.is_none_or(|ids| ids.contains(&row.0)))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "OpenCode v2 sessions could not be decoded".to_string())?;

    let total = sessions.len();
    progress(0, total);
    let mut skipped = HashSet::new();
    for (index, (id, title, directory, model, updated)) in sessions.into_iter().enumerate() {
        progress(index + 1, total);
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
                    let text = data["text"].as_str().unwrap_or_default();
                    if !text.trim().is_empty()
                        || data["files"]
                            .as_array()
                            .is_some_and(|files| !files.is_empty())
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
                "system" | "synthetic" => {
                    if let Some(text) = data["text"].as_str().filter(|text| !text.trim().is_empty())
                    {
                        transcript.push(text_item("record", text, at));
                    }
                }
                "compaction" => {
                    if let Some(text) = data["summary"]
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                    {
                        transcript.push(text_item("record", text, at));
                    }
                }
                _ => {}
            }
        }
        finish_turn(&mut transcript, &mut turn, &mut usage);
        if transcript.is_empty() {
            skipped.insert(id);
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
        candidate(Candidate {
            source_id: id,
            title,
            directory: (!directory.trim().is_empty()).then_some(directory),
            model: model_name(model.as_deref()),
            transcript,
            at_ms: ms(Some(updated)),
        })?;
    }
    Ok((warnings, skipped))
}

fn load_v1(
    connection: &Connection,
    filter: Option<&HashSet<String>>,
    candidate: &mut dyn FnMut(Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    // Early V1 databases have no session-level model column.
    let model = if connection
        .prepare("SELECT model FROM session LIMIT 0")
        .is_ok()
    {
        "model"
    } else {
        "NULL"
    };
    let mut session_statement = connection
        .prepare(&format!(
            "SELECT id, title, directory, {model}, time_updated
             FROM session
             WHERE parent_id IS NULL
             ORDER BY time_updated DESC"
        ))
        .or_else(|_| {
            connection.prepare(&format!(
                "SELECT id, title, directory, {model}, time_updated
                 FROM session
                  ORDER BY time_updated DESC"
            ))
        })
        .map_err(|_| "OpenCode session schema is not recognized".to_string())?;
    let sessions = session_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| "OpenCode sessions could not be read".to_string())?
        .filter(|row| {
            row.as_ref()
                .map_or(true, |row| filter.is_none_or(|ids| ids.contains(&row.0)))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "OpenCode sessions could not be decoded".to_string())?;

    let total = sessions.len();
    progress(0, total);
    for (index, (id, title, directory, mut model, updated)) in sessions.into_iter().enumerate() {
        progress(index + 1, total);
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
            if let Some(name) = data["modelID"].as_str() {
                model = Some(serde_json::json!({"id": name}).to_string());
            }
            let at = ms(data.pointer("/time/created").and_then(Value::as_i64));
            if role == "user" {
                finish_turn(&mut transcript, &mut turn, &mut usage);
            }
            let before_len = transcript.len();
            for part in parts.get(&row.0).into_iter().flatten() {
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
        candidate(Candidate {
            source_id: id,
            title,
            directory: (!directory.trim().is_empty()).then_some(directory),
            model: model_name(model.as_deref()),
            transcript,
            at_ms: ms(Some(updated)),
        })?;
    }
    Ok(warnings)
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
        speaker: None,
        failure: None,
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
    let (additions, deletions, locations, diff, file_changes) = tool_changes(state);
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
            file_changes,
        },
        duration_ms: 0,
        at_ms: at,
    }
}

fn tool_changes(
    state: &Value,
) -> (
    u32,
    u32,
    Vec<String>,
    Option<String>,
    Vec<dunlin_agent::FileChange>,
) {
    let metadata = &state["metadata"];
    let mut additions = 0u32;
    let mut deletions = 0u32;
    let mut locations = Vec::new();
    let mut diff = None;
    let mut file_changes = Vec::new();
    let mut consume = |file: &Value| {
        let patch = file["patch"].as_str();
        let (patch_additions, patch_deletions) = patch.map(diff_counts).unwrap_or_default();
        let file_additions = file["additions"]
            .as_u64()
            .map(|value| value.min(u32::MAX as u64) as u32)
            .unwrap_or(patch_additions);
        let file_deletions = file["deletions"]
            .as_u64()
            .map(|value| value.min(u32::MAX as u64) as u32)
            .unwrap_or(patch_deletions);
        additions = additions.saturating_add(file_additions);
        deletions = deletions.saturating_add(file_deletions);
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
        if let Some(path) = path.filter(|path| !path.trim().is_empty())
            && (file_additions > 0 || file_deletions > 0)
        {
            // Preserve authoritative per-file counters even when the
            // bounded chat preview cannot retain the entire patch.
            let recorded = patch.filter(|patch| patch.len() <= 16 * 1024).unwrap_or("");
            let recorded = if recorded.starts_with("@@") {
                format!("--- a/{path}\n+++ b/{path}\n{recorded}")
            } else {
                recorded.into()
            };
            file_changes.push(dunlin_agent::FileChange {
                path: path.into(),
                additions: file_additions,
                deletions: file_deletions,
                diff: recorded,
            });
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
        file_changes = crate::timeline::parse_patch(patch);
        for change in &mut file_changes {
            if change.diff.len() > 16 * 1024 {
                change.diff.clear();
            }
        }
    }
    (additions, deletions, locations, diff, file_changes)
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
        .or_else(|| {
            model
                .filter(|model| !model.trim().is_empty() && !model.starts_with('{'))
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
    #[test]
    fn imported_file_counts_survive_bounded_diff_previews_without_losing_files() {
        let large = format!("@@ -0,0 +1,20000 @@\n{}", "+line\n".repeat(20_000));
        let state = serde_json::json!({ "metadata": { "files": [
            {"file":"large.rs", "additions":20000, "deletions":0, "patch":large},
            {"file":"small.rs", "additions":1, "deletions":1, "patch":"@@ -1 +1 @@\n-old\n+new\n"}
        ]}});
        let (added, removed, locations, _, changes) = super::tool_changes(&state);
        assert_eq!((added, removed), (20_001, 1));
        assert_eq!(locations.len(), 2);
        assert_eq!(changes.len(), 2);
        assert!(changes[0].diff.is_empty());
        assert!(changes[1].diff.starts_with("--- a/small.rs"));
    }

    use super::{load, load_selected};
    use crate::harness::ChatItem;
    use rusqlite::Connection;
    use std::collections::HashSet;

    #[test]
    fn mixed_database_imports_both_versions_with_per_session_fallback_and_no_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE session_v2 (id TEXT, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
             CREATE TABLE session_message (session_id TEXT, type TEXT, seq INTEGER, data TEXT);
             CREATE TABLE session (id TEXT, title TEXT, directory TEXT, parent_id TEXT, time_updated INTEGER);
             CREATE TABLE message (id TEXT, session_id TEXT, data TEXT, time_created INTEGER);
             CREATE TABLE part (message_id TEXT, session_id TEXT, id TEXT, data TEXT, time_created INTEGER);
             INSERT INTO session_v2 VALUES ('shared', 'V2', '/repo', NULL, NULL, 3), ('shell', 'Migrated', '/repo', NULL, NULL, 2);
             INSERT INTO session_message VALUES ('shared', 'user', 1, '{\"text\":\"new history\"}');
             INSERT INTO session VALUES ('shared', 'Old', '/repo', NULL, 1), ('shell', 'Fallback', '/repo', NULL, 1), ('legacy', 'Legacy', '/repo', NULL, 1);
             INSERT INTO message VALUES ('m1', 'shared', '{\"role\":\"user\"}', 1), ('m2', 'shell', '{\"role\":\"user\"}', 1), ('m3', 'legacy', '{\"role\":\"assistant\",\"modelID\":\"old-model\"}', 1);
             INSERT INTO part VALUES ('m1', 'shared', 'p1', '{\"type\":\"text\",\"text\":\"old history\"}', 1), ('m2', 'shell', 'p2', '{\"type\":\"text\",\"text\":\"recovered\"}', 1), ('m3', 'legacy', 'p3', '{\"type\":\"text\",\"text\":\"legacy history\"}', 1);"
        ).unwrap();
        let (sessions, warnings) = load(&path).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sessions.len(), 3);
        let shared = sessions.iter().find(|s| s.source_id == "shared").unwrap();
        assert!(
            matches!(&shared.transcript[0], ChatItem::User { text, .. } if text == "new history")
        );
        let fallback = sessions.iter().find(|s| s.source_id == "shell").unwrap();
        assert!(
            matches!(&fallback.transcript[0], ChatItem::User { text, .. } if text == "recovered")
        );
        assert_eq!(
            sessions
                .iter()
                .find(|s| s.source_id == "legacy")
                .unwrap()
                .model,
            "old-model"
        );
        let (selected, _) = load_selected(&path, &HashSet::from(["shell".into()])).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].source_id, "shell");
    }

    #[test]
    fn both_database_schemas_read_all_sessions_including_old_selected_histories() {
        for v2 in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("opencode.db");
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(if v2 {
                "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
                 CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER, data TEXT);"
            } else {
                "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, time_created INTEGER);
                 CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, data TEXT, time_created INTEGER);"
            }).unwrap();
            let transaction = connection.unchecked_transaction().unwrap();
            for index in 0..600 {
                let id = format!("s{index}");
                transaction
                    .execute(
                        &format!(
                            "INSERT INTO {} VALUES (?1, ?1, '/repo', NULL, NULL, ?2)",
                            if v2 { "session_v2" } else { "session" }
                        ),
                        rusqlite::params![id, index],
                    )
                    .unwrap();
                if v2 {
                    transaction.execute("INSERT INTO session_message VALUES (?1, ?1, 'user', 1, '{\"text\":\"hello\"}')", [&id]).unwrap();
                } else {
                    transaction
                        .execute(
                            "INSERT INTO message VALUES (?1, ?1, '{\"role\":\"user\"}', 1)",
                            [&id],
                        )
                        .unwrap();
                    transaction.execute("INSERT INTO part VALUES (?1, ?1, ?1, '{\"type\":\"text\",\"text\":\"hello\"}', 1)", [&id]).unwrap();
                }
            }
            transaction.commit().unwrap();
            let mut progress = Vec::new();
            let mut count = 0;
            let warnings = super::visit(
                &path,
                None,
                &mut |_| {
                    count += 1;
                    Ok(())
                },
                &mut |done, total| progress.push((done, total)),
            )
            .unwrap();
            assert_eq!(count, 600);
            assert!(warnings.is_empty());
            assert_eq!(progress.first(), Some(&(0, 600)));
            assert_eq!(progress.last(), Some(&(600, 600)));
            let (sessions, warnings) = load_selected(&path, &HashSet::from(["s0".into()])).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(sessions.len(), 1);
            assert_eq!(
                sessions[0].source_id, "s0",
                "selection is applied before limiting/reading transcripts"
            );
        }
    }

    #[test]
    fn visitor_failure_is_not_swallowed_or_retried_against_another_schema() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE session_v2 (id TEXT, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
             CREATE TABLE session_message (session_id TEXT, type TEXT, seq INTEGER, data TEXT);
             INSERT INTO session_v2 VALUES ('s1', 'Test', '/repo', NULL, NULL, 1);
             INSERT INTO session_message VALUES ('s1', 'user', 1, '{\"text\":\"hello\"}');"
        ).unwrap();
        let error = super::visit(
            &path,
            None,
            &mut |_| Err("Import cache is full".into()),
            &mut |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error, "Import cache is full");
    }

    #[test]
    fn empty_v2_histories_still_fall_back_to_readable_legacy_sessions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE session_v2 (id TEXT, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
             CREATE TABLE session_message (session_id TEXT, type TEXT, seq INTEGER, data TEXT);
             INSERT INTO session_v2 VALUES ('empty', 'Empty', '/repo', NULL, NULL, 1);
             CREATE TABLE session (id TEXT, title TEXT, directory TEXT, model TEXT, parent_id TEXT, time_updated INTEGER);
             CREATE TABLE message (id TEXT, session_id TEXT, data TEXT, time_created INTEGER);
             CREATE TABLE part (message_id TEXT, session_id TEXT, id TEXT, data TEXT, time_created INTEGER);
             INSERT INTO session VALUES ('legacy', 'Legacy', '/repo', NULL, NULL, 1);
             INSERT INTO message VALUES ('message', 'legacy', '{\"role\":\"user\"}', 1);
             INSERT INTO part VALUES ('message', 'legacy', 'part', '{\"type\":\"text\",\"text\":\"hello\"}', 1);"
        ).unwrap();
        let (sessions, warnings) = load(&path).unwrap();
        assert_eq!(
            warnings.len(),
            1,
            "only the genuinely empty shell is reported"
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source_id, "legacy");
    }

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
                ChatItem::User { timing, .. } => timing.clone(),
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
                ChatItem::User { timing, .. } => timing.clone(),
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
