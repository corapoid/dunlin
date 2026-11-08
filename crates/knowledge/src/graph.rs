use crate::model::{KnowledgeGraph, KnowledgeSchema, MAX_EDGES, MAX_OBJECTS};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphIssue {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

impl GraphIssue {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }
}

pub fn validate_graph(schema: &KnowledgeSchema, graph: &KnowledgeGraph) -> Vec<GraphIssue> {
    let mut issues = Vec::new();
    if graph.objects.len() > MAX_OBJECTS {
        issues.push(GraphIssue::error(
            "objects_limit",
            format!("Knowledge graph exceeds {MAX_OBJECTS} objects"),
        ));
    }
    if graph.edges.len() > MAX_EDGES {
        issues.push(GraphIssue::error(
            "edges_limit",
            format!("Knowledge graph exceeds {MAX_EDGES} edges"),
        ));
    }

    let mut ids = HashSet::new();
    for object in &graph.objects {
        if !ids.insert(object.id.as_str()) {
            issues.push(GraphIssue::error(
                "duplicate_object",
                format!("Duplicate knowledge object {}", object.id),
            ));
        }
        for error in object.validate(schema) {
            issues.push(GraphIssue::error("invalid_object", error));
        }
    }

    for edge in &graph.edges {
        if edge.from_id == edge.to_id {
            issues.push(GraphIssue::warning(
                "self_edge",
                format!("{} {} itself", edge.from_id, edge.relation),
            ));
        }
        let Some(relation) = schema.relation(&edge.relation) else {
            issues.push(GraphIssue::error(
                "unknown_relation",
                format!("Unknown relation {}", edge.relation),
            ));
            continue;
        };
        let Some(from) = graph.get(&edge.from_id) else {
            issues.push(GraphIssue::error(
                "orphan_edge",
                format!("Edge {} has no source object", edge.relation),
            ));
            continue;
        };
        let Some(to) = graph.get(&edge.to_id) else {
            issues.push(GraphIssue::error(
                "orphan_edge",
                format!("Edge {} has no target object {}", edge.relation, edge.to_id),
            ));
            continue;
        };
        if !relation.from_kinds.is_empty()
            && !relation.from_kinds.iter().any(|kind| kind == &from.kind)
        {
            issues.push(GraphIssue::error(
                "relation_source_kind",
                format!(
                    "Relation {} cannot start at kind {}",
                    edge.relation, from.kind
                ),
            ));
        }
        if !relation.to_kinds.is_empty() && !relation.to_kinds.iter().any(|kind| kind == &to.kind) {
            issues.push(GraphIssue::error(
                "relation_target_kind",
                format!("Relation {} cannot end at kind {}", edge.relation, to.kind),
            ));
        }
    }

    if let Err(message) = topological_order(graph, "depends_on") {
        issues.push(GraphIssue::error("dependency_cycle", message));
    }
    issues
}

