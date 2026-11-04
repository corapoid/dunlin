//! OSC 7, OSC 133/633 and OSC 1337 shell metadata.
//!
//! `vt100` exposes unrecognized OSC sequences through `Callbacks`. Prompt
//! markers never change the screen, so keeping their state alongside the parser
//! avoids a second terminal emulator. The desktop reads the snapshot for prompt
//! gutters, command output actions and the current directory.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};

pub(crate) const MAX_BLOCKS: usize = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellBlock {
    pub id: u64,
    /// Absolute line where the prompt started (OSC 133 `A`).
    pub prompt_line: i64,
    /// Absolute line/column where the user line began (OSC 133 `B`).
    pub command_line: Option<i64>,
    pub command_col: u16,
    /// Absolute line where the command started running (OSC 133 `C`).
    pub output_line: Option<i64>,
    /// Absolute line where the next prompt reported completion (OSC 133 `D`).
    pub finished_line: Option<i64>,
    pub command: String,
    pub exit_code: Option<i32>,
}

impl ShellBlock {
    pub fn finished(&self) -> bool {
        self.finished_line.is_some()
    }

    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellSnapshot {
    pub integration: bool,
    pub cwd: Option<String>,
    pub remote_host: Option<String>,
    pub title: Option<String>,
    pub blocks: Vec<ShellBlock>,
}

impl ShellSnapshot {
    pub fn last_block(&self) -> Option<&ShellBlock> {
        self.blocks.last()
    }

