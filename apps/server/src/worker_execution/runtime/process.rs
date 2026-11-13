//! Dedicated-process Linux containment. No process identifier is recovered from
//! SQL. The independent exec-self guard and main are each child's subreaper.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
    process::Stdio,
};
#[path = "guardian.rs"]
mod guardian;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    nonce: String,
    program: String,
    args: Vec<String>,
    cwd: String,
    environment: BTreeMap<String, String>,
    deadline_ns: u64,
    budget_ns: u64,
    ledger: Option<OrphanRecord>,
    program_digest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Bootstrap { nonce: String },
    ContinueBootstrap { nonce: String },
    Go { nonce: String },
    Renew { deadline_ns: u64 },
    Ready { nonce: String },
    Stopped { nonce: String },
    Finalized { nonce: String },
    Stop { nonce: String },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrphanRecord {
    directory: PathBuf,
    scope: local::ExportScope,
    owner: GuardianOwner,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GuardianOwner {
    Execution {
        identity: wire::ExecutionIdentity,
        fence: wire::AttemptFence,
    },
    Session {
        identity: wire::session::SessionIdentity,
        fence: wire::session::SessionFence,
    },
}
#[derive(Clone, PartialEq, Eq)]
enum ProcessFence {
    Execution(wire::AttemptFence),
    Session(wire::session::SessionFence),
}
impl GuardianOwner {
    fn process_fence(&self) -> ProcessFence {
        match self {
            Self::Execution { fence, .. } => ProcessFence::Execution(fence.clone()),
            Self::Session { fence, .. } => ProcessFence::Session(fence.clone()),
        }
    }
    fn channel(&self) -> &wire::ChannelFence {
        match self {
            Self::Execution { fence, .. } => &fence.channel,
            Self::Session { fence, .. } => &fence.channel,
        }
    }
}
/// Trusted caller supplies a canonical session receipt, never a peer PID.
/// The live session actor separately owns authority checks before release().
pub(super) struct SessionOwner {
    pub identity: wire::session::SessionIdentity,
    pub fence: wire::session::SessionFence,
    pub group: Arc<Group>,
}
#[derive(Clone)]
pub(super) struct Platform {
    program: PathBuf,
    digest: String,
}
pub(super) fn monotonic_ns() -> std::io::Result<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        u64::try_from(now.tv_sec)
            .ok()
            .and_then(|s| s.checked_mul(1_000_000_000))
            .and_then(|v| v.checked_add(now.tv_nsec as u64))
            .ok_or_else(|| std::io::Error::other("Monotonic clock overflow"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::other("Unsupported worker platform"))
    }
}
/// Only call in the dedicated worker CLI before any other child spawner. The
/// relay/controller process must never adopt this process-wide cleanup policy.
pub(super) fn dedicated_subreaper() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
            return Err(std::io::Error::other("Child ownership cannot be preserved"));
        }
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::other("Unsupported worker platform"))
    }
}
fn pidfd(pid: u32) -> std::io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(std::io::Error::other("Unsupported worker platform"))
    }
}
fn signal_pidfd(fd: RawFd) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd,
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } < 0
        {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = fd;
        Err(std::io::Error::other("Unsupported worker platform"))
    }
}
fn readable(fd: RawFd, milliseconds: i32) -> std::io::Result<bool> {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll, 1, milliseconds) };
    if result < 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    Ok(result > 0 && poll.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}
