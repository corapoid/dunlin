//! Thin headless adapter. Local catalog reads are read-only; runs are fresh,
//! ephemeral sessions, not a second owner of the desktop's active session.
use dunlin_agent::HarnessConfig;
use dunlin_relay_client as relay;
use dunlin_runtime::run::{self, Cancellation, PermissionDecision};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const HELP: &str = "dunlin-cli — headless Dunlin client

Usage:
  dunlin-cli project list --db PATH [--json]
  dunlin-cli project list --remote CONNECTION [--json]
  dunlin-cli session list --db PATH [--project ID] [--json]
  dunlin-cli session list --remote CONNECTION --project ID [--json]
  dunlin-cli remote pair --url URL --connection NEW_FILE [--device NAME] [--json]
  dunlin-cli remote list --connection FILE [--json]
  dunlin-cli run --local --harness COMMAND --cwd PATH --prompt TEXT [--model ID]
  dunlin-cli run --remote CONNECTION --project ID --cwd REMOTE_PATH --prompt TEXT [--model ID]

Run options: --timeout SECONDS (default 180, max 3600), --json.
Tool permissions are denied by default. --permission ask requests per-use terminal approval.
Pairing reads one code from stdin and creates a private connection file, never overwriting it.
Runs create fresh sessions, do not load desktop defaults or attach its vault/MCP policies,
and do not write desktop history. A harness still has the OS permissions of this process.
Exit codes: 0 success; 1 operation/refusal; 2 usage; 124 deadline; 130 cancelled.";

#[derive(Debug)]
struct Failure {
    code: i32,
    message: String,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self { code: 1, message }
    }
}
fn usage(message: impl Into<String>) -> Failure {
    Failure {
        code: 2,
        message: message.into(),
    }
}

#[derive(Serialize, Deserialize)]
struct Connection {
    url: String,
    token: String,
    user: dunlin_relay_protocol::User,
    device_id: String,
}

struct Options {
    values: HashMap<String, String>,
    json: bool,
    local: bool,
}
impl Options {
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self, Failure> {
        let mut result = Self {
            values: HashMap::new(),
            json: false,
            local: false,
        };
        let mut iter = args.iter();
        while let Some(key) = iter.next() {
            if !allowed.contains(&key.as_str()) {
                return Err(usage(format!("Unknown option: {key}")));
            }
            if result.values.contains_key(key) {
                return Err(usage(format!("Duplicate option: {key}")));
            }
            let value = if key == "--json" {
                result.json = true;
                String::new()
            } else if key == "--local" {
                result.local = true;
                String::new()
            } else {
                iter.next()
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| usage(format!("Missing value for {key}")))?
                    .clone()
            };
            result.values.insert(key.clone(), value);
        }
        Ok(result)
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    fn required(&self, key: &str) -> Result<&str, Failure> {
        self.get(key)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| usage(format!("Required option: {key}")))
    }
}

fn read_connection(path: &str) -> Result<Connection, Failure> {
    let file =
        std::fs::File::open(path).map_err(|_| "Could not open connection file".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not inspect connection file".to_string())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err("Invalid connection file".to_string().into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "Connection file must not be accessible to group or other users (use chmod 600)"
                    .to_string()
                    .into(),
            );
        }
    }
    let connection: Connection =
        serde_json::from_reader(file).map_err(|_| "Invalid connection file".to_string())?;
    relay::validate_relay_url(&connection.url)?;
    if connection.token.is_empty() {
        return Err("Connection file has no token".to_string().into());
    }
    Ok(connection)
}

fn save_connection(path: &str, connection: &Connection) -> Result<(), Failure> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Could not create connection file; it must not already exist".to_string())?;
    serde_json::to_writer(&mut file, connection)
        .map_err(|_| "Could not write connection file".to_string())?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save connection file".to_string())?;
    Ok(())
}