    pub fn current_block(&self) -> Option<&ShellBlock> {
        self.blocks.iter().rev().find(|block| !block.finished())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ShellState {
    integration: bool,
    cwd: Option<String>,
    remote_host: Option<String>,
    title: Option<String>,
    blocks: Vec<ShellBlock>,
    current: Option<usize>,
    next_id: u64,
}

impl ShellState {
    pub fn snapshot(&self) -> ShellSnapshot {
        ShellSnapshot {
            integration: self.integration,
            cwd: self.cwd.clone(),
            remote_host: self.remote_host.clone(),
            title: self.title.clone(),
            blocks: self.blocks.clone(),
        }
    }

    pub fn clear(&mut self) {
        self.integration = false;
        self.cwd = None;
        self.remote_host = None;
        self.title = None;
        self.blocks.clear();
        self.current = None;
    }

    fn begin_prompt(&mut self, line: i64) {
        self.integration = true;
        if let Some(index) = self.current
            && let Some(block) = self.blocks.get_mut(index)
            && !block.finished()
        {
            block.finished_line = Some(line);
        }
        if self.blocks.len() >= MAX_BLOCKS {
            let remove = self.blocks.len() - MAX_BLOCKS + 1;
            self.blocks.drain(..remove);
            self.current = self.current.and_then(|index| index.checked_sub(remove));
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.blocks.push(ShellBlock {
            id: self.next_id,
            prompt_line: line,
            ..ShellBlock::default()
        });
        self.current = Some(self.blocks.len() - 1);
    }

    fn begin_command(&mut self, line: i64, col: u16) {
        self.integration = true;
        if self.current.is_none() {
            self.begin_prompt(line);
        }
        if let Some(block) = self.current.and_then(|index| self.blocks.get_mut(index)) {
            block.command_line = Some(line);
            block.command_col = col;
        }
    }

    fn begin_output(&mut self, line: i64) -> Option<(usize, i64, u16)> {
        self.integration = true;
        if self.current.is_none() {
            self.begin_prompt(line);
        }
        let index = self.current?;
        let block = self.blocks.get_mut(index)?;
        block.output_line = Some(line);
        Some((index, block.command_line.unwrap_or(line), block.command_col))
    }

    fn finish_command(&mut self, line: i64, exit_code: Option<i32>) {
        self.integration = true;
        // A `D` before the first prompt (or a duplicate `D`) has no block to
        // close. Do not create an empty history entry for it.
        if let Some(index) = self.current.take()
            && let Some(block) = self.blocks.get_mut(index)
        {
            block.finished_line = Some(line);
            block.exit_code = exit_code;
        }
    }

    fn set_command(&mut self, index: usize, command: String) {
        if let Some(block) = self.blocks.get_mut(index) {
            block.command = command;
        }
    }

    fn osc(&mut self, screen: &mut vt100::Screen, origin: i64, params: &[&[u8]]) {
        match params {
            [b"7", value] => {
                if let Some(path) = file_uri_path(&String::from_utf8_lossy(value)) {
                    self.cwd = Some(path);
                }
            }
            [b"1337", value] => {
                let value = String::from_utf8_lossy(value);
                if let Some(rest) = value.strip_prefix("CurrentDir=") {
                    self.cwd = file_uri_path(rest).or_else(|| Some(rest.to_string()));
                } else if let Some(rest) = value.strip_prefix("RemoteHost=") {
                    self.remote_host = Some(rest.to_string());
                }
            }
            [b"133" | b"633", rest @ ..] => {
                let Some((kind, extra)) = rest.split_first() else {
                    return;
                };
                let kind = kind.first().copied().unwrap_or_default();
                let line = absolute_cursor_line(screen, origin);
                let (_, col) = screen.cursor_position();
                match kind {
                    b'A' => self.begin_prompt(line),
                    b'B' => self.begin_command(line, col),
                    b'C' => {
                        if let Some((index, start_line, start_col)) = self.begin_output(line) {
                            let command = capture_command(screen, start_line, start_col, line, col);
                            self.set_command(index, command);
                        }
                        let _ = extra;
                    }
                    b'D' => {
                        let code = extra
                            .first()
                            .and_then(|value| std::str::from_utf8(value).ok())
                            .and_then(|value| value.parse::<i32>().ok());
                        self.finish_command(line, code);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn absolute_cursor_line(screen: &vt100::Screen, origin: i64) -> i64 {
    let (row, _) = screen.cursor_position();
    // `origin` is the absolute line at viewport row 0 while the live viewport is
    // at the bottom. Prompt markers are emitted there; `scrollback()` only
    // changes what the user is looking at.
    origin + i64::from(row)
}

fn capture_command(
    screen: &vt100::Screen,
    start_line: i64,
    start_col: u16,
    end_line: i64,
    end_col: u16,
) -> String {
    let (rows, cols) = screen.size();
    let rows = rows.get();
    let cols = cols.get();
    let offset = screen.scrollback() as i64;
    let top = screen.scrollback_len() as i64 - offset;
    let start_row = start_line - top;
    let end_row = end_line - top;
    if start_row < 0 || end_row < 0 || start_row >= i64::from(rows) || end_row >= i64::from(rows) {
        return String::new();
    }
    let start_col = start_col.min(cols.saturating_sub(1));
    let end_col = end_col.min(cols.saturating_sub(1));
    screen
        .contents_between(start_row as u16, start_col, end_row as u16, end_col)
        .trim_end()
        .to_string()
}

fn file_uri_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = if let Some(rest) = value.strip_prefix("file://") {
        if rest.starts_with('/') {
            rest
        } else {
            rest.find('/').map(|index| &rest[index..])?
        }
    } else if value.starts_with('/') {
        value
    } else {
        return None;
    };
    Some(percent_decode(path))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// `vt100` callback bridge. `Arc<Mutex<_>>` is cheap and keeps the parser
/// Send-compatible for the UI thread without exposing the lock to callers.
pub(crate) struct ShellCallbacks {
    pub state: Arc<Mutex<ShellState>>,
    /// Absolute line at viewport row 0 when the live viewport is at the bottom.
    pub origin: Arc<AtomicI64>,
    /// Rows discarded from a full scrollback. `Screen` folds this into `origin`
    /// after each parser step.
    pub discarded: Arc<AtomicU64>,
    /// `vt100` emits `on_scroll` while resizing too; that adjustment is applied
    /// explicitly by `Screen::resize` to keep line IDs stable.
    pub resizing: Arc<AtomicBool>,
}

impl vt100::Callbacks for ShellCallbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        if let Ok(mut state) = self.state.lock() {
            state.title = Some(String::from_utf8_lossy(title).trim().to_string());
        }
    }

    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, name: &[u8]) {
        if let Ok(mut state) = self.state.lock()
            && state.title.is_none()
        {
            state.title = Some(String::from_utf8_lossy(name).trim().to_string());
        }
    }

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        let origin = self.origin.load(Ordering::Relaxed);
        if let Ok(mut state) = self.state.lock() {
            state.osc(screen, origin, params);
        }
    }

    fn on_scroll(&mut self, _: vt100::capture::RowContents<'_>, alternate_screen: bool) {
        if !alternate_screen && !self.resizing.load(Ordering::Relaxed) {
            self.discarded.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_uris_and_percent_escapes() {
        assert_eq!(
            file_uri_path("file://host/tmp/a%20b").as_deref(),
            Some("/tmp/a b")
        );
        assert_eq!(file_uri_path("file:///tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(file_uri_path("/tmp/direct").as_deref(), Some("/tmp/direct"));
        assert_eq!(file_uri_path("relative"), None);
    }

    #[test]
    fn block_state_tracks_prompt_command_and_exit() {
        let mut state = ShellState::default();
        state.begin_prompt(10);
        state.begin_command(10, 4);
        state.finish_command(12, Some(7));
        let snapshot = state.snapshot();
        assert!(snapshot.integration);
        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].prompt_line, 10);
        assert_eq!(snapshot.blocks[0].command_line, Some(10));
        assert_eq!(snapshot.blocks[0].exit_code, Some(7));
        assert!(snapshot.blocks[0].finished());
        assert!(snapshot.current_block().is_none());
    }

    #[test]
    fn old_blocks_are_bounded() {
        let mut state = ShellState::default();
        for line in 0..(MAX_BLOCKS as i64 + 20) {
            state.begin_prompt(line);
            state.finish_command(line, Some(0));
        }
        assert_eq!(state.blocks.len(), MAX_BLOCKS);
        assert_eq!(
            state.blocks.last().unwrap().prompt_line,
            MAX_BLOCKS as i64 + 19
        );
    }
}
