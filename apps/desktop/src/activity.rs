//! View-only disclosure, virtualization and export. Never changes the recorded
//! transcript.
use crate::app::TraceFilter;
use dunlin_domain::{AgentKind, AgentNode, AgentStatus, SpanKind, TraceSpan};
use dunlin_ui::{ScrollHandle, UniformListScrollHandle};
use std::collections::{HashMap, HashSet};

pub(crate) struct State {
    session: String,
    pub agents_expanded: HashSet<String>,
    pub spans_expanded: HashSet<String>,
    pub graph_zoom: f32,
    pub trace_zoom: f32,
    /// First paint of a session centres the root instead of starting at the
    /// far-left of a wide fan-out.
    pub graph_centered: bool,
    /// The graph is a 2D canvas; this handle supplies both the scroll offset used
    /// for viewport culling and the maximum extent used by Fit and reveal.
    pub graph_scroll: ScrollHandle,
    /// The trace uses a uniform virtual list, so this handle also carries the
    /// deferred scroll-to-row request without rebuilding every row element.
    pub trace_scroll: UniformListScrollHandle,
}

impl Default for State {
    fn default() -> Self {
        Self {
            session: String::new(),
            agents_expanded: HashSet::new(),
            spans_expanded: HashSet::new(),
            graph_zoom: 1.,
            trace_zoom: 1.,
            graph_centered: false,
            graph_scroll: ScrollHandle::new(),
            trace_scroll: UniformListScrollHandle::new(),
        }
    }
}

impl State {
    pub fn prepare(&mut self, session: &str) -> bool {
        if self.session == session {
            return false;
        }
        *self = Self {
            session: session.into(),
            ..Default::default()
        };
        true
    }
}

pub(crate) fn graph_nodes(all: Vec<AgentNode>, expanded: &HashSet<String>) -> Vec<AgentNode> {
    let mut visible: Vec<_> = all
        .into_iter()
        .filter(|node| {
            node.kind != AgentKind::Tool
                || node.parent.as_ref().is_some_and(|id| expanded.contains(id))
        })
        .collect();
    if !visible.is_empty() {
        let children = visible.split_off(1);
        crate::projection::layout_children(&mut visible, children);
        // A root with no visible children must not retain its expanded layout.
        if visible.len() == 1 {
            visible[0].x = (1150. - visible[0].width) / 2.;
        }
    }
    visible
}

/// Keep matches and their ancestor chain so a revealed search result remains
/// visibly connected. Empty queries return the same allocation; this matters
/// when a 10k-node graph is scrolled or re-rendered.
pub(crate) fn filter_graph_nodes(nodes: Vec<AgentNode>, query: &str) -> Vec<AgentNode> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return nodes;
    }
    let matches = |node: &AgentNode| {
        node.title.to_ascii_lowercase().contains(&query)
            || node.task.to_ascii_lowercase().contains(&query)
            || node.model.to_ascii_lowercase().contains(&query)
            || node.harness.to_ascii_lowercase().contains(&query)
    };
    let by_id: HashMap<&str, &AgentNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut keep = HashSet::new();
    for node in nodes.iter().filter(|node| matches(node)) {
        keep.insert(node.id.clone());
        let mut parent = node.parent.as_deref();
        for _ in 0..=nodes.len() {
            let Some(id) = parent else { break };
            if !keep.insert(id.to_string()) {
                break;
            }
            parent = by_id.get(id).and_then(|node| node.parent.as_deref());
        }
    }
    drop(by_id);
    nodes
        .into_iter()
        .filter(|node| keep.contains(&node.id))
        .collect()
}

pub(crate) fn filter_spans(spans: Vec<TraceSpan>, query: &str) -> Vec<TraceSpan> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return spans;
    }
    let matches = |span: &TraceSpan| {
        span.name.to_ascii_lowercase().contains(&query)
            || span.agent.to_ascii_lowercase().contains(&query)
            || span.detail.to_ascii_lowercase().contains(&query)
    };
    let by_id: HashMap<&str, &TraceSpan> =
        spans.iter().map(|span| (span.id.as_str(), span)).collect();
    let mut keep = HashSet::new();
    for span in spans.iter().filter(|span| matches(span)) {
        keep.insert(span.id.clone());
        let mut parent = span.parent.as_deref();
        for _ in 0..=spans.len() {
            let Some(id) = parent else { break };
            if !keep.insert(id.to_string()) {
                break;
            }
            parent = by_id.get(id).and_then(|span| span.parent.as_deref());
        }
    }
    drop(by_id);
    spans
        .into_iter()
        .filter(|span| keep.contains(&span.id))
        .collect()
}

