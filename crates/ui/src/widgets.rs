use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;

use crate::theme::{Palette, palette};

/// A crisp Lucide icon at an explicit size and colour.
pub fn icon(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

pub fn dot(color: Hsla, size: f32) -> Div {
    div()
        .size(px(size))
        .rounded_full()
        .bg(color)
        .flex_shrink_0()
}

/// A compact keyboard hint, matching the quiet chrome in Kobra.
pub fn kbd(label: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(5.))
        .rounded(px(5.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_family("Geist Mono")
        .text_color(p.muted_fg)
        .child(label.into())
}

pub fn badge(label: impl Into<SharedString>, fg: Hsla, bg: Hsla, p: Palette) -> Div {
    div()
        .h(px(19.))
        .px(px(7.))
        .rounded_full()
        .bg(bg)
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .flex()
        .items_center()
        .gap_1()
        .border_1()
        .border_color(p.border.opacity(0.0))
        .child(label.into())
}

pub fn source_badge(source: &str, p: Palette) -> Div {
    let (fg, bg) = match source {
        "live" => (p.green, p.green_soft),
        "session" => (p.blue, p.blue_soft),
        "estimated" => (p.amber, p.amber_soft),
        _ => (p.muted_fg, p.muted),
    };
    badge(source, fg, bg, p).text_size(px(9.)).h(px(17.))
}

/// Compact provider usage row shown in the sidebar footer and usage sheet.
pub fn usage_meter(
    provider: impl Into<SharedString>,
    plan: impl Into<SharedString>,
    percent: f32,
    accent: Hsla,
    source: &str,
    detail: impl Into<SharedString>,
    p: Palette,
) -> Div {
    let provider = provider.into();
    let plan = plan.into();
    let detail = detail.into();
    let pct = percent.clamp(0.0, 100.0);

    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .py(px(7.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(provider),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(plan),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family("Geist Mono")
                        .text_color(if pct > 85. { p.red } else { p.muted_fg })
                        .child(format!("{pct:.0}%")),
                ),
        )
        .child(
            div()
                .h(px(4.))
                .w_full()
                .rounded_full()
                .bg(p.muted)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(relative(pct / 100.0))
                        .rounded_full()
                        .bg(accent),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(source_badge(source, p))
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(detail),
                ),
        )
}

/// Quiet dot grid used behind the agent graph.
pub fn dot_grid(color: Hsla) -> impl IntoElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let step = 22.0f32;
            let radius = px(0.9);
            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            let mut y = 11.0f32;
            while y < height {
                let mut x = 11.0f32;
                while x < width {
                    let origin = bounds.origin + point(px(x), px(y));
                    window.paint_quad(quad(
                        Bounds::new(origin, size(radius * 2., radius * 2.)),
                        Corners::all(radius),
                        color,
                        Edges::default(),
                        color,
                        BorderStyle::default(),
                    ));
                    x += step;
                }
                y += step;
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A thin separator that matches the surrounding surface.
pub fn divider(p: Palette) -> Div {
    div().h(px(1.)).w_full().bg(p.border)
}

/// Small monospaced metadata label.
pub fn meta(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(10.5))
        .font_family("Geist Mono")
        .text_color(p.muted_fg)
        .child(text.into())
}

pub fn app_palette(cx: &App) -> Palette {
    palette(cx)
}
