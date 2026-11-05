//! Explicit argv-based local checks. No discovery execution, shell interpolation,
//! success inference from text, or automatic retry. Cancellation is cooperative
//! at the process-group boundary; scripts are not a filesystem sandbox.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const OUTPUT_LIMIT: u64 = 512 * 1024;
pub fn display_text(value: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut escape = 0u8;
    for c in value.chars() {
        if out.len() >= limit { out.push_str("\n[truncated; inspect full output]"); break; }
        match escape {
            1 => { escape = if c == '[' { 2 } else if c == ']' { 3 } else { 0 }; continue; }
            2 => { if ('@'..='~').contains(&c) { escape = 0; } continue; }
            3 => { if c == '\x07' { escape = 0; } else if c == '\x1b' { escape = 4; } continue; }
            4 => { escape = if c == '\\' { 0 } else { 3 }; continue; }
            _ => {}
        }
        if c == '\x1b' { escape = 1; continue; }
        if (c.is_control() && !matches!(c, '\n' | '\t')) || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { continue; }
        out.push(c);
    }
    out
}
#[test]
fn output_strips_terminal_and_bidi_controls_without_losing_unicode() {
    assert_eq!(display_text("\x1b[31mżółć\x1b[0m\u{202e}\x1b]0;title\x07\n", 1024), "żółć\n");
    assert!(display_text("abcdef", 3).starts_with("abc\n[truncated"));
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub path: String,
    pub line: u64,
    pub message: String,
    pub level: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub root: String,
    pub argv: Vec<String>,
    pub started_ms: u64,
    pub elapsed_ms: u64,
    pub result: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug)]
