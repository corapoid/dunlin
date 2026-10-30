//! Explicit file formats only. No harness databases, commands or credential reads.
use crate::harness::ChatItem;
use serde_json::Value;
use std::path::{Component, Path};

pub const MAX_FILE: u64 = 16 * 1024 * 1024;
pub const MAX_SESSIONS: usize = 256;
#[derive(Clone)]
pub struct Candidate {
    pub source_id: String,
    pub title: String,
    pub directory: Option<String>,
    pub model: String,
    pub transcript: Vec<ChatItem>,
    pub at_ms: u64,
}
fn text(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.into();
    }
    if let Some(parts) = value.as_array() {
        return parts
            .iter()
            .map(|p| {
                p.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "[Imported {}]\n{}",
                            p["type"].as_str().unwrap_or("content"),
                            p
                        )
                    })
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if value.is_null() {
        String::new()
    } else {
        value.to_string()
    }
}
fn item(role: &str, content: String, at_ms: u64) -> ChatItem {
    match role {
        "user" => ChatItem::User {
            text: content,
            timing: None,
            at_ms,
        },
        "reasoning" | "thinking" => ChatItem::Thought {
            text: content,
            streaming: false,
            message_id: None,
            at_ms,
        },
        _ => ChatItem::AgentText {
            text: content,
            streaming: false,
            message_id: None,
            at_ms,
        },
    }
}
fn time(v: &Value) -> u64 {
    v.as_u64().unwrap_or_else(|| {
        v.as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis().max(0) as u64)
            .unwrap_or(0)
    })
}
fn string(v: &Value, key: &str) -> Option<String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}
fn model(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .or_else(|| string(v, "modelID"))
        .or_else(|| string(v, "id"))
        .unwrap_or_else(|| "Auto".into())
}

