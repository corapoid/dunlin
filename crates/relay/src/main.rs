//! Kestrel relay: accounts, pairing, project grants and ACP-over-WebSocket.
//!
//! The relay stores only metadata (users, devices, grants and a session index).
//! Prompts, tool output and transcripts stay in OpenCode and in the clients.
//! The harness runs on this machine, so a session keeps working when the
//! laptop that started it is closed.

mod acp;
mod api;
mod auth;
mod config;
mod store;

use std::path::PathBuf;

use api::AppState;
use config::Config;
use store::Store;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_help();
        return;
    }

    let result = match args[0].as_str() {
        "init" => cmd_init(&args[1..]),
        "user" => cmd_user(&args[1..]),
        "project" => cmd_project(&args[1..]),
        "grant" => cmd_grant(&args[1..]),
        "pair" => cmd_pair(&args[1..]),
        "devices" => cmd_devices(&args[1..]),
        "revoke" => cmd_revoke(&args[1..]),
        "serve" => cmd_serve(&args[1..]),
        other => Err(format!("Unknown command: {other}")),
    };

    if let Err(error) = result {
        eprintln!("kestrel-relay: {error}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "kestrel-relay — remote ACP server for Kestrel\n\
         \n\
         Usage:\n\
           kestrel-relay init [--db PATH]\n\
           kestrel-relay user add NAME [--admin]\n\
           kestrel-relay user list\n\
           kestrel-relay project add NAME PATH [--user USER]...\n\
           kestrel-relay project list\n\
           kestrel-relay grant PROJECT USER\n\
           kestrel-relay pair USER [--ttl MINUTES]\n\
           kestrel-relay devices USER\n\
           kestrel-relay revoke DEVICE_ID\n\
           kestrel-relay serve [--config PATH] [--db PATH] [--listen ADDR] [--harness CMD]\n\
         \n\
         Defaults:\n\
           db       ~/Library/Application Support/Kestrel/relay.db (macOS)\n\
           listen   127.0.0.1:8787\n\
           harness  opencode acp"
    );
}

fn cmd_init(args: &[String]) -> Result<(), String> {
    let db = flag(args, "--db")
        .map(PathBuf::from)
        .unwrap_or_else(default_db);
    if db.exists() && !has_flag(args, "--force") {
        return Err(format!(
            "{} already exists (use --force to add another admin)",
            db.display()
        ));
    }
    let store = Store::open(&db).map_err(|error| error.to_string())?;
    let admin = store
        .create_user("admin", "admin")
        .map_err(|error| error.to_string())?;
    let (code, expires) = store
        .create_pairing_code(&admin.id, 30 * 60)
        .map_err(|error| error.to_string())?;
    println!("Created {}", db.display());
    println!("Admin user: {} ({})", admin.name, admin.id);
    println!("Pairing code: {code}  (valid until {expires})");
    println!(
        "Pair a device with: kestrel-relay pair {} --db {}",
        admin.name,
        db.display()
    );
    println!("Serve with: kestrel-relay serve --db {}", db.display());
    Ok(())
}

fn cmd_user(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args
                .get(1)
                .ok_or_else(|| "user add needs a name".to_string())?;
            let role = if has_flag(args, "--admin") {
                "admin"
            } else {
                "member"
            };
            let user = db
                .create_user(name, role)
                .map_err(|error| error.to_string())?;
            println!("{} ({}) · {role}", user.name, user.id);
            Ok(())
        }
        Some("list") => {
            for user in db.list_users().map_err(|error| error.to_string())? {
                println!("{} · {} · {}", user.id, user.name, user.role);
            }
            Ok(())
        }
        _ => Err("usage: kestrel-relay user add NAME [--admin] | user list".into()),
    }
}

fn cmd_project(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args
                .get(1)
                .ok_or_else(|| "project add needs a name".to_string())?;
            let path = args
                .get(2)
                .ok_or_else(|| "project add needs a path".to_string())?;
            let canonical =
                std::fs::canonicalize(path).map_err(|error| format!("{path}: {error}"))?;
            if !canonical.is_dir() {
                return Err(format!("{path} is not a folder"));
            }
            let project = db
                .create_project(name, &canonical.to_string_lossy())
                .map_err(|error| error.to_string())?;
            println!("{} ({}) · {}", project.name, project.id, project.path);
            let mut index = 3;
            while index < args.len() {
                if args[index] == "--user"
                    && let Some(user_name) = args.get(index + 1)
                {
                    let user = db
                        .find_user_by_name(user_name)
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("Unknown user: {user_name}"))?;
                    db.grant(&project.id, &user.id)
                        .map_err(|error| error.to_string())?;
                    println!("Granted {} to {}", project.name, user.name);
                    index += 2;
                    continue;
                }
                index += 1;
            }
            Ok(())
        }
        Some("list") => {
            for project in db.list_projects().map_err(|error| error.to_string())? {
                println!("{} · {} · {}", project.id, project.name, project.path);
            }
            Ok(())
        }
        _ => {
            Err("usage: kestrel-relay project add NAME PATH [--user USER]... | project list".into())
        }
    }
}

