//! Local PTY terminals. Sessions are explicit, bounded and never resumed after
//! a restart. Remote projects have no hidden local fallback.
use crate::app::DunlinApp;
use dunlin_domain::Project;
use dunlin_terminal::{
    Color, Exit, Screen, Session, SpawnOptions, count_matches, highlight_parts, link_parts,
};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::Input;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Running,
    Exited(Exit),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Shell,
    Run { record: String },
}

pub(crate) struct Tab {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub screen: Screen,
    pub status: Status,
    pub exit: Option<Exit>,
    pub origin: Origin,
    session: Option<Session>,
}

pub(crate) struct State {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub error: Option<String>,
    next: u64,
    pub last_size: (u16, u16),
    pub focus: FocusHandle,
    pub search_open: bool,
}

impl State {
    pub fn new(cx: &mut Context<DunlinApp>) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            error: None,
            next: 1,
            last_size: (dunlin_terminal::DEFAULT_ROWS, dunlin_terminal::DEFAULT_COLS),
            focus: cx.focus_handle(),
            search_open: false,
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let active = self.active;
        self.tabs.get_mut(active)
    }
    pub fn tab_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }
    fn next_id(&mut self) -> String {
        let id = format!("term-{}", self.next);
        self.next = self.next.wrapping_add(1);
        id
    }
}

fn shell_options(project: &Project) -> Result<SpawnOptions, String> {
    if project.is_remote() {
        return Err(
            "Remote PTY is not available. Dunlin does not run a local shell for a relay project."
                .into(),
        );
    }
    if !project.trusted {
        return Err("Trust this project before opening a local terminal.".into());
    }
    if !PathBuf::from(&project.path).is_dir() {
        return Err("Project folder is not available".into());
    }
    let mut options = SpawnOptions::shell(PathBuf::from(&project.path))?;
    options.rows = dunlin_terminal::DEFAULT_ROWS;
    options.cols = dunlin_terminal::DEFAULT_COLS;
    Ok(options)
}

impl DunlinApp {
    pub(crate) fn open_terminal_tab(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let project = self.project().clone();
        let options = shell_options(&project)?;
        let title = format!("Shell · {}", project.name);
        let tab = self.terminal_spawn(options, title, project.path.clone(), Origin::Shell, cx)?;
        self.terminal.active = self
            .terminal
            .tabs
            .iter()
            .position(|candidate| candidate.id == tab)
            .unwrap_or(0);
        self.terminal.error = None;
        cx.notify();
        Ok(())
    }

    pub(crate) fn open_run_terminal(
        &mut self,
        window: &mut Window,
        candidate: dunlin_workspace::run::RunCandidate,
        record_id: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let project = self.project().clone();
        if project.is_remote() {
            return Err("Run is local-only until the relay exposes a PTY grant".into());
        }
        if !project.trusted {
            return Err("Trust this project before running a detected command".into());
        }
        let cwd = if candidate.cwd == "." {
            PathBuf::from(&project.path)
        } else {
            let joined = PathBuf::from(&project.path).join(&candidate.cwd);
            let canonical = std::fs::canonicalize(&joined)
                .map_err(|_| "Run working directory is not available".to_string())?;
            let root = std::fs::canonicalize(&project.path)
                .map_err(|_| "Project folder is not available".to_string())?;
            if !canonical.starts_with(root) {
                return Err("Run working directory escapes the project".into());
            }
            canonical
        };
        let mut options = SpawnOptions::command(
            cwd.clone(),
            candidate.program.clone(),
            candidate.args.clone(),
        );
        if let Ok(settings) = self.resolved_project_settings(&project)
            && let Ok(environment) = settings.process_environment()
        {
            options.env = environment.into_iter().collect();
        }
        let title = candidate.label.chars().take(48).collect::<String>();
        self.terminal_spawn(
            options,
            title,
            cwd.to_string_lossy().to_string(),
            Origin::Run { record: record_id },
            cx,
        )?;
        self.terminal.active = self.terminal.tabs.len().saturating_sub(1);
        self.terminal.error = None;
        self.bottom_panel = crate::app::BottomPanel::Terminal;
        self.persist_settings(cx);
        self.terminal.focus.focus(window, cx);
        cx.notify();
        Ok(())
    }

