//! Administrator-only host management. Commands come from a fixed catalog.
use crate::{
    api::{ApiError, AppState, AuthUser},
    config::Config,
};
use axum::{
    Json,
    extract::{Path as RoutePath, State},
    http::StatusCode,
};
use dunlin_workspace::server::{HARNESSES, Harness, Job, Server};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub mod copy;

#[derive(Default)]
pub struct Management {
    pub jobs: HashMap<String, Running>,
    pub copies: HashMap<String, copy::Pending>,
}
pub struct Running {
    pub owner: String,
    pub created: Instant,
    pub job: Arc<Mutex<Job>>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    name: String,
    default_harness: String,
    host_accounts: bool,
    isolated_agents: bool,
}

pub fn root(config: &Config) -> PathBuf {
    config.db.with_file_name("server")
}
pub fn tools(config: &Config) -> PathBuf {
    root(config).join("tools")
}
pub fn isolated_agents(config: &Config) -> bool {
    if cfg!(target_os = "linux") {
        return true;
    }
    match std::fs::read(root(config).join("settings.json")) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map(|s| s.isolated_agents)
            .unwrap_or(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}
pub fn agent_home(config: &Config, user: &str) -> PathBuf {
    if !isolated_agents(config)
        && let Some(home) = std::env::var_os("HOME")
    {
        return home.into();
    }
    root(config).join("agent-homes").join(user)
}
fn settings(config: &Config) -> Settings {
    std::fs::read(root(config).join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn write_settings(config: &Config, value: &Settings) -> Result<(), String> {
    private_dir(&root(config))?;
    let temp = root(config).join(format!("settings-{}.json", uuid::Uuid::now_v7()));
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, root(config).join("settings.json")).map_err(|e| e.to_string())
}
pub fn private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    if std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Server data directories must not be symbolic links".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn admin(auth: &AuthUser) -> Result<(), ApiError> {
    if auth.user.role != "admin" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Server administration requires an administrator",
        ));
    }
    Ok(())
}
pub fn path(config: &Config) -> std::ffi::OsString {
    let mut paths = vec![tools(config).join("bin"), root(config).join("node/bin")];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).filter(|p| p.is_absolute()));
    }
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/bin"));
    }
    std::env::join_paths(paths).unwrap_or_default()
}
pub fn executable(config: &Config, program: &str) -> Option<PathBuf> {
    std::env::split_paths(&path(config))
        .map(|p| p.join(program))
        .find(|p| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                p.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                p.is_file()
            }
        })
}
pub fn harness(config: &Config) -> String {
    let selected = settings(config).default_harness;
    HARNESSES
        .iter()
        .find(|s| s.0 == selected)
        .map(|s| s.2.to_owned())
        .unwrap_or_else(|| config.harness.clone())
}
pub fn host_accounts(config: &Config) -> bool {
    match std::fs::read(root(config).join("settings.json")) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map(|s| s.host_accounts)
            .unwrap_or(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}
pub async fn info(auth: AuthUser, State(state): State<AppState>) -> Result<Json<Server>, ApiError> {
    let settings = settings(&state.config);
    Ok(Json(Server {
        isolated_agents: isolated_agents(&state.config),
        role: auth.user.role,
        name: if settings.name.is_empty() { state.config.name.clone() } else { settings.name },
        version: env!("CARGO_PKG_VERSION").into(), os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into(),
        harnesses: HARNESSES.iter().map(|(id, name, command)| Harness {
            id: (*id).into(), name: (*name).into(), installed: executable(&state.config, command.split_whitespace().next().unwrap()).is_some(), command: (*command).into(),
        }).collect(),
        default_harness: harness(&state.config),
        account_scope: "Accounts belong to the relay OS account. Only administrators can authorize or use account-management operations; do not share this relay with untrusted users.".into(),
        sandbox_available: if cfg!(target_os = "macos") { Path::new("/usr/bin/sandbox-exec").is_file() } else { executable(&state.config, "bwrap").is_some() },
    }))
}

#[derive(Deserialize)]
pub struct Update {
    pub name: Option<String>,
    pub harness: Option<String>,
}
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(update): Json<Update>,
) -> Result<StatusCode, ApiError> {
    admin(&auth)?;
    let _lock = state
        .management
        .lock()
        .map_err(|_| ApiError::internal("Management lock poisoned"))?;
    let mut value = settings(&state.config);
    if let Some(name) = update.name {
        if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(ApiError::bad_request("Invalid server name"));
        }
        value.name = name.trim().into();
    }
    if let Some(id) = update.harness {
        if !HARNESSES.iter().any(|s| s.0 == id) {
            return Err(ApiError::bad_request("Unknown harness"));
        }
        value.default_harness = id;
    }
    write_settings(&state.config, &value).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub action: String,
    pub target: String,
}

