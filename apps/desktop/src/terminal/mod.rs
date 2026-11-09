//! Local, relay and SSH terminals. Sessions are explicit, bounded and never
//! resumed after a restart. Remote projects never fall back to a local shell.
use crate::app::DunlinApp;
use dunlin_domain::Project;
use dunlin_terminal::{
    Color, Exit, Point as TermPoint, Screen, Selection, Session, SpawnOptions, count_matches,
    highlight_parts, link_parts, safe_link,
};
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::input::InputState;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

mod layout;
mod relay;

use layout::{Axis, SplitNode};

const ROW_HEIGHT: f32 = 15.;
const CHAR_WIDTH: f32 = 7.1;
const FONT_SIZE: f32 = 11.5;

fn terminal_font(cx: &App) -> Font {
    static FALLBACKS: std::sync::OnceLock<FontFallbacks> = std::sync::OnceLock::new();
    Font {
        fallbacks: Some(
            FALLBACKS
                .get_or_init(|| {
                    let mut names: Vec<String> = cx
                        .text_system()
                        .all_font_names()
                        .into_iter()
                        .filter(|name| symbol_font_candidate(name))
                        .collect();
                    names.sort();
                    names.dedup();
                    names.retain(|name| name != "Symbols Nerd Font Mono");
                    names.push("Symbols Nerd Font Mono".into());
                    FontFallbacks::from_fonts(names)
                })
                .clone(),
        ),
        features: FontFeatures::disable_ligatures(),
        ..font(dunlin_ui::theme::code_font())
    }
}

fn symbol_font_candidate(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("nerd")
        || name.contains("powerline")
        || name.contains("meslo")
        || name.ends_with(" nf")
        || name.ends_with(" nfm")
        || name.ends_with(" nfp")
}

