//! Generic knowledge cockpit: list, detail, relation graph and pending
//! proposals. The view is derived from file-first artifacts; it does not
//! rewrite files and does not require a particular plugin to be enabled.

use crate::app::DunlinApp;
use dunlin_artifacts::{Kind, Snapshot};
use dunlin_knowledge::{KnowledgeEdge, KnowledgeGraph, KnowledgeObject, parse_frontmatter};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Disableable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::{Icon, Selectable, Sizable};
use dunlin_ui::*;
use dunlin_workspace::preview::PreviewKind;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    List,
    Graph,
    Artifacts,
}

#[derive(Clone, Debug)]
pub(crate) struct Proposal {
    pub path: PathBuf,
    pub title: String,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ArtifactView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub mime: String,
    pub bytes: i64,
    pub sha256: String,
    pub created_at: i64,
    pub blob_path: Option<String>,
    pub remote: bool,
}

#[derive(Default)]
pub(crate) struct State {
    pub graph: KnowledgeGraph,
    pub selected: Option<String>,
    pub mode: Mode,
    pub filter: Option<String>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub proposals: Vec<Proposal>,
    pub show_proposals: bool,
    pub artifacts: Vec<ArtifactView>,
    pub artifact_selected: Option<String>,
    pub artifact_share: Option<(String, String, i64)>,
    pub artifact_preview: Option<(String, Arc<Vec<u8>>)>,
    pub artifact_error: Option<String>,
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    this.refresh_knowledge_if_needed(cx);
    let p = palette(cx);
    if this.projects.is_empty() {
        return crate::views::empty_activity(
            "No project selected",
            "Open a project to browse its intent, concepts, decisions and tasks.",
            IconName::BookOpen,
            cx,
        );
    }
    let project = this.project().clone();
    let total = this.knowledge.graph.objects.len();
    let pending = this.knowledge.proposals.len();
    let mode = this.knowledge.mode;
    let filter = this.knowledge.filter.clone();
    let toolbar = div()
        .h(px(42.))
        .flex_shrink_0()
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .child(
            Icon::new(IconName::BookOpen)
                .size(px(14.))
                .text_color(p.violet),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Knowledge"),
        )
        .child(
            div()
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(format!("{total} object(s) · {}", project.name)),
        )
        .child(div().flex_1())
        .child(
            dunlin_ui::button("knowledge-list")
                .ghost()
                .small()
                .label("List")
                .selected(mode == Mode::List)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.knowledge.mode = Mode::List;
                    cx.notify();
                })),
        )
        .child(
            dunlin_ui::button("knowledge-graph")
                .ghost()
                .small()
                .label("Graph")
                .selected(mode == Mode::Graph)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.knowledge.mode = Mode::Graph;
                    cx.notify();
                })),
        )
        .child(
            dunlin_ui::button("knowledge-artifacts")
                .ghost()
                .small()
                .label(format!("Artifacts · {}", this.knowledge.artifacts.len()))
                .selected(mode == Mode::Artifacts)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.knowledge.mode = Mode::Artifacts;
                    cx.notify();
                })),
        )
        .child(
            dunlin_ui::button("knowledge-proposals")
                .ghost()
                .small()
                .label(if pending == 0 {
                    "Proposals".to_string()
                } else {
                    format!("Proposals · {pending}")
                })
                .selected(this.knowledge.show_proposals)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.knowledge.show_proposals = !this.knowledge.show_proposals;
                    cx.notify();
                })),
        )
        .child(
            dunlin_ui::action_button("knowledge-refresh")
                .label("Refresh")
                .on_click(cx.listener(|this, _, _, cx| this.refresh_knowledge(cx))),
        );

    let body = if this.knowledge.show_proposals {
        proposals_view(this, cx)
    } else if this.knowledge.loading && this.knowledge.graph.objects.is_empty() {
        loading_view(cx)
    } else if let Some(error) = &this.knowledge.error {
        error_view(error, cx)
    } else if mode == Mode::Graph {
        graph_view(this, window, cx)
    } else if mode == Mode::Artifacts {
        artifacts_view(this, window, cx)
    } else {
        list_view(this, cx)
    };
    let filters = filter_bar(this, filter, &p, cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .bg(p.card)
        .child(toolbar)
        .child(filters)
        .child(body)
        .into_any_element()
}

fn filter_bar(
    this: &DunlinApp,
    filter: Option<String>,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for object in &this.knowledge.graph.objects {
        *counts.entry(kind_id_str(&object.kind)).or_default() += 1;
    }
    let mut bar = div()
        .h(px(32.))
        .flex_shrink_0()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(5.))
        .border_b_1()
        .border_color(p.border);
    bar = bar.child(filter_chip("All", None, filter.is_none(), p, cx));
    for (kind, count) in counts {
        bar = bar.child(filter_chip(
            &format!("{kind} · {count}"),
            Some(kind.to_string()),
            filter.as_deref() == Some(kind),
            p,
            cx,
        ));
    }
    bar.into_any_element()
}

fn filter_chip(
    label: &str,
    value: Option<String>,
    active: bool,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let id = format!("knowledge-filter:{}", value.as_deref().unwrap_or("all"));
    let mut button = dunlin_ui::button(id)
        .ghost()
        .small()
        .label(label.to_string())
        .selected(active)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.knowledge.filter = value.clone();
            cx.notify();
        }));
    if active {
        button = button.text_color(p.fg);
    }
    button.into_any_element()
}

fn loading_view(cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted_fg)
                .child("Indexing knowledge…"),
        )
        .into_any_element()
}

fn error_view(error: &str, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .flex_1()
        .p(px(20.))
        .child(
            div()
                .p(px(12.))
                .rounded(px(dunlin_ui::radii::PANEL))
                .bg(p.red_soft)
                .text_color(p.red)
                .text_size(px(11.))
                .child(error.to_string()),
        )
        .into_any_element()
}

