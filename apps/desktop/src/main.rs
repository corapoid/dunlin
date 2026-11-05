#![recursion_limit = "512"]

mod activity;
mod ai_commit;
mod app;
mod branch_picker;
mod chat_presentation;
mod chat_scroll;
mod chat_timing;
mod comparisons;
mod dashboard;
mod effects;
mod explorer;
mod factory_reset;
mod file_icons;
mod file_preview;
mod git_history;
mod git_actions;
#[cfg(debug_assertions)]
mod glass_smoke;
mod group_settings;
mod harness;
mod harness_discovery;
mod harness_brand;
mod inline_blame;
mod local_projects;
mod project_import;
mod logging;
mod markdown_editor;
mod mcp;
mod minimap;
mod native_menu;
#[cfg(debug_assertions)]
mod onboarding_smoke;
mod overlay_window;
mod platform;
mod plugins;
mod project_activity;
mod project_groups;
mod project_icons;
mod project_catalog;
mod project_runtime_settings;
mod project_tree;
mod projection;
mod provider_auth;
mod providers;
mod qa;
mod remote;
mod run;
mod security;
mod session_import;
mod session_list;
mod sidebar_layout;
mod skills;
mod spec;
mod subtask_worktrees;
mod task_budgets;
mod terminal;
mod usage;
mod vault;
mod views;
mod watch;
mod window_state;
mod workbench;
#[cfg(debug_assertions)]
mod workspace_ui_smoke;

use app::DunlinApp;
use dunlin_ui::component::TitleBar;
use dunlin_ui::*;

fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--project-import-smoke") {
        project_import::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--workspace-ui-smoke") {
        workspace_ui_smoke::run();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--mcp-vault") {
        std::process::exit(dunlin_vault::mcp::run_stdio());
    }
    if std::env::args().nth(1).as_deref() == Some("--vault-keychain-smoke") {
        use dunlin_vault::store::{KeychainStore, SecretStore};
        let id = format!("smoke-{}", uuid::Uuid::now_v7().simple());
        let value = "dunlin-vault-smoke-value";
        let policy = r#"{"smoke":true}"#;
        let result = (|| -> Result<(), String> {
            KeychainStore.set(&id, value)?;
            let read = KeychainStore.get(&id)?;
            if read.as_str() != value {
                return Err("value round-trip mismatch".into());
            }
            KeychainStore.set_policy(&id, policy)?;
            let read = KeychainStore.get_policy(&id)?;
            if read.as_str() != policy {
                return Err("policy round-trip mismatch".into());
            }
            KeychainStore.delete(&id)?;
            KeychainStore.delete_policy(&id)?;
            if KeychainStore.get(&id).is_ok() {
                return Err("value was not deleted".into());
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                println!("vault keychain smoke passed");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("vault keychain smoke failed: {error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--markdown-smoke") {
        markdown_editor::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--git-smoke") {
        git_history::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--workbench-smoke") {
        git_history::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--glass-smoke") {
        glass_smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--onboarding-smoke") {
        onboarding_smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--session-import-smoke") {
        session_import::smoke::run();
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
