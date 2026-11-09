//! Agent graph: the orchestrator tree, subagent fan-out, handoffs and live
//! status painted on a quiet dot grid.

use std::time::Duration;

use dunlin_domain::{AgentKind, AgentNode, AgentStatus, MainView, SessionStatus, ToolKind};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Disableable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::{ActiveTheme, Icon, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, agent_status_palette, format_duration, tone_color, tool_icon};

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
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
    if all.is_empty() {
        return super::empty_activity(
            "No agent activity yet",
            "Send your first message in Chat. Only activity from this conversation will appear here.",
            IconName::Network,
            cx,
        );
    }
    let subagents = all
        .iter()
        .filter(|node| node.kind == AgentKind::Subagent)
        .count();
    let tools = all
        .iter()
        .filter(|node| node.kind == AgentKind::Tool)
        .count();
    let running = all
        .iter()
        .filter(|node| node.kind != AgentKind::Tool && node.status == AgentStatus::Running)
        .count();
    let total_cost: f64 = all.iter().map(|node| node.cost_usd).sum();
    let expandable: std::collections::HashSet<String> =
        all.iter().filter_map(|node| node.parent.clone()).collect();
    let (badge_bg, badge_fg, badge_label) = match session.status {
        SessionStatus::Running => (p.green_soft, p.green, "LIVE"),
        SessionStatus::Waiting => (p.amber_soft, p.amber, "WAITING"),
        SessionStatus::Failed => (p.red_soft, p.red, "FAILED"),
        _ => (p.muted, p.muted_fg, "IDLE"),
    };
    let nodes = crate::activity::graph_nodes(all, &this.activity.agents_expanded);
    let mut visible_nodes = crate::activity::filter_graph_nodes(nodes, &this.activity_query);
    if visible_nodes.is_empty() {
        return super::empty_activity(
            "No graph matches",
            "Clear the search field to show the full agent graph.",
            IconName::Network,
            cx,
        );
    }
    let zoom = this.activity.graph_zoom.clamp(0.5, 1.8);
    scale_nodes(&mut visible_nodes, zoom * crate::workspace_text::scale(cx));
    let selected = this.selected_node.clone();
    let scroll = this.activity.graph_scroll.clone();
    let focus = this.activity_focus.clone();
    if !this.activity.graph_centered
        && let Some(root) = visible_nodes.iter().find(|node| node.parent.is_none())
    {
        this.activity.graph_centered = true;
        let (x, y, width, height) = crate::activity::scaled_node_rect(root, zoom);
        let bounds = scroll.bounds().size;
        let viewport_width = if f32::from(bounds.width) > 0.0 {
            f32::from(bounds.width)
        } else {
            f32::from(window.viewport_size().width).max(800.0)
        };
        let viewport_height = if f32::from(bounds.height) > 0.0 {
            f32::from(bounds.height)
        } else {
            f32::from(window.viewport_size().height).max(600.0)
        };
        scroll.set_offset(point(
            px(-(x + width / 2.0 - viewport_width / 2.0).max(0.0)),
            px(-(y + height / 2.0 - viewport_height / 2.0).max(0.0)),
        ));
    }

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
                .flex()
                .items_center()
                .child(
                    MorphText::new("graph-subagents", subagents.to_string())
                        .numeric()
                        .scope(session.id.clone())
                        .reduce_motion(this.reduce_motion),
                )
                .child(" subagents · ")
                .child(
                    MorphText::new("graph-tools", tools.to_string())
                        .numeric()
                        .scope(session.id.clone())
                        .reduce_motion(this.reduce_motion),
                )
                .child(" tools · ")
                .child(
                    MorphText::new("graph-running", running.to_string())
                        .numeric()
                        .scope(session.id.clone())
                        .reduce_motion(this.reduce_motion),
                )
                .child(" running"),
        )
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .text_size(px(11.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(
                    MorphText::new("graph-total-cost", format!("${total_cost:.2}"))
                        .numeric()
                        .scope(session.id.clone())
                        .reduce_motion(this.reduce_motion),
                ),
        )
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .w(px(150.))
                .child(search_input(&this.activity_search, "Search agent graph")),
        )
        .child(
            dunlin_ui::button("graph-zoom-out")
                .ghost()
                .small()
                .label("−")
                .accessibility_label("Zoom graph out")
                .disabled(zoom <= 0.5)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.activity.graph_zoom = (this.activity.graph_zoom - 0.1).clamp(0.5, 1.8);
                    cx.notify();
                })),
        )
        .child(
            div()
                .min_w(px(34.))
                .text_center()
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(format!("{:.0}%", zoom * 100.)),
        )
        .child(
            dunlin_ui::button("graph-zoom-in")
                .ghost()
                .small()
                .label("+")
                .accessibility_label("Zoom graph in")
                .disabled(zoom >= 1.8)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.activity.graph_zoom = (this.activity.graph_zoom + 0.1).clamp(0.5, 1.8);
                    cx.notify();
                })),
        )
        .child(
            dunlin_ui::button("graph-zoom-reset")
                .ghost()
                .small()
                .label("Fit")
                .accessibility_label("Fit graph to the window")
                .on_click(cx.listener(|this, _, _, cx| this.fit_graph(cx))),
        )
        .child(div().flex_1())
        .child(
            dunlin_ui::action_button("graph-export-svg")
                .icon(IconName::Download)
                .label("Save SVG")
                .accessibility_label("Save graph as SVG")
                .on_click(cx.listener(|this, _, _, cx| this.export_graph_svg(cx))),
        );

    let content = graph_canvas(
        &visible_nodes,
        selected.as_deref(),
        &this.activity.agents_expanded,
        &expandable,
        this.reduce_motion,
        &scroll,
        &focus,
        window,
        cx,
        &p,
    );

    let legend = div()
        .absolute()
        .left(px(12.))
        .bottom(px(10.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(dunlin_ui::radii::CONTROL))
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
        .and_then(|id| visible_nodes.iter().find(|node| &node.id == id))
        .map(|node| node_detail(node, this.reduce_motion, cx, &p));

    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.card)
        .child(toolbar)
        .child(content)
        .child(legend)
        .child(graph_minimap(&visible_nodes, &scroll, &p));

    if let Some(detail) = detail {
        root = root.child(detail);
    }
    root.into_any_element()
}