fn list_view(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let objects: Vec<KnowledgeObject> = filtered_objects(this);
    let selected = this
        .knowledge
        .selected
        .clone()
        .filter(|id| objects.iter().any(|object| &object.id == id))
        .or_else(|| objects.first().map(|object| object.id.clone()));
    this.knowledge.selected = selected.clone();
    let mut list = div()
        .id("knowledge-list")
        .w(px(300.))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(p.border)
        .p(px(6.));
    if objects.is_empty() {
        list = list.child(
            div()
                .p(px(10.))
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("No knowledge objects match this filter."),
        );
    }
    for (index, object) in objects.iter().enumerate() {
        let active = selected.as_deref() == Some(object.id.as_str());
        let id = object.id.clone();
        list = list.child(
            action_row(("knowledge-row", index), format!("Select {}", object.title))
                .px(px(9.))
                .py(px(7.))
                .selected(active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.knowledge.selected = Some(id.clone());
                    cx.notify();
                }))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(kind_badge(&object.kind, &p))
                                .child(status_badge(&object.status, &p)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(if active {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .truncate()
                                .child(object.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(9.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(object.path.clone().unwrap_or_default()),
                        ),
                ),
        );
    }
    let detail = selected
        .as_deref()
        .and_then(|id| this.knowledge.graph.get(id))
        .map(|object| object_detail(object, &this.knowledge.graph, &p))
        .unwrap_or_else(|| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("Select an object")
                .into_any_element()
        });
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(list)
        .child(detail)
        .into_any_element()
}

fn filtered_objects(this: &DunlinApp) -> Vec<KnowledgeObject> {
    let mut objects: Vec<_> = this
        .knowledge
        .graph
        .objects
        .iter()
        .filter(|object| {
            this.knowledge
                .filter
                .as_deref()
                .is_none_or(|kind| object.kind == kind)
        })
        .cloned()
        .collect();
    objects.sort_by(|a, b| {
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
    objects
}

fn object_detail(object: &KnowledgeObject, graph: &KnowledgeGraph, p: &Palette) -> AnyElement {
    let mut detail = div()
        .id("knowledge-detail")
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_y_scroll()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(10.));
    detail = detail
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(kind_badge(&object.kind, p))
                .child(status_badge(&object.status, p))
                .child(
                    div()
                        .text_size(px(10.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(object.id.clone()),
                ),
        )
        .child(
            div()
                .text_size(px(19.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(object.title.clone()),
        )
        .child(
            div()
                .text_size(px(10.))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(object.path.clone().unwrap_or_default()),
        );
    if !object.fields.is_empty() {
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .p(px(10.))
            .rounded(px(dunlin_ui::radii::PANEL))
            .bg(p.panel)
            .border_1()
            .border_color(p.border);
        for (key, value) in &object.fields {
            fields = fields.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(110.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child(key.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.fg)
                            .child(render_value(value)),
                    ),
            );
        }
        detail = detail.child(fields);
    }
    if !object.body.trim().is_empty() {
        detail = detail.child(
            div()
                .p(px(10.))
                .rounded(px(dunlin_ui::radii::PANEL))
                .bg(p.panel)
                .text_size(px(11.5))
                .line_height(relative(1.5))
                .text_color(p.fg)
                .child(object.body.clone()),
        );
    }
    let relations: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.from_id == object.id || edge.to_id == object.id)
        .collect();
    if !relations.is_empty() {
        let mut relation_box = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(10.))
            .rounded(px(dunlin_ui::radii::PANEL))
            .bg(p.panel)
            .border_1()
            .border_color(p.border)
            .child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_fg)
                    .child("Relations"),
            );
        for edge in relations {
            let outgoing = edge.from_id == object.id;
            let other = if outgoing {
                edge.to_id.clone()
            } else {
                edge.from_id.clone()
            };
            let other_title = graph
                .get(&other)
                .map(|object| object.title.as_str())
                .unwrap_or(other.as_str());
            relation_box = relation_box.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(10.5))
                    .child(
                        div()
                            .text_color(if outgoing { p.blue } else { p.violet })
                            .child(if outgoing {
                                format!("{} →", edge.relation)
                            } else {
                                format!("← {}", edge.relation)
                            }),
                    )
                    .child(div().text_color(p.fg).child(other_title.to_string())),
            );
        }
        detail = detail.child(relation_box);
    }
    detail.into_any_element()
}

fn graph_view(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let objects = filtered_objects(this);
    if objects.is_empty() {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(p.muted_fg)
            .child("No knowledge objects match this filter.")
            .into_any_element();
    }
    let layout = graph_layout(&objects, &this.knowledge.graph);
    let selected = this.knowledge.selected.clone();
    let edges = layout.edges.clone();
    let p_for_canvas = p;
    let canvas_element = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let origin = bounds.origin;
            for edge in &edges {
                let (sx, sy, sw, sh) = edge.from_rect;
                let (tx, ty, _, th) = edge.to_rect;
                let start_x = sx + sw;
                let start_y = sy + sh / 2.0;
                let end_x = tx;
                let end_y = ty + th / 2.0;
                let mid_x = (start_x + end_x) / 2.0;
                line(
                    window,
                    origin,
                    start_x,
                    start_y,
                    mid_x - start_x,
                    1.0,
                    p_for_canvas.border,
                );
                line(
                    window,
                    origin,
                    mid_x,
                    start_y.min(end_y),
                    1.0,
                    (end_y - start_y).abs().max(1.0),
                    p_for_canvas.border,
                );
                line(
                    window,
                    origin,
                    mid_x,
                    end_y,
                    end_x - mid_x,
                    1.0,
                    p_for_canvas.border,
                );
            }
        },
    );
    let mut graph = div()
        .id("knowledge-graph")
        .relative()
        .w(px(layout.width))
        .h(px(layout.height))
        .child(canvas_element);
    for (index, node) in layout.nodes.iter().enumerate() {
        let active = selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        graph = graph.child(
            div()
                .id(("knowledge-node", index))
                .absolute()
                .left(px(node.x))
                .top(px(node.y))
                .w(px(node.w))
                .h(px(node.h))
                .rounded(px(dunlin_ui::radii::CONTROL))
                .border_1()
                .border_color(if active { p.blue } else { p.border })
                .bg(if active { p.hover } else { p.panel })
                .px(px(9.))
                .py(px(6.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.knowledge.selected = Some(id.clone());
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(kind_badge(&node.kind, &p))
                        .child(
                            div()
                                .text_size(px(8.5))
                                .text_color(p.muted_fg)
                                .child(node.status.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(node.title.clone()),
                ),
        );
    }
    let scroll = div()
        .id("knowledge-graph-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .child(graph);
    let detail = selected
        .as_deref()
        .and_then(|id| this.knowledge.graph.get(id))
        .map(|object| {
            let mut text = format!("{} · {} · {}", object.id, object.status, object.title);
            let related: Vec<_> = this
                .knowledge
                .graph
                .edges
                .iter()
                .filter(|edge| edge.from_id == object.id || edge.to_id == object.id)
                .map(|edge| {
                    if edge.from_id == object.id {
                        format!("{} → {}", edge.relation, edge.to_id)
                    } else {
                        format!("{} ← {}", edge.relation, edge.from_id)
                    }
                })
                .collect();
            if !related.is_empty() {
                text.push_str(" · ");
                text.push_str(&related.join(" · "));
            }
            div()
                .h(px(34.))
                .flex_shrink_0()
                .px(px(14.))
                .flex()
                .items_center()
                .border_t_1()
                .border_color(p.border)
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .truncate()
                .child(text)
                .into_any_element()
        });
    let mut root = div().flex().flex_col().flex_1().min_h(px(0.)).child(scroll);
    if let Some(detail) = detail {
        root = root.child(detail);
    }
    root.into_any_element()
}

struct GraphNode {
    id: String,
    kind: String,
    status: String,
    title: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone)]
struct GraphEdge {
    from_rect: (f32, f32, f32, f32),
    to_rect: (f32, f32, f32, f32),
}

struct GraphLayout {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    width: f32,
    height: f32,
}

fn graph_layout(objects: &[KnowledgeObject], graph: &KnowledgeGraph) -> GraphLayout {
    let mut columns: BTreeMap<u8, Vec<&KnowledgeObject>> = BTreeMap::new();
    for object in objects {
        columns
            .entry(kind_order(&object.kind))
            .or_default()
            .push(object);
    }
    let mut nodes = Vec::new();
    let mut positions = BTreeMap::new();
    let mut max_rows = 0usize;
    let mut col_index = 0u8;
    for (_, mut column) in columns {
        column.sort_by(|a, b| a.title.cmp(&b.title));
        max_rows = max_rows.max(column.len());
        for (row, object) in column.into_iter().enumerate() {
            let x = 40.0 + f32::from(col_index) * 240.0;
            let y = 36.0 + row as f32 * 76.0;
            let w = 190.0;
            let h = 52.0;
            positions.insert(object.id.clone(), (x, y, w, h));
            nodes.push(GraphNode {
                id: object.id.clone(),
                kind: object.kind.clone(),
                status: object.status.clone(),
                title: object.title.clone(),
                x,
                y,
                w,
                h,
            });
        }
        col_index = col_index.saturating_add(1);
    }
    let edges = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let from = positions.get(&edge.from_id)?;
            let to = positions.get(&edge.to_id)?;
            Some(GraphEdge {
                from_rect: *from,
                to_rect: *to,
            })
        })
        .collect();
    GraphLayout {
        nodes,
        edges,
        width: 40.0 + f32::from(col_index) * 240.0 + 80.0,
        height: 36.0 + max_rows as f32 * 76.0 + 60.0,
    }
}

