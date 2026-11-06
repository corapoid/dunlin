//! Project a live transcript into the graph and trace surfaces.
//!
//! The transcript is the source of truth. Graph nodes describe the work the
//! agent is doing (plan steps or recent tools); trace spans describe when it
//! happened. Both projections are pure functions so the views stay cheap and
//! deterministic, and persisted transcripts produce the same shapes after a
//! restart.

use std::collections::HashMap;

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
const VERTICAL_GAP: f32 = 42.0;
const NODE_HEIGHT: f32 = 128.0;
const NODE_Y: f32 = 16.0;

/// Build the orchestrator tree for the current transcript.
pub(crate) fn nodes(
    transcript: &[ChatItem],
    project: &Project,
    session: &SessionSummary,
) -> Vec<AgentNode> {
    if transcript.is_empty() {
        return Vec::new();
    }
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

    let children = child_nodes(transcript, session);
    nodes.extend(children);
    nodes
}

/// Lay out a visible agent forest with a deterministic tidy-tree pass.
///
/// The old algorithm rescanned the whole slice for every parent and stopped at
/// eight levels. This version indexes parents once, breaks stray cycles, and
/// handles arbitrary depth without recursion, so streaming very deep subagent
/// chains or expanding thousands of tools stays linear. Widths are packed by
/// subtree in input order; a parent wider than its children stays centred over
/// them instead of forcing a wider lane.
pub(crate) fn layout_children(nodes: &mut Vec<AgentNode>, mut children: Vec<AgentNode>) {
    if children.is_empty() {
        nodes[0].x = (CANVAS_WIDTH - nodes[0].width) / 2.;
        nodes[0].y = NODE_Y;
        return;
    }

    let index: HashMap<_, _> = children
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect();

    // Never let a projection bug make the layout loop: a missing parent, a
    // self-reference or a detached cycle becomes another root of the forest.
    for node in &mut children {
        let valid = node.parent.as_deref().is_some_and(|parent| {
            parent == "root" || (parent != node.id && index.contains_key(parent))
        });
        if !valid {
            node.parent = Some("root".into());
        }
    }
    break_cycles(&mut children, &index);
    for node in &mut children {
        if node.parent.is_none() {
            node.parent = Some("root".into());
        }
    }

    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); children.len()];
    for (child, node) in children.iter().enumerate() {
        if let Some(parent) = node.parent.as_deref().and_then(|parent| index.get(parent)) {
            // Parent normalization above already rejected self-references.
            kids[*parent].push(child);
        }
    }
    let mut roots: Vec<usize> = (0..children.len())
        .filter(|index| children[*index].parent.as_deref() == Some("root"))
        .collect();
    if roots.is_empty() {
        // A defensive fallback for malformed data: every node becomes a root
        // rather than disappearing from the graph.
        for (index, node) in children.iter_mut().enumerate() {
            node.parent = Some("root".into());
            roots.push(index);
        }
    }

    let mut order = Vec::with_capacity(children.len());
    let mut seen = vec![false; children.len()];
    let mut stack: Vec<(usize, bool)> = roots.iter().rev().map(|index| (*index, false)).collect();
    while let Some((index, expanded)) = stack.pop() {
        if expanded {
            order.push(index);
            continue;
        }
        if seen[index] {
            continue;
        }
        seen[index] = true;
        stack.push((index, true));
        for child in kids[index].iter().rev() {
            if !seen[*child] {
                stack.push((*child, false));
            }
        }
    }
    if order.len() != children.len() {
        // Only reachable if the input was mutated between indexing passes.
        // Keep the remaining nodes visible as roots.
        for index in 0..children.len() {
            if !seen[index] {
                children[index].parent = Some("root".into());
                roots.push(index);
                order.push(index);
            }
        }
    }

    let row_width = |indices: &[usize], widths: &[f32]| {
        indices.iter().map(|index| widths[*index]).sum::<f32>()
            + indices.len().saturating_sub(1) as f32 * CHILD_GAP
    };

    let mut subtree = vec![0.0; children.len()];
    for &index in &order {
        subtree[index] = row_width(&kids[index], &subtree).max(children[index].width);
    }

    let mut depths = vec![0usize; children.len()];
    let mut stack: Vec<(usize, usize)> = roots.iter().map(|index| (*index, 0)).collect();
    let mut max_depth = 0;
    while let Some((index, depth)) = stack.pop() {
        depths[index] = depth;
        max_depth = max_depth.max(depth);
        for child in &kids[index] {
            stack.push((*child, depth + 1));
        }
    }
    let mut level_height = vec![0.0f32; max_depth + 1];
    for (index, node) in children.iter().enumerate() {
        level_height[depths[index]] = level_height[depths[index]].max(node.height);
    }
    let mut level_y = vec![CHILD_Y; max_depth + 1];
    for depth in 1..=max_depth {
        level_y[depth] = level_y[depth - 1] + level_height[depth - 1] + VERTICAL_GAP;
    }

    let total = row_width(&roots, &subtree);
    let left = ((CANVAS_WIDTH - total) / 2.0).max(20.0);
    nodes[0].x = left + (total - nodes[0].width) / 2.0;
    nodes[0].y = NODE_Y;

    let mut stack: Vec<(usize, f32, usize)> = Vec::new();
    let mut cursor = left;
    for root in &roots {
        stack.push((*root, cursor, 0));
        cursor += subtree[*root] + CHILD_GAP;
    }
    while let Some((index, box_left, depth)) = stack.pop() {
        let node = &mut children[index];
        node.x = box_left + (subtree[index] - node.width) / 2.0;
        node.y = level_y[depth];

        let children_width = row_width(&kids[index], &subtree);
        let mut child_left = box_left + (subtree[index] - children_width) / 2.0;
        let mut boxes = Vec::with_capacity(kids[index].len());
        for child in &kids[index] {
            boxes.push((*child, child_left, depth + 1));
            child_left += subtree[*child] + CHILD_GAP;
        }
        for entry in boxes.into_iter().rev() {
            stack.push(entry);
        }
    }

    nodes.extend(children);
}

