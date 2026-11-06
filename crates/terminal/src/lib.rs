//! Local and relay-owned PTY ownership plus a bounded VT screen.
//!
//! The PTY reader and child waiter run on dedicated threads. The UI owns the
//! `vt100::Parser` and processes output in the foreground, so rendering never
//! blocks on a shell. Dropping a session kills its child; there is no hidden
//! relaunch or replay. Remote sessions use the same `Session` API but carry
//! bytes over caller-owned channels (the relay WebSocket lives in the desktop).

pub use vt100::{Cell, Color};

mod integration;
mod selection;
mod shell;

pub use selection::{Point, Selection, word_bounds};
pub use shell::{ShellBlock, ShellSnapshot};

use async_channel::{Receiver, Sender};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};

pub const DEFAULT_ROWS: u16 = 30;
pub const DEFAULT_COLS: u16 = 110;
pub const DEFAULT_SCROLLBACK: usize = 5_000;
pub const MAX_SCROLLBACK: usize = 50_000;
pub const OUTPUT_CHUNK: usize = 32 * 1024;
const MAX_PENDING_OSC: usize = 1024 * 1024;
const MAX_LINKED_CELLS: usize = 500_000;

#[derive(Clone, Debug)]
pub struct SpawnOptions {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
    /// Use the built-in zsh/bash wrapper that emits OSC 133 and OSC 7. Only
    /// interactive local shells opt in; Run commands are never modified.
    pub shell_integration: bool,
}

impl SpawnOptions {
    pub fn shell(cwd: PathBuf) -> Result<Self, String> {
        let program = default_shell()?;
        Ok(Self {
            program,
            args: vec!["-l".into()],
            cwd,
            env: Vec::new(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            scrollback: DEFAULT_SCROLLBACK,
            shell_integration: true,
        })
    }

    pub fn command(cwd: PathBuf, program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd,
            env: Vec::new(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            scrollback: DEFAULT_SCROLLBACK,
            shell_integration: false,
        }
    }

    /// Explicit SSH terminal transport. The host is an OpenSSH alias from the
    /// user's config; Dunlin never writes keys or disables host verification.
    /// `remote_path` is optional and is quoted as one shell word on the remote
    /// side. The local process never receives project environment variables or
    /// Dunlin bearer tokens.
    pub fn ssh(
        target: &str,
        local_cwd: PathBuf,
        remote_path: Option<&str>,
    ) -> Result<Self, String> {
        validate_ssh_target(target)?;
        let control_dir = ssh_control_dir()?;
        let program = if Path::new("/usr/bin/ssh").is_file() {
            "/usr/bin/ssh".to_string()
        } else {
            "ssh".to_string()
        };
        let mut args = vec![
            "-tt".to_string(),
            "-o".to_string(),
            "ControlMaster=auto".to_string(),
            "-o".to_string(),
            format!(
                "ControlPath={}",
                control_dir.join("dunlin-%C").to_string_lossy()
            ),
            "-o".to_string(),
            "ControlPersist=60".to_string(),
            "-o".to_string(),
            "ConnectTimeout=15".to_string(),
            "--".to_string(),
            target.to_string(),
        ];
        let remote = if let Some(path) = remote_path.filter(|path| !path.trim().is_empty()) {
            format!(
                "cd {} && exec \"${{SHELL:-/bin/sh}}\" -l",
                integration::quote(path.trim())
            )
        } else {
            "exec \"${SHELL:-/bin/sh}\" -l".to_string()
        };
        args.push(remote);
        Ok(Self {
            program,
            args,
            cwd: local_cwd,
            env: Vec::new(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            scrollback: DEFAULT_SCROLLBACK,
            shell_integration: false,
        })
    }

    pub fn ssh_with_identity(
        target: &str,
        local_cwd: PathBuf,
        remote_path: Option<&str>,
        identity: Option<&Path>,
        port: Option<u16>,
    ) -> Result<Self, String> {
        let mut options = Self::ssh(target, local_cwd, remote_path)?;
        if let Some(identity) = identity {
            let metadata = std::fs::symlink_metadata(identity)
                .map_err(|_| "SSH private key is unavailable")?;
            if !identity.is_absolute()
                || !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 65536
            {
                return Err(
                    "Choose an absolute, regular SSH private-key file (not a symlink)".into(),
                );
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(
                        "SSH private key permissions must exclude group/other access (chmod 600)"
                            .into(),
                    );
                }
            }
            let identity = identity
                .to_str()
                .filter(|p| !p.chars().any(char::is_control))
                .ok_or("Invalid SSH key path")?;
            options.args.splice(
                0..0,
                [
                    "-i",
                    identity,
                    "-o",
                    "IdentitiesOnly=yes",
                    "-o",
                    "ControlMaster=no",
                    "-o",
                    "ControlPath=none",
                ]
                .map(str::to_string),
            );
        }
        if let Some(port) = port {
            if port == 0 {
                return Err("SSH port must be between 1 and 65535".into());
            }
            options.args.splice(0..0, ["-p".into(), port.to_string()]);
        }
        Ok(options)
    }
}

fn effective_env(key: &str, env: &[(String, String)]) -> Option<String> {
    env.iter()
        .rev()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

fn validate_ssh_target(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty()
        || target.len() > 256
        || target.starts_with('-')
        || target.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'@' | b':' | b'[' | b']' | b'%'))
        })
        || target.matches('@').count() > 1
    {
        return Err("Enter an SSH host or user@host alias without spaces or options".into());
    }
    Ok(())
}

fn ssh_control_dir() -> Result<PathBuf, String> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        // OpenSSH limits Unix control sockets to roughly a hundred bytes. The
        // per-user macOS temp directory is too long once `%C` is expanded, so
        // use the short system temp root with a random, private directory.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let root = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = root.join(format!("dunlin-ssh-{}-{stamp}", std::process::id()));
        if std::fs::create_dir_all(&dir).is_err() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Some(dir)
    })
    .clone()
    .ok_or_else(|| "Could not create a private SSH control directory".to_string())
}

