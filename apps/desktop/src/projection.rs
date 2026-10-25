//! Project a live transcript into the graph and trace surfaces.
//!
//! The transcript is the source of truth. Graph nodes describe the work the
//! agent is doing (plan steps or recent tools); trace spans describe when it
//! happened. Both projections are pure functions so the views stay cheap and
//! deterministic, and persisted transcripts produce the same shapes after a
//! restart.

use dunlin_agent::PlanStatus;
use dunlin_domain::{
    AgentKind, AgentNode, AgentStatus, Project, SessionStatus, SessionSummary, SpanKind,
    SpanStatus, ToolKind, ToolStatus, TraceSpan,
};

use crate::harness::ChatItem;

const CANVAS_WIDTH: f32 = 1150.0;
const ROOT_WIDTH: f32 = 270.0;
const CHILD_WIDTH: f32 = 240.0;
const CHILD_GAP: f32 = 24.0;
const CHILD_Y: f32 = 224.0;
const NODE_HEIGHT: f32 = 128.0;
const MAX_CHILDREN: usize = 7;

/// Build the orchestrator tree for the current transcript.
pub(crate) fn nodes(
    transcript: &[ChatItem],
    project: &Project,
    session: &SessionSummary,
) -> Vec<AgentNode> {
    let tools: Vec<ToolKind> = transcript
        .iter()
        .filter_map(|item| match item {
            ChatItem::Tool { view, .. } => Some(view.kind),
            _ => None,
        })
        .fold(Vec::new(), |mut kinds, kind| {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
            kinds
        });

    let task = if session.title == "New session" {
        format!(
            "Waiting for the first prompt in {}.",
            if project.branch.is_some() {
                "this workspace"
            } else {
                project.name.as_str()
            }
        )
    } else if session.summary.is_empty() {
        session.title.clone()
    } else {
        session.summary.clone()
    };

    let mut nodes = vec![AgentNode {
        id: "root".into(),
        parent: None,
        title: "Orchestrator".into(),
        task,
        model: session.model.clone(),
        harness: session.harness.clone(),
        status: agent_status(session.status),
        kind: AgentKind::Orchestrator,
        tools,
        cost_usd: session.cost_usd,
        duration_secs: session.duration_secs,
        x: (CANVAS_WIDTH - ROOT_WIDTH) / 2.0,
        y: 16.0,
        width: ROOT_WIDTH,
        height: NODE_HEIGHT,
    }];

    let plan = transcript.iter().rev().find_map(|item| match item {
        ChatItem::Plan { items, .. } => Some(items),
        _ => None,
    });

    let children: Vec<AgentNode> = match plan {
        Some(items) if !items.is_empty() => items
            .iter()
            .take(MAX_CHILDREN)
            .enumerate()
            .map(|(index, item)| AgentNode {
                id: format!("plan-{index}"),
                parent: Some("root".into()),
                title: truncate(&item.content, 34),
                task: item.content.clone(),
                model: session.model.clone(),
                harness: session.harness.clone(),
                status: plan_agent_status(item.status),
                kind: AgentKind::Subagent,
                tools: vec![ToolKind::Plan],
                cost_usd: 0.0,
                duration_secs: 0,
                x: 0.0,
                y: CHILD_Y,
                width: CHILD_WIDTH,
                height: NODE_HEIGHT,
            })
            .collect(),
        _ => {
            let mut tools: Vec<&ChatItem> = transcript
                .iter()
                .filter(|item| matches!(item, ChatItem::Tool { .. }))
                .collect();
            let start = tools.len().saturating_sub(MAX_CHILDREN);
            tools.drain(..start);
            tools
                .into_iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let ChatItem::Tool {
                        view, duration_ms, ..
                    } = item
                    else {
                        return None;
                    };
                    Some(AgentNode {
                        id: format!("tool-{index}"),
                        parent: Some("root".into()),
                        title: truncate(&view.title, 34),
                        task: if view.detail.is_empty() {
                            "Tool call".into()
                        } else {
                            truncate(&view.detail, 120)
                        },
                        model: session.model.clone(),
                        harness: session.harness.clone(),
                        status: tool_agent_status(view.status),
                        kind: AgentKind::Tool,
                        tools: vec![view.kind],
                        cost_usd: 0.0,
                        duration_secs: duration_ms / 1000,
                        x: 0.0,
                        y: CHILD_Y,
                        width: CHILD_WIDTH,
                        height: NODE_HEIGHT,
                    })
                })
                .collect()
        }
    };

    layout_children(&mut nodes, children);
    nodes
}