/// Break parent cycles in linear time. Exactly one incoming edge per cycle is
/// detached; the caller later promotes the detached node to a forest root.
fn break_cycles(children: &mut [AgentNode], index: &HashMap<String, usize>) {
    let mut state = vec![0u8; children.len()]; // 0 = new, 1 = on path, 2 = done
    for start in 0..children.len() {
        if state[start] != 0 {
            continue;
        }
        let mut path: Vec<usize> = Vec::new();
        let mut current = Some(start);
        while let Some(node) = current {
            if state[node] == 2 {
                break;
            }
            if state[node] == 1 {
                if let Some(last) = path.last() {
                    children[*last].parent = None;
                }
                break;
            }
            state[node] = 1;
            path.push(node);
            current = children[node]
                .parent
                .as_deref()
                .and_then(|parent| index.get(parent).copied())
                .filter(|parent| *parent != node);
        }
        for node in path {
            state[node] = 2;
        }
    }
}

/// The wrapper identifies actual child sessions; a websearch/tool is not an agent.
pub(crate) fn child_id(detail: &str) -> Option<&str> {
    let tag = detail
        .trim_start()
        .strip_prefix("<subagent ")?
        .split_once('>')?
        .0;
    let id = tag.split_once("sessionID=\"")?.1.split_once('"')?.0;
    (!id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
    .then_some(id)
}

pub(crate) fn tool_summary(detail: &str) -> &str {
    if child_id(detail).is_some() {
        detail
            .trim_start()
            .split_once('>')
            .map(|(_, body)| body.trim().trim_end_matches("</subagent>").trim())
            .unwrap_or(detail)
    } else {
        detail
    }
}

fn child_nodes(transcript: &[ChatItem], session: &SessionSummary) -> Vec<AgentNode> {
    fn collect(
        items: &[ChatItem],
        session: &SessionSummary,
        parent: &str,
        nodes: &mut Vec<AgentNode>,
    ) {
        for item in items {
            match item {
                ChatItem::Child {
                    id,
                    parent_id,
                    title,
                    items,
                    ..
                } => {
                    let parent = if Some(parent_id.as_str()) == session.acp_session_id.as_deref() {
                        "root"
                    } else {
                        parent_id
                    };
                    if !nodes.iter().any(|n| n.id == *id) {
                        nodes.push(AgentNode {
                            id: id.clone(),
                            parent: Some(parent.into()),
                            title: title.clone().unwrap_or_else(|| "Subagent".into()),
                            task: "Delegated session".into(),
                            model: session.model.clone(),
                            harness: session.harness.clone(),
                            status: agent_status(session.status),
                            kind: AgentKind::Subagent,
                            tools: Vec::new(),
                            cost_usd: 0.,
                            duration_secs: 0,
                            x: 0.,
                            y: CHILD_Y,
                            width: CHILD_WIDTH,
                            height: NODE_HEIGHT,
                        });
                    }
                    if let Some(node) = nodes.iter_mut().find(|n| n.id == *id)
                        && let Some(title) = title
                    {
                        node.title = title.clone();
                    }
                    collect(items, session, id, nodes);
                }
                ChatItem::Tool {
                    view, duration_ms, ..
                } => {
                    if let Some(id) = child_id(&view.detail) {
                        if !nodes.iter().any(|n| n.id == id) {
                            nodes.push(AgentNode {
                                id: id.into(),
                                parent: Some(parent.into()),
                                title: if view.title == "subagent" {
                                    "Subagent".into()
                                } else {
                                    view.title.clone()
                                },
                                task: truncate(tool_summary(&view.detail), 120),
                                model: session.model.clone(),
                                harness: session.harness.clone(),
                                status: tool_agent_status(view.status),
                                kind: AgentKind::Subagent,
                                tools: vec![view.kind],
                                cost_usd: 0.,
                                duration_secs: duration_ms / 1000,
                                x: 0.,
                                y: CHILD_Y,
                                width: CHILD_WIDTH,
                                height: NODE_HEIGHT,
                            });
                        } else if let Some(node) = nodes.iter_mut().find(|n| n.id == id) {
                            node.status = tool_agent_status(view.status);
                            node.duration_secs = duration_ms / 1000;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn tools(
        items: &[ChatItem],
        session: &SessionSummary,
        parent: &str,
        nodes: &mut Vec<AgentNode>,
    ) {
        for item in items {
            match item {
                ChatItem::Child { id, items, .. } => tools(items, session, id, nodes),
                ChatItem::Tool {
                    view, duration_ms, ..
                } if child_id(&view.detail).is_none() => {
                    let parent = view
                        .id
                        .split_once(':')
                        .map(|(id, _)| id)
                        .filter(|id| {
                            nodes
                                .iter()
                                .any(|n| n.id == *id && n.kind == AgentKind::Subagent)
                        })
                        .unwrap_or(parent);
                    nodes.push(AgentNode {
                        id: format!("tool:{}", view.id),
                        parent: Some(parent.into()),
                        title: truncate(&view.title, 34),
                        task: truncate(tool_summary(&view.detail), 120),
                        model: session.model.clone(),
                        harness: session.harness.clone(),
                        status: tool_agent_status(view.status),
                        kind: AgentKind::Tool,
                        tools: vec![view.kind],
                        cost_usd: 0.,
                        duration_secs: duration_ms / 1000,
                        x: 0.,
                        y: CHILD_Y,
                        width: CHILD_WIDTH,
                        height: NODE_HEIGHT,
                    });
                }
                _ => {}
            }
        }
    }
    let mut result = Vec::new();
    collect(transcript, session, "root", &mut result);
    tools(transcript, session, "root", &mut result);
    let ids: std::collections::HashSet<_> = result.iter().map(|n| n.id.clone()).collect();
    for node in &mut result {
        if node
            .parent
            .as_ref()
            .is_none_or(|p| p != "root" && !ids.contains(p))
        {
            node.parent = Some("root".into());
        }
    }
    result
}

/// Build a waterfall from the timestamps and tool durations in the transcript.
pub(crate) fn spans(transcript: &[ChatItem], session: &SessionSummary) -> Vec<TraceSpan> {
    if transcript.is_empty() {
        return Vec::new();
    }
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
        if let ChatItem::User {
            timing: Some(timing),
            ..
        } = item
        {
            total = total.max(
                timing
                    .finished_at_ms
                    .unwrap_or(timing.started_at_ms)
                    .saturating_sub(base),
            );
        }
    }
    total = total.max(1);

    let turns: Vec<(usize, u64, u64)> = transcript
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, ChatItem::User { .. }))
        .map(|(index, item)| {
            let start = relative[index];
            if let ChatItem::User {
                timing: Some(timing),
                ..
            } = item
                && let Some(elapsed) = timing.elapsed_ms
            {
                let start = timing.started_at_ms.saturating_sub(base);
                return (index, start, start.saturating_add(elapsed));
            }
            let next_user = transcript
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, item)| matches!(item, ChatItem::User { .. }))
                .map(|(next, _)| next)
                .unwrap_or(transcript.len());
            // Without a recorded completion, only include observed activity.
            // Never stretch a final response to the user's next request.
            let end = (index..next_user)
                .map(|i| {
                    relative[i]
                        + match &transcript[i] {
                            ChatItem::Tool { duration_ms, .. } => *duration_ms,
                            _ => 0,
                        }
                })
                .max()
                .unwrap_or(start);
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
        duration_known: true,
        depth: 0,
        cost_usd: session.cost_usd,
        tokens: transcript
            .iter()
            .rev()
            .find(|item| matches!(item, ChatItem::User { .. }))
            .and_then(|item| match item {
                ChatItem::User {
                    timing: Some(timing),
                    ..
                } => timing.usage.map(|usage| usage.total),
                _ => None,
            }),
        detail: session.summary.clone(),
    });

    for (turn_index, (item_index, start, end)) in turns.iter().enumerate() {
        let last_turn = turn_index + 1 == turns.len();
        spans.push(TraceSpan {
            id: format!("turn-{turn_index}"),
            parent: Some("root".into()),
            name: format!(
                "Request {} · {}",
                turn_index + 1,
                match &transcript[*item_index] {
                    ChatItem::User { text, .. } => truncate(text, 65),
                    _ => String::new(),
                }
            ),
            agent: "User".into(),
            kind: SpanKind::Turn,
            status: if last_turn {
                span_status(session.status)
            } else {
                SpanStatus::Done
            },
            start_ms: *start,
            duration_ms: end.saturating_sub(*start).max(1),
            duration_known: matches!(&transcript[*item_index], ChatItem::User { timing: Some(t), .. } if t.elapsed_ms.is_some()),
            depth: 1,
            cost_usd: 0.0,
            tokens: turn_tokens(transcript, *item_index),
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
        let turn_end = current_turn.map(|t| turns[t].2).unwrap_or(total);
        let next = relative
            .get(index + 1)
            .copied()
            .unwrap_or(turn_end)
            .min(turn_end)
            .max(start);
        let known_end = current_turn.is_some_and(|t| matches!(&transcript[turns[t].0], ChatItem::User { timing: Some(t), .. } if t.elapsed_ms.is_some()))
            || transcript.get(index + 1).is_some_and(|item| !matches!(item, ChatItem::User { .. }));
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
            ChatItem::Child {
                id, title, items, ..
            } => (
                title.clone().unwrap_or_else(|| "Subagent".into()),
                id.clone(),
                SpanKind::Agent,
                span_status(session.status),
                format!("{} child-session events", items.len()),
                next.saturating_sub(start).max(1),
            ),
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
            duration_known: matches!(item, ChatItem::Tool { .. }) || known_end,
            depth,
            cost_usd: 0.0,
            tokens: None,
            detail,
        });
        if let ChatItem::Child { items, .. } = item {
            let child_base = items
                .iter()
                .map(item_at)
                .filter(|at| *at > 0)
                .min()
                .unwrap_or(item_at(item));
            let shift = child_base.saturating_sub(base);
            for mut child in self::spans(items, session).into_iter().skip(1) {
                child.id = format!("child-{index}:{}", child.id);
                child.parent = Some(match child.parent.as_deref() {
                    Some("root") | None => format!("item-{index}"),
                    Some(parent) => format!("child-{index}:{parent}"),
                });
                child.start_ms = child.start_ms.saturating_add(shift);
                child.depth += depth;
                spans.push(child);
            }
        }
    }

    for (index, pair) in turns.windows(2).enumerate() {
        let end = pair[0].2;
        let next = pair[1].1;
        if next > end.saturating_add(1000) {
            let known = matches!(&transcript[pair[0].0], ChatItem::User { timing: Some(t), .. } if t.elapsed_ms.is_some());
            spans.push(TraceSpan { id: format!("gap-{index}"), parent: Some("root".into()), name: if known { "Idle between requests" } else { "Unrecorded gap between requests" }.into(), agent: "User".into(), kind: SpanKind::Waiting, status: SpanStatus::Done, start_ms: end, duration_ms: next - end, duration_known: true, depth: 1, cost_usd: 0., tokens: None,
                detail: if known { "The previous request had completed. This interval is idle time, not model work." } else { "The previous request has no recorded completion. The gap after its final recorded event is not attributed to model work." }.into() });
        }
    }
    spans
}

// The ACP extension reports cumulative counters. Attribute only a difference
// between two recorded snapshots, never context occupancy or guessed text length.
pub(crate) fn turn_tokens(items: &[ChatItem], index: usize) -> Option<u64> {
    let ChatItem::User {
        timing: Some(timing),
        ..
    } = &items[index]
    else {
        return None;
    };
    let current = timing.usage?;
    let previous = timing.usage_before?;
    current.total.checked_sub(previous.total)
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
        ChatItem::Child { at_ms, .. } => *at_ms,
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
        SessionStatus::Idle => AgentStatus::Stopped,
        SessionStatus::Running => AgentStatus::Running,
        SessionStatus::Waiting => AgentStatus::Waiting,
        SessionStatus::Done => AgentStatus::Done,
        SessionStatus::Failed => AgentStatus::Failed,
        SessionStatus::Interrupted => AgentStatus::Stopped,
    }
}

fn span_status(status: SessionStatus) -> SpanStatus {
    match status {
        SessionStatus::Idle => SpanStatus::Done,
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
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    fn session() -> SessionSummary {
        SessionSummary {
            workspace: None,
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
    fn empty_conversations_never_invent_agents_traces_or_costs() {
        for status in [
            SessionStatus::Idle,
            SessionStatus::Waiting,
            SessionStatus::Running,
            SessionStatus::Done,
            SessionStatus::Interrupted,
        ] {
            let mut session = session();
            session.status = status;
            assert!(nodes(&[], &project(), &session).is_empty());
            assert!(spans(&[], &session).is_empty());
        }
    }

    #[test]
    fn an_hour_between_requests_is_idle_not_the_previous_response() {
        use crate::harness::TurnTiming;
        let items = vec![
            ChatItem::User {
                text: "first".into(),
                at_ms: 1_000,
                timing: Some(TurnTiming {
                    started_at_ms: 1_000,
                    finished_at_ms: Some(4_000),
                    elapsed_ms: Some(3_000),
                    usage: None,
                    usage_before: None,
                    speaker: None,
                }),
            },
            ChatItem::AgentText {
                text: "done".into(),
                at_ms: 2_000,
                streaming: false,
                message_id: None,
            },
            ChatItem::User {
                text: "second".into(),
                at_ms: 3_604_000,
                timing: None,
            },
        ];
        let spans = spans(&items, &session());
        let response = spans.iter().find(|s| s.name == "Response").unwrap();
        assert_eq!(response.duration_ms, 2_000);
        assert!(response.duration_known);
        let gap = spans.iter().find(|s| s.id == "gap-0").unwrap();
        assert_eq!(gap.duration_ms, 3_600_000);
        assert_eq!(gap.start_ms, 3_000);
        assert_eq!(gap.name, "Idle between requests");
    }

    #[test]
    fn missing_completion_never_borrows_the_next_request_timestamp() {
        let items = vec![
            ChatItem::User {
                text: "first".into(),
                timing: None,
                at_ms: 1_000,
            },
            ChatItem::AgentText {
                text: "done".into(),
                streaming: false,
                message_id: None,
                at_ms: 2_000,
            },
            ChatItem::User {
                text: "second".into(),
                timing: None,
                at_ms: 3_604_000,
            },
        ];
        let spans = spans(&items, &session());
        let response = spans.iter().find(|s| s.name == "Response").unwrap();
        assert!(!response.duration_known);
        assert!(response.duration_ms <= 1);
        let gap = spans.iter().find(|s| s.id == "gap-0").unwrap();
        assert_eq!(gap.name, "Unrecorded gap between requests");
        assert!(gap.duration_ms > 3_600_000);
    }

    #[test]
    fn real_activity_is_projected_without_sample_agents() {
        let transcript = vec![ChatItem::User {
            text: "Hello".into(),
            timing: None,
            at_ms: 1,
        }];
        let nodes = nodes(&transcript, &project(), &session());
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, "root");
        assert_eq!(nodes[0].model, "Auto");
        assert!(!spans(&transcript, &session()).is_empty());
    }

    #[test]
    fn plan_steps_are_work_items_not_invented_subagents() {
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
        assert_eq!(nodes.len(), 1); // Plan steps are not graph agents or tool calls.
    }

    #[test]
    fn two_delegated_sessions_and_four_searches_are_two_agents_not_six() {
        let mut transcript = Vec::new();
        for id in ["child-one", "child-two"] {
            let mut delegated = tool();
            if let ChatItem::Tool { view, .. } = &mut delegated {
                view.id = format!("delegate-{id}");
                view.title = "subagent".into();
                view.detail = format!(
                    "<subagent sessionID=\"{id}\" state=\"completed\">\n# Result\n</subagent>"
                );
            }
            transcript.push(delegated);
            for n in 0..2 {
                let mut search = tool();
                if let ChatItem::Tool { view, .. } = &mut search {
                    view.id = format!("{id}:search-{n}");
                    view.title = "websearch".into();
                }
                transcript.push(search);
            }
        }
        let all = nodes(&transcript, &project(), &session());
        let projected = crate::activity::graph_nodes(
            all.clone(),
            &all.iter().map(|node| node.id.clone()).collect(),
        );
        assert_eq!(
            projected
                .iter()
                .filter(|node| node.kind == dunlin_domain::AgentKind::Subagent)
                .count(),
            2
        );
        assert_eq!(
            projected
                .iter()
                .filter(|node| node.kind == dunlin_domain::AgentKind::Tool)
                .count(),
            4
        );
        assert!(
            projected
                .iter()
                .filter(|node| node.kind == dunlin_domain::AgentKind::Tool)
                .all(|node| node
                    .parent
                    .as_deref()
                    .is_some_and(|p| p.starts_with("child-")))
        );
        assert!(
            projected
                .iter()
                .all(|node| !node.task.contains("<subagent"))
        );
        for node in projected.iter().skip(1) {
            let parent = projected
                .iter()
                .find(|parent| Some(parent.id.as_str()) == node.parent.as_deref())
                .unwrap();
            assert!(node.y >= parent.y + parent.height);
            assert!(node.x.is_finite() && node.y.is_finite());
        }
        for (index, node) in projected.iter().enumerate() {
            for other in projected
                .iter()
                .skip(index + 1)
                .filter(|other| other.y == node.y)
            {
                assert!(node.x + node.width <= other.x || other.x + other.width <= node.x);
            }
        }
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
                timing: None,
                at_ms: 1_000,
            },
            tool(),
            ChatItem::AgentText {
                text: "All tests passed.".into(),
                streaming: false,
                message_id: None,
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

    #[test]
    fn trace_uses_recorded_usage_deltas_not_context_occupancy_or_text_estimates() {
        use crate::harness::TurnTiming;
        use dunlin_agent::TokenUsage;
        let user = |total: Option<u64>, before: Option<u64>| ChatItem::User {
            text: "Request".into(),
            at_ms: 1000,
            timing: Some(TurnTiming {
                started_at_ms: 1100,
                finished_at_ms: Some(1600),
                elapsed_ms: Some(500),
                usage: total.map(|total| TokenUsage {
                    total,
                    ..Default::default()
                }),
                usage_before: before.map(|total| TokenUsage {
                    total,
                    ..Default::default()
                }),
                speaker: None,
            }),
        };
        for (total, baseline, expected) in [
            (Some(1200), Some(1000), Some(200)),
            (Some(1200), Some(0), Some(1200)),
            (Some(1200), None, None),
            (None, Some(0), None),
            (Some(20), Some(1200), None),
        ] {
            let items = vec![user(total, baseline), tool()];
            let spans = spans(&items, &session());
            let request = spans
                .iter()
                .find(|span| span.kind == SpanKind::Turn)
                .unwrap();
            assert_eq!(request.tokens, expected);
            assert_eq!(request.duration_ms, 500);
            assert_eq!(spans[0].tokens, total); // Last reported cumulative snapshot, not session.tokens (context).
            assert!(
                spans
                    .iter()
                    .filter(|span| span.kind == SpanKind::Tool)
                    .all(|span| span.tokens.is_none())
            );
            let restored: Vec<ChatItem> =
                serde_json::from_str(&serde_json::to_string(&items).unwrap()).unwrap();
            assert_eq!(super::turn_tokens(&restored, 0), expected);
        }
    }
}
