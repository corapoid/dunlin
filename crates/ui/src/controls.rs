//! Dunlin's styled controls reuse GPUI Kit's keyboard, focus and menu behavior.
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    switch::Switch,
    tab::{Tab, TabBar},
};
use gpui_kit::*;

use crate::{Palette, scale};

/// Standard action. Use the kit's semantic variants, sizes and loading state.
pub fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .small()
        .rounded(px(scale::RADIUS_MD))
        // The kit activates on key-up; keep key-down from activating a parent tab.
        .on_key_down(|event, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
            }
        })
}

/// A toolbar action always has an accessible name, even without visible text.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    button(id)
        .ghost()
        .size(px(28.))
        .icon(icon)
        .accessibility_label(label.clone())
        .tooltip(label)
}

pub fn switch(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Switch {
    Switch::new(id)
        .small()
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

pub fn document_tabs(id: impl Into<ElementId>) -> TabBar {
    // shortcut: kit tabs rely on app navigation shortcuts; adopt roving focus when the kit supports it.
    TabBar::new(id).outline().small().max_width(px(240.))
}

pub fn document_tab(label: impl Into<SharedString>) -> Tab {
    Tab::new().label(label)
}

pub fn segment_group(p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(scale::RADIUS_MD + 2.))
        .border_1()
        .border_color(p.border)
        .bg(p.subtle)
}

pub fn segment_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Button {
    button(id)
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

pub fn menu_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    button(id)
        .ghost()
        .accessibility_label(label.clone())
        .child(div().flex_1().min_w(px(0.)).text_left().child(label))
        .w_full()
        .justify_start()
        .flex_shrink_0()
}

/// Shared opaque surface; glass belongs to window chrome, not reading areas.
pub fn card_surface(p: Palette) -> Div {
    div()
        .rounded(px(scale::RADIUS_LG))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
}

pub fn dialog_surface(p: Palette) -> Div {
    card_surface(p)
        .border_color(p.border_strong)
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
    panel_header(p)
        .bg(p.subtle)
        .font_family(crate::theme::code_font())
}

/// One inset workspace contains editors and docks, preserving their split layout.
pub fn workspace_surface(p: Palette) -> Div {
    card_surface(p)
        .rounded(px(scale::WORKSPACE_RADIUS))
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
}

pub fn chrome_background(p: Palette, reduce_transparency: bool) -> Hsla {
    if cfg!(target_os = "macos") && !reduce_transparency {
        p.panel.opacity(0.45)
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
            titlebar.traffic_light_position = Some(point(px(16.), px(18.)));
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keeps_native_titlebar_behavior_and_platform_fallback() {
        let options = window_options();
        assert!(options.app_owns_titlebar_drag);
        assert!(options.titlebar.unwrap().appears_transparent);
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
        for p in [crate::theme::light(), crate::theme::dark()] {
            assert_eq!(chrome_background(p, true).a, 1.);
            assert_eq!(
                chrome_background(p, false).a,
                if cfg!(target_os = "macos") { 0.45 } else { 1. }
            );
            assert_eq!(p.card.a, 1.);
            assert!(scale::WORKSPACE_RADIUS > scale::RADIUS_MD);
        }
    }
}
