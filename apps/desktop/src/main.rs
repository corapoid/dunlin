#![recursion_limit = "256"]

mod app;
mod demo;
mod git;
mod harness;
mod projection;
mod search;
mod tree;
mod views;

use app::KestrelApp;
use kestrel_ui::component::TitleBar;
use kestrel_ui::*;

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        kestrel_ui::init(cx);

        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(120.), px(90.)),
                    size(px(1360.), px(860.)),
                ))),
                window_min_size: Some(size(px(1100.), px(720.))),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| cx.new(|cx| KestrelApp::new(window, cx)),
        )
        .expect("failed to open the Kestrel window");
    });
}
