//! Trace waterfall: span hierarchy, timings, cost and the selected span's
//! details. Designed for long agent runs where the interesting work is nested
//! three levels deep.

use std::time::Duration;

use dunlin_domain::{SessionStatus, SpanKind, SpanStatus, TraceSpan};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, TraceFilter, format_ms, format_tokens, span_color};

const LABEL_WIDTH: f32 = 296.0;

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let preview = this.harness.transcript.is_empty();
    let session = this.session().clone();
    let spans = if preview {
        crate::demo::trace_spans()
    } else {
        crate::projection::spans(&this.harness.transcript, &session)
    };
    let total_ms = spans
        .iter()
        .map(TraceSpan::end_ms)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let tokens = session.tokens;
    let total_duration = format_ms(total_ms as u64);
    let filter = this.trace_filter;
    let selected = this.selected_span;
    let following = this.follow_running;
    let zoom = this.trace_zoom;
    let row_h = (28.0 + (zoom - 1.0) * 10.0).clamp(24.0, 36.0);
    let filtered: Vec<TraceSpan> = spans
        .iter()
        .filter(|span| filter.matches(span.kind))
        .cloned()
        .collect();
    let total_cost: f64 = spans.iter().map(|span| span.cost_usd).sum();
    let running = spans
        .iter()
        .filter(|span| span.status == SpanStatus::Running)
        .count();

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
            Icon::new(IconName::ChartNoAxesGantt)
                .size(px(14.))
                .text_color(p.teal),
        )
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Trace waterfall"),
        )
        .child(
            div()
                .h(px(17.))
                .px(px(5.))
                .rounded_full()
                .bg(if preview {
                    p.amber_soft
                } else {
                    match session.status {
                        SessionStatus::Running => p.green_soft,
                        SessionStatus::Waiting => p.amber_soft,
                        SessionStatus::Failed => p.red_soft,
                        _ => p.muted,
                    }
                })
                .flex()
                .items_center()
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(if preview {
                            p.amber
                        } else {
                            match session.status {
                                SessionStatus::Running => p.green,
                                SessionStatus::Waiting => p.amber,
                                SessionStatus::Failed => p.red,
                                _ => p.muted_fg,
                            }
                        })
                        .child(if preview {
                            "PREVIEW"
                        } else {
                            match session.status {
                                SessionStatus::Running => "LIVE",
                                SessionStatus::Waiting => "WAITING",
                                SessionStatus::Failed => "FAILED",
                                _ => "IDLE",
                            }
                        }),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!("{} spans · {running} running", spans.len())),
        )
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .text_size(px(11.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!(
                    "{} · ${total_cost:.2} · {}",
                    total_duration,
                    format_tokens(tokens)
                )),
        )
        .child(div().flex_1());

    let mut toolbar = toolbar;
    for chip in TraceFilter::ALL {
        let active = filter == chip;
        let count = spans.iter().filter(|span| chip.matches(span.kind)).count();
        toolbar = toolbar.child(
            div()
                .id(format!("trace-filter:{}", chip.label()))
                .h(px(22.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(if active {
                    p.border_strong
                } else {
                    p.border.opacity(0.0)
                })
                .bg(if active { p.card } else { p.bg.opacity(0.0) })
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.trace_filter = chip;
                    this.selected_span = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if active { p.fg } else { p.muted_fg })
                        .child(chip.label()),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(count.to_string()),
                ),
        );
    }

    toolbar = toolbar
        .child(div().w(px(1.)).h(px(13.)).bg(p.border))
        .child(
            div()
                .id("trace-follow")
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
                        .text_size(px(11.0))
                        .text_color(if following { p.blue } else { p.muted_fg })
                        .child("Follow"),
                ),
        )
        .child(
            div()
                .h(px(23.))
                .px(px(5.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap(px(2.))
                .child(zoom_button(IconName::ZoomOut, -0.1, cx, &p))
                .child(
                    div()
                        .min_w(px(32.))
                        .text_center()
                        .text_size(px(11.0))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(format!("{:.0}%", zoom * 100.)),
                )
                .child(zoom_button(IconName::ZoomIn, 0.1, cx, &p)),
        );

    let header = div()
        .h(px(26.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(p.border)
        .bg(p.subtle)
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_shrink_0()
                .pl(px(12.))
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("SPAN"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .pr(px(10.))
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.65))
                        .child(format!("{} shown", filtered.len())),
                ),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .relative()
                .flex()
                .items_center()
                .justify_between()
                .px(px(4.))
                .children([0, 25, 50, 75, 100].map(|pct| {
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.75))
                        .child(axis_label(total_ms * pct as f32 / 100.0))
                        .into_any_element()
                })),
        );

    let mut rows = div().flex().flex_col();
    for span in &filtered {
        let index = spans
            .iter()
            .position(|candidate| candidate.id == span.id)
            .unwrap_or(0);
        rows = rows.child(span_row(span, index, selected, row_h, total_ms, &p, cx));
    }

    let timeline = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(header)
        .child(
            div()
                .id("trace-rows")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(rows),
        );

    let detail = selected
        .and_then(|index| spans.get(index))
        .map(|span| span_detail(span, &spans, cx, &p));

    let mut body = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(timeline);
    if let Some(detail) = detail {
        body = body.child(detail);
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.bg)
        .child(toolbar)
        .child(body)
        .into_any_element()
}

