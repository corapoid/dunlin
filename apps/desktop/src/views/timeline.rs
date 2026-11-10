//! Live, native file-edit timeline. Geometry is painted on the GPU; controls,
//! file lanes and diff details remain ordinary accessible GPUI elements.
use crate::{
    app::DunlinApp,
    timeline::{self, Edit, Snapshot},
};
use dunlin_ui::{assets::IconName, component::tooltip::Tooltip, *};
use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

const HEIGHT: f32 = 260.;
const LEFT: f32 = 52.;
const RIGHT: f32 = scale::SPACE_4;
const DETAIL_WIDTH: f32 = 360.;
const LANE_HEIGHT: f32 = 24.;
const TRANSITION: f32 = 0.36;

struct State {
    data: Rc<Snapshot>,
    item_count: usize,
    revision: u64,
    source: usize,
    entered: bool,
    arrivals: HashMap<String, Instant>,
    hovered: Option<String>,
    pinned: Option<String>,
    turn: Option<usize>,
    zoom: f32,
    pan: f32,
    following: bool,
    bounds: Option<Bounds<Pixels>>,
    drag: Option<(f32, f32)>,
    focus: Option<FocusHandle>,
    was_running: bool,
    clock_pending: bool,
    axis_end: u64,
    axis_turn: Option<usize>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            data: Rc::new(Snapshot::default()),
            item_count: 0,
            revision: 0,
            source: 0,
            entered: false,
            arrivals: HashMap::new(),
            hovered: None,
            pinned: None,
            turn: None,
            zoom: 1.,
            pan: 1.,
            following: true,
            bounds: None,
            drag: None,
            focus: None,
            was_running: false,
            clock_pending: false,
            axis_end: 0,
            axis_turn: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Range {
    start: f64,
    end: f64,
}
impl Range {
    fn new(start: u64, end: u64, zoom: f32, pan: f32) -> Self {
        let start = start as f64;
        let total = (end as f64 - start).max(1.);
        let span = total / f64::from(zoom.clamp(1., 32.));
        let start = start + (total - span) * f64::from(pan.clamp(0., 1.));
        Self {
            start,
            end: start + span,
        }
    }
    fn x(self, ms: u64, width: f32) -> f32 {
        ((ms as f64 - self.start) / (self.end - self.start)) as f32 * width
    }
    fn time(self, x: f32, width: f32) -> f64 {
        self.start + f64::from((x / width.max(1.)).clamp(0., 1.)) * (self.end - self.start)
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let key = format!(
        "file-timeline:{}:{}",
        this.session().id,
        this.workbench.panes.focused
    );
    let state = window.use_keyed_state(key.clone(), cx, |_, _| State::default());
    let reduce_motion = this.reduce_motion || cx.reduce_motion();
    let running = this.harness.running;
    let count = this.harness.transcript.len();
    let data = state.update(cx, |state, _| {
        // Do not clone/reparse a long transcript at animation-frame frequency.
        // Length alone is insufficient: ACP completes a tool in place.
        if !state.entered
            || state.item_count != count
            || state.was_running != running
            || state.revision != this.harness.file_revision
            || state.source != this.harness.transcript.as_ptr() as usize
        {
            let next = timeline::project(&this.harness.transcript);
            if state.entered {
                let existing: std::collections::HashSet<_> = state
                    .data
                    .edits
                    .iter()
                    .map(|edit| edit.id.as_str())
                    .collect();
                for edit in &next.edits {
                    if !existing.contains(edit.id.as_str()) {
                        state.arrivals.insert(edit.id.clone(), Instant::now());
                    }
                }
            }
            state
                .arrivals
                .retain(|_, time| time.elapsed().as_secs_f32() < TRANSITION);
            state.data = Rc::new(next);
            state.entered = true;
            state.item_count = count;
            state.revision = this.harness.file_revision;
            state.source = this.harness.transcript.as_ptr() as usize;
            state.was_running = running;
            if state
                .turn
                .is_some_and(|turn| turn >= state.data.prompts.len())
            {
                state.turn = None;
            }
            let exists = |id: &str| state.data.edits.iter().any(|edit| edit.id == id);
            if state.pinned.as_deref().is_some_and(|id| !exists(id)) {
                state.pinned = None;
            }
            if state.hovered.as_deref().is_some_and(|id| !exists(id)) {
                state.hovered = None;
            }
        }
        if state.following {
            state.pan = 1.;
        }
        state.data.clone()
    });
    let p = palette(cx);
    let (turn, zoom, pan, following, selected, arrivals) = {
        let state = state.read(cx);
        (
            state.turn,
            state.zoom,
            state.pan,
            state.following,
            state.pinned.clone().or_else(|| state.hovered.clone()),
            state.arrivals.clone(),
        )
    };
    let edits: Vec<_> = data
        .edits
        .iter()
        .filter(|edit| turn.is_none_or(|turn| edit.turn == turn))
        .collect();
    let (added, removed, files) = data.totals(turn);
    let unknown = edits.iter().filter(|edit| edit.additions.is_none()).count();
    let now = crate::harness::now_ms();
    let duration = if running && data.origin_ms > 0 {
        data.duration_ms.max(now.saturating_sub(data.origin_ms))
    } else {
        data.duration_ms
    };
    let scope_start = turn
        .and_then(|turn| data.prompts.get(turn))
        .map(|prompt| prompt.at_ms)
        .unwrap_or(0);
    let scope_end = turn
        .and_then(|turn| data.prompts.get(turn + 1))
        .map(|prompt| prompt.at_ms)
        .unwrap_or(duration);
    let target_end =
        scope_start.saturating_add(timeline::axis_end(scope_end.saturating_sub(scope_start)));
    let end = state.update(cx, |state, _| {
        if state.following || state.axis_end == 0 || state.axis_turn != turn {
            state.axis_end = target_end;
            state.axis_turn = turn;
        }
        state.axis_end
    });
    let range = Range::new(scope_start, end, zoom, pan);
    let focus = state.update(cx, |state, cx| {
        state.focus.get_or_insert_with(|| cx.focus_handle()).clone()
    });
    let active = if running && (turn.is_none() || turn == data.prompts.len().checked_sub(1)) {
        data.active.clone()
    } else {
        Vec::new()
    };
    let paths = timeline::lanes(&edits, &active);
    let animated = !reduce_motion
        && arrivals
            .values()
            .any(|start| start.elapsed().as_secs_f32() < TRANSITION);
    if animated || running {
        // No animation task survives a hidden view. Reduced motion still gets
        // low-rate clock/data updates, but no pulse or interpolated geometry.
        let app = cx.entity().downgrade();
        if animated || (!reduce_motion && !active.is_empty()) {
            window.on_next_frame(move |_, cx| {
                let _ = app.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        } else if !state.read(cx).clock_pending {
            state.update(cx, |state, _| state.clock_pending = true);
            let clock = state.downgrade();
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let _ = clock.update(cx, |state, _| state.clock_pending = false);
                let _ = app.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    let mut header = div()
        .id("timeline-header")
        .relative()
        .children(crate::gui_probe::marker("timeline-header"))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(scale::space(scale::SPACE_3)))
        .px(px(scale::space(scale::SPACE_4)))
        .py(px(scale::space(scale::SPACE_3)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(scale::space(2.)))
                .child(
                    div()
                        .text_size(px(scale::text(scale::TEXT_TITLE)))
                        .line_height(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("File timeline"),
                )
                .child(
                    div()
                        .text_size(px(scale::text(scale::TEXT_CAPTION)))
                        .line_height(px(14.))
                        .truncate()
                        .text_color(p.muted_fg)
                        .child(if let Some(path) = active.first() {
                            format!(
                                "Editing {}",
                                crate::app::display_path(
                                    &this.project().path,
                                    std::path::Path::new(path)
                                )
                            )
                        } else if running {
                            "Agent is working · waiting for the next confirmed edit".into()
                        } else {
                            format!(
                                "{} edits · {} elapsed",
                                edits.len(),
                                timeline::time_label(duration)
                            )
                        }),
                ),
        )
        .child(
            div()
                .text_size(px(scale::text(scale::TEXT_TITLE)))
                .font_family(theme::code_font())
                .text_color(p.green)
                .child(
                    MorphText::new("timeline-added", format!("+{added}"))
                        .numeric()
                        .scope(this.session().id.clone())
                        .reduce_motion(reduce_motion),
                ),
        )
        .child(
            div()
                .text_size(px(scale::text(scale::TEXT_TITLE)))
                .font_family(theme::code_font())
                .text_color(p.red)
                .child(
                    MorphText::new("timeline-removed", format!("−{removed}"))
                        .numeric()
                        .scope(this.session().id.clone())
                        .reduce_motion(reduce_motion),
                ),
        )
        .child(
            div()
                .text_size(px(scale::text(scale::TEXT_SMALL)))
                .text_color(p.muted_fg)
                .child(format!("across {files} files")),
        );
    if running {
        header = header.child(
            div()
                .text_size(px(scale::text(scale::TEXT_MICRO)))
                .text_color(p.green)
                .child("LIVE"),
        );
    }

    let all = state.clone();
    let recent = state.clone();
    let previous = state.clone();
    let next = state.clone();
    let out = state.clone();
    let into = state.clone();
    let live = state.clone();
    let fit = state.clone();
    let mut toolbar = div()
        .id("timeline-toolbar")
        .relative()
        .children(crate::gui_probe::marker("timeline-toolbar"))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(scale::space(scale::SPACE_1)))
        .px(px(scale::space(scale::SPACE_4)))
        .py(px(scale::space(scale::SPACE_1)))
        .border_b_1()
        .border_color(p.border)
        .child(
            toolbar_button("timeline-session")
                .label("Whole session")
                .on_click(move |_, _, cx| {
                    all.update(cx, |state, cx| {
                        state.turn = None;
                        state.pinned = None;
                        state.zoom = 1.;
                        state.following = true;
                        cx.notify();
                    });
                }),
        )
        .child(
            toolbar_button("timeline-prompt")
                .label("Selected prompt")
                .on_click(move |_, _, cx| {
                    recent.update(cx, |state, cx| {
                        state.turn = state.data.prompts.len().checked_sub(1);
                        state.pinned = None;
                        state.zoom = 1.;
                        state.following = true;
                        cx.notify();
                    });
                }),
        );
    if let Some(turn) = turn {
        toolbar = toolbar
            .child(
                toolbar_button("timeline-previous-prompt")
                    .label("‹")
                    .accessibility_label("Previous prompt")
                    .on_click(move |_, _, cx| {
                        previous.update(cx, |state, cx| {
                            state.turn = Some(turn.saturating_sub(1));
                            state.pinned = None;
                            cx.notify();
                        });
                    }),
            )
            .child(
                div()
                    .id("timeline-prompt-label")
                    .text_size(px(scale::text(scale::TEXT_SMALL)))
                    .text_color(p.muted_fg)
                    .child(format!("Prompt {} / {}", turn + 1, data.prompts.len()))
                    .tooltip({
                        let label = data.prompts[turn].label.clone();
                        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                    }),
            )
            .child(
                toolbar_button("timeline-next-prompt")
                    .label("›")
                    .accessibility_label("Next prompt")
                    .on_click(move |_, _, cx| {
                        next.update(cx, |state, cx| {
                            state.turn =
                                Some((turn + 1).min(state.data.prompts.len().saturating_sub(1)));
                            state.pinned = None;
                            cx.notify();
                        });
                    }),
            );
    }
    toolbar = toolbar
        .child(div().flex_1())
        .child(
            toolbar_button("timeline-zoom-out")
                .label("−")
                .accessibility_label("Zoom out timeline")
                .on_click(move |_, _, cx| {
                    out.update(cx, |state, cx| {
                        state.zoom = (state.zoom / 1.5).max(1.);
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .text_size(px(scale::text(scale::TEXT_CAPTION)))
                .text_color(p.muted_fg)
                .child(format!("{zoom:.1}×")),
        )
        .child(
            toolbar_button("timeline-zoom-in")
                .label("+")
                .accessibility_label("Zoom in timeline")
                .on_click(move |_, _, cx| {
                    into.update(cx, |state, cx| {
                        state.zoom = (state.zoom * 1.5).min(32.);
                        cx.notify();
                    });
                }),
        )
        .child(
            toolbar_button("timeline-fit")
                .label("Fit")
                .on_click(move |_, _, cx| {
                    fit.update(cx, |state, cx| {
                        state.zoom = 1.;
                        state.pan = 1.;
                        state.following = true;
                        cx.notify();
                    });
                }),
        )
        .child(
            toolbar_button("timeline-live")
                .label(if following {
                    "Following live"
                } else {
                    "Back to live"
                })
                .on_click(move |_, _, cx| {
                    live.update(cx, |state, cx| {
                        state.following = true;
                        state.pan = 1.;
                        state.pinned = None;
                        cx.notify();
                    });
                }),
        );

    let indices: Rc<Vec<usize>> = Rc::new(
        data.edits
            .iter()
            .enumerate()
            .filter(|(_, edit)| turn.is_none_or(|turn| edit.turn == turn))
            .map(|(index, _)| index)
            .collect(),
    );
    let paint_data = data.clone();
    let paint_indices = indices.clone();
    let paint_selected = selected.clone();
    let measure = state.clone();
    let movement = state.clone();
    let press = state.clone();
    let release = state.clone();
    let leave = state.clone();
    let wheel = state.clone();
    let keys = state.clone();
    let move_data = data.clone();
    let move_indices = indices.clone();
    let key_data = data.clone();
    let key_indices = indices.clone();
    let target_scale = Scale::new(added, removed);
    let motion =
        gpui_kit::base::Spring::new(Duration::from_millis(360)).with_travel(!reduce_motion);
    let y = Scale {
        positive: gpui_kit::base::spring(
            format!("{key}:scale-positive"),
            target_scale.positive,
            motion,
            window,
            cx,
        ),
        negative: gpui_kit::base::spring(
            format!("{key}:scale-negative"),
            target_scale.negative,
            motion,
            window,
            cx,
        ),
    };
    let chart = div().id("file-timeline-chart").relative().h(px(HEIGHT)).ml(px(LEFT)).mr(px(RIGHT)).overflow_hidden()
        .children(crate::gui_probe::marker("file-timeline-chart"))
        .track_focus(&focus).role(Role::Image).aria_label(format!("File changes over time: {added} added, {removed} removed, {files} files. Arrow keys inspect edits; Enter pins details; Escape clears selection."))
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            press.update(cx, |state, cx| { state.drag = Some((f32::from(event.position.x), state.pan)); if let Some(focus) = &state.focus { focus.focus(window, cx); } cx.notify(); });
        })
        .on_mouse_move(move |event, _, cx| {
            movement.update(cx, |state, cx| {
                let Some(bounds) = state.bounds else { return };
                let width = f32::from(bounds.size.width).max(1.);
                if let Some((start, pan)) = state.drag
                    && event.pressed_button == Some(MouseButton::Left) && state.zoom > 1. {
                        let delta = (f32::from(event.position.x) - start) / width;
                        state.pan = (pan - delta * state.zoom / (state.zoom - 1.)).clamp(0., 1.);
                        state.following = false;
                        cx.notify(); return;
                    }
                let at = range.time(f32::from(event.position.x - bounds.left()), width);
                let hovered = nearest(&move_data, &move_indices, at).map(|edit| edit.id.clone());
                if state.hovered != hovered { state.hovered = hovered; cx.notify(); }
            });
        })
        .on_mouse_up(MouseButton::Left, move |event, _, cx| {
            release.update(cx, |state, cx| {
                if let Some((start, _)) = state.drag.take() {
                    if (f32::from(event.position.x) - start).abs() < 4. { state.pinned = if state.pinned == state.hovered { None } else { state.hovered.clone() }; if state.pinned.is_some() { state.following = false; } }
                    cx.notify();
                }
            });
        })
        .on_mouse_up_out(MouseButton::Left, { let state = state.clone(); move |_, _, cx| { state.update(cx, |state, _| state.drag = None); } })
        .on_hover(move |hovered, _, cx| { if !hovered { leave.update(cx, |state, cx| { state.hovered = None; cx.notify(); }); } })
        .on_scroll(move |event, _, cx| {
            wheel.update(cx, |state, cx| {
                let delta = event.delta.pixel_delta(px(24.));
                if event.modifiers.control || event.modifiers.platform {
                    state.zoom = (state.zoom * (f32::from(delta.y) * 0.005).exp()).clamp(1., 32.);
                } else if state.zoom > 1. { state.pan = (state.pan - f32::from(delta.x + delta.y) * 0.002).clamp(0., 1.); state.following = false; }
                else { return; }
                cx.stop_propagation(); cx.notify();
            });
        })
        .on_key_down(move |event, _, cx| {
            keys.update(cx, |state, cx| {
                let current = state.pinned.as_ref().or(state.hovered.as_ref()).and_then(|id| key_indices.iter().position(|index| &key_data.edits[*index].id == id));
                let selected = match event.keystroke.key.as_str() {
                    "left" => Some(current.unwrap_or(key_indices.len()).saturating_sub(1)),
                    "right" => Some(current.map(|index| index + 1).unwrap_or(0).min(key_indices.len().saturating_sub(1))),
                    "home" => Some(0), "end" => Some(key_indices.len().saturating_sub(1)),
                    "escape" => { state.pinned = None; state.hovered = None; None }
                    "enter" | "space" => { state.pinned = state.hovered.clone().or_else(|| state.pinned.clone()); None }
                    _ => return,
                };
                if let Some(edit) = selected.and_then(|index| key_indices.get(index)).map(|index| &key_data.edits[*index]) {
                    state.pinned = Some(edit.id.clone()); state.hovered = None;
                    let span = (end - scope_start).max(1) as f32;
                    state.pan = ((edit.at_ms.saturating_sub(scope_start) as f32 / span - 0.5 / state.zoom) / (1. - 1. / state.zoom).max(0.001)).clamp(0., 1.);
                    state.following = false;
                }
                cx.stop_propagation(); cx.notify();
            });
        })
        .child(canvas(move |bounds, _, cx| { measure.update(cx, |state, _| state.bounds = Some(bounds)); },
            move |bounds, _, window, _| { paint_chart(bounds, window, &paint_data, &paint_indices, range, y, duration, paint_selected.as_deref(), &arrivals, reduce_motion, p); }).size_full());

    let mut graph = div()
        .relative()
        .mt(px(scale::space(scale::SPACE_3)))
        .child(chart);
    for (value, label) in [
        (y.positive, format!("+{}", y.positive as u64)),
        (y.positive / 2., format!("+{}", (y.positive / 2.) as u64)),
        (0., "0".into()),
        (-y.negative, format!("−{}", y.negative as u64)),
    ] {
        graph = graph.child(
            div()
                .absolute()
                .left(px(8.))
                .top(px(y.y(value) - 7.))
                .w(px(LEFT - scale::SPACE_5))
                .text_right()
                .text_size(px(scale::text(scale::TEXT_CAPTION)))
                .font_family(theme::code_font())
                .text_color(p.muted_fg)
                .child(label),
        );
    }
    if let Some(edit) = selected
        .as_deref()
        .and_then(|id| edits.iter().find(|edit| edit.id == id))
    {
        let width = state
            .read(cx)
            .bounds
            .map(|bounds| f32::from(bounds.size.width))
            .unwrap_or(600.);
        let (left, detail_width) = detail_geometry(width, range.x(edit.at_ms, width));
        let display_path =
            crate::app::display_path(&this.project().path, std::path::Path::new(&edit.path));
        graph = graph.child(detail(
            edit,
            &display_path,
            state.read(cx).pinned.is_some(),
            left,
            detail_width,
            cx,
            p,
        ));
    }

    let mut lane_list = div()
        .id("timeline-file-lanes")
        .flex()
        .flex_col()
        .gap(px(scale::space(2.)))
        .max_h(px(220.))
        .overflow_y_scroll()
        .mt(px(scale::space(scale::SPACE_3)));
    // Paint the history in one canvas per file, not one element per edit.
    let mut per_path: HashMap<&str, Vec<usize>> = HashMap::new();
    for index in indices.iter() {
        per_path
            .entry(&data.edits[*index].path)
            .or_default()
            .push(*index);
    }
    for (row, path) in paths.iter().enumerate() {
        let lane_indices = Rc::new(per_path.remove(path.as_str()).unwrap_or_default());
        let lane_data = data.clone();
        let hover_data = data.clone();
        let hover_indices = lane_indices.clone();
        let lane_selected = selected.clone();
        let highlighted = selected.as_deref().is_some_and(|id| {
            data.edits
                .iter()
                .any(|edit| edit.id == id && &edit.path == path)
        });
        let is_active = active.contains(path);
        let lane_state = state.clone();
        let click_state = state.clone();
        let first = lane_indices
            .first()
            .map(|index| data.edits[*index].id.clone());
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path);
        let measure = state.clone();
        lane_list = lane_list.child(
            div()
                .id(("timeline-lane", row))
                .relative()
                .children(crate::gui_probe::marker_named(format!(
                    "timeline-lane-{row}"
                )))
                .h(px(scale::control(LANE_HEIGHT)))
                .flex_shrink_0()
                .ml(px(LEFT))
                .mr(px(RIGHT))
                .on_mouse_move(move |event, _, cx| {
                    lane_state.update(cx, |state, cx| {
                        let Some(bounds) = state.bounds else { return };
                        let at = range.time(
                            f32::from(event.position.x - bounds.left()),
                            f32::from(bounds.size.width),
                        );
                        let hovered =
                            nearest(&hover_data, &hover_indices, at).map(|edit| edit.id.clone());
                        if state.hovered != hovered {
                            state.hovered = hovered;
                            cx.notify();
                        }
                    });
                })
                .on_hover(move |hovered, _, cx| {
                    if !hovered {
                        measure.update(cx, |state, cx| {
                            state.hovered = None;
                            cx.notify();
                        });
                    }
                })
                .on_click(move |_, _, cx| {
                    click_state.update(cx, |state, cx| {
                        state.pinned = state.hovered.clone().or(first.clone());
                        state.following = false;
                        cx.notify();
                    });
                })
                .tooltip({
                    let label = format!(
                        "{path} · {} edits{}",
                        lane_indices.len(),
                        if is_active { " · editing" } else { "" }
                    );
                    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                })
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            paint_lane(
                                bounds,
                                window,
                                &lane_data,
                                &lane_indices,
                                range,
                                duration,
                                lane_selected.as_deref(),
                                is_active,
                                reduce_motion,
                                p,
                            );
                        },
                    )
                    .absolute()
                    .inset_0()
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top(px((scale::control(LANE_HEIGHT) - 16.) / 2.))
                        .px(px(scale::space(scale::SPACE_2)))
                        .bg(p.card.opacity(0.94))
                        .text_size(px(scale::text(scale::TEXT_SMALL)))
                        .line_height(px(16.))
                        .font_family(theme::code_font())
                        .text_color(if highlighted || is_active {
                            p.fg
                        } else {
                            p.muted_fg
                        })
                        .child(name.to_string()),
                ),
        );
    }
    let mut axis = div()
        .relative()
        .h(px(28.))
        .ml(px(LEFT))
        .mr(px(RIGHT))
        .mt(px(scale::space(scale::SPACE_1)));
    for index in 0..=4 {
        let at = range.start + (range.end - range.start) * index as f64 / 4.;
        axis = axis
            .child(
                div()
                    .absolute()
                    .left(relative(index as f32 / 4.))
                    .top_0()
                    .w(px(1.))
                    .h(px(5.))
                    .bg(p.border),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(index as f32 / 4.))
                    .ml(px(if index == 4 { -38. } else { 0. }))
                    .top(px(8.))
                    .text_size(px(scale::text(scale::TEXT_CAPTION)))
                    .font_family(theme::code_font())
                    .text_color(p.muted_fg)
                    .child(timeline::time_label(at as u64)),
            );
    }
    let legend = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(scale::space(scale::SPACE_4)))
        .px(px(LEFT))
        .py(px(scale::space(scale::SPACE_3)))
        .text_size(px(scale::text(scale::TEXT_CAPTION)))
        .child(div().text_color(p.green).child("━ Added"))
        .child(div().text_color(p.red).child("━ Removed"))
        .child(
            div()
                .text_color(p.muted_fg)
                .child(format!("┄ Net {:+}", added as i64 - removed as i64)),
        )
        .child(div().text_color(p.muted_fg).child(if unknown > 0 {
            format!("{unknown} edits without line counts")
        } else {
            "Cumulative edits · not the final Git diff".into()
        }));

