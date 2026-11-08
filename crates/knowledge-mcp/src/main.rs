//! Minimal stdio MCP server for the governance knowledge pack.
//!
//! Read tools are project-scoped and bounded. Write tools never modify project
//! files: they enqueue pending proposals for a human to accept in Dunlin.

use dunlin_knowledge::{
    EnvelopeOptions, KnowledgeObject, KnowledgeSchema, build_envelope, discover,
};
use dunlin_plugin_host::{PluginRegistry, WasmPlugin};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Server {
    project: PathBuf,
    project_id: String,
    plugin: WasmPlugin,
    schema: KnowledgeSchema,
    proposals: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("dunlin-knowledge-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project = arg(&args, "--project")
        .or_else(|| std::env::var("DUNLIN_PROJECT_ROOT").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let plugin_root = arg(&args, "--plugin")
        .or_else(|| std::env::var("DUNLIN_GOVERNANCE_PLUGIN").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist"));
    let plugin_root = if plugin_root.join("plugin.json").is_file() {
        plugin_root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        plugin_root
    };
    let registry = PluginRegistry::discover(&plugin_root);
    let installed = registry
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.wasm.is_some())
        .ok_or_else(|| format!("No WASM governance plugin under {}", plugin_root.display()))?;
    let plugin = installed.load().map_err(|error| error.to_string())?;
    let schema = plugin.describe().map_err(|error| error.to_string())?;
    schema
        .validate()
        .map_err(|error| format!("plugin schema: {error}"))?;
    let project = project
        .canonicalize()
        .map_err(|error| format!("project folder: {error}"))?;
    let project_id = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string();
    let proposals = proposal_dir()?;
    std::fs::create_dir_all(&proposals).map_err(|error| format!("proposal directory: {error}"))?;
    let server = Server {
        project,
        project_id,
        plugin,
        schema,
        proposals,
    };
    server.serve()
}

impl Server {
    fn serve(&self) -> Result<(), String> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let line = line.map_err(|error| format!("stdin: {error}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(error) => {
                    write_response(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": Value::Null,
                            "error": {"code": -32700, "message": format!("invalid JSON: {error}")}
                        }),
                    )?;
                    continue;
                }
            };
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if method.starts_with("notifications/") {
                continue;
            }
            let response = match method {
                "initialize" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "dunlin-knowledge", "version": "0.1.0"}
                    }
                }),
                "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
                "tools/list" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"tools": tools()}
                }),
                "tools/call" => match self.call_tool(&request) {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(error) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": error}],
                            "isError": true
                        }
                    }),
                },
                other => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("unknown method {other}")}
                }),
            };
            write_response(&mut stdout, response)?;
        }
        Ok(())
    }

    fn call_tool(&self, request: &Value) -> Result<Value, String> {
        let params = request
            .get("params")
            .ok_or_else(|| "missing params".to_string())?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing tool name".to_string())?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match name {
            "knowledge_context" => {
                let (graph, warnings) = discover(
                    &self.project,
                    &self.schema,
                    &self.project_id,
                    Some("dunlin-governance"),
                )?;
                let envelope = build_envelope(&self.schema, &graph, &EnvelopeOptions::default());
                let text = format!(
                    "{}\n\n<!-- envelope sha256: {}; objects: {}; warnings: {} -->",
                    envelope.text,
                    envelope.hash,
                    graph.objects.len(),
                    warnings.len()
                );
                Ok(text_result(text))
            }
            "object_list" => {
                let kind = arguments.get("kind").and_then(Value::as_str);
                let (graph, _) = discover(
                    &self.project,
                    &self.schema,
                    &self.project_id,
                    Some("dunlin-governance"),
                )?;
                let objects: Vec<_> = graph
                    .objects
                    .iter()
                    .filter(|object| kind.is_none_or(|kind| object.kind == kind))
                    .take(500)
                    .collect();
                Ok(text_result(
                    serde_json::to_string_pretty(&objects).unwrap_or_default(),
                ))
            }
            "object_get" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "object_get requires id".to_string())?;
                let (graph, _) = discover(
                    &self.project,
                    &self.schema,
                    &self.project_id,
                    Some("dunlin-governance"),
                )?;
                let object = graph
                    .get(id)
                    .ok_or_else(|| format!("object {id} not found"))?;
                Ok(text_result(
                    serde_json::to_string_pretty(object).unwrap_or_default(),
                ))
            }
            "object_propose" => self.propose_object(&arguments),
            "relation_propose" => self.propose_relation(&arguments),
            "evidence_link" => self.propose_evidence_link(&arguments),
            other => Err(format!("unknown tool {other}")),
        }
    }

    fn propose_object(&self, arguments: &Value) -> Result<Value, String> {
        let kind = arguments
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "object_propose requires kind".to_string())?;
        let kind_def = self
            .schema
            .kind(kind)
            .ok_or_else(|| format!("unknown knowledge kind {kind}"))?;
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .ok_or_else(|| "object_propose requires a title".to_string())?;
        let id = arguments
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("PROP-{}", now_ms()));
        let status = arguments
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(&kind_def.default_status)
            .to_string();
        let fields = arguments
            .get("fields")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let fields: BTreeMap<String, Value> = serde_json::from_value(fields)
            .map_err(|error| format!("fields must be an object: {error}"))?;
        let object = KnowledgeObject {
            id: id.clone(),
            project_id: self.project_id.clone(),
            kind: kind.to_string(),
            title: title.to_string(),
            status,
            path: None,
            revision: format!("proposal-{}", now_ms()),
            updated_ms: now_ms() as i64,
            fields,
            body: arguments
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            plugin_id: Some("dunlin-governance".into()),
            schema_version: 1,
        };
        let errors = self
            .plugin
            .validate(kind, &object)
            .map_err(|error| error.to_string())?;
        if !errors.is_empty() {
            return Err(format!("proposal rejected: {}", errors.join("; ")));
        }
        let path = self.write_proposal(json!({
            "kind": "object",
            "status": "pending",
            "created_ms": now_ms(),
            "project_id": self.project_id,
            "object": object,
        }))?;
        Ok(text_result(format!(
            "Pending object proposal {id} written to {}. No project file was changed; a human must accept it in Dunlin.",
            path.display()
        )))
    }

    fn propose_relation(&self, arguments: &Value) -> Result<Value, String> {
        let from = arguments
            .get("from")
            .and_then(Value::as_str)
            .ok_or_else(|| "relation_propose requires from".to_string())?;
        let to = arguments
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| "relation_propose requires to".to_string())?;
        let relation = arguments
            .get("relation")
            .and_then(Value::as_str)
            .ok_or_else(|| "relation_propose requires relation".to_string())?;
        if self.schema.relation(relation).is_none() {
            return Err(format!("unknown relation {relation}"));
        }
        let path = self.write_proposal(json!({
            "kind": "relation",
            "status": "pending",
            "created_ms": now_ms(),
            "project_id": self.project_id,
            "from": from,
            "to": to,
            "relation": relation,
        }))?;
        Ok(text_result(format!(
            "Pending relation proposal written to {}. No project file was changed.",
            path.display()
        )))
    }

    fn propose_evidence_link(&self, arguments: &Value) -> Result<Value, String> {
        let evidence = arguments
            .get("evidence")
            .and_then(Value::as_str)
            .ok_or_else(|| "evidence_link requires evidence".to_string())?;
        let claim = arguments
            .get("claim")
            .and_then(Value::as_str)
            .ok_or_else(|| "evidence_link requires claim".to_string())?;
        let relation = arguments
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("supports");
        if !matches!(relation, "supports" | "refutes" | "context") {
            return Err("evidence relation must be supports, refutes or context".into());
        }
        let path = self.write_proposal(json!({
            "kind": "evidence_link",
            "status": "pending",
            "created_ms": now_ms(),
            "project_id": self.project_id,
            "evidence": evidence,
            "claim": claim,
            "relation": relation,
        }))?;
        Ok(text_result(format!(
            "Pending evidence link written to {}. No claim or file was changed.",
            path.display()
        )))
    }

    fn write_proposal(&self, proposal: Value) -> Result<PathBuf, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "{}-{}-{}",
            now_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed),
            proposal
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("proposal")
        );
        let path = self.proposals.join(format!("{id}.json"));
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&proposal).map_err(|e| e.to_string())?,
        )
        .map_err(|error| format!("write proposal: {error}"))?;
        Ok(path)
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "knowledge_context",
            "description": "Read the bounded intent/concept/decision context envelope for this project.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "object_list",
            "description": "List indexed knowledge objects, optionally filtered by kind.",
            "inputSchema": {
                "type": "object",
                "properties": {"kind": {"type": "string"}},
                "additionalProperties": false
            }
        },
        {
            "name": "object_get",
            "description": "Read one indexed knowledge object by id.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "object_propose",
            "description": "Propose a new intent, concept or decision. Writes a pending proposal only; a human must accept it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string"},
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "status": {"type": "string"},
                    "fields": {"type": "object"},
                    "body": {"type": "string"}
                },
                "required": ["kind", "title"],
                "additionalProperties": false
            }
        },
        {
            "name": "relation_propose",
            "description": "Propose a typed relation between knowledge objects. Writes a pending proposal only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": {"type": "string"},
                    "to": {"type": "string"},
                    "relation": {"type": "string"}
                },
                "required": ["from", "to", "relation"],
                "additionalProperties": false
            }
        },
        {
            "name": "evidence_link",
            "description": "Propose a link between evidence and a claim. Writes a pending proposal only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "evidence": {"type": "string"},
                    "claim": {"type": "string"},
                    "relation": {"type": "string", "enum": ["supports", "refutes", "context"]}
                },
                "required": ["evidence", "claim"],
                "additionalProperties": false
            }
        }
    ])
}

fn text_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn write_response(stdout: &mut std::io::Stdout, response: Value) -> Result<(), String> {
    writeln!(
        stdout,
        "{}",
        serde_json::to_string(&response).map_err(|e| e.to_string())?
    )
    .map_err(|error| format!("stdout: {error}"))?;
    stdout.flush().map_err(|error| format!("stdout: {error}"))
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn proposal_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    let home = PathBuf::from(home);
    let data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Dunlin")
    } else {
        home.join(".local/share/dunlin")
    };
    Ok(data.join("proposals"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_is_stable_and_bounded() {
        let tools = tools();
        let tools = tools.as_array().unwrap();
        assert_eq!(tools.len(), 6);
        assert!(tools.iter().any(|tool| tool["name"] == "knowledge_context"));
        assert!(tools.iter().any(|tool| tool["name"] == "object_propose"));
    }

    #[test]
    fn text_results_are_mcp_content_blocks() {
        let result = text_result("hello".into());
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "hello");
        assert_eq!(result["isError"], false);
    }
}
