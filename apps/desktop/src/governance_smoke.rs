//! End-to-end smoke for the disabled-by-default governance WASM pack.
//!
//! It proves the host can discover a plugin manifest without loading WASM,
//! explicitly load and hash-check the module, validate governance objects and
//! build a bounded context envelope.

use dunlin_knowledge::{
    EnvelopeOptions, KnowledgeEdge, KnowledgeGraph, KnowledgeObject, build_envelope,
};
use dunlin_plugin_host::PluginRegistry;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) fn run() -> Result<(), String> {
    let root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist"));
    let root = if root.join("plugin.json").is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root
    };
    let registry = PluginRegistry::discover(&root);
    for warning in &registry.warnings {
        eprintln!("warning: {warning}");
    }
    let plugin = registry
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.wasm.is_some())
        .ok_or_else(|| format!("No WASM plugin found under {}", root.display()))?;
    println!(
        "manifest: {} v{} api {} (wasm not loaded yet)",
        plugin.manifest.id, plugin.manifest.version, plugin.manifest.api
    );
    println!("loading wasm...");
    let wasm = plugin.load().map_err(|error| error.to_string())?;
    println!("wasm loaded");
    let schema = wasm.describe().map_err(|error| error.to_string())?;
    println!("schema described");
    schema
        .validate()
        .map_err(|error| format!("plugin schema: {error}"))?;
    println!(
        "schema: {} kinds, {} relations",
        schema.kinds.len(),
        schema.relations.len()
    );

    let objects = sample_objects();
    let mut graph = KnowledgeGraph::default();
    for object in &objects {
        let errors = wasm
            .validate(&object.kind, object)
            .map_err(|error| error.to_string())?;
        if !errors.is_empty() {
            return Err(format!("{}: {}", object.id, errors.join("; ")));
        }
        graph.objects.push(object.clone());
    }
    graph.edges = vec![
        KnowledgeEdge {
            from_id: "INT-001".into(),
            to_id: "CON-001".into(),
            relation: "has_concept".into(),
            metadata: BTreeMap::new(),
        },
        KnowledgeEdge {
            from_id: "INT-001".into(),
            to_id: "DEC-001".into(),
            relation: "resolved_by".into(),
            metadata: BTreeMap::new(),
        },
        KnowledgeEdge {
            from_id: "CON-001".into(),
            to_id: "INT-001".into(),
            relation: "realizes".into(),
            metadata: BTreeMap::new(),
        },
    ];
    let issues = dunlin_knowledge::validate_graph(&schema, &graph);
    for issue in &issues {
        eprintln!("graph {:?}: {}", issue.severity, issue.message);
    }
    if issues
        .iter()
        .any(|issue| issue.severity == dunlin_knowledge::Severity::Error)
    {
        return Err("graph validation failed".into());
    }
    let envelope = build_envelope(&schema, &graph, &EnvelopeOptions::default());
    println!("envelope hash: {}", envelope.hash);
    println!("envelope bytes: {}", envelope.text.len());
    println!("{}", envelope.text);
    Ok(())
}

fn sample_objects() -> Vec<KnowledgeObject> {
    vec![
        KnowledgeObject {
            id: "INT-001".into(),
            project_id: "smoke".into(),
            kind: "intent".into(),
            title: "Agent-first knowledge spine".into(),
            status: "accepted".into(),
            path: Some("intent/2026-11-08-agent-first.md".into()),
            revision: "rev-int".into(),
            updated_ms: 1,
            fields: BTreeMap::from([
                (
                    "problem".into(),
                    json!("Agents and humans lack a shared, durable intent."),
                ),
                (
                    "outcome".into(),
                    json!("Every run is linked to an accepted intent and evidence."),
                ),
            ]),
            body: "The spine is the source of truth; chat is a view.".into(),
            plugin_id: Some("dunlin-governance".into()),
            schema_version: 1,
        },
        KnowledgeObject {
            id: "CON-001".into(),
            project_id: "smoke".into(),
            kind: "concept".into(),
            title: "Workspace".into(),
            status: "accepted".into(),
            path: Some("concepts/CON-001-workspace.md".into()),
            revision: "rev-con".into(),
            updated_ms: 1,
            fields: BTreeMap::from([(
                "definition".into(),
                json!("A durable place for project knowledge and execution."),
            )]),
            body: String::new(),
            plugin_id: Some("dunlin-governance".into()),
            schema_version: 1,
        },
        KnowledgeObject {
            id: "DEC-001".into(),
            project_id: "smoke".into(),
            kind: "decision".into(),
            title: "File-first knowledge".into(),
            status: "accepted".into(),
            path: Some("decisions/DEC-001-file-first.md".into()),
            revision: "rev-dec".into(),
            updated_ms: 1,
            fields: BTreeMap::from([
                (
                    "context".into(),
                    json!("Knowledge must survive app upgrades."),
                ),
                (
                    "decision".into(),
                    json!("Markdown is canonical; SQLite is an index."),
                ),
            ]),
            body: String::new(),
            plugin_id: Some("dunlin-governance".into()),
            schema_version: 1,
        },
    ]
}
