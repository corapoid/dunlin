//! Dunlin relay: accounts, pairing, project grants and ACP-over-WebSocket.
//!
//! The relay stores metadata (users, devices, grants and a session index) and,
//! when `store_transcripts` is enabled (the default), transcript snapshots for
//! recovery on another device. OpenCode remains the source of truth for live
//! sessions. The legacy ACP bridge still owns its harness per WebSocket; only
//! the approved-job and opt-in interactive runtimes are independent of that
//! client lifecycle. Persistent remote sessions use their own fenced runtime.

mod acp;
mod api;
mod auth;
mod checkout_identity;
mod config;
mod coordination;
mod interactive;
mod jobs;
mod linear;
mod local_metadata;
mod management;
mod mcp_oauth;
mod oidc;
mod remote_execution;
mod remote_services;
mod remote_sessions;
mod scoped_secrets;
mod store;
mod tailscale;
mod terminal;
mod vault;
mod worker_channel;
mod worker_client;
mod worker_execution;
mod worker_exports;
#[cfg(all(test, unix))]
mod worker_fixture;

use std::path::PathBuf;

use api::AppState;
use config::Config;
use store::Store;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--mcp-tools") {
        std::process::exit(interactive::mcp::run_stdio());
    }
    if args.first().map(String::as_str) == Some("--mcp-vault") {
        std::process::exit(dunlin_vault::mcp::run_stdio());
    }
    if args.first().map(String::as_str) == Some("--mcp-coordination") {
        std::process::exit(dunlin_coordination::mcp::run_stdio());
    }
    if args.is_empty() || matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_help();
        return;
    }

    let result = match args[0].as_str() {
        "init" => cmd_init(&args[1..]),
        "user" => cmd_user(&args[1..]),
        "project" => cmd_project(&args[1..]),
        "grant" => cmd_grant(&args[1..], true),
        "ungrant" => cmd_grant(&args[1..], false),
        "pair" => cmd_pair(&args[1..]),
        "devices" => cmd_devices(&args[1..]),
        "revoke" => cmd_revoke(&args[1..]),
        "serve" => cmd_serve(&args[1..]),
        "mcp-oauth-legacy" => mcp_oauth::maintenance::command(&args[1..]),
        "mcp-client-metadata" => mcp_oauth::config::export_client_metadata(&args[1..])
            .map(|document| println!("{document}")),
        "host" => tailscale::command(&args[1..]),
        "local-metadata" => local_metadata::command(&args[1..]),
        // Private local guardian entry must run before any Tokio worker threads.
        "worker" if args.get(1).is_some_and(|arg| arg == "execution-guard") => {
            worker_execution::runtime::execution_guard(&args[2..])
        }
        "worker" => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| "worker runtime unavailable".to_string())
            .and_then(|runtime| runtime.block_on(worker_client::command(&args[1..]))),
        "vault" => cmd_vault(&args[1..]),
        "setup" => management::setup(&args[1..]),
        other => Err(format!("Unknown command: {other}")),
    };

    if let Err(error) = result {
        eprintln!("dunlin-relay: {error}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "dunlin-relay — remote ACP server for Dunlin\n\
         \n\
         Usage:\n\
           dunlin-relay init [--db PATH]\n\
           dunlin-relay user add NAME [--admin]\n\
           dunlin-relay user list\n\
           dunlin-relay project add NAME PATH [--user USER]...\n\
           dunlin-relay project list\n\
           dunlin-relay grant PROJECT USER [--expected-revision N]\n\
           dunlin-relay ungrant PROJECT USER [--expected-revision N]\n\
           dunlin-relay pair USER [--ttl MINUTES]\n\
           dunlin-relay devices USER\n\
           dunlin-relay revoke DEVICE_ID\n\
           dunlin-relay serve [--config PATH] [--db PATH] [--listen ADDR] [--harness CMD] [--web-root PATH]\n\
           dunlin-relay worker run --config PATH --credential-stdin\n\
           dunlin-relay mcp-oauth-legacy --help\n\
           dunlin-relay mcp-client-metadata --config PATH --provider ID\n\
           dunlin-relay local-metadata {{show|list|approve|rebind|revoke|grant|ungrant|check}} --db ABSOLUTE_EXISTING_DB --project-id ID [explicit flags]\n\
           dunlin-relay local-metadata --help\n\
           dunlin-relay host inspect-local --socket PATH --node-id ID --origin https://node.tail.ts.net --listen 127.0.0.1:8787\n\
           dunlin-relay setup [--name NAME] [--db PATH]\n\
         \n\
         Relay vault (HTTP keys; values stay in this host's Keychain):\n\
           dunlin-relay vault add ALIAS --host HOST --project PROJECT [--ttl MINUTES] [--writes] [--header NAME] [--prefix PREFIX] [--db PATH]\n\
           dunlin-relay vault list [--db PATH]\n\
           dunlin-relay vault remove ALIAS [--db PATH]\n\
           dunlin-relay vault grant ALIAS PROJECT [--db PATH]\n\
           dunlin-relay vault revoke ALIAS PROJECT [--db PATH]\n\
         \n\
         Defaults:\n\
           db       ~/Library/Application Support/Dunlin/relay.db (macOS)\n\
           listen   127.0.0.1:8787\n\
           harness  opencode acp"
    );
}

fn cmd_init(args: &[String]) -> Result<(), String> {
    let db = flag(args, "--db")
        .map(PathBuf::from)
        .unwrap_or_else(config::default_db);
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
        .create_pairing_code(&admin.id, 15 * 60)
        .map_err(|error| error.to_string())?;
    println!("Created {}", db.display());
    println!("Admin user: {} ({})", admin.name, admin.id);
    println!("Pairing code: {code}  (valid until {expires})");
    println!(
        "Pair a device with: dunlin-relay pair {} --db {}",
        admin.name,
        db.display()
    );
    println!("Serve with: dunlin-relay serve --db {}", db.display());
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
            let config = Config {
                db: flag(args, "--db")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("DUNLIN_RELAY_DB").map(PathBuf::from))
                    .unwrap_or_else(config::default_db),
                ..Config::default()
            };
            if role == "member" && management::host_accounts(&config) {
                return Err("This relay has host-level Git accounts. Run a separate relay service/OS account for non-admin members.".into());
            }
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
        _ => Err("usage: dunlin-relay user add NAME [--admin] | user list".into()),
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
            Err("usage: dunlin-relay project add NAME PATH [--user USER]... | project list".into())
        }
    }
}

