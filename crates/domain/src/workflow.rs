//! Versioned, bounded workflow graphs. No execution, filesystem paths, network
//! discovery, arbitrary code nodes or permission grants are accepted here.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

pub const WORKFLOW_SCHEMA_VERSION: u16 = 1;
const MAX_NODES: usize = 256;
const MAX_TOTAL_NODES: usize = 512;
const MAX_EDGES: usize = 1024;
const MAX_DEPTH: usize = 4;

/// Deliberately a bounded v1 port vocabulary, not a full JSON Schema evaluator.
/// Provider-specific structured outputs still need their own schema validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    Text,
    Boolean,
    Integer,
    Number,
    Json,
    Artifact,
}

impl PortType {
    pub fn accepts(self, source: Self) -> bool {
        self == source || self == Self::Json || (self == Self::Number && source == Self::Integer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub kind: PortType,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TriggerSource {
    Manual,
    Linear {
        connection_id: String,
        label_id: String,
    },
    Schedule {
        schedule_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeKind {
    Trigger {
        source: TriggerSource,
    },
    Agent {
        agent_version_id: String,
    },
    McpTool {
        connection_id: String,
        tool_name: String,
    },
    A2aAgent {
        connection_id: String,
        skill_id: String,
    },
    Workspace {
        binding_id: String,
    },
    Condition {
        input_port: String,
    },
    Parallel,
    Join,
    HumanReview {
        minimum_approvals: u8,
        separation_of_duties: bool,
    },
    Wait {
        event_type: String,
        timeout_seconds: u32,
    },
    Repeat {
        body: Box<WorkflowDefinition>,
        until_output: String,
        max_iterations: u16,
    },
    Publish {
        integration_id: String,
        target: String,
    },
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub inputs: BTreeMap<String, Port>,
    pub outputs: BTreeMap<String, Port>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeOutcome {
    Always,
    True,
    False,
    Approved,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub outcome: EdgeOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortBinding {
    pub from_node: String,
    pub from_port: String,
    pub to_node: String,
    pub to_port: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputBinding {
    pub node: String,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLimits {
    pub max_duration_seconds: u32,
    pub max_agent_turns: u16,
    pub max_step_attempts: u8,
    pub max_concurrent_steps: u8,
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            max_duration_seconds: 3600,
            max_agent_turns: 128,
            max_step_attempts: 3,
            max_concurrent_steps: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub schema_version: u16,
    pub name: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub bindings: Vec<PortBinding>,
    pub outputs: BTreeMap<String, OutputBinding>,
    pub limits: WorkflowLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

fn invalid(path: impl Into<String>, message: &str) -> ValidationError {
    ValidationError {
        path: path.into(),
        message: message.into(),
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn reference(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}

impl WorkflowDefinition {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.validate_at("workflow", 0, &mut 0)
    }

    fn validate_at(
        &self,
        path: &str,
        depth: usize,
        total: &mut usize,
    ) -> Result<(), ValidationError> {
        if depth >= MAX_DEPTH {
            return Err(invalid(path, "Repeat nesting exceeds the limit"));
        }
        *total += self.nodes.len();
        if self.schema_version != WORKFLOW_SCHEMA_VERSION {
            return Err(invalid(path, "Unsupported workflow schema version"));
        }
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(invalid(path, "A bounded workflow name is required"));
        }
        if self.nodes.is_empty()
            || self.nodes.len() > MAX_NODES
            || *total > MAX_TOTAL_NODES
            || self.edges.len() > MAX_EDGES
            || self.bindings.len() > MAX_EDGES
            || self.outputs.len() > 64
        {
            return Err(invalid(path, "Workflow graph exceeds its size limit"));
        }
        let limits = &self.limits;
        if limits.max_duration_seconds == 0
            || limits.max_duration_seconds > 7 * 24 * 3600
            || limits.max_agent_turns == 0
            || limits.max_agent_turns > 1000
            || limits.max_step_attempts == 0
            || limits.max_step_attempts > 8
            || limits.max_concurrent_steps == 0
            || limits.max_concurrent_steps > 16
        {
            return Err(invalid(
                format!("{path}.limits"),
                "Execution limits are missing or too large",
            ));
        }

        let mut nodes = BTreeMap::new();
        let mut triggers = Vec::new();
        let mut ends = Vec::new();
        for node in &self.nodes {
            let node_path = format!("{path}.nodes.{}", node.id);
            if !identifier(&node.id) || nodes.insert(node.id.as_str(), node).is_some() {
                return Err(invalid(
                    node_path,
                    "Node ids must be bounded, valid and unique",
                ));
            }
            if node.name.trim().is_empty()
                || node.name.len() > 256
                || node.inputs.len() > 64
                || node.outputs.len() > 64
                || node
                    .inputs
                    .keys()
                    .chain(node.outputs.keys())
                    .any(|key| !identifier(key))
            {
                return Err(invalid(node_path, "Invalid node name or port declarations"));
            }
            match &node.kind {
                NodeKind::Trigger { source } => {
                    triggers.push(node.id.as_str());
                    let valid = match source {
                        TriggerSource::Manual => true,
                        TriggerSource::Linear {
                            connection_id,
                            label_id,
                        } => reference(connection_id) && reference(label_id),
                        TriggerSource::Schedule { schedule_id } => reference(schedule_id),
                    };
                    if !valid {
                        return Err(invalid(node_path, "Trigger references are invalid"));
                    }
                }
                NodeKind::Agent { agent_version_id } if !reference(agent_version_id) => {
                    return Err(invalid(node_path, "An immutable agent version is required"));
                }
                NodeKind::McpTool {
                    connection_id,
                    tool_name,
                } if !reference(connection_id) || !reference(tool_name) => {
                    return Err(invalid(node_path, "MCP connection and tool are required"));
                }
                NodeKind::A2aAgent {
                    connection_id,
                    skill_id,
                } if !reference(connection_id) || !reference(skill_id) => {
                    return Err(invalid(node_path, "A2A connection and skill are required"));
                }
                NodeKind::Workspace { binding_id } if !reference(binding_id) => {
                    return Err(invalid(
                        node_path,
                        "An explicit project binding is required",
                    ));
                }
                NodeKind::Condition { input_port }
                    if !node
                        .inputs
                        .get(input_port)
                        .is_some_and(|port| port.kind == PortType::Boolean) =>
                {
                    return Err(invalid(
                        node_path,
                        "Condition requires a declared boolean input",
                    ));
                }
                NodeKind::HumanReview {
                    minimum_approvals, ..
                } if *minimum_approvals == 0 || *minimum_approvals > 16 => {
                    return Err(invalid(
                        node_path,
                        "Review must require between 1 and 16 human approvals",
                    ));
                }
                NodeKind::Wait {
                    event_type,
                    timeout_seconds,
                } if !reference(event_type)
                    || *timeout_seconds == 0
                    || *timeout_seconds > limits.max_duration_seconds =>
                {
                    return Err(invalid(
                        node_path,
                        "Wait needs an explicit event and a bounded timeout",
                    ));
                }
                NodeKind::Publish {
                    integration_id,
                    target,
                } if !reference(integration_id) || !reference(target) => {
                    return Err(invalid(
                        node_path,
                        "Publishing requires an explicit integration and target",
                    ));
                }
                NodeKind::Repeat {
                    body,
                    until_output,
                    max_iterations,
                } => {
                    if *max_iterations == 0 || *max_iterations > 20 {
                        return Err(invalid(node_path, "Repeat requires 1–20 iterations"));
                    }
                    body.validate_at(&format!("{node_path}.body"), depth + 1, total)?;
                    let output = body.outputs.get(until_output).and_then(|binding| {
                        body.nodes
                            .iter()
                            .find(|candidate| candidate.id == binding.node)
                            .and_then(|candidate| candidate.outputs.get(&binding.port))
                    });
                    if !output.is_some_and(|port| port.kind == PortType::Boolean) {
                        return Err(invalid(
                            node_path,
                            "Repeat until_output must refer to a boolean workflow output",
                        ));
                    }
                }
                NodeKind::Complete => ends.push(node.id.as_str()),
                _ => {}
            }
        }
        if triggers.len() != 1 || ends.len() != 1 {
            return Err(invalid(
                path,
                "Workflow needs exactly one trigger and one completion node",
            ));
        }

        let mut outgoing: BTreeMap<&str, Vec<&str>> =
            nodes.keys().map(|id| (*id, Vec::new())).collect();
        let mut incoming = outgoing.clone();
        let mut seen_edges = BTreeSet::new();
        for edge in &self.edges {
            if !nodes.contains_key(edge.from.as_str())
                || !nodes.contains_key(edge.to.as_str())
                || edge.from == edge.to
            {
                return Err(invalid(
                    format!("{path}.edges"),
                    "Edges must connect two existing, distinct nodes",
                ));
            }
            if !seen_edges.insert((&edge.from, &edge.to, edge.outcome)) {
                return Err(invalid(format!("{path}.edges"), "Duplicate control edge"));
            }
            let valid_outcome = match nodes[edge.from.as_str()].kind {
                NodeKind::Condition { .. } => {
                    matches!(edge.outcome, EdgeOutcome::True | EdgeOutcome::False)
                }
                NodeKind::HumanReview { .. } => {
                    matches!(edge.outcome, EdgeOutcome::Approved | EdgeOutcome::Rejected)
                }
                _ => edge.outcome == EdgeOutcome::Always,
            };
            if !valid_outcome {
                return Err(invalid(
                    format!("{path}.edges"),
                    "Edge outcome does not match its source node",
                ));
            }
            outgoing
                .get_mut(edge.from.as_str())
                .expect("validated source")
                .push(&edge.to);
            incoming
                .get_mut(edge.to.as_str())
                .expect("validated target")
                .push(&edge.from);
        }
        if !incoming[triggers[0]].is_empty() || !outgoing[ends[0]].is_empty() {
            return Err(invalid(
                path,
                "Trigger cannot have predecessors and completion cannot have successors",
            ));
        }
        let mut degrees: BTreeMap<&str, usize> = incoming
            .iter()
            .map(|(id, edges)| (*id, edges.len()))
            .collect();
        let mut ready: VecDeque<&str> = degrees
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect();
        let mut visited = 0;
        while let Some(id) = ready.pop_front() {
            visited += 1;
            for child in &outgoing[id] {
                let count = degrees.get_mut(child).expect("validated target");
                *count -= 1;
                if *count == 0 {
                    ready.push_back(child);
                }
            }
        }
        if visited != self.nodes.len() {
            return Err(invalid(
                path,
                "Unbounded control cycle; use an explicit Repeat node",
            ));
        }
        if reachable(triggers[0], &outgoing).len() != self.nodes.len()
            || reachable(ends[0], &incoming).len() != self.nodes.len()
        {
            return Err(invalid(
                path,
                "Every node must be reachable from trigger and lead to completion",
            ));
        }

        let mut bound = BTreeSet::new();
        for binding in &self.bindings {
            let source = nodes
                .get(binding.from_node.as_str())
                .and_then(|node| node.outputs.get(&binding.from_port));
            let target = nodes
                .get(binding.to_node.as_str())
                .and_then(|node| node.inputs.get(&binding.to_port));
            let (Some(source), Some(target)) = (source, target) else {
                return Err(invalid(
                    format!("{path}.bindings"),
                    "Binding refers to an undeclared node or port",
                ));
            };
            if !target.kind.accepts(source.kind) {
                return Err(invalid(
                    format!("{path}.bindings"),
                    "Binding has incompatible port types",
                ));
            }
            if binding.from_node == binding.to_node
                || !reachable(&binding.from_node, &outgoing).contains(binding.to_node.as_str())
            {
                return Err(invalid(
                    format!("{path}.bindings"),
                    "Data producer must be a control predecessor of its consumer",
                ));
            }
            if !bound.insert((&binding.to_node, &binding.to_port)) {
                return Err(invalid(
                    format!("{path}.bindings"),
                    "An input cannot have multiple producers",
                ));
            }
        }
        for node in &self.nodes {
            let outcomes: BTreeSet<EdgeOutcome> = self
                .edges
                .iter()
                .filter(|edge| edge.from == node.id)
                .map(|edge| edge.outcome)
                .collect();
            if matches!(node.kind, NodeKind::Condition { .. })
                && outcomes != [EdgeOutcome::True, EdgeOutcome::False].into()
            {
                return Err(invalid(
                    path,
                    "Condition requires true and false successors",
                ));
            }
            if matches!(node.kind, NodeKind::HumanReview { .. })
                && outcomes != [EdgeOutcome::Approved, EdgeOutcome::Rejected].into()
            {
                return Err(invalid(
                    path,
                    "Human review requires approved and rejected successors",
                ));
            }
            if !matches!(node.kind, NodeKind::Trigger { .. }) {
                for (name, port) in &node.inputs {
                    if port.required && !bound.contains(&(&node.id, name)) {
                        return Err(invalid(
                            format!("{path}.nodes.{}.inputs.{name}", node.id),
                            "Required input has no binding",
                        ));
                    }
                }
            }
            if matches!(node.kind, NodeKind::Parallel) && outgoing[node.id.as_str()].len() < 2 {
                return Err(invalid(path, "Parallel requires at least two successors"));
            }
            if matches!(node.kind, NodeKind::Join) && incoming[node.id.as_str()].len() < 2 {
                return Err(invalid(path, "Join requires at least two predecessors"));
            }
        }
        for (name, binding) in &self.outputs {
            if !identifier(name)
                || !nodes
                    .get(binding.node.as_str())
                    .is_some_and(|node| node.outputs.contains_key(&binding.port))
            {
                return Err(invalid(
                    format!("{path}.outputs"),
                    "Workflow output refers to an undeclared node or port",
                ));
            }
        }
        Ok(())
    }
}

fn reachable<'a>(start: &'a str, edges: &BTreeMap<&'a str, Vec<&'a str>>) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![start];
    while let Some(node) = queue.pop() {
        if seen.insert(node)
            && let Some(children) = edges.get(node)
        {
            queue.extend(children.iter().copied());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            name: id.into(),
            kind,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
        }
    }
    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            outcome: EdgeOutcome::Always,
        }
    }
    fn graph() -> WorkflowDefinition {
        WorkflowDefinition {
            schema_version: 1,
            name: "fixture".into(),
            limits: WorkflowLimits::default(),
            nodes: vec![
                node(
                    "start",
                    NodeKind::Trigger {
                        source: TriggerSource::Manual,
                    },
                ),
                node(
                    "agent",
                    NodeKind::Agent {
                        agent_version_id: "agent-version-1".into(),
                    },
                ),
                node("end", NodeKind::Complete),
            ],
            edges: vec![edge("start", "agent"), edge("agent", "end")],
            bindings: vec![],
            outputs: BTreeMap::new(),
        }
    }

    #[test]
    fn valid_graph_and_serialization_round_trip() {
        let graph = graph();
        assert_eq!(graph.validate(), Ok(()));
        let encoded = serde_json::to_string(&graph).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkflowDefinition>(&encoded).unwrap(),
            graph
        );
    }

    #[test]
    fn duplicate_unknown_and_disconnected_nodes_are_rejected() {
        let mut g = graph();
        g.nodes.push(g.nodes[1].clone());
        assert!(g.validate().is_err());
        let mut g = graph();
        g.edges[0].to = "unknown".into();
        assert!(g.validate().is_err());
        let mut g = graph();
        g.nodes.push(node(
            "orphan",
            NodeKind::Workspace {
                binding_id: "b".into(),
            },
        ));
        assert!(g.validate().is_err());
    }

    #[test]
    fn arbitrary_cycles_are_rejected() {
        let mut g = graph();
        g.nodes.push(node(
            "second",
            NodeKind::Agent {
                agent_version_id: "v2".into(),
            },
        ));
        g.edges.extend([
            edge("agent", "second"),
            edge("second", "agent"),
            edge("second", "end"),
        ]);
        assert!(g.validate().unwrap_err().message.contains("cycle"));
    }

    #[test]
    fn input_binding_types_and_ownership_are_checked() {
        let mut g = graph();
        g.nodes[0].outputs.insert(
            "brief".into(),
            Port {
                kind: PortType::Text,
                required: true,
            },
        );
        g.nodes[1].inputs.insert(
            "brief".into(),
            Port {
                kind: PortType::Text,
                required: true,
            },
        );
        assert!(g.validate().is_err());
        g.bindings.push(PortBinding {
            from_node: "start".into(),
            from_port: "brief".into(),
            to_node: "agent".into(),
            to_port: "brief".into(),
        });
        assert!(g.validate().is_ok());
        g.nodes[1].inputs.get_mut("brief").unwrap().kind = PortType::Boolean;
        assert!(g.validate().is_err());
        g.nodes[1].inputs.get_mut("brief").unwrap().kind = PortType::Text;
        g.bindings.push(g.bindings[0].clone());
        assert!(g.validate().is_err());
    }

    #[test]
    fn a_later_data_producer_cannot_feed_an_earlier_step() {
        let mut g = graph();
        g.nodes[2].outputs.insert(
            "value".into(),
            Port {
                kind: PortType::Text,
                required: true,
            },
        );
        g.nodes[1].inputs.insert(
            "value".into(),
            Port {
                kind: PortType::Text,
                required: true,
            },
        );
        g.bindings.push(PortBinding {
            from_node: "end".into(),
            from_port: "value".into(),
            to_node: "agent".into(),
            to_port: "value".into(),
        });
        assert!(g.validate().is_err());
    }

    #[test]
    fn unknown_versions_and_unbounded_execution_are_rejected() {
        let mut g = graph();
        g.schema_version = 99;
        assert!(g.validate().is_err());
        let mut g = graph();
        g.limits.max_duration_seconds = 0;
        assert!(g.validate().is_err());
        let mut g = graph();
        g.limits.max_concurrent_steps = 255;
        assert!(g.validate().is_err());
    }

    #[test]
    fn repeat_requires_a_boolean_result_and_an_iteration_limit() {
        let mut body = graph();
        body.nodes[1].outputs.insert(
            "accepted".into(),
            Port {
                kind: PortType::Boolean,
                required: true,
            },
        );
        body.outputs.insert(
            "accepted".into(),
            OutputBinding {
                node: "agent".into(),
                port: "accepted".into(),
            },
        );
        let mut g = graph();
        g.nodes[1].kind = NodeKind::Repeat {
            body: Box::new(body),
            until_output: "accepted".into(),
            max_iterations: 3,
        };
        assert!(g.validate().is_ok());
        if let NodeKind::Repeat { max_iterations, .. } = &mut g.nodes[1].kind {
            *max_iterations = 0;
        }
        assert!(g.validate().is_err());
        if let NodeKind::Repeat {
            max_iterations,
            until_output,
            ..
        } = &mut g.nodes[1].kind
        {
            *max_iterations = 3;
            *until_output = "missing".into();
        }
        assert!(g.validate().is_err());
    }

    #[test]
    fn review_cannot_have_zero_required_approvals() {
        let mut g = graph();
        g.nodes[1].kind = NodeKind::HumanReview {
            minimum_approvals: 0,
            separation_of_duties: true,
        };
        assert!(g.validate().is_err());
    }

    #[test]
    fn numeric_widening_is_allowed_but_not_boolean_coercion() {
        assert!(PortType::Number.accepts(PortType::Integer));
        assert!(!PortType::Integer.accepts(PortType::Number));
        assert!(!PortType::Boolean.accepts(PortType::Text));
    }
}
