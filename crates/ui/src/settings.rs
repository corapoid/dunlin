//! Shared settings surfaces. Pages own one scroll container; cards never scroll.
use crate::*;

pub fn page() -> Div {
    div()
        .w_full()
        .min_w(px(0.))
        .px(px(24.))
        .py(px(20.))
        .flex()
        .flex_col()
        .gap(px(20.))
}

pub fn section(label: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.fg)
        .child(label.into())
}

pub fn card(p: Palette) -> Div {
    SurfaceGeometry::SETTINGS_CARD.paint(p.card, p.border)
}

/// Painted header flush with a settings card's top edge; never rely on overflow.
pub fn card_header(p: Palette) -> Div {
    SurfaceGeometry::SETTINGS_CARD
        .round_child(div(), SurfaceEdges::Top)
        .border_b_1()
        .border_color(p.border)
        .bg(p.subtle)
}

pub fn actions() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.))
}

pub fn description(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(p.muted_fg)
        .child(text.into())
}

pub fn notice(text: impl Into<SharedString>, p: Palette) -> Div {
    card(p).p(px(12.)).child(description(text, p))
}

pub fn divider(p: Palette) -> Div {
    div().mx(px(14.)).h(px(1.)).bg(p.border)
}

pub fn row(title: &str, description: &str, control: AnyElement, p: Palette) -> Div {
    div()
        .min_h(px(scale::control(56.)))
        .px(px(14.))
        .py(px(11.))
        .flex()
        .items_center()
        .gap(px(16.))
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
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_owned()),
                )
                .child(self::description(description.to_owned(), p)),
        )
        .child(div().flex_shrink_0().max_w(relative(0.55)).child(control))
}

pub fn field(label: &str, control: impl IntoElement, p: Palette) -> Div {
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(section(label.to_owned(), p))
        .child(control)
}