#[cfg(test)]
mod font_tests {
    #[test]
    fn installed_terminal_symbol_families_are_used_without_picking_proportional_ui_fonts() {
        for name in [
            "MesloLGS NF",
            "Hack Nerd Font",
            "Symbols Nerd Font Mono",
            "DejaVu Sans Mono for Powerline",
            "FiraCode NFM",
        ] {
            assert!(super::symbol_font_candidate(name), "{name}");
        }
        for name in ["Arial", "Helvetica", "Geist"] {
            assert!(!super::symbol_font_candidate(name));
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn smoke_glyphs(window: &Window, cx: &App) {
    for text in [
        "\u{e0b0}", "\u{e0a0}", "\u{e718}", "\u{f07c}", "\u{f015}", "界",
    ] {
        let line = window.text_system().shape_line(
            text.into(),
            px(FONT_SIZE),
            &[TextRun {
                len: text.len(),
                font: terminal_font(cx),
                color: palette(cx).fg,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        assert!(!line.runs.is_empty());
        assert!(
            line.runs
                .iter()
                .flat_map(|r| &r.glyphs)
                .all(|g| g.id.0 != 0),
            "missing terminal glyph: {text}"
        );
    }
    assert!(cell_width(cx) > 0.);
}

fn cell_width(cx: &App) -> f32 {
    let font = cx.text_system().resolve_font(&terminal_font(cx));
    cx.text_system()
        .advance(font, crate::workspace_text::size(FONT_SIZE, cx), 'M')
        .map(|advance| f32::from(advance.width))
        .ok()
        .filter(|width| width.is_finite() && *width > 0.)
        .unwrap_or(CHAR_WIDTH * crate::workspace_text::scale(cx))
}
const BODY_PADDING: f32 = 8.;
const PANE_HEADER: f32 = 0.;

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
    Relay,
    Ssh { target: String },
}

pub(crate) struct Tab {
    pub id: String,
    pub project: String,
    pub workspace: bool,
    pub title: String,
    pub cwd: String,
    pub screen: Screen,
    pub status: Status,
    pub exit: Option<Exit>,
    pub origin: Origin,
    pub selection: Option<Selection>,
    pub drag_anchor: Option<TermPoint>,
    pub bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub size: (u16, u16),
    session: Option<Session>,
}

pub(crate) struct SplitDrag {
    id: u64,
    axis: Axis,
    start: f32,
    initial_ratio: f32,
    extent: f32,
}

pub(crate) struct State {
    cell_width: f32,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub error: Option<String>,
    next: u64,
    next_split: u64,
    pub layout: Option<SplitNode>,
    pub split_drag: Option<SplitDrag>,
    pub focus: FocusHandle,
    pub search_open: bool,
    pub ssh_target: Option<Entity<InputState>>,
    pub ssh_path: Option<Entity<InputState>>,
    pub dock_width: f32,
    pub dock_height: f32,
    run_previous_tab: Option<String>,
}

impl State {
    pub fn new(cx: &mut Context<DunlinApp>) -> Self {
        cx.bind_keys([
            KeyBinding::new("tab", NoAction, Some("DunlinTerminal")),
            KeyBinding::new("shift-tab", NoAction, Some("DunlinTerminal")),
        ]);
        Self {
            cell_width: cell_width(cx),
            tabs: Vec::new(),
            active: 0,
            error: None,
            next: 1,
            next_split: 1,
            layout: None,
            split_drag: None,
            focus: cx.focus_handle(),
            search_open: false,
            ssh_target: None,
            ssh_path: None,
            dock_width: 900.,
            dock_height: 300.,
            run_previous_tab: None,
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

    fn next_split_id(&mut self) -> u64 {
        let id = self.next_split;
        self.next_split = self.next_split.wrapping_add(1);
        id
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// Show a tab in the active pane. Tabs that are already visible are focused
    /// in place; a hidden tab replaces only the active pane.
    fn show_tab(&mut self, id: &str) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let previous = self.tabs.get(self.active).map(|tab| tab.id.clone());
        self.active = index;
        match self.layout.as_mut() {
            None => self.layout = Some(SplitNode::leaf(id.to_string())),
            Some(layout) if layout.contains(id) => {}
            Some(layout) => {
                let previous = previous
                    .filter(|candidate| layout.contains(candidate))
                    .or_else(|| layout.leaves().into_iter().next())
                    .unwrap_or_else(|| id.to_string());
                if !layout.replace_tab(&previous, id) {
                    self.layout = Some(SplitNode::leaf(id.to_string()));
                }
            }
        }
    }

    fn split_active(&mut self, axis: Axis, previous: &str, new_tab: &str) {
        let id = self.next_split_id();
        match self.layout.as_mut() {
            None => self.layout = Some(SplitNode::leaf(new_tab.to_string())),
            Some(layout) => {
                if !layout.contains(previous) {
                    // `open_terminal_tab` temporarily replaced the active leaf
                    // with the new tab. Restore the previous session first so a
                    // split never duplicates the same leaf.
                    if !layout.replace_tab(new_tab, previous) {
                        let fallback = layout.leaves().first().cloned().unwrap_or_default();
                        layout.replace_tab(&fallback, previous);
                    }
                }
                let target = if layout.contains(previous) {
                    previous.to_string()
                } else {
                    layout
                        .leaves()
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| new_tab.to_string())
                };
                if !layout.split_leaf(&target, id, axis, new_tab) {
                    self.layout = Some(SplitNode::leaf(new_tab.to_string()));
                }
            }
        }
    }

    fn remove_from_layout(&mut self, tab: &str) {
        let Some(layout) = self.layout.as_mut() else {
            return;
        };
        layout.remove_tab(tab);
        if layout.leaves().is_empty() {
            self.layout = None;
        }
    }

    pub fn set_split_ratio(&mut self, id: u64, ratio: f32) {
        if let Some(layout) = &mut self.layout {
            layout.set_ratio(id, ratio);
        }
    }

    pub fn split_ratio(&self, id: u64) -> Option<f32> {
        self.layout.as_ref().and_then(|layout| layout.ratio(id))
    }

    pub fn split_extent(&self, id: u64, width: f32, height: f32) -> Option<(Axis, f32)> {
        self.layout
            .as_ref()
            .and_then(|layout| layout.extent(id, width, height))
    }

    pub fn pane_sizes(&self, width: f32, height: f32) -> Vec<(String, (f32, f32))> {
        self.layout
            .as_ref()
            .map(|layout| layout.sizes(width, height))
            .unwrap_or_default()
    }
}

fn shell_options(project: &Project, integration: bool) -> Result<SpawnOptions, String> {
    if !project.trusted {
        return Err("Trust this project before opening a local terminal.".into());
    }
    if !PathBuf::from(&project.path).is_dir() {
        return Err("Project folder is not available".into());
    }
    let mut options = SpawnOptions::shell(PathBuf::from(&project.path))?;
    options.shell_integration = integration;
    options.rows = dunlin_terminal::DEFAULT_ROWS;
    options.cols = dunlin_terminal::DEFAULT_COLS;
    Ok(options)
}

impl DunlinApp {
    pub(crate) fn open_server_agent_setup(
        &mut self,
        host: crate::remote::RemoteHost,
        token: String,
        agent: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match relay::connect_setup(&host, &token, agent) {
            Ok(session) => {
                let id = self.terminal_add(
                    session,
                    format!("{} · {agent} authorization", host.name),
                    host.url,
                    Origin::Relay,
                    dunlin_terminal::DEFAULT_ROWS,
                    dunlin_terminal::DEFAULT_COLS,
                    cx,
                );
                self.terminal.active = self.terminal.index_of(&id).unwrap_or(0);
                self.settings_open = false;
                self.bottom_panel = crate::app::BottomPanel::Terminal;
                self.terminal.focus.focus(window, cx);
                self.set_notice("Agent authorization runs on the selected server. No local credentials are copied.", cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
        cx.notify();
    }
    pub(crate) fn open_workspace_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_workspace_tab();
        let layout = self.terminal.layout.clone();
        let active = self.terminal.active;
        if let Err(error) = self.open_terminal_tab(cx) {
            self.file_notice(error, cx);
            return;
        }
        let id = self.terminal.active_tab().unwrap().id.clone();
        self.terminal.tab_mut(&id).unwrap().workspace = true;
        self.terminal.layout = layout;
        self.terminal.active = active;
        self.workbench
            .panes
            .opened(crate::workspace_panes::TabKey::Terminal(id.clone()));
        self.activate_workspace_tab(&crate::workspace_panes::TabKey::Terminal(id), window, cx);
        cx.notify();
    }

    pub(crate) fn activate_workspace_terminal(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .terminal
            .tabs
            .iter()
            .position(|tab| tab.id == id && tab.workspace && tab.project == self.project().id)
        else {
            return false;
        };
        self.remember_workspace_tab();
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.git_history.active = false;
        self.spec.active = false;
        self.workbench.terminal = Some(id.into());
        self.terminal.active = index;
        self.terminal.focus.focus(window, cx);
        self.remember_workspace_tab();
        cx.notify();
        true
    }

    pub(crate) fn open_terminal_tab(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.group_cleanup.busy {
            return Err("Wait for generated-folder cleanup to finish".into());
        }
        let project = self.project().clone();
        if project.is_remote() {
            let workspace = self.remote_workspace_for_project(&project)?;
            let rows = dunlin_terminal::DEFAULT_ROWS;
            let cols = dunlin_terminal::DEFAULT_COLS;
            let session = relay::connect(&workspace, rows, cols)?;
            let title = format!("Relay · {}", project.name);
            let id = self.terminal_add(
                session,
                title,
                project.path.clone(),
                Origin::Relay,
                rows,
                cols,
                cx,
            );
            self.terminal.active = self.terminal.index_of(&id).unwrap_or(0);
        } else {
            let options = shell_options(&project, self.shell_integration)?;
            let title = format!("Shell · {}", project.name);
            let tab =
                self.terminal_spawn(options, title, project.path.clone(), Origin::Shell, cx)?;
            self.terminal.active = self.terminal.index_of(&tab).unwrap_or(0);
        }
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
            return Err("Run is local-only; open a relay terminal for a remote project".into());
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
        let installing = candidate.kind == "dependency";
        let previous_active = self.terminal.active;
        let previous_layout = self.terminal.layout.clone();
        self.terminal_spawn(
            options,
            title,
            cwd.to_string_lossy().to_string(),
            Origin::Run { record: record_id },
            cx,
        )?;
        if installing {
            self.terminal.active = previous_active;
            self.terminal.layout = previous_layout;
            cx.notify();
            return Ok(());
        }
        self.terminal.active = self.terminal.tabs.len().saturating_sub(1);
        self.terminal.error = None;
        self.bottom_panel = crate::app::BottomPanel::Terminal;
        self.persist_settings(cx);
        self.terminal.focus.focus(window, cx);
        cx.notify();
        Ok(())
    }

    pub(crate) fn open_ssh_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.group_cleanup.busy {
            return Err("Wait for generated-folder cleanup to finish".into());
        }
        self.ensure_ssh_inputs(window, cx);
        let target = self
            .terminal
            .ssh_target
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if target.is_empty() {
            return Err("Enter an SSH host or a ~/.ssh/config alias".into());
        }
        let path = self
            .terminal
            .ssh_path
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let local_cwd = PathBuf::from(&self.project().path);
        let local_cwd = if local_cwd.is_dir() {
            local_cwd
        } else {
            std::env::temp_dir()
        };
        let identity = self
            .remote_connection
            .identity
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .filter(|s| !s.is_empty());
        let port = self
            .remote_connection
            .port
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<u16>()
                    .map_err(|_| "SSH port must be between 1 and 65535".to_string())
            })
            .transpose()?;
        let options = SpawnOptions::ssh_with_identity(
            &target,
            local_cwd,
            (!path.is_empty()).then_some(path.as_str()),
            identity.as_deref().map(std::path::Path::new),
            port,
        )?;
        let title = format!("SSH · {target}");
        let tab = self.terminal_spawn(
            options,
            title,
            target.clone(),
            Origin::Ssh {
                target: target.clone(),
            },
            cx,
        )?;
        self.terminal.active = self.terminal.index_of(&tab).unwrap_or(0);
        self.terminal_ssh_target = target;
        self.terminal_ssh_path = path;
        self.terminal.error = None;
        self.bottom_panel = crate::app::BottomPanel::Terminal;
        self.persist_settings(cx);
        self.terminal.focus.focus(window, cx);
        cx.notify();
        Ok(())
    }

    pub(crate) fn terminal_split(
        &mut self,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let previous = self.terminal.active_tab().map(|tab| tab.id.clone());
        self.open_terminal_tab(cx)?;
        let new_tab = self
            .terminal
            .active_tab()
            .map(|tab| tab.id.clone())
            .ok_or_else(|| "The split terminal did not start".to_string())?;
        if let Some(previous) = previous {
            self.terminal.split_active(axis, &previous, &new_tab);
        }
        self.terminal.active = self.terminal.index_of(&new_tab).unwrap_or(0);
        self.terminal.focus.focus(window, cx);
        cx.notify();
        Ok(())
    }

    pub(crate) fn ensure_ssh_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.ssh_target.is_none() {
            let target = self.terminal_ssh_target.clone();
            self.terminal.ssh_target = Some(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("user@host or SSH config alias")
                    .default_value(target)
            }));
        }
        if self.terminal.ssh_path.is_none() {
            let path = self.terminal_ssh_path.clone();
            self.terminal.ssh_path = Some(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Optional remote path")
                    .default_value(path)
            }));
        }
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
                                Origin::Shell | Origin::Relay | Origin::Ssh { .. } => {
                                    (None, Some(tab.title.clone()))
                                }
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

    #[allow(clippy::too_many_arguments)]
    fn terminal_add(
        &mut self,
        session: Session,
        title: String,
        cwd: String,
        origin: Origin,
        rows: u16,
        cols: u16,
        cx: &mut Context<Self>,
    ) -> String {
        let output = session.output();
        let exit = session.exit();
        let id = self.terminal.next_id();
        self.terminal.tabs.push(Tab {
            id: id.clone(),
            project: self.project().id.clone(),
            workspace: false,
            title,
            cwd,
            screen: Screen::new(rows, cols, dunlin_terminal::DEFAULT_SCROLLBACK),
            status: Status::Running,
            exit: None,
            origin,
            selection: None,
            drag_anchor: None,
            bounds: Rc::new(Cell::new(None)),
            size: (rows, cols),
            session: Some(session),
        });
        self.terminal.show_tab(&id);
        self.terminal_stream(id.clone(), output, exit, cx);
        id
    }

    fn terminal_spawn(
        &mut self,
        options: SpawnOptions,
        title: String,
        cwd: String,
        origin: Origin,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        if self.group_cleanup.busy {
            return Err("Wait for generated-folder cleanup to finish".into());
        }
        let rows = options.rows;
        let cols = options.cols;
        let session = Session::spawn(options)?;
        Ok(self.terminal_add(session, title, cwd, origin, rows, cols, cx))
    }

    pub(crate) fn kill_terminal_process(&self, id: &str) {
        if let Some(session) = self
            .terminal
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.session.as_ref())
        {
            session.kill();
        }
    }