fn artifacts_view(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let artifacts = this.knowledge.artifacts.clone();
    let selected = this
        .knowledge
        .artifact_selected
        .clone()
        .filter(|id| artifacts.iter().any(|artifact| &artifact.id == id))
        .or_else(|| artifacts.first().map(|artifact| artifact.id.clone()));
    this.knowledge.artifact_selected = selected.clone();
    let mut left = div()
        .id("artifact-list")
        .w(px(300.))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(p.border)
        .p(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(6.))
                .py(px(6.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Evidence artifacts"),
                )
                .child(
                    dunlin_ui::action_button("artifact-save-clipboard")
                        .label("Save clipboard")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_clipboard_artifact(cx);
                        })),
                ),
        );
    if artifacts.is_empty() {
        left = left.child(
            div()
                .p(px(10.))
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("No artifacts yet. Copy a diff or test output and use Save clipboard."),
        );
    }
    for (index, artifact) in artifacts.iter().enumerate() {
        let active = selected.as_deref() == Some(artifact.id.as_str());
        let id = artifact.id.clone();
        left = left.child(
            action_row(
                ("artifact-row", index),
                format!("Select artifact {}", artifact.title),
            )
            .px(px(9.))
            .py(px(7.))
            .selected(active)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.knowledge.artifact_selected = Some(id.clone());
                this.knowledge.artifact_error = None;
                cx.notify();
            }))
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.violet)
                                    .child(artifact.kind.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .text_color(p.muted_fg)
                                    .child(format!("{} B", artifact.bytes)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .truncate()
                            .child(artifact.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .truncate()
                            .child(artifact.mime.clone()),
                    ),
            ),
        );
    }
    let detail = selected
        .as_deref()
        .and_then(|id| artifacts.iter().find(|artifact| artifact.id == id))
        .map(|artifact| artifact_detail(this, artifact, cx))
        .unwrap_or_else(|| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("Select an artifact")
                .into_any_element()
        });
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(left)
        .child(detail)
        .into_any_element()
}

fn artifact_detail(
    this: &mut DunlinApp,
    artifact: &ArtifactView,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let id = artifact.id.clone();
    let share_id = id.clone();
    let preview_id = id.clone();
    let evidence_id = id.clone();
    let revoke_id = id.clone();
    let delete_id = id.clone();
    let remote = artifact.remote;
    let mut detail = div()
        .id("artifact-detail")
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_y_scroll()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(
                    div()
                        .h(px(16.))
                        .px(px(5.))
                        .rounded(px(dunlin_ui::radii::BADGE))
                        .bg(p.violet.opacity(0.12))
                        .text_size(px(9.))
                        .text_color(p.violet)
                        .child(artifact.kind.clone()),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{} · {} B · sha256 {}… · {}",
                            artifact.mime,
                            artifact.bytes,
                            &artifact.sha256[..artifact.sha256.len().min(12)],
                            chrono::DateTime::from_timestamp(artifact.created_at, 0)
                                .map(|date| date
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string())
                                .unwrap_or_else(|| artifact.created_at.to_string())
                        )),
                ),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(artifact.title.clone()),
        );
    if let Some(error) = &this.knowledge.artifact_error {
        detail = detail.child(
            div()
                .p(px(9.))
                .rounded(px(dunlin_ui::radii::CONTROL))
                .bg(p.red_soft)
                .text_color(p.red)
                .text_size(px(10.5))
                .child(error.clone()),
        );
    }
    if let Some((_token, url, expires_at)) = &this.knowledge.artifact_share {
        let url = url.clone();
        let expires = chrono::DateTime::from_timestamp(*expires_at, 0)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| expires_at.to_string());
        detail = detail.child(
            div()
                .p(px(9.))
                .rounded(px(dunlin_ui::radii::CONTROL))
                .bg(p.green_soft)
                .text_color(p.green)
                .text_size(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .child(format!("Share link: {url} · expires {expires}")),
                )
                .child(
                    dunlin_ui::action_button("artifact-copy-share")
                        .label("Copy link")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            cx.notify();
                        })),
                ),
        );
    }
    let actions = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            dunlin_ui::action_button("artifact-share")
                .label(if remote {
                    "Share 7 days"
                } else {
                    "Share (relay only)"
                })
                .disabled(!remote)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.share_artifact(&share_id, cx);
                })),
        )
        .child(
            dunlin_ui::action_button("artifact-revoke")
                .label("Revoke")
                .disabled(!remote)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.revoke_artifact(&revoke_id, cx);
                })),
        )
        .child(
            dunlin_ui::action_button("artifact-load-preview")
                .label("Load preview")
                .disabled(!remote)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.load_artifact_preview(&preview_id, cx);
                })),
        )
        .child(
            dunlin_ui::action_button("artifact-use-evidence")
                .label("Use as evidence")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.link_artifact_as_evidence(&evidence_id, cx);
                })),
        )
        .child(
            dunlin_ui::action_button("artifact-delete")
                .danger()
                .outline()
                .label("Delete")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.delete_artifact(&delete_id, cx);
                })),
        );
    detail = detail.child(actions);
    detail = detail.child(artifact_preview(
        artifact,
        this.knowledge.artifact_preview.as_ref(),
        &p,
    ));
    detail.into_any_element()
}