fn cmd_grant(args: &[String], active: bool) -> Result<(), String> {
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
    if has_flag(args, "--expected-revision") && flag(args, "--expected-revision").is_none() {
        return Err("Expected grant revision is required".into());
    }
    let revision = if let Some(value) = flag(args, "--expected-revision") {
        let expected = value
            .parse::<u64>()
            .map_err(|_| "Invalid expected grant revision".to_string())?;
        db.set_native_grant(
            &project.id,
            &user.id,
            active,
            if expected == 0 { None } else { Some(expected) },
        )
        .map_err(|error| error.to_string())?
    } else {
        if active {
            db.grant(&project.id, &user.id)
        } else {
            db.revoke_grant(&project.id, &user.id)
        }
        .map_err(|error| error.to_string())?;
        db.native_grant_revision(&project.id, &user.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(0)
    };
    println!(
        "{} {} for {} · revision {}",
        if active { "Granted" } else { "Revoked" },
        project.name,
        user.name,
        revision
    );
    Ok(())
}

fn cmd_pair(args: &[String]) -> Result<(), String> {
    let db = open_store_for(args)?;
    let name = args
        .first()
        .ok_or_else(|| "pair needs a user name".to_string())?;
    let ttl = flag(args, "--ttl")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(15)
        .clamp(1, 15);
    let user = db
        .find_user_by_name(name)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Unknown user: {name}"))?;
    let (code, expires) = db
        .create_pairing_code(&user.id, ttl * 60)
        .map_err(|error| error.to_string())?;
    println!("Pairing code for {}: {code}", user.name);
    println!("Expires in {ttl} minutes ({expires}).");
    println!("Open Dunlin → Remote → Pair device and enter this code.");
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

fn cmd_vault(args: &[String]) -> Result<(), String> {
    use dunlin_domain::vault::{HttpMethod, Secret, SecretPolicy, normalize_ttl, now_secs};
    use dunlin_vault::store::{SecretStore, ServiceKeychainStore};

    let db = flag(args, "--db")
        .map(PathBuf::from)
        .unwrap_or_else(config::default_db);
    let relay_store = Store::open(&db).map_err(|error| error.to_string())?;
    let vault_store = dunlin_storage::Store::open(vault::vault_database(&db))
        .map_err(|error| format!("Could not open relay vault storage: {error}"))?;
    let secrets = ServiceKeychainStore::relay();
    let subcommand = args.first().map(String::as_str).unwrap_or("help");
    match subcommand {
        "add" => {
            let alias = relay_slug(
                args.get(1)
                    .ok_or_else(|| "vault add needs an alias".to_string())?,
            );
            if vault_store
                .secret_by_alias(&alias)
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(format!("vault alias {alias} already exists"));
            }
            let host = flag(args, "--host")
                .ok_or_else(|| "vault add needs --host API_HOST".to_string())?;
            let project_value = flag(args, "--project")
                .ok_or_else(|| "vault add needs --project PROJECT".to_string())?;
            let project = relay_store
                .list_projects()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|project| project.id == project_value || project.name == project_value)
                .ok_or_else(|| format!("relay project {project_value} was not found"))?;
            let ttl = flag(args, "--ttl")
                .map(|value| value.parse::<i64>())
                .transpose()
                .map_err(|_| "--ttl must be a number of minutes".to_string())?
                .unwrap_or(60)
                * 60;
            let ttl = normalize_ttl(ttl)?;
            let injection = vault_injection(args)?;
            let mut methods = vec![HttpMethod::Get, HttpMethod::Head];
            if has_flag(args, "--writes") {
                methods.extend([
                    HttpMethod::Post,
                    HttpMethod::Put,
                    HttpMethod::Patch,
                    HttpMethod::Delete,
                ]);
            }
            let now = now_secs();
            let id = format!("dunlin/{}", uuid::Uuid::now_v7().simple());
            let secret = Secret {
                id: id.clone(),
                namespace: "dunlin".into(),
                alias: alias.clone(),
                label: alias.clone(),
                description: flag(args, "--description").unwrap_or_default(),
                injection,
                allowed_hosts: vec![host],
                path_prefix: "/".into(),
                allowed_methods: methods,
                allowed_headers: vec!["Accept".into(), "Content-Type".into()],
                auto_approve: true,
                expires_at: Some(now + ttl),
                created_at: now,
                updated_at: now,
                last_used_at: None,
            };
            let policy = secret.policy(vec![project.id.clone()], Vec::new(), Vec::new());
            policy.validate()?;
            let policy_json = serde_json::to_string(&policy).map_err(|error| error.to_string())?;
            let value = read_vault_secret(args)?;
            if value.trim().is_empty() {
                return Err("the credential value is empty".into());
            }
            secrets
                .set(&id, value.trim())
                .map_err(|error| format!("Could not save the relay credential: {error}"))?;
            if let Err(error) = secrets.set_policy(&id, &policy_json) {
                let _ = secrets.delete(&id);
                return Err(format!(
                    "Could not save the relay credential policy: {error}"
                ));
            }
            if let Err(error) = vault_store.upsert_secret(&secret) {
                let _ = secrets.delete(&id);
                let _ = secrets.delete_policy(&id);
                return Err(format!("Could not save relay credential metadata: {error}"));
            }
            println!(
                "Added {alias} for project {} · expires in {} minutes · hosts {}",
                project.name,
                ttl / 60,
                secret.allowed_hosts.join(", ")
            );
            Ok(())
        }
        "list" => {
            for secret in vault_store.secrets().map_err(|error| error.to_string())? {
                let policy = secrets
                    .get_policy(&secret.id)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<SecretPolicy>(&raw).ok());
                match policy {
                    Some(policy) => println!(
                        "{} · projects {} · hosts {} · expires {}",
                        secret.alias,
                        policy.project_ids.join(","),
                        policy.allowed_hosts.join(","),
                        policy
                            .expires_at
                            .map(|expires_at| (expires_at - now_secs()).max(0).to_string() + "s")
                            .unwrap_or_else(|| "never".into())
                    ),
                    None => println!("{} · policy unavailable", secret.alias),
                }
            }
            Ok(())
        }
        "remove" => {
            let alias = relay_slug(
                args.get(1)
                    .ok_or_else(|| "vault remove needs an alias".to_string())?,
            );
            let secret = vault_store
                .secret_by_alias(&alias)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("vault alias {alias} was not found"))?;
            secrets
                .delete(&secret.id)
                .map_err(|error| format!("Could not remove the relay credential: {error}"))?;
            let _ = secrets.delete_policy(&secret.id);
            vault_store
                .delete_secret(&secret.id)
                .map_err(|error| error.to_string())?;
            println!("Removed {alias}");
            Ok(())
        }
        "grant" | "revoke" => {
            let alias = relay_slug(
                args.get(1)
                    .ok_or_else(|| format!("vault {subcommand} needs an alias"))?,
            );
            let project_value = args
                .get(2)
                .ok_or_else(|| format!("vault {subcommand} needs a project"))?;
            let project = relay_store
                .list_projects()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|project| project.id == *project_value || project.name == *project_value)
                .ok_or_else(|| format!("relay project {project_value} was not found"))?;
            let secret = vault_store
                .secret_by_alias(&alias)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("vault alias {alias} was not found"))?;
            let mut policy: SecretPolicy = secrets
                .get_policy(&secret.id)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .ok_or_else(|| "relay credential policy is unavailable".to_string())?;
            if subcommand == "grant" {
                if !policy.project_ids.contains(&project.id) {
                    policy.project_ids.push(project.id.clone());
                }
            } else {
                policy.project_ids.retain(|id| id != &project.id);
            }
            policy.validate()?;
            secrets
                .set_policy(
                    &secret.id,
                    &serde_json::to_string(&policy).map_err(|error| error.to_string())?,
                )
                .map_err(|error| {
                    format!("Could not update the relay credential policy: {error}")
                })?;
            println!("{} {alias} {}", subcommand, project.name);
            Ok(())
        }
        "help" | "-h" | "--help" => {
            println!(
                "dunlin-relay vault add ALIAS --host HOST --project PROJECT [--ttl MINUTES] [--writes] [--header NAME] [--prefix PREFIX] [--value-file PATH]\n\
                 dunlin-relay vault list\n\
                 dunlin-relay vault remove ALIAS\n\
                 dunlin-relay vault grant ALIAS PROJECT\n\
                 dunlin-relay vault revoke ALIAS PROJECT"
            );
            Ok(())
        }
        other => Err(format!("Unknown vault command: {other}")),
    }
}

