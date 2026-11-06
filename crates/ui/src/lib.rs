mod controls;
mod prompts;
pub mod scale;
pub mod theme;
mod widgets;

pub use controls::*;
pub use gpui_kit::component;
pub use gpui_kit::*;
#[cfg(debug_assertions)]
pub use prompts::debug_confirmation_answer;
pub use theme::{Palette, apply, card, install, palette, set_mode, toggle_mode};
pub use widgets::*;

pub fn init(cx: &mut App) {
    // GPUI Kit owns the theme, input, dock and command registries. Install it
    // before Dunlin overrides any tokens.
    gpui_kit::init(cx);
    theme::install(cx);
    prompts::install(cx);
}
