//! Bounded stdin/stdout and process-group lifetime, including a child that never
//! reads its request body or leaves inherited stdout open in a descendant.
use std::{
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
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
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    limit: u64,
) -> Result<(ExitStatus, Vec<u8>), String> {
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
    let mut child = Child(command.spawn().map_err(|_| "Could not start GitHub CLI")?);
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