fn vault_injection(args: &[String]) -> Result<dunlin_domain::vault::Injection, String> {
    use dunlin_domain::vault::Injection;

    let oauth_flags = ("--oauth-token-url", "--oauth-client-id");
    let aws_flags = ("--aws-access-key-id", "--aws-region", "--aws-service");
    let oauth_requested = has_flag(args, oauth_flags.0) || has_flag(args, oauth_flags.1);
    let aws_requested =
        has_flag(args, aws_flags.0) || has_flag(args, aws_flags.1) || has_flag(args, aws_flags.2);
    if oauth_requested && aws_requested {
        return Err("Choose either OAuth 2.0 or AWS SigV4 injection, not both".into());
    }
    if oauth_requested {
        let token_url = flag(args, oauth_flags.0)
            .ok_or_else(|| "OAuth 2.0 injection needs --oauth-token-url".to_string())?;
        let client_id = flag(args, oauth_flags.1)
            .ok_or_else(|| "OAuth 2.0 injection needs --oauth-client-id".to_string())?;
        return Ok(Injection::OAuth2 {
            token_url,
            client_id,
        });
    }
    if aws_requested {
        let access_key_id = flag(args, aws_flags.0)
            .ok_or_else(|| "AWS SigV4 injection needs --aws-access-key-id".to_string())?;
        let region = flag(args, aws_flags.1)
            .ok_or_else(|| "AWS SigV4 injection needs --aws-region".to_string())?;
        let service = flag(args, aws_flags.2)
            .ok_or_else(|| "AWS SigV4 injection needs --aws-service".to_string())?;
        return Ok(Injection::AwsSigV4 {
            access_key_id,
            region,
            service,
        });
    }
    Ok(Injection::Header {
        name: flag(args, "--header").unwrap_or_else(|| "Authorization".into()),
        prefix: flag(args, "--prefix").unwrap_or_else(|| "Bearer ".into()),
    })
}