fn layout_children(nodes: &mut Vec<AgentNode>, mut children: Vec<AgentNode>) {
    if children.is_empty() {
        return;
    }
    let count = children.len() as f32;
    let total = count * CHILD_WIDTH + (count - 1.0).max(0.0) * CHILD_GAP;
    let start = ((CANVAS_WIDTH - total) / 2.0).max(20.0);
    for (index, child) in children.iter_mut().enumerate() {
        child.x = start + index as f32 * (CHILD_WIDTH + CHILD_GAP);
    }
    nodes.extend(children);
}

/// Build a waterfall from the timestamps and tool durations in the transcript.
pub(crate) fn spans(transcript: &[ChatItem], session: &SessionSummary) -> Vec<TraceSpan> {
    let starts: Vec<u64> = transcript
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let at = item_at(item);
            if at > 0 { at } else { index as u64 * 1_000 }
        })
        .collect();
    let base = starts
        .iter()
        .copied()
        .filter(|at| *at > 0)
        .min()
        .unwrap_or(0);

    let relative: Vec<u64> = starts.iter().map(|at| at.saturating_sub(base)).collect();

    let mut total = relative.iter().copied().max().unwrap_or(0) + 1;
    for (index, item) in transcript.iter().enumerate() {
        let duration = item_duration(item).max(1);
        total = total.max(relative[index] + duration);
    }
    total = total.max(1);

    let turns: Vec<(usize, u64, u64)> = transcript
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, ChatItem::User { .. }))
        .map(|(index, _)| {
            let start = relative[index];
            let end = transcript
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, item)| matches!(item, ChatItem::User { .. }))
                .map(|(next, _)| relative[next])
                .unwrap_or(total);
            (index, start, end.max(start + 1))
        })
        .collect();

    let mut spans = Vec::new();
    spans.push(TraceSpan {
        id: "root".into(),
        parent: None,
        name: format!("Session · {}", session.title),
        agent: "Orchestrator".into(),
        kind: SpanKind::Agent,
        status: span_status(session.status),
        start_ms: 0,
        duration_ms: total,
        depth: 0,
        cost_usd: session.cost_usd,
        tokens: session.tokens,
        detail: session.summary.clone(),
    });

    for (turn_index, (item_index, start, end)) in turns.iter().enumerate() {
        let last_turn = turn_index + 1 == turns.len();
        spans.push(TraceSpan {
            id: format!("turn-{turn_index}"),
            parent: Some("root".into()),
            name: format!("Turn {}", turn_index + 1),
            agent: "User".into(),
            kind: SpanKind::Turn,
            status: if last_turn {
                span_status(session.status)
            } else {
                SpanStatus::Done
            },
            start_ms: *start,
            duration_ms: end.saturating_sub(*start).max(1),
            depth: 1,
            cost_usd: 0.0,
            tokens: 0,
            detail: transcript
                .get(*item_index)
                .and_then(|item| match item {
                    ChatItem::User { text, .. } => Some(truncate(text, 160)),
                    _ => None,
                })
                .unwrap_or_default(),
        });
    }

    let mut current_turn: Option<usize> = None;
    for (index, item) in transcript.iter().enumerate() {
        if matches!(item, ChatItem::User { .. }) {
            current_turn = turns
                .iter()
                .position(|(item_index, _, _)| *item_index == index);
            continue;
        }
        let start = relative[index];
        let next = relative.get(index + 1).copied().unwrap_or(total);
        let parent = current_turn
            .map(|turn| format!("turn-{turn}"))
            .unwrap_or_else(|| "root".into());
        let depth = if current_turn.is_some() { 2 } else { 1 };
        let (name, agent, kind, status, detail, duration) = match item {
            ChatItem::AgentText { text, .. } => (
                "Response".to_string(),
                session.harness.clone(),
                SpanKind::Llm,
                SpanStatus::Done,
                truncate(text, 200),
                next.saturating_sub(start).max(1),
            ),
            ChatItem::Thought { text, .. } => (
                "Reasoning".to_string(),
                session.harness.clone(),
                SpanKind::Llm,
                SpanStatus::Done,
                truncate(text, 200),
                next.saturating_sub(start).max(1),
            ),
            ChatItem::Tool {
                view, duration_ms, ..
            } => (
                view.title.clone(),
                format!("Tool · {}", view.kind.label()),
                SpanKind::Tool,
                tool_span_status(view.status),
                truncate(&view.detail, 200),
                (*duration_ms).max(1),
            ),
            ChatItem::Plan { items, .. } => (
                "Plan".to_string(),
                session.harness.clone(),
                SpanKind::Agent,
                plan_span_status(
                    items
                        .iter()
                        .find(|item| item.status == PlanStatus::InProgress)
                        .map(|item| item.status)
                        .unwrap_or_else(|| {
                            items
                                .first()
                                .map(|item| item.status)
                                .unwrap_or(PlanStatus::Pending)
                        }),
                ),
                format!("{} steps", items.len()),
                next.saturating_sub(start).max(1),
            ),
            ChatItem::Permission {
                title, resolved, ..
            } => (
                title.clone(),
                "User".into(),
                SpanKind::Permission,
                if resolved.is_some() {
                    SpanStatus::Done
                } else {
                    SpanStatus::Waiting
                },
                resolved
                    .clone()
                    .unwrap_or_else(|| "Awaiting a decision".into()),
                next.saturating_sub(start).max(1),
            ),
            ChatItem::Error { message, .. } => (
                "Error".to_string(),
                session.harness.clone(),
                SpanKind::Waiting,
                SpanStatus::Failed,
                truncate(message, 200),
                next.saturating_sub(start).max(1),
            ),
            ChatItem::User { .. } => continue,
        };

        spans.push(TraceSpan {
            id: format!("item-{index}"),
            parent: Some(parent),
            name,
            agent,
            kind,
            status,
            start_ms: start,
            duration_ms: duration,
            depth,
            cost_usd: 0.0,
            tokens: 0,
            detail,
        });
    }

    spans
}

