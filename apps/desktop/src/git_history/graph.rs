//! Virtualized, identity-colored DAG. Column compaction never chooses a color.
use super::*;
use dunlin_ui::component::{menu::ContextMenuExt, tooltip::Tooltip};

pub(super) const ROW_HEIGHT: f32 = 32.;

pub(super) fn hash(value: &str) -> usize {
    // Fixed FNV, unlike a process-randomized hash: refreshes and older pages keep
    // the same color. The first-parent chain carries this identity through rows.
    value.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    }) as usize
}

pub(super) fn identity(commit: &Commit) -> usize {
    let reference = commit
        .refs
        .split(", ")
        .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r))
        .filter(|r| !r.is_empty() && *r != "HEAD")
        .min();
    hash(reference.unwrap_or(&commit.oid))
}

pub(super) fn distinct(seed: usize, occupied: impl Iterator<Item = usize>) -> usize {
    let mut used = [false; 6];
    for identity in occupied {
        used[identity % used.len()] = true;
    }
    (0..used.len())
        .map(|offset| seed.wrapping_add(offset))
        .find(|identity| !used[identity % used.len()])
        .unwrap_or(seed)
}

fn color(identity: usize, p: &Palette) -> Hsla {
    [p.amber, p.blue, p.violet, p.teal, p.green, p.red][identity % 6]
}

fn cell(
    row: LaneRow,
    width: f32,
    highlighted: Option<usize>,
    selected: bool,
    merge: bool,
    p: Palette,
) -> AnyElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let origin = bounds.origin;
            let position =
                |column: usize, y: f32| origin + point(px(14. + column as f32 * 14.), px(y));
            let stroke = |from: usize,
                          to: usize,
                          top: f32,
                          bottom: f32,
                          identity: usize,
                          window: &mut Window| {
                let emphasis = highlighted == Some(identity);
                let mut path = PathBuilder::stroke(px(if emphasis { 2.4 } else { 1.5 }));
                path.move_to(position(from, top));
                if from == to {
                    path.line_to(position(to, bottom));
                } else {
                    let middle = (top + bottom) * 0.5;
                    path.cubic_bezier_to(
                        position(to, bottom),
                        position(from, middle),
                        position(to, middle),
                    );
                }
                if let Ok(path) = path.build() {
                    let alpha = if highlighted.is_some() && !emphasis {
                        0.36
                    } else {
                        0.95
                    };
                    window.paint_path(path, color(identity, &p).opacity(alpha));
                }
            };
            for lane in 0..row.incoming {
                stroke(
                    lane,
                    lane,
                    0.,
                    ROW_HEIGHT * 0.5,
                    row.incoming_colors[lane],
                    window,
                );
            }
            for ((from, to), identity) in row.edges.iter().zip(&row.edge_colors) {
                // Non-commit lanes bend over the entire lower half, with a
                // horizontal tangent at both ends rather than square elbows.
                stroke(*from, *to, ROW_HEIGHT * 0.5, ROW_HEIGHT, *identity, window);
            }
            let center = position(row.lane, ROW_HEIGHT * 0.5);
            let node_color = color(row.color, &p);
            let mut dot = |radius: f32, fill: Hsla| {
                window.paint_quad(quad(
                    Bounds::new(
                        center - point(px(radius), px(radius)),
                        size(px(radius * 2.), px(radius * 2.)),
                    ),
                    Corners::all(px(radius)),
                    fill,
                    Edges::default(),
                    fill,
                    BorderStyle::default(),
                ));
            };
            if selected || highlighted == Some(row.color) {
                dot(7., node_color.opacity(0.18));
            }
            dot(if merge || selected { 4.7 } else { 3.4 }, node_color);
            if merge || selected {
                dot(2.6, p.panel);
            }
        },
    )
    .w(px(width))
    .h(px(ROW_HEIGHT))
    .flex_shrink_0()
    .into_any_element()
}