#[cfg(test)]
static INTERRUPT_WAIT_ONCE: AtomicBool = AtomicBool::new(false);
fn waitid_retry(
    kind: libc::idtype_t,
    id: libc::id_t,
    flags: libc::c_int,
) -> std::io::Result<libc::siginfo_t> {
    let until = Instant::now() + Duration::from_millis(100);
    loop {
        #[cfg(test)]
        if INTERRUPT_WAIT_ONCE.swap(false, Ordering::AcqRel) {
            continue;
        }
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::waitid(kind, id, &mut info, flags) } == 0 {
            return Ok(info);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted && Instant::now() < until {
            continue;
        }
        return Err(error);
    }
}
fn owned_pidfd(fd: RawFd, reap: bool) -> std::io::Result<Option<u32>> {
    #[cfg(target_os = "linux")]
    {
        let flags = libc::WEXITED | libc::WNOHANG | if reap { 0 } else { libc::WNOWAIT };
        let info = waitid_retry(libc::P_PIDFD, fd as libc::id_t, flags)?;
        let pid = unsafe { info.si_pid() };
        Ok((pid > 0).then_some(pid as u32))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (fd, reap);
        Err(std::io::Error::other("Unsupported worker platform"))
    }
}
/// Dedicated-process only. Candidate pidfds must prove direct-child ownership
/// AFTER opening and BEFORE signalling. No stale proc PID or saved SQL PID is
/// authority. Do not run concurrently with another child spawner/wait-any code.
fn cleanup_adopted() -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut ids = std::collections::BTreeSet::new();
        for task in std::fs::read_dir("/proc/self/task")? {
            let path = task?.path().join("children");
            match std::fs::read_to_string(path) {
                Ok(raw) => {
                    for pid in raw.split_whitespace() {
                        ids.insert(pid.parse::<u32>().map_err(|_| {
                            std::io::Error::other("Invalid child ownership record")
                        })?);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        // proc children may omit live children racing another child's exit.
        // Only ECHILD from the exclusive wait-any ownership boundary proves the
        // entire dedicated descendant set is gone; never an empty listing alone.
        match waitid_retry(
            libc::P_ALL,
            0,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        ) {
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        for id in ids {
            let candidate = match pidfd(id) {
                Ok(fd) => fd,
                Err(e) if e.raw_os_error() == Some(libc::ESRCH) => continue,
                Err(e) => return Err(e),
            };
            match owned_pidfd(candidate.as_raw_fd(), false) {
                Ok(_) => {}
                Err(e) if e.raw_os_error() == Some(libc::ECHILD) => continue,
                Err(e) => return Err(e),
            }
            let status = match std::fs::read_to_string(format!("/proc/{id}/status")) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let parent = status
                .lines()
                .find_map(|l| l.strip_prefix("PPid:"))
                .and_then(|v| v.trim().parse::<u32>().ok());
            let uid = status
                .lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<u32>().ok());
            if parent != Some(std::process::id()) || uid != Some(unsafe { libc::geteuid() }) {
                return Err(std::io::Error::other("Adopted child scope differs"));
            }
            signal_pidfd(candidate.as_raw_fd())?;
            while !readable(candidate.as_raw_fd(), 20)? {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::other("Owned child shutdown unconfirmed"));
                }
            }
            match owned_pidfd(candidate.as_raw_fd(), true) {
                Ok(_) => {}
                Err(e) if e.raw_os_error() == Some(libc::ECHILD) => {}
                Err(e) => return Err(e),
            }
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(
                "Namespace adoption shutdown unconfirmed",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn encode(value: &impl Serialize) -> std::io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > 65536 {
        return Err(std::io::Error::other(
            "Private control frame exceeds its bound",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}
fn line(stream: &mut UnixStream, deadline_ns: u64) -> std::io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    loop {
        if monotonic_ns()? >= deadline_ns {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Private control deadline",
            ));
        }
        if !readable(stream.as_raw_fd(), 20)? {
            continue;
        }
        let mut byte = [0];
        let count = stream.read(&mut byte)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Private controller disconnected",
            ));
        }
        if byte[0] == b'\n' {
            if monotonic_ns()? >= deadline_ns {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Private control deadline elapsed while reading",
                ));
            }
            return Ok(raw);
        }
        raw.push(byte[0]);
        if raw.len() > 65536 {
            return Err(std::io::Error::other("Private control frame limit"));
        }
    }
}
fn send_fd(stream: &UnixStream, bytes: &[u8], fd: RawFd) -> std::io::Result<()> {
    let mut space = [0usize; 16];
    let mut vector = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = space.as_mut_ptr().cast();
    message.msg_controllen =
        unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as usize;
        std::ptr::write(libc::CMSG_DATA(header).cast::<RawFd>(), fd);
    }
    let count = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
    if count < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if count as usize != bytes.len() {
        return Err(std::io::Error::other("Incomplete private pidfd handoff"));
    }
    Ok(())
}
fn receive_fd(stream: &UnixStream) -> std::io::Result<(Vec<u8>, OwnedFd)> {
    let mut space = [0usize; 16];
    let mut bytes = [0u8; 256];
    let mut vector = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = space.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&space);
    let count = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) };
    if count <= 0 {
        return Err(if count == 0 {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Guardian exited before handoff",
            )
        } else {
            std::io::Error::last_os_error()
        });
    }
    let mut descriptors = Vec::new();
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let payload = (*header)
                    .cmsg_len
                    .saturating_sub(libc::CMSG_LEN(0) as usize);
                for index in 0..payload / std::mem::size_of::<RawFd>() {
                    descriptors.push(OwnedFd::from_raw_fd(std::ptr::read(
                        libc::CMSG_DATA(header).cast::<RawFd>().add(index),
                    )));
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if descriptors.len() != 1
        || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
        || bytes[count as usize - 1] != b'\n'
    {
        return Err(std::io::Error::other("Invalid private pidfd handoff"));
    }
    Ok((
        bytes[..count as usize - 1].to_vec(),
        descriptors.pop().expect("one checked descriptor"),
    ))
}
#[derive(Default)]
pub(super) struct Group {
    stopped: AtomicBool,
    namespace: Mutex<Option<Arc<OwnedFd>>>,
    outer_monitor: Mutex<Option<Arc<OwnedFd>>>,
    guardian: Mutex<Option<Arc<OwnedFd>>>,
    control: Mutex<Option<UnixStream>>,
    nonce: Mutex<String>,
    deadline_ns: AtomicU64,
}
impl Group {
    pub fn terminate(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(fd) = self.namespace.lock()
            && let Some(fd) = fd.as_ref()
        {
            let _ = signal_pidfd(fd.as_raw_fd());
        }
        if let Ok(mut control) = self.control.lock()
            && let Some(control) = control.as_mut()
        {
            // STOP is a negative control; keep the read/write channel alive so
            // main can persist its truthful outcome and send Finalized afterward.
            let nonce = self.nonce.lock().map(|n| n.clone()).unwrap_or_default();
            if encode(&Control::Stop { nonce })
                .and_then(|bytes| control.write_all(&bytes))
                .is_err()
            {
                let _ = control.shutdown(std::net::Shutdown::Write);
            }
        }
    }
    pub(super) fn guard_exited(&self) -> bool {
        self.guardian
            .lock()
            .ok()
            .and_then(|fd| {
                fd.as_ref()
                    .map(|fd| readable(fd.as_raw_fd(), 0).unwrap_or(true))
            })
            .unwrap_or(false)
    }
    #[cfg(test)]
    pub(super) fn fault_kill_guard(&self) {
        if let Ok(fd) = self.guardian.lock()
            && let Some(fd) = fd.as_ref()
        {
            signal_pidfd(fd.as_raw_fd()).expect("owned guardian fault signal");
        }
    }
    pub fn renew(&self, deadline_ns: u64) -> std::io::Result<()> {
        let old = self.deadline_ns.load(Ordering::Acquire);
        if self.stopped.load(Ordering::Acquire) || (old != 0 && monotonic_ns()? >= old) {
            self.terminate();
            return Err(std::io::Error::other("Guardian permanently stopped"));
        }
        let sent = (|| {
            let mut control = self
                .control
                .lock()
                .map_err(|_| std::io::Error::other("Control lock unavailable"))?;
            let Some(stream) = control.as_mut() else {
                // No guardian has been spawned yet; the caller's live lease and
                // independent main clock still fence this startup reservation.
                return Ok(());
            };
            stream.write_all(&encode(&Control::Renew { deadline_ns })?)
        })();
        if sent.is_err()
            || self.stopped.load(Ordering::Acquire)
            || (old != 0 && monotonic_ns()? >= old)
        {
            self.terminate();
            return Err(std::io::Error::other("Guardian renewal unconfirmed"));
        }
        self.deadline_ns.store(deadline_ns, Ordering::Release);
        Ok(())
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        self.terminate();
    }
}
/// Cannot be built from a peer boolean or a SQL status.
pub(super) struct StopProof {
    fence: Option<ProcessFence>,
    recovered: bool,
}
impl StopProof {
    pub fn attest(&self, fence: &wire::AttemptFence) -> local::Result<()> {
        if self.fence.as_ref() == Some(&ProcessFence::Execution(fence.clone())) {
            Ok(())
        } else {
            Err(local::Error::Denied)
        }
    }
    pub(super) fn never_spawned(fence: wire::AttemptFence) -> Self {
        Self {
            fence: Some(ProcessFence::Execution(fence)),
            recovered: false,
        }
    }
    pub(super) fn attest_session(&self, fence: &wire::session::SessionFence) -> local::Result<()> {
        if self.fence.as_ref() == Some(&ProcessFence::Session(fence.clone())) {
            Ok(())
        } else {
            Err(local::Error::Denied)
        }
    }
    pub(super) fn never_spawned_session(fence: wire::session::SessionFence) -> Self {
        Self {
            fence: Some(ProcessFence::Session(fence)),
            recovered: false,
        }
    }
    pub(super) fn recovered(&self) -> bool {
        self.recovered
    }
}
pub(super) struct Owned {
    pub child: tokio::process::Child,
    group: Arc<Group>,
    control: UnixStream,
    nonce: String,
    ledger: Option<OrphanRecord>,
    guardian_fd: Option<OwnedFd>,
    initial_error: Option<std::io::Error>,
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.group.terminate();
    }
}
pub(super) fn guard_command(args: &[String]) -> Result<(), String> {
    guardian::command(args).map_err(|_| "Worker execution guard failed closed".into())
}