pub(crate) fn graph_size(nodes: &[AgentNode]) -> (f32, f32) {
    (
        nodes
            .iter()
            .map(|n| n.x + n.width + 24.)
            .fold(1150., f32::max),
        nodes
            .iter()
            .map(|n| n.y + n.height + 60.)
            .fold(620., f32::max),
    )
}

/// Exact content extent for scrolling and fit. Unlike the SVG-oriented
/// `graph_size`, this does not add the 1150×620 design minimum, so a small
/// graph does not gain a misleading scroll range.
pub(crate) fn graph_extent(nodes: &[AgentNode]) -> (f32, f32) {
    (
        nodes
            .iter()
            .map(|n| n.x + n.width + 24.)
            .fold(1.0, f32::max),
        nodes
            .iter()
            .map(|n| n.y + n.height + 60.)
            .fold(1.0, f32::max),
    )
}

/// Return indices into `spans` in waterfall order. The uniform list renders
/// only the requested slice, so this avoids cloning every span on each frame.
pub(crate) fn trace_row_indices(
    spans: &[TraceSpan],
    expanded: &HashSet<String>,
    filter: TraceFilter,
) -> Vec<usize> {
    let by_id: HashMap<_, _> = spans
        .iter()
        .enumerate()
        .map(|(index, span)| (span.id.as_str(), index))
        .collect();
    let mut relevant = HashSet::new();
    for span in spans.iter().filter(|span| filter.matches(span.kind)) {
        let mut current = Some(span);
        for _ in 0..=spans.len() {
            let Some(span) = current else {
                break;
            };
            if !relevant.insert(span.id.as_str()) {
                break;
            }
            current = span
                .parent
                .as_deref()
                .and_then(|id| by_id.get(id).and_then(|index| spans.get(*index)));
        }
    }
    let mut children: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
    for (index, span) in spans.iter().enumerate() {
        children
            .entry(span.parent.as_deref())
            .or_default()
            .push(index);
    }
    for rows in children.values_mut() {
        rows.sort_by_key(|index| (spans[*index].start_ms, *index));
    }
    let mut stack = children.get(&None).cloned().unwrap_or_default();
    stack.reverse();
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    while let Some(index) = stack.pop() {
        let span = &spans[index];
        if !relevant.contains(span.id.as_str()) || !seen.insert(index) {
            continue;
        }
        output.push(index);
        if (span.id == "root" || expanded.contains(&span.id))
            && let Some(children) = children.get(&Some(span.id.as_str()))
        {
            stack.extend(children.iter().rev().copied());
        }
    }
    output
}

/// A world-space rectangle used to cull an arbitrarily large graph before any
/// GPUI element is created. Offsets follow `ScrollHandle`: negative means the
/// content moved up/left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Viewport {
    pub(crate) fn new(
        offset_x: f32,
        offset_y: f32,
        width: f32,
        height: f32,
        overscan: f32,
    ) -> Self {
        let width = if width.is_finite() && width > 0.0 {
            width
        } else {
            1.0
        };
        let height = if height.is_finite() && height > 0.0 {
            height
        } else {
            1.0
        };
        let offset_x = if offset_x.is_finite() { offset_x } else { 0.0 };
        let offset_y = if offset_y.is_finite() { offset_y } else { 0.0 };
        let overscan = if overscan.is_finite() {
            overscan.max(0.0)
        } else {
            0.0
        };
        let left = -offset_x - overscan;
        let top = -offset_y - overscan;
        Self {
            left,
            top,
            right: left + width + overscan * 2.0,
            bottom: top + height + overscan * 2.0,
        }
    }

    pub(crate) fn intersects(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        x.is_finite()
            && y.is_finite()
            && width.is_finite()
            && height.is_finite()
            && width >= 0.0
            && height >= 0.0
            && x + width >= self.left
            && x <= self.right
            && y + height >= self.top
            && y <= self.bottom
    }
}