fn zoom_button(
    icon_name: IconName,
    delta: f32,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    div()
        .id(if delta > 0.0 {
            "trace-zoom-in"
        } else {
            "trace-zoom-out"
        })
        .size(px(19.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.trace_zoom = (this.trace_zoom + delta).clamp(0.7, 1.5);
            cx.notify();
        }))
        .child(Icon::new(icon_name).size(px(11.)).text_color(p.muted_fg))
        .into_any_element()
}

fn axis_label(ms: f32) -> String {
    if ms < 1_000.0 {
        format!("{}ms", ms as u32)
    } else {
        format!("{:.1}s", ms / 1_000.0)
    }
}

/// Fractions occupied by a trace bar, including zero-duration events at the
/// end of the timeline. Never use the minimum width as a clamp lower bound:
/// in f32, `1.0 - 0.999` is slightly less than `0.001` and reverses the bounds.
fn span_bar_geometry(start_ms: u64, duration_ms: u64, total_ms: f32) -> (f32, f32) {
    let total_ms = if total_ms.is_finite() && total_ms > 0.0 {
        total_ms
    } else {
        1.0
    };
    let start = (start_ms as f32 / total_ms).clamp(0.0, 0.999);
    let remaining = 1.0 - start;
    let width = (duration_ms as f32 / total_ms).max(0.001).min(remaining);
    (start, width)
}

fn span_row(
    span: &TraceSpan,
    index: usize,
    selected: Option<usize>,
    row_h: f32,
    total_ms: f32,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let active = selected == Some(index);
    let color = span_color(p, span.kind);
    let (status_color, _status_soft) = status_palette(p, span.status);
    let (start, width) = span_bar_geometry(span.start_ms, span.duration_ms, total_ms);
    let indent = 10.0 + span.depth as f32 * 10.0;

    let bar_width = width;
    let bar = div()
        .absolute()
        .top(px((row_h - 15.0) / 2.0))
        .left(relative(start))
        .w(relative(width))
        .min_w(px(2.))
        .h(px(15.))
        .rounded(px(3.))
        .bg(if span.status == SpanStatus::Done {
            color.opacity(0.72)
        } else {
            color
        })
        .when(span.status == SpanStatus::Running, |this| {
            this.border_1().border_color(color)
        })
        .when(active, |this| {
            this.border_1().border_color(p.fg.opacity(0.45))
        })
        .with_animation(
            ElementId::Name(format!("trace-bar-{}", span.id).into()),
            Animation::new(Duration::from_millis(320)).with_easing(ease_out_quint()),
            move |this, t| this.w(relative(bar_width * t)),
        );

    let row = div()
        .id(("span-row", index))
        .h(px(row_h))
        .flex_shrink_0()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(p.border.opacity(0.55))
        .when(active, |this| this.bg(p.accent))
        .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_span = if this.selected_span == Some(index) {
                None
            } else {
                Some(index)
            };
            cx.notify();
        }))
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .h_full()
                .flex_shrink_0()
                .pl(px(indent))
                .pr(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .size(px(18.))
                        .flex_shrink_0()
                        .rounded(px(5.))
                        .bg(color.opacity(0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(span_icon(span.kind))
                                .size(px(10.5))
                                .text_color(color),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(0.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(
                                    if span.kind == SpanKind::Agent || span.kind == SpanKind::Turn {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    },
                                )
                                .text_color(if active { p.fg } else { p.fg.opacity(0.85) })
                                .truncate()
                                .child(span.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg.opacity(0.75))
                                .truncate()
                                .child(format!(
                                    "{} · {} · {}",
                                    span.agent,
                                    span.kind.label(),
                                    format_ms(span.duration_ms)
                                )),
                        ),
                )
                .when(
                    span.status == SpanStatus::Waiting || span.status == SpanStatus::Running,
                    |this| {
                        this.child(
                            div().child(dot(status_color, 5.0)).with_animation(
                                ElementId::Name(format!("trace-pulse-{}", span.id).into()),
                                Animation::new(Duration::from_millis(1600))
                                    .repeat_synced()
                                    .with_easing(pulsating_between(0.35, 1.0)),
                                |this, t| this.opacity(t),
                            ),
                        )
                    },
                ),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .relative()
                .child(grid_lines(p))
                .child(bar),
        );

    row.with_animation(
        ElementId::Name(format!("trace-row-{}", span.id).into()),
        Animation::new(Duration::from_millis(280)).with_easing(ease_out_quint()),
        |this, t| {
            this.opacity(0.3 + 0.7 * t)
                .relative()
                .left(px((1.0 - t) * 5.0))
        },
    )
    .into_any_element()
}