fn binary_digest(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::Digest;
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(std::io::Error::other("Unsupported containment binary"));
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(std::io::Error::other("Containment binary exceeds limit"));
    }
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}
fn guardian_command(control: &UnixStream) -> std::io::Result<std::process::Command> {
    use std::os::unix::process::CommandExt;
    #[cfg(not(test))]
    let executable = std::env::current_exe()?;
    #[cfg(test)]
    let executable = std::env::var_os("DUNLIN_RELAY_E2E_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .ok_or_else(|| {
            std::io::Error::other("Actual production executable required for kernel tests")
        })?;
    let mut command = std::process::Command::new(executable);
    command
        .args(["worker", "execution-guard", "--control-fd", "3"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    let descriptor = unsafe { libc::fcntl(control.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 200) };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(descriptor.as_raw_fd(), 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(command)
}
fn manifest(
    platform: &Platform,
    command: &tokio::process::Command,
    deadline_ns: u64,
    budget_ns: u64,
    ledger: Option<OrphanRecord>,
) -> std::io::Result<Manifest> {
    let command = command.as_std();
    if command.get_program() != platform.program.as_os_str() {
        return Err(std::io::Error::other("Unverified containment binary"));
    }
    let args = command
        .get_args()
        .map(|v| {
            v.to_str()
                .map(str::to_owned)
                .ok_or_else(|| std::io::Error::other("Non-UTF8 containment argument"))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let cwd = command
        .get_current_dir()
        .and_then(|p| p.to_str())
        .ok_or_else(|| std::io::Error::other("Explicit checkout cwd required"))?
        .to_owned();
    let mut environment = BTreeMap::new();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            let name = name
                .to_str()
                .ok_or_else(|| std::io::Error::other("Invalid environment key"))?;
            if ![
                "PATH",
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "XDG_DATA_HOME",
                "LANG",
                "LC_ALL",
                "TMPDIR",
            ]
            .contains(&name)
            {
                return Err(std::io::Error::other("Unsupported execution environment"));
            }
            environment.insert(
                name.to_owned(),
                value
                    .to_str()
                    .ok_or_else(|| std::io::Error::other("Invalid environment value"))?
                    .to_owned(),
            );
        }
    }
    Ok(Manifest {
        nonce: format!(
            "{}{}",
            uuid::Uuid::now_v7().simple(),
            uuid::Uuid::now_v7().simple()
        ),
        program: platform
            .program
            .to_str()
            .ok_or_else(|| std::io::Error::other("Invalid containment path"))?
            .into(),
        program_digest: platform.digest.clone(),
        args,
        cwd,
        environment,
        deadline_ns,
        budget_ns,
        ledger,
    })
}
impl Owned {
    pub(super) fn spawn(
        platform: &Platform,
        config: &RuntimeConfig,
        command: tokio::process::Command,
        active: &Active,
    ) -> std::io::Result<Self> {
        let lease = active
            .lease
            .lock()
            .map_err(|_| std::io::Error::other("Lease unavailable"))?;
        let sample = monotonic_ns()?;
        let remaining = lease.remaining().map_err(std::io::Error::other)?;
        let deadline_ns = sample
            .checked_add(remaining.as_nanos() as u64)
            .ok_or_else(|| std::io::Error::other("Clock overflow"))?;
        let budget_ns = sample
            .checked_add(
                u64::try_from(
                    lease
                        .budget_remaining()
                        .map_err(std::io::Error::other)?
                        .as_nanos(),
                )
                .map_err(std::io::Error::other)?,
            )
            .ok_or_else(|| std::io::Error::other("Clock overflow"))?;
        let ledger = OrphanRecord {
            directory: config.state_dir.clone(),
            scope: config.scope.clone(),
            owner: GuardianOwner::Execution {
                identity: active.command.identity.clone(),
                fence: lease.fence().clone(),
            },
        };
        let owned = Self::spawn_owned(
            platform,
            command,
            active.group.clone(),
            ledger,
            deadline_ns,
            budget_ns,
        )?;
        drop(lease);
        Ok(owned)
    }
    /// Clock callback is sampled AFTER CLOCK_MONOTONIC, so translating an
    /// Instant-based lease can only shorten the deadline, never reset a budget.
    pub(super) fn spawn_session(
        platform: &Platform,
        config: &RuntimeConfig,
        command: tokio::process::Command,
        owner: SessionOwner,
        clock: impl FnOnce() -> local::Result<(Duration, Duration)>,
    ) -> std::io::Result<Self> {
        owner.identity.validate().map_err(std::io::Error::other)?;
        owner.fence.validate().map_err(std::io::Error::other)?;
        if owner.fence.channel.worker_boot_id != config.worker_boot_id {
            return Err(std::io::Error::other("Session worker boot differs"));
        }
        let sampled = monotonic_ns()?;
        let (remaining, budget) = clock().map_err(std::io::Error::other)?;
        let deadline_ns = sampled
            .checked_add(u64::try_from(remaining.as_nanos()).map_err(std::io::Error::other)?)
            .ok_or_else(|| std::io::Error::other("Session clock overflow"))?;
        let budget_ns = sampled
            .checked_add(u64::try_from(budget.as_nanos()).map_err(std::io::Error::other)?)
            .ok_or_else(|| std::io::Error::other("Session budget overflow"))?;
        let ledger = OrphanRecord {
            directory: config.state_dir.clone(),
            scope: config.scope.clone(),
            owner: GuardianOwner::Session {
                identity: owner.identity,
                fence: owner.fence,
            },
        };
        Self::spawn_owned(
            platform,
            command,
            owner.group,
            ledger,
            deadline_ns,
            budget_ns,
        )
    }
    fn spawn_owned(
        platform: &Platform,
        command: tokio::process::Command,
        group: Arc<Group>,
        ledger: OrphanRecord,
        deadline_ns: u64,
        budget_ns: u64,
    ) -> std::io::Result<Self> {
        let now = monotonic_ns()?;
        if deadline_ns <= now
            || deadline_ns > now.saturating_add(10_000_000_000)
            || budget_ns < deadline_ns
            || budget_ns > now.saturating_add(1_800_000_000_000)
            || group.stopped.load(Ordering::Acquire)
        {
            return Err(std::io::Error::other("Invalid owned process clock"));
        }
        let data = manifest(
            platform,
            &command,
            deadline_ns,
            budget_ns,
            Some(ledger.clone()),
        )?;
        let (mut control, child_control) = UnixStream::pair()?;
        control.set_nonblocking(true)?;
        let mut command = tokio::process::Command::from(guardian_command(&child_control)?);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        group.deadline_ns.store(deadline_ns, Ordering::Release);
        let encoded = encode(&data)?;
        let reader = control.try_clone()?;
        let writer = control.try_clone()?;
        *group
            .nonce
            .lock()
            .map_err(|_| std::io::Error::other("Control unavailable"))? = data.nonce.clone();
        // Every fallible allocation/descriptor preparation precedes fork. Once
        // the guard exists, its ownership is returned even if setup fails.
        let child = command.spawn()?;
        drop(child_control);
        let guardian_fd = child.id().and_then(|pid| pidfd(pid).ok());
        let observer = guardian_fd
            .as_ref()
            .and_then(|fd| fd.try_clone().ok())
            .map(Arc::new);
        let mut initial_error = if guardian_fd.is_none() {
            Some(std::io::Error::other("Guardian pidfd unavailable"))
        } else {
            None
        };
        if let Err(error) = control.write_all(&encoded) {
            initial_error = Some(error);
        }
        // Manifest is the first complete frame. Publish the control writer only
        // afterward, so concurrent renew/stop cannot precede or interleave it.
        match group.control.lock() {
            Ok(mut slot) => *slot = Some(writer),
            Err(_) => initial_error = Some(std::io::Error::other("Control unavailable")),
        }
        if let Ok(mut slot) = group.guardian.lock() {
            *slot = observer;
        } else {
            initial_error = Some(std::io::Error::other("Guardian observer unavailable"));
        }
        if group.stopped.load(Ordering::Acquire) {
            group.terminate();
        }
        let owned = Self {
            child,
            group: group.clone(),
            control: reader,
            nonce: data.nonce.clone(),
            ledger: Some(ledger),
            guardian_fd,
            initial_error,
        };
        Ok(owned)
    }
    pub(super) async fn ready(&mut self, connection: &Connected) -> std::io::Result<()> {
        if let Some(error) = self.initial_error.take() {
            return Err(error);
        }
        if self
            .ledger
            .as_ref()
            .is_some_and(|record| record.owner.channel() != &connection.fence)
        {
            return Err(std::io::Error::other(
                "Owner channel differs before bootstrap",
            ));
        }
        let stream = self.control.try_clone()?;
        let expected = self.nonce.clone();
        let group = self.group.clone();
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
            let until = monotonic_ns()? + 3_000_000_000;
            while !readable(stream.as_raw_fd(), 20)? {
                if monotonic_ns()? >= until {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Guard readiness deadline",
                    ));
                }
            }
            let (raw,outer)=receive_fd(&stream)?;
            if !matches!(serde_json::from_slice::<Control>(&raw)?,Control::Bootstrap{nonce} if nonce==expected){return Err(std::io::Error::other("Invalid bootstrap checkpoint"));}
            *group.outer_monitor.lock().map_err(|_|std::io::Error::other("Outer monitor unavailable"))?=Some(Arc::new(outer));
            #[cfg(test)]
            super::tests::fault_checkpoint("bootstrap",&group);
            connection.authorize().map_err(std::io::Error::other)?;
            if group.stopped.load(Ordering::Acquire)||monotonic_ns()?>=group.deadline_ns.load(Ordering::Acquire){group.terminate();return Err(std::io::Error::other("Bootstrap lease stopped"));}
            let mut sender=stream.try_clone()?;
            sender.write_all(&encode(&Control::ContinueBootstrap{nonce:expected.clone()})?)?;
            while !readable(stream.as_raw_fd(),20)? {if monotonic_ns()?>=until{return Err(std::io::Error::other("Namespace handoff timed out"));}}
            let (raw,descriptor)=receive_fd(&stream)?;
            if !matches!(serde_json::from_slice::<Control>(&raw)?,Control::Ready{nonce} if nonce==expected){return Err(std::io::Error::other("Invalid namespace handoff"));}
            // This FD came through the private exec-self socket from the owning
            // bootstrap guard. It is never created by interpreting a peer PID.
            *group
                .namespace
                .lock()
                .map_err(|_| std::io::Error::other("Namespace unavailable"))? =
                Some(Arc::new(descriptor));
            #[cfg(test)]
            super::tests::fault_checkpoint("handoff",&group);
            if group.stopped.load(Ordering::Acquire) {
                group.terminate();
                return Err(std::io::Error::other("Guard stopped before launch"));
            }
            Ok(())
        })
        .await
        .map_err(std::io::Error::other)?
    }
    pub(super) fn release(&mut self) -> std::io::Result<()> {
        if self.group.stopped.load(Ordering::Acquire)
            || monotonic_ns()? >= self.group.deadline_ns.load(Ordering::Acquire)
        {
            self.group.terminate();
            return Err(std::io::Error::other("Launch permanently stopped"));
        }
        self.control.write_all(&encode(&Control::Go {
            nonce: self.nonce.clone(),
        })?)
    }
    pub(super) async fn stop(&mut self) -> std::io::Result<StopProof> {
        self.group.terminate();
        let mut control = self.control.try_clone()?;
        let nonce = self.nonce.clone();
        let reply = tokio::task::spawn_blocking(move || {
            let deadline = monotonic_ns()? + 5_000_000_000;
            loop {
                let raw = line(&mut control, deadline)?;
                match serde_json::from_slice::<Control>(&raw)? {
                    Control::Stopped { nonce: actual } if actual == nonce => {
                        return Ok::<_, std::io::Error>(());
                    }
                    Control::Ready { nonce: actual } | Control::Bootstrap { nonce: actual }
                        if actual == nonce => {}
                    _ => {
                        return Err(std::io::Error::other(
                            "Invalid guardian stop acknowledgement",
                        ));
                    }
                }
            }
        })
        .await
        .map_err(std::io::Error::other)?;
        let recovered = if reply.is_ok() {
            false
        } else {
            // A killed guard cannot acknowledge. Only its owned pidfd is used;
            // then main proves all adopted descendants gone via P_ALL ECHILD.
            if let Some(fd) = &self.guardian_fd {
                signal_pidfd(fd.as_raw_fd())?;
            }
            tokio::time::timeout(Duration::from_secs(2), self.child.wait())
                .await
                .map_err(|_| std::io::Error::other("Guardian shutdown unconfirmed"))??;
            tokio::task::spawn_blocking(cleanup_adopted)
                .await
                .map_err(std::io::Error::other)??;
            true
        };
        if let Some(fd) = self
            .group
            .namespace
            .lock()
            .map_err(|_| std::io::Error::other("Namespace unavailable"))?
            .as_ref()
            && !readable(fd.as_raw_fd(), 0)?
        {
            return Err(std::io::Error::other("Namespace exit unconfirmed"));
        }
        Ok(StopProof {
            fence: self.ledger.as_ref().map(|r| r.owner.process_fence()),
            recovered,
        })
    }
    pub(super) async fn reap(&mut self) -> std::io::Result<()> {
        tokio::time::timeout(Duration::from_secs(6), self.child.wait())
            .await
            .map_err(|_| std::io::Error::other("Guardian exit unconfirmed"))??;
        Ok(())
    }
    pub(super) fn finalized(&mut self) -> std::io::Result<()> {
        self.control.write_all(&encode(&Control::Finalized {
            nonce: self.nonce.clone(),
        })?)
    }
}

