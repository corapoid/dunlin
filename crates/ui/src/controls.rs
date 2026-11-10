//! Dunlin's styled controls reuse GPUI Kit's keyboard, focus and menu behavior.
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    FocusableExt, Selectable, Sizable,
    button::{Button, ButtonVariant, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    radio::Radio,
    switch::Switch,
    tab::{Tab, TabBar},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{Palette, SurfaceEdges, SurfaceGeometry, scale};

/// One single-line field geometry for forms, toolbars and search. The kit still
/// owns focus, selection, validation and disabled behavior.
pub fn input(state: &Entity<InputState>) -> Input {
    let height = px(scale::control(scale::INPUT_HEIGHT));
    Styled::h(Input::new(state), height)
        .min_h(height)
        .flex_shrink_0()
        .rounded(px(crate::radii::MODAL))
        .py(px(4.))
        .text_size(px(scale::TEXT_BODY_LARGE))
        .font_family(crate::theme::ui_font())
}

pub fn compact_input(state: &Entity<InputState>) -> Input {
    input(state)
}

pub fn search_input(state: &Entity<InputState>, label: impl Into<SharedString>) -> Input {
    compact_input(state)
        .aria_label(label)
        .prefix(gpui_kit::component::Icon::new(IconName::Search).size(px(12.)))
}

pub fn code_input(state: &Entity<InputState>) -> Input {
    input(state).font_family(crate::theme::code_font())
}

pub fn textarea(state: &Entity<TextareaState>) -> Textarea {
    Textarea::new(state).font_family(crate::theme::ui_font())
}

/// The composer is a compound control; its surrounding card owns the focus border.
/// Document editors are intentionally not form fields and keep their own surface.
pub fn composer_input(state: &Entity<TextareaState>) -> Textarea {
    textarea(state).appearance(false).bordered(false)
}

/// Quiet base for navigation, status controls and compound surfaces.
/// Ordinary form/settings actions use `action_button`, not this constructor.
pub fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .ghost()
        .self_center()
        .rounded(px(scale::RADIUS_MD))
        // The kit activates on key-up; keep key-down from activating a parent tab.
        .on_key_down(|event, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
            }
        })
}

/// Standard labeled action, kept separate from navigation and status chips.
pub fn action_button(id: impl Into<ElementId>) -> Button {
    button(id)
        .with_variant(ButtonVariant::Default)
        .h(px(scale::control(scale::INPUT_HEIGHT)))
        .raised(true)
        .label_renderer(|label| crate::MorphText::new("button-label", label).into_any_element())
}

/// A toolbar action always has an accessible name, even without visible text.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    named_icon_button(id, icon, label.clone()).tooltip(label)
}

/// Flat, compact labeled actions for toolbars (not raised form buttons).
pub fn toolbar_button(id: impl Into<ElementId>) -> Button {
    button(id)
        .small()
        .h(px(28.))
        .px(px(10.))
        .rounded(px(crate::radii::CONTROL))
        .shadow_none()
        .text_size(px(11.))
}

/// One sliding selection surface; labels must not paint a second selected pill.
/// The caller animates `position` and supplies equal-width, transparent buttons.
pub fn sliding_segments(p: Palette, position: f32, count: usize, height: f32) -> Div {
    assert!(count > 0);
    div()
        .relative()
        .flex()
        .items_center()
        .h(px(height))
        .p(px(3.))
        .rounded(px(crate::settings::Tokens::CONTROL_RADIUS))
        .bg(p.subtle)
        .child(
            div().absolute().inset_0().m(px(3.)).child(
                div()
                    .absolute()
                    .left(relative(
                        position.clamp(0., (count - 1) as f32) / count as f32,
                    ))
                    .w(relative(1. / count as f32))
                    .h_full()
                    .rounded(px(crate::settings::Tokens::CONTROL_RADIUS))
                    .bg(p.card),
            ),
        )
}

pub fn sliding_segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    p: Palette,
    cx: &App,
) -> Button {
    use gpui_kit::component::button::ButtonCustomVariant;
    button(id)
        .custom(ButtonCustomVariant::new(cx).foreground(p.fg))
        .small()
        .h_full()
        .flex_1()
        .min_w(px(0.))
        .p(px(0.))
        .border_0()
        .shadow_none()
        .rounded(px(crate::settings::Tokens::CONTROL_RADIUS))
        .toggled(active)
        .text_size(px(11.))
        .label(label)
}