    fn terminal_stream(
        &mut self,
        id: String,
        output: async_channel::Receiver<Vec<u8>>,
        exit: async_channel::Receiver<Exit>,
        cx: &mut Context<Self>,
    ) {
        let output_id = id.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(chunk) = output.recv().await {
                let keep = this
                    .update(cx, |this, cx| {
                        let Some(tab) = this.terminal.tab_mut(&output_id) else {
                            return false;
                        };
                        tab.screen.process(&chunk);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            if let Ok(exit) = exit.recv().await {
                let _ = this.update(cx, |this, cx| {
                    let (record, title) = match this.terminal.tab_mut(&id) {
                        Some(tab) => {
                            tab.exit = Some(exit.clone());
                            tab.status = Status::Exited(exit.clone());
                            match &tab.origin {
                                Origin::Run { record } => (Some(record.clone()), None),
                                Origin::Shell => (None, Some(tab.title.clone())),
                            }
                        }
                        None => (None, None),
                    };
                    if let Some(record) = record {
                        this.finish_run(&record, Some(exit.clone()), cx);
                    } else if let Some(title) = title {
                        this.set_notice(
                            format!(
                                "{title} exited ({})",
                                exit.code
                                    .map(|code| format!("code {code}"))
                                    .or_else(|| exit.signal.clone())
                                    .unwrap_or_else(|| "unknown status".into())
                            ),
                            cx,
                        );
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn terminal_spawn(
        &mut self,
        options: SpawnOptions,
        title: String,
        cwd: String,
        origin: Origin,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let rows = options.rows;
        let cols = options.cols;
        let session = Session::spawn(options)?;
        let output = session.output();
        let exit = session.exit();
        let id = self.terminal.next_id();
        self.terminal.tabs.push(Tab {
            id: id.clone(),
            title,
            cwd,
            screen: Screen::new(rows, cols, dunlin_terminal::DEFAULT_SCROLLBACK),
            status: Status::Running,
            exit: None,
            origin,
            session: Some(session),
        });
        self.terminal.active = self.terminal.tabs.len().saturating_sub(1);
        self.terminal_stream(id.clone(), output, exit, cx);
        Ok(id)
    }

    pub(crate) fn close_terminal_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.terminal.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = self.terminal.tabs.remove(index);
        if let Some(session) = &tab.session {
            session.kill();
        }
        if let Origin::Run { record } = &tab.origin
            && matches!(tab.status, Status::Running)
        {
            self.finish_run(record, None, cx);
        }
        self.terminal.active = self
            .terminal
            .active
            .min(self.terminal.tabs.len().saturating_sub(1));
        cx.notify();
    }

    pub(crate) fn terminal_write(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        tab.screen.scroll_bottom();
        if let Some(session) = &tab.session
            && let Err(error) = session.write(bytes)
        {
            tab.status = Status::Failed(error.clone());
            self.terminal.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn terminal_paste(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        tab.screen.scroll_bottom();
        if let Some(session) = &tab.session
            && let Err(error) = session.paste(text)
        {
            tab.status = Status::Failed(error.clone());
            self.terminal.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn terminal_copy(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab() else {
            return;
        };
        let mut text = tab.screen.plain_text();
        if text.len() > 1024 * 1024 {
            text.truncate(1024 * 1024);
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_notice("Copied terminal text", cx);
    }

    pub(crate) fn terminal_scroll(&mut self, up: bool, lines: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        if up {
            tab.screen.scroll_up(lines);
        } else {
            tab.screen.scroll_down(lines);
        }
        cx.notify();
    }

    pub(crate) fn terminal_clear(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        tab.screen.process(b"\x1bc");
        if let Some(session) = &tab.session {
            let _ = session.write(b"\x0c");
        }
        cx.notify();
    }

    pub(crate) fn terminal_resize_to(&mut self, width: f32, height: f32) {
        let (rows, cols) = terminal_size(width, height);
        if (rows, cols) == self.terminal.last_size {
            return;
        }
        self.terminal.last_size = (rows, cols);
        for tab in &mut self.terminal.tabs {
            tab.screen.resize(rows, cols);
            if let Some(session) = &tab.session {
                let _ = session.resize(rows, cols);
            }
        }
    }

    pub(crate) fn terminal_handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let platform = keystroke.modifiers.platform;
        let control = keystroke.modifiers.control;
        if platform {
            match keystroke.key.as_str() {
                "c" => {
                    self.terminal_copy(cx);
                    return true;
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.terminal_paste(&text, cx);
                    }
                    return true;
                }
                "w" => {
                    if let Some(id) = self.terminal.active_tab().map(|tab| tab.id.clone()) {
                        self.close_terminal_tab(&id, cx);
                        return true;
                    }
                }
                "f" => {
                    self.terminal.search_open = true;
                    self.terminal_search
                        .update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                    return true;
                }
                _ => {}
            }
        }
        if platform {
            return false;
        }
        if control {
            let byte = match keystroke.key.as_str() {
                "c" => Some(0x03),
                "d" => Some(0x04),
                "z" => Some(0x1a),
                "l" => Some(0x0c),
                "a" => Some(0x01),
                "e" => Some(0x05),
                "u" => Some(0x15),
                "k" => Some(0x0b),
                "w" => Some(0x17),
                _ => None,
            };
            if let Some(byte) = byte {
                self.terminal_write(&[byte], cx);
                return true;
            }
        }
        let bytes: Option<Vec<u8>> = match keystroke.key.as_str() {
            "enter" | "return" => Some(b"\r".to_vec()),
            "tab" => Some(b"\t".to_vec()),
            "backspace" => Some(vec![0x7f]),
            "escape" => Some(vec![0x1b]),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "insert" => Some(b"\x1b[2~".to_vec()),
            "up" => Some(b"\x1b[A".to_vec()),
            "down" => Some(b"\x1b[B".to_vec()),
            "right" => Some(b"\x1b[C".to_vec()),
            "left" => Some(b"\x1b[D".to_vec()),
            "home" => Some(b"\x1b[H".to_vec()),
            "end" => Some(b"\x1b[F".to_vec()),
            "pageup" => {
                self.terminal_scroll(true, 10, cx);
                return true;
            }
            "pagedown" => {
                self.terminal_scroll(false, 10, cx);
                return true;
            }
            _ => None,
        };
        if let Some(bytes) = bytes {
            self.terminal_write(&bytes, cx);
            return true;
        }
        if !keystroke.modifiers.control && !keystroke.modifiers.alt {
            if let Some(text) = keystroke.key_char.as_deref().or_else(|| {
                (keystroke.key.chars().count() == 1 && keystroke.key != " ")
                    .then_some(keystroke.key.as_str())
            }) {
                self.terminal_write(text.as_bytes(), cx);
                self.terminal.focus.focus(window, cx);
                return true;
            }
            if keystroke.key == "space" {
                self.terminal_write(b" ", cx);
                return true;
            }
        }
        false
    }
}

pub(crate) fn terminal_size(width: f32, height: f32) -> (u16, u16) {
    let cols = ((width - 24.) / 7.1).clamp(24., 220.) as u16;
    let rows = ((height - 78.) / 15.5).clamp(6., 70.) as u16;
    (rows, cols)
}

fn color(value: Color, default: Hsla) -> Hsla {
    match value {
        Color::Default => default,
        Color::Rgb(r, g, b) => {
            rgba((u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | 0xff).into()
        }
        Color::Idx(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> Hsla {
    const ANSI: [u32; 16] = [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    match index {
        0..=15 => rgb(ANSI[index as usize]).into(),
        16..=231 => {
            let value = index - 16;
            let r = value / 36;
            let g = (value % 36) / 6;
            let b = value % 6;
            rgb(((u32::from(r) * 51) << 16) | ((u32::from(g) * 51) << 8) | (u32::from(b) * 51))
                .into()
        }
        232..=255 => {
            let level = 8 + (u32::from(index - 232) * 10);
            rgb((level << 16) | (level << 8) | level).into()
        }
    }
}

fn rgba(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 24) & 0xff) as f32 / 255.,
        g: ((value >> 16) & 0xff) as f32 / 255.,
        b: ((value >> 8) & 0xff) as f32 / 255.,
        a: (value & 0xff) as f32 / 255.,
    }
}

#[derive(Clone)]
struct Run {
    text: String,
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
}

fn row_element(
    tab: &Tab,
    row: u16,
    cursor: Option<(u16, u16)>,
    hide_cursor: bool,
    query: &str,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let screen = tab.screen.parser().screen();
    let cols = tab.screen.cols();
    let mut runs: Vec<Run> = Vec::new();
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let mut fg = color(cell.fgcolor(), p.fg);
        let mut bg = color(cell.bgcolor(), p.bg);
        if cell.inverse() {
            std::mem::swap(&mut fg, &mut bg);
        }
        if !hide_cursor && cursor == Some((row, col)) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let contents = cell.contents().to_string();
        if contents.is_empty() && (hide_cursor || cursor != Some((row, col))) {
            continue;
        }
        let contents = if contents.is_empty() {
            " ".into()
        } else {
            contents
        };
        let background = (bg != p.bg).then_some(bg);
        if let Some(last) = runs.last_mut()
            && last.fg == fg
            && last.bg == background
            && last.bold == cell.bold()
        {
            last.text.push_str(&contents);
        } else {
            runs.push(Run {
                text: contents,
                fg,
                bg: background,
                bold: cell.bold(),
            });
        }
    }
    let mut line = div()
        .h(px(15.))
        .w_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    let mut link_index = 0usize;
    for run in runs {
        for (part, highlighted) in highlight_parts(&run.text, query) {
            for (piece, link) in link_parts(&part) {
                if piece.is_empty() {
                    continue;
                }
                let handled = link.clone();
                let base = div()
                    .text_size(px(11.5))
                    .line_height(px(15.))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(if link.is_some() { p.blue } else { run.fg })
                    .when_some(run.bg, |this, bg| this.bg(bg))
                    .when(highlighted, |this| this.bg(p.amber_soft))
                    .when(run.bold, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(piece);
                let span = if let Some(url) = handled {
                    link_index += 1;
                    base.id(format!("terminal-link:{row}:{link_index}"))
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                        .into_any_element()
                } else {
                    base.into_any_element()
                };
                line = line.child(span);
            }
        }
    }
    line.into_any_element()
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let viewport_width = f32::from(window.viewport_size().width);
    let viewport_height = f32::from(window.viewport_size().height);
    match this.terminal_dock {
        crate::app::TerminalDock::Bottom => {
            this.terminal_resize_to(viewport_width - 330., this.bottom_panel_height)
        }
        crate::app::TerminalDock::Left | crate::app::TerminalDock::Right => {
            this.terminal_resize_to(this.bottom_panel_width, viewport_height - 140.)
        }
    }
    let p = palette(cx);
    if this.project().is_remote() {
        return super::views::empty_activity(
            "Remote terminal is not available",
            "The relay does not yet expose a PTY grant. Dunlin does not start a local shell for a remote project.",
            IconName::Terminal,
            cx,
        );
    }
    let mut toolbar = div()
        .h(px(38.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(p.border)
        .bg(p.subtle);
    for (index, tab) in this.terminal.tabs.iter().enumerate() {
        let active = index == this.terminal.active;
        let id = tab.id.clone();
        let close = id.clone();
        let status_color = match tab.status {
            Status::Running => p.green,
            Status::Exited(ref exit) if exit.success => p.green,
            Status::Exited(_) | Status::Failed(_) => p.red,
        };
        toolbar = toolbar.child(
            div()
                .id(format!("terminal-tab:{id}"))
                .h(px(25.))
                .max_w(px(230.))
                .px(px(7.))
                .rounded(px(5.))
                .border_1()
                .border_color(if active {
                    p.border_strong
                } else {
                    p.border.opacity(0.)
                })
                .bg(if active { p.card } else { p.bg.opacity(0.) })
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(index) = this.terminal.tabs.iter().position(|tab| tab.id == id) {
                        this.terminal.active = index;
                        cx.notify();
                    }
                }))
                .child(div().size(px(5.)).rounded_full().bg(status_color))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(10.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(if active { p.fg } else { p.muted_fg })
                        .child(tab.title.clone()),
                )
                .child(
                    Button::new(format!("terminal-close:{close}"))
                        .ghost()
                        .small()
                        .label("×")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_terminal_tab(&close, cx);
                        })),
                ),
        );
    }
    toolbar = toolbar
        .child(
            Button::new("terminal-new")
                .ghost()
                .small()
                .label("+")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Err(error) = this.open_terminal_tab(cx) {
                        this.terminal.error = Some(error.clone());
                        this.set_notice(error, cx);
                    }
                })),
        )
        .when(this.terminal.search_open, |bar| {
            let matches = this
                .terminal
                .active_tab()
                .map(|tab| {
                    count_matches(
                        &tab.screen.parser().screen().contents(),
                        &this.terminal_query,
                    )
                })
                .unwrap_or(0);
            bar.child(
                div()
                    .w(px(190.))
                    .child(Input::new(&this.terminal_search).small()),
            )
            .child(
                div()
                    .text_size(px(9.5))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg)
                    .child(format!("{matches} visible")),
            )
            .child(
                Button::new("terminal-search-close")
                    .ghost()
                    .small()
                    .label("×")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.terminal.search_open = false;
                        this.terminal_query.clear();
                        this.terminal_search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
            )
        })
        .child(div().flex_1());
    if let Some(tab) = this.terminal.active_tab() {
        let (position, total) = tab.screen.scroll_position();
        toolbar = toolbar
            .child(
                div()
                    .text_size(px(10.))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg)
                    .child(if total > 0 {
                        format!("scroll {position}/{total}")
                    } else {
                        "live".into()
                    }),
            )
            .child(
                Button::new("terminal-find")
                    .ghost()
                    .small()
                    .label("Find")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.terminal.search_open = true;
                        this.terminal_search
                            .update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-copy")
                    .ghost()
                    .small()
                    .label("Copy")
                    .on_click(cx.listener(|this, _, _, cx| this.terminal_copy(cx))),
            )
            .child(
                Button::new("terminal-clear")
                    .ghost()
                    .small()
                    .label("Clear")
                    .on_click(cx.listener(|this, _, _, cx| this.terminal_clear(cx))),
            );
    }
    let body = if let Some(tab) = this.terminal.active_tab() {
        let screen = tab.screen.parser().screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let hide_cursor = screen.hide_cursor();
        let rows = tab.screen.rows();
        let mut lines = div().flex().flex_col().p(px(8.)).child(
            div()
                .text_size(px(10.))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(format!("{} · {}", tab.title, tab.cwd)),
        );
        for row in 0..rows {
            lines = lines.child(row_element(
                tab,
                row,
                Some((cursor_row, cursor_col)),
                hide_cursor,
                &this.terminal_query,
                &p,
                cx,
            ));
        }
        div()
            .id("terminal-body")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_hidden()
            .track_focus(&this.terminal.focus)
            .on_click(cx.listener(|this, _, window, cx| {
                this.terminal.focus.focus(window, cx);
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.)).y;
                let lines = (delta / px(1.)).abs().max(1.) as usize / 12;
                if delta > px(0.) {
                    this.terminal_scroll(true, lines.max(1), cx);
                } else if delta < px(0.) {
                    this.terminal_scroll(false, lines.max(1), cx);
                }
            }))
            .bg(p.bg)
            .child(lines)
            .into_any_element()
    } else {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(p.muted_fg)
            .child("No terminal tab. Open a local shell to begin.")
            .child(
                Button::new("terminal-empty-new")
                    .small()
                    .label("New local terminal")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.open_terminal_tab(cx) {
                            this.terminal.error = Some(error.clone());
                            this.set_notice(error, cx);
                        }
                    })),
            )
            .into_any_element()
    };
    let mut root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .bg(p.bg)
        .child(toolbar)
        .child(body);
    if let Some(error) = &this.terminal.error {
        root = root.child(
            div()
                .px(px(10.))
                .py(px(4.))
                .text_size(px(10.5))
                .text_color(p.red)
                .child(error.clone()),
        );
    }
    root.into_any_element()
}