    pub(crate) fn close_terminal_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.terminal.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let active = self.terminal.active_tab().map(|tab| tab.id.clone());
        let tab = self.terminal.tabs.remove(index);
        if tab.workspace {
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::Terminal(id.into()));
            if self.workbench.terminal.as_deref() == Some(id) {
                self.workbench.terminal = None;
            }
        }
        if let Some(index) = active.as_deref().and_then(|id| self.terminal.index_of(id)) {
            self.terminal.active = index;
        }
        if active.as_deref() == Some(id)
            && let Some(previous) = self.terminal.run_previous_tab.take()
            && let Some(index) = self.terminal.index_of(&previous)
        {
            self.terminal.active = index;
        }
        if let Some(session) = &tab.session {
            session.kill();
        }
        if let Origin::Run { record } = &tab.origin
            && matches!(tab.status, Status::Running)
        {
            self.finish_run(record, None, cx);
        }
        self.terminal.remove_from_layout(id);
        if self.terminal.tabs.is_empty() {
            self.terminal.layout = None;
        } else if self.terminal.layout.is_none() {
            let active = self
                .terminal
                .tabs
                .iter()
                .find(|tab| !tab.workspace)
                .map(|tab| tab.id.clone());
            if let Some(active) = active {
                self.terminal.show_tab(&active);
            }
        } else {
            let active_id = self
                .terminal
                .tabs
                .get(self.terminal.active)
                .map(|tab| tab.id.clone());
            let visible = self
                .terminal
                .layout
                .as_ref()
                .map(|layout| layout.leaves())
                .unwrap_or_default();
            if !active_id
                .as_deref()
                .is_some_and(|id| visible.iter().any(|candidate| candidate == id))
                && let Some(first) = visible.first()
                && let Some(index) = self.terminal.index_of(first)
            {
                self.terminal.active = index;
            }
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

    pub(crate) fn terminal_select_all(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        let rows = tab.screen.rows();
        let cols = tab.screen.cols();
        let (scroll_offset, scrollback_len) = tab.screen.scroll_position();
        let last_line = tab.screen.visible_line(rows.saturating_sub(1)) + scroll_offset as i64;
        let first_line = last_line
            .saturating_sub(rows.saturating_sub(1) as i64)
            .saturating_sub(scrollback_len as i64);
        tab.selection = Some(Selection::new(
            TermPoint::new(first_line, 0),
            TermPoint::new(last_line, cols.saturating_sub(1)),
        ));
        tab.drag_anchor = None;
        cx.notify();
    }

    pub(crate) fn terminal_copy(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        let selection = tab.selection.filter(|selection| !selection.is_empty());
        let text = if let Some(selection) = selection {
            tab.screen.text_between(selection.start(), selection.end())
        } else {
            tab.screen.plain_text()
        };
        let text = truncate_terminal_text(text);
        if text.is_empty() {
            self.set_notice("Nothing to copy from the terminal", cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_notice(
            if selection.is_some() {
                "Copied terminal selection"
            } else {
                "Copied terminal buffer"
            },
            cx,
        );
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

    pub(crate) fn terminal_scroll_tab(
        &mut self,
        id: &str,
        up: bool,
        lines: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.terminal.index_of(id) {
            self.terminal.active = index;
        }
        if let Some(tab) = self.terminal.tab_mut(id) {
            if up {
                tab.screen.scroll_up(lines);
            } else {
                tab.screen.scroll_down(lines);
            }
        }
        cx.notify();
    }

    pub(crate) fn terminal_resize_to(&mut self, width: f32, height: f32, cx: &App) {
        self.terminal.dock_width = width.max(1.);
        self.terminal.dock_height = height.max(1.);
        let sizes = self.terminal.pane_sizes(width, height);
        if sizes.is_empty() {
            let (rows, cols) = terminal_size(
                width,
                height,
                self.terminal.cell_width,
                crate::workspace_text::scale(cx),
            );
            if let Some(tab) = self.terminal.active_tab_mut()
                && tab.size != (rows, cols)
            {
                tab.size = (rows, cols);
                tab.screen.resize(rows, cols);
                if let Some(session) = &tab.session {
                    let _ = session.resize(rows, cols);
                }
            }
            return;
        }
        let mut updates = Vec::with_capacity(sizes.len());
        for (id, (width, height)) in sizes {
            let (rows, cols) = terminal_size(
                width,
                height,
                self.terminal.cell_width,
                crate::workspace_text::scale(cx),
            );
            updates.push((id, rows, cols));
        }
        for (id, rows, cols) in updates {
            let Some(tab) = self.terminal.tab_mut(&id) else {
                continue;
            };
            if tab.size == (rows, cols) {
                continue;
            }
            tab.size = (rows, cols);
            tab.screen.resize(rows, cols);
            if let Some(session) = &tab.session {
                let _ = session.resize(rows, cols);
            }
        }
    }

    pub(crate) fn terminal_pointer_down(
        &mut self,
        tab_id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.terminal.index_of(tab_id) else {
            return;
        };
        self.terminal.active = index;
        self.terminal.focus.focus(window, cx);
        let Some(point) = terminal_point(
            &self.terminal.tabs[index],
            event.position,
            self.terminal.cell_width,
            ROW_HEIGHT * crate::workspace_text::scale(cx),
        ) else {
            cx.notify();
            return;
        };
        let tab = &mut self.terminal.tabs[index];
        if event.click_count >= 3 {
            tab.selection = Some(Selection::line(point.line, tab.screen.cols()));
            tab.drag_anchor = None;
        } else if event.click_count == 2 {
            tab.selection = tab.screen.word_selection(point.line, point.col);
            tab.drag_anchor = None;
        } else if event.modifiers.shift {
            let anchor = tab
                .selection
                .map(|selection| selection.anchor)
                .unwrap_or(point);
            tab.selection = Some(Selection::new(anchor, point));
            tab.drag_anchor = None;
        } else {
            tab.selection = None;
            tab.drag_anchor = Some(point);
        }
        cx.notify();
    }

    pub(crate) fn terminal_pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let cell_width = self.terminal.cell_width;
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(tab) = self.terminal.active_tab_mut() else {
            return;
        };
        let Some(anchor) = tab.drag_anchor else {
            return;
        };
        let Some(bounds) = tab.bounds.get() else {
            return;
        };
        let y = f32::from(event.position.y);
        let top = f32::from(bounds.top()) + BODY_PADDING + PANE_HEADER;
        let row_height = ROW_HEIGHT * crate::workspace_text::scale(cx);
        let bottom = top + f32::from(tab.screen.rows()) * row_height;
        if y < top {
            tab.screen.scroll_up(1);
        } else if y > bottom {
            tab.screen.scroll_down(1);
        }
        let Some(head) = terminal_point(tab, event.position, cell_width, row_height) else {
            return;
        };
        if head != anchor || tab.selection.is_some() {
            tab.selection = Some(Selection::new(anchor, head));
        }
        cx.notify();
    }

    pub(crate) fn terminal_pointer_up(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.terminal.active_tab_mut() {
            tab.drag_anchor = None;
        }
        cx.notify();
    }

    pub(crate) fn terminal_split_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = &self.terminal.split_drag else {
            return false;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.terminal.split_drag = None;
            return true;
        }
        let current = if drag.axis == Axis::Horizontal {
            f32::from(event.position.x)
        } else {
            f32::from(event.position.y)
        };
        let ratio = drag.initial_ratio + (current - drag.start) / drag.extent.max(1.);
        let id = drag.id;
        self.terminal.set_split_ratio(id, ratio);
        cx.notify();
        true
    }

    pub(crate) fn terminal_finish_split_drag(&mut self, cx: &mut Context<Self>) {
        if self.terminal.split_drag.take().is_some() {
            cx.notify();
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
        if self.showing_dependency_terminal()
            && platform
            && matches!(keystroke.key.as_str(), "d" | "w" | "f")
        {
            return true;
        }
        if platform {
            if keystroke.key == "d" && self.terminal.active_tab().is_some_and(|tab| tab.workspace) {
                self.split_workspace(window, cx);
                if self.focus_workspace_pane(1 - self.workbench.panes.focused, window, cx) {
                    self.open_workspace_terminal(window, cx);
                }
                return true;
            }
            match keystroke.key.as_str() {
                "a" => {
                    self.terminal_select_all(cx);
                    return true;
                }
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
                "d" if keystroke.modifiers.shift => {
                    let _ = self.terminal_split(Axis::Horizontal, window, cx);
                    return true;
                }
                "d" => {
                    let _ = self.terminal_split(Axis::Vertical, window, cx);
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
            "tab" if keystroke.modifiers.shift => Some(b"\x1b[Z".to_vec()),
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

pub(crate) fn terminal_size(width: f32, height: f32, cell_width: f32, scale: f32) -> (u16, u16) {
    let cols = ((width - BODY_PADDING * 2. - 2.) / cell_width).clamp(20., 512.) as u16;
    let rows = ((height - 78.) / (15.5 * scale)).clamp(3., 80.) as u16;
    (rows, cols)
}

fn terminal_point(
    tab: &Tab,
    position: Point<Pixels>,
    cell_width: f32,
    row_height: f32,
) -> Option<TermPoint> {
    let bounds = tab.bounds.get()?;
    let x = f32::from(position.x) - f32::from(bounds.left()) - BODY_PADDING;
    let y = f32::from(position.y) - f32::from(bounds.top()) - BODY_PADDING - PANE_HEADER;
    let row = (y / row_height).floor().max(0.) as u16;
    let col = (x / cell_width).floor().max(0.) as u16;
    let row = row.min(tab.screen.rows().saturating_sub(1));
    let col = col.min(tab.screen.cols().saturating_sub(1));
    Some(TermPoint::new(tab.screen.visible_line(row), col))
}

fn truncate_terminal_text(mut text: String) -> String {
    const LIMIT: usize = 1024 * 1024;
    if text.len() > LIMIT {
        let mut end = LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
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
    col: u16,
    cells: u16,
    wide: bool,
    text: String,
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    link: Option<String>,
    selected: bool,
}

#[allow(clippy::too_many_arguments)]
fn row_runs(
    terminal: &Screen,
    row: u16,
    cursor: Option<(u16, u16)>,
    hide_cursor: bool,
    selection: Option<Selection>,
    p: &Palette,
) -> Vec<Run> {
    let screen = terminal.screen();
    let cols = terminal.cols();
    let line = terminal.visible_line(row);
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
        let selected = selection.is_some_and(|selection| selection.contains(line, col));
        let link = terminal.link_at(row, col).map(str::to_string);
        let contents = cell.contents().to_string();
        if contents.is_empty()
            && !selected
            && link.is_none()
            && (hide_cursor || cursor != Some((row, col)))
            && bg == p.bg
        {
            continue;
        }
        let contents = if contents.is_empty() {
            " ".into()
        } else {
            contents
        };
        let background = (bg != p.bg).then_some(bg);
        if let Some(last) = runs.last_mut()
            && last.col + last.cells == col
            && last.text.is_ascii()
            && contents.is_ascii()
            && !last.wide
            && !cell.is_wide()
            && last.fg == fg
            && last.bg == background
            && last.bold == cell.bold()
            && last.link == link
            && last.selected == selected
        {
            last.text.push_str(&contents);
            last.cells += 1;
        } else {
            runs.push(Run {
                col,
                cells: if cell.is_wide() { 2 } else { 1 },
                wide: cell.is_wide(),
                text: contents,
                fg,
                bg: background,
                bold: cell.bold(),
                link,
                selected,
            });
        }
    }
    runs
}

#[allow(clippy::too_many_arguments)]
fn row_element(
    tab: &Tab,
    row: u16,
    cursor: Option<(u16, u16)>,
    hide_cursor: bool,
    query: &str,
    selection: Option<Selection>,
    cell_width: f32,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let runs = row_runs(&tab.screen, row, cursor, hide_cursor, selection, p);
    let selection_color = p.blue.opacity(0.28);
    let mut line_element = div()
        .relative()
        .h(crate::workspace_text::size(ROW_HEIGHT, cx))
        .w_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    let mut link_index = 0usize;
    for run in runs {
        let mut run_element = div()
            .absolute()
            .left(px(f32::from(run.col) * cell_width))
            .top_0()
            .w(px(f32::from(run.cells) * cell_width))
            .h(crate::workspace_text::size(ROW_HEIGHT, cx))
            .flex()
            .items_center()
            .when_some(run.bg, |element, bg| element.bg(bg))
            .when(run.selected, |element| element.bg(selection_color));
        for (part, highlighted) in highlight_parts(&run.text, query) {
            let pieces = if run.link.is_some() {
                vec![(part, None)]
            } else {
                link_parts(&part)
            };
            for (piece, plain_link) in pieces {
                if piece.is_empty() {
                    continue;
                }
                let handled = run.link.clone().or(plain_link);
                let target = handled.as_deref().and_then(safe_link);
                let is_link = target.is_some();
                let base = div()
                    .text_size(crate::workspace_text::size(FONT_SIZE, cx))
                    .line_height(crate::workspace_text::size(ROW_HEIGHT, cx))
                    .font(terminal_font(cx))
                    .flex_shrink_0()
                    .text_color(if is_link { p.blue } else { run.fg })
                    .when_some(run.bg, |this, bg| this.bg(bg))
                    .when(run.selected, |this| this.bg(selection_color))
                    .when(highlighted, |this| this.bg(p.amber_soft))
                    .when(run.bold, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(piece);
                let span = if let Some(url) = target {
                    link_index += 1;
                    base.id(format!("terminal-link:{}:{row}:{link_index}", tab.id))
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                        .into_any_element()
                } else {
                    base.into_any_element()
                };
                run_element = run_element.child(span);
            }
        }
        line_element = line_element.child(run_element);
    }
    line_element.into_any_element()
}

fn render_layout(
    this: &mut DunlinApp,
    node: &SplitNode,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    match node {
        SplitNode::Leaf { tab } => render_pane(this, tab, window, cx),
        SplitNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            let p = palette(cx);
            let split_id = *id;
            let first_element = render_layout(this, first, window, cx);
            let second_element = render_layout(this, second, window, cx);
            let divider = div()
                .id(format!("terminal-split:{id}"))
                .aria_label("Drag to resize terminal panes")
                .flex_shrink_0()
                .occlude()
                .cursor(if *axis == Axis::Horizontal {
                    CursorStyle::ResizeLeftRight
                } else {
                    CursorStyle::ResizeUpDown
                })
                .hover(move |style| style.bg(p.border_strong))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let width = this.terminal.dock_width;
                        let height = this.terminal.dock_height;
                        let Some((axis, extent)) =
                            this.terminal.split_extent(split_id, width, height)
                        else {
                            return;
                        };
                        if let Some(ratio) = this.terminal.split_ratio(split_id) {
                            this.terminal.split_drag = Some(SplitDrag {
                                id: split_id,
                                axis,
                                start: if axis == Axis::Horizontal {
                                    f32::from(event.position.x)
                                } else {
                                    f32::from(event.position.y)
                                },
                                initial_ratio: ratio,
                                extent,
                            });
                        }
                        cx.notify();
                    }),
                );
            let divider = if *axis == Axis::Horizontal {
                divider
                    .w(px(6.))
                    .h_full()
                    .bg(p.border.opacity(0.5))
                    .into_any_element()
            } else {
                divider
                    .h(px(6.))
                    .w_full()
                    .bg(p.border.opacity(0.5))
                    .into_any_element()
            };
            let ratio = ratio.clamp(0.15, 0.85);
            if *axis == Axis::Horizontal {
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .size_full()
                    .child(
                        div()
                            .flex_grow(ratio)
                            .flex_shrink_1()
                            .flex_basis(px(0.))
                            .min_w(px(0.))
                            .h_full()
                            .child(first_element),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex_grow(1. - ratio)
                            .flex_shrink_1()
                            .flex_basis(px(0.))
                            .min_w(px(0.))
                            .h_full()
                            .child(second_element),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .size_full()
                    .child(
                        div()
                            .flex_grow(ratio)
                            .flex_shrink_1()
                            .flex_basis(px(0.))
                            .min_h(px(0.))
                            .w_full()
                            .child(first_element),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex_grow(1. - ratio)
                            .flex_shrink_1()
                            .flex_basis(px(0.))
                            .min_h(px(0.))
                            .w_full()
                            .child(second_element),
                    )
                    .into_any_element()
            }
        }
    }
}

fn render_pane(
    this: &mut DunlinApp,
    tab_id: &str,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let Some(index) = this.terminal.index_of(tab_id) else {
        return div().into_any_element();
    };
    let active = index == this.terminal.active;
    let p = palette(cx);
    let query = if active {
        this.terminal_query.clone()
    } else {
        String::new()
    };
    let tab = &this.terminal.tabs[index];
    let screen = tab.screen.screen();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let hide_cursor = screen.hide_cursor();
    let rows = tab.screen.rows();
    let selection = tab.selection.filter(|selection| !selection.is_empty());
    let bounds = tab.bounds.clone();
    let id_for_down = tab.id.clone();
    let id_for_wheel = tab.id.clone();
    let mut lines = div().flex().flex_col().p(px(BODY_PADDING));
    for row in 0..rows {
        lines = lines.child(row_element(
            tab,
            row,
            Some((cursor_row, cursor_col)),
            hide_cursor,
            &query,
            selection,
            this.terminal.cell_width,
            &p,
            cx,
        ));
    }
    let body = div()
        .id(format!("terminal-body:{}", tab.id))
        .key_context("DunlinTerminal")
        .track_focus(&this.terminal.focus)
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .w_full()
        .h_full()
        .overflow_hidden()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.terminal_pointer_down(&id_for_down, event, window, cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                this.terminal_pointer_up(cx);
            }),
        )
        .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, cx| {
            let delta = event.delta.pixel_delta(px(16.)).y;
            let lines = (delta / px(1.)).abs().max(1.) as usize / 12;
            if delta > px(0.) {
                this.terminal_scroll_tab(&id_for_wheel, true, lines.max(1), cx);
            } else if delta < px(0.) {
                this.terminal_scroll_tab(&id_for_wheel, false, lines.max(1), cx);
            }
        }))
        .child(lines)
        .child(
            canvas(
                move |pane, _, _| {
                    bounds.set(Some(pane));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .size_full(),
        );
    body.into_any_element()
}

/// Reuse the PTY viewport inside Run without replacing the user's terminal split layout.
pub(crate) fn render_run_output(
    this: &mut DunlinApp,
    id: &str,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    if let Some(index) = this.terminal.index_of(id)
        && this.terminal.active != index
    {
        this.terminal.run_previous_tab = this.terminal.active_tab().map(|tab| tab.id.clone());
        this.terminal.active = index;
    }
    render_tab(this, id, window, cx)
}

pub(crate) fn render_tab(
    this: &mut DunlinApp,
    id: &str,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    this.terminal.cell_width = cell_width(cx);
    let cell_width = this.terminal.cell_width;
    let row_height = ROW_HEIGHT * crate::workspace_text::scale(cx);
    if let Some(tab) = this.terminal.tab_mut(id) {
        if let Some(bounds) = tab.bounds.get() {
            let cols = ((f32::from(bounds.size.width) - BODY_PADDING * 2.) / cell_width)
                .clamp(20., 512.) as u16;
            let rows = ((f32::from(bounds.size.height) - BODY_PADDING * 2.) / row_height)
                .clamp(3., 80.) as u16;
            if tab.size != (rows, cols) {
                tab.size = (rows, cols);
                tab.screen.resize(rows, cols);
                if let Some(session) = &tab.session {
                    let _ = session.resize(rows, cols);
                }
            }
        }
        let pane = render_pane(this, id, window, cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .when(
                this.terminal.search_open
                    && this.terminal.active_tab().is_some_and(|tab| tab.id == id),
                |root| {
                    root.child(search_controls(this, cx).mx(px(dunlin_ui::scale::WORKSPACE_RADIUS)))
                },
            )
            .child(pane)
            .into_any_element()
    } else {
        div()
            .flex_1()
            .p(px(12.))
            .text_color(palette(cx).muted_fg)
            .child("Terminal closed.")
            .into_any_element()
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    if let Some(previous) = this.terminal.run_previous_tab.take()
        && let Some(index) = this.terminal.index_of(&previous)
    {
        this.terminal.active = index;
    }
    this.terminal.cell_width = cell_width(cx);
    let viewport_width = f32::from(window.viewport_size().width);
    let viewport_height = f32::from(window.viewport_size().height);
    match this.terminal_dock {
        crate::app::TerminalDock::Bottom => {
            this.terminal_resize_to(viewport_width - 330., this.bottom_panel_height, cx)
        }
        crate::app::TerminalDock::Left | crate::app::TerminalDock::Right => {
            this.terminal_resize_to(this.bottom_panel_width, viewport_height - 140., cx)
        }
    }
    let p = palette(cx);
    let mut toolbar = div()
        .h(px(34.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(p.border)
        .bg(p.card);
    for (index, tab) in this
        .terminal
        .tabs
        .iter()
        .enumerate()
        .filter(|(_, tab)| !tab.workspace)
    {
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
                .aria_label(format!("{} · {}", tab.title, tab.cwd))
                .h(px(25.))
                .max_w(px(230.))
                .px(px(7.))
                .rounded(px(dunlin_ui::radii::ICON))
                .bg(if active {
                    p.accent
                } else {
                    transparent_black()
                })
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.terminal.show_tab(&id);
                    cx.notify();
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
                    dunlin_ui::button(format!("terminal-close:{close}"))
                        .ghost()
                        .small()
                        .label("×")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_terminal_tab(&close, cx);
                        })),
                ),
        );
    }
    toolbar = toolbar
        .child(
            dunlin_ui::button("terminal-new")
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
        .child(
            dunlin_ui::button("terminal-split-right")
                .ghost()
                .small()
                .label("◫")
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Err(error) = this.terminal_split(Axis::Horizontal, window, cx) {
                        this.terminal.error = Some(error.clone());
                        this.set_notice(error, cx);
                    }
                })),
        )
        .child(
            dunlin_ui::button("terminal-split-down")
                .ghost()
                .small()
                .label("⬓")
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Err(error) = this.terminal_split(Axis::Vertical, window, cx) {
                        this.terminal.error = Some(error.clone());
                        this.set_notice(error, cx);
                    }
                })),
        );
    if this.terminal.search_open && this.terminal.active_tab().is_some_and(|tab| !tab.workspace) {
        toolbar = toolbar.child(search_controls(this, cx));
    }
    toolbar = toolbar.child(div().flex_1());

    let mut root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .bg(p.card)
        .child(toolbar);

    let body = match this.terminal.layout.clone() {
        Some(node) => render_layout(this, &node, window, cx),
        None => div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(p.muted_fg)
            .child(if this.project().is_remote() {
                "No relay terminal tab. Open a relay shell to begin."
            } else {
                "No terminal tab. Open a local shell to begin."
            })
            .child(
                dunlin_ui::button("terminal-empty-new")
                    .small()
                    .label("New terminal")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.open_terminal_tab(cx) {
                            this.terminal.error = Some(error.clone());
                            this.set_notice(error, cx);
                        }
                    })),
            )
            .into_any_element(),
    };
    root = root.child(div().flex_1().min_h(px(0.)).flex().child(body));
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