fn artifact_preview(
    artifact: &ArtifactView,
    loaded: Option<&(String, Arc<Vec<u8>>)>,
    p: &Palette,
) -> AnyElement {
    let bytes = if let Some((_, bytes)) = loaded.filter(|(id, _)| id == &artifact.id) {
        bytes.clone()
    } else if let Some(path) = &artifact.blob_path {
        match std::fs::read(path) {
            Ok(bytes) => Arc::new(bytes),
            Err(error) => {
                return div()
                    .p(px(10.))
                    .rounded(px(dunlin_ui::radii::PANEL))
                    .bg(p.red_soft)
                    .text_color(p.red)
                    .text_size(px(10.5))
                    .child(format!("Could not read artifact: {error}"))
                    .into_any_element();
            }
        }
    } else {
        return div()
            .p(px(10.))
            .rounded(px(dunlin_ui::radii::PANEL))
            .bg(p.panel)
            .text_size(px(10.5))
            .text_color(p.muted_fg)
            .child("Remote artifact. Use Load preview to fetch the bytes from the relay.")
            .into_any_element();
    };
    let kind = match artifact.mime.as_str() {
        "image/png" => PreviewKind::Png,
        "image/jpeg" => PreviewKind::Jpeg,
        "image/svg+xml" => PreviewKind::Svg,
        "application/pdf" => PreviewKind::Pdf,
        _ => PreviewKind::Text,
    };
    if kind != PreviewKind::Text
        && let Ok(preview) = crate::file_preview::decode(bytes.clone(), kind, 1)
    {
        let image = preview.image.clone();
        return div()
            .flex_1()
            .min_h(px(260.))
            .rounded(px(dunlin_ui::radii::PANEL))
            .bg(p.panel)
            .overflow_hidden()
            .child(canvas(
                |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let viewport = Bounds::new(
                        bounds.origin + point(px(12.), px(12.)),
                        size(
                            (bounds.size.width - px(24.)).max(px(0.)),
                            (bounds.size.height - px(24.)).max(px(0.)),
                        ),
                    );
                    if viewport.size.width > px(0.) && viewport.size.height > px(0.) {
                        let image_size = image.size(0);
                        let scale = (f32::from(viewport.size.width) / (image_size.width.0 as f32))
                            .min(f32::from(viewport.size.height) / (image_size.height.0 as f32));
                        let fitted_size = size(
                            px((image_size.width.0 as f32) * scale),
                            px((image_size.height.0 as f32) * scale),
                        );
                        let fitted = Bounds::new(
                            viewport.origin
                                + point(
                                    px((f32::from(viewport.size.width)
                                        - f32::from(fitted_size.width))
                                        / 2.),
                                    px((f32::from(viewport.size.height)
                                        - f32::from(fitted_size.height))
                                        / 2.),
                                ),
                            fitted_size,
                        );
                        let _ = window.paint_image(
                            fitted,
                            fitted,
                            Corners::default(),
                            image.clone(),
                            0,
                            false,
                        );
                    }
                },
            ))
            .into_any_element();
    }
    let truncated = bytes.len() > 64 * 1024;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]);
    let mut preview = text.to_string();
    if truncated {
        preview.push_str(
            "
… [preview truncated]",
        );
    }
    div()
        .id("artifact-preview")
        .flex_1()
        .min_h(px(200.))
        .p(px(10.))
        .rounded(px(dunlin_ui::radii::PANEL))
        .bg(p.panel)
        .border_1()
        .border_color(p.border)
        .overflow_y_scroll()
        .text_size(px(10.5))
        .font_family(dunlin_ui::theme::code_font())
        .text_color(p.fg)
        .child(preview)
        .into_any_element()
}

