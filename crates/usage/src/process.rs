//! Bounded local probes. The process group stays owned until the leader is reaped.
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub(super) struct ProbeProcess {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: ChildStdout,
    deadline: Instant,
    bytes: usize,
}
impl ProbeProcess {
    pub fn spawn(
        mut command: Command,
        timeout: Duration,
        authorized: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        check(authorized)?;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(not(unix))]
        return Err("Bounded account probes require macOS or Linux".into());
        #[cfg(unix)]
        {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Could not start account probe".to_owned())?;
            // Own the process before any fallible pipe setup.
            let input = child.stdin.take();
            let output = child.stdout.take().expect("piped probe output");
            let process = Self {
                child: Some(child),
                input,
                output,
                deadline: Instant::now() + timeout,
                bytes: 0,
            };
            nonblocking(&process.output)?;
            if let Some(input) = &process.input {
                nonblocking(input)?;
            }
            check(authorized)?;
            Ok(process)
        }
    }
    fn guard(&self, authorized: &dyn Fn() -> bool) -> Result<(), String> {
        check(authorized)?;
        if Instant::now() >= self.deadline {
            return Err("Account probe timed out".into());
        }
        Ok(())
    }
    pub fn write(&mut self, mut bytes: &[u8], authorized: &dyn Fn() -> bool) -> Result<(), String> {
        while !bytes.is_empty() {
            self.guard(authorized)?;
            match self
                .input
                .as_mut()
                .ok_or("Account probe input closed")?
                .write(bytes)
            {
                Ok(0) => return Err("Account probe input closed".into()),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => pause(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err("Account probe input failed".into()),
            }
        }
        Ok(())
    }
    pub fn close_input(&mut self) {
        self.input.take();
    }
    pub fn read(&mut self, authorized: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
        let mut buffer = [0u8; 4096];
        loop {
            self.guard(authorized)?;
            match self.output.read(&mut buffer) {
                Ok(n) => {
                    self.bytes += n;
                    if self.bytes > 256 * 1024 {
                        return Err("Account probe exceeded its output limit".into());
                    }
                    check(authorized)?;
                    return Ok(buffer[..n].to_vec());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => pause(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err("Account probe output failed".into()),
            }
        }
    }
    pub fn finish(&mut self, authorized: &dyn Fn() -> bool) -> Result<ExitStatus, String> {
        loop {
            self.guard(authorized)?;
            #[cfg(unix)]
            {
                let pid = self.child.as_ref().ok_or("Account probe finished")?.id();
                // WNOWAIT keeps the leader unreaped, so the group identity cannot
                // be reused before cleanup of inherited pipes/descendants.
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result == -1 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err("Account probe wait failed".into());
                }
                if unsafe { info.si_pid() } != 0 {
                    if !kill_group(pid) {
                        return Err("Account probe process ownership changed".into());
                    }
                    let status = self
                        .child
                        .as_mut()
                        .unwrap()
                        .wait()
                        .map_err(|_| "Account probe wait failed".to_owned())?;
                    self.child.take();
                    check(authorized)?;
                    return Ok(status);
                }
            }
            pause();
        }
    }
}
impl Drop for ProbeProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            #[cfg(unix)]
            if kill_group(child.id()) {
                let _ = child.kill();
            }
            #[cfg(not(unix))]
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn check(authorized: &dyn Fn() -> bool) -> Result<(), String> {
    if authorized() {
        Ok(())
    } else {
        Err("Account probe authorization changed".into())
    }
}
fn pause() {
    std::thread::sleep(Duration::from_millis(10));
}
#[cfg(unix)]
fn kill_group(pid: u32) -> bool {
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } == 0
        {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return false;
        }
    }
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    true
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err("Could not bound account probe I/O".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "dunlin-probe-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn command(&self, script: &str) -> Command {
            let mut command = Command::new("python3");
            command.args(["-c", script]).env("PROBE_TEST_ROOT", &self.0);
            command
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn denied_probe_never_starts_a_process() {
        let f = Fixture::new();
        let command = f.command(
            "import os,pathlib;pathlib.Path(os.environ['PROBE_TEST_ROOT'],'started').touch()",
        );
        assert!(ProbeProcess::spawn(command, Duration::from_secs(1), &|| false).is_err());
        assert!(!f.0.join("started").exists());
    }
    #[test]
    fn revocation_kills_the_owned_process_group_without_late_effects() {
        let f = Fixture::new();
        let command=f.command("import os,pathlib,time\nr=pathlib.Path(os.environ['PROBE_TEST_ROOT'])\nchild=os.fork()\nif child==0:\n time.sleep(0.3)\n (r/'late-effect').touch()\n time.sleep(30)\nelse:\n (r/'leader').write_text(str(os.getpid()))\n (r/'ready').touch()\n time.sleep(30)");
        let allowed = || !f.0.join("ready").exists();
        match ProbeProcess::spawn(command, Duration::from_secs(2), &allowed) {
            Ok(mut process) => {
                assert!(
                    process
                        .read(&allowed)
                        .unwrap_err()
                        .contains("authorization changed")
                );
            }
            Err(error) => assert!(error.contains("authorization changed")),
        }
        let pid: i32 = std::fs::read_to_string(f.0.join("leader"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(f.0.join("ready").exists());
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "Owned leader was not reaped"
        );
        std::thread::sleep(Duration::from_millis(450));
        assert!(
            !f.0.join("late-effect").exists(),
            "Descendant survived revocation"
        );
    }
    #[test]
    fn silent_or_inherited_output_has_a_deadline_and_bounded_cleanup() {
        let f = Fixture::new();
        let command=f.command("import os,pathlib,time\nr=pathlib.Path(os.environ['PROBE_TEST_ROOT'])\nif os.fork()==0:\n (r/'ready').touch()\n time.sleep(1)\n (r/'late-effect').touch()\n time.sleep(30)\nelse:\n os._exit(0)");
        let start = Instant::now();
        let mut p = ProbeProcess::spawn(command, Duration::from_millis(500), &|| true).unwrap();
        assert!(p.read(&|| true).unwrap_err().contains("timed out"));
        drop(p);
        assert!(
            f.0.join("ready").exists(),
            "Fixture descendant never started"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!f.0.join("late-effect").exists());
    }
    #[test]
    fn unlimited_output_fails_at_the_byte_limit_and_reaps_the_process() {
        let f = Fixture::new();
        let command=f.command("import sys,time\nsys.stdout.buffer.write(b'x'*300000)\nsys.stdout.flush()\ntime.sleep(30)");
        let mut p = ProbeProcess::spawn(command, Duration::from_secs(2), &|| true).unwrap();
        let pid = p.child.as_ref().unwrap().id();
        loop {
            match p.read(&|| true) {
                Ok(bytes) => assert!(!bytes.is_empty()),
                Err(error) => {
                    assert!(error.contains("output limit"));
                    break;
                }
            }
        }
        drop(p);
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
    #[test]
    fn closed_stdout_cannot_turn_finish_into_an_unbounded_wait() {
        let f = Fixture::new();
        let command = f.command("import os,time\nos.close(1)\ntime.sleep(30)");
        let mut p = ProbeProcess::spawn(command, Duration::from_secs(1), &|| true).unwrap();
        assert!(p.read(&|| true).unwrap().is_empty());
        assert!(p.finish(&|| true).unwrap_err().contains("timed out"));
    }
}