fn search_controls(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Div {
    let matches = this
        .terminal
        .active_tab()
        .map(|tab| count_matches(&tab.screen.screen().contents(), &this.terminal_query))
        .unwrap_or(0);
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_shrink_0()
        .child(div().w(px(190.)).child(search_input(
            &this.terminal_search,
            "Search terminal output",
        )))
        .child(
            div()
                .text_size(px(9.5))
                .text_color(palette(cx).muted_fg)
                .child(format!("{matches} visible")),
        )
        .child(
            dunlin_ui::button("terminal-search-close")
                .ghost()
                .small()
                .label("×")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.terminal.search_open = false;
                    this.terminal_query.clear();
                    this.terminal_search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    this.terminal.focus.focus(window, cx);
                    cx.notify();
                })),
        )
}

#[cfg(test)]
mod tests {
    // Keep the test attribute built-in; a `super::*` glob would also import the
    // UI kit's `test` macro and recurse while expanding `#[test]`.
    use super::terminal_size;
    use crate::terminal::layout::{Axis, SplitNode};

    #[test]
    fn terminal_size_respects_dock_limits() {
        let (rows, cols) = terminal_size(900., 300., 7.1, 1.);
        assert!((3..=80).contains(&rows));
        assert!((20..=512).contains(&cols));
        let (rows, cols) = terminal_size(10., 10., 7.1, 1.);
        assert_eq!(rows, 3);
        assert_eq!(cols, 20);
    }

