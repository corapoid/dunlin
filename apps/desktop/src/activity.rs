//! View-only disclosure and export. Never changes the recorded transcript.
use crate::app::TraceFilter;
use dunlin_domain::{AgentKind, AgentNode, SpanKind, TraceSpan};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct State {
    session: String,
    pub agents_expanded: HashSet<String>,
    pub spans_expanded: HashSet<String>,
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

pub(crate) fn graph_nodes(all: &[AgentNode], expanded: &HashSet<String>) -> Vec<AgentNode> {
    let mut visible: Vec<_> = all
        .iter()
        .filter(|node| {
            node.kind != AgentKind::Tool
                || node.parent.as_ref().is_some_and(|id| expanded.contains(id))
        })
        .cloned()
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

pub(crate) fn trace_rows(
    spans: &[TraceSpan],
    expanded: &HashSet<String>,
    filter: TraceFilter,
) -> Vec<TraceSpan> {
    let by_id: HashMap<_, _> = spans.iter().map(|span| (span.id.as_str(), span)).collect();
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
            current = span.parent.as_deref().and_then(|id| by_id.get(id).copied());
        }
    }
    let mut children: HashMap<Option<&str>, Vec<&TraceSpan>> = HashMap::new();
    for span in spans {
        children
            .entry(span.parent.as_deref())
            .or_default()
            .push(span);
    }
    for rows in children.values_mut() {
        rows.sort_by_key(|span| span.start_ms);
    }
    let mut stack = children.get(&None).cloned().unwrap_or_default();
    stack.reverse();
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    while let Some(span) = stack.pop() {
        if !relevant.contains(span.id.as_str()) || !seen.insert(span.id.as_str()) {
            continue;
        }
        output.push(span.clone());
        if (span.id == "root" || expanded.contains(&span.id))
            && let Some(children) = children.get(&Some(span.id.as_str()))
        {
            stack.extend(children.iter().rev().copied());
        }
    }
    output
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
    use dunlin_domain::{AgentStatus, SpanStatus};

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
    fn graph_defaults_to_real_agents_and_expands_only_the_clicked_agents_tools() {
        let all = vec![
            node("root", None, AgentKind::Orchestrator),
            node("child", Some("root"), AgentKind::Subagent),
            node("root-tool", Some("root"), AgentKind::Tool),
            node("child-tool", Some("child"), AgentKind::Tool),
        ];
        let collapsed = graph_nodes(&all, &HashSet::new());
        assert_eq!(
            collapsed.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["root", "child"]
        );
        let expanded = graph_nodes(&all, &HashSet::from(["child".into()]));
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
        let rows = trace_rows(&all, &HashSet::new(), TraceFilter::All);
        assert_eq!(
            rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["root", "r1", "r2"]
        );
        let rows = trace_rows(&all, &HashSet::from(["r1".into()]), TraceFilter::All);
        assert_eq!(
            rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["root", "r1", "t1", "r2"]
        );
        let rows = trace_rows(&all, &HashSet::from(["r1".into()]), TraceFilter::Tool);
        assert_eq!(
            rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["root", "r1", "t1", "r2"]
        );
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
        let collapsed = graph_nodes(&all, &HashSet::new());
        let svg = graph_svg(&collapsed, "A&B <title>", true);
        assert!(svg.contains("A&amp;B &lt;title&gt;"));
        assert!(svg.contains("Task &lt;&amp;&gt; żółć"));
        assert!(!svg.contains("<script>"));
        assert!(!svg.contains("tool-0"));
        let expanded = graph_nodes(&all, &HashSet::from(["root".into()]));
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
}
