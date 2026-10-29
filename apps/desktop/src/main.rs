#![recursion_limit = "512"]

mod app;
mod demo;
mod harness;
mod logging;
mod platform;
mod projection;
mod providers;
mod remote;
mod views;
mod watch;
mod window_state;

use app::DunlinApp;
use dunlin_ui::component::TitleBar;
use dunlin_ui::*;

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        cx.set_app_identity("dev.dunlin.ide", "Dunlin");
        dunlin_ui::init(cx);
        logging::init();
        cx.bind_keys([KeyBinding::new("cmd-q", app::Quit, None)]);
        cx.set_menus([Menu::new("Dunlin").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit Dunlin", app::Quit),
        ])]);

        let window_bounds = window_state::load()
            .map(|geometry| {
                let displays: Vec<window_state::DisplayRect> = cx
                    .displays()
                    .iter()
                    .map(|display| {
                        window_state::DisplayRect::from_bounds(&display.visible_bounds())
                    })
                    .collect();
                window_state::to_bounds(window_state::clamp(geometry, &displays))
            })
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
            |window, cx| cx.new(|cx| DunlinApp::new(window, cx)),
        )
        .expect("failed to open the Dunlin window");
    });
}