fn relay_slug(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "key".into() } else { out }
}

fn read_vault_secret(args: &[String]) -> Result<String, String> {
    use std::io::{Read, Write};
    if let Some(path) = flag(args, "--value-file") {
        let mut value = String::new();
        std::fs::File::open(&path)
            .map_err(|error| format!("Could not open --value-file: {error}"))?
            .read_to_string(&mut value)
            .map_err(|error| format!("Could not read --value-file: {error}"))?;
        return Ok(value.lines().next().unwrap_or_default().to_string());
    }
    let echo_off = std::process::Command::new("stty")
        .arg("-echo")
        .status()
        .is_ok();
    eprint!("Credential value: ");
    let _ = std::io::stdout().flush();
    let mut value = String::new();
    std::io::stdin()
        .read_line(&mut value)
        .map_err(|error| format!("Could not read the credential value: {error}"))?;
    if echo_off {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
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
    if let Some(path) = flag(args, "--web-root") {
        config.web_root = Some(PathBuf::from(path));
    }
    config.apply_env();
    if let Some(root) = &config.web_root {
        let root = std::fs::canonicalize(root)
            .map_err(|error| format!("Could not open web build directory: {error}"))?;
        if !root.join("index.html").is_file() {
            return Err("Web build directory must contain index.html; build apps/web first".into());
        }
        config.web_root = Some(root);
    }
    let listen_address = config.listen_address()?;
    if !config.db.is_absolute() {
        config.db = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(&config.db);
    }
    management::private_dir(&management::root(&config))?;
    // This is the single-threaded startup boundary, before vault/runtime workers.
    unsafe {
        std::env::set_var("PATH", management::path(&config));
    }

    let store = Store::open(&config.db).map_err(|error| error.to_string())?;
    let relay_vault = match vault::RelayVault::start(&config.db) {
        Ok(vault) => Some(vault),
        Err(error) => {
            eprintln!("[relay] vault disabled · {error}");
            None
        }
    };
    if store
        .list_users()
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err(format!(
            "{} has no users. Run `dunlin-relay init --db {}` first.",
            config.db.display(),
            config.db.display()
        ));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let mut state = AppState::new_with_vault(store, config.clone(), relay_vault);
        state.coordination = Some(std::sync::Arc::new(
            dunlin_coordination::service::Service::start(
                &config.db.with_extension("coordination"),
            )?,
        ));
        let listener = tokio::net::TcpListener::bind(listen_address)
            .await
            .map_err(|error| format!("Could not listen on {}: {error}", config.listen))?;
        state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned".to_string())?
            .recover_interactive_sessions()
            .map_err(|error| error.to_string())?;
        linear::start_background(state.clone());
        jobs::start_background(state.clone())?;
        let remote_services = remote_services::Services::start(&state).await?;
        let interactive = state.interactive.clone();
        let worker_channels = state.worker_channels.clone();
        let app = api::router(state);
        eprintln!(
            "[relay] listening on {} · db {} · harness {}",
            config.listen,
            config.db.display(),
            config.harness
        );
        if config.sandbox {
            eprintln!("[relay] harness sandbox enabled · missing platform support fails closed");
        }
        eprintln!(
            "[relay] {} · no prompt logging",
            if config.store_transcripts {
                "metadata + transcript snapshots"
            } else {
                "metadata only"
            }
        );
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                interactive.shutdown().await;
                remote_services.shutdown().await;
                worker_channels.shutdown().await;
            })
            .await
            .map_err(|error| error.to_string())
    })
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
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
        .or_else(|| std::env::var_os("DUNLIN_RELAY_DB").map(PathBuf::from))
        .unwrap_or_else(config::default_db);
    Store::open(&path).map_err(|error| error.to_string())
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::vault::Injection;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn vault_injection_reads_exactly_one_cli_mode() {
        assert_eq!(
            vault_injection(&args(&["add", "key", "--host", "api.example.com"])).unwrap(),
            Injection::Header {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            }
        );
        assert_eq!(
            vault_injection(&args(&[
                "add",
                "key",
                "--header",
                "X-Api-Key",
                "--prefix",
                "",
            ]))
            .unwrap(),
            Injection::Header {
                name: "X-Api-Key".into(),
                prefix: String::new(),
            }
        );
        assert_eq!(
            vault_injection(&args(&[
                "add",
                "key",
                "--oauth-token-url",
                "https://auth.example.com/token",
                "--oauth-client-id",
                "client-1",
            ]))
            .unwrap(),
            Injection::OAuth2 {
                token_url: "https://auth.example.com/token".into(),
                client_id: "client-1".into(),
            }
        );
        assert_eq!(
            vault_injection(&args(&[
                "add",
                "key",
                "--aws-access-key-id",
                "AKIAIOSFODNN7EXAMPLE",
                "--aws-region",
                "us-east-1",
                "--aws-service",
                "execute-api",
            ]))
            .unwrap(),
            Injection::AwsSigV4 {
                access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
                region: "us-east-1".into(),
                service: "execute-api".into(),
            }
        );
    }

    #[test]
    fn vault_injection_rejects_partial_or_mixed_modes() {
        assert!(
            vault_injection(&args(&[
                "add",
                "key",
                "--oauth-token-url",
                "https://auth.example.com/token",
            ]))
            .is_err()
        );
        assert!(
            vault_injection(&args(&[
                "add",
                "key",
                "--aws-access-key-id",
                "AKIAIOSFODNN7EXAMPLE",
            ]))
            .is_err()
        );
        assert!(
            vault_injection(&args(&[
                "add",
                "key",
                "--oauth-token-url",
                "https://auth.example.com/token",
                "--oauth-client-id",
                "client-1",
                "--aws-access-key-id",
                "AKIAIOSFODNN7EXAMPLE",
                "--aws-region",
                "us-east-1",
                "--aws-service",
                "execute-api",
            ]))
            .is_err()
        );
    }

    #[test]
    fn native_grant_cli_cas_revoke_and_regrant_preserve_stale_authority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.db");
        let db = Store::open(&path).unwrap();
        let user = db.create_user("cli-member", "member").unwrap();
        let project = db.create_project("cli-project", "/fixture/cli").unwrap();
        let command = |revision: &str| {
            vec![
                "cli-project".to_string(),
                "cli-member".to_string(),
                "--expected-revision".to_string(),
                revision.to_string(),
                "--db".to_string(),
                path.to_str().unwrap().to_string(),
            ]
        };
        cmd_grant(&command("0"), true).unwrap();
        let old = db.capture_native_grant(&project.id, &user.id).unwrap();
        let compatible = vec![
            "cli-project".into(),
            "cli-member".into(),
            "--db".into(),
            path.to_str().unwrap().into(),
        ];
        cmd_grant(&compatible, true).unwrap();
        assert_eq!(db.capture_native_grant(&project.id, &user.id).unwrap(), old);
        assert!(cmd_grant(&command("0"), false).is_err());
        db.revalidate_native_grant(&old).unwrap();
        cmd_grant(&command("1"), false).unwrap();
        assert!(
            db.project_for_user(&project.id, &user.id)
                .unwrap()
                .is_none()
        );
        cmd_grant(&command("2"), true).unwrap();
        assert!(
            db.project_for_user(&project.id, &user.id)
                .unwrap()
                .is_some()
        );
        assert!(db.revalidate_native_grant(&old).is_err());
    }

    #[test]
    fn relay_slug_is_stable_and_safe() {
        assert_eq!(relay_slug("GitHub Work Key!"), "github-work-key");
        assert_eq!(relay_slug("---"), "key");
        assert_eq!(relay_slug(""), "key");
        assert!(relay_slug(&"A".repeat(200)).len() <= 48);
    }
}