/// Kahn's algorithm over one relation. Returns a stable order; the caller may
/// use it for task dispatch. Cycles are reported with the remaining ids.
pub fn topological_order(graph: &KnowledgeGraph, relation: &str) -> Result<Vec<String>, String> {
    let mut nodes: Vec<&str> = graph
        .objects
        .iter()
        .map(|object| object.id.as_str())
        .collect();
    nodes.sort_unstable();
    let mut indegree: HashMap<&str, usize> = nodes.iter().map(|id| (*id, 0)).collect();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    let known: HashSet<&str> = nodes.iter().copied().collect();
    for edge in graph.edges.iter().filter(|edge| edge.relation == relation) {
        if !known.contains(edge.from_id.as_str()) || !known.contains(edge.to_id.as_str()) {
            continue;
        }
        if edge.from_id == edge.to_id {
            return Err(format!("{} depends on itself", edge.from_id));
        }
        outgoing
            .entry(edge.from_id.as_str())
            .or_default()
            .push(edge.to_id.as_str());
        *indegree.entry(edge.to_id.as_str()).or_default() += 1;
    }
    let mut ready: Vec<&str> = nodes
        .iter()
        .copied()
        .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    ready.sort_unstable();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(id) = ready.first().copied() {
        ready.remove(0);
        order.push(id.to_string());
        if let Some(targets) = outgoing.get(id) {
            let mut next = Vec::new();
            for target in targets {
                let degree = indegree.get_mut(target).expect("known target");
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    next.push(*target);
                }
            }
            next.sort_unstable();
            ready.extend(next);
            ready.sort_unstable();
        }
    }
    if order.len() != nodes.len() {
        let remaining: Vec<_> = nodes
            .into_iter()
            .filter(|id| !order.iter().any(|done| done == id))
            .collect();
        return Err(format!(
            "Dependency cycle involves: {}",
            remaining.join(", ")
        ));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        FieldDef, FieldKind, KnowledgeEdge, KnowledgeKind, KnowledgeObject, KnowledgeSchema,
        RelationDef,
    };
    use std::collections::BTreeMap;

    fn schema() -> KnowledgeSchema {
        KnowledgeSchema {
            api: 1,
            kinds: vec![
                KnowledgeKind {
                    id: "intent".into(),
                    label: "Intent".into(),
                    directory: "intent".into(),
                    statuses: vec!["accepted".into()],
                    default_status: "accepted".into(),
                    fields: Vec::new(),
                    relations: vec!["has_concept".into()],
                },
                KnowledgeKind {
                    id: "concept".into(),
                    label: "Concept".into(),
                    directory: "concepts".into(),
                    statuses: vec!["accepted".into()],
                    default_status: "accepted".into(),
                    fields: vec![FieldDef {
                        id: "problem".into(),
                        label: "Problem".into(),
                        kind: FieldKind::Markdown,
                        required: true,
                        values: Vec::new(),
                    }],
                    relations: Vec::new(),
                },
                KnowledgeKind {
                    id: "task".into(),
                    label: "Task".into(),
                    directory: "tasks".into(),
                    statuses: vec!["todo".into(), "done".into()],
                    default_status: "todo".into(),
                    fields: Vec::new(),
                    relations: Vec::new(),
                },
            ],
            relations: vec![
                RelationDef {
                    id: "has_concept".into(),
                    label: "has concept".into(),
                    from_kinds: vec!["intent".into()],
                    to_kinds: vec!["concept".into()],
                },
                RelationDef {
                    id: "depends_on".into(),
                    label: "depends on".into(),
                    from_kinds: vec!["task".into()],
                    to_kinds: vec!["task".into()],
                },
            ],
        }
    }

    fn object(id: &str, kind: &str) -> KnowledgeObject {
        KnowledgeObject {
            id: id.into(),
            project_id: "p".into(),
            kind: kind.into(),
            title: id.into(),
            status: if kind == "task" {
                "todo".into()
            } else {
                "accepted".into()
            },
            path: None,
            revision: "r".into(),
            updated_ms: 0,
            fields: if kind == "concept" {
                BTreeMap::from([("problem".into(), serde_json::json!("why"))])
            } else {
                BTreeMap::new()
            },
            body: String::new(),
            plugin_id: None,
            schema_version: 1,
        }
    }

    #[test]
    fn graph_reports_orphans_unknown_relations_and_bad_kinds() {
        let graph = KnowledgeGraph {
            objects: vec![object("INT-1", "intent"), object("CON-1", "concept")],
            edges: vec![
                KnowledgeEdge {
                    from_id: "INT-1".into(),
                    to_id: "CON-1".into(),
                    relation: "has_concept".into(),
                    metadata: BTreeMap::new(),
                },
                KnowledgeEdge {
                    from_id: "INT-1".into(),
                    to_id: "missing".into(),
                    relation: "has_concept".into(),
                    metadata: BTreeMap::new(),
                },
                KnowledgeEdge {
                    from_id: "INT-1".into(),
                    to_id: "CON-1".into(),
                    relation: "unknown".into(),
                    metadata: BTreeMap::new(),
                },
            ],
        };
        let issues = validate_graph(&schema(), &graph);
        assert!(issues.iter().any(|issue| issue.code == "orphan_edge"));
        assert!(issues.iter().any(|issue| issue.code == "unknown_relation"));
    }

    #[test]
    fn dependency_order_is_stable_and_cycles_are_reported() {
        let mut a = object("TASK-1", "task");
        a.status = "todo".into();
        let graph = KnowledgeGraph {
            objects: vec![a, object("TASK-2", "task"), object("TASK-3", "task")],
            edges: vec![
                KnowledgeEdge {
                    from_id: "TASK-1".into(),
                    to_id: "TASK-2".into(),
                    relation: "depends_on".into(),
                    metadata: BTreeMap::new(),
                },
                KnowledgeEdge {
                    from_id: "TASK-2".into(),
                    to_id: "TASK-3".into(),
                    relation: "depends_on".into(),
                    metadata: BTreeMap::new(),
                },
            ],
        };
        let order = topological_order(&graph, "depends_on").unwrap();
        assert_eq!(order, vec!["TASK-1", "TASK-2", "TASK-3"]);

        let cyclic = KnowledgeGraph {
            objects: vec![object("TASK-1", "task"), object("TASK-2", "task")],
            edges: vec![
                KnowledgeEdge {
                    from_id: "TASK-1".into(),
                    to_id: "TASK-2".into(),
                    relation: "depends_on".into(),
                    metadata: BTreeMap::new(),
                },
                KnowledgeEdge {
                    from_id: "TASK-2".into(),
                    to_id: "TASK-1".into(),
                    relation: "depends_on".into(),
                    metadata: BTreeMap::new(),
                },
            ],
        };
        assert!(topological_order(&cyclic, "depends_on").is_err());
    }
}