/// At most one startup probe can be quarantined. Its std child and pinned pidfd
/// remain owned for the life of this dedicated worker, and no further platform
/// startup/Hello3 is permitted. This is not a stop proof or an automatic retry.
static VERSION_QUARANTINE: Mutex<Option<VersionOwner>> = Mutex::new(None);
struct VersionOwner {
    child: Option<std::process::Child>,
    pin: Option<OwnedFd>,
    verified: bool,
}
impl VersionOwner {
    fn id(&self) -> u32 {
        self.child.as_ref().expect("owned version child").id()
    }
    fn observe(&self, reap: bool) -> std::io::Result<libc::siginfo_t> {
        let flags = libc::WEXITED | libc::WNOHANG | if reap { 0 } else { libc::WNOWAIT };
        if self.verified {
            waitid_retry(
                libc::P_PIDFD,
                self.pin.as_ref().expect("verified pidfd").as_raw_fd() as libc::id_t,
                flags,
            )
        } else {
            waitid_retry(libc::P_PID, self.id(), flags)
        }
    }
    fn kill_owned(&self) {
        if self.child.is_none() {
            return;
        }
        // Numeric group ID is used ONLY while a fresh wait proves this direct
        // child still owned and unreaped. The verified pidfd remains safe even
        // after an unexpected external reaper, and cannot address a reused PID.
        if self.observe(false).is_ok() {
            unsafe {
                libc::kill(-(self.id() as i32), libc::SIGKILL);
            }
        }
        if self.verified
            && let Some(fd) = &self.pin
        {
            let _ = signal_pidfd(fd.as_raw_fd());
        }
    }
    fn stop(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::unix::process::ExitStatusExt;
        self.kill_owned();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            match self.observe(true) {
                Ok(info) if unsafe { info.si_pid() } > 0 => {
                    let value = unsafe { info.si_status() };
                    let status = match info.si_code {
                        libc::CLD_EXITED => {
                            Some(std::process::ExitStatus::from_raw((value & 255) << 8))
                        }
                        libc::CLD_KILLED => Some(std::process::ExitStatus::from_raw(value & 127)),
                        libc::CLD_DUMPED => {
                            Some(std::process::ExitStatus::from_raw((value & 127) | 128))
                        }
                        _ => None,
                    };
                    self.child.take();
                    return Ok(status);
                }
                Err(error) if error.raw_os_error() == Some(libc::ECHILD) && self.verified => {
                    if readable(self.pin.as_ref().expect("verified pidfd").as_raw_fd(), 0)? {
                        self.child.take();
                        return Ok(None);
                    }
                    return Err(error);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
                Ok(_) => {}
            }
            if Instant::now() >= until {
                return Err(std::io::Error::other(
                    "Version process shutdown unconfirmed; dedicated startup quarantined",
                ));
            }
            self.kill_owned();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for VersionOwner {
    fn drop(&mut self) {
        self.kill_owned();
    }
}
/// Nonblocking output <=128 bytes, 2s deadline, and cleanup on EVERY error path.
fn version_output(program: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use std::os::unix::process::CommandExt;
    if VERSION_QUARANTINE
        .lock()
        .map_err(|_| std::io::Error::other("Version quarantine unavailable"))?
        .is_some()
    {
        return Err(std::io::Error::other(
            "Dedicated worker startup is quarantined",
        ));
    }
    let mut command = std::process::Command::new(program);
    command
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let parent = std::process::id();
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                || libc::getppid() as u32 != parent
            {
                return Err(std::io::Error::other("Version owner changed before exec"));
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    // Ownership exists immediately after spawn, before any fallible pidfd/read.
    let mut owner = VersionOwner {
        child: Some(child),
        pin: None,
        verified: false,
    };
    let until = Instant::now() + Duration::from_secs(2);
    let result = (|| -> std::io::Result<Vec<u8>> {
        owner.pin = Some(pidfd(owner.id())?);
        owned_pidfd(owner.pin.as_ref().expect("opened pidfd").as_raw_fd(), false)?;
        owner.verified = true;
        let mut stdout = owner
            .child
            .as_mut()
            .expect("owned child")
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("Version pipe missing"))?;
        let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let mut output = Vec::new();
        loop {
            if Instant::now() >= until {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Containment version probe timed out",
                ));
            }
            if !readable(stdout.as_raw_fd(), 20)? {
                continue;
            }
            let mut bytes = [0; 256];
            match stdout.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    if output.len() + count > 128 {
                        return Err(std::io::Error::other(
                            "Containment version output exceeds its bound",
                        ));
                    }
                    output.extend_from_slice(&bytes[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        loop {
            if unsafe { owner.observe(false)?.si_pid() } > 0 {
                break;
            }
            if Instant::now() >= until {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Version process did not exit",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(output)
    })();
    let stopped = owner.stop();
    // Attempt adoption cleanup even if observe/reap failed. No early `?` after
    // spawn may discard the only ownership handle or omit descendant cleanup.
    let adopted = cleanup_adopted();
    if stopped.is_err() || adopted.is_err() {
        owner.kill_owned();
        let mut quarantine = VERSION_QUARANTINE
            .lock()
            .map_err(|_| std::io::Error::other("Version quarantine unavailable"))?;
        *quarantine = Some(owner);
        return Err(std::io::Error::other(
            "Version shutdown unconfirmed; dedicated startup quarantined",
        ));
    }
    let status = stopped.expect("checked stop result");
    if result.is_ok() && !status.is_some_and(|status| status.success()) {
        return Err(std::io::Error::other("Containment version process failed"));
    }
    result
}
#[cfg(test)]
pub(super) fn fault_version_probe(
    program: &std::path::Path,
    interrupt: bool,
) -> std::io::Result<()> {
    dedicated_subreaper()?;
    INTERRUPT_WAIT_ONCE.store(interrupt, Ordering::Release);
    let result = version_output(program);
    assert!(
        VERSION_QUARANTINE.lock().unwrap().is_none(),
        "actual test shutdown must be confirmed"
    );
    match waitid_retry(
        libc::P_ALL,
        0,
        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
    ) {
        Err(error) if error.raw_os_error() == Some(libc::ECHILD) => {}
        _ => panic!("version probe left owned descendants before fixture exit"),
    }
    result.map(|_| ())
}

impl Platform {
    pub(super) fn preflight(config: &RuntimeConfig) -> std::io::Result<Self> {
        dedicated_subreaper()?;
        let self_fd = pidfd(std::process::id())?;
        drop(self_fd);
        let program = crate::management::executable(&config.server_config, "bwrap")
            .unwrap_or_else(|| PathBuf::from("/usr/bin/bwrap"));
        // Supported source ordering is verified in official v0.8.0 and v0.11.0.
        // No unknown-version or non-bubblewrap fallback may advertise v3.
        let output = version_output(&program)?;
        let version = std::str::from_utf8(&output)
            .map_err(std::io::Error::other)?
            .trim()
            .to_owned();
        if !["bubblewrap 0.8.0", "bubblewrap 0.11.0"].contains(&version.as_str()) {
            return Err(std::io::Error::other(
                "Unsupported containment source ordering",
            ));
        }
        let platform = Self {
            digest: binary_digest(&program)?,
            program,
        };
        let probe = config
            .state_dir
            .join("execution-runtime")
            .join(format!("probe-{}", uuid::Uuid::now_v7()));
        crate::management::private_dir(&probe).map_err(std::io::Error::other)?;
        let harness = LocalHarness {
            program: "/usr/bin/true".into(),
            args: vec![],
        };
        let command = super::sandbox_command(
            config,
            &harness,
            &probe,
            &crate::store::native_checkout::SandboxMetadataRoots::default(),
            "platform-probe",
        )?;
        let now = monotonic_ns()?;
        let data = manifest(
            &platform,
            &command,
            now + 5_000_000_000,
            now + 5_000_000_000,
            None,
        )?;
        let (mut control, child_control) = UnixStream::pair()?;
        control.set_read_timeout(Some(Duration::from_secs(3)))?;
        control.set_write_timeout(Some(Duration::from_secs(1)))?;
        let mut command = guardian_command(&child_control)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        drop(child_control);
        let result = (|| -> std::io::Result<()> {
            control.write_all(&encode(&data)?)?;
            if !readable(control.as_raw_fd(), 3000)? {
                return Err(std::io::Error::other("Containment handoff timed out"));
            }
            let (bootstrap, outer) = receive_fd(&control)?;
            if !matches!(serde_json::from_slice::<Control>(&bootstrap)?,Control::Bootstrap{nonce} if nonce==data.nonce)
            {
                return Err(std::io::Error::other(
                    "Platform bootstrap checkpoint differs",
                ));
            }
            let _outer_backup = outer;
            control.write_all(&encode(&Control::ContinueBootstrap {
                nonce: data.nonce.clone(),
            })?)?;
            if !readable(control.as_raw_fd(), 3000)? {
                return Err(std::io::Error::other("Platform init handoff timed out"));
            }
            let (raw, namespace) = receive_fd(&control)?;
            if !matches!(serde_json::from_slice::<Control>(&raw)?,Control::Ready{nonce} if nonce==data.nonce)
            {
                return Err(std::io::Error::other("Containment handoff differs"));
            }
            control.write_all(&encode(&Control::Go {
                nonce: data.nonce.clone(),
            })?)?;
            // No user-provided program or scope participates in this startup probe.
            signal_pidfd(namespace.as_raw_fd())?;
            control.write_all(&encode(&Control::Stop {
                nonce: data.nonce.clone(),
            })?)?;
            if !matches!(serde_json::from_slice::<Control>(&line(&mut control,monotonic_ns()?+5_000_000_000)?)?,Control::Stopped{nonce} if nonce==data.nonce)
            {
                return Err(std::io::Error::other("Containment stop proof missing"));
            }
            if !readable(namespace.as_raw_fd(), 0)? {
                return Err(std::io::Error::other("Namespace init remains active"));
            }
            control.write_all(&encode(&Control::Finalized {
                nonce: data.nonce.clone(),
            })?)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = control.shutdown(std::net::Shutdown::Both);
        }
        let until = Instant::now() + Duration::from_secs(8);
        while child.try_wait()?.is_none() {
            if Instant::now() >= until {
                let candidate = pidfd(child.id())?;
                if owned_pidfd(candidate.as_raw_fd(), false).is_ok() {
                    signal_pidfd(candidate.as_raw_fd())?;
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.wait();
        cleanup_adopted()?;
        std::fs::remove_dir(&probe)?;
        result?;
        Ok(platform)
    }
    pub(super) fn program_directory(&self) -> PathBuf {
        self.program
            .parent()
            .expect("absolute verified program")
            .to_owned()
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    #[test]
    fn valid_renewal_before_guard_spawn_does_not_permanently_stop_reservation() {
        let group = Group::default();
        group
            .renew(monotonic_ns().unwrap() + 5_000_000_000)
            .unwrap();
        group
            .renew(monotonic_ns().unwrap() + 6_000_000_000)
            .unwrap();
        assert!(!group.stopped.load(Ordering::Acquire));
    }
}

#[cfg(test)]
#[path = "process_owner_tests.rs"]
mod owner_tests;