fn cmd_grant(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    let project = args
        .first()
        .ok_or_else(|| "grant needs a project name".to_string())?;
    let user = args
        .get(1)
        .ok_or_else(|| "grant needs a user name".to_string())?;
    let project = db
        .find_project_by_name(project)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Unknown project: {project}"))?;
    let user = db
        .find_user_by_name(user)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Unknown user: {user}"))?;
    db.grant(&project.id, &user.id)
        .map_err(|error| error.to_string())?;
    println!("Granted {} to {}", project.name, user.name);
    Ok(())
}

fn cmd_pair(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    let name = args
        .first()
        .ok_or_else(|| "pair needs a user name".to_string())?;
    let ttl = flag(args, "--ttl")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(10)
        .clamp(1, 24 * 60);
    let user = db
        .find_user_by_name(name)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Unknown user: {name}"))?;
    let (code, expires) = db
        .create_pairing_code(&user.id, ttl * 60)
        .map_err(|error| error.to_string())?;
    println!("Pairing code for {}: {code}", user.name);
    println!("Expires in {ttl} minutes ({expires}).");
    println!("Open Kestrel → Remote → Pair device and enter this code.");
    Ok(())
}

fn cmd_devices(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    let name = args
        .first()
        .ok_or_else(|| "devices needs a user name".to_string())?;
    let user = db
        .find_user_by_name(name)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Unknown user: {name}"))?;
    for device in db
        .list_devices(&user.id)
        .map_err(|error| error.to_string())?
    {
        let status = if device.revoked_at.is_some() {
            "revoked"
        } else {
            "active"
        };
        println!(
            "{} · {} · {status} · last seen {}",
            device.id,
            device.name,
            device.last_seen_at.unwrap_or(0)
        );
    }
    Ok(())
}

fn cmd_revoke(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    let id = args
        .first()
        .ok_or_else(|| "revoke needs a device id".to_string())?;
    db.revoke_device(id).map_err(|error| error.to_string())?;
    println!("Revoked {id}");
    Ok(())
}

fn cmd_serve(args: &[String]) -> Result<(), String> {
    let mut config = flag(args, "--config")
        .map(|path| Config::load(PathBuf::from(path)))
        .transpose()?
        .unwrap_or_default();
    if let Some(db) = flag(args, "--db") {
        config.db = PathBuf::from(db);
    }
    if let Some(listen) = flag(args, "--listen") {
        config.listen = listen;
    }
    if let Some(harness) = flag(args, "--harness") {
        config.harness = harness;
    }
    config.apply_env();

    let store = Store::open(&config.db).map_err(|error| error.to_string())?;
    if store
        .list_users()
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err(format!(
            "{} has no users. Run `kestrel-relay init --db {}` first.",
            config.db.display(),
            config.db.display()
        ));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let state = AppState::new(store, config.clone());
        let app = api::router(state);
        let listener = tokio::net::TcpListener::bind(&config.listen)
            .await
            .map_err(|error| format!("Could not listen on {}: {error}", config.listen))?;
        eprintln!(
            "[relay] listening on {} · db {} · harness {}",
            config.listen,
            config.db.display(),
            config.harness
        );
        eprintln!("[relay] metadata only · no prompt logging");
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|error| error.to_string())
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("[relay] shutting down");
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn open_store_for(args: &[String]) -> Result<Store, String> {
    let path = flag(args, "--db")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("KESTREL_RELAY_DB").map(PathBuf::from))
        .unwrap_or_else(default_db);
    Store::open(&path).map_err(|error| error.to_string())
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn default_db() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if cfg!(target_os = "macos") {
            return home.join("Library/Application Support/Kestrel/relay.db");
        }
        return home.join(".local/share/kestrel/relay.db");
    }
    PathBuf::from("relay.db")
}