impl DunlinApp {
    pub(crate) fn save_clipboard_artifact(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            self.set_notice("Clipboard does not contain text", cx);
            return;
        };
        let title = format!(
            "Clipboard · {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M")
        );
        self.save_artifact_bytes("text", &title, "text/plain", text.as_bytes(), cx);
    }

    pub(crate) fn save_file_artifact(&mut self, cx: &mut Context<Self>) {
        let Some(inspector) = self.inspector.clone() else {
            self.set_notice("Open a file before saving it as an artifact", cx);
            return;
        };
        let text = self.editor.read(cx).text().to_string();
        let mime = if inspector.name.ends_with(".md") || inspector.name.ends_with(".markdown") {
            "text/markdown"
        } else {
            "text/plain"
        };
        self.save_artifact_bytes("file", &inspector.name, mime, text.as_bytes(), cx);
    }

    fn save_artifact_bytes(
        &mut self,
        kind: &str,
        title: &str,
        mime: &str,
        bytes: &[u8],
        cx: &mut Context<Self>,
    ) {
        let project = self.project().clone();
        if project.is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(error) => {
                    self.knowledge.artifact_error = Some(error);
                    cx.notify();
                    return;
                }
            };
            match backend.upload_artifact(kind, title, mime, bytes) {
                Ok(_) => {
                    self.set_notice("Artifact uploaded to relay", cx);
                    self.refresh_knowledge(cx);
                }
                Err(error) => {
                    self.knowledge.artifact_error = Some(error);
                    cx.notify();
                }
            }
            return;
        }
        let Some(store) = self.store.as_ref() else {
            self.knowledge.artifact_error = Some("Storage is unavailable".into());
            cx.notify();
            return;
        };
        let sha256 = hex::encode(Sha256::digest(bytes));
        let Some(data_dir) = crate::app::data_dir() else {
            self.knowledge.artifact_error =
                Some("Application data directory is unavailable".into());
            cx.notify();
            return;
        };
        let directory = data_dir.join("artifacts").join(&project.id);
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.knowledge.artifact_error = Some(format!("Create artifact directory: {error}"));
            cx.notify();
            return;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let path = directory.join(&id);
        if let Err(error) = std::fs::write(&path, bytes) {
            self.knowledge.artifact_error = Some(format!("Write artifact: {error}"));
            cx.notify();
            return;
        }
        let record = dunlin_storage::ArtifactRecord {
            id,
            project_id: project.id.clone(),
            run_id: None,
            session_id: Some(self.session().id.clone()),
            intent_id: None,
            task_id: None,
            kind: kind.to_string(),
            title: title.to_string(),
            mime: mime.to_string(),
            bytes: bytes.len() as i64,
            sha256,
            blob_path: path.to_string_lossy().to_string(),
            created_ms: chrono::Local::now().timestamp_millis(),
            visibility: "local".into(),
            expires_ms: None,
            revoked_ms: None,
            payload: serde_json::json!({}),
        };
        match store.upsert_artifact(&record) {
            Ok(()) => {
                self.set_notice("Artifact saved", cx);
                self.refresh_knowledge(cx);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                self.knowledge.artifact_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn share_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.knowledge.artifact_error = Some(error);
                cx.notify();
                return;
            }
        };
        match backend.share_artifact(id, 7 * 24 * 60 * 60) {
            Ok(share) => {
                let url = match &backend {
                    crate::remote::WorkspaceBackend::Remote(remote) => {
                        format!("{}{}", remote.host.url.trim_end_matches('/'), share.url)
                    }
                    crate::remote::WorkspaceBackend::Local { .. } => share.url,
                };
                self.knowledge.artifact_share = Some((share.token, url, share.expires_at));
                self.knowledge.artifact_error = None;
                cx.notify();
            }
            Err(error) => {
                self.knowledge.artifact_error = Some(error);
                cx.notify();
            }
        }
    }

    fn revoke_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        match self
            .backend()
            .and_then(|backend| backend.revoke_artifact_shares(id))
        {
            Ok(()) => {
                self.knowledge.artifact_share = None;
                self.set_notice("Artifact share revoked", cx);
            }
            Err(error) => self.knowledge.artifact_error = Some(error),
        }
        cx.notify();
    }

    fn load_artifact_preview(&mut self, id: &str, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.knowledge.artifact_error = Some(error);
                cx.notify();
                return;
            }
        };
        let request_id = id.to_string();
        let state_id = request_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.download_artifact(&request_id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(bytes) => {
                        this.knowledge.artifact_preview = Some((state_id.clone(), Arc::new(bytes)));
                        this.knowledge.artifact_error = None;
                    }
                    Err(error) => this.knowledge.artifact_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn link_artifact_as_evidence(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(artifact) = self
            .knowledge
            .artifacts
            .iter()
            .find(|artifact| artifact.id == id)
            .cloned()
        else {
            return;
        };
        let Some(store) = self.store.as_ref() else {
            self.knowledge.artifact_error = Some("Storage is unavailable".into());
            cx.notify();
            return;
        };
        let claim_id = format!("CLM-{}", uuid::Uuid::now_v7().simple());
        let evidence_id = format!("EV-{}", uuid::Uuid::now_v7().simple());
        let claim = dunlin_storage::ClaimRecord {
            id: claim_id.clone(),
            project_id: self.project().id.clone(),
            intent_id: None,
            task_id: None,
            statement: format!("Artifact is evidence: {}", artifact.title),
            status: "unverified".into(),
            confidence: "low".into(),
            scope: "artifact".into(),
            last_verified_ms: None,
            stale_reason: None,
            payload: serde_json::json!({"artifact_id": artifact.id}),
        };
        let evidence = dunlin_storage::EvidenceRecord {
            id: evidence_id.clone(),
            project_id: self.project().id.clone(),
            kind: "artifact".into(),
            artifact_id: Some(artifact.id.clone()),
            run_id: None,
            source: "artifact".into(),
            revision: artifact.sha256.clone(),
            hash: artifact.sha256.clone(),
            observed_ms: artifact.created_at,
            freshness: "fresh".into(),
            summary: artifact.title.clone(),
            payload: serde_json::json!({"mime": artifact.mime}),
        };
        let result = (|| -> Result<(), String> {
            store
                .upsert_claim(&claim)
                .map_err(|error| error.to_string())?;
            store
                .upsert_evidence(&evidence)
                .map_err(|error| error.to_string())?;
            store
                .link_evidence(&evidence_id, &claim_id, "supports")
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => self.set_notice(
                format!("Artifact linked as evidence to claim {claim_id}"),
                cx,
            ),
            Err(error) => {
                self.knowledge.artifact_error = Some(error);
                cx.notify();
            }
        }
    }

    fn delete_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        let remote = self
            .knowledge
            .artifacts
            .iter()
            .find(|artifact| artifact.id == id)
            .is_some_and(|artifact| artifact.remote);
        if remote {
            match self
                .backend()
                .and_then(|backend| backend.delete_artifact(id))
            {
                Ok(()) => {
                    self.set_notice("Remote artifact deleted", cx);
                    self.refresh_knowledge(cx);
                }
                Err(error) => {
                    self.knowledge.artifact_error = Some(error);
                    cx.notify();
                }
            }
            return;
        }
        let Some(store) = self.store.as_ref() else {
            self.knowledge.artifact_error = Some("Storage is unavailable".into());
            cx.notify();
            return;
        };
        let record = self
            .knowledge
            .artifacts
            .iter()
            .find(|artifact| artifact.id == id)
            .cloned();
        if let Some(record) = record {
            if let Some(path) = record.blob_path {
                let _ = std::fs::remove_file(path);
            }
            if let Err(error) = store.delete_artifact(id) {
                self.knowledge.artifact_error = Some(error.to_string());
                cx.notify();
                return;
            }
            self.set_notice("Artifact deleted", cx);
            self.refresh_knowledge(cx);
        }
    }
}

