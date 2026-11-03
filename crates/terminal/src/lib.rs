//! Local PTY ownership and a bounded VT screen.
//!
//! The PTY reader and child waiter run on dedicated threads. The UI owns the
//! `vt100::Parser` and processes output in the foreground, so rendering never
//! blocks on a shell. Dropping a session kills its child; there is no hidden
//! relaunch or replay.

pub use vt100::{Cell, Color};

use async_channel::Receiver;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const DEFAULT_ROWS: u16 = 30;
pub const DEFAULT_COLS: u16 = 110;
pub const DEFAULT_SCROLLBACK: usize = 5_000;
pub const MAX_SCROLLBACK: usize = 50_000;
pub const OUTPUT_CHUNK: usize = 32 * 1024;

#[derive(Clone, Debug)]
pub struct SpawnOptions {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
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
        }
    }
}

pub fn default_shell() -> Result<String, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| value.starts_with('/'))
        .filter(|value| Path::new(value).is_file())
        .unwrap_or_else(|| "/bin/zsh".into());
    if !Path::new(&shell).is_file() {
        return Err("No interactive shell was found".into());
    }
    Ok(shell)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Exit {
    pub success: bool,
    pub code: Option<u32>,
    pub signal: Option<String>,
}

pub struct Session {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Receiver<Vec<u8>>,
    exit: Receiver<Exit>,
    pid: Option<u32>,
    _reader: std::thread::JoinHandle<()>,
    _waiter: std::thread::JoinHandle<()>,
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
        let mut command = CommandBuilder::new(&options.program);
        for arg in &options.args {
            command.arg(arg);
        }
        if !options.cwd.is_dir() {
            return Err("Terminal working directory is not available".into());
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
            "DUNLIN_RELAY_SANDBOX",
            "DUNLIN_RELAY_STORE_TRANSCRIPTS",
            "DUNLIN_RELAY_DEBUG",
            "DUNLIN_AUTOPROMPT",
            "DUNLIN_HARNESS_COMMAND",
            "DUNLIN_DATA_DIR",
            "DUNLIN_LOG_FILE",
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
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
            killer: Mutex::new(killer),
            output: output_rx,
            exit: exit_rx,
            pid,
            _reader: reader_thread,
            _waiter: waiter_thread,
        })
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

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "Terminal writer is unavailable".to_string())?;
        writer
            .write_all(bytes)
            .and_then(|_| writer.flush())
            .map_err(|error| format!("Terminal write failed: {error}"))
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
        let master = self
            .master
            .lock()
            .map_err(|_| "Terminal resize is unavailable".to_string())?;
        master
            .resize(PtySize {
                rows: rows.max(2),
                cols: cols.max(2),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Terminal resize failed: {error}"))
    }

    pub fn kill(&self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A bounded terminal screen. The parser is intentionally owned by the caller,
/// not by the reader thread, so output ordering matches UI updates.
pub struct Screen {
    parser: vt100::Parser,
    rows: u16,
    cols: u16,
    scrollback: usize,
}

impl Screen {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        let rows = rows.max(2);
        let cols = cols.max(2);
        let scrollback = scrollback.clamp(100, MAX_SCROLLBACK);
        Self {
            parser: vt100::Parser::new(
                NonZeroU16::new(rows).unwrap_or(NonZeroU16::MIN),
                NonZeroU16::new(cols).unwrap_or(NonZeroU16::MIN),
                scrollback,
            ),
            rows,
            cols,
            scrollback,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn parser(&self) -> &vt100::Parser {
        &self.parser
    }

    pub fn parser_mut(&mut self) -> &mut vt100::Parser {
        &mut self.parser
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
        self.parser.set_size(
            NonZeroU16::new(self.rows).unwrap_or(NonZeroU16::MIN),
            NonZeroU16::new(self.cols).unwrap_or(NonZeroU16::MIN),
        );
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
/// and mailto links are recognized; OSC 8 hyperlinks are not interpreted.
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
    }
}