pub(crate) fn visible_graph_indices(nodes: &[AgentNode], viewport: Viewport) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| viewport.intersects(node.x, node.y, node.width, node.height))
        .map(|(index, _)| index)
        .collect()
}

/// Uniform scale keeps relative geometry and hit targets stable. The previous
/// clamp-to-minimum behaviour made 50% cards overlap their neighbours.
pub(crate) fn scaled_node_rect(node: &AgentNode, zoom: f32) -> (f32, f32, f32, f32) {
    let zoom = if zoom.is_finite() {
        zoom.clamp(0.5, 1.8)
    } else {
        1.0
    };
    (
        node.x * zoom,
        node.y * zoom,
        node.width * zoom,
        node.height * zoom,
    )
}

pub(crate) fn fit_zoom(
    content_width: f32,
    content_height: f32,
    viewport_width: f32,
    viewport_height: f32,
    min: f32,
    max: f32,
) -> f32 {
    if !content_width.is_finite()
        || !content_height.is_finite()
        || content_width <= 0.0
        || content_height <= 0.0
        || !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width <= 0.0
        || viewport_height <= 0.0
    {
        return 1.0;
    }
    (viewport_width / content_width)
        .min(viewport_height / content_height)
        .clamp(min, max)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NodeA11yMeta {
    pub level: usize,
    pub position: usize,
    pub siblings: usize,
    pub has_children: bool,
}

/// Metadata for tree semantics. Kept separate from `AgentNode` so the domain
/// type stays a projection concern rather than an accessibility implementation.
pub(crate) fn graph_a11y_meta(nodes: &[AgentNode]) -> HashMap<&str, NodeA11yMeta> {
    let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut kids: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut roots = Vec::new();
    for node in nodes {
        match node.parent.as_deref() {
            Some(parent) if parent != node.id && ids.contains(parent) => {
                kids.entry(parent).or_default().push(node.id.as_str());
            }
            _ => roots.push(node.id.as_str()),
        }
    }

    let mut levels: HashMap<&str, usize> = HashMap::new();
    let mut stack = Vec::new();
    for root in roots {
        if !levels.contains_key(root) {
            levels.insert(root, 0);
            stack.push(root);
        }
    }
    while let Some(parent) = stack.pop() {
        let level = levels[parent];
        if let Some(children) = kids.get(parent) {
            for child in children {
                if !levels.contains_key(child) {
                    levels.insert(child, level + 1);
                    stack.push(child);
                }
            }
        }
    }

    let mut totals: HashMap<Option<&str>, usize> = HashMap::new();
    for node in nodes {
        let parent = node
            .parent
            .as_deref()
            .filter(|parent| *parent != node.id && ids.contains(parent));
        *totals.entry(parent).or_default() += 1;
    }
    let mut positions: HashMap<Option<&str>, usize> = HashMap::new();
    let mut meta = HashMap::with_capacity(nodes.len());
    for node in nodes {
        let parent = node
            .parent
            .as_deref()
            .filter(|parent| *parent != node.id && ids.contains(parent));
        let position = positions.entry(parent).or_insert(0);
        *position += 1;
        meta.insert(
            node.id.as_str(),
            NodeA11yMeta {
                level: levels.get(node.id.as_str()).copied().unwrap_or(0),
                position: *position,
                siblings: totals.get(&parent).copied().unwrap_or(1),
                has_children: kids
                    .get(node.id.as_str())
                    .is_some_and(|children| !children.is_empty()),
            },
        );
    }
    meta
}

/// Sibling position/count for the rows that are actually visible. A global
/// waterfall index is not a valid tree position because collapsed and filtered
/// children are absent from the virtual list.
pub(crate) fn trace_a11y_positions(
    spans: &[TraceSpan],
    rows: &[usize],
) -> HashMap<usize, (usize, usize)> {
    let mut totals: HashMap<Option<&str>, usize> = HashMap::new();
    for index in rows {
        *totals.entry(spans[*index].parent.as_deref()).or_default() += 1;
    }
    let mut seen: HashMap<Option<&str>, usize> = HashMap::new();
    let mut output = HashMap::with_capacity(rows.len());
    for index in rows {
        let parent = spans[*index].parent.as_deref();
        let position = seen.entry(parent).or_insert(0);
        *position += 1;
        output.insert(*index, (*position, totals[&parent]));
    }
    output
}

pub(crate) fn node_label(node: &AgentNode) -> String {
    format!(
        "{}, {}, {}, {}",
        short(&node.title, 64),
        kind_label(node.kind),
        node.status.label(),
        crate::app::format_duration(node.duration_secs)
    )
}

pub(crate) fn node_description(node: &AgentNode) -> String {
    let tools = if node.tools.is_empty() {
        "no tools reported".into()
    } else {
        node.tools
            .iter()
            .map(|tool| tool.label())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{} · {} · {} · ${:.2} · {}",
        short(&node.task, 180),
        node.model,
        node.harness,
        node.cost_usd,
        tools
    )
}

pub(crate) fn span_label(span: &TraceSpan) -> String {
    let duration = if span.duration_known {
        crate::app::format_ms(span.duration_ms)
    } else {
        "unknown duration".into()
    };
    format!(
        "{}, {}, {}, {}, {}",
        short(&span.name, 90),
        span.kind.label(),
        span.agent,
        span.status.label(),
        duration
    )
}

pub(crate) fn span_description(span: &TraceSpan) -> String {
    let tokens = span
        .tokens
        .map(|tokens| format!("{} tokens", crate::app::format_tokens(tokens)))
        .unwrap_or_else(|| "tokens not reported".into());
    format!(
        "{} · depth {} · ${:.2} · {}",
        short(&span.detail, 180),
        span.depth,
        span.cost_usd,
        tokens
    )
}

/// Keep a selection on a row that actually exists after filtering or a
/// collapse. Falling back to the first visible row preserves the focus target
/// that a keyboard user or screen reader was following.
pub(crate) fn normalize_selection(current: Option<&str>, ids: &[String]) -> Option<String> {
    if let Some(current) = current
        && ids.iter().any(|id| id == current)
    {
        return Some(current.to_string());
    }
    ids.first().cloned()
}

pub(crate) fn move_selection(
    current: Option<&str>,
    ids: &[String],
    delta: isize,
) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let index = current
        .and_then(|id| ids.iter().position(|candidate| candidate == id))
        .map(|index| {
            if delta < 0 {
                index.saturating_sub(delta.unsigned_abs())
            } else {
                (index + delta as usize).min(ids.len() - 1)
            }
        })
        .unwrap_or(if delta < 0 { ids.len() - 1 } else { 0 });
    Some(ids[index].clone())
}