fn proposals_view(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut panel = div()
        .id("knowledge-proposals")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(8.));
    panel = panel.child(
        div()
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child("Pending proposals"),
    );
    panel = panel.child(
        div()
            .text_size(px(10.5))
            .text_color(p.muted_fg)
            .child("Agent proposals never modify project files. Accept writes one reviewed file; Reject archives the proposal."),
    );
    if this.knowledge.proposals.is_empty() {
        panel = panel.child(
            div()
                .p(px(10.))
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("No pending proposals."),
        );
    }
    for (index, proposal) in this.knowledge.proposals.iter().enumerate() {
        let path = proposal.path.clone();
        let accept_path = path.clone();
        let reject_path = path.clone();
        panel = panel.child(
            div()
                .p(px(11.))
                .rounded(px(dunlin_ui::radii::PANEL))
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .child(proposal.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(9.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child(proposal.summary.clone()),
                        ),
                )
                .child(
                    dunlin_ui::action_button(format!("knowledge-accept:{index}"))
                        .label("Accept")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            match accept_proposal(this, &accept_path) {
                                Ok(message) => {
                                    this.set_notice(message, cx);
                                    this.refresh_knowledge(cx);
                                }
                                Err(error) => this
                                    .set_notice(format!("Could not accept proposal: {error}"), cx),
                            }
                            cx.notify();
                        })),
                )
                .child(
                    dunlin_ui::action_button(format!("knowledge-reject:{index}"))
                        .label("Reject")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            match reject_proposal(&reject_path) {
                                Ok(()) => {
                                    this.set_notice("Proposal rejected and archived", cx);
                                    this.refresh_knowledge(cx);
                                }
                                Err(error) => this
                                    .set_notice(format!("Could not reject proposal: {error}"), cx),
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    panel.into_any_element()
}

impl DunlinApp {
    pub(crate) fn refresh_knowledge_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.knowledge.loaded && !self.knowledge.loading {
            self.refresh_knowledge(cx);
        }
    }

    pub(crate) fn refresh_knowledge(&mut self, cx: &mut Context<Self>) {
        let project = self.project().clone();
        if !project.trusted {
            self.knowledge.error =
                Some("Trust the project before reading its knowledge files".into());
            self.knowledge.loaded = true;
            cx.notify();
            return;
        }
        self.knowledge.loading = true;
        self.knowledge.error = None;
        self.knowledge.loaded = true;
        let include_governance = crate::spec::governance_enabled(&self.plugins, &project.id);
        let local_artifacts = self
            .store
            .as_ref()
            .and_then(|store| store.artifacts(&project.id).ok())
            .unwrap_or_default()
            .into_iter()
            .map(artifact_view_local)
            .collect::<Vec<_>>();
        if project.is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(error) => {
                    self.knowledge.loading = false;
                    self.knowledge.error = Some(error);
                    cx.notify();
                    return;
                }
            };
            let project_for_task = project.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let snapshot = crate::spec::remote_snapshot(&project_for_task, &backend);
                        let artifacts = backend
                            .artifact_list()
                            .unwrap_or_default()
                            .into_iter()
                            .map(artifact_view_remote)
                            .collect::<Vec<_>>();
                        (snapshot, load_proposals(), artifacts)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.finish_knowledge_refresh(
                        result.0,
                        result.1,
                        result.2,
                        include_governance,
                        cx,
                    );
                });
            })
            .detach();
        } else {
            let path = project.path.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let snapshot = dunlin_artifacts::discover(Path::new(&path));
                        (snapshot, load_proposals(), local_artifacts)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let snapshot = match result.0 {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            this.knowledge.loading = false;
                            this.knowledge.error = Some(error);
                            this.knowledge.proposals = result.1;
                            this.knowledge.artifacts = result.2;
                            cx.notify();
                            return;
                        }
                    };
                    this.finish_knowledge_refresh(
                        snapshot,
                        result.1,
                        result.2,
                        include_governance,
                        cx,
                    );
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn finish_knowledge_refresh(
        &mut self,
        snapshot: Snapshot,
        proposals: Vec<Proposal>,
        artifacts: Vec<ArtifactView>,
        include_governance: bool,
        cx: &mut Context<Self>,
    ) {
        let graph = graph_from_snapshot(&snapshot, include_governance);
        if self
            .knowledge
            .selected
            .as_deref()
            .is_none_or(|id| graph.get(id).is_none())
        {
            self.knowledge.selected = graph.objects.first().map(|object| object.id.clone());
        }
        self.knowledge.graph = graph;
        self.knowledge.proposals = proposals;
        self.knowledge.artifacts = artifacts;
        if self
            .knowledge
            .artifact_selected
            .as_deref()
            .is_none_or(|id| {
                !self
                    .knowledge
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.id == id)
            })
        {
            self.knowledge.artifact_selected = self
                .knowledge
                .artifacts
                .first()
                .map(|artifact| artifact.id.clone());
        }
        self.knowledge.loading = false;
        self.knowledge.error = None;
        cx.notify();
    }
}

fn artifact_view_local(record: dunlin_storage::ArtifactRecord) -> ArtifactView {
    ArtifactView {
        id: record.id,
        kind: record.kind,
        title: record.title,
        mime: record.mime,
        bytes: record.bytes,
        sha256: record.sha256,
        created_at: record.created_ms,
        blob_path: Some(record.blob_path),
        remote: false,
    }
}

fn artifact_view_remote(record: crate::remote::RemoteArtifact) -> ArtifactView {
    ArtifactView {
        id: record.id,
        kind: record.kind,
        title: record.title,
        mime: record.mime,
        bytes: record.bytes,
        sha256: record.sha256,
        created_at: record.created_at,
        blob_path: None,
        remote: true,
    }
}

