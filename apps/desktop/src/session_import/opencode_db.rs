//! Read-only import of OpenCode's local session database.
//!
//! Only `session`, `message` and `part` rows are read. Credentials, blobs,
//! snapshots and the writable OpenCode state are never touched.

use super::adapters::Candidate;
use crate::harness::ChatItem;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MAX_SESSIONS: usize = 256;
const MAX_MESSAGES_PER_SESSION: usize = 30_000;
const MAX_PARTS_PER_SESSION: usize = 60_000;
const MAX_TOOL_TEXT: usize = 16_384;

pub(super) fn load(path: &Path) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "OpenCode database is locked or unavailable".to_string())?;
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
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
        for row in message_rows.flatten() {
            let Ok(data) = serde_json::from_str::<Value>(&row.1) else {
                continue;
            };
            let role = data["role"].as_str().unwrap_or("assistant");
            let at = ms(data.pointer("/time/created").and_then(Value::as_i64));
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
                        let text = tool_text(part);
                        if !text.trim().is_empty() {
                            transcript.push(text_item("tool", &text, at));
                        }
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
        }
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

fn tool_text(part: &Value) -> String {
    let tool = part["tool"].as_str().unwrap_or("tool");
    let state = &part["state"];
    let mut text = format!("[Tool · {tool}]");
    if let Some(title) = state["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
    {
        text.push(' ');
        text.push_str(title);
    }
    if let Some(status) = state["status"].as_str() {
        text.push_str(&format!("\nStatus: {status}"));
        if status != "completed"
            && let Some(error) = state["error"].as_str()
        {
            text.push('\n');
            text.push_str(error);
        }
    }
    if let Some(input) = state.get("input").filter(|value| !value.is_null()) {
        text.push_str("\nInput: ");
        text.push_str(&truncate(&input.to_string(), 4096));
    }
    if let Some(output) = state.get("output").filter(|value| !value.is_null()) {
        text.push_str("\nOutput: ");
        text.push_str(&truncate(&output.to_string(), MAX_TOOL_TEXT));
    }
    text
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
    use super::load;
    use rusqlite::Connection;

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
                    ('m2', 's1', '{"role":"assistant","time":{"created":1700000001000}}', 2),
                    ('m3', 's2', '{"role":"user"}', 3);
                INSERT INTO part VALUES
                    ('p1', 'm1', 's1', '{"type":"text","text":"hello"}', 1),
                    ('p2', 'm2', 's1', '{"type":"reasoning","text":"thought"}', 2),
                    ('p3', 'm2', 's1', '{"type":"tool","tool":"bash","state":{"status":"completed","input":{"cmd":"ls"},"output":"ok"}}', 3),
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
        assert!(warnings.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