fn named_icon_button(id: impl Into<ElementId>, icon: IconName, label: SharedString) -> Button {
    button(id)
        .small()
        .ghost()
        .size(px(28.))
        .icon(icon)
        .accessibility_label(label)
}

/// Rich hover content keeps the same size and keyboard behavior as ordinary icons.
pub fn tooltip_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
) -> Button {
    let mut button = named_icon_button(id, icon, label.into());
    button.interactivity().tooltip(tooltip);
    button
}

pub fn switch(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Switch {
    Switch::new(id)
        .small()
        .focus_ring(false)
        .accessibility_label(label)
        .checked(checked)
}

pub fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
) -> Checkbox {
    Checkbox::new(id)
        .small()
        .accessibility_label(label)
        .checked(checked)
}

pub fn radio(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Radio {
    Radio::new(id).accessibility_label(label).checked(checked)
}

pub fn document_tabs(id: impl Into<ElementId>) -> TabBar {
    // shortcut: kit tabs rely on app navigation shortcuts; adopt roving focus when the kit supports it.
    TabBar::new(id)
        .folder()
        .max_width(px(240.))
        .border_0()
        .bg(transparent_black())
}

pub fn document_tab(label: impl Into<SharedString>) -> Tab {
    Tab::new()
        .label(label)
        .border_0()
        .border_color(transparent_black())
}

pub fn segment_group(p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(scale::RADIUS_MD + 2.))
        .bg(p.subtle)
}

pub fn navigation_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    button(id)
        .ghost()
        .size(px(36.))
        .px(px(0.))
        .accessibility_label(label.clone())
        .tooltip(label)
        .child(gpui_kit::component::Icon::new(icon).size(px(17.)))
}

pub fn segment_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Button {
    button(id)
        .small()
        .ghost()
        .label(label)
        .selected(active)
        .toggled(active)
}

/// A rich action row; interactive suffixes belong outside this button.
pub fn action_row(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id)
        .ghost()
        .accessibility_label(label)
        .w_full()
        .h_auto()
        .py(px(8.))
        .justify_start()
        .flex_shrink_0()
}

/// Full-width card action: hover/selected paint must follow the card, not the
/// smaller ordinary button radius. An expanded card has a square internal join.
pub fn card_action_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    expanded: bool,
) -> Button {
    SurfaceGeometry::CARD.round_child(
        action_row(id, label),
        if expanded {
            SurfaceEdges::Top
        } else {
            SurfaceEdges::All
        },
    )
}

/// Exclusive choices use the same focus, selected and disabled behavior as actions.
pub fn choice_card(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    p: Palette,
) -> Button {
    action_row(id, label)
        .selected(selected)
        .toggled(selected)
        .rounded(px(SurfaceGeometry::CHOICE_CARD.radius))
        .border(px(SurfaceGeometry::CHOICE_CARD.border))
        .border_color(if selected { p.border_strong } else { p.border })
        .p(px(14.))
        .items_start()
}

/// Decorative indicator inside a keyboard-accessible choice, never a separate radio.
pub fn choice_indicator(selected: bool, p: Palette) -> Div {
    div()
        .mt(px(1.))
        .size(px(16.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(if selected { p.fg } else { p.border_strong })
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |this| {
            this.child(div().size(px(7.)).rounded_full().bg(p.fg))
        })
}

pub fn menu_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    button(id)
        .ghost()
        .with_size(gpui_kit::component::Size::Medium)
        .h(px(34.))
        .accessibility_label(label.clone())
        .child(div().flex_1().min_w(px(0.)).text_left().child(label))
        .w_full()
        .justify_start()
        .flex_shrink_0()
}

/// Shared opaque surface; glass belongs to window chrome, not reading areas.
pub fn card_surface(p: Palette) -> Div {
    SurfaceGeometry::CARD.paint(p.card, p.border)
}

pub fn dialog_surface(p: Palette) -> Div {
    SurfaceGeometry::DIALOG
        .paint(p.card, p.border_strong)
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(12.))
}