pub fn default_shell() -> Result<String, String> {
    let configured = std::env::var("SHELL").ok();
    for candidate in
        configured
            .into_iter()
            .chain(["/bin/zsh".into(), "/bin/bash".into(), "/bin/sh".into()])
    {
        if candidate.starts_with('/') && Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
    }
    Err("No interactive shell was found".into())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Exit {
    pub success: bool,
    pub code: Option<u32>,
    pub signal: Option<String>,
}

/// Control messages for a relay-owned PTY.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Resize { rows: u16, cols: u16 },
    Close,
}

enum Backend {
    Local {
        writer: Mutex<Box<dyn Write + Send>>,
        master: Mutex<Box<dyn MasterPty + Send>>,
        killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    },
    Remote {
        input: Sender<Vec<u8>>,
        control: Sender<Control>,
    },
}

pub struct Session {
    backend: Backend,
    output: Receiver<Vec<u8>>,
    exit: Receiver<Exit>,
    pid: Option<u32>,
    _reader: Option<std::thread::JoinHandle<()>>,
    _waiter: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("pid", &self.pid).finish()
    }
}

impl Session {
    pub fn spawn(options: SpawnOptions) -> Result<Self, String> {
        let rows = options.rows.max(2);
        let cols = options.cols.max(2);
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty = native_pty_system()
            .openpty(size)
            .map_err(|error| format!("Could not open a PTY: {error}"))?;
        if !options.cwd.is_dir() {
            return Err("Terminal working directory is not available".into());
        }
        let home_override = effective_env("HOME", &options.env);
        let integration = options
            .shell_integration
            .then(|| integration::prepare(&options.program, home_override.as_deref()))
            .flatten();
        let args = integration
            .as_ref()
            .map(|integration| integration.args.as_slice())
            .unwrap_or(options.args.as_slice());
        let mut command = CommandBuilder::new(&options.program);
        for arg in args {
            command.arg(arg);
        }
        command.cwd(&options.cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // Never hand the terminal a bearer token or app-internal control data.
        for key in [
            "DUNLIN_REMOTE_TOKEN",
            "DUNLIN_RELAY_DB",
            "DUNLIN_RELAY_LISTEN",
            "DUNLIN_RELAY_HARNESS",
            "DUNLIN_RELAY_LINGER_SECS",
            "DUNLIN_RELAY_SANDBOX",
            "DUNLIN_RELAY_STORE_TRANSCRIPTS",
            "DUNLIN_RELAY_DEBUG",
            "DUNLIN_AUTOPROMPT",
            "DUNLIN_HARNESS_COMMAND",
            "DUNLIN_DATA_DIR",
            "DUNLIN_LOG_FILE",
            "DUNLIN_SHELL_INTEGRATION",
        ] {
            command.env_remove(key);
        }
        for (key, value) in &options.env {
            if !key.is_empty()
                && !key.contains('=')
                && !key.starts_with("DUNLIN_")
                && !matches!(key.as_str(), "PATH" | "BASH_ENV" | "ENV" | "ZDOTDIR")
            {
                command.env(key, value);
            }
        }
        if let Some(integration) = &integration {
            for (key, value) in &integration.env {
                command.env(key, value);
            }
            // zsh chooses its default history file before our wrapper restores
            // ZDOTDIR. Point it at the user's normal history file so multiple
            // Dunlin tabs do not share a temporary wrapper history.
            let shell_name = Path::new(&options.program)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if shell_name.ends_with("zsh") && std::env::var_os("HISTFILE").is_none() {
                let home =
                    effective_env("HOME", &options.env).or_else(|| std::env::var("HOME").ok());
                if let Some(base) = integration.original_zdotdir.clone().or(home) {
                    command.env("HISTFILE", format!("{base}/.zsh_history"));
                }
            }
        }
        let child = pty
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Could not start the terminal shell: {error}"))?;
        drop(pty.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|error| format!("Could not read from the PTY: {error}"))?;
        let writer = pty
            .master
            .take_writer()
            .map_err(|error| format!("Could not write to the PTY: {error}"))?;
        let (output_tx, output_rx) = async_channel::bounded(128);
        let (exit_tx, exit_rx) = async_channel::bounded(1);
        let reader_thread = std::thread::Builder::new()
            .name("dunlin-terminal-reader".into())
            .spawn(move || {
                let mut buffer = vec![0u8; OUTPUT_CHUNK];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if output_tx.send_blocking(buffer[..read].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|error| format!("Could not start the PTY reader: {error}"))?;
        let waiter_thread = std::thread::Builder::new()
            .name("dunlin-terminal-waiter".into())
            .spawn(move || {
                let mut child = child;
                let exit = match child.wait() {
                    Ok(status) => Exit {
                        success: status.success(),
                        code: status.signal().is_none().then(|| status.exit_code()),
                        signal: status.signal().map(str::to_string),
                    },
                    Err(_) => Exit {
                        success: false,
                        code: None,
                        signal: Some("wait failed".into()),
                    },
                };
                let _ = exit_tx.send_blocking(exit);
            })
            .map_err(|error| format!("Could not start the PTY waiter: {error}"))?;
        Ok(Self {
            backend: Backend::Local {
                writer: Mutex::new(writer),
                master: Mutex::new(pty.master),
                killer: Mutex::new(killer),
            },
            output: output_rx,
            exit: exit_rx,
            pid,
            _reader: Some(reader_thread),
            _waiter: Some(waiter_thread),
        })
    }

    /// Wrap caller-owned relay channels. The caller owns the WebSocket thread
    /// and is responsible for forwarding input/control messages and for
    /// closing the connection when `Control::Close` arrives.
    pub fn remote(
        output: Receiver<Vec<u8>>,
        exit: Receiver<Exit>,
        input: Sender<Vec<u8>>,
        control: Sender<Control>,
    ) -> Self {
        Self {
            backend: Backend::Remote { input, control },
            output,
            exit,
            pid: None,
            _reader: None,
            _waiter: None,
        }
    }

    pub fn output(&self) -> Receiver<Vec<u8>> {
        self.output.clone()
    }

    pub fn exit(&self) -> Receiver<Exit> {
        self.exit.clone()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote { .. })
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        match &self.backend {
            Backend::Local { writer, .. } => {
                let mut writer = writer
                    .lock()
                    .map_err(|_| "Terminal writer is unavailable".to_string())?;
                writer
                    .write_all(bytes)
                    .and_then(|_| writer.flush())
                    .map_err(|error| format!("Terminal write failed: {error}"))
            }
            Backend::Remote { input, .. } => input
                .try_send(bytes.to_vec())
                .map_err(|_| "The relay terminal input buffer is full".to_string()),
        }
    }

    pub fn paste(&self, text: &str) -> Result<(), String> {
        if text.len() > 1024 * 1024 {
            return Err("Paste is limited to 1 MiB".into());
        }
        // Bracketed paste is safe for multi-line shell input when the shell
        // enabled it. Fall back to plain bytes; never execute text from logs.
        self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let rows = rows.max(2);
        let cols = cols.max(2);
        match &self.backend {
            Backend::Local { master, .. } => {
                let master = master
                    .lock()
                    .map_err(|_| "Terminal resize is unavailable".to_string())?;
                master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|error| format!("Terminal resize failed: {error}"))
            }
            Backend::Remote { control, .. } => control
                .try_send(Control::Resize { rows, cols })
                .map_err(|_| "The relay terminal control buffer is full".to_string()),
        }
    }

    pub fn kill(&self) {
        match &self.backend {
            Backend::Local { killer, .. } => {
                if let Ok(mut killer) = killer.lock() {
                    let _ = killer.kill();
                }
            }
            Backend::Remote { control, .. } => {
                let _ = control.try_send(Control::Close);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ScanState {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    String,
}

/// Splits one output byte stream into printable runs and complete escape
/// tokens. Incomplete escape sequences stay buffered until the next chunk so a
/// split CSI/OSC sequence can never be mistaken for printable text.
#[derive(Default)]
struct OutputScanner {
    state: ScanState,
    escape: Vec<u8>,
    printable: Vec<u8>,
}

impl OutputScanner {
    fn escape_complete(&self) -> bool {
        let bytes = &self.escape;
        match self.state {
            ScanState::Escape => match bytes.len() {
                0 | 1 => false,
                2 => !matches!(bytes[1], b' '..=b'/' | b'[' | b']' | b'P' | b'_' | b'^'),
                _ => (b'0'..=b'~').contains(bytes.last().unwrap()),
            },
            ScanState::Csi => bytes.len() > 2 && (b'@'..=b'~').contains(bytes.last().unwrap()),
            ScanState::Osc | ScanState::String => {
                bytes.last() == Some(&0x07)
                    || (bytes.len() >= 2
                        && bytes[bytes.len() - 2] == 0x1b
                        && bytes[bytes.len() - 1] == b'\\')
            }
            ScanState::Ground => true,
        }
    }
}

/// A bounded terminal screen. The parser is intentionally owned by the caller,
/// not by the reader thread, so output ordering matches UI updates.
pub struct Screen {
    parser: vt100::Parser<shell::ShellCallbacks>,
    rows: u16,
    cols: u16,
    scrollback: usize,
    pending_osc: Vec<u8>,
    active_link: Option<Arc<str>>,
    links: BTreeMap<(i64, u16), Arc<str>>,
    shell: Arc<Mutex<shell::ShellState>>,
    origin: Arc<AtomicI64>,
    discarded: Arc<AtomicU64>,
    resizing: Arc<AtomicBool>,
    last_scrollback: usize,
    /// Absolute exclusive end of content ever written (or the viewport bottom
    /// after a scroll). Used to preserve line ids across height changes even
    /// when the terminal is not full.
    content_bottom: i64,
    scanner: OutputScanner,
}

impl Screen {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        let rows = rows.max(2);
        let cols = cols.max(2);
        let scrollback = scrollback.clamp(100, MAX_SCROLLBACK);
        let shell = Arc::new(Mutex::new(shell::ShellState::default()));
        let origin = Arc::new(AtomicI64::new(0));
        let discarded = Arc::new(AtomicU64::new(0));
        let resizing = Arc::new(AtomicBool::new(false));
        Self {
            parser: vt100::Parser::new_with_callbacks(
                NonZeroU16::new(rows).unwrap_or(NonZeroU16::MIN),
                NonZeroU16::new(cols).unwrap_or(NonZeroU16::MIN),
                scrollback,
                shell::ShellCallbacks {
                    state: shell.clone(),
                    origin: origin.clone(),
                    discarded: discarded.clone(),
                    resizing: resizing.clone(),
                },
            ),
            rows,
            cols,
            scrollback,
            pending_osc: Vec::new(),
            active_link: None,
            links: BTreeMap::new(),
            shell,
            origin,
            discarded,
            resizing,
            last_scrollback: 0,
            content_bottom: 0,
            scanner: OutputScanner::default(),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.pending_osc.extend_from_slice(bytes);
        if self.pending_osc.len() > MAX_PENDING_OSC {
            // A malicious producer cannot grow the pending OSC buffer without
            // bound. Flush what we have as ordinary output and reset the link
            // state; the VT parser still consumes the bytes safely.
            let drained = std::mem::take(&mut self.pending_osc);
            self.feed(&drained);
            self.active_link = None;
            return;
        }
        loop {
            let Some(start) = find_bytes(&self.pending_osc, b"\x1b]8;") else {
                let keep = partial_suffix(&self.pending_osc, b"\x1b]8;");
                let split = self.pending_osc.len().saturating_sub(keep);
                if split > 0 {
                    let chunk = self.pending_osc[..split].to_vec();
                    self.feed(&chunk);
                    self.pending_osc.drain(..split);
                }
                break;
            };
            if start > 0 {
                let chunk = self.pending_osc[..start].to_vec();
                self.feed(&chunk);
                self.pending_osc.drain(..start);
                continue;
            }
            let payload_start = 4; // ESC ] 8 ;
            let Some((end, consumed)) = osc_terminator(&self.pending_osc, payload_start) else {
                break;
            };
            let payload = self.pending_osc[payload_start..end].to_vec();
            self.pending_osc.drain(..consumed);
            self.apply_osc8(&payload);
        }
    }

    fn apply_osc8(&mut self, payload: &[u8]) {
        let payload = &payload[..payload.len().min(8192)];
        let uri = payload
            .iter()
            .position(|byte| *byte == b';')
            .map(|index| &payload[index + 1..])
            .unwrap_or(payload);
        if uri.is_empty() {
            self.active_link = None;
        } else {
            let uri = String::from_utf8_lossy(uri);
            self.active_link = safe_link(&uri).map(Arc::from);
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        let mut scanner = std::mem::take(&mut self.scanner);
        for &byte in bytes {
            match scanner.state {
                ScanState::Ground => {
                    if byte == 0x1b {
                        self.flush_printable(&mut scanner.printable);
                        scanner.escape.clear();
                        scanner.escape.push(byte);
                        scanner.state = ScanState::Escape;
                    } else if byte < 0x20 || byte == 0x7f {
                        self.flush_printable(&mut scanner.printable);
                        self.feed_token(&[byte]);
                    } else {
                        scanner.printable.push(byte);
                    }
                }
                _ => {
                    scanner.escape.push(byte);
                    if scanner.state == ScanState::Escape && scanner.escape.len() == 2 {
                        scanner.state = match byte {
                            b'[' => ScanState::Csi,
                            b']' => ScanState::Osc,
                            b'P' | b'_' | b'^' => ScanState::String,
                            _ => ScanState::Escape,
                        };
                    }
                    if scanner.escape_complete() {
                        let token = std::mem::take(&mut scanner.escape);
                        self.feed_token(&token);
                        scanner.state = ScanState::Ground;
                    }
                }
            }
        }
        if matches!(scanner.state, ScanState::Ground) {
            self.flush_printable(&mut scanner.printable);
        }
        self.scanner = scanner;
    }

    fn flush_printable(&mut self, printable: &mut Vec<u8>) {
        if printable.is_empty() {
            return;
        }
        self.sync_origin();
        let run = std::mem::take(printable);
        let link = self.active_link.clone();
        let (start_row, start_col) = self.parser.screen().cursor_position();
        let start_line = self.origin.load(Ordering::Relaxed) + i64::from(start_row);
        self.parser.process(&run);
        self.sync_origin();
        let (end_row, end_col) = self.parser.screen().cursor_position();
        let end_line = self.origin.load(Ordering::Relaxed) + i64::from(end_row);
        let end = if end_line == start_line && end_col <= start_col {
            // The cursor does not advance after drawing the final column until
            // the next glyph. The cell was still written; mark it explicitly.
            (end_line, start_col.saturating_add(1))
        } else {
            (end_line, end_col)
        };
        self.mark_cells((start_line, start_col), end, link.as_deref());
    }

    /// Fold retained-scrollback growth and discarded rows into the absolute
    /// line origin. `on_scroll` alone is insufficient: it fires only when a
    /// row leaves a full scrollback, not when the scrollback is still growing.
    fn sync_origin(&mut self) {
        let scrollback = self.parser.screen().scrollback_len();
        let discarded = self.discarded.swap(0, Ordering::Relaxed) as i64;
        let grown = (scrollback as i64 - self.last_scrollback as i64).max(0);
        let delta = grown + discarded;
        let origin = if delta != 0 {
            self.origin.fetch_add(delta, Ordering::Relaxed) + delta
        } else {
            self.origin.load(Ordering::Relaxed)
        };
        let (row, _) = self.parser.screen().cursor_position();
        self.content_bottom = self.content_bottom.max(origin + i64::from(row) + 1);
        if delta > 0 {
            self.content_bottom = self.content_bottom.max(origin + i64::from(self.rows));
        }
        self.last_scrollback = scrollback;
    }

    fn feed_token(&mut self, token: &[u8]) {
        let full_reset = token == b"\x1bc";
        let clear_all = token
            .windows(5)
            .any(|window| window == b"1049h" || window == b"1049l")
            || token.ends_with(b"J");
        let clear_line = token.ends_with(b"K");
        let clear_scrollback = token.ends_with(b"3J");
        self.sync_origin();
        self.parser.process(token);
        self.sync_origin();
        if full_reset {
            self.links.clear();
            self.active_link = None;
            self.origin.store(0, Ordering::Relaxed);
            self.discarded.store(0, Ordering::Relaxed);
            self.last_scrollback = 0;
            self.content_bottom = 0;
            if let Ok(mut shell) = self.shell.lock() {
                shell.clear();
            }
            return;
        }
        if clear_all {
            if clear_scrollback {
                self.links.clear();
            } else {
                let top =
                    self.origin.load(Ordering::Relaxed) - self.parser.screen().scrollback() as i64;
                self.links.retain(|(line, _), _| *line < top);
            }
            let (row, _) = self.parser.screen().cursor_position();
            self.content_bottom = self.origin.load(Ordering::Relaxed) + i64::from(row) + 1;
        }
        if clear_line {
            let (row, _) = self.parser.screen().cursor_position();
            let line = self.origin.load(Ordering::Relaxed) + i64::from(row);
            self.links.retain(|(candidate, _), _| *candidate != line);
        }
    }

    fn mark_cells(&mut self, start: (i64, u16), end: (i64, u16), link: Option<&str>) {
        let cols = self.cols;
        if cols == 0 {
            return;
        }
        let (start_line, start_col) = start;
        let (end_line, end_col) = end;
        if end_line < start_line || (end_line == start_line && end_col <= start_col) {
            return;
        }
        let mut line = start_line;
        while line <= end_line {
            let from = if line == start_line { start_col } else { 0 };
            let to = if line == end_line { end_col } else { cols };
            for col in from..to.min(cols) {
                if let Some(link) = link {
                    self.links.insert((line, col), Arc::<str>::from(link));
                } else {
                    self.links.remove(&(line, col));
                }
            }
            line += 1;
        }
        if self.links.len() > MAX_LINKED_CELLS {
            let drop_until = self.links.len() - MAX_LINKED_CELLS / 2;
            let keys: Vec<_> = self.links.keys().take(drop_until).copied().collect();
            for key in keys {
                self.links.remove(&key);
            }
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn screen_mut(&mut self) -> &mut vt100::Screen {
        self.parser.screen_mut()
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn scrollback_capacity(&self) -> usize {
        self.scrollback
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows.max(2);
        self.cols = cols.max(2);
        self.resizing.store(true, Ordering::Relaxed);
        self.parser.set_size(
            NonZeroU16::new(self.rows).unwrap_or(NonZeroU16::MIN),
            NonZeroU16::new(self.cols).unwrap_or(NonZeroU16::MIN),
        );
        self.resizing.store(false, Ordering::Relaxed);
        // Resize-time `on_scroll` callbacks are ignored. The top line is derived
        // from the absolute content bottom so sparse terminals do not gain
        // phantom history when their viewport shrinks.
        self.discarded.store(0, Ordering::Relaxed);
        let used_bottom = self.content_bottom.max(self.origin.load(Ordering::Relaxed));
        let next = (used_bottom - i64::from(self.rows)).max(0);
        self.origin.store(next, Ordering::Relaxed);
        self.last_scrollback = self.parser.screen().scrollback_len();
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let max = self.parser.screen().scrollback_len();
        let next = self
            .parser
            .screen()
            .scrollback()
            .saturating_add(lines)
            .min(max);
        self.parser.screen_mut().set_scrollback(next);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let next = self.parser.screen().scrollback().saturating_sub(lines);
        self.parser.screen_mut().set_scrollback(next);
    }

    pub fn scroll_top(&mut self) {
        let max = self.parser.screen().scrollback_len();
        self.parser.screen_mut().set_scrollback(max);
    }

    pub fn scroll_bottom(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }

    pub fn scroll_position(&self) -> (usize, usize) {
        (
            self.parser.screen().scrollback(),
            self.parser.screen().scrollback_len(),
        )
    }

    pub fn plain_text(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn shell_snapshot(&self) -> ShellSnapshot {
        self.shell
            .lock()
            .map(|state| state.snapshot())
            .unwrap_or_default()
    }

    pub fn reset_metadata(&mut self) {
        self.pending_osc.clear();
        self.active_link = None;
        self.links.clear();
        self.scanner = OutputScanner::default();
        self.origin.store(0, Ordering::Relaxed);
        self.discarded.store(0, Ordering::Relaxed);
        self.last_scrollback = self.parser.screen().scrollback_len();
        self.content_bottom = 0;
        if let Ok(mut shell) = self.shell.lock() {
            shell.clear();
        }
    }

    /// Absolute line currently displayed at viewport row `row`.
    pub fn visible_line(&self, row: u16) -> i64 {
        self.origin.load(Ordering::Relaxed) - self.parser.screen().scrollback() as i64
            + i64::from(row)
    }

    pub fn link_at(&self, row: u16, col: u16) -> Option<&str> {
        self.links
            .get(&(self.visible_line(row), col))
            .map(|link| link.as_ref())
    }

    fn position_for_line(&mut self, line: i64) -> Option<(u16, usize)> {
        let origin = self.origin.load(Ordering::Relaxed);
        let screen = self.parser.screen();
        let scrollback = screen.scrollback_len() as i64;
        let rows = i64::from(self.rows);
        if line < origin - scrollback || line >= origin + rows {
            return None;
        }
        let saved = screen.scrollback();
        let (offset, row) = if line >= origin {
            (0, (line - origin) as u16)
        } else {
            ((origin - line) as usize, 0)
        };
        self.parser.screen_mut().set_scrollback(offset);
        Some((row, saved))
    }

    /// Extract text between two absolute cells. Trailing blanks are trimmed per
    /// line and wrapped rows do not gain a synthetic newline.
    pub fn text_between(&mut self, start: Point, end: Point) -> String {
        let (start, end) = Selection::new(start, end).normalized();
        if end < start {
            return String::new();
        }
        let saved = self.parser.screen().scrollback();
        let mut out = String::new();
        let mut line = start.line;
        while line <= end.line {
            let Some((row, _)) = self.position_for_line(line) else {
                line += 1;
                continue;
            };
            let (first, last) = if line == start.line {
                (
                    start.col.min(self.cols.saturating_sub(1)),
                    self.cols.saturating_sub(1),
                )
            } else {
                (0, self.cols.saturating_sub(1))
            };
            let last = if line == end.line {
                end.col.min(last)
            } else {
                last
            };
            let wrapped = self.parser.screen().row_wrapped(row);
            let mut text = String::new();
            for col in first..=last {
                let Some(cell) = self.parser.screen().cell(row, col) else {
                    break;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                if cell.contents().is_empty() {
                    text.push(' ');
                } else {
                    text.push_str(cell.contents());
                }
            }
            out.push_str(text.trim_end());
            if line < end.line && !wrapped {
                out.push('\n');
            }
            line += 1;
        }
        self.parser.screen_mut().set_scrollback(saved);
        out
    }

    pub fn line_text(&mut self, line: i64) -> String {
        self.text_between(
            Point::new(line, 0),
            Point::new(line, self.cols.saturating_sub(1)),
        )
    }

    /// Double-click selection. Wide continuation cells are represented as
    /// spaces so a returned column always matches the rendered cell grid.
    pub fn word_selection(&mut self, line: i64, col: u16) -> Option<Selection> {
        let saved = self.parser.screen().scrollback();
        let (row, _) = self.position_for_line(line)?;
        let mut chars = Vec::with_capacity(usize::from(self.cols));
        for column in 0..self.cols {
            match self.parser.screen().cell(row, column) {
                Some(cell) if !cell.is_wide_continuation() => {
                    let mut iter = cell.contents().chars();
                    chars.push(iter.next().unwrap_or(' '));
                }
                _ => chars.push(' '),
            }
        }
        let bounds = word_bounds(&chars, col);
        self.parser.screen_mut().set_scrollback(saved);
        bounds.map(|(start, end)| Selection::new(Point::new(line, start), Point::new(line, end)))
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn partial_suffix(haystack: &[u8], needle: &[u8]) -> usize {
    let max = haystack.len().min(needle.len().saturating_sub(1));
    (1..=max)
        .rev()
        .find(|len| haystack[haystack.len() - len..] == needle[..*len])
        .unwrap_or(0)
}

fn osc_terminator(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            0x07 => return Some((index, index + 1)),
            0x1b if index + 1 < bytes.len() && bytes[index + 1] == b'\\' => {
                return Some((index, index + 2));
            }
            _ => index += 1,
        }
    }
    None
}

/// Split text into search-highlighted fragments. ASCII matching is
/// case-insensitive; non-ASCII falls back to an exact match rather than
/// misindexing lowercased bytes.
pub fn highlight_parts(text: &str, query: &str) -> Vec<(String, bool)> {
    if query.is_empty() || text.is_empty() {
        return vec![(text.to_string(), false)];
    }
    let (haystack, needle) = if text.is_ascii() && query.is_ascii() {
        (text.to_ascii_lowercase(), query.to_ascii_lowercase())
    } else {
        (text.to_string(), query.to_string())
    };
    if needle.is_empty() {
        return vec![(text.to_string(), false)];
    }
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find(&needle) {
        let start = cursor + found;
        let end = start + needle.len();
        if start > cursor {
            parts.push((text[cursor..start].to_string(), false));
        }
        parts.push((text[start..end].to_string(), true));
        cursor = end;
    }
    if cursor < text.len() {
        parts.push((text[cursor..].to_string(), false));
    }
    if parts.is_empty() {
        parts.push((text.to_string(), false));
    }
    parts
}

/// Split text into plain and clickable URL fragments. Only explicit HTTP(S)
/// and mailto links are recognized; OSC 8 URIs go through [`safe_link`].
pub fn link_parts(text: &str) -> Vec<(String, Option<String>)> {
    let mut parts = Vec::new();
    let mut rest = text;
    loop {
        let next = ["https://", "http://", "mailto:"]
            .into_iter()
            .filter_map(|prefix| rest.find(prefix).map(|index| (index, prefix)))
            .min_by_key(|(index, _)| *index);
        let Some((index, _)) = next else {
            if !rest.is_empty() {
                parts.push((rest.to_string(), None));
            }
            break;
        };
        if index > 0 {
            parts.push((rest[..index].to_string(), None));
        }
        let tail = &rest[index..];
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ')', ']', ';', ':']);
        if url.is_empty() {
            parts.push((tail.to_string(), None));
            break;
        }
        parts.push((url.to_string(), Some(url.to_string())));
        rest = &tail[url.len()..];
    }
    parts
}

/// Explicit allowlist for OSC 8 and plain terminal links.
pub fn safe_link(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if uri.is_empty() || uri.chars().any(char::is_control) {
        return None;
    }
    let lower = uri.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        .then(|| uri.to_string())
}

pub fn count_matches(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    let (haystack, needle) = if text.is_ascii() && query.is_ascii() {
        (text.to_ascii_lowercase(), query.to_ascii_lowercase())
    } else {
        (text.to_string(), query.to_string())
    };
    if needle.is_empty() {
        0
    } else {
        haystack.matches(&needle).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_a_pty_and_returns_output_and_exit_status() {
        let temp = std::env::temp_dir().join(format!(
            "dunlin-terminal-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&temp).unwrap();
        let mut options = SpawnOptions::command(
            temp.clone(),
            "/bin/sh",
            vec!["-c".into(), "printf 'hello\\n'; exit 7".into()],
        );
        options.rows = 12;
        options.cols = 80;
        let session = Session::spawn(options).unwrap();
        let output = session.output();
        let exit = session.exit();
        let mut bytes = Vec::new();
        while let Ok(chunk) = output.recv_blocking() {
            bytes.extend_from_slice(&chunk);
        }
        let status = exit.recv_blocking().unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("hello"));
        assert_eq!(status.code, Some(7));
        assert!(!status.success);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn screen_parses_ansi_and_bounded_scrollback() {
        let mut screen = Screen::new(3, 20, 100);
        screen.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        assert!(screen.plain_text().contains("five"));
        assert!(screen.scrollback_capacity() >= 100);
        screen.scroll_up(2);
        assert!(screen.scroll_position().0 > 0);
        screen.scroll_bottom();
        assert_eq!(screen.scroll_position().0, 0);
    }

    #[test]
    fn terminal_defaults_are_bounded() {
        let options = SpawnOptions::shell(std::env::temp_dir()).unwrap();
        assert!(options.rows >= 2);
        assert!(options.cols >= 2);
        assert!(options.scrollback >= 100);
    }

    fn uuid_like() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0)
    }

    #[test]
    fn terminal_search_highlights_case_insensitively_and_links_trim_punctuation() {
        assert_eq!(
            highlight_parts("Hello hello", "hello"),
            vec![
                ("Hello".into(), true),
                (" ".into(), false),
                ("hello".into(), true)
            ]
        );
        assert_eq!(highlight_parts("abc", ""), vec![("abc".into(), false)]);
        assert_eq!(count_matches("Needle needle", "NEEDLE"), 2);
        let links = link_parts("see https://example.test/a). now");
        assert_eq!(links[1].1.as_deref(), Some("https://example.test/a"));
        assert_eq!(links[0].0, "see ");
        assert_eq!(safe_link("javascript:alert(1)"), None);
        assert_eq!(
            safe_link("https://example.test").as_deref(),
            Some("https://example.test")
        );
    }

    #[test]
    fn osc8_links_survive_split_chunks_and_never_allow_script_urls() {
        let mut screen = Screen::new(4, 40, 100);
        screen.process(b"one \x1b]8;;https://exa");
        screen.process(b"mple.test/a\x07link\x1b]8;;\x07 two");
        assert_eq!(screen.link_at(0, 4), Some("https://example.test/a"));
        assert_eq!(screen.link_at(0, 7), Some("https://example.test/a"));
        assert_eq!(screen.link_at(0, 8), None);
        screen.process(b"\x1b]8;;javascript:alert(1)\x07bad");
        assert_eq!(screen.link_at(0, 12), None);
        screen.process(b"\x1b]8;;https://ok.test\x07ok");
        let row = screen.visible_line(0);
        assert_eq!(row, 0);
        assert_eq!(screen.link_at(0, 15), Some("https://ok.test"));
    }

    #[test]
    fn hyperlinks_are_cleared_by_erase_sequences() {
        let mut screen = Screen::new(3, 30, 100);
        screen.process(b"\x1b]8;;https://example.test\x07linked\x1b]8;;\x07");
        assert_eq!(screen.link_at(0, 0), Some("https://example.test"));
        screen.process(b"\x1b[K");
        assert_eq!(screen.link_at(0, 0), None);
    }

    #[test]
    fn hyperlink_on_the_final_column_is_retained() {
        let mut screen = Screen::new(2, 5, 100);
        screen.process(b"\x1b]8;;https://edge.test\x07abcde\x1b]8;;\x07");
        assert_eq!(screen.link_at(0, 3), Some("https://edge.test"));
        assert_eq!(screen.link_at(0, 4), Some("https://edge.test"));
    }

    #[test]
    fn full_reset_clears_origin_links_and_shell_metadata() {
        let mut screen = Screen::new(3, 30, 2);
        for index in 0..8 {
            screen.process(format!("line-{index}\r\n").as_bytes());
        }
        screen.process(b"\x1b]8;;https://example.test\x07linked\x1b]8;;\x07\x1b]133;A\x07");
        screen.process(b"\x1bc");
        assert_eq!(screen.visible_line(0), 0);
        assert_eq!(screen.link_at(0, 0), None);
        assert!(!screen.shell_snapshot().integration);
        screen.process(b"fresh");
        assert_eq!(screen.visible_line(0), 0);
    }

    #[test]
    fn osc133_tracks_cwd_command_and_exit_code() {
        let mut screen = Screen::new(8, 60, 100);
        screen.process(b"\x1b]7;file://host/tmp/a%20b\x07");
        assert_eq!(screen.shell_snapshot().cwd.as_deref(), Some("/tmp/a b"));
        screen.process(b"\x1b]133;A\x07$ \x1b]133;B\x07printf 'hi'\r\n");
        screen.process(b"\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07");
        let shell = screen.shell_snapshot();
        assert!(shell.integration);
        assert_eq!(shell.blocks.len(), 1);
        assert_eq!(shell.blocks[0].command, "printf 'hi'");
        assert_eq!(shell.blocks[0].exit_code, Some(0));
    }

    #[test]
    fn selection_extraction_reads_viewport_and_scrollback() {
        let mut screen = Screen::new(3, 20, 100);
        screen.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        let start = screen.visible_line(0) - 3;
        let text = screen.text_between(Point::new(start, 0), Point::new(start + 1, 3));
        assert_eq!(text, "one\ntwo");
        assert_eq!(screen.word_selection(start, 1).unwrap().end().col, 2);
        assert!(screen.line_text(start + 2).contains("three"));
    }

    #[test]
    fn absolute_lines_keep_moving_after_scrollback_capacity_is_reached() {
        let mut screen = Screen::new(3, 20, 2);
        for index in 0..12 {
            screen.process(format!("line-{index}\r\n").as_bytes());
        }
        let first = screen.visible_line(0);
        assert!(first > 0);
        screen.process(b"\x1b]8;;https://stable.test\x07linked\x1b]8;;\x07\r\n");
        assert!(screen.visible_line(0) > first);
        let linked = (0..screen.rows())
            .find(|row| screen.link_at(*row, 0) == Some("https://stable.test"))
            .expect("hyperlink should stay on its visible cell");
        assert!(
            screen
                .line_text(screen.visible_line(linked))
                .contains("linked")
        );
    }

    #[test]
    fn absolute_lines_follow_viewport_height_changes() {
        let mut screen = Screen::new(3, 20, 100);
        for index in 0..10 {
            screen.process(format!("row-{index}\r\n").as_bytes());
        }
        let top = screen.visible_line(0);
        screen.resize(6, 20);
        let taller = screen.visible_line(0);
        assert_eq!(taller, (top - 3).max(0));
        screen.resize(3, 20);
        assert_eq!(screen.visible_line(0), top);
    }

    #[test]
    fn shrinking_a_sparse_terminal_does_not_create_phantom_history() {
        let mut screen = Screen::new(10, 20, 100);
        for index in 0..6 {
            screen.process(format!("row-{index}\r\n").as_bytes());
        }
        screen.resize(5, 20);
        assert_eq!(screen.visible_line(0), 2);
        assert_eq!(screen.line_text(screen.visible_line(0)).trim(), "row-2");
        screen.resize(10, 20);
        assert_eq!(screen.visible_line(0), 0);
    }

    #[test]
    fn ssh_command_is_quoted_and_validated() {
        let options = SpawnOptions::ssh(
            "dev@example.test",
            std::env::temp_dir(),
            Some("/srv/my project"),
        )
        .unwrap();
        assert!(options.args.iter().any(|arg| arg == "dev@example.test"));
        assert!(
            options
                .args
                .iter()
                .any(|arg| arg.contains("cd '/srv/my project' && exec"))
        );
        assert!(SpawnOptions::ssh("-oProxyCommand=evil", std::env::temp_dir(), None).is_err());
        assert!(SpawnOptions::ssh("two@hosts@example", std::env::temp_dir(), None).is_err());
    }

    #[test]
    fn shell_integration_is_local_shell_only() {
        let options = SpawnOptions::shell(std::env::temp_dir()).unwrap();
        assert!(options.shell_integration);
        let run = SpawnOptions::command(std::env::temp_dir(), "/bin/sh", Vec::new());
        assert!(!run.shell_integration);
    }

    #[test]
    fn ssh_identity_and_port_are_explicit_validated_and_do_not_reuse_other_credentials() {
        let key = std::env::temp_dir().join(format!(
            "dunlin-key-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&key)
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let result = SpawnOptions::ssh_with_identity(
            "dev@example.test",
            std::env::temp_dir(),
            Some("/srv/my project's"),
            Some(&key),
            Some(2222),
        );
        let options = result.unwrap();
        assert_eq!(&options.args[..2], &["-p", "2222"]);
        for expected in ["IdentitiesOnly=yes", "ControlMaster=no", "ControlPath=none"] {
            assert!(options.args.iter().any(|arg| arg == expected));
        }
        assert!(options.env.is_empty());
        assert!(
            !options
                .args
                .iter()
                .any(|arg| arg.contains("StrictHostKeyChecking=no"))
        );
        assert!(
            SpawnOptions::ssh_with_identity("host", std::env::temp_dir(), None, None, Some(0))
                .is_err()
        );
        assert!(
            SpawnOptions::ssh_with_identity(
                "host",
                std::env::temp_dir(),
                None,
                Some(Path::new("relative")),
                None
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(
                SpawnOptions::ssh_with_identity(
                    "host",
                    std::env::temp_dir(),
                    None,
                    Some(&key),
                    None
                )
                .is_err()
            );
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .unwrap();
            let link = key.with_extension("link");
            std::os::unix::fs::symlink(&key, &link).unwrap();
            assert!(
                SpawnOptions::ssh_with_identity(
                    "host",
                    std::env::temp_dir(),
                    None,
                    Some(&link),
                    None
                )
                .is_err()
            );
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_file(key).unwrap();
    }

    #[test]
    fn zsh_shell_integration_emits_a_finished_block_when_zsh_is_available() {
        if !Path::new("/bin/zsh").is_file() {
            return;
        }
        let temp = std::env::temp_dir().join(format!(
            "dunlin-zsh-integration-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join(".zshrc"), "").unwrap();
        let mut options = SpawnOptions::shell(temp.clone()).unwrap();
        options.program = "/bin/zsh".into();
        options.args = vec!["-l".into()];
        options.shell_integration = true;
        options
            .env
            .push(("HOME".into(), temp.to_string_lossy().to_string()));
        options.rows = 24;
        options.cols = 96;
        let session = Session::spawn(options).unwrap();
        let output = session.output();
        let mut screen = Screen::new(24, 96, 1000);
        session.write(b"echo DUNLIN_E2E\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut finished = false;
        while std::time::Instant::now() < deadline {
            match output.try_recv() {
                Ok(chunk) => screen.process(&chunk),
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
            if screen.shell_snapshot().blocks.iter().any(|block| {
                block.finished()
                    && block.exit_code == Some(0)
                    && block.command.trim() == "echo DUNLIN_E2E"
            }) {
                finished = true;
                break;
            }
        }
        let _ = session.write(b"exit\n");
        let _ = std::fs::remove_dir_all(&temp);
        assert!(finished, "zsh did not emit a finished OSC 133 block");
    }

    #[test]
    fn bash_shell_integration_emits_a_finished_block_when_bash_is_available() {
        if !Path::new("/bin/bash").is_file() {
            return;
        }
        let temp = std::env::temp_dir().join(format!(
            "dunlin-bash-integration-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join(".bashrc"), "").unwrap();
        let mut options = SpawnOptions::shell(temp.clone()).unwrap();
        options.program = "/bin/bash".into();
        options.shell_integration = true;
        options
            .env
            .push(("HOME".into(), temp.to_string_lossy().to_string()));
        options.rows = 24;
        options.cols = 96;
        let session = Session::spawn(options).unwrap();
        let output = session.output();
        let mut screen = Screen::new(24, 96, 1000);
        session.write(b"echo DUNLIN_BASH_E2E\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut finished = false;
        while std::time::Instant::now() < deadline {
            match output.try_recv() {
                Ok(chunk) => screen.process(&chunk),
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
            if screen.shell_snapshot().blocks.iter().any(|block| {
                block.finished()
                    && block.exit_code == Some(0)
                    && block.command.trim() == "echo DUNLIN_BASH_E2E"
            }) {
                finished = true;
                break;
            }
        }
        let _ = session.write(b"exit\n");
        let _ = std::fs::remove_dir_all(&temp);
        assert!(finished, "bash did not emit a finished OSC 133 block");
    }
}
