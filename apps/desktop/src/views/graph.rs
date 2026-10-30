//! Agent graph: the orchestrator tree, subagent fan-out, handoffs and live
//! status painted on a quiet dot grid.

use std::time::Duration;

use dunlin_domain::{AgentKind, AgentNode, AgentStatus, MainView, SessionStatus, ToolKind};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{ActiveTheme, Icon};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, agent_status_palette, format_duration, tone_color, tool_icon};

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let session = this.session().clone();
    let project = this.project().clone();
    if this.activity.prepare(&session.id) {
        this.selected_node = None;
        this.selected_span = None;
    }
    let all = crate::projection::nodes(&this.harness.transcript, &project, &session);
    let nodes = crate::activity::graph_nodes(&all, &this.activity.agents_expanded);
    if nodes.is_empty() {
        return super::empty_activity(
            "No agent activity yet",
            "Send your first message in Chat. Only activity from this conversation will appear here.",
            IconName::Network,
            cx,
        );
    }
    let (badge_bg, badge_fg, badge_label) = match session.status {
        SessionStatus::Running => (p.green_soft, p.green, "LIVE"),
        SessionStatus::Waiting => (p.amber_soft, p.amber, "WAITING"),
        SessionStatus::Failed => (p.red_soft, p.red, "FAILED"),
        _ => (p.muted, p.muted_fg, "IDLE"),
    };
    let selected = this.selected_node.clone();
    let running = nodes
        .iter()
        .filter(|node| node.kind != AgentKind::Tool && node.status == AgentStatus::Running)
        .count();
    let total_cost: f64 = nodes.iter().map(|node| node.cost_usd).sum();

    let toolbar = div()
        .h(px(40.))
        .flex_shrink_0()
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .child(
            Icon::new(IconName::Network)
                .size(px(14.))
                .text_color(p.blue),
        )
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Agent graph"),
        )
        .child(
            div()
                .h(px(17.))
                .px(px(5.))
                .rounded_full()
                .bg(badge_bg)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(badge_fg, 4.5))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(badge_fg)
                        .child(badge_label),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(format!(
                    "{} subagents · {} tools · {running} running",
                    nodes
                        .iter()
                        .filter(|n| n.kind == AgentKind::Subagent)
                        .count(),
                    all.iter().filter(|n| n.kind == AgentKind::Tool).count()
                )),
        )
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .text_size(px(11.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(format!("${total_cost:.2}")),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("graph-export-svg")
                .aria_label("Save graph as SVG")
                .h(px(23.))
                .px(px(7.))
                .rounded(px(5.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.export_graph_svg(cx)))
                .child(
                    Icon::new(IconName::Download)
                        .size(px(11.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(p.muted_fg)
                        .child("Save SVG"),
                ),
        );

    let content = graph_canvas(
        &nodes,
        selected.as_deref(),
        &this.activity.agents_expanded,
        cx,
        &p,
    );

    let legend = div()
        .absolute()
        .left(px(12.))
        .bottom(px(10.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(7.))
        .border_1()
        .border_color(p.border)
        .bg(p.card.opacity(0.94))
        .flex()
        .items_center()
        .gap(px(9.))
        .child(legend_item("Running", p.blue, &p))
        .child(legend_item("Waiting", p.amber, &p))
        .child(legend_item("Done", p.green, &p))
        .child(legend_item("Handoff", p.violet, &p))
        .child(div().w(px(1.)).h(px(11.)).bg(p.border))
        .child(
            div()
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child("dashed = handoff"),
        );

    let detail = selected
        .as_ref()
        .and_then(|id| nodes.iter().find(|node| &node.id == id))
        .map(|node| node_detail(node, cx, &p));

    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.bg)
        .child(toolbar)
        .child(content)
        .child(legend);

    if let Some(detail) = detail {
        root = root.child(detail);
    }
    root.into_any_element()
}

fn graph_canvas(
    nodes: &[AgentNode],
    selected: Option<&str>,
    expanded: &std::collections::HashSet<String>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (width, height) = crate::activity::graph_size(nodes);

    let mut edges: Vec<Edge> = Vec::new();
    for node in nodes {
        let Some(parent_id) = &node.parent else {
            continue;
        };
        let Some(parent) = nodes.iter().find(|candidate| &candidate.id == parent_id) else {
            continue;
        };
        let color = match node.status {
            AgentStatus::Running => p.blue,
            AgentStatus::Waiting => p.amber,
            AgentStatus::Done => p.green.opacity(0.55),
            AgentStatus::Failed => p.red,
            AgentStatus::Stopped => p.muted_fg.opacity(0.5),
        };
        edges.push(Edge {
            x1: parent.x + parent.width / 2.0,
            y1: parent.y + parent.height,
            x2: node.x + node.width / 2.0,
            y2: node.y,
            color,
            running: node.status == AgentStatus::Running,
            handoff: node.kind == AgentKind::Subagent,
        });
    }

    let grid = p.graph_grid;
    let handoff_color = p.violet;
    let edge_layer = canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let origin = bounds.origin;
            for edge in &edges {
                let mid = (edge.y1 + edge.y2) / 2.0;
                line(
                    window,
                    origin,
                    edge.x1,
                    edge.y1,
                    1.0,
                    (mid - edge.y1).max(0.0),
                    edge.color,
                );
                let (left, span) = if edge.x1 <= edge.x2 {
                    (edge.x1, edge.x2 - edge.x1)
                } else {
                    (edge.x2, edge.x1 - edge.x2)
                };
                line(window, origin, left, mid, span.max(1.0), 1.0, edge.color);
                line(
                    window,
                    origin,
                    edge.x2,
                    mid,
                    1.0,
                    (edge.y2 - mid).max(0.0),
                    edge.color,
                );
                if edge.running {
                    dot_at(window, origin, edge.x1, mid, 3.0, edge.color);
                }
                if edge.handoff {
                    dot_at(window, origin, edge.x2, mid, 2.2, handoff_color);
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .w(px(width))
    .h(px(height))
    .with_animation(
        ElementId::NamedInteger("graph-edges".into(), nodes.len() as u64),
        Animation::new(Duration::from_millis(360)).with_easing(ease_out_quint()),
        |this, t| this.opacity(t),
    )
    .into_any_element();

    let mut layer = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .child(dot_grid(grid))
        .child(edge_layer);

    for node in nodes {
        layer = layer.child(node_card(
            node,
            selected == Some(node.id.as_str()),
            expanded.contains(&node.id),
            cx,
            p,
        ));
    }

    div()
        .id("graph-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .child(layer)
        .into_any_element()
}

struct Edge {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: Hsla,
    running: bool,
    handoff: bool,
}

fn line(
    window: &mut Window,
    origin: Point<Pixels>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: Hsla,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let bounds = Bounds::new(
        origin + point(px(x), px(y)),
        size(px(width.max(0.8)), px(height.max(0.8))),
    );
    window.paint_quad(quad(
        bounds,
        Corners::all(px(0.4)),
        color,
        Edges::default(),
        color,
        BorderStyle::default(),
    ));
}

fn dot_at(window: &mut Window, origin: Point<Pixels>, x: f32, y: f32, radius: f32, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(
            origin + point(px(x - radius), px(y - radius)),
            size(px(radius * 2.0), px(radius * 2.0)),
        ),
        Corners::all(px(radius)),
        color,
        Edges::default(),
        color,
        BorderStyle::default(),
    ));
}

fn node_card(
    node: &AgentNode,
    selected: bool,
    expanded: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    let is_tool = node.kind == AgentKind::Tool;
    let active = node.status == AgentStatus::Running;
    let selected_id = node.id.clone();
    let accent = tone_color(
        p,
        match node.kind {
            AgentKind::Orchestrator => dunlin_domain::ProjectTone::Blue,
            AgentKind::Subagent => dunlin_domain::ProjectTone::Violet,
            AgentKind::Tool => dunlin_domain::ProjectTone::Teal,
        },
    );
    let base_y = node.y;

    let mut card = div()
        .id(format!("agent-node:{}", node.id))
        .absolute()
        .left(px(node.x))
        .top(px(node.y))
        .w(px(node.width))
        .h(px(node.height))
        .rounded(px(11.))
        .border_1()
        .border_color(if selected {
            accent
        } else if active {
            p.blue.opacity(0.5)
        } else {
            p.border
        })
        .bg(if is_tool { p.subtle } else { p.card })
        .overflow_hidden()
        .cursor_pointer()
        .hover(move |style| style.border_color(p.border_strong))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_node = if this.selected_node.as_deref() == Some(&selected_id) {
                None
            } else {
                Some(selected_id.clone())
            };
            if !is_tool && !this.activity.agents_expanded.remove(&selected_id) {
                this.activity.agents_expanded.insert(selected_id.clone());
            }
            cx.notify();
        }))
        .child(
            div()
                .h(px(30.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(if selected {
                    accent.opacity(0.35)
                } else {
                    p.border
                })
                .bg(if selected {
                    accent.opacity(0.06)
                } else {
                    p.card
                })
                .child(dot(status_color, 5.5))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .truncate()
                        .child(node.title.clone()),
                )
                .child(
                    div()
                        .h(px(15.))
                        .px(px(4.))
                        .rounded(px(3.))
                        .bg(status_soft)
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(status_color)
                                .child(node.status.label().to_uppercase()),
                        ),
                ),
        )
        .child(
            div()
                .px(px(9.))
                .py(px(6.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .h(px(34.))
                        .overflow_hidden()
                        .text_size(px(11.0))
                        .line_height(relative(1.45))
                        .text_color(if is_tool {
                            p.muted_fg
                        } else {
                            p.fg.opacity(0.78)
                        })
                        .child(node.task.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .when(is_tool || expanded, |view| {
                            view.children(node.tools.iter().take(3).map(|kind| tool_chip(*kind, p)))
                        })
                        .when(!is_tool && !expanded, |view| {
                            view.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(p.muted_fg)
                                    .child("Click to show tools"),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.75))
                                .child(format!("${:.2}", node.cost_usd)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            Icon::new(IconName::Cpu)
                                .size(px(9.))
                                .text_color(p.muted_fg.opacity(0.65)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(10.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(node.model.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.7))
                                .child(format_duration(node.duration_secs)),
                        ),
                ),
        );

    if node.status == AgentStatus::Running {
        card = card.child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(2.))
                .bg(p.muted)
                .child(
                    div()
                        .h_full()
                        .w(relative(0.62))
                        .bg(status_color)
                        .rounded_full()
                        .with_animation(
                            ElementId::Name(format!("graph-running-{}", node.id).into()),
                            Animation::new(Duration::from_millis(1800))
                                .repeat_synced()
                                .with_easing(pulsating_between(0.4, 1.0)),
                            |this, t| this.opacity(t),
                        ),
                ),
        );
    }

    let node_id = node.id.clone();
    card.with_animation(
        ElementId::Name(format!("graph-node-{node_id}").into()),
        Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
        move |this, t| {
            this.top(px(base_y + (1.0 - t) * 10.0))
                .opacity(0.25 + 0.75 * t)
        },
    )
    .into_any_element()
}

fn tool_chip(kind: ToolKind, p: &Palette) -> AnyElement {
    div()
        .h(px(16.))
        .px(px(4.))
        .rounded(px(3.))
        .bg(p.muted)
        .flex()
        .items_center()
        .gap(px(3.))
        .child(
            Icon::new(tool_icon(kind))
                .size(px(8.5))
                .text_color(p.muted_fg),
        )
        .child(
            div()
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(kind.label()),
        )
        .into_any_element()
}

fn legend_item(label: &str, color: Hsla, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(dot(color, 5.0))
        .child(
            div()
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(label.to_string()),
        )
        .into_any_element()
}

fn node_detail(node: &AgentNode, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    let detail = div()
        .absolute()
        .right(px(12.))
        .bottom(px(10.))
        .w(px(338.))
        .p(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.card.opacity(0.97))
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(dot(status_color, 5.0))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child(node.title.clone()),
                )
                .child(
                    div()
                        .h(px(15.))
                        .px(px(4.))
                        .rounded(px(3.))
                        .bg(status_soft)
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(status_color)
                                .child(node.status.label().to_uppercase()),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("close-node-detail")
                        .size(px(18.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.selected_node = None;
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg)),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .line_height(relative(1.5))
                .text_color(p.muted_fg)
                .child(node.task.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(meta_chip(&node.model, p))
                .child(meta_chip(&node.harness, p))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "${:.2} · {}",
                            node.cost_usd,
                            format_duration(node.duration_secs)
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(node.tools.iter().map(|kind| tool_chip(*kind, p)))
                .child(div().flex_1())
                .child(
                    div()
                        .id("open-node-session")
                        .h(px(21.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(p.border)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_view(MainView::Chat, cx);
                            this.set_notice("Opened the live transcript", cx);
                        }))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(p.muted_fg)
                                .child("Transcript"),
                        )
                        .child(
                            Icon::new(IconName::ArrowUpRight)
                                .size(px(9.5))
                                .text_color(p.muted_fg),
                        ),
                ),
        );

    detail
        .with_animation(
            ElementId::Name(format!("graph-detail-{}", node.id).into()),
            Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
            |this, t| {
                this.opacity(t)
                    .relative()
                    .left(px((1.0 - t) * 8.0))
                    .top(px((1.0 - t) * 5.0))
            },
        )
        .into_any_element()
}

fn meta_chip(text: &str, p: &Palette) -> AnyElement {
    div()
        .h(px(16.))
        .px(px(4.))
        .rounded(px(3.))
        .bg(p.muted)
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(text.to_string()),
        )
        .into_any_element()
}

impl DunlinApp {
    fn export_graph_svg(&mut self, cx: &mut Context<Self>) {
        let all =
            crate::projection::nodes(&self.harness.transcript, self.project(), self.session());
        let nodes = crate::activity::graph_nodes(&all, &self.activity.agents_expanded);
        let svg = crate::activity::graph_svg(&nodes, &self.session().title, cx.theme().is_dark());
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join("Downloads"))
            .filter(|path| path.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let receiver = cx.prompt_for_new_path(&directory, Some("agent-graph.svg"));
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::write(&path, svg)
                            .map(|_| path)
                            .map_err(|error| error.to_string())
                    })
                    .await;
                let _ = this.update(cx, |this, cx| match result {
                    Ok(path) => this.set_notice(format!("Saved graph to {}", path.display()), cx),
                    Err(error) => this.set_notice(format!("Could not save SVG: {error}"), cx),
                });
            }
        })
        .detach();
    }
}
