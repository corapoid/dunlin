//! Best-effort macOS seatbelt execution for explicitly approved local checks.
//!
//! The profile denies network, denies writes except one scratch directory and
//! denies common credential stores. Resource limits are applied with setrlimit.
//! This is a local check runner, not a container and not a sandbox for an
//! arbitrary ACP harness.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub wall_secs: u64,
    pub cpu_secs: u64,
    pub address_space_bytes: u64,
    pub file_size_bytes: u64,
    pub processes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_secs: 120,
            cpu_secs: 60,
            address_space_bytes: 2 * 1024 * 1024 * 1024,
            file_size_bytes: 64 * 1024 * 1024,
            processes: 64,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Output {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// A deny-by-default-writes profile with an explicit scratch exception.
pub fn profile(scratch: &Path, limits: &Limits) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut text = String::from("(version 1)\n(allow default)\n(deny network*)\n");
    text.push_str(&format!(
        "(deny file-write*)\n(allow file-write* (subpath \"{}\"))\n",
        escape(scratch)
    ));
    if let Some(home) = home {
        for relative in [
            ".ssh",
            ".aws",
            ".gnupg",
            ".config",
            "Library/Keychains",
            "Library/Application Support/Dunlin",
            "Library/Application Support/Codex",
        ] {
            text.push_str(&format!(
                "(deny file-read* (subpath \"{}\"))\n",
                escape(&home.join(relative))
            ));
        }
    }
    text.push_str(&format!(
        "; limits: wall={}s cpu={}s as={} fsize={} nproc={}\n",
        limits.wall_secs,
        limits.cpu_secs,
        limits.address_space_bytes,
        limits.file_size_bytes,
        limits.processes
    ));
    text
}

#[cfg(unix)]
fn apply_limits(limits: &Limits) -> io::Result<()> {
    // macOS rejects some POSIX limits (notably RLIMIT_AS) on some releases.
    // Apply each limit independently and keep the seatbelt profile as the
    // mandatory boundary; the wall-clock deadline is enforced by the parent.
    unsafe {
        let cpu = libc::rlimit {
            rlim_cur: limits.cpu_secs as libc::rlim_t,
            rlim_max: limits.cpu_secs as libc::rlim_t,
        };
        let _ = libc::setrlimit(libc::RLIMIT_CPU, &cpu);
        let address = libc::rlimit {
            rlim_cur: limits.address_space_bytes as libc::rlim_t,
            rlim_max: limits.address_space_bytes as libc::rlim_t,
        };
        let _ = libc::setrlimit(libc::RLIMIT_AS, &address);
        let file = libc::rlimit {
            rlim_cur: limits.file_size_bytes as libc::rlim_t,
            rlim_max: limits.file_size_bytes as libc::rlim_t,
        };
        let _ = libc::setrlimit(libc::RLIMIT_FSIZE, &file);
        let processes = libc::rlimit {
            rlim_cur: limits.processes as libc::rlim_t,
            rlim_max: limits.processes as libc::rlim_t,
        };
        let _ = libc::setrlimit(libc::RLIMIT_NPROC, &processes);
    }
    Ok(())
}

fn clip(value: &[u8]) -> String {
    String::from_utf8_lossy(&value[..value.len().min(256 * 1024)]).to_string()
}

#[cfg(target_os = "macos")]
pub fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    scratch: &Path,
    limits: &Limits,
) -> Result<Output, String> {
    if program.trim().is_empty() || program.contains('\0') {
        return Err("program must be a non-empty argv[0]".into());
    }
    if !cwd.is_dir() {
        return Err("working directory is unavailable".into());
    }
    if !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err("sandbox-exec is unavailable; refusing to execute target code".into());
    }
    std::fs::create_dir_all(scratch).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(scratch, std::fs::Permissions::from_mode(0o700));
    }
    let profile = profile(scratch, limits);
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-p")
        .arg(profile)
        .arg(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let limits = *limits;
        unsafe {
            command.pre_exec(move || apply_limits(&limits));
        }
    }
    let child = command
        .spawn()
        .map_err(|error| format!("Could not start sandboxed check: {error}"))?;
    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = sender.send(output);
    });
    match receiver.recv_timeout(Duration::from_secs(limits.wall_secs)) {
        Ok(Ok(output)) => Ok(Output {
            success: output.status.success(),
            exit_code: output.status.code(),
            timed_out: false,
            stdout: clip(&output.stdout),
            stderr: clip(&output.stderr),
        }),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            let _ = receiver.recv_timeout(Duration::from_secs(2));
            Ok(Output {
                success: false,
                exit_code: None,
                timed_out: true,
                stdout: String::new(),
                stderr: format!("Sandboxed check exceeded {} seconds", limits.wall_secs),
            })
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn run(
    _program: &str,
    _args: &[String],
    _cwd: &Path,
    _scratch: &Path,
    _limits: &Limits,
) -> Result<Output, String> {
    Err("sandboxed local checks are currently macOS-only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_denies_network_and_writes_outside_scratch() {
        let text = profile(Path::new("/tmp/audit-scratch"), &Limits::default());
        assert!(text.contains("(deny network*)"));
        assert!(text.contains("(deny file-write*)"));
        assert!(text.contains("/tmp/audit-scratch"));
        assert!(text.contains("Library/Application Support/Dunlin"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_bounded_echo_check_runs_without_a_shell() {
        let temp = tempfile::tempdir().unwrap();
        let scratch = temp.path().join("scratch");
        let output = run(
            "/bin/echo",
            &["sandbox-ok".to_string()],
            temp.path(),
            &scratch,
            &Limits {
                wall_secs: 10,
                cpu_secs: 5,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(output.success);
        assert!(output.stdout.contains("sandbox-ok"));
    }
}
