//! Shared settings surfaces. Pages own one scroll container; cards never scroll.
use crate::*;
use component::{
    button::Button,
    input::{Input, InputState},
};

/// Settings use one semantic recipe, never page-specific colors or dimensions.
/// A future skin only needs to supply a Palette and change this recipe.
pub struct Tokens {
    pub surface: Hsla,
    pub sidebar: Hsla,
    pub field: Hsla,
    pub outline: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
}

impl Tokens {
    pub const NAV_WIDTH: f32 = 216.;
    pub const PAGE_INSET: f32 = 24.;
    pub const PAGE_GAP: f32 = 18.;
    pub const CONTROL_HEIGHT: f32 = scale::INPUT_HEIGHT;
    pub const CONTROL_RADIUS: f32 = radii::MODAL;
    pub const TITLE_SIZE: f32 = 18.;
    pub const BODY_SIZE: f32 = 12.;
    pub const CAPTION_SIZE: f32 = 11.;

    pub fn new(p: Palette) -> Self {
        Self {
            surface: p.bg,
            sidebar: p.panel,
            field: p.subtle,
            outline: p.border,
            text: p.fg,
            secondary: p.muted_fg,
        }
    }
}

/// Keep the shared keyboard/disabled/loading behavior and animated action label.
pub fn button(id: impl Into<ElementId>) -> Button {
    crate::action_button(id)
        .shadow_none()
        .h(px(scale::control(Tokens::CONTROL_HEIGHT)))
        .rounded(px(Tokens::CONTROL_RADIUS))
        .px(px(14.))
        .text_size(px(Tokens::BODY_SIZE))
}

/// Filled fields retain the kit's real focus frame and accessible input behavior.
pub fn input(state: &Entity<InputState>, p: Palette) -> Input {
    crate::input(state).bg(Tokens::new(p).field)
}

pub fn search(state: &Entity<InputState>, label: impl Into<SharedString>, p: Palette) -> Input {
    input(state, p)
        .aria_label(label)
        .prefix(component::Icon::new(assets::IconName::Search).size(px(14.)))
}

pub fn segments(p: Palette) -> Div {
    crate::segment_group(p)
        .rounded(px(Tokens::CONTROL_RADIUS))
        .self_start()
}

pub fn segment(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Button {
    crate::segment_button(id, label, active)
        .h(px(scale::control(Tokens::CONTROL_HEIGHT) - 4.))
        .rounded(px(Tokens::CONTROL_RADIUS))
        .px(px(12.))
        .text_size(px(Tokens::CAPTION_SIZE))
}

pub fn page() -> Div {
    div()
        .w_full()
        .min_w(px(0.))
        .px(px(Tokens::PAGE_INSET))
        .pt(px(12.))
        .pb(px(24.))
        .flex()
        .flex_col()
        .gap(px(Tokens::PAGE_GAP))
}

pub fn section(label: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(Tokens::BODY_SIZE))
        .font_weight(FontWeight::MEDIUM)
        .text_color(Tokens::new(p).secondary)
        .child(label.into())
}

pub fn card(p: Palette) -> Div {
    let t = Tokens::new(p);
    SurfaceGeometry::SETTINGS_CARD.paint(t.surface, t.outline)
}

/// Ordinary preferences need whitespace, not a box around every group.
pub fn group() -> Div {
    div().w_full().min_w(px(0.)).flex().flex_col()
}

/// Painted header flush with a settings card's top edge; never rely on overflow.
pub fn card_header(p: Palette) -> Div {
    SurfaceGeometry::SETTINGS_CARD
        .round_child(div(), SurfaceEdges::Top)
        .border_b_1()
        .border_color(p.border)
        .bg(Tokens::new(p).surface)
}

pub fn actions() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.))
}

pub fn description(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(Tokens::CAPTION_SIZE))
        .line_height(relative(1.4))
        .text_color(p.muted_fg)
        .child(text.into())
}

pub fn notice(text: impl Into<SharedString>, p: Palette) -> Div {
    card(p).p(px(12.)).child(description(text, p))
}

pub fn divider(p: Palette) -> Div {
    div()
        .mx(px(18.))
        .h(px(1.))
        .bg(Tokens::new(p).outline.opacity(0.55))
}

pub fn row(title: &str, description: &str, control: AnyElement, p: Palette) -> Div {
    div()
        .min_h(px(scale::control(62.)))
        .px(px(14.))
        .py(px(scale::space(11.)))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .flex_1()
                .min_w(px(220.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(Tokens::BODY_SIZE))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_owned()),
                )
                .child(self::description(description.to_owned(), p)),
        )
        .child(div().ml_auto().flex_shrink_0().max_w_full().child(control))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_follows_both_application_palettes_without_a_private_color_scheme() {
        for p in [theme::light(), theme::dark()] {
            let t = Tokens::new(p);
            assert_eq!(t.surface, p.bg);
            assert_eq!(t.sidebar, p.panel);
            assert_eq!(t.field, p.subtle);
            assert_eq!(t.outline, p.border);
            assert_eq!(t.text, p.fg);
            assert_eq!(t.secondary, p.muted_fg);
            assert_ne!(t.field, t.surface);
        }
        const {
            assert!(Tokens::CONTROL_HEIGHT >= 32.);
            assert!(Tokens::TITLE_SIZE > Tokens::BODY_SIZE);
            assert!(Tokens::BODY_SIZE > Tokens::CAPTION_SIZE);
        }
    }
}
