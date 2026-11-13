//! Independent exec-self guardian, deliberately outside the main worker's
//! PDEATHSIG chain. It owns bootstrap/reap and survives main's SIGKILL.
use super::*;
use std::{fs::File, os::unix::process::CommandExt};
struct Namespace {
    child: std::process::Child,
    outer: OwnedFd,
    init: Option<OwnedFd>,
    gate: File,
    filled: usize,
    launch_allowed: bool,
}
fn pipe() -> std::io::Result<(File, File)> {
    let mut descriptors = [-1; 2];
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((unsafe { File::from_raw_fd(descriptors[0]) }, unsafe {
        File::from_raw_fd(descriptors[1])
    }))
}
fn nonblocking(fd: RawFd, enabled: bool) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                fd,
                libc::F_SETFL,
                if enabled {
                    flags | libc::O_NONBLOCK
                } else {
                    flags & !libc::O_NONBLOCK
                },
            )
        } < 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
fn duplicate(fd: RawFd) -> std::io::Result<OwnedFd> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 200) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}
fn bootstrap(manifest: &Manifest, control: &mut UnixStream) -> std::io::Result<Namespace> {
    let (mut info, info_writer) = pipe()?;
    let (gate, mut gate_writer) = pipe()?;
    nonblocking(gate_writer.as_raw_fd(), true)?;
    let mut filled = 0;
    loop {
        match gate_writer.write(&[b'x'; 4096]) {
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    nonblocking(gate_writer.as_raw_fd(), false)?;
    nonblocking(info.as_raw_fd(), true)?;
    let info_descriptor = duplicate(info_writer.as_raw_fd())?;
    let gate_descriptor = duplicate(gate_writer.as_raw_fd())?;
    let split = manifest
        .args
        .iter()
        .position(|a| a == "--")
        .ok_or_else(|| std::io::Error::other("Containment boundary is missing"))?;
    if !manifest.args[..split].iter().any(|a| a == "--unshare-all")
        || !manifest.args[..split]
            .iter()
            .any(|a| a == "--die-with-parent")
    {
        return Err(std::io::Error::other(
            "Required namespace containment is missing",
        ));
    }
    let mut command = std::process::Command::new(&manifest.program);
    command
        .args(&manifest.args[..split])
        .args(["--info-fd", "100", "--json-status-fd", "101"])
        .args(&manifest.args[split..]);
    command
        .current_dir(&manifest.cwd)
        .env_clear()
        .envs(&manifest.environment)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0);
    let expected_parent = std::process::id();
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                || libc::getppid() as u32 != expected_parent
            {
                return Err(std::io::Error::other(
                    "Containment parent changed before exec",
                ));
            }
            if libc::dup2(info_descriptor.as_raw_fd(), 100) < 0
                || libc::dup2(gate_descriptor.as_raw_fd(), 101) < 0
                || libc::fcntl(100, libc::F_SETFD, 0) < 0
                || libc::fcntl(101, libc::F_SETFD, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop(command);
    drop(info_writer);
    drop(gate_writer);
    let outer = pidfd(child.id())?;
    let mut namespace = Namespace {
        child,
        outer,
        init: None,
        gate,
        filled,
        launch_allowed: true,
    };
    let deadline = monotonic_ns()?
        .saturating_add(3_000_000_000)
        .min(manifest.deadline_ns);
    let mut raw = Vec::new();
    loop {
        if monotonic_ns()? >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Containment bootstrap timed out",
            ));
        }
        if !readable(info.as_raw_fd(), 20)? {
            continue;
        }
        let mut bytes = [0; 1024];
        match info.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => {
                raw.extend_from_slice(&bytes[..count]);
                if raw.len() > 4096 {
                    return Err(std::io::Error::other("Containment info exceeds bound"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    let value: serde_json::Value = serde_json::from_slice(&raw)?;
    let pid = value["child-pid"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 1)
        .ok_or_else(|| std::io::Error::other("Namespace identity missing"))?;
    if owned_pidfd(namespace.outer.as_raw_fd(), false)?.is_some() {
        return Err(std::io::Error::other(
            "Bootstrap owner exited before checkpoint",
        ));
    }
    send_fd(
        control,
        &encode(&Control::Bootstrap {
            nonce: manifest.nonce.clone(),
        })?,
        namespace.outer.as_raw_fd(),
    )?;
    namespace.launch_allowed = matches!(line(control,deadline).ok().and_then(|raw|serde_json::from_slice::<Control>(&raw).ok()),Some(Control::ContinueBootstrap{nonce}) if nonce==manifest.nonce);
    // Even EOF/timeout must acquire/validate the init descriptor before normal
    // cleanup. The still-blocked owning monitor cannot reap/reuse its child PID.
    let candidate = pidfd(pid)?;
    // The full blocking status pipe prevents the owning bwrap parent from
    // entering monitor/reap. Its child is still blocked on child_wait_fd. The
    // candidate is NEVER signalled before this owned-pidfd alive check.
    if owned_pidfd(namespace.outer.as_raw_fd(), false)?.is_some() {
        return Err(std::io::Error::other(
            "Bootstrap owner exited before handoff",
        ));
    }
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    let nested = status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))
        .and_then(|v| v.split_whitespace().last())
        .and_then(|v| v.parse::<u32>().ok());
    if nested != Some(1)
        || std::fs::read_link(format!("/proc/{pid}/ns/pid"))?
            == std::fs::read_link("/proc/self/ns/pid")?
    {
        return Err(std::io::Error::other(
            "Expected isolated namespace init is missing",
        ));
    }
    namespace.init = Some(candidate);
    Ok(namespace)
}
impl Namespace {
    fn release(&mut self, deadline: u64, budget: u64) -> std::io::Result<()> {
        while self.filled > 0 {
            let now = monotonic_ns()?;
            if now >= deadline || now >= budget {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Launch lease elapsed before barrier release",
                ));
            }
            let mut bytes = [0; 4096];
            let length = self.filled.min(bytes.len());
            let count = self.gate.read(&mut bytes[..length])?;
            if count == 0 || bytes[..count].iter().any(|b| *b != b'x') {
                return Err(std::io::Error::other("Bootstrap barrier changed"));
            }
            self.filled -= count;
        }
        Ok(())
    }
    fn stop(&mut self) -> std::io::Result<()> {
        if let Some(fd) = &self.init {
            signal_pidfd(fd.as_raw_fd())?;
        }
        // Parent group PID remains unreaped/owned until kill; no future numeric
        // signalling occurs after cleanup/reap. Namespace pidfd handles setsid.
        if owned_pidfd(self.outer.as_raw_fd(), false).is_ok() {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
        }
        cleanup_adopted()?;
        if let Some(fd) = &self.init
            && !readable(fd.as_raw_fd(), 0)?
        {
            return Err(std::io::Error::other("Namespace shutdown unconfirmed"));
        }
        Ok(())
    }
}
fn validate(manifest: &Manifest) -> std::io::Result<()> {
    if manifest.nonce.len() != 64
        || !manifest.nonce.bytes().all(|c| c.is_ascii_hexdigit())
        || manifest.args.len() > 512
        || manifest.args.iter().map(String::len).sum::<usize>() > 60000
        || manifest.args.iter().any(|a| a.contains('\0'))
        || !std::path::Path::new(&manifest.program).is_absolute()
        || !std::path::Path::new(&manifest.cwd).is_absolute()
    {
        return Err(std::io::Error::other("Invalid private execution manifest"));
    }
    let now = monotonic_ns()?;
    if manifest.deadline_ns <= now
        || manifest.deadline_ns > now.saturating_add(10_000_000_000)
        || manifest.budget_ns < manifest.deadline_ns
        || manifest.budget_ns > now.saturating_add(1_800_000_000_000)
    {
        return Err(std::io::Error::other("Invalid private execution deadline"));
    }
    if manifest.environment.keys().any(|k| {
        ![
            "PATH",
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "LANG",
            "LC_ALL",
            "TMPDIR",
        ]
        .contains(&k.as_str())
    }) {
        return Err(std::io::Error::other("Unexpected private environment"));
    }
    if binary_digest(std::path::Path::new(&manifest.program))? != manifest.program_digest {
        return Err(std::io::Error::other("Containment program changed"));
    }
    Ok(())
}
fn fallback(manifest: &Manifest) -> std::io::Result<()> {
    if let Some(record) = &manifest.ledger {
        let store = WorkerStore::open_existing(&record.directory, record.scope.clone())
            .map_err(std::io::Error::other)?;
        // Only called AFTER ECHILD + init-pidfd exit, never from a peer boolean.
        match &record.owner {
            GuardianOwner::Execution { identity, fence } => {
                store
                    .finish_stopped_execution(&record.scope, identity, fence, |actual| {
                        if actual == fence {
                            Ok(())
                        } else {
                            Err(local::Error::Denied)
                        }
                    })
                    .map_err(std::io::Error::other)?;
            }
            GuardianOwner::Session { identity, fence } => {
                store
                    .finish_stopped_session(&record.scope, identity, fence, |actual| {
                        if actual == fence {
                            Ok(())
                        } else {
                            Err(local::Error::Denied)
                        }
                    })
                    .map_err(std::io::Error::other)?;
            }
        }
    }
    Ok(())
}
pub(super) fn command(args: &[String]) -> std::io::Result<()> {
    if args.len() != 2 || args[0] != "--control-fd" {
        return Err(std::io::Error::other(
            "Explicit private control descriptor required",
        ));
    }
    let fd = args[1]
        .parse::<RawFd>()
        .ok()
        .filter(|n| (3..=1023).contains(n))
        .ok_or_else(|| std::io::Error::other("Invalid private control descriptor"))?;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&credentials) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    } < 0
        || credentials.pid != unsafe { libc::getppid() }
        || credentials.uid != unsafe { libc::geteuid() }
    {
        return Err(std::io::Error::other("Private guardian parent mismatch"));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut control = unsafe { UnixStream::from_raw_fd(fd) };
    control.set_read_timeout(Some(Duration::from_millis(200)))?;
    control.set_write_timeout(Some(Duration::from_millis(200)))?;
    dedicated_subreaper()?;
    let manifest: Manifest =
        serde_json::from_slice(&line(&mut control, monotonic_ns()? + 5_000_000_000)?)?;
    validate(&manifest)?;
    let started = bootstrap(&manifest, &mut control);
    let mut namespace = match started {
        Ok(namespace) => namespace,
        Err(error) => {
            // Owned dedicated subreaper cleanup is mandatory even when bootstrap
            // failed before identifying PID1; empty /proc alone is not proof.
            if cleanup_adopted().is_ok() {
                fallback(&manifest)?;
            }
            return Err(error);
        }
    };
    let outcome = (|| -> std::io::Result<()> {
        if !namespace.launch_allowed {
            return Err(std::io::Error::other("Main did not acknowledge bootstrap"));
        }
        send_fd(
            &control,
            &encode(&Control::Ready {
                nonce: manifest.nonce.clone(),
            })?,
            namespace
                .init
                .as_ref()
                .ok_or_else(|| std::io::Error::other("Namespace descriptor missing"))?
                .as_raw_fd(),
        )?;
        let mut deadline = manifest.deadline_ns;
        loop {
            match serde_json::from_slice::<Control>(&line(&mut control, deadline)?)? {
                Control::Go { nonce } if nonce == manifest.nonce => {
                    let now = monotonic_ns()?;
                    if now >= deadline || now >= manifest.budget_ns {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Launch lease expired",
                        ));
                    }
                    namespace.release(deadline, manifest.budget_ns)?;
                    break;
                }
                Control::Renew { deadline_ns }
                    if monotonic_ns()? < deadline
                        && deadline_ns > monotonic_ns()?
                        && deadline_ns <= manifest.budget_ns =>
                {
                    deadline = deadline_ns
                }
                _ => {
                    return Err(std::io::Error::other(
                        "Launch stopped before parent acknowledgement",
                    ));
                }
            }
        }
        loop {
            if !namespace
                .init
                .as_ref()
                .is_some_and(|fd| readable(fd.as_raw_fd(), 0).unwrap_or(false))
            {
                match serde_json::from_slice::<Control>(&line(&mut control, deadline)?)? {
                    Control::Renew { deadline_ns }
                        if monotonic_ns()? < deadline
                            && deadline_ns > monotonic_ns()?
                            && deadline_ns <= manifest.budget_ns =>
                    {
                        deadline = deadline_ns
                    }
                    _ => return Err(std::io::Error::other("Execution guardian stopped")),
                }
            } else {
                return Err(std::io::Error::other("Namespace init exited"));
            }
        }
    })();
    // No graceful response, EOF, timeout or malformed control bypasses cleanup.
    namespace.stop()?;
    let stopped = encode(&Control::Stopped {
        nonce: manifest.nonce.clone(),
    })?;
    let sent = control.write_all(&stopped).is_ok();
    let until = monotonic_ns()? + 2_000_000_000;
    let finalized = sent
        && (|| -> std::io::Result<bool> {
            loop {
                match serde_json::from_slice::<Control>(&line(&mut control, until)?)? {
                    Control::Finalized { nonce } if nonce == manifest.nonce => return Ok(true),
                    Control::Stop { nonce } if nonce == manifest.nonce => {}
                    Control::Renew { .. } => {} // Stopped is permanent, never revive.
                    _ => return Ok(false),
                }
            }
        })()
        .unwrap_or(false);
    if !finalized {
        fallback(&manifest)?;
    }
    let _ = outcome;
    Ok(())
}