pub async fn start(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(action): Json<Action>,
) -> Result<Json<Job>, ApiError> {
    admin(&auth)?;
    let mut management = state
        .management
        .lock()
        .map_err(|_| ApiError::internal("Management lock poisoned"))?;
    management
        .jobs
        .retain(|_, r| r.created.elapsed() < Duration::from_secs(1800));
    if management
        .jobs
        .values()
        .any(|r| r.job.lock().is_ok_and(|j| !j.finished))
    {
        return Err(ApiError::conflict("Another server action is running"));
    }
    let mut command;
    let config = &state.config;
    let kind = format!("{}:{}", action.action, action.target);
    match (action.action.as_str(), action.target.as_str()) {
        ("install", id) if HARNESSES.iter().any(|s| s.0 == id) => {
            if state.tools_busy() {
                return Err(ApiError::conflict(
                    "Close active relay agent connections and terminals before installing or updating tools",
                ));
            }
            command = tokio::process::Command::new("/bin/bash");
            command.args([
                "-c",
                include_str!("../../../scripts/install-harness.sh"),
                "dunlin-install",
                id,
            ]);
            command.env("DUNLIN_TOOLS", tools(config));
        }
        ("login", "github" | "gitlab") => {
            // Host CLI keyrings are not a per-Dunlin-user security boundary.
            // Refuse mixed-trust deployments instead of sharing credentials silently.
            let mut value = settings(config);
            let store = state
                .store
                .lock()
                .map_err(|_| ApiError::internal("Store lock poisoned"))?;
            if store.list_users()?.iter().any(|user| user.role != "admin") {
                return Err(ApiError::conflict(
                    "Remote host-account authorization requires an administrator-only relay. Use a separate relay OS account/service for untrusted users.",
                ));
            }
            drop(store);
            let github = action.target == "github";
            let program = if github { "gh" } else { "glab" };
            let program = executable(config, program).ok_or_else(|| {
                ApiError::conflict(
                    "Install gh/glab on the server first (the relay installer offers both)",
                )
            })?;
            ensure_keyring(config)?;
            value.host_accounts = true;
            write_settings(config, &value).map_err(ApiError::internal)?;
            command = tokio::process::Command::new(program);
            if github {
                command = tokio::process::Command::new("/bin/sh");
                command.args(["-c", "gh auth login --hostname github.com --web --git-protocol https && gh auth setup-git --hostname github.com"]);
            } else {
                command.args([
                    "auth",
                    "login",
                    "--hostname",
                    "gitlab.com",
                    "--device",
                    "--git-protocol",
                    "https",
                    "--setup-git-credential-helper",
                ]);
            }
            command.env("BROWSER", "false").env("GH_BROWSER", "false");
        }
        ("status", "github" | "gitlab") => {
            command = tokio::process::Command::new(if action.target == "github" {
                "gh"
            } else {
                "glab"
            });
            command.args([
                "auth",
                "status",
                "--hostname",
                if action.target == "github" {
                    "github.com"
                } else {
                    "gitlab.com"
                },
            ]);
        }
        ("logout", "github" | "gitlab") => {
            command = tokio::process::Command::new(if action.target == "github" {
                "gh"
            } else {
                "glab"
            });
            command.args([
                "auth",
                "logout",
                "--hostname",
                if action.target == "github" {
                    "github.com"
                } else {
                    "gitlab.com"
                },
            ]);
        }
        _ => return Err(ApiError::bad_request("Unsupported server action")),
    }
    command
        .env("PATH", path(config))
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
    for key in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_ACCESS_TOKEN",
        "OAUTH_TOKEN",
        "GH_DEBUG",
        "GLAB_DEBUG_HTTP",
        "CI",
        "GITLAB_CI",
    ] {
        command.env_remove(key);
    }
    let id = uuid::Uuid::now_v7().to_string();
    let job = Job {
        id: id.clone(),
        kind,
        output: String::new(),
        finished: false,
        success: false,
    };
    let shared = Arc::new(Mutex::new(job.clone()));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    management.jobs.insert(
        id,
        Running {
            owner: auth.user.id,
            created: Instant::now(),
            job: shared.clone(),
            cancel: cancel.clone(),
        },
    );
    drop(management);
    tokio::spawn(run(command, shared, cancel));
    Ok(Json(job))
}

fn ensure_keyring(config: &Config) -> Result<(), ApiError> {
    let _ = config;
    #[cfg(target_os = "linux")]
    {
        // gh/glab otherwise silently fall back to cleartext configuration files.
        use dunlin_vault::store::{SecretStore, ServiceKeychainStore};
        let store = ServiceKeychainStore::relay();
        let id = format!("probe-{}", uuid::Uuid::now_v7());
        store.set(&id, "dunlin-keyring-probe").map_err(|_| ApiError::conflict("Unlock a Secret Service keyring for the relay service before authorizing Git accounts; plaintext fallback is not allowed"))?;
        store.delete(&id).map_err(ApiError::internal)?;
    }
    Ok(())
}