pub(crate) const MINIMAP_CELL: f32 = 2.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MinimapGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Option<AgentStatus>>,
}

/// Aggregate a large graph into a tiny status grid. A 10k-node minimap must not
/// create 10k elements, and drawing one quad per map cell is bounded.
pub(crate) fn minimap_cells(nodes: &[AgentNode], scale: f32) -> MinimapGrid {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        0.08
    };
    let max_x = nodes
        .iter()
        .map(|node| (node.x + node.width) * scale)
        .fold(0.0f32, f32::max);
    let max_y = nodes
        .iter()
        .map(|node| (node.y + node.height) * scale)
        .fold(0.0f32, f32::max);
    let cols = ((max_x / MINIMAP_CELL).ceil() as usize).max(1);
    let rows = ((max_y / MINIMAP_CELL).ceil() as usize).max(1);
    let mut cells = vec![None; cols * rows];
    for node in nodes {
        let x0 = ((node.x * scale / MINIMAP_CELL).floor() as usize).min(cols - 1);
        let y0 = ((node.y * scale / MINIMAP_CELL).floor() as usize).min(rows - 1);
        let x1 =
            (((node.x + node.width) * scale / MINIMAP_CELL).ceil() as usize).clamp(x0 + 1, cols);
        let y1 =
            (((node.y + node.height) * scale / MINIMAP_CELL).ceil() as usize).clamp(y0 + 1, rows);
        for y in y0..y1 {
            for x in x0..x1 {
                let cell = &mut cells[y * cols + x];
                let replace = cell
                    .map(|current| status_rank(node.status) >= status_rank(current))
                    .unwrap_or(true);
                if replace {
                    *cell = Some(node.status);
                }
            }
        }
    }
    MinimapGrid { cols, rows, cells }
}

fn status_rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Stopped => 0,
        AgentStatus::Done => 1,
        AgentStatus::Waiting => 2,
        AgentStatus::Running => 3,
        AgentStatus::Failed => 4,
    }
}

