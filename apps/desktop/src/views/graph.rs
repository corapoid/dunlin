//! Agent graph: the orchestrator tree, subagent fan-out, handoffs and live
//! status painted on a quiet dot grid.

use kestrel_domain::{AgentKind, AgentNode, AgentStatus, MainView, SessionStatus, ToolKind};
use kestrel_ui::assets::IconName;
use kestrel_ui::component::Icon;
use kestrel_ui::*;

use crate::app::{KestrelApp, agent_status_palette, format_duration, tone_color, tool_icon};

pub(crate) fn render(
    this: &mut KestrelApp,
    _window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let preview = this.harness.transcript.is_empty();
    let session = this.session().clone();
    let project = this.project().clone();
    let nodes = if preview {
        crate::demo::agent_nodes()
    } else {
        crate::projection::nodes(&this.harness.transcript, &project, &session)
    };
    let (badge_bg, badge_fg, badge_label) = if preview {
        (p.amber_soft, p.amber, "PREVIEW")
    } else {
        match session.status {
            SessionStatus::Running => (p.green_soft, p.green, "LIVE"),
            SessionStatus::Waiting => (p.amber_soft, p.amber, "WAITING"),
            SessionStatus::Failed => (p.red_soft, p.red, "FAILED"),
            _ => (p.muted, p.muted_fg, "IDLE"),
        }
    };
    let selected = this.selected_node;
    let following = this.follow_running;
    let zoom = this.graph_zoom;
    let running = nodes
        .iter()
        .filter(|node| node.status == AgentStatus::Running)
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
                .text_size(px(11.))
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
                        .text_size(px(8.))
                        .font_family("Geist Mono")
                        .text_color(badge_fg)
                        .child(badge_label),
                ),
        )
        .child(
            div()
                .text_size(px(9.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!("{} nodes · {running} running", nodes.len())),
        )
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .text_size(px(9.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!("${total_cost:.2}")),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("graph-follow")
                .h(px(23.))
                .px(px(7.))
                .rounded(px(5.))
                .border_1()
                .border_color(if following {
                    p.blue.opacity(0.3)
                } else {
                    p.border
                })
                .bg(if following { p.blue_soft } else { p.card })
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.follow_running = !this.follow_running;
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Crosshair)
                        .size(px(11.))
                        .text_color(if following { p.blue } else { p.muted_fg }),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .text_color(if following { p.blue } else { p.muted_fg })
                        .child("Follow"),
                ),
        )
        .child(
            div()
                .h(px(23.))
                .px(px(6.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap(px(2.))
                .child(zoom_button(IconName::ZoomOut, cx, &p))
                .child(
                    div()
                        .min_w(px(32.))
                        .text_center()
                        .text_size(px(9.))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(format!("{:.0}%", zoom * 100.)),
                )
                .child(zoom_button(IconName::ZoomIn, cx, &p)),
        )
        .child(
            div()
                .id("graph-fit")
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
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.graph_zoom = 1.0;
                    this.set_notice("Graph fitted to the visible tree", cx);
                }))
                .child(
                    Icon::new(IconName::Maximize2)
                        .size(px(10.5))
                        .text_color(p.muted_fg),
                )
                .child(div().text_size(px(9.)).text_color(p.muted_fg).child("Fit")),
        );

    let content = graph_canvas(&nodes, selected, following, cx, &p);

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
                .text_size(px(8.))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child("dashed = handoff"),
        );

    let detail = selected
        .and_then(|index| nodes.get(index))
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

fn zoom_button(icon_name: IconName, cx: &mut Context<KestrelApp>, p: &Palette) -> AnyElement {
    let delta = if icon_name == IconName::ZoomIn {
        0.1
    } else {
        -0.1
    };
    div()
        .id(if delta > 0.0 {
            "graph-zoom-in"
        } else {
            "graph-zoom-out"
        })
        .size(px(19.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.graph_zoom = (this.graph_zoom + delta).clamp(0.6, 1.4);
            cx.notify();
        }))
        .child(Icon::new(icon_name).size(px(11.)).text_color(p.muted_fg))
        .into_any_element()
}

fn graph_canvas(
    nodes: &[AgentNode],
    selected: Option<usize>,
    following: bool,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    let width = 1150.0f32;
    let height = 620.0f32;

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
    .h(px(height));

    let mut layer = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .child(dot_grid(grid))
        .child(edge_layer);

    for (index, node) in nodes.iter().enumerate() {
        layer = layer.child(node_card(
            index,
            node,
            selected == Some(index),
            following,
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
    index: usize,
    node: &AgentNode,
    selected: bool,
    following: bool,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    let is_tool = node.kind == AgentKind::Tool;
    let kind_label = match node.kind {
        AgentKind::Orchestrator => "orchestrator",
        AgentKind::Subagent => "subagent",
        AgentKind::Tool => "tool",
    };
    let active = following && node.status == AgentStatus::Running;
    let accent = tone_color(
        p,
        match node.kind {
            AgentKind::Orchestrator => kestrel_domain::ProjectTone::Blue,
            AgentKind::Subagent => kestrel_domain::ProjectTone::Violet,
            AgentKind::Tool => kestrel_domain::ProjectTone::Teal,
        },
    );

    let mut card = div()
        .id(("agent-node", index))
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
            this.selected_node = if this.selected_node == Some(index) {
                None
            } else {
                Some(index)
            };
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
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .truncate()
                        .child(node.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(7.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.75))
                        .child(kind_label),
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
                                .text_size(px(7.))
                                .font_family("Geist Mono")
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
                        .h(px(30.))
                        .text_size(px(9.))
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
                        .children(node.tools.iter().take(3).map(|kind| tool_chip(*kind, p)))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(7.5))
                                .font_family("Geist Mono")
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
                                .text_size(px(8.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(node.model.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(8.))
                                .font_family("Geist Mono")
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
                        .rounded_full(),
                ),
        );
    }

    card.into_any_element()
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
                .text_size(px(7.))
                .font_family("Geist Mono")
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
                .text_size(px(8.))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(label.to_string()),
        )
        .into_any_element()
}

fn node_detail(node: &AgentNode, cx: &mut Context<KestrelApp>, p: &Palette) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    div()
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
                        .text_size(px(11.))
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
                                .text_size(px(7.))
                                .font_family("Geist Mono")
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
                .text_size(px(9.5))
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
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
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
                            this.view = MainView::Chat;
                            this.settings_open = false;
                            this.set_notice("Opened the live transcript", cx);
                        }))
                        .child(
                            div()
                                .text_size(px(8.5))
                                .text_color(p.muted_fg)
                                .child("Transcript"),
                        )
                        .child(
                            Icon::new(IconName::ArrowUpRight)
                                .size(px(9.5))
                                .text_color(p.muted_fg),
                        ),
                ),
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
                .text_size(px(7.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(text.to_string()),
        )
        .into_any_element()
}
