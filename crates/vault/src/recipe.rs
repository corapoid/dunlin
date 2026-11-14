//! Fixed CLI recipes. The command template is user-owned; the agent only fills
//! declared parameters and never gets the credential value.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::access::LiveAccess;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecipeInput {
    pub secret: String,
    pub recipe: String,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecipeOutput {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub timed_out: bool,
}

#[derive(Clone)]
pub struct RecipePlan {
    pub program: String,
    pub args: Vec<String>,
    pub env_var: String,
    pub secret: Zeroizing<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub program_hash: Option<String>,
}

impl std::fmt::Debug for RecipePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecipePlan")
            .field("program", &self.program)
            .field("argument_count", &self.args.len())
            .field("env_var", &self.env_var)
            .field("secret", &"[REDACTED]")
            .field("cwd", &self.cwd)
            .field("timeout", &self.timeout)
            .field("program_hash", &self.program_hash)
            .finish()
    }
}

// Keep the leader unreaped until group cleanup and watchdog shutdown. Otherwise
// an exited parent's PID/PGID may be reused while inherited pipes are still open.
struct OwnedChild(Child, bool);
impl OwnedChild {
    fn reap(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let result = self.0.wait();
        self.1 = true;
        result
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        #[cfg(unix)]
        kill_owned_group(self.0.id());
        #[cfg(not(unix))]
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(unix)]
fn exit_pending(pid: u32) -> std::io::Result<bool> {
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
#[cfg(unix)]
fn kill_owned_group(pid: u32) {
    if exit_pending(pid).is_ok() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

pub trait RecipeExecutor: Send + Sync {
    fn execute(&self, plan: &RecipePlan) -> Result<RecipeOutput, String>;

    /// Executors must explicitly support cancellation before accepting guarded
    /// requests. A third-party executor cannot silently bypass the kill switch.
    fn execute_guarded(
        &self,
        plan: &RecipePlan,
        access: Option<LiveAccess>,
    ) -> Result<RecipeOutput, String> {
        if access.is_some() {
            return Err("The recipe executor does not support live authority".into());
        }
        self.execute(plan)
    }
}

#[cfg(target_os = "linux")]
mod isolated;
#[cfg(target_os = "linux")]
pub use isolated::LinuxRecipeExecutor;

mod redaction;
pub(crate) use redaction::redact_output;
mod remote;
pub use remote::{RemoteRecipeExecutor, RemoteRecipePlan};

pub struct CommandExecutor;

impl RecipeExecutor for CommandExecutor {
    fn execute(&self, plan: &RecipePlan) -> Result<RecipeOutput, String> {
        self.execute_inner(plan, None)
    }

    fn execute_guarded(
        &self,
        plan: &RecipePlan,
        access: Option<LiveAccess>,
    ) -> Result<RecipeOutput, String> {
        self.execute_inner(plan, access)
    }
}

impl CommandExecutor {
    fn execute_inner(
        &self,
        plan: &RecipePlan,
        access: Option<LiveAccess>,
    ) -> Result<RecipeOutput, String> {
        #[cfg(not(unix))]
        if access.is_some() {
            return Err("Guarded recipe execution is unsupported on this platform".into());
        }
        if access.as_ref().is_some_and(|access| !access.allowed()) {
            return Err("Credential authority is no longer valid".into());
        }
        let program = std::fs::canonicalize(&plan.program)
            .map_err(|_| "The recipe program does not exist".to_string())?;
        let metadata = std::fs::metadata(&program)
            .map_err(|_| "The recipe program is not readable".to_string())?;
        if !metadata.is_file() {
            return Err("The recipe program is not a regular file".into());
        }
        let cwd = std::fs::canonicalize(&plan.cwd)
            .map_err(|_| "The project folder is unavailable".to_string())?;
        if program.starts_with(&cwd) {
            return Err(
                "Recipe programs inside the agent's project are refused; use a fixed binary outside the project"
                    .into(),
            );
        }
        if let Some(expected) = &plan.program_hash {
            let actual = program_hash(&program)?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(
                    "The recipe program changed; review it and update the pinned hash".into(),
                );
            }
        }
        let mut command = Command::new(&program);
        command
            .args(&plan.args)
            .current_dir(&cwd)
            .env_clear()
            .env("PATH", safe_path())
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env(plan.env_var.as_str(), plan.secret.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Ok(home) = std::env::var("HOME") {
            command.env("HOME", home);
        }
        if let Ok(tmp) = std::env::var("TMPDIR") {
            command.env("TMPDIR", tmp);
        }
        run_command(command, plan.timeout, access)
    }
}

fn run_command(
    mut command: Command,
    timeout: Duration,
    access: Option<LiveAccess>,
) -> Result<RecipeOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = {
        // Process creation can fail transiently on loaded CI hosts (fork
        // pressure, or a just-written executable still busy). Retry only
        // unambiguous transient errors; all other spawn failures stay
        // visible and fail closed.
        let mut attempt = 0;
        loop {
            if access.as_ref().is_some_and(|access| !access.allowed()) {
                return Err("Credential authority is no longer valid".into());
            }
            match command.spawn() {
                Ok(child) => break child,
                Err(error)
                    if attempt < 5
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::Interrupted
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::ExecutableFileBusy
                        ) =>
                {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(20 * attempt));
                }
                Err(error) => {
                    return Err(format!("Could not start the recipe program: {error}"));
                }
            }
        }
    };
    let mut child = OwnedChild(child, false);
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| "Could not capture recipe output".to_string())?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or_else(|| "Could not capture recipe output".to_string())?;
    let out_reader =
        std::thread::spawn(move || read_capped(stdout, crate::http::MAX_RESPONSE_BODY));
    let err_reader =
        std::thread::spawn(move || read_capped(stderr, crate::http::MAX_RESPONSE_BODY));
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_watchdog(
        &mut child.0,
        timeout,
        done.clone(),
        timed_out.clone(),
        access.clone(),
    );
    #[cfg(unix)]
    let observed = loop {
        match exit_pending(child.0.id()) {
            Ok(false) => std::thread::sleep(Duration::from_millis(10)),
            Ok(true) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    #[cfg(unix)]
    kill_owned_group(child.0.id());
    // Join the watchdog before reaping the leader, retaining group identity
    // for every possible signal. Kill remaining group members on normal exit
    // too, so they cannot keep stdout/stderr open after the parent finishes.
    #[cfg(not(unix))]
    let status = child.reap();
    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    #[cfg(unix)]
    observed.map_err(|_| "Could not observe the recipe process".to_string())?;
    #[cfg(unix)]
    let status = child.reap();
    let (stdout, stdout_truncated) = out_reader.join().unwrap_or_else(|_| (String::new(), true));
    let (stderr, stderr_truncated) = err_reader.join().unwrap_or_else(|_| (String::new(), true));
    if access.as_ref().is_some_and(|access| !access.allowed()) {
        return Err("Credential authority was revoked; recipe outcome unknown".into());
    }
    let status = status.map_err(|error| format!("Could not wait for the recipe: {error}"))?;
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    Ok(RecipeOutput {
        exit_code: status.code(),
        signal,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
        timed_out: timed_out.load(Ordering::Relaxed),
    })
}

fn safe_path() -> String {
    "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into()
}

pub fn program_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| "Could not read the recipe program for hash verification".to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "Could not read the recipe program".to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn spawn_watchdog(
    child: &mut Child,
    timeout: Duration,
    done: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    access: Option<LiveAccess>,
) -> std::thread::JoinHandle<()> {
    let pid = child.id();
    std::thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if done.load(Ordering::Relaxed) {
                return;
            }
            if access.as_ref().is_some_and(|access| {
                !std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| access.allowed()))
                    .unwrap_or_else(|_| {
                        access.revoke();
                        false
                    })
            }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !done.load(Ordering::Relaxed) {
            timed_out.store(Instant::now() >= deadline, Ordering::Relaxed);
            #[cfg(unix)]
            kill_owned_group(pid);
        }
    })
}