pub(super) fn width(view: &View) -> f32 {
    (view.lanes.iter().map(|r| r.width).max().unwrap_or(1) as f32 * 14. + 18.).max(44.)
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let p = palette(cx);
    let width = width(view);
    uniform_list(
        "git-history-list",
        view.rows.len(),
        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
            let Some(view) = &this.git_history.view else {
                return Vec::new();
            };
            let reduce_motion = this.reduce_motion || cx.reduce_motion();
            let highlighted = view
                .hovered
                .and_then(|i| view.lanes.get(i))
                .map(|r| r.color);
            range
                .filter_map(|i| Some((i, view.rows.get(i)?, view.lanes.get(i)?)))
                .map(|(i, commit, lane)| {
                    let selected = view.selected == Some(i);
                    let date = chrono::DateTime::from_timestamp(commit.timestamp, 0)
                        .map(|d| {
                            d.with_timezone(&chrono::Local)
                                .format("%Y/%m/%d %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    let tooltip = format!(
                        "{}\n{}\n{} · {}{}",
                        commit.subject,
                        commit.oid,
                        commit.author,
                        date,
                        if commit.parents.len() > 1 {
                            " · merge commit"
                        } else {
                            ""
                        }
                    );
                    let target = commit.oid.clone();
                    let id = view.id;
                    let row =
                        div()
                            .id(("history-commit", i))
                            .h(px(ROW_HEIGHT))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .min_w(px(0.))
                            .w_full()
                            .gap(px(6.))
                            .pr(px(10.))
                            .cursor_pointer()
                            .when(selected, |s| s.bg(p.selection))
                            .when(!selected, |s| s.hover(move |s| s.bg(p.hover)))
                            .aria_label(format!(
                                "Commit {}: {}{}",
                                commit.oid,
                                commit.subject,
                                if selected { ", selected" } else { "" }
                            ))
                            .tooltip(move |w, cx| Tooltip::new(tooltip.clone()).build(w, cx))
                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                if let Some(view) = &mut this.git_history.view {
                                    let next = if *hovered {
                                        Some(i)
                                    } else if view.hovered == Some(i) {
                                        None
                                    } else {
                                        view.hovered
                                    };
                                    if view.hovered != next {
                                        view.hovered = next;
                                        cx.notify();
                                    }
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(view) = &this.git_history.view {
                                    view.focus.focus(window, cx);
                                }
                                this.select_history_commit(i, 0, cx);
                            }))
                            .child(div().w(px(width)).flex_shrink_0().overflow_hidden().child(
                                cell(
                                    lane.clone(),
                                    width,
                                    highlighted,
                                    selected,
                                    commit.parents.len() > 1,
                                    p,
                                ),
                            ))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_1()
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(commit.subject.clone()),
                            )
                            .when(!commit.refs.is_empty(), |row| {
                                row.child(
                                    dunlin_ui::button(format!("history-refs-{i}"))
                                        .ghost()
                                        .small()
                                        .rounded(px(dunlin_ui::radii::CODE))
                                        .max_w(relative(0.22))
                                        .text_size(px(10.))
                                        .text_color(color(lane.color, &p))
                                        .label(commit.refs.clone())
                                        .tooltip(format!(
                                            "{} · show history ending at this commit",
                                            commit.refs
                                        ))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            if this
                                                .git_history
                                                .view
                                                .as_ref()
                                                .is_some_and(|v| v.id == id)
                                            {
                                                if let Some(view) = &this.git_history.view {
                                                    view.revision.update(cx, |s, cx| {
                                                        s.set_value(target.clone(), window, cx)
                                                    });
                                                }
                                                this.load_git_history(cx);
                                            }
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .w(px(112.))
                                    .min_w(px(70.))
                                    .text_size(px(10.))
                                    .text_color(p.muted_fg)
                                    .font_family(dunlin_ui::theme::code_font())
                                    .truncate()
                                    .child(date),
                            );
                    let menu = actions::menu(cx.entity().downgrade(), id, commit.clone());
                    let row = row.context_menu(menu);
                    if selected && !reduce_motion {
                        row.with_animation(
                            format!("history-selection-{}-{}", view.id, view.detail_request),
                            Animation::new(std::time::Duration::from_millis(150)),
                            |row, t| row.opacity(0.75 + t * 0.25),
                        )
                        .into_any_element()
                    } else {
                        row.into_any_element()
                    }
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&view.scroll)
    .flex_1()
    .min_h(px(0.))
    .min_w(px(0.))
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::super::{layout, scroll_anchor};
    use super::{Commit, hash, identity};

    fn commit(oid: &str, parents: &[&str], refs: &str) -> Commit {
        Commit {
            oid: oid.into(),
            parents: parents.iter().map(|s| (*s).into()).collect(),
            author: String::new(),
            timestamp: 0,
            subject: String::new(),
            refs: refs.into(),
        }
    }

    #[test]
    fn undecorated_tips_use_their_oid_not_an_empty_reference() {
        assert_eq!(identity(&commit("a", &[], "")), hash("a"));
        assert_eq!(identity(&commit("a", &[], "HEAD")), hash("a"));
        assert_ne!(
            identity(&commit("a", &[], "")),
            identity(&commit("b", &[], ""))
        );
    }

    #[test]
    fn reference_order_and_head_decoration_do_not_change_identity() {
        assert_eq!(
            identity(&commit("a", &[], "HEAD -> main, origin/main")),
            identity(&commit("b", &[], "origin/main, main"))
        );
    }

    #[test]
    fn lane_compaction_keeps_the_path_color() {
        let rows = layout(&[
            commit("a", &["a0"], "main"),
            commit("b", &["b0"], "topic"),
            commit("a0", &[], ""),
            commit("b0", &[], ""),
        ]);
        assert_eq!((rows[1].lane, rows[3].lane), (1, 0));
        assert_eq!(rows[1].color, rows[3].color);
        assert_ne!(rows[0].color, rows[1].color);
    }

    #[test]
    fn extending_the_loaded_history_does_not_recolor_existing_rows() {
        let commits = [
            commit("merge", &["a", "b"], "main"),
            commit("a", &["root"], ""),
            commit("b", &["root"], "topic"),
            commit("root", &[], ""),
        ];
        let partial = layout(&commits[..2]);
        let full = layout(&commits);
        assert_eq!(partial, full[..2]);
        assert_eq!(full[0].edges.len(), 2);
    }

    #[test]
    fn fork_colors_are_distinct_even_when_hashes_share_a_palette_slot() {
        let rows = layout(&[
            commit("merge", &["first", "main"], "main"),
            commit("first", &[], ""),
            commit("main", &[], ""),
        ]);
        assert_eq!(rows[0].color, hash("main"));
        assert_ne!(rows[0].edge_colors[0] % 6, rows[0].edge_colors[1] % 6);
    }

    #[test]
    fn every_edge_meets_the_same_color_in_the_next_virtual_row() {
        let rows = layout(&[
            commit("m", &["a", "b", "c"], "main"),
            commit("a", &["root"], ""),
            commit("b", &["root"], ""),
            commit("c", &["root"], ""),
            commit("root", &[], ""),
        ]);
        for pair in rows.windows(2) {
            for ((_, to), color) in pair[0].edges.iter().zip(&pair[0].edge_colors) {
                assert_eq!(*color, pair[1].incoming_colors[*to]);
            }
        }
    }

    #[test]
    fn refresh_anchors_the_visible_commit_and_fractional_offset() {
        let before = [
            commit("a", &[], ""),
            commit("b", &[], ""),
            commit("c", &[], ""),
        ];
        let after = [
            commit("new", &[], ""),
            before[0].clone(),
            before[1].clone(),
            before[2].clone(),
        ];
        assert_eq!(scroll_anchor(&before, &after, -35.), Some(-67.));
        assert_eq!(scroll_anchor(&before, &before, -35.), Some(-35.));
        assert_eq!(scroll_anchor(&before, &[], -35.), None);
        assert_eq!(scroll_anchor(&[], &after, 0.), None);
    }
}