fn item_at(item: &ChatItem) -> u64 {
    match item {
        ChatItem::User { at_ms, .. }
        | ChatItem::AgentText { at_ms, .. }
        | ChatItem::Thought { at_ms, .. }
        | ChatItem::Tool { at_ms, .. }
        | ChatItem::Plan { at_ms, .. }
        | ChatItem::Permission { at_ms, .. }
        | ChatItem::Error { at_ms, .. } => *at_ms,
    }
}

fn item_duration(item: &ChatItem) -> u64 {
    match item {
        ChatItem::Tool { duration_ms, .. } => *duration_ms,
        _ => 100,
    }
}

fn agent_status(status: SessionStatus) -> AgentStatus {
    match status {
        SessionStatus::Running => AgentStatus::Running,
        SessionStatus::Waiting => AgentStatus::Waiting,
        SessionStatus::Done => AgentStatus::Done,
        SessionStatus::Failed => AgentStatus::Failed,
        SessionStatus::Interrupted => AgentStatus::Stopped,
    }
}

fn span_status(status: SessionStatus) -> SpanStatus {
    match status {
        SessionStatus::Running => SpanStatus::Running,
        SessionStatus::Waiting => SpanStatus::Waiting,
        SessionStatus::Done => SpanStatus::Done,
        SessionStatus::Failed => SpanStatus::Failed,
        SessionStatus::Interrupted => SpanStatus::Failed,
    }
}

fn tool_agent_status(status: ToolStatus) -> AgentStatus {
    match status {
        ToolStatus::Running => AgentStatus::Running,
        ToolStatus::Waiting => AgentStatus::Waiting,
        ToolStatus::Done => AgentStatus::Done,
        ToolStatus::Failed => AgentStatus::Failed,
    }
}

fn tool_span_status(status: ToolStatus) -> SpanStatus {
    match status {
        ToolStatus::Running => SpanStatus::Running,
        ToolStatus::Waiting => SpanStatus::Waiting,
        ToolStatus::Done => SpanStatus::Done,
        ToolStatus::Failed => SpanStatus::Failed,
    }
}

