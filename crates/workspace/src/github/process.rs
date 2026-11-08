//! Bounded stdin/stdout and process-group lifetime, including a child that never
//! reads its request body or leaves inherited stdout open in a descendant.
use std::{
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
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
pub(crate) fn run(
    command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    limit: u64,
) -> Result<(ExitStatus, Vec<u8>), String> {
    run_cancellable(command, input, timeout, limit, &AtomicBool::new(false))
}

pub(crate) fn run_cancellable(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    limit: u64,
    cancelled: &AtomicBool,
) -> Result<(ExitStatus, Vec<u8>), String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Search cancelled".into());
    }
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = {
        // A just-written fixture or a loaded host can transiently fail to spawn.
        // Retry only unambiguous transient errors; all other failures stay closed.
        let mut attempt = 0;
        loop {
            match command.spawn() {
                Ok(child) => break Child(child),
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
                Err(_) => return Err("Could not start GitHub CLI".into()),
            }
        }
    };
    let mut stdout = child.0.stdout.take().ok_or("Missing GitHub output")?;
    let (out_tx, out_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = out_tx.send(result);
    });
    let input_rx = input.map(|bytes| {
        let input = child.0.stdin.take();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = input
                .ok_or(std::io::ErrorKind::BrokenPipe.into())
                .and_then(|mut pipe| pipe.write_all(&bytes));
            let _ = tx.send(result);
        });
        rx
    });
    let start = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Search cancelled".into());
        }
        match child.0.try_wait(){Ok(Some(status))=>break status,Ok(None) if start.elapsed()<timeout=>std::thread::sleep(Duration::from_millis(10)),_=>return Err("GitHub CLI timed out. A submitted mutation may have succeeded; inspect remote state. No automatic retry.".into())}
    };
    let bytes = out_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "GitHub output stream did not close")?
        .map_err(|_| "Could not read GitHub output")?;
    if bytes.len() as u64 > limit {
        return Err("GitHub response exceeds the output limit".into());
    }
    if let Some(rx) = input_rx {
        rx.recv_timeout(Duration::from_secs(1))
            .map_err(|_| "GitHub request body was not consumed")?
            .map_err(
                |_| "Could not send GitHub request body; inspect remote state before resubmitting",
            )?;
    }
    Ok((status, bytes))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_stops_a_running_cli_instead_of_waiting_for_its_timeout() {
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "touch \"$1\"; sleep 20", "fixture"])
            .arg(&ready);
        let flag = cancelled.clone();
        let cancel = std::thread::spawn(move || {
            let start = Instant::now();
            while !ready.exists() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            flag.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        let result = run_cancellable(command, None, Duration::from_secs(30), 1024, &cancelled);
        cancel.join().unwrap();
        assert_eq!(result.unwrap_err(), "Search cancelled");
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