pub fn panel_header(p: Palette) -> Div {
    div()
        .h(px(scale::HEADER_HEIGHT))
        .flex_shrink_0()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .text_size(px(12.))
        .text_color(p.muted_fg)
}

pub fn code_surface(p: Palette) -> Div {
    card_surface(p)
        .overflow_hidden()
        .font_family(crate::theme::code_font())
}

pub fn code_header(p: Palette) -> Div {
    SurfaceGeometry::CARD
        .round_child(panel_header(p), SurfaceEdges::Top)
        .bg(p.subtle)
        .font_family(crate::theme::code_font())
}

/// One inset workspace contains editors and docks, preserving their split layout.
pub fn workspace_surface(p: Palette) -> Div {
    SurfaceGeometry::WORKSPACE
        .paint(transparent_black(), p.border)
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
}

pub fn chrome_background(p: Palette, reduce_transparency: bool) -> Hsla {
    if cfg!(target_os = "macos") && !reduce_transparency {
        p.chrome
    } else {
        p.panel
    }
}

/// Native backdrop is provided by GPUI/AppKit, never by a screenshot or gradient.
pub fn window_options() -> WindowOptions {
    let mut options = gpui_kit::component::TitleBar::window_options();
    if cfg!(target_os = "macos") {
        options.window_background = WindowBackgroundAppearance::Blurred;
        if let Some(titlebar) = &mut options.titlebar {
            titlebar.traffic_light_position = Some(point(px(16.), px(12.)));
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_keep_keyboard_support_without_a_focus_outline() {
        assert!(!switch("test-switch", "Test switch", false).is_focus_ring_enabled());
    }

    #[test]
    fn choices_share_theme_tokens_in_both_palettes() {
        for p in [crate::theme::light(), crate::theme::dark()] {
            for selected in [false, true] {
                let mut choice = choice_card("choice", "Permission mode", selected, p);
                assert_eq!(
                    choice.style().border_color,
                    Some(if selected { p.border_strong } else { p.border })
                );
                assert_eq!(choice.style().border_widths.top, Some(px(1.).into()));
                let mut indicator = choice_indicator(selected, p);
                assert_eq!(
                    indicator.style().border_color,
                    Some(if selected { p.fg } else { p.border_strong })
                );
            }
        }
    }

    #[test]
    fn document_tabs_have_no_frame() {
        let mut tab = document_tab("Conversation");
        let mut bar = document_tabs("tabs");
        for borders in [
            tab.style().border_widths.clone(),
            bar.style().border_widths.clone(),
        ] {
            assert_eq!(borders.top, Some(px(0.).into()));
            assert_eq!(borders.bottom, Some(px(0.).into()));
            assert_eq!(borders.left, Some(px(0.).into()));
            assert_eq!(borders.right, Some(px(0.).into()));
        }
    }

    #[test]
    fn window_keeps_native_titlebar_behavior_and_platform_fallback() {
        let options = window_options();
        assert!(options.app_owns_titlebar_drag);
        let titlebar = options.titlebar.unwrap();
        assert!(titlebar.appears_transparent);
        if cfg!(target_os = "macos") {
            assert_eq!(
                titlebar.traffic_light_position.unwrap().y + px(7.),
                px(scale::TITLEBAR_HEIGHT / 2.)
            );
        }
        assert_eq!(
            options.window_background,
            if cfg!(target_os = "macos") {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            }
        );
    }

    #[test]
    fn reduced_transparency_and_non_mac_chrome_are_opaque() {
        const { assert!(scale::WORKSPACE_RADIUS > scale::RADIUS_MD) };
        for p in [crate::theme::light(), crate::theme::dark()] {
            assert_eq!(chrome_background(p, true).a, 1.);
            assert_eq!(
                chrome_background(p, false).a,
                if cfg!(target_os = "macos") {
                    p.chrome.a
                } else {
                    1.
                }
            );
            assert_eq!(p.card.a, 1.);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn dark_glass_stays_dark_even_over_a_white_background() {
        let tint: Rgba = chrome_background(crate::theme::dark(), false).into();
        for channel in [tint.r, tint.g, tint.b] {
            assert!(channel * tint.a + 1. - tint.a < 0.3);
        }
    }
}