    let mut body = div()
        .id("file-timeline")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.card)
        .text_color(p.fg)
        .child(header)
        .child(toolbar);
    if edits.is_empty() && active.is_empty() {
        body = body.child(super::empty_activity(
            "No file edits yet",
            "Confirmed file changes from this conversation will appear here as the agent works.",
            IconName::ChartNoAxesGantt,
            cx,
        ));
    } else {
        body = body.child(
            div()
                .id("timeline-content")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(graph)
                .child(lane_list)
                .child(axis)
                .child(legend),
        );
    }
    body.into_any_element()
}

fn nearest<'a>(data: &'a Snapshot, indices: &[usize], at: f64) -> Option<&'a Edit> {
    let index = indices.partition_point(|index| (data.edits[*index].at_ms as f64) < at);
    [index.checked_sub(1), Some(index)]
        .into_iter()
        .flatten()
        .filter_map(|index| indices.get(index))
        .map(|index| &data.edits[*index])
        .min_by(|a, b| ((a.at_ms as f64 - at).abs()).total_cmp(&(b.at_ms as f64 - at).abs()))
}

/// Keep the compact inspector inside the pane, including split workspaces.
fn detail_geometry(plot_width: f32, edit_x: f32) -> (f32, f32) {
    let content_width = LEFT + plot_width.max(0.) + RIGHT;
    let width = DETAIL_WIDTH.min((content_width - RIGHT * 2.).max(1.));
    let right_aligned = (content_width - RIGHT - width).max(RIGHT);
    let left = if edit_x < plot_width * 0.5 {
        right_aligned
    } else {
        LEFT + scale::SPACE_3
    };
    (left.clamp(RIGHT, right_aligned), width)
}

