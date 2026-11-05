pub mod scale;
pub mod theme;
mod controls;
mod widgets;
mod prompts;

pub use gpui_kit::component;
pub use gpui_kit::*;
pub use theme::{Palette, apply, card, install, palette, set_mode, toggle_mode};
pub use widgets::*;
pub use controls::*;
#[cfg(debug_assertions)]
pub use prompts::debug_confirmation_answer;

pub fn init(cx: &mut App) {
    // GPUI Kit owns the theme, input, dock and command registries. Install it
    // before Dunlin overrides any tokens.
    gpui_kit::init(cx);
    theme::install(cx);
    prompts::install(cx);
}
