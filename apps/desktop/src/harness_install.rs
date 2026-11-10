//! Explicit onboarding installs: curated packages, app-owned prefixes, no shell
//! or system-global writes. Detection and selecting a row never call this code.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Control {
    cancelled: AtomicBool,
    pid: Mutex<Option<u32>>,
}
impl Control {
    /// Stop our owned npm process group synchronously, including on app quit.
    /// The mutex closes the spawn/register and reap/cancel races.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let pid = self.pid.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(unix)]
        if let Some(pid) = *pid {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        #[cfg(not(unix))]
        let _ = pid;
    }
    fn run(&self, mut command: Command, timeout: Duration) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = {
            let mut pid = self.pid.lock().unwrap_or_else(|e| e.into_inner());
            if self.cancelled.load(Ordering::Acquire) {
                return Err("Installation cancelled".into());
            }
            let child = command
                .spawn()
                .map_err(|e| format!("Cannot start npm: {e}. Install Node.js/npm, then retry."))?;
            *pid = Some(child.id());
            child
        };
        let started = Instant::now();
        loop {
            let cancelled = self.cancelled.load(Ordering::Acquire);
            if cancelled || started.elapsed() > timeout {
                self.cancel();
                let mut pid = self.pid.lock().unwrap_or_else(|e| e.into_inner());
                let _ = child.kill();
                let _ = child.wait();
                *pid = None;
                return Err(if cancelled {
                    "Installation cancelled"
                } else {
                    "Adapter installation timed out. Check your connection and retry."
                }
                .into());
            }
            let status = {
                let mut pid = self.pid.lock().unwrap_or_else(|e| e.into_inner());
                match child.try_wait() {
                    Ok(status) => {
                        if status.is_some() {
                            *pid = None;
                        }
                        status
                    }
                    Err(error) => {
                        #[cfg(unix)]
                        unsafe {
                            libc::kill(-(child.id() as i32), libc::SIGTERM);
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        *pid = None;
                        return Err(format!("Could not monitor installation: {error}"));
                    }
                }
            };
            match status {
                Some(status) if status.success() => return Ok(()),
                Some(status) => {
                    return Err(format!(
                        "npm could not install the adapter ({status}). Check your Internet connection and Node.js/npm installation, then retry."
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

pub(crate) struct Recipe {
    pub name: &'static str,
    pub directory: &'static str,
    pub package: &'static str,
    pub executable: &'static str,
}

// Official npm releases checked 2026-11-10. Pin the top-level adapter, not a
// moving dist-tag; dependency lifecycle scripts are deliberately disabled.
pub(crate) const RECIPES: &[Recipe] = &[
    Recipe {
        name: "Claude Code",
        directory: "claude-code",
        package: "@agentclientprotocol/claude-agent-acp@0.103.0",
        executable: "claude-agent-acp",
    },
    Recipe {
        name: "OpenAI Codex",
        directory: "codex",
        package: "@agentclientprotocol/codex-acp@2.6.0",
        executable: "codex-acp",
    },
];

pub(crate) fn recipe(name: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.name == name)
}
pub(crate) fn prefix(recipe: &Recipe) -> Result<PathBuf, String> {
    let data = crate::app::data_dir().ok_or("Dunlin's data directory is unavailable")?;
    if !data.is_absolute() {
        return Err("Dunlin's data directory must be absolute".into());
    }
    Ok(data.join("adapters").join(recipe.directory))
}
pub(crate) fn managed_bins() -> Vec<PathBuf> {
    RECIPES
        .iter()
        .filter_map(|r| prefix(r).ok().map(|p| p.join("bin")))
        .collect()
}

fn install_command(recipe: &Recipe, prefix: &Path) -> Command {
    let mut command = crate::harness_discovery::local_command("npm");
    command
        .current_dir(prefix)
        .args(["install", "--global", "--prefix"])
        .arg(prefix)
        .args([
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--registry=https://registry.npmjs.org/",
        ])
        .arg("--cache")
        .arg(prefix.join("cache"))
        .arg(recipe.package)
        .env("CI", "1")
        .env("NO_UPDATE_NOTIFIER", "1")
        .env("npm_config_fetch_retries", "1")
        .env("npm_config_fetch_timeout", "60000")
        .env("NPM_CONFIG_USERCONFIG", "/dev/null")
        .env_remove("NODE_OPTIONS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(crate) fn install(
    recipe: &Recipe,
    prefix: &Path,
    cancelled: Arc<Control>,
) -> Result<(), String> {
    if cancelled.cancelled.load(Ordering::Acquire) {
        return Err("Installation cancelled".into());
    }
    let output = crate::providers::command_output(
        &mut crate::harness_discovery::local_command("node").arg("--version"),
        Duration::from_secs(5),
    )
    .map_err(|_| {
        "Node.js 22 or newer and npm are required. Install Node.js, then retry.".to_string()
    })?;
    let major = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|n| n.parse::<u32>().ok());
    if !output.status.success() || major.is_none_or(|v| v < 22) {
        return Err("Node.js 22 or newer is required. Update Node.js, then retry.".into());
    }
    std::fs::create_dir_all(prefix)
        .map_err(|e| format!("Cannot create the adapter directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(prefix, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    cancelled.run(install_command(recipe, prefix), Duration::from_secs(240))?;
    if !prefix.join("bin").join(recipe.executable).is_file() {
        return Err(
            "npm finished without creating the adapter executable. Retry installation.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_before_spawn_never_executes_the_installer() {
        let control = Control::default();
        control.cancel();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("must-not-exist");
        let mut command = Command::new("/usr/bin/touch");
        command.arg(&marker);
        assert!(control.run(command, Duration::from_secs(1)).is_err());
        assert!(!marker.exists());
    }
    #[test]
    fn installs_only_curated_pinned_adapters_into_the_given_prefix_without_scripts_or_a_shell() {
        assert!(recipe("untrusted; echo something").is_none());
        for recipe in RECIPES {
            let path = Path::new("/private/adapter fixture");
            let command = install_command(recipe, path);
            let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy()).collect();
            assert!(args.contains(&"--ignore-scripts".into()));
            assert!(args.contains(&path.to_string_lossy()));
            assert!(args.contains(&recipe.package.into()));
            assert_eq!(command.get_current_dir(), Some(path));
            assert!(!recipe.package.ends_with("@latest") && !recipe.package.ends_with("@preview"));
        }
    }
}
