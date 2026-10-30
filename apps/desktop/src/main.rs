#![recursion_limit = "512"]

mod activity;
mod ai_commit;
mod app;
mod branch_picker;
mod chat_presentation;
mod chat_scroll;
mod chat_timing;
mod dashboard;
mod effects;
mod explorer;
mod file_icons;
mod file_preview;
mod git_history;
#[cfg(debug_assertions)]
mod glass_smoke;
mod harness;
mod harness_discovery;
mod local_projects;
mod logging;
mod mcp;
mod minimap;
mod native_menu;
mod overlay_window;
mod platform;
mod project_activity;
mod project_groups;
mod project_icons;
mod project_tree;
mod projection;
mod provider_auth;
mod providers;
mod remote;
mod session_import;
mod session_list;
mod sidebar_layout;
mod skills;
mod usage;
mod views;
mod watch;
mod window_state;
mod workbench;

use app::DunlinApp;
use dunlin_ui::component::TitleBar;
use dunlin_ui::*;

fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--glass-smoke") {
        glass_smoke::run();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--syntax-notices") {
        print!("{}", two_face::acknowledgement::listing().to_md());
        return;
    }
    application().with_assets(assets::AllAssets).run(|cx| {
        cx.set_app_identity("dev.dunlin.ide", "Dunlin");
        dunlin_ui::init(cx);
        logging::init();
        cx.bind_keys([KeyBinding::new("cmd-q", app::Quit, None)]);
        native_menu::install(cx);

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