pub struct Prepared {
    pub root: String,
    pub argv: Vec<String>,
    identity: String,
    prepared: Instant,
}
fn identity(root: &str) -> Result<String, String> {
    let m = std::fs::metadata(root).map_err(|_| "Check working directory unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!(
            "{:?}",
            m.created()
                .map_err(|_| "Working directory identity unavailable")?
        ))
    }
}
pub fn prepare(root: &str, argv: Vec<String>) -> Result<Prepared, String> {
    if argv.is_empty()
        || argv.len() > 64
        || argv[0].is_empty()
        || argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
        || argv.iter().map(|a| a.len()).sum::<usize>() > 16384
    {
        return Err(
            "Provide a nonempty JSON argv array, at most 64 arguments / 16 KiB, without NUL".into(),
        );
    }
    let root = std::fs::canonicalize(root).map_err(|_| "Project folder unavailable")?;
    if !root.is_dir() {
        return Err("Project is not a directory".into());
    }
    let root = root.to_str().ok_or("Non-UTF-8 project path")?.to_owned();
    let identity = identity(&root)?;
    Ok(Prepared {
        root,
        argv,
        identity,
        prepared: Instant::now(),
    })
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn stream(
    mut pipe: impl Read + Send + 'static,
    overflow: Arc<AtomicBool>,
) -> mpsc::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buf = [0; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(n) => {
                    let remaining = (OUTPUT_LIMIT as usize).saturating_sub(bytes.len());
                    bytes.extend_from_slice(&buf[..n.min(remaining)]);
                    if n > remaining {
                        overflow.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }
        let _ = tx.send(Some(bytes));
    });
    rx
}
pub fn run(plan: Prepared, cancel: Arc<AtomicBool>) -> Result<Report, String> {
    run_timeout(plan, cancel, Duration::from_secs(120))
}
fn run_timeout(
    plan: Prepared,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<Report, String> {
    if plan.prepared.elapsed() > Duration::from_secs(300) || identity(&plan.root)? != plan.identity
    {
        return Err(
            "Check preview expired or its working directory was replaced; prepare again".into(),
        );
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Check cancelled before launch".into());
    }
    let mut command = Command::new(&plan.argv[0]);
    command
        .args(&plan.argv[1..])
        .current_dir(&plan.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = Child(
        command
            .spawn()
            .map_err(|_| "Could not launch this command; check executable availability")?,
    );
    let overflow = Arc::new(AtomicBool::new(false));
    let out = stream(
        child.0.stdout.take().ok_or("No check stdout")?,
        overflow.clone(),
    );
    let err = stream(
        child.0.stderr.take().ok_or("No check stderr")?,
        overflow.clone(),
    );
    let start = Instant::now();
    let started_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let (result, exit_code) = loop {
        if cancel.load(Ordering::Acquire) {
            break ("cancelled", None);
        }
        if overflow.load(Ordering::Acquire) {
            break ("output_limit", None);
        }
        if start.elapsed() > timeout {
            break ("timed_out", None);
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                break (
                    if status.success() {
                        "exited_zero"
                    } else {
                        "exited_nonzero"
                    },
                    status.code(),
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break ("wait_failed", None),
        }
    };
    drop(child);
    let out = out.recv_timeout(Duration::from_secs(1)).ok().flatten();
    let err = err.recv_timeout(Duration::from_secs(1)).ok().flatten();
    let complete = out.is_some() && err.is_some();
    let stdout = String::from_utf8_lossy(&out.unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.unwrap_or_default()).into_owned();
    let diagnostics = diagnostics(&plan.root, &format!("{stdout}\n{stderr}"));
    let result = if identity(&plan.root).ok().as_deref() != Some(&plan.identity) {
        "scope_changed"
    } else if overflow.load(Ordering::Acquire) {
        "output_limit"
    } else if !complete {
        "output_incomplete"
    } else {
        result
    };
    Ok(Report {
        id: uuid::Uuid::now_v7().to_string(),
        root: plan.root,
        argv: plan.argv,
        started_ms,
        elapsed_ms: start.elapsed().as_millis() as u64,
        result: result.into(),
        exit_code,
        stdout: display_text(&stdout, OUTPUT_LIMIT as usize),
        stderr: display_text(&stderr, OUTPUT_LIMIT as usize),
        diagnostics,
    })
}
fn diagnostics(root: &str, text: &str) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value["reason"] != "compiler-message" {
            continue;
        }
        let message = &value["message"];
        let Some(spans) = message["spans"].as_array() else {
            continue;
        };
        for span in spans.iter().filter(|s| s["is_primary"] == true) {
            let Some(path) = span["file_name"].as_str() else {
                continue;
            };
            let path = std::path::Path::new(path);
            let relative = if path.is_absolute() {
                let Ok(path) = path.strip_prefix(root) else {
                    continue;
                };
                path
            } else {
                path
            };
            if !relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
            {
                continue;
            }
            if result.len() >= 200 {
                return result;
            }
            result.push(Diagnostic {
                path: relative.to_string_lossy().into_owned(),
                line: span["line_start"].as_u64().unwrap_or(1),
                message: display_text(
                    message["message"].as_str().unwrap_or("Diagnostic"),
                    4000,
                ),
                level: message["level"].as_str().unwrap_or("unknown").into(),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn records_real_exit_status_and_captures_both_streams() {
        let d = tempfile::tempdir().unwrap();
        let p = prepare(
            d.path().to_str().unwrap(),
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf out; printf err >&2; exit 7".into(),
            ],
        )
        .unwrap();
        let r = run(p, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(r.exit_code, Some(7));
        assert_eq!(r.stdout, "out");
        assert_eq!(r.stderr, "err");
        assert_eq!(r.result, "exited_nonzero");
    }
    #[cfg(unix)]
    #[test]
    fn timeouts_and_prelaunch_cancellation_are_not_success() {
        let d = tempfile::tempdir().unwrap();
        let p = prepare(
            d.path().to_str().unwrap(),
            vec!["/bin/sleep".into(), "30".into()],
        )
        .unwrap();
        let r = run_timeout(
            p,
            Arc::new(AtomicBool::new(false)),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.result, "timed_out");
        let p = prepare(d.path().to_str().unwrap(), vec!["/bin/true".into()]).unwrap();
        assert!(run(p, Arc::new(AtomicBool::new(true))).is_err());
    }
    #[test]
    fn parses_only_structured_primary_diagnostics_confined_to_project() {
        let value = serde_json::json!({"reason":"compiler-message","message":{"message":"bad type","level":"error","spans":[{"file_name":"src/main.rs","line_start":7,"is_primary":true},{"file_name":"../outside","line_start":1,"is_primary":true}]}});
        let d = diagnostics("/repo", &value.to_string());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 7);
        assert!(diagnostics("/repo", "error at evil:7").is_empty());
    }
}