#[derive(Clone, Copy)]
struct Scale {
    positive: f32,
    negative: f32,
}
impl Scale {
    fn new(added: u64, removed: u64) -> Self {
        fn nice(value: f32) -> f32 {
            let power = 10f32.powf(value.max(1.).log10().floor());
            (value / power).ceil() * power
        }
        let positive = nice((added as f32 * 1.08).max(10.));
        Self {
            positive,
            negative: nice((removed as f32 * 1.08).max(positive * 0.22)),
        }
    }
    fn y(self, value: f32) -> f32 {
        22. + (HEIGHT - 44.) * (self.positive - value) / (self.positive + self.negative)
    }
}

fn line(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Hsla, width: f32) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(from);
    path.line_to(to);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
fn dot(window: &mut Window, center: Point<Pixels>, radius: f32, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(
            center - point(px(radius), px(radius)),
            size(px(radius * 2.), px(radius * 2.)),
        ),
        Corners::all(px(radius)),
        color,
        Edges::default(),
        color,
        BorderStyle::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
fn paint_chart(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    data: &Snapshot,
    indices: &[usize],
    range: Range,
    scale: Scale,
    duration: u64,
    selected: Option<&str>,
    arrivals: &HashMap<String, Instant>,
    reduce_motion: bool,
    p: Palette,
) {
    let width = f32::from(bounds.size.width).max(1.);
    let pos = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
    for value in [scale.positive, scale.positive / 2., -scale.negative] {
        let y = scale.y(value);
        for x in (0..width as usize).step_by(7) {
            dot(window, pos(x as f32, y), 0.65, p.border);
        }
    }
    line(
        window,
        pos(0., scale.y(0.)),
        pos(width, scale.y(0.)),
        p.border,
        1.,
    );
    for (index, prompt) in data.prompts.iter().enumerate() {
        let x = range.x(prompt.at_ms, width);
        if index > 0 && x >= 0. && x <= width {
            line(
                window,
                pos(x, 12.),
                pos(x, HEIGHT - 12.),
                p.border.opacity(0.45),
                1.,
            );
        }
    }
    let mut added = 0f32;
    let mut removed = 0f32;
    let mut green = vec![(0f32, scale.y(0.))];
    let mut red = green.clone();
    let mut net = green.clone();
    let mut last_x = 0.;
    let mut selected_point = None;
    for index in indices {
        let edit = &data.edits[*index];
        let factor = if reduce_motion {
            1.
        } else {
            arrivals
                .get(&edit.id)
                .map(|start| {
                    let t = (start.elapsed().as_secs_f32() / TRANSITION).min(1.);
                    1. - (1. - t).powi(3)
                })
                .unwrap_or(1.)
        };
        let target_x = range.x(edit.at_ms, width);
        let x = last_x + (target_x - last_x) * factor;
        added += edit.additions.unwrap_or(0) as f32 * factor;
        removed += edit.deletions.unwrap_or(0) as f32 * factor;
        if x < 0. {
            green[0].1 = scale.y(added);
            red[0].1 = scale.y(-removed);
            net[0].1 = scale.y(added - removed);
        } else if x <= width {
            // Added/removed are monotone: coalescing subpixel steps preserves
            // their totals while bounding tessellation to the viewport width.
            if green.last().is_some_and(|point| x - point.0 < 1.) {
                *green.last_mut().unwrap() = (x, scale.y(added));
                *red.last_mut().unwrap() = (x, scale.y(-removed));
                *net.last_mut().unwrap() = (x, scale.y(added - removed));
            } else {
                green.push((x, scale.y(added)));
                red.push((x, scale.y(-removed)));
                net.push((x, scale.y(added - removed)));
            }
        }
        if selected == Some(edit.id.as_str()) && x >= 0. && x <= width {
            selected_point = Some((
                x,
                scale.y(added),
                scale.y(-removed),
                scale.y(added - removed),
            ));
        }
        last_x = target_x;
    }
    // Time continues while the model thinks, but the line stays flat.
    let tail = range.x(duration, width).clamp(0., width);
    for points in [&mut green, &mut red, &mut net] {
        if let Some(&(x, y)) = points.last()
            && tail > x
        {
            points.push((tail, y));
        }
    }
    let mut curve = |points: &[(f32, f32)], color: Hsla, fill: bool, dashed: bool| {
        if points.len() < 2 {
            return;
        }
        if fill {
            let mut area = PathBuilder::fill();
            area.move_to(pos(points[0].0, scale.y(0.)));
            area.line_to(pos(points[0].0, points[0].1));
            for pair in points.windows(2) {
                let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
                let bend = (x2 - x1).clamp(0., 10.);
                area.line_to(pos((x2 - bend).max(x1), y1));
                area.cubic_bezier_to(
                    pos(x2, y2),
                    pos(x2 - bend / 2., y1),
                    pos(x2 - bend / 2., y2),
                );
            }
            area.line_to(pos(points.last().unwrap().0, scale.y(0.)));
            area.close();
            if let Ok(path) = area.build() {
                window.paint_path(path, color.opacity(0.075));
            }
        }
        let mut stroke = PathBuilder::stroke(px(2.2));
        stroke.move_to(pos(points[0].0, points[0].1));
        for pair in points.windows(2) {
            let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
            let bend = (x2 - x1).clamp(0., 10.);
            if dashed {
                let count = ((x2 - x1).abs() / 7.).ceil().max(1.) as usize;
                for index in 0..count {
                    let t1 = index as f32 / count as f32;
                    let t2 = (index as f32 + 0.5) / count as f32;
                    line(
                        window,
                        pos(x1 + (x2 - x1) * t1, y1 + (y2 - y1) * t1),
                        pos(x1 + (x2 - x1) * t2, y1 + (y2 - y1) * t2),
                        color,
                        1.2,
                    );
                }
            } else {
                stroke.line_to(pos((x2 - bend).max(x1), y1));
                stroke.cubic_bezier_to(
                    pos(x2, y2),
                    pos(x2 - bend / 2., y1),
                    pos(x2 - bend / 2., y2),
                );
            }
        }
        if !dashed && let Ok(path) = stroke.build() {
            window.paint_path(path, color);
        }
        if let Some(&(x, y)) = points.last() {
            dot(window, pos(x, y), 6., color.opacity(0.16));
            dot(window, pos(x, y), 2.6, color);
        }
    };
    curve(&green, p.green, true, false);
    curve(&red, p.red, true, false);
    curve(&net, p.muted_fg.opacity(0.65), false, true);
    if let Some((x, green, red, net)) = selected_point {
        line(
            window,
            pos(x, 8.),
            pos(x, HEIGHT - 8.),
            p.fg.opacity(0.3),
            1.,
        );
        for (y, color) in [(green, p.green), (red, p.red), (net, p.muted_fg)] {
            dot(window, pos(x, y), 4., color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_lane(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    data: &Snapshot,
    indices: &[usize],
    range: Range,
    duration: u64,
    selected: Option<&str>,
    active: bool,
    reduce_motion: bool,
    p: Palette,
) {
    let width = f32::from(bounds.size.width);
    let center = f32::from(bounds.size.height) / 2.;
    let pos = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
    if let (Some(first), Some(last)) = (indices.first(), indices.last()) {
        let start = range.x(data.edits[*first].at_ms, width).clamp(0., width);
        let end = range.x(data.edits[*last].at_ms, width).clamp(0., width);
        line(
            window,
            pos(start, center),
            pos(end, center),
            p.muted_fg.opacity(0.22),
            5.,
        );
    }
    // Multiple edits in the same screen pixel share a marker, keeping paint
    // work bounded even for a file edited thousands of times.
    let mut previous = -10.;
    for index in indices {
        let edit = &data.edits[*index];
        let x = range.x(edit.at_ms, width);
        if x < 0. || x > width {
            continue;
        }
        let emphasis = selected == Some(edit.id.as_str());
        if !emphasis && x - previous < 2. {
            continue;
        }
        let color = if edit.additions.is_none() {
            p.muted_fg
        } else if edit.deletions > edit.additions {
            p.red
        } else {
            p.green
        };
        line(
            window,
            pos(x, center - if emphasis { 8. } else { 5. }),
            pos(x, center + if emphasis { 8. } else { 5. }),
            color.opacity(if selected.is_some() && !emphasis {
                0.45
            } else {
                1.
            }),
            if emphasis { 3. } else { 2. },
        );
        previous = x;
    }
    if active {
        let alpha = if reduce_motion {
            0.9
        } else {
            0.6 + (crate::harness::now_ms() as f64 / 320.).sin() as f32 * 0.25
        };
        dot(
            window,
            pos(
                range.x(duration, width).clamp(4., (width - 4.).max(4.)),
                center,
            ),
            4.,
            p.green.opacity(alpha),
        );
    }
}

fn detail(
    edit: &Edit,
    display_path: &str,
    pinned: bool,
    left: f32,
    width: f32,
    cx: &mut Context<DunlinApp>,
    p: Palette,
) -> AnyElement {
    let source = edit.source;
    let patch = edit.diff.clone();
    let path = edit.path.clone();
    let mut card = div()
        .id("timeline-edit-details")
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .absolute()
        .left(px(left))
        .top(px(scale::space(scale::SPACE_3)))
        .w(px(width))
        .children(crate::gui_probe::marker("timeline-edit-details"))
        .p(px(scale::space(scale::SPACE_3)))
        .rounded(px(radii::MESSAGE))
        .bg(p.floating_surface(0.97))
        .border_1()
        .border_color(p.border)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(scale::space(scale::SPACE_2)))
        .child(
            div()
                .id("timeline-detail-path")
                .relative()
                .children(crate::gui_probe::marker("timeline-detail-path"))
                .min_w(px(0.))
                .text_size(px(scale::text(scale::TEXT_BODY)))
                .line_height(px(16.))
                .font_family(theme::code_font())
                .truncate()
                .child(display_path.to_string())
                .tooltip({
                    let path = edit.path.clone();
                    move |window, cx| Tooltip::new(path.clone()).build(window, cx)
                }),
        )
        .child(
            div()
                .flex()
                .gap(px(scale::space(scale::SPACE_3)))
                .text_size(px(scale::text(scale::TEXT_CAPTION)))
                .line_height(px(14.))
                .text_color(p.muted_fg)
                .child(if edit.timing_known {
                    timeline::time_label(edit.at_ms)
                } else {
                    "Time not recorded".into()
                })
                .child(div().min_w(px(0.)).truncate().child(edit.agent.clone()))
                .child(
                    if let (Some(added), Some(removed)) = (edit.additions, edit.deletions) {
                        format!("+{added} / −{removed}")
                    } else {
                        "Line counts unavailable".into()
                    },
                ),
        );
    if let Some(diff) = &edit.diff {
        let mut preview = div()
            .id("timeline-diff-preview")
            .relative()
            .children(crate::gui_probe::marker("timeline-diff-preview"))
            .rounded(px(radii::ICON))
            .overflow_hidden()
            .font_family(theme::code_font())
            .text_size(px(scale::text(scale::TEXT_CAPTION)))
            .line_height(px(16.));
        for line in diff
            .lines()
            .filter(|line| {
                !line.starts_with("--- ") && !line.starts_with("+++ ") && !line.starts_with("@@")
            })
            .take(5)
        {
            let (background, foreground) = if line.starts_with('+') {
                (p.green_soft, p.green)
            } else if line.starts_with('-') {
                (p.red_soft, p.red)
            } else {
                (p.muted, p.muted_fg)
            };
            preview = preview.child(
                div()
                    .h(px(18.))
                    .px(px(scale::space(scale::SPACE_2)))
                    .bg(background)
                    .text_color(foreground)
                    .truncate()
                    .child(line.chars().take(100).collect::<String>()),
            );
        }
        card = card.child(preview);
    }
    if pinned {
        let mut actions =
            div()
                .flex()
                .gap(px(scale::space(scale::SPACE_1)))
                .child(crate::gui_probe::target(
                    "timeline-open-chat",
                    toolbar_button("timeline-open-chat")
                        .label("Show in Chat")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_timeline_in_chat(source, cx);
                        })),
                ));
        if let Some(patch) = patch {
            actions = actions.child(crate::gui_probe::target(
                "timeline-open-diff",
                toolbar_button("timeline-open-diff")
                    .label("Open full diff")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_timeline_diff(&path, &patch, cx);
                    })),
            ));
        }
        card = card.child(actions);
    } else {
        card = card.child(
            div()
                .text_size(px(scale::text(scale::TEXT_MICRO)))
                .text_color(p.muted_fg)
                .child("Click to pin this edit"),
        );
    }
    card.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_inspector_stays_inside_narrow_and_split_panes() {
        for plot_width in [40., 100., 240., 500., 1000.] {
            for edit_x in [0., plot_width / 2., plot_width] {
                let (left, width) = detail_geometry(plot_width, edit_x);
                assert!(width > 0. && width <= DETAIL_WIDTH);
                assert!(left >= RIGHT);
                assert!(left + width <= LEFT + plot_width);
            }
        }
    }
    #[test]
    fn viewport_zoom_pan_and_coordinate_round_trip_stay_bounded() {
        for zoom in [1., 2., 8., 32.] {
            for pan in [0., 0.5, 1.] {
                let range = Range::new(1000, 61_000, zoom, pan);
                assert!(range.start >= 1000. && range.end <= 61_001.);
                let at = ((range.start + range.end) / 2.) as u64;
                assert!((range.time(range.x(at, 800.), 800.) - at as f64).abs() < 0.1);
            }
        }
    }
    #[test]
    fn empty_and_one_sided_scales_are_finite() {
        for (added, removed) in [(0, 0), (300, 0), (0, 100), (u32::MAX as u64, 2)] {
            let scale = Scale::new(added, removed);
            for value in [added as f32, -(removed as f32), 0.] {
                assert!(scale.y(value).is_finite());
            }
            assert!(scale.y(0.) > 0. && scale.y(0.) < HEIGHT);
        }
    }
}
