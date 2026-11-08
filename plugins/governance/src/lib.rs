//! Governance WASM plugin: Intent / Concept / Decision schema and pure logic.
//!
//! The host owns files, SQLite, UI and permissions. This module only validates
//! and renders objects. It has no imports, no WASI, no filesystem and no
//! network access.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::alloc::{Layout, alloc, dealloc};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Request {
    #[serde(default = "default_api")]
    api: u32,
    op: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    input: Value,
}

fn default_api() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    output: Value,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_alloc(len: u32) -> u32 {
    let len = len.max(1) as usize;
    let layout = match Layout::from_size_align(len, 1) {
        Ok(layout) => layout,
        Err(_) => return 0,
    };
    // SAFETY: layout has non-zero size and alignment 1.
    let ptr = unsafe { alloc(layout) };
    ptr as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_free(ptr: u32, len: u32) {
    if ptr == 0 {
        return;
    }
    let len = len.max(1) as usize;
    if let Ok(layout) = Layout::from_size_align(len, 1) {
        // SAFETY: the host only frees allocations returned by dunlin_alloc with
        // the same length.
        unsafe { dealloc(ptr as *mut u8, layout) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_call(ptr: u32, len: u32) -> u64 {
    let input = if ptr == 0 || len == 0 {
        &[][..]
    } else {
        // SAFETY: the host writes a request of exactly `len` bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
    };
    let response = match serde_json::from_slice::<Request>(input) {
        Ok(request) => handle(request),
        Err(error) => Response {
            ok: false,
            output: Value::Null,
            warnings: Vec::new(),
            error: Some(format!("invalid request: {error}")),
        },
    };
    let bytes = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(error) => format!(
            "{{\"ok\":false,\"output\":null,\"warnings\":[],\"error\":\"serialize: {error}\"}}"
        )
        .into_bytes(),
    };
    let out_ptr = dunlin_alloc(bytes.len() as u32);
    if out_ptr == 0 {
        return 0;
    }
    // SAFETY: dunlin_alloc returned at least bytes.len() writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr as *mut u8, bytes.len()) };
    ((out_ptr as u64) << 32) | bytes.len() as u64
}

fn handle(request: Request) -> Response {
    if request.api != 1 {
        return fail(format!("unsupported plugin API {}", request.api));
    }
    let output = match request.op.as_str() {
        "describe" => Ok(json!({"schema_text": SCHEMA})),
        "validate" => validate(&request),
        "parse" => parse_document(&request.input),
        "migrate" => Ok(json!({"migrated": true, "object": request.input})),
        "context_section" => context_section(&request.input),
        "template" => Ok(json!({"text": template(request.kind.as_deref().unwrap_or("intent"))})),
        other => Err(format!("unsupported operation {other}")),
    };
    match output {
        Ok(output) => Response {
            ok: true,
            output,
            warnings: Vec::new(),
            error: None,
        },
        Err(error) => fail(error),
    }
}

fn fail(error: impl Into<String>) -> Response {
    Response {
        ok: false,
        output: Value::Null,
        warnings: Vec::new(),
        error: Some(error.into()),
    }
}

const SCHEMA: &str = r#"{
  "api": 1,
  "kinds": [
    {
      "id": "intent",
      "label": "Intent",
      "directory": "intent",
      "statuses": ["draft", "clarifying", "accepted", "implementing", "converged", "superseded"],
      "default_status": "draft",
      "fields": [
        {"id": "problem", "label": "Problem", "kind": "markdown", "required": true},
        {"id": "outcome", "label": "Proposed outcome", "kind": "markdown", "required": true},
        {"id": "users", "label": "Affected users and systems", "kind": "markdown", "required": false},
        {"id": "constraints", "label": "Constraints", "kind": "markdown", "required": false},
        {"id": "owners", "label": "Owners", "kind": "list", "required": false}
      ],
      "relations": ["has_concept", "resolved_by", "supersedes"]
    },
    {
      "id": "concept",
      "label": "Concept",
      "directory": "concepts",
      "statuses": ["proposed", "accepted", "deprecated"],
      "default_status": "proposed",
      "fields": [
        {"id": "definition", "label": "Definition", "kind": "markdown", "required": true},
        {"id": "invariants", "label": "Invariants", "kind": "list", "required": false},
        {"id": "owners", "label": "Owners", "kind": "list", "required": false}
      ],
      "relations": ["depends_on", "constrains", "realizes", "supersedes"]
    },
    {
      "id": "decision",
      "label": "Decision",
      "directory": "decisions",
      "statuses": ["proposed", "accepted", "superseded"],
      "default_status": "proposed",
      "fields": [
        {"id": "context", "label": "Context", "kind": "markdown", "required": true},
        {"id": "decision", "label": "Decision", "kind": "markdown", "required": true},
        {"id": "consequences", "label": "Consequences", "kind": "markdown", "required": false},
        {"id": "owners", "label": "Owners", "kind": "list", "required": false}
      ],
      "relations": ["resolves", "affects", "supersedes"]
    }
  ],
  "relations": [
    {"id": "has_concept", "label": "has concept", "from_kinds": ["intent"], "to_kinds": ["concept"]},
    {"id": "resolved_by", "label": "resolved by", "from_kinds": ["intent"], "to_kinds": ["decision"]},
    {"id": "supersedes", "label": "supersedes", "from_kinds": ["intent", "concept", "decision"], "to_kinds": ["intent", "concept", "decision"]},
    {"id": "depends_on", "label": "depends on", "from_kinds": ["concept", "decision"], "to_kinds": ["concept", "decision"]},
    {"id": "constrains", "label": "constrains", "from_kinds": ["concept", "decision"], "to_kinds": ["concept", "decision"]},
    {"id": "realizes", "label": "realizes", "from_kinds": ["concept"], "to_kinds": ["intent"]},
    {"id": "resolves", "label": "resolves", "from_kinds": ["decision"], "to_kinds": ["intent"]},
    {"id": "affects", "label": "affects", "from_kinds": ["decision"], "to_kinds": ["concept"]}
  ]
}"#;

fn validate(request: &Request) -> Result<Value, String> {
    let kind = request
        .kind
        .as_deref()
        .or_else(|| request.input.get("kind").and_then(Value::as_str))
        .ok_or("missing kind")?;
    let object = request.input.get("object").unwrap_or(&request.input);
    let id = object.get("id").and_then(Value::as_str).unwrap_or("");
    let title = object.get("title").and_then(Value::as_str).unwrap_or("");
    let status = object.get("status").and_then(Value::as_str).unwrap_or("");
    let fields = object.get("fields").cloned().unwrap_or_else(|| json!({}));
    let mut errors = Vec::new();
    if id.trim().is_empty() {
        errors.push("missing id".to_string());
    }
    if title.trim().is_empty() {
        errors.push("missing title".to_string());
    }
    let required: &[&str] = match kind {
        "intent" => &["problem", "outcome"],
        "concept" => &["definition"],
        "decision" => &["context", "decision"],
        _ => &[],
    };
    for field in required {
        if fields
            .get(*field)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            errors.push(format!("missing required field {field}"));
        }
    }
    let statuses: &[&str] = match kind {
        "intent" => &[
            "draft",
            "clarifying",
            "accepted",
            "implementing",
            "converged",
            "superseded",
        ],
        "concept" => &["proposed", "accepted", "deprecated"],
        "decision" => &["proposed", "accepted", "superseded"],
        _ => &[],
    };
    if !statuses.is_empty() && !statuses.iter().any(|known| *known == status) {
        errors.push(format!("invalid status {status} for kind {kind}"));
    }
    Ok(json!({"errors": errors}))
}

fn parse_document(input: &Value) -> Result<Value, String> {
    let source = input
        .get("source")
        .and_then(Value::as_str)
        .ok_or("parse requires source")?;
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("document.md");
    let (frontmatter, body) = split_frontmatter(source);
    let mut object = serde_json::Map::new();
    object.insert("path".into(), json!(path));
    object.insert("body".into(), json!(body));
    object.insert("frontmatter".into(), json!(frontmatter));
    Ok(json!({"object": object}))
}

fn context_section(input: &Value) -> Result<Value, String> {
    let object = input.get("object").unwrap_or(input);
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let title = object.get("title").and_then(Value::as_str).unwrap_or("");
    let status = object.get("status").and_then(Value::as_str).unwrap_or("");
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
    let body = object.get("body").and_then(Value::as_str).unwrap_or("");
    Ok(json!({
        "text": format!("### {id} · {status} · {title}\n{}\n", body.trim())
            .trim_end()
            .to_string(),
        "kind": kind
    }))
}

fn template(kind: &str) -> String {
    match kind {
        "concept" => "---\ndunlin: 1\nkind: concept\nid: CON-001\ntitle: \nstatus: proposed\n---\n\n## Definition\n\n## Invariants\n".into(),
        "decision" => "---\ndunlin: 1\nkind: decision\nid: DEC-001\ntitle: \nstatus: proposed\n---\n\n## Context\n\n## Decision\n\n## Consequences\n".into(),
        _ => "---\ndunlin: 1\nkind: intent\nid: INT-001\ntitle: \nstatus: draft\n---\n\n## Problem\n\n## Proposed outcome\n\n## Affected users and systems\n\n## Constraints\n\n## Open questions\n".into(),
    }
}

fn split_frontmatter(source: &str) -> (Value, String) {
    let mut lines = source.lines();
    if lines.next() != Some("---") {
        return (json!({}), source.to_string());
    }
    let mut frontmatter = serde_json::Map::new();
    let mut body = String::new();
    let mut in_body = false;
    for line in lines {
        if !in_body && line == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body.push_str(line);
            body.push('\n');
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            if !key.is_empty() {
                frontmatter.insert(key.to_string(), json!(value));
            }
        }
    }
    (Value::Object(frontmatter), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_json_and_contains_governance_kinds() {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        let kinds = schema["kinds"].as_array().unwrap();
        assert_eq!(kinds.len(), 3);
        assert_eq!(kinds[0]["id"], "intent");
    }

    #[test]
    fn validation_requires_kind_fields_and_status() {
        let request = Request {
            api: 1,
            op: "validate".into(),
            kind: Some("concept".into()),
            schema_version: Some(1),
            input: json!({
                "id": "CON-1",
                "title": "Workspace",
                "status": "accepted",
                "fields": {"definition": "A shared place"}
            }),
        };
        let response = handle(request);
        assert!(response.ok);
        assert_eq!(response.output["errors"].as_array().unwrap().len(), 0);

        let bad = Request {
            api: 1,
            op: "validate".into(),
            kind: Some("concept".into()),
            schema_version: Some(1),
            input: json!({"id": "CON-1", "title": "Workspace", "status": "accepted"}),
        };
        let response = handle(bad);
        assert!(response.ok);
        assert!(!response.output["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn context_section_is_markdown() {
        let response = handle(Request {
            api: 1,
            op: "context_section".into(),
            kind: Some("intent".into()),
            schema_version: Some(1),
            input: json!({"object": {"id": "INT-1", "title": "Intent", "status": "accepted", "kind": "intent", "body": "Why"}}),
        });
        assert!(response.ok);
        assert!(response.output["text"].as_str().unwrap().contains("INT-1"));
    }
}