fn grid_lines(p: &Palette) -> AnyElement {
    let mut lines = div().absolute().top_0().left_0().size_full();
    for pct in [0.25f32, 0.5, 0.75] {
        lines = lines.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(pct))
                .w(px(1.))
                .bg(p.border.opacity(0.4)),
        );
    }
    lines.into_any_element()
}

fn span_detail(
    span: &TraceSpan,
    spans: &[TraceSpan],
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let color = span_color(p, span.kind);
    let (status_color, status_soft) = status_palette(p, span.status);
    let parent = span
        .parent
        .as_ref()
        .and_then(|id| spans.iter().find(|candidate| &candidate.id == id));
    let parent_index = parent.and_then(|p| spans.iter().position(|c| c.id == p.id));
    let children: Vec<&TraceSpan> = spans
        .iter()
        .filter(|candidate| candidate.parent.as_ref() == Some(&span.id))
        .collect();
    let span_id = span.id.clone();
    let span_json = serde_json::to_string_pretty(span).unwrap_or_default();

    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(11.))
        .px(px(12.))
        .py(px(11.))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(7.))
                .child(
                    div()
                        .size(px(24.))
                        .flex_shrink_0()
                        .rounded(px(6.))
                        .bg(color.opacity(0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(span_icon(span.kind))
                                .size(px(13.))
                                .text_color(color),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .child(span.name.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .h(px(16.))
                                        .px(px(5.))
                                        .rounded(px(3.))
                                        .bg(status_soft)
                                        .flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(10.0))
                                                .font_family("Geist Mono")
                                                .text_color(status_color)
                                                .child(span.status.label().to_uppercase()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg)
                                        .child(format!("{} · {}", span.kind.label(), span.agent)),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("close-span-detail")
                        .size(px(19.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.selected_span = None;
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg)),
                ),
        )
        .child(
            div()
                .rounded(px(7.))
                .border_1()
                .border_color(p.border)
                .bg(p.subtle)
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(relative(1.5))
                        .text_color(p.muted_fg)
                        .child(span.detail.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(metric("Duration", format_ms(span.duration_ms), p))
                .child(metric("Start", format_ms(span.start_ms), p)),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(metric("Cost", format!("${:.2}", span.cost_usd), p))
                .child(metric("Tokens", format_tokens(span.tokens), p)),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(metric("Depth", span.depth.to_string(), p))
                .child(metric("Children", children.len().to_string(), p)),
        );

    if let Some(parent) = parent {
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("PARENT"),
                )
                .child(
                    div()
                        .id("trace-parent")
                        .h(px(26.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.card)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(index) = parent_index {
                                this.selected_span = Some(index);
                                cx.notify();
                            }
                        }))
                        .child(
                            Icon::new(span_icon(parent.kind))
                                .size(px(10.5))
                                .text_color(span_color(p, parent.kind)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(p.fg.opacity(0.85))
                                .truncate()
                                .child(parent.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child(format_ms(parent.duration_ms)),
                        ),
                ),
        );
    }

    if !children.is_empty() {
        let mut child_list = div().flex().flex_col().gap(px(2.)).child(
            div()
                .text_size(px(10.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!("CHILDREN · {}", children.len())),
        );
        for child in children.iter().take(5) {
            let child_index = spans.iter().position(|c| c.id == child.id);
            child_list = child_list.child(
                div()
                    .id(format!("trace-child:{}", child.id))
                    .h(px(23.))
                    .px(px(5.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(index) = child_index {
                            this.selected_span = Some(index);
                            cx.notify();
                        }
                    }))
                    .child(
                        Icon::new(span_icon(child.kind))
                            .size(px(9.5))
                            .text_color(span_color(p, child.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(p.muted_fg)
                            .truncate()
                            .child(child.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .font_family("Geist Mono")
                            .text_color(p.muted_fg.opacity(0.7))
                            .child(format_ms(child.duration_ms)),
                    ),
            );
        }
        content = content.child(child_list);
    }

    content = content.child(
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                div()
                    .id("span-open-chat")
                    .h(px(23.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .bg(p.fg)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_span_in_chat(&span_id, cx);
                    }))
                    .child(
                        Icon::new(IconName::MessageSquare)
                            .size(px(10.))
                            .text_color(p.bg),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.bg)
                            .child("Open in chat"),
                    ),
            )
            .child(
                div()
                    .id("span-copy-json")
                    .h(px(23.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.border)
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !span_json.is_empty() {
                            cx.write_to_clipboard(ClipboardItem::new_string(span_json.clone()));
                        }
                        this.set_notice("Span JSON copied to the clipboard", cx);
                    }))
                    .child(
                        Icon::new(IconName::Copy)
                            .size(px(9.5))
                            .text_color(p.muted_fg),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.muted_fg)
                            .child("Copy JSON"),
                    ),
            ),
    );

    let detail = div()
        .w(px(324.))
        .h_full()
        .flex_shrink_0()
        .border_l_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(34.))
                .flex_shrink_0()
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(5.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    Icon::new(IconName::ChartSpline)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child("Span detail"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(span.id.clone()),
                ),
        )
        .child(
            div()
                .id("trace-detail-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(content),
        );

    detail
        .with_animation(
            ElementId::Name(format!("trace-detail-{}", span.id).into()),
            Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
            |this, t| this.opacity(t).relative().left(px((1.0 - t) * 8.0)),
        )
        .into_any_element()
}