fn catalog(options: &Options, sessions: bool) -> Result<Value, Failure> {
    match (options.get("--db"), options.get("--remote")) {
        (Some(path), None) => {
            let store = dunlin_storage::Store::read_only(path)
                .map_err(|_| "Could not open existing desktop database read-only".to_string())?;
            if sessions {
                serde_json::to_value(
                    store
                        .sessions(options.get("--project"))
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string().into())
            } else {
                serde_json::to_value(store.projects().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string().into())
            }
        }
        (None, Some(path)) => {
            let project = if sessions {
                Some(options.required("--project")?)
            } else {
                None
            };
            let connection = read_connection(path)?;
            if let Some(project) = project {
                serde_json::to_value(
                    relay::sessions(&connection.url, &connection.token, project)
                        .map_err(|_| "Could not list remote sessions".to_string())?,
                )
                .map_err(|e| e.to_string().into())
            } else {
                serde_json::to_value(
                    relay::projects(&connection.url, &connection.token)
                        .map_err(|_| "Could not list remote projects".to_string())?,
                )
                .map_err(|e| e.to_string().into())
            }
        }
        _ => Err(usage(
            "Choose exactly one backend: --db PATH or --remote CONNECTION",
        )),
    }
}

fn run_prompt(options: &Options) -> Result<(Value, i32), Failure> {
    let prompt = options.required("--prompt")?;
    let cwd = options.required("--cwd")?;
    let timeout = options
        .get("--timeout")
        .unwrap_or("180")
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=3600).contains(n))
        .ok_or_else(|| usage("Timeout must be an integer between 1 and 3600"))?;
    let ask = match options.get("--permission").unwrap_or("deny") {
        "deny" => false,
        "ask" => true,
        _ => return Err(usage("Permission mode must be deny or ask")),
    };
    if ask && !io::stdin().is_terminal() {
        return Err(usage("--permission ask requires an interactive terminal"));
    }
    let mut config = match (options.local, options.get("--remote")) {
        (true, None) => {
            if options.get("--project").is_some() {
                return Err(usage("--project is only valid for remote runs"));
            }
            let command = options.required("--harness")?;
            let cwd = std::fs::canonicalize(cwd)
                .map_err(|_| usage("Local working directory does not exist"))?;
            if !cwd.is_dir() {
                return Err(usage("Local working directory is not a directory"));
            }
            HarnessConfig::new(command, cwd)
        }
        (false, Some(path)) => {
            if options.get("--harness").is_some() {
                return Err(usage(
                    "Remote harness selection belongs to the server, not --harness",
                ));
            }
            let project = options.required("--project")?;
            if !Path::new(cwd).is_absolute() {
                return Err(usage("Remote working directory must be absolute"));
            }
            let connection = read_connection(path)?;
            HarnessConfig::remote(
                connection.url,
                connection.token,
                project,
                PathBuf::from(cwd),
            )
        }
        _ => {
            return Err(usage(
                "Choose exactly one backend: --local or --remote CONNECTION",
            ));
        }
    };
    config.client_name = "Dunlin CLI".into();
    config.default_model = options.get("--model").map(str::to_owned);
    let cancel = Cancellation::default();
    let signal = cancel.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not initialize signal handling".to_string())?;
    std::thread::spawn(move || {
        runtime.block_on(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.cancel();
            }
        })
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout);
    let outcome = run::execute(
        config,
        prompt,
        Duration::from_secs(timeout),
        &cancel,
        |title, _| {
            if ask {
                eprint!("Permission requested: {title}\nAllow this operation once? [y/N] ");
                let _ = io::stderr().flush();
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let mut answer = String::new();
                    let allowed = io::stdin().read_line(&mut answer).is_ok()
                        && matches!(answer.trim(), "y" | "Y");
                    let _ = tx.send(allowed);
                });
                while !cancel.is_cancelled() && std::time::Instant::now() < deadline {
                    match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(true) => return PermissionDecision::AllowOnce,
                        Ok(false) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            }
            PermissionDecision::Deny
        },
    )
    .map_err(|message| Failure {
        code: if cancel.is_cancelled() {
            130
        } else if message == "Run deadline exceeded" {
            124
        } else {
            1
        },
        message,
    })?;
    let code = if outcome.stop_reason == "end_turn" {
        0
    } else {
        1
    };
    Ok((
        serde_json::to_value(outcome).map_err(|e| e.to_string())?,
        code,
    ))
}