fn graph_from_snapshot(snapshot: &Snapshot, include_governance: bool) -> KnowledgeGraph {
    let mut graph = KnowledgeGraph::default();
    for artifact in &snapshot.artifacts {
        if !include_governance && matches!(artifact.kind, Kind::Concept | Kind::Decision) {
            continue;
        }
        let id = artifact
            .frontmatter
            .get("id")
            .cloned()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| artifact.id.clone());
        let mut fields = BTreeMap::new();
        for (key, value) in &artifact.frontmatter {
            if matches!(
                key.as_str(),
                "dunlin" | "kind" | "id" | "title" | "status" | "links"
            ) {
                continue;
            }
            fields.insert(key.clone(), Value::String(value.clone()));
        }
        let object = KnowledgeObject {
            id,
            project_id: snapshot.root.clone(),
            kind: kind_id(artifact.kind).to_string(),
            title: artifact.title.clone(),
            status: artifact.status.label().to_string(),
            path: Some(artifact.path.clone()),
            revision: artifact.revision.clone(),
            updated_ms: artifact.modified_ms,
            fields,
            body: artifact.body.clone(),
            plugin_id: None,
            schema_version: 1,
        };
        let source = format!("---\n{}\n---\n{}", artifact.frontmatter_raw, artifact.body);
        for link in parse_frontmatter(&source).links {
            if link.relation.trim().is_empty() || link.target.trim().is_empty() {
                continue;
            }
            graph.edges.push(KnowledgeEdge {
                from_id: object.id.clone(),
                to_id: link.target,
                relation: link.relation,
                metadata: BTreeMap::new(),
            });
        }
        graph.objects.push(object);
    }
    graph.objects.sort_by(|a, b| {
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
    graph.objects.dedup_by(|a, b| a.id == b.id);
    graph
}

fn kind_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Intent => "intent",
        Kind::Concept => "concept",
        Kind::Decision => "decision",
        Kind::AgentContract => "agent-contract",
        Kind::Constitution => "constitution",
        Kind::Spec => "spec",
        Kind::Plan => "plan",
        Kind::Tasks => "tasks",
        Kind::Checklist => "checklist",
        Kind::Other => "document",
    }
}

fn kind_id_str(kind: &str) -> &str {
    kind
}

pub(crate) fn kind_label(kind: &str) -> &str {
    match kind {
        "intent" => "Intent",
        "concept" => "Concept",
        "decision" => "Decision",
        "spec" => "Spec",
        "plan" => "Plan",
        "tasks" => "Tasks",
        "checklist" => "Checklist",
        "agent-contract" => "Contract",
        "constitution" => "Constitution",
        other => other,
    }
}

fn kind_order(kind: &str) -> u8 {
    match kind {
        "intent" => 0,
        "concept" => 1,
        "decision" => 2,
        "spec" => 3,
        "plan" => 4,
        "tasks" => 5,
        "checklist" => 6,
        "agent-contract" => 7,
        "constitution" => 8,
        _ => 9,
    }
}

