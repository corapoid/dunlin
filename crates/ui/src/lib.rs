pub mod scale;
pub mod theme;
mod widgets;

pub use gpui_kit::component;
pub use gpui_kit::*;
pub use theme::{Palette, apply, card, install, palette, set_mode, toggle_mode};
pub use widgets::*;

pub fn init(cx: &mut App) {
    // GPUI Kit owns the theme, input, dock and command registries. Install it
    // before Dunlin overrides any tokens.
    gpui_kit::init(cx);
    theme::install(cx);
}