fn read_capped(mut reader: impl Read, cap: usize) -> (String, bool) {
    let mut stored = Zeroizing::new(Vec::new());
    let mut truncated = false;
    let mut buffer = Zeroizing::new([0u8; 16 * 1024]);
    loop {
        match reader.read(&mut *buffer) {
            Ok(0) => break,
            Ok(read) => {
                if stored.len() < cap {
                    let remaining = cap - stored.len();
                    let keep = remaining.min(read);
                    stored.extend_from_slice(&buffer[..keep]);
                    if keep < read {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => {
                truncated = true;
                break;
            }
        }
    }
    (String::from_utf8_lossy(&stored).to_string(), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(program: &str) -> RecipePlan {
        RecipePlan {
            program: program.into(),
            args: Vec::new(),
            env_var: "DUNLIN_SECRET".into(),
            secret: Zeroizing::new("tok_value-123".into()),
            cwd: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            program_hash: None,
        }
    }

    #[test]
    fn refuses_programs_inside_the_project_and_supports_hashes() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("script.sh");
        std::fs::write(&script, "#!/bin/sh\necho hello\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut plan = plan(script.to_str().unwrap());
        plan.cwd = temp.path().to_path_buf();
        assert!(CommandExecutor.execute(&plan).is_err());
        let cwd = tempfile::tempdir().unwrap();
        plan.cwd = cwd.path().to_path_buf();
        plan.program_hash = Some(program_hash(&script).unwrap());
        let executed = CommandExecutor.execute(&plan);
        assert!(
            executed.is_ok(),
            "hashed script outside the project should run: {executed:?}"
        );
        std::fs::write(&script, "#!/bin/sh\necho changed\n").unwrap();
        assert!(CommandExecutor.execute(&plan).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn live_revocation_kills_running_recipe_before_timeout() {
        // The child creates a marker only after starting, then waits. Revocation
        // follows that marker so this exercises a running process, not preflight.
        let cwd = tempfile::tempdir().unwrap();
        let started = cwd.path().join("started");
        let mut plan = plan("/bin/sh");
        plan.cwd = cwd.path().to_path_buf();
        plan.args = vec!["-c".into(), "touch started; sleep 20; touch escaped".into()];
        plan.timeout = Duration::from_secs(30);
        let marker = started.clone();
        let access = LiveAccess::new(Instant::now() + Duration::from_secs(10), move || {
            !marker.exists()
        });
        let start = Instant::now();
        let result = CommandExecutor.execute_guarded(&plan, Some(access));
        assert!(started.exists(), "child must have started");
        assert!(result.unwrap_err().contains("unknown"));
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(!cwd.path().join("escaped").exists());
    }

    #[cfg(unix)]
    #[test]
    fn captures_output_exit_status_and_timeout() {
        let mut plan = plan("/usr/bin/printenv");
        plan.args = vec!["DUNLIN_SECRET".into()];
        let output = CommandExecutor.execute(&plan).unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("tok_value-123"));
        plan.program = "/usr/bin/false".into();
        plan.args = Vec::new();
        let output = CommandExecutor.execute(&plan).unwrap();
        assert_eq!(output.exit_code, Some(1));
        plan.program = "/bin/sleep".into();
        plan.args = vec!["2".into()];
        plan.timeout = Duration::from_millis(100);
        let output = CommandExecutor.execute(&plan).unwrap();
        assert!(output.timed_out);
    }
    #[cfg(unix)]
    #[test]
    fn exited_parent_cannot_leave_its_group_holding_output_pipes() {
        let cwd = tempfile::tempdir().unwrap();
        let mut command = plan("/bin/sh");
        command.cwd = cwd.path().to_path_buf();
        command.timeout = Duration::from_secs(30);
        command.args = vec!["-c".into(), "(echo child-output; touch child-ready; sleep 20; touch escaped) & while ! test -f child-ready; do sleep 0.01; done; exit 0".into()];
        let started = Instant::now();
        let result = CommandExecutor.execute(&command).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("child-output"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!cwd.path().join("escaped").exists());
    }
    #[cfg(unix)]
    #[test]
    fn panicking_live_authority_fails_closed_and_kills_the_owned_group() {
        let cwd = tempfile::tempdir().unwrap();
        let marker = cwd.path().join("started");
        let mut command = plan("/bin/sh");
        command.cwd = cwd.path().to_path_buf();
        command.args = vec!["-c".into(), "touch started; sleep 20; touch escaped".into()];
        command.timeout = Duration::from_secs(30);
        let access = LiveAccess::new(Instant::now() + Duration::from_secs(20), move || {
            assert!(!marker.exists(), "recipe authority fixture fault");
            true
        });
        let started = Instant::now();
        assert!(
            CommandExecutor
                .execute_guarded(&command, Some(access))
                .unwrap_err()
                .contains("unknown")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!cwd.path().join("escaped").exists());
    }
}