fn kind_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Orchestrator => "orchestrator",
        AgentKind::Subagent => "subagent",
        AgentKind::Tool => "tool",
    }
}

fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| *c >= ' ' || matches!(c, '\n' | '\r' | '\t'))
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => c.to_string(),
        })
        .collect()
}

fn short(text: &str, limit: usize) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= limit {
        clean
    } else {
        format!(
            "{}…",
            clean
                .chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    }
}

/// Standalone SVG of the currently disclosed tree, with real text and no external assets.
pub(crate) fn graph_svg(nodes: &[AgentNode], title: &str, dark: bool) -> String {
    use std::fmt::Write;
    let (width, height) = graph_size(nodes);
    let (bg, card, fg, muted, border) = if dark {
        ("#101112", "#191b1e", "#f0f1f3", "#a0a5ad", "#35383e")
    } else {
        ("#fafafa", "#ffffff", "#20242b", "#626974", "#d8dce2")
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\"><title>{}</title><rect width=\"100%\" height=\"100%\" fill=\"{bg}\"/><g font-family=\"system-ui, sans-serif\">",
        xml(title)
    );
    for node in nodes {
        if let Some(parent) = node
            .parent
            .as_ref()
            .and_then(|id| nodes.iter().find(|n| &n.id == id))
        {
            let (x1, y1, x2, y2) = (
                parent.x + parent.width / 2.,
                parent.y + parent.height,
                node.x + node.width / 2.,
                node.y,
            );
            let mid = (y1 + y2) / 2.;
            let _ = write!(
                svg,
                "<path d=\"M{x1} {y1}V{mid}H{x2}V{y2}\" fill=\"none\" stroke=\"{muted}\"/>"
            );
        }
    }
    for node in nodes {
        let (x, y, w, h) = (node.x, node.y, node.width, node.height);
        let status = match node.status {
            dunlin_domain::AgentStatus::Running => "#5ba5ff",
            dunlin_domain::AgentStatus::Waiting => "#dca94a",
            dunlin_domain::AgentStatus::Done => "#36c986",
            dunlin_domain::AgentStatus::Failed => "#eb6464",
            dunlin_domain::AgentStatus::Stopped => muted,
        };
        let _ = write!(
            svg,
            "<g transform=\"translate({x} {y})\"><title>{}</title><rect width=\"{w}\" height=\"{h}\" rx=\"11\" fill=\"{card}\" stroke=\"{border}\"/><circle cx=\"12\" cy=\"16\" r=\"3\" fill=\"{status}\"/><text x=\"22\" y=\"20\" fill=\"{fg}\" font-size=\"12\" font-weight=\"600\">{}</text><text x=\"10\" y=\"49\" fill=\"{muted}\" font-size=\"11\">{}</text><text x=\"10\" y=\"69\" fill=\"{status}\" font-size=\"10\">{}</text><text x=\"10\" y=\"94\" fill=\"{muted}\" font-size=\"10\">{}</text><text x=\"10\" y=\"114\" fill=\"{muted}\" font-size=\"10\">{} s</text></g>",
            xml(&node.task),
            xml(&short(&node.title, 32)),
            xml(&short(&node.task, 35)),
            xml(node.status.label()),
            xml(&short(&node.model, 37)),
            node.duration_secs
        );
    }
    svg.push_str("</g></svg>");
    svg
}

pub(crate) fn token_label(tokens: Option<u64>) -> String {
    tokens
        .map(|n| format!("{} tok", crate::app::format_tokens(n)))
        .unwrap_or_else(|| "— tok".into())
}