/// Scale geometry without clamping individual cards. Clamping made adjacent
/// cards overlap at the minimum zoom and broke hit testing as well as reading
/// order; a uniform transform keeps the layout truthful at every density.
fn scale_nodes(nodes: &mut [AgentNode], zoom: f32) {
    for node in nodes {
        let (x, y, width, height) = crate::activity::scaled_node_rect(node, zoom);
        node.x = x;
        node.y = y;
        node.width = width;
        node.height = height;
    }
}

fn graph_minimap(nodes: &[AgentNode], scroll: &ScrollHandle, p: &Palette) -> AnyElement {
    let (width, height) = crate::activity::graph_extent(nodes);
    let scale = (168.0 / width).min(104.0 / height).min(0.08);
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        0.08
    };
    let grid = crate::activity::minimap_cells(nodes, scale);
    let map_width = grid.cols as f32 * crate::activity::MINIMAP_CELL;
    let map_height = grid.rows as f32 * crate::activity::MINIMAP_CELL;
    let offset = scroll.offset();
    let bounds = scroll.bounds().size;
    let view_left = (-f32::from(offset.x) * scale).clamp(0.0, map_width);
    let view_top = (-f32::from(offset.y) * scale).clamp(0.0, map_height);
    let view_width = (f32::from(bounds.width) * scale).clamp(1.0, map_width);
    let view_height = (f32::from(bounds.height) * scale).clamp(1.0, map_height);
    let view_left = view_left.min((map_width - view_width).max(0.0));
    let view_top = view_top.min((map_height - view_height).max(0.0));
    let cells = grid.cells;
    let cols = grid.cols;
    let viewport_outline = p.graph_ring;
    let running_color = p.blue;
    let waiting_color = p.amber;
    let done_color = p.green;
    let failed_color = p.red;
    let stopped_color = p.muted_fg;

    div()
        .id("graph-minimap")
        .role(Role::Image)
        .aria_label(format!("Graph minimap with {} visible nodes", nodes.len()))
        .absolute()
        .right(px(12.))
        .bottom(px(44.))
        .w(px(map_width + 8.))
        .h(px(map_height + 8.))
        .rounded(px(dunlin_ui::radii::ICON))
        .border_1()
        .border_color(p.border)
        .bg(p.card.opacity(0.88))
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    for (index, status) in cells.into_iter().enumerate() {
                        let Some(status) = status else { continue };
                        let x = (index % cols) as f32 * crate::activity::MINIMAP_CELL;
                        let y = (index / cols) as f32 * crate::activity::MINIMAP_CELL;
                        let color = match status {
                            AgentStatus::Running => running_color,
                            AgentStatus::Waiting => waiting_color,
                            AgentStatus::Done => done_color,
                            AgentStatus::Failed => failed_color,
                            AgentStatus::Stopped => stopped_color,
                        };
                        window.paint_quad(quad(
                            Bounds::new(
                                bounds.origin + point(px(4. + x), px(4. + y)),
                                size(
                                    px(crate::activity::MINIMAP_CELL),
                                    px(crate::activity::MINIMAP_CELL),
                                ),
                            ),
                            Corners::all(px(0.5)),
                            color,
                            Edges::default(),
                            color,
                            BorderStyle::default(),
                        ));
                    }
                    let mut outline = |x: f32, y: f32, w: f32, h: f32| {
                        window.paint_quad(quad(
                            Bounds::new(
                                bounds.origin + point(px(4. + x), px(4. + y)),
                                size(px(w.max(1.0)), px(h.max(1.0))),
                            ),
                            Corners::all(px(0.5)),
                            viewport_outline.opacity(0.0),
                            Edges::all(px(1.)),
                            viewport_outline,
                            BorderStyle::default(),
                        ));
                    };
                    outline(view_left, view_top, view_width, view_height);
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn graph_canvas(
    nodes: &[AgentNode],
    selected: Option<&str>,
    expanded: &std::collections::HashSet<String>,
    expandable: &std::collections::HashSet<String>,
    reduce_motion: bool,
    scroll: &ScrollHandle,
    focus: &FocusHandle,
    window: &Window,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (width, height) = crate::activity::graph_extent(nodes);
    let viewport = graph_viewport(scroll, window);
    let by_id: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    for node in nodes {
        let Some(parent) = node
            .parent
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied())
        else {
            continue;
        };
        let parent = &nodes[parent];
        let color = match node.status {
            AgentStatus::Running => p.blue,
            AgentStatus::Waiting => p.amber,
            AgentStatus::Done => p.green.opacity(0.55),
            AgentStatus::Failed => p.red,
            AgentStatus::Stopped => p.muted_fg.opacity(0.5),
        };
        let x1 = parent.x + parent.width / 2.0;
        let y1 = parent.y + parent.height;
        let x2 = node.x + node.width / 2.0;
        let y2 = node.y;
        let left = x1.min(x2);
        let right = x1.max(x2);
        let top = y1.min(y2);
        let bottom = y1.max(y2);
        if !viewport.intersects(left, top, (right - left).max(1.0), (bottom - top).max(1.0)) {
            continue;
        }
        edges.push(Edge {
            x1,
            y1,
            x2,
            y2,
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
        Animation::new(if reduce_motion {
            Duration::ZERO
        } else {
            Duration::from_millis(360)
        })
        .with_easing(ease_out_quint()),
        |this, t| this.opacity(t),
    )
    .into_any_element();

    let mut layer = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .child(edge_layer);

    let visible = crate::activity::visible_graph_indices(nodes, viewport);
    let meta = crate::activity::graph_a11y_meta(nodes);
    for index in &visible {
        let node = &nodes[*index];
        layer = layer.child(node_card(
            node,
            meta.get(node.id.as_str()),
            selected == Some(node.id.as_str()),
            expanded.contains(&node.id),
            expandable.contains(node.id.as_str()),
            reduce_motion,
            cx,
            p,
        ));
    }

    let viewport = div()
        .id("graph-scroll")
        .role(Role::Tree)
        .aria_label("Agent graph")
        .aria_description(format!(
            "{} of {} nodes are in the current viewport.",
            visible.len(),
            nodes.len()
        ))
        .aria_keyshortcuts("Up Down Left Right Enter Space Home End Escape")
        .track_focus(focus)
        .tab_stop(true)
        .track_scroll(scroll)
        .on_scroll(cx.listener(|this, _, _, cx| {
            // A scroll changes which slice is visible. Re-render only the new
            // viewport instead of laying out the whole graph.
            cx.notify();
            let _ = this;
        }))
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_scroll()
        .child(layer)
        .into_any_element();
    let background = div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .child(dot_grid(grid))
        .child(viewport);
    #[cfg(debug_assertions)]
    let background = background.children(crate::workspace_ui_smoke::probe("graph-background"));
    background.into_any_element()
}

fn graph_viewport(scroll: &ScrollHandle, window: &Window) -> crate::activity::Viewport {
    let offset = scroll.offset();
    let size = scroll.bounds().size;
    let width = if f32::from(size.width) > 0.0 {
        f32::from(size.width)
    } else {
        f32::from(window.viewport_size().width).max(800.0)
    };
    let height = if f32::from(size.height) > 0.0 {
        f32::from(size.height)
    } else {
        f32::from(window.viewport_size().height).max(600.0)
    };
    crate::activity::Viewport::new(
        f32::from(offset.x),
        f32::from(offset.y),
        width,
        height,
        240.0,
    )
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

#[allow(clippy::too_many_arguments)]
fn node_card(
    node: &AgentNode,
    meta: Option<&crate::activity::NodeA11yMeta>,
    selected: bool,
    expanded: bool,
    expandable: bool,
    reduce_motion: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    let is_tool = node.kind == AgentKind::Tool;
    let active = node.status == AgentStatus::Running;
    let selected_id = node.id.clone();
    let node_label = crate::activity::node_label(node);
    let node_description = crate::activity::node_description(node);
    let accessibility_id = format!("agent-node:{}", node.id);
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
        .role(Role::TreeItem)
        .accessibility_id(accessibility_id)
        .aria_label(node_label)
        .aria_description(node_description)
        .aria_selected(selected)
        .when(selected, |this| this.aria_active_descendant())
        .absolute()
        .left(px(node.x))
        .top(px(node.y))
        .w(px(node.width))
        .h(px(node.height))
        .rounded(px(SurfaceGeometry::GRAPH_NODE.radius))
        .border(px(SurfaceGeometry::GRAPH_NODE.border))
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
        .on_click(cx.listener(move |this, _, window, cx| {
            this.activity_focus.focus(window, cx);
            this.selected_node = if this.selected_node.as_deref() == Some(&selected_id) {
                None
            } else {
                Some(selected_id.clone())
            };
            this.selected_span = None;
            if !is_tool && !this.activity.agents_expanded.remove(&selected_id) {
                this.activity.agents_expanded.insert(selected_id.clone());
            }
            cx.notify();
        }));
    if let Some(meta) = meta {
        card = card
            .aria_level(meta.level + 1)
            .aria_position_in_set(meta.position)
            .aria_size_of_set(meta.siblings);
        if !is_tool && (meta.has_children || expandable) {
            card = card.aria_expanded(expanded);
        }
    }
    let mut card = card
        .child(
            SurfaceGeometry::GRAPH_NODE
                .round_child(div(), SurfaceEdges::Top)
                .h(crate::workspace_text::size(30., cx))
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
                        .text_size(crate::workspace_text::size(12., cx))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .truncate()
                        .child(node.title.clone()),
                )
                .child(
                    div()
                        .h(px(15.))
                        .px(px(4.))
                        .rounded(px(dunlin_ui::radii::BADGE))
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
                        .h(crate::workspace_text::size(34., cx))
                        .overflow_hidden()
                        .text_size(crate::workspace_text::size(11., cx))
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
            SurfaceGeometry::GRAPH_NODE
                .bottom_indicator(div())
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
                            Animation::new(if reduce_motion {
                                Duration::ZERO
                            } else {
                                Duration::from_millis(1800)
                            })
                            .repeat_synced()
                            .with_easing(pulsating_between(0.4, 1.0)),
                            |this, t| this.opacity(t),
                        ),
                ),
        );
    }

    if reduce_motion {
        return card.into_any_element();
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
        .rounded(px(dunlin_ui::radii::BADGE))
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

fn node_detail(
    node: &AgentNode,
    reduce_motion: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (status_color, status_soft) = agent_status_palette(p, node.status);
    let detail = div()
        .absolute()
        .right(px(12.))
        .bottom(px(10.))
        .w(px(338.))
        .p(px(10.))
        .rounded(px(dunlin_ui::radii::DRAG_PREVIEW))
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
                        .rounded(px(dunlin_ui::radii::BADGE))
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
                    icon_button("close-node-detail", IconName::X, "Close agent details").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.selected_node = None;
                            cx.notify();
                        }),
                    ),
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
                    dunlin_ui::action_button("open-node-session")
                        .icon(IconName::ArrowUpRight)
                        .label("Transcript")
                        .accessibility_label("Open the live transcript for this node")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_view(MainView::Chat, cx);
                            this.set_notice("Opened the live transcript", cx);
                        })),
                ),
        );

    if reduce_motion {
        return detail.into_any_element();
    }
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
        .rounded(px(dunlin_ui::radii::BADGE))
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
    fn fit_graph(&mut self, cx: &mut Context<Self>) {
        let all =
            crate::projection::nodes(&self.harness.transcript, self.project(), self.session());
        let nodes = crate::activity::graph_nodes(all, &self.activity.agents_expanded);
        let nodes = crate::activity::filter_graph_nodes(nodes, &self.activity_query);
        let (width, height) = crate::activity::graph_extent(&nodes);
        let viewport = self.activity.graph_scroll.bounds().size;
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let zoom = if viewport_width > 48. && viewport_height > 48. {
            crate::activity::fit_zoom(
                width,
                height,
                viewport_width - 32.,
                viewport_height - 32.,
                0.5,
                1.8,
            )
        } else {
            1.0
        };
        self.activity.graph_zoom = zoom;
        self.activity.graph_centered = true;
        let content_width = width * zoom;
        let content_height = height * zoom;
        self.activity.graph_scroll.set_offset(point(
            px(-(content_width - viewport_width).max(0.0) / 2.),
            px(-(content_height - viewport_height).max(0.0) / 2.),
        ));
        cx.notify();
    }

    fn export_graph_svg(&mut self, cx: &mut Context<Self>) {
        let all =
            crate::projection::nodes(&self.harness.transcript, self.project(), self.session());
        let nodes = crate::activity::graph_nodes(all, &self.activity.agents_expanded);
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