fn kind_badge(kind: &str, p: &Palette) -> AnyElement {
    let color = match kind {
        "intent" => p.blue,
        "concept" => p.violet,
        "decision" => p.amber,
        "spec" => p.green,
        "plan" => p.teal,
        "tasks" => p.amber,
        _ => p.muted_fg,
    };
    div()
        .h(px(15.))
        .px(px(5.))
        .rounded(px(dunlin_ui::radii::BADGE))
        .bg(color.opacity(0.12))
        .text_size(px(8.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(kind_label(kind).to_string())
        .into_any_element()
}

fn status_badge(status: &str, p: &Palette) -> AnyElement {
    let color = match status {
        "accepted" | "done" => p.green,
        "rejected" | "blocked" => p.red,
        "in review" | "proposed" | "waiting" => p.amber,
        _ => p.muted_fg,
    };
    div()
        .h(px(15.))
        .px(px(5.))
        .rounded(px(dunlin_ui::radii::BADGE))
        .bg(color.opacity(0.10))
        .text_size(px(8.5))
        .text_color(color)
        .child(status.to_string())
        .into_any_element()
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => values
            .iter()
            .map(render_value)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
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
    window.paint_quad(quad(
        Bounds::new(
            origin + point(px(x), px(y)),
            size(px(width.max(0.8)), px(height.max(0.8))),
        ),
        Corners::all(px(0.4)),
        color,
        Edges::default(),
        color,
        BorderStyle::default(),
    ));
}

fn load_proposals() -> Vec<Proposal> {
    let Some(directory) = crate::app::data_dir().map(|data| data.join("proposals")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let value: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            let kind = value.get("kind").and_then(Value::as_str)?.to_string();
            let title = match kind.as_str() {
                "object" => value
                    .get("object")
                    .and_then(|object| object.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Object proposal")
                    .to_string(),
                "relation" => format!(
                    "{} → {}",
                    value.get("from").and_then(Value::as_str).unwrap_or("?"),
                    value.get("to").and_then(Value::as_str).unwrap_or("?")
                ),
                "evidence_link" => format!(
                    "{} → {}",
                    value.get("evidence").and_then(Value::as_str).unwrap_or("?"),
                    value.get("claim").and_then(Value::as_str).unwrap_or("?")
                ),
                _ => "Proposal".into(),
            };
            let summary = match kind.as_str() {
                "object" => format!(
                    "object · {} · {}",
                    value
                        .get("object")
                        .and_then(|object| object.get("kind"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                ),
                "relation" => format!(
                    "relation · {}",
                    value
                        .get("relation")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ),
                "evidence_link" => format!(
                    "evidence · {}",
                    value
                        .get("relation")
                        .and_then(Value::as_str)
                        .unwrap_or("supports")
                ),
                _ => path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
            };
            Some(Proposal {
                path,
                title,
                summary,
            })
        })
        .collect()
}

fn accept_proposal(this: &mut DunlinApp, path: &Path) -> Result<String, String> {
    let value: Value = serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("read proposal: {error}"))?,
    )
    .map_err(|error| format!("parse proposal: {error}"))?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("proposal has no kind")?;
    let project = this.project().clone();
    match kind {
        "object" => {
            let object: KnowledgeObject = serde_json::from_value(
                value
                    .get("object")
                    .cloned()
                    .ok_or("object proposal has no object")?,
            )
            .map_err(|error| format!("object: {error}"))?;
            let written = write_object(&project.path, &object)?;
            archive_proposal(path, "accepted")?;
            this.knowledge.selected = Some(object.id.clone());
            Ok(format!(
                "Accepted {} and wrote {}",
                object.id,
                written.display()
            ))
        }
        "relation" => {
            let from = value
                .get("from")
                .and_then(Value::as_str)
                .ok_or("relation proposal has no from")?;
            let to = value
                .get("to")
                .and_then(Value::as_str)
                .ok_or("relation proposal has no to")?;
            let relation = value
                .get("relation")
                .and_then(Value::as_str)
                .ok_or("relation proposal has no relation")?;
            let source = this
                .knowledge
                .graph
                .get(from)
                .and_then(|object| object.path.clone())
                .ok_or_else(|| format!("source object {from} has no file"))?;
            insert_link(&project.path, &source, relation, to)?;
            archive_proposal(path, "accepted")?;
            Ok(format!("Accepted {relation} {from} → {to}"))
        }
        "evidence_link" => {
            let evidence = value
                .get("evidence")
                .and_then(Value::as_str)
                .ok_or("evidence proposal has no evidence")?;
            let claim = value
                .get("claim")
                .and_then(Value::as_str)
                .ok_or("evidence proposal has no claim")?;
            let relation = value
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("supports");
            let store = this.store.as_ref().ok_or("storage is unavailable")?;
            store
                .link_evidence(evidence, claim, relation)
                .map_err(|error| error.to_string())?;
            archive_proposal(path, "accepted")?;
            Ok(format!("Linked evidence {evidence} {relation} {claim}"))
        }
        other => Err(format!("unknown proposal kind {other}")),
    }
}

fn reject_proposal(path: &Path) -> Result<(), String> {
    archive_proposal(path, "rejected")
}

fn archive_proposal(path: &Path, state: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err("proposal has no parent".into());
    };
    let directory = parent.join(state);
    std::fs::create_dir_all(&directory).map_err(|error| format!("archive directory: {error}"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("proposal has no name")?;
    let target = directory.join(name);
    std::fs::rename(path, &target).map_err(|error| format!("archive proposal: {error}"))?;
    Ok(())
}

fn write_object(project: &str, object: &KnowledgeObject) -> Result<PathBuf, String> {
    let directory = match object.kind.as_str() {
        "intent" => "intent",
        "concept" => "concepts",
        "decision" => "decisions",
        _ => ".dunlin/knowledge",
    };
    let directory = Path::new(project).join(directory);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let id = safe_component(&object.id);
    let slug = slugify(&object.title);
    let mut path = directory.join(format!("{id}-{slug}.md"));
    let mut suffix = 2;
    while path.exists() {
        path = directory.join(format!("{id}-{slug}-{suffix}.md"));
        suffix += 1;
        if suffix > 100 {
            return Err("too many files with the same id".into());
        }
    }
    let contents = render_object_markdown(object);
    atomic_write(&path, contents.as_bytes())?;
    Ok(path)
}

fn render_object_markdown(object: &KnowledgeObject) -> String {
    let mut out = String::from("---\n");
    out.push_str("dunlin: 1\n");
    out.push_str(&format!("kind: {}\n", yaml_scalar(&object.kind)));
    out.push_str(&format!("id: {}\n", yaml_scalar(&object.id)));
    out.push_str(&format!("title: {}\n", yaml_scalar(&object.title)));
    out.push_str(&format!("status: {}\n", yaml_scalar(&object.status)));
    for (key, value) in &object.fields {
        if matches!(
            key.as_str(),
            "dunlin" | "kind" | "id" | "title" | "status" | "links"
        ) {
            continue;
        }
        out.push_str(&format!("{key}: {}\n", yaml_value(value)));
    }
    out.push_str("---\n\n");
    out.push_str(object.body.trim_end());
    out.push('\n');
    out
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn yaml_value(value: &Value) -> String {
    match value {
        Value::String(text) => yaml_scalar(text),
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(yaml_value).collect::<Vec<_>>().join(", ")
        ),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}

fn safe_component(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            output.push(character);
        } else {
            output.push('-');
        }
        if output.len() >= 80 {
            break;
        }
    }
    let output = output.trim_matches(['-', '.']).to_string();
    if output.is_empty() {
        "object".into()
    } else {
        output
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "object".into()
    } else {
        slug
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("md.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(())
}

fn insert_link(project: &str, source: &str, relation: &str, target: &str) -> Result<(), String> {
    let path = Path::new(project).join(source);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if lines.first().map(String::as_str) != Some("---") {
        return Err("source file has no frontmatter".into());
    }
    let Some(close) = lines.iter().skip(1).position(|line| line.trim() == "---") else {
        return Err("source frontmatter is not closed".into());
    };
    let close = close + 1;
    let link_lines = vec![
        format!("  - relation: {relation}"),
        format!("    target: {target}"),
    ];
    if let Some(links_index) = lines[..close]
        .iter()
        .position(|line| line.trim() == "links:")
    {
        let mut insert = close;
        for (index, line) in lines.iter().enumerate().skip(links_index + 1) {
            if index >= close {
                break;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') && !line.trim().is_empty() {
                insert = index;
                break;
            }
        }
        lines.splice(insert..insert, link_lines);
    } else {
        let mut block = vec!["links:".to_string()];
        block.extend(link_lines);
        lines.splice(close..close, block);
    }
    let mut output = lines.join("\n");
    output.push('\n');
    atomic_write(&path, output.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object() -> KnowledgeObject {
        KnowledgeObject {
            id: "CON-001".into(),
            project_id: "p".into(),
            kind: "concept".into(),
            title: "Workspace".into(),
            status: "proposed".into(),
            path: Some("concepts/CON-001-workspace.md".into()),
            revision: "rev".into(),
            updated_ms: 0,
            fields: BTreeMap::from([("definition".into(), Value::String("A shared place".into()))]),
            body: "Body".into(),
            plugin_id: None,
            schema_version: 1,
        }
    }

    #[test]
    fn object_markdown_round_trips_through_the_parser() {
        let text = render_object_markdown(&object());
        assert!(text.starts_with("---\n"));
        assert!(text.contains("kind: \"concept\""));
        assert!(text.contains("title: \"Workspace\""));
        assert!(text.contains("definition: \"A shared place\""));
    }

    #[test]
    fn graph_from_snapshot_keeps_links_and_filters_governance() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("concepts")).unwrap();
        std::fs::write(
            root.path().join("concepts/CON-001-workspace.md"),
            "---\nid: CON-001\ntitle: Workspace\nstatus: accepted\nlinks:\n  - relation: realizes\n    target: INT-001\n---\n\nBody\n",
        )
        .unwrap();
        let snapshot = dunlin_artifacts::discover(root.path()).unwrap();
        let graph = graph_from_snapshot(&snapshot, true);
        assert_eq!(graph.objects.len(), 1);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to_id, "INT-001");
        let filtered = graph_from_snapshot(&snapshot, false);
        assert!(filtered.objects.is_empty());
    }

    #[test]
    fn safe_components_and_slugs_do_not_escape() {
        assert_eq!(safe_component("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("A B / C"), "a-b-c");
        assert_eq!(slugify("!!!"), "object");
    }
}