fn plan_agent_status(status: PlanStatus) -> AgentStatus {
    match status {
        PlanStatus::Pending => AgentStatus::Waiting,
        PlanStatus::InProgress => AgentStatus::Running,
        PlanStatus::Completed => AgentStatus::Done,
    }
}

fn plan_span_status(status: PlanStatus) -> SpanStatus {
    match status {
        PlanStatus::Pending => SpanStatus::Waiting,
        PlanStatus::InProgress => SpanStatus::Running,
        PlanStatus::Completed => SpanStatus::Done,
    }
}

fn truncate(text: &str, max: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        format!(
            "{}…",
            cleaned
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{nodes, spans, truncate};
    use crate::harness::ChatItem;
    use dunlin_agent::{PlanItem, PlanStatus, ToolCallView};
    use dunlin_domain::{
        Project, ProjectTone, SessionOrigin, SessionStatus, SessionSummary, SpanKind, ToolKind,
        ToolStatus,
    };

    fn project() -> Project {
        Project {
            id: "dunlin".into(),
            name: "Dunlin".into(),
            path: "/tmp/dunlin".into(),
            branch: Some("main".into()),
            tone: ProjectTone::Teal,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    fn session() -> SessionSummary {
        SessionSummary {
            id: "ses-1".into(),
            project_id: "dunlin".into(),
            title: "Fix the build".into(),
            status: SessionStatus::Running,
            harness: "OpenCode".into(),
            model: "Auto".into(),
            branch: Some("main".into()),
            updated_at: "now".into(),
            cost_usd: 0.2,
            duration_secs: 12,
            tokens: 2_000,
            summary: "Repair the failing build.".into(),
            origin: SessionOrigin::Local,
            acp_session_id: None,
        }
    }

    fn tool() -> ChatItem {
        ChatItem::Tool {
            view: ToolCallView {
                id: "tool-1".into(),
                title: "Run tests".into(),
                kind: ToolKind::Shell,
                status: ToolStatus::Done,
                detail: "cargo test".into(),
                locations: Vec::new(),
                additions: 0,
                deletions: 0,
                diff: None,
            },
            duration_ms: 1_500,
            at_ms: 1_000,
        }
    }

    #[test]
    fn empty_transcript_still_has_an_orchestrator() {
        let nodes = nodes(&[], &project(), &session());
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, "root");
        assert_eq!(nodes[0].status, dunlin_domain::AgentStatus::Running);
    }

    #[test]
    fn plan_steps_become_subagent_nodes() {
        let transcript = vec![ChatItem::Plan {
            items: vec![
                PlanItem {
                    content: "Read the failing test".into(),
                    status: PlanStatus::Completed,
                },
                PlanItem {
                    content: "Patch the parser".into(),
                    status: PlanStatus::InProgress,
                },
            ],
            at_ms: 1_000,
        }];
        let nodes = nodes(&transcript, &project(), &session());
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[1].parent.as_deref(), Some("root"));
        assert_eq!(nodes[2].status, dunlin_domain::AgentStatus::Running);
    }

    #[test]
    fn tools_become_nodes_when_there_is_no_plan() {
        let nodes = nodes(&[tool()], &project(), &session());
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[1].kind, dunlin_domain::AgentKind::Tool);
    }

    #[test]
    fn spans_use_real_timestamps_and_tool_durations() {
        let transcript = vec![
            ChatItem::User {
                text: "run the tests".into(),
                at_ms: 1_000,
            },
            tool(),
            ChatItem::AgentText {
                text: "All tests passed.".into(),
                streaming: false,
                at_ms: 3_000,
            },
        ];
        let spans = spans(&transcript, &session());
        let tool_span = spans
            .iter()
            .find(|span| span.kind == SpanKind::Tool)
            .unwrap();
        assert_eq!(tool_span.start_ms, 0);
        assert_eq!(tool_span.duration_ms, 1_500);
        let root = &spans[0];
        assert_eq!(root.start_ms, 0);
        assert!(root.duration_ms >= 2_000);
    }

    #[test]
    fn truncation_collapses_whitespace() {
        assert_eq!(truncate("  hello   world  ", 20), "hello world");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }
}
