#![recursion_limit = "512"]

mod activity;
mod ai_commit;
mod app;
mod backup;
mod branch_picker;
mod chat_presentation;
mod chat_scroll;
mod chat_timing;
mod cli;
mod comparisons;
#[cfg(debug_assertions)]
mod controls_smoke;
mod coordination;
mod dashboard;
mod effects;
mod explorer;
mod factory_reset;
mod file_icons;
mod file_preview;
mod git_actions;
mod git_history;
#[cfg(debug_assertions)]
mod glass_smoke;
#[cfg(debug_assertions)]
mod governance_smoke;
mod group_cleanup;
mod group_settings;
mod gui_probe;
#[cfg(debug_assertions)]
mod gui_scenarios;
#[cfg(debug_assertions)]
mod gui_test;
mod harness;
mod harness_brand;
mod harness_discovery;
mod inline_blame;
mod knowledge;
mod local_projects;
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
mod project_catalog;
mod project_groups;
mod project_icons;
mod project_import;
mod project_runtime_settings;
mod project_tree;
mod projection;
mod provider_auth;
mod providers;
mod qa;
mod remote;
mod remote_connection;
#[cfg(debug_assertions)]
mod remote_e2e;
mod run;
#[cfg(debug_assertions)]
mod save_smoke;
mod security;
mod server_panel;
mod session_import;
mod session_list;
mod sidebar_layout;
mod skills;
mod spec;
mod ssh;
mod subtask_worktrees;
#[cfg(debug_assertions)]
mod surfaces_smoke;
mod task_budgets;
mod terminal;
#[cfg(test)]
mod ui_contract_tests;
mod usage;
mod user_profile;
mod vault;
mod views;
mod watch;
mod window_state;
mod workbench;
mod workspace_panes;
mod workspace_text;
#[cfg(debug_assertions)]
mod workspace_ui_smoke;

use app::DunlinApp;
use dunlin_ui::*;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Err(error) = cli::validate(&args, cfg!(debug_assertions)) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    #[cfg(debug_assertions)]
    if args.first().is_some_and(|arg| arg == "--remote-e2e") {
        std::process::exit(remote_e2e::run());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: dunlin [--help | --version | --syntax-notices | --mcp-vault | --mcp-coordination]"
        );
        #[cfg(debug_assertions)]
        println!(
            "Debug native tests: --list-native-smokes (JSON registry for scripts/release_qa.py)"
        );
        return;
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!("Dunlin {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    #[cfg(debug_assertions)]
    if args
        .first()
        .is_some_and(|arg| arg == "--list-native-smokes")
    {
        println!("{}", serde_json::to_string(&cli::native_smokes()).unwrap());
        return;
    }
    #[cfg(debug_assertions)]
    if let Some(scenario) = args
        .first()
        .and_then(|arg| arg.to_str())
        .and_then(gui_scenarios::find)
    {
        gui_scenarios::run(scenario);
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--surfaces-smoke") {
        surfaces_smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--settings-smoke") {
        views::settings::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--save-smoke") {
        save_smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--coordination-smoke") {
        coordination::smoke::run();
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--controls-smoke") {
        controls_smoke::run();
        return;
    }
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
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--governance-smoke") {
        match governance_smoke::run() {
            Ok(()) => {
                println!("governance smoke passed");
                return;
            }
            Err(error) => {
                eprintln!("governance smoke failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if std::env::args().nth(1).as_deref() == Some("--mcp-vault") {
        std::process::exit(dunlin_vault::mcp::run_stdio());
    }
    if std::env::args().nth(1).as_deref() == Some("--mcp-coordination") {
        std::process::exit(dunlin_coordination::mcp::run_stdio());
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
                ..dunlin_ui::window_options()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut app = DunlinApp::new(window, cx);
                    app.refresh_user_profile(cx);
                    app
                })
            },
        )
        .expect("failed to open the Dunlin window");
    });
}