fn execute(args: &[String]) -> Result<(Value, bool, i32), Failure> {
    let (command, count) = match args {
        [a, b, ..] if a == "project" && b == "list" => ("projects", 2),
        [a, b, ..] if a == "session" && b == "list" => ("sessions", 2),
        [a, b, ..] if a == "remote" && b == "pair" => ("pair", 2),
        [a, b, ..] if a == "remote" && b == "list" => ("remotes", 2),
        [a, ..] if a == "run" => ("run", 1),
        _ => return Err(usage("Unknown command; use --help")),
    };
    let allowed: &[&str] = match command {
        "projects" => &["--db", "--remote", "--json"],
        "sessions" => &["--db", "--remote", "--project", "--json"],
        "pair" => &["--url", "--connection", "--device", "--json"],
        "remotes" => &["--connection", "--json"],
        _ => &[
            "--local",
            "--remote",
            "--harness",
            "--cwd",
            "--prompt",
            "--project",
            "--model",
            "--timeout",
            "--permission",
            "--json",
        ],
    };
    let options = Options::parse(&args[count..], allowed)?;
    let (value, code) = match command {
        "projects" | "sessions" => (catalog(&options, command == "sessions")?, 0),
        "remotes" => {
            let connection = read_connection(options.required("--connection")?)?;
            (
                json!([{"url":connection.url, "user":connection.user, "device_id":connection.device_id}]),
                0,
            )
        }
        "pair" => {
            let url = options.required("--url")?;
            relay::validate_relay_url(url)?;
            let path = options.required("--connection")?;
            if Path::new(path).symlink_metadata().is_ok() {
                return Err(usage("Connection file already exists"));
            }
            if io::stdin().is_terminal() {
                eprint!("Pairing code: ");
                let _ = io::stderr().flush();
            }
            let mut code = String::new();
            use std::io::Read;
            io::stdin()
                .lock()
                .take(128)
                .read_line(&mut code)
                .map_err(|_| "Could not read pairing code".to_string())?;
            if code.trim().is_empty() || code.len() > 65 {
                return Err(usage("Invalid pairing code"));
            }
            let paired =
                relay::pair(url, &code, options.get("--device").unwrap_or("Dunlin CLI"))
                    .map_err(|_| "Pairing failed; check the code and relay address".to_string())?;
            let connection = Connection {
                url: url.into(),
                token: paired.token,
                user: paired.user,
                device_id: paired.device_id,
            };
            save_connection(path, &connection)?;
            (
                json!({"url":url, "user":connection.user, "device_id":connection.device_id, "connection":path}),
                0,
            )
        }
        _ => run_prompt(&options)?,
    };
    Ok((value, options.json, code))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{HELP}");
        return;
    }
    if args.len() == 1 && matches!(args[0].as_str(), "--version" | "-V") {
        println!("dunlin-cli {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let code = match execute(&args) {
        Ok((value, json, code)) => {
            if !json && value.get("text").is_some() {
                println!("{}", value["text"].as_str().unwrap_or_default());
            } else if json {
                println!("{value}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).expect("JSON value")
                );
            }
            code
        }
        Err(error) => {
            eprintln!("dunlin-cli: {}", error.message);
            error.code
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_rejects_unknown_and_duplicate_flags() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(Options::parse(&args(&["--json", "--json"]), &["--json"]).is_err());
        assert!(Options::parse(&args(&["--db"]), &["--db"]).is_err());
        assert!(Options::parse(&args(&["--typo"]), &["--json"]).is_err());
        assert_eq!(execute(&args(&["project", "list"])).unwrap_err().code, 2);
    }

    #[test]
    fn connection_files_are_private_never_overwritten_and_not_printed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connection.json");
        let connection = Connection {
            url: "https://relay.tail123.ts.net".into(),
            token: "test-secret".into(),
            user: dunlin_relay_protocol::User {
                id: "u".into(),
                name: "admin".into(),
                role: "admin".into(),
            },
            device_id: "d".into(),
        };
        save_connection(path.to_str().unwrap(), &connection).unwrap();
        assert!(save_connection(path.to_str().unwrap(), &connection).is_err());
        let (value, _, _) = execute(&[
            "remote".into(),
            "list".into(),
            "--connection".into(),
            path.to_str().unwrap().into(),
            "--json".into(),
        ])
        .unwrap();
        assert!(!value.to_string().contains("test-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