async fn run(
    mut command: tokio::process::Command,
    job: Arc<Mutex<Job>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let result = async {
        let mut child = command.spawn().map_err(|e| format!("Could not start server operation: {e}"))?;
        let pid = child.id();
        async fn collect(mut pipe: impl tokio::io::AsyncRead + Unpin, job: Arc<Mutex<Job>>) {
            let mut buf = [0u8; 2048];
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 { break; }
                if let Ok(mut job) = job.lock()
                    && job.output.len() < 32768 {
                        job.output.extend(String::from_utf8_lossy(&buf[..n]).chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')));
                }
            }
        }
        let out = tokio::spawn(collect(child.stdout.take().ok_or("No operation output")?, job.clone()));
        let err = tokio::spawn(collect(child.stderr.take().ok_or("No operation output")?, job.clone()));
        let status = tokio::select! {
            status = child.wait() => status.map_err(|e| e.to_string()),
            _ = async { loop { if cancel.load(std::sync::atomic::Ordering::Relaxed) { break; } tokio::time::sleep(Duration::from_millis(100)).await; } } => Err("Operation cancelled; inspect server state before retrying".into()),
            _ = tokio::time::sleep(Duration::from_secs(900)) => Err("Operation expired after 15 minutes".into()),
        };
        #[cfg(unix)] if let Some(pid) = pid { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        let _ = child.kill().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), async { let _ = out.await; let _ = err.await; }).await;
        status.map(|s| s.success())
    }.await;
    if let Ok(mut job) = job.lock() {
        job.finished = true;
        match result {
            Ok(success) => job.success = success,
            Err(error) => {
                job.output.push('\n');
                job.output.push_str(&error);
            }
        }
    }
}

pub async fn job(
    auth: AuthUser,
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Job>, ApiError> {
    admin(&auth)?;
    let management = state
        .management
        .lock()
        .map_err(|_| ApiError::internal("Management lock poisoned"))?;
    let running = management
        .jobs
        .get(&id)
        .filter(|r| r.owner == auth.user.id && r.created.elapsed() < Duration::from_secs(1800))
        .ok_or_else(ApiError::not_found)?;
    let job = running
        .job
        .lock()
        .map_err(|_| ApiError::internal("Job lock poisoned"))?
        .clone();
    Ok(Json(job))
}
pub async fn cancel(
    auth: AuthUser,
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Result<StatusCode, ApiError> {
    admin(&auth)?;
    let management = state
        .management
        .lock()
        .map_err(|_| ApiError::internal("Management lock poisoned"))?;
    let running = management
        .jobs
        .get(&id)
        .filter(|r| r.owner == auth.user.id)
        .ok_or_else(ApiError::not_found)?;
    running
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(StatusCode::ACCEPTED)
}

pub fn setup(args: &[String]) -> Result<(), String> {
    let mut config = Config::default();
    if let Some(db) = crate::flag(args, "--db") {
        config.db = db.into();
    }
    let name = crate::flag(args, "--name").unwrap_or_else(|| "Dunlin server".into());
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err("Invalid server name".into());
    }
    let mut value = settings(&config);
    value.name = name;
    value.isolated_agents = true;
    write_settings(&config, &value)?;
    println!(
        "Server name saved; dedicated agent homes enabled. Legacy host credentials/history are retained but are not copied; reauthorize agents and start new conversations if upgrading. Run dunlin-relay pair admin for a new 15-minute pairing code."
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneProject {
    provider: String,
    repository: String,
}
pub async fn clone_project(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CloneProject>,
) -> Result<Json<crate::store::Project>, ApiError> {
    admin(&auth)?;
    if !matches!(request.provider.as_str(), "github" | "gitlab")
        || request.repository.len() > 500
        || request.repository.split('/').count() < 2
        || !request
            .repository
            .split('/')
            .all(dunlin_workspace::server::component)
    {
        return Err(ApiError::bad_request(
            "Use owner/repository or group/project, without credentials or a URL",
        ));
    }
    let permit = state.terminal_permit()?;
    let destination = root(&state.config).join("projects");
    private_dir(&destination).map_err(ApiError::internal)?;
    let project = crate::api::run_blocking(move || {
        let _permit = permit;
        let github = request.provider == "github";
        let program = executable(&state.config, if github { "gh" } else { "glab" })
            .ok_or("Install the hosting CLI on the server first")?;
        let mut command = std::process::Command::new(program);
        command
            .env("PATH", path(&state.config))
            .env("GH_PROMPT_DISABLED", "1")
            .env("GLAB_PROMPT_DISABLED", "1")
            .env("GH_HOST", "github.com")
            .env("GITLAB_HOST", "gitlab.com");
        for key in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GITLAB_TOKEN",
            "GITLAB_ACCESS_TOKEN",
            "OAUTH_TOKEN",
            "GH_DEBUG",
            "GLAB_DEBUG_HTTP",
        ] {
            command.env_remove(key);
        }
        let path = dunlin_workspace::github::imports::clone_with(
            command,
            if github { "github.com" } else { "gitlab.com" },
            &request.repository,
            &destination,
        )?;
        let name = request.repository.replace('/', "-");
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned".to_string())?;
        let project = store
            .create_project(&name, path.to_str().ok_or("Invalid cloned project path")?)
            .map_err(|e| {
                format!(
                    "Clone completed at {}; registration failed: {e}",
                    path.display()
                )
            })?;
        store
            .grant(&project.id, &auth.user.id)
            .map_err(|e| e.to_string())?;
        Ok(project)
    })
    .await?;
    Ok(Json(project))
}