    #[test]
    fn row_geometry_maps_the_last_visible_cell() {
        let mut node = SplitNode::leaf("term-1".into());
        node.split_leaf("term-1", 1, Axis::Horizontal, "term-2");
        let sizes = node.sizes(800., 300.);
        assert_eq!(sizes.len(), 2);
        for (_, (width, height)) in sizes {
            let (rows, cols) = terminal_size(width, height, 7.1, 1.);
            assert!(rows >= 3);
            assert!(cols >= 20);
        }
    }

    #[test]
    fn runs_preserve_empty_columns_right_prompts_and_wide_cell_positions() {
        let mut screen = dunlin_terminal::Screen::new(3, 100, 10);
        screen.process("left\x1b[70G\u{e718} 24.18.0".as_bytes());
        let runs = super::row_runs(&screen, 0, None, true, None, &dunlin_ui::theme::dark());
        assert_eq!(runs[0].col, 0);
        assert_eq!(runs[0].text, "left");
        assert_eq!(runs[1].col, 69);
        assert!(runs[1].text.starts_with('\u{e718}'));
        screen.process("\r\n界A\x1b[20GZ".as_bytes());
        let runs = super::row_runs(&screen, 1, None, true, None, &dunlin_ui::theme::light());
        assert_eq!((runs[0].col, runs[0].cells), (0, 2));
        assert_eq!((runs[1].col, runs[1].text.as_str()), (2, "A"));
        assert_eq!((runs[2].col, runs[2].text.as_str()), (19, "Z"));
        screen.process(b"\r\n\x1b[41m\x1b[K");
        let runs = super::row_runs(&screen, 2, None, true, None, &dunlin_ui::theme::dark());
        assert_eq!((runs[0].col, runs[0].cells), (0, 100));
        assert!(
            runs[0].bg.is_some(),
            "erased colored columns keep their background"
        );
        assert_eq!(terminal_size(1000., 400., 10., 1.).1, 98);
        let normal = terminal_size(1000., 400., 10., 1.);
        let zoomed = terminal_size(1000., 400., 20., 2.);
        assert!(
            zoomed.0 < normal.0 && zoomed.1 < normal.1,
            "PTY rows and columns must follow the measured font zoom"
        );
    }
}