pub fn parse(harness: &str, bytes: &[u8]) -> Result<Vec<Candidate>, String> {
    if bytes.len() as u64 > MAX_FILE {
        return Err("Session file exceeds 16 MiB".into());
    }
    let source = std::str::from_utf8(bytes).map_err(|_| "Session file must be UTF-8 JSON/JSONL")?;
    let single = serde_json::from_str::<Value>(source).ok();
    let mut result = if matches!(harness, "Claude Code" | "OpenAI Codex")
        && !single
            .as_ref()
            .is_some_and(|v| v["format"] == "dunlin.session.v1" || v.is_array())
    {
        vec![jsonl(harness, source)?]
    } else if let Some(value) = single {
        let values = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        if values.len() > MAX_SESSIONS {
            return Err("At most 256 sessions per preview".into());
        }
        values
            .iter()
            .map(|v| match harness {
                "OpenCode" => opencode(v),
                _ => portable(v),
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![jsonl(harness, source)?]
    };
    for candidate in &mut result {
        if candidate.source_id.is_empty() || candidate.source_id.len() > 512 {
            return Err("Missing or oversized source session ID".into());
        }
        if candidate.transcript.is_empty() {
            return Err("No transcript messages in this session file".into());
        }
        if candidate.transcript.len() > 30_000 {
            return Err("Session has more than 30,000 transcript entries".into());
        }
        if candidate.title.trim().is_empty() {
            candidate.title = "Imported conversation".into();
        }
    }
    Ok(result)
}

fn opencode(value: &Value) -> Result<Candidate, String> {
    // OpenCode V2 experimental.session.export: {data:{info,messages}}.
    let data = value.get("data").unwrap_or(value);
    let info = &data["info"];
    let messages = data["messages"]
        .as_array()
        .ok_or("Expected OpenCode export with info and messages")?;
    let mut transcript = Vec::new();
    for message in messages {
        // V2 projected messages have type/text/content; CLI exports with info/parts
        // are accepted explicitly too, without guessing a local DB schema.
        let meta = message.get("info").unwrap_or(message);
        let role = meta["type"]
            .as_str()
            .or_else(|| meta["role"].as_str())
            .unwrap_or("record");
        let at = time(&meta["time"]["created"]);
        if let Some(s) = message["text"].as_str() {
            let mut content = if matches!(role, "user" | "assistant") {
                s.to_string()
            } else {
                format!("[Imported {role}]\n{s}")
            };
            for key in ["files", "agents", "skills"] {
                if message[key].as_array().is_some_and(|a| !a.is_empty()) {
                    content.push_str(&format!("\n[Imported {key}]\n{}", message[key]));
                }
            }
            transcript.push(item(role, content, at));
        } else if let Some(parts) = message["content"]
            .as_array()
            .or_else(|| message["parts"].as_array())
        {
            for part in parts {
                let kind = part["type"].as_str().unwrap_or("record");
                let role = if matches!(kind, "reasoning" | "thinking") {
                    kind
                } else {
                    role
                };
                let content =
                    string(part, "text").unwrap_or_else(|| format!("[Imported {kind}]\n{part}"));
                transcript.push(item(role, content, at));
            }
        } else {
            transcript.push(item("record", format!("[Imported {role}]\n{message}"), at));
        }
    }
    Ok(Candidate {
        source_id: string(info, "id").ok_or("Missing OpenCode session ID")?,
        title: string(info, "title").unwrap_or_default(),
        directory: string(&info["location"], "directory").or_else(|| string(info, "directory")),
        model: model(&info["model"]),
        transcript,
        at_ms: time(&info["time"]["updated"]),
    })
}

/// Portable format for custom ACP adapters; documented in docs/session-import.md.
fn portable(value: &Value) -> Result<Candidate, String> {
    if value["format"] != "dunlin.session.v1" {
        return Err(
            "Expected a dunlin.session.v1 JSON export or the selected harness's JSONL log".into(),
        );
    }
    let messages = value["messages"].as_array().ok_or("Missing messages")?;
    let transcript = messages
        .iter()
        .map(|m| {
            item(
                m["role"].as_str().unwrap_or("assistant"),
                text(&m["content"]),
                time(&m["timestamp"]),
            )
        })
        .collect();
    Ok(Candidate {
        source_id: string(value, "id").ok_or("Missing session ID")?,
        title: string(value, "title").unwrap_or_default(),
        directory: string(value, "directory"),
        model: model(&value["model"]),
        transcript,
        at_ms: time(&value["updated_at"]),
    })
}
fn jsonl(harness: &str, source: &str) -> Result<Candidate, String> {
    if !matches!(harness, "Claude Code" | "OpenAI Codex") {
        return Err("Choose an OpenCode export JSON or portable dunlin.session.v1 JSON".into());
    }
    let mut candidate = Candidate {
        source_id: String::new(),
        title: String::new(),
        directory: None,
        model: "Auto".into(),
        transcript: Vec::new(),
        at_ms: 0,
    };
    let mut records = 0;
    for line in source.lines().filter(|s| !s.trim().is_empty()) {
        records += 1;
        if records > 30_000 {
            return Err("Too many JSONL records".into());
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("Invalid JSONL record {records}: {e}"))?;
        let kind = v["type"].as_str().unwrap_or("");
        let at = time(&v["timestamp"]);
        candidate.at_ms = candidate.at_ms.max(at);
        if harness == "Claude Code" {
            if let Some(id) = string(&v, "sessionId") {
                if !candidate.source_id.is_empty() && candidate.source_id != id {
                    return Err(
                        "Multiple Claude session IDs in one file; choose one session log per file"
                            .into(),
                    );
                }
                candidate.source_id = id;
            }
            if let Some(cwd) = string(&v, "cwd") {
                candidate.directory = Some(cwd);
            }
            if let Some(m) = string(&v["message"], "model") {
                candidate.model = m;
            }
            if matches!(kind, "user" | "assistant") {
                candidate
                    .transcript
                    .push(item(kind, text(&v["message"]["content"]), at));
            } else {
                candidate
                    .transcript
                    .push(item("record", format!("[Imported {kind}]\n{v}"), at));
            }
        } else {
            let payload = &v["payload"];
            if kind == "session_meta" {
                let id = string(payload, "id").ok_or("Codex session_meta is missing an ID")?;
                if !candidate.source_id.is_empty() && candidate.source_id != id {
                    return Err("Multiple Codex session IDs in one file".into());
                }
                candidate.source_id = id;
                candidate.directory = string(payload, "cwd");
            }
            if kind == "turn_context"
                && let Some(m) = string(payload, "model")
            {
                candidate.model = m;
            }
            // response_item is the authoritative transcript. event_msg mirrors it
            // and is retained as metadata rather than duplicate user messages.
            if kind == "response_item" && payload["type"] == "message" {
                candidate.transcript.push(item(
                    payload["role"].as_str().unwrap_or("assistant"),
                    text(&payload["content"]),
                    at,
                ));
            } else {
                candidate
                    .transcript
                    .push(item("record", format!("[Imported {kind}]\n{v}"), at));
            }
        }
    }
    if candidate.source_id.is_empty() {
        return Err("No source session ID found".into());
    }
    candidate.title = candidate
        .transcript
        .iter()
        .find_map(|m| {
            if let ChatItem::User { text, .. } = m {
                Some(text.chars().take(100).collect())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "Imported conversation".into());
    Ok(candidate)
}

fn canonical(path: &Path) -> Option<std::path::PathBuf> {
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    std::fs::canonicalize(path).ok()
}
/// Only existing canonical local paths. Deepest containing project wins; aliases
/// of the same root are ambiguous. Names and missing/moved paths never auto-match.
pub fn match_project(
    directory: Option<&str>,
    projects: &[dunlin_domain::Project],
) -> Option<String> {
    let directory = canonical(Path::new(directory?))?;
    let mut matches: Vec<_> = projects
        .iter()
        .filter(|p| !p.is_remote())
        .filter_map(|p| {
            let root = canonical(Path::new(&p.path))?;
            directory
                .starts_with(&root)
                .then_some((root.components().count(), p.id.clone()))
        })
        .collect();
    matches.sort_by_key(|a| std::cmp::Reverse(a.0));
    let best = matches.first()?;
    if matches.get(1).is_some_and(|next| next.0 == best.0) {
        None
    } else {
        Some(best.1.clone())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn matching_uses_canonical_containment_not_names_and_rejects_ambiguity() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("dunlin-match-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(root.join("repo/nested/src")).unwrap();
        fs::create_dir_all(root.join("repo-other")).unwrap();
        let project = |id: &str, suffix: &str| dunlin_domain::Project {
            id: id.into(),
            name: "same name".into(),
            path: root.join(suffix).to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: false,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        };
        let parent = project("parent", "repo");
        let child = project("child", "repo/nested");
        let cwd = root.join("repo/nested/src").to_string_lossy().to_string();
        assert_eq!(
            super::match_project(Some(&cwd), &[parent.clone(), child.clone()]).as_deref(),
            Some("child")
        );
        assert!(
            super::match_project(
                Some(root.join("repo-other").to_str().unwrap()),
                std::slice::from_ref(&parent)
            )
            .is_none()
        );
        assert!(
            super::match_project(Some("relative/repo"), std::slice::from_ref(&parent)).is_none()
        );
        assert!(
            super::match_project(
                Some(root.join("missing").to_str().unwrap()),
                std::slice::from_ref(&parent)
            )
            .is_none()
        );
        let mut remote = child.clone();
        remote.remote = Some(dunlin_domain::RemoteProject::default());
        assert!(super::match_project(Some(&cwd), &[remote]).is_none());
        let alias = project("duplicate-root", "repo/nested");
        assert!(super::match_project(Some(&cwd), &[child, alias]).is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn portable_format_and_single_record_jsonl_are_supported_but_wrong_formats_are_rejected() {
        let custom = json!({"format":"dunlin.session.v1", "id":"custom-one", "directory":"/work", "messages":[{"role":"user", "content":"Question"},{"role":"assistant", "content":"Answer"}]});
        let bytes = serde_json::to_vec(&custom).unwrap();
        assert_eq!(
            super::parse("Custom ACP", &bytes).unwrap()[0]
                .transcript
                .len(),
            2
        );
        let single = br#"{"type":"user","sessionId":"single","message":{"content":"Hello"}}"#;
        assert_eq!(
            super::parse("Claude Code", single).unwrap()[0].source_id,
            "single"
        );
        assert!(super::parse("Custom ACP", br#"{"id":"bad","messages":[]}"#).is_err());
        assert!(super::parse("Custom ACP", &vec![b' '; super::MAX_FILE as usize + 1]).is_err());
    }
    #[test]
    fn opencode_v2_export_preserves_text_reasoning_tools_and_directory() {
        let value = json!({"data":{"info":{"id":"ses_one","title":"Hello","location":{"directory":"/work/project"}},"messages":[{"type":"user","text":"Question","time":{"created":123}},{"type":"assistant","content":[{"type":"text","text":"Answer"},{"type":"reasoning","text":"Thought"},{"type":"tool","name":"shell","output":"done"}]}]}});
        let rows = super::parse("OpenCode", &serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(rows[0].directory.as_deref(), Some("/work/project"));
        assert_eq!(rows[0].transcript.len(), 4);
        assert!(
            matches!(&rows[0].transcript[2], crate::harness::ChatItem::Thought {text,..} if text == "Thought")
        );
    }
    #[test]
    fn jsonl_adapters_use_source_identity_and_never_execute_tools() {
        let claude = "{\"type\":\"user\",\"sessionId\":\"abc\",\"cwd\":\"/work\",\"message\":{\"content\":\"Hi\"}}\n{\"type\":\"assistant\",\"sessionId\":\"abc\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"exit 1\"}}]}}\n";
        let rows = super::parse("Claude Code", claude.as_bytes()).unwrap();
        assert_eq!(rows[0].source_id, "abc");
        assert_eq!(rows[0].transcript.len(), 2);
        let codex = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"xyz\",\"cwd\":\"/repo\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Hi\"}]}}\n";
        assert_eq!(
            super::parse("OpenAI Codex", codex.as_bytes()).unwrap()[0].source_id,
            "xyz"
        );
        assert!(super::parse("OpenCode", claude.as_bytes()).is_err());
        assert!(super::parse("Claude Code", b"not json").is_err());
    }
}
