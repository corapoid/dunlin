#![recursion_limit = "256"]

mod app;
mod demo;
mod harness;
mod logging;
mod projection;
mod remote;
mod usage;
mod views;
mod window_state;

use app::KestrelApp;
use kestrel_ui::component::TitleBar;
use kestrel_ui::*;

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        kestrel_ui::init(cx);
        logging::init();

        let window_bounds = window_state::load()
            .map(window_state::to_bounds)
            .unwrap_or_else(|| {
                WindowBounds::Windowed(Bounds::new(
                    point(px(120.), px(90.)),
                    size(px(1360.), px(860.)),
                ))
            });

        open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                window_min_size: Some(size(px(1100.), px(720.))),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| cx.new(|cx| KestrelApp::new(window, cx)),
        )
        .expect("failed to open the Kestrel window");
    });
}