fn metric(label: &str, value: String, p: &Palette) -> AnyElement {
    div()
        .flex_1()
        .rounded(px(6.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .px(px(7.))
        .py(px(5.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(10.0))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(label.to_uppercase()),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_family("Geist Mono")
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.fg)
                .child(value),
        )
        .into_any_element()
}

fn status_palette(p: &Palette, status: SpanStatus) -> (Hsla, Hsla) {
    match status {
        SpanStatus::Running => (p.blue, p.blue_soft),
        SpanStatus::Done => (p.green, p.green_soft),
        SpanStatus::Failed => (p.red, p.red_soft),
        SpanStatus::Waiting => (p.amber, p.amber_soft),
    }
}

fn span_icon(kind: SpanKind) -> IconName {
    match kind {
        SpanKind::Agent => IconName::Bot,
        SpanKind::Turn => IconName::MessageSquare,
        SpanKind::Llm => IconName::Sparkles,
        SpanKind::Tool => IconName::Wrench,
        SpanKind::Permission => IconName::ShieldCheck,
        SpanKind::Waiting => IconName::Clock3,
        SpanKind::Handoff => IconName::GitFork,
        SpanKind::Compaction => IconName::Minimize2,
    }
}

#[cfg(test)]
mod tests {
    use super::span_bar_geometry;

    #[test]
    fn trace_bars_at_the_timeline_end_do_not_panic() {
        for start_ms in [999, 1_000, 1_001] {
            let (start, width) = span_bar_geometry(start_ms, 0, 1_000.0);
            assert!(width > 0.0);
            assert!(start + width <= 1.0);
        }
    }

    #[test]
    fn normal_trace_bar_preserves_its_position_and_duration() {
        assert_eq!(span_bar_geometry(250, 500, 1_000.0), (0.25, 0.5));
        assert_eq!(span_bar_geometry(0, 1_000, 1_000.0), (0.0, 1.0));
    }

    #[test]
    fn trace_geometry_stays_finite_and_inside_the_timeline() {
        for total in [0.0, -1.0, 1.0, 1_000.0, f32::NAN, f32::INFINITY] {
            for start_ms in [0, 999, 1_000, u64::MAX] {
                for duration_ms in [0, 1, 1_000, u64::MAX] {
                    let (start, width) = span_bar_geometry(start_ms, duration_ms, total);
                    assert!(start.is_finite() && width.is_finite());
                    assert!(start >= 0.0 && width > 0.0);
                    assert!(start + width <= 1.0);
                }
            }
        }
    }
}