pub(crate) fn is_request(span: &TraceSpan) -> bool {
    span.kind == SpanKind::Turn && span.parent.as_deref() == Some("root")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::SpanStatus;

    fn node(id: &str, parent: Option<&str>, kind: AgentKind) -> AgentNode {
        AgentNode {
            id: id.into(),
            parent: parent.map(str::to_string),
            title: id.into(),
            task: "Task <&> żółć".into(),
            model: "Test".into(),
            harness: "Mock".into(),
            status: AgentStatus::Done,
            kind,
            tools: vec![],
            cost_usd: 0.,
            duration_secs: 1,
            x: 0.,
            y: 16.,
            width: 240.,
            height: 128.,
        }
    }
    fn span(id: &str, parent: Option<&str>, kind: SpanKind) -> TraceSpan {
        TraceSpan {
            id: id.into(),
            parent: parent.map(str::to_string),
            name: id.into(),
            agent: "Mock".into(),
            kind,
            status: SpanStatus::Done,
            start_ms: 0,
            duration_ms: 100,
            duration_known: true,
            depth: 0,
            cost_usd: 0.,
            tokens: None,
            detail: "".into(),
        }
    }

    #[test]
    fn graph_and_trace_search_keep_ancestor_context_and_ignore_empty_queries() {
        let all = vec![
            node("root", None, AgentKind::Orchestrator),
            node("child", Some("root"), AgentKind::Subagent),
            node("tool", Some("child"), AgentKind::Tool),
        ];
        assert_eq!(filter_graph_nodes(all.clone(), "  ").len(), 3);
        let filtered = filter_graph_nodes(all.clone(), "child");
        assert_eq!(
            filtered.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["root", "child"]
        );
        assert!(filter_graph_nodes(all.clone(), "missing").is_empty());

        let spans = vec![
            span("root", None, SpanKind::Turn),
            span("child", Some("root"), SpanKind::Tool),
            span("other", Some("root"), SpanKind::Llm),
        ];
        let filtered = filter_spans(spans.clone(), "child");
        assert_eq!(
            filtered.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["root", "child"]
        );
        assert_eq!(filter_spans(spans.clone(), "").len(), 3);
        assert!(filter_spans(spans, "missing").is_empty());
    }

    #[test]
    fn graph_defaults_to_real_agents_and_expands_only_the_clicked_agents_tools() {
        let all = vec![
            node("root", None, AgentKind::Orchestrator),
            node("child", Some("root"), AgentKind::Subagent),
            node("root-tool", Some("root"), AgentKind::Tool),
            node("child-tool", Some("child"), AgentKind::Tool),
        ];
        let collapsed = graph_nodes(all.clone(), &HashSet::new());
        assert_eq!(
            collapsed.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["root", "child"]
        );
        let expanded = graph_nodes(all.clone(), &HashSet::from(["child".into()]));
        assert_eq!(
            expanded.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["root", "child", "child-tool"]
        );
        assert!(expanded[2].y >= expanded[1].y + expanded[1].height);
        assert_eq!(all.len(), 4); // Disclosure never removes recorded work.
    }

    #[test]
    fn trace_defaults_to_requests_and_keeps_their_children_grouped_when_filtered() {
        // Projection emits request rows before event rows; rendering must group by parent.
        let all = vec![
            span("root", None, SpanKind::Agent),
            span("r1", Some("root"), SpanKind::Turn),
            span("r2", Some("root"), SpanKind::Turn),
            span("t1", Some("r1"), SpanKind::Tool),
            span("t2", Some("r2"), SpanKind::Tool),
        ];
        let ids = |rows: Vec<usize>| {
            rows.into_iter()
                .map(|index| all[index].id.as_str())
                .collect::<Vec<_>>()
        };
        let rows = trace_row_indices(&all, &HashSet::new(), TraceFilter::All);
        assert_eq!(ids(rows), ["root", "r1", "r2"]);
        let rows = trace_row_indices(&all, &HashSet::from(["r1".into()]), TraceFilter::All);
        assert_eq!(ids(rows), ["root", "r1", "t1", "r2"]);
        let rows = trace_row_indices(&all, &HashSet::from(["r1".into()]), TraceFilter::Tool);
        assert_eq!(ids(rows), ["root", "r1", "t1", "r2"]);
    }

    #[test]
    fn session_switches_reset_disclosure_and_unknown_tokens_are_not_zero() {
        let mut state = State::default();
        assert!(state.prepare("one"));
        state.agents_expanded.insert("root".into());
        state.spans_expanded.insert("turn-0".into());
        assert!(!state.prepare("one"));
        assert!(state.prepare("two"));
        assert!(state.agents_expanded.is_empty() && state.spans_expanded.is_empty());
        assert_eq!(token_label(None), "— tok");
        assert_eq!(token_label(Some(0)), "0 tok");
    }

    #[test]
    fn svg_escapes_content_and_exports_the_visible_layout_without_clipping() {
        let mut all = vec![node("root", None, AgentKind::Orchestrator)];
        for i in 0..12 {
            all.push(node(&format!("tool-{i}"), Some("root"), AgentKind::Tool));
        }
        all[0].title = "<script>alert('x')</script>".into();
        let collapsed = graph_nodes(all.clone(), &HashSet::new());
        let svg = graph_svg(&collapsed, "A&B <title>", true);
        assert!(svg.contains("A&amp;B &lt;title&gt;"));
        assert!(svg.contains("Task &lt;&amp;&gt; żółć"));
        assert!(!svg.contains("<script>"));
        assert!(!svg.contains("tool-0"));
        let expanded = graph_nodes(all.clone(), &HashSet::from(["root".into()]));
        let (w, h) = graph_size(&expanded);
        assert!(w > 1150.);
        assert!(
            expanded
                .iter()
                .all(|n| n.x >= 0. && n.y >= 0. && n.x + n.width <= w && n.y + n.height <= h)
        );
        let svg = graph_svg(&expanded, "Expanded", false);
        assert_eq!(svg.matches("<g transform=").count(), expanded.len());
        assert!(svg.contains(&format!("viewBox=\"0 0 {w} {h}\"")));
    }

    #[test]
    fn tidy_layout_handles_ten_thousand_deep_nodes_without_a_depth_cap() {
        let mut all = vec![node("root", None, AgentKind::Orchestrator)];
        let mut parent = "root".to_string();
        for index in 0..10_000 {
            let id = format!("agent-{index}");
            all.push(node(&id, Some(&parent), AgentKind::Subagent));
            parent = id;
        }
        let nodes = graph_nodes(all, &HashSet::new());
        assert_eq!(nodes.len(), 10_001);
        assert!(nodes.iter().all(|node| {
            node.x.is_finite() && node.y.is_finite() && node.width > 0.0 && node.height > 0.0
        }));
        for pair in nodes.windows(2) {
            assert!(pair[1].y >= pair[0].y + pair[0].height);
        }
        let (width, height) = graph_size(&nodes);
        assert!(width.is_finite() && height.is_finite());
    }

    #[test]
    fn viewport_culling_keeps_a_ten_thousand_node_graph_bounded() {
        let mut all = vec![node("root", None, AgentKind::Orchestrator)];
        for index in 0..10_000 {
            all.push(node(
                &format!("tool-{index}"),
                Some("root"),
                AgentKind::Tool,
            ));
        }
        let nodes = graph_nodes(all, &HashSet::from(["root".into()]));
        assert_eq!(nodes.len(), 10_001);
        let visible = visible_graph_indices(&nodes, Viewport::new(0.0, 0.0, 1_200.0, 800.0, 240.0));
        assert!(visible.len() < 100, "rendered {} nodes", visible.len());
        assert!(visible.iter().all(|index| *index < nodes.len()));
        assert!(!Viewport::new(f32::NAN, 0.0, 1.0, 1.0, 1.0).intersects(f32::NAN, 0.0, 1.0, 1.0));
    }

    #[test]
    fn minimap_aggregation_is_bounded_and_keeps_the_worst_status() {
        let mut all = vec![node("root", None, AgentKind::Orchestrator)];
        for index in 0..2_000 {
            let mut child = node(&format!("child-{index}"), Some("root"), AgentKind::Subagent);
            child.status = if index == 77 {
                AgentStatus::Failed
            } else {
                AgentStatus::Done
            };
            all.push(child);
        }
        let nodes = graph_nodes(all, &HashSet::new());
        let (width, _) = graph_size(&nodes);
        let scale = 168.0 / width;
        let grid = minimap_cells(&nodes, scale);
        assert!(grid.cells.len() <= 90 * 60);
        assert!(grid.cells.contains(&Some(AgentStatus::Failed)));
    }

    #[test]
    fn tree_metadata_exposes_levels_sibling_positions_and_active_children() {
        let nodes = vec![
            node("root", None, AgentKind::Orchestrator),
            node("a", Some("root"), AgentKind::Subagent),
            node("b", Some("root"), AgentKind::Subagent),
            node("a-child", Some("a"), AgentKind::Tool),
        ];
        let meta = graph_a11y_meta(&nodes);
        assert_eq!(meta["root"].level, 0);
        assert_eq!(meta["root"].siblings, 1);
        assert_eq!(meta["a"].level, 1);
        assert_eq!(meta["a"].position, 1);
        assert_eq!(meta["b"].position, 2);
        assert_eq!(meta["a"].siblings, 2);
        assert_eq!(meta["a-child"].level, 2);
        assert!(meta["root"].has_children);
        assert!(meta["a"].has_children);
        assert!(!meta["a-child"].has_children);
    }

    #[test]
    fn aria_labels_are_concise_and_keep_status_and_unknown_values_explicit() {
        let mut agent = node("agent", Some("root"), AgentKind::Subagent);
        agent.status = AgentStatus::Waiting;
        agent.title = "Very long delegated task ".repeat(40);
        agent.task = "Detail ".repeat(100);
        let label = node_label(&agent);
        assert!(label.contains("Waiting"));
        assert!(label.chars().count() < 120);
        assert!(node_description(&agent).chars().count() < 320);

        let mut span = span("request", Some("root"), SpanKind::Turn);
        span.status = SpanStatus::Running;
        span.duration_known = false;
        let label = span_label(&span);
        assert!(label.contains("running"));
        assert!(label.contains("unknown duration"));
        let description = span_description(&span);
        assert!(description.contains("tokens not reported"));
        assert!(!description.contains("0 tokens"));
    }

    #[test]
    fn selection_normalizes_and_clamps_instead_of_losing_focus() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(move_selection(None, &ids, 1).as_deref(), Some("a"));
        assert_eq!(move_selection(None, &ids, -1).as_deref(), Some("c"));
        assert_eq!(move_selection(Some("b"), &ids, 1).as_deref(), Some("c"));
        assert_eq!(move_selection(Some("b"), &ids, 99).as_deref(), Some("c"));
        assert_eq!(move_selection(Some("b"), &ids, -99).as_deref(), Some("a"));
        assert_eq!(normalize_selection(Some("b"), &ids).as_deref(), Some("b"));
        assert_eq!(
            normalize_selection(Some("gone"), &ids).as_deref(),
            Some("a")
        );
        assert_eq!(normalize_selection(None, &[]), None);
    }

    #[test]
    fn fit_zoom_is_bounded_and_never_produces_nan() {
        assert_eq!(fit_zoom(2_000.0, 1_000.0, 1_000.0, 800.0, 0.5, 1.8), 0.5);
        assert_eq!(fit_zoom(100.0, 100.0, 1_000.0, 800.0, 0.5, 1.8), 1.8);
        assert_eq!(fit_zoom(f32::NAN, 1.0, 1.0, 1.0, 0.5, 1.8), 1.0);
        assert_eq!(fit_zoom(1_000.0, 1_000.0, 2_000.0, 500.0, 0.5, 1.8), 0.5);
    }

    #[test]
    fn trace_tree_positions_count_only_visible_siblings() {
        let spans = vec![
            span("root", None, SpanKind::Agent),
            span("r1", Some("root"), SpanKind::Turn),
            span("r2", Some("root"), SpanKind::Turn),
            span("t1", Some("r1"), SpanKind::Tool),
            span("t2", Some("r2"), SpanKind::Tool),
        ];
        let rows = trace_row_indices(&spans, &HashSet::from(["r1".into()]), TraceFilter::All);
        assert_eq!(
            rows.iter()
                .map(|index| spans[*index].id.as_str())
                .collect::<Vec<_>>(),
            ["root", "r1", "t1", "r2"]
        );
        let positions = trace_a11y_positions(&spans, &rows);
        assert_eq!(positions[&0], (1, 1));
        assert_eq!(positions[&1], (1, 2));
        assert_eq!(positions[&2], (2, 2));
        assert_eq!(positions[&3], (1, 1));
    }

    #[test]
    fn ten_thousand_trace_rows_are_indexed_without_cloning_every_span() {
        let mut spans = vec![span("root", None, SpanKind::Agent)];
        for index in 0..10_000 {
            let mut row = span(&format!("turn-{index}"), Some("root"), SpanKind::Turn);
            row.start_ms = index;
            spans.push(row);
        }
        let rows = trace_row_indices(&spans, &HashSet::new(), TraceFilter::All);
        assert_eq!(rows.len(), 10_001);
        assert_eq!(spans[rows[0]].id, "root");
        assert_eq!(spans[rows[1]].id, "turn-0");
        assert_eq!(spans[*rows.last().unwrap()].id, "turn-9999");
    }
}
