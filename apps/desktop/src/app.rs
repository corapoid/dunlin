use std::fs;
use std::path::{Path, PathBuf};

use kestrel_domain::*;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::input::{InputEvent, InputState, TextareaState};
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;

use crate::tree::FileTree;
use crate::{demo, git, harness, search, views};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Harnesses,
    Permissions,
    Editor,
    Usage,
    Advanced,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 6] = [
        SettingsTab::General,
        SettingsTab::Harnesses,
        SettingsTab::Permissions,
        SettingsTab::Editor,
        SettingsTab::Usage,
        SettingsTab::Advanced,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Harnesses => "Harnesses",
            SettingsTab::Permissions => "Permissions",
            SettingsTab::Editor => "Editor",
            SettingsTab::Usage => "Usage",
            SettingsTab::Advanced => "Advanced",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            SettingsTab::General => IconName::Settings2,
            SettingsTab::Harnesses => IconName::Bot,
            SettingsTab::Permissions => IconName::ShieldCheck,
            SettingsTab::Editor => IconName::Code,
            SettingsTab::Usage => IconName::ChartNoAxesColumn,
            SettingsTab::Advanced => IconName::SlidersHorizontal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionMode {
    Ask,
    ReadOnly,
    AutoApprove,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask every time",
            PermissionMode::ReadOnly => "Read-only",
            PermissionMode::AutoApprove => "Auto-approve",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceFilter {
    All,
    Agent,
    Llm,
    Tool,
    Permission,
}

impl TraceFilter {
    pub const ALL: [TraceFilter; 5] = [
        TraceFilter::All,
        TraceFilter::Agent,
        TraceFilter::Llm,
        TraceFilter::Tool,
        TraceFilter::Permission,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TraceFilter::All => "All",
            TraceFilter::Agent => "Agents",
            TraceFilter::Llm => "LLM",
            TraceFilter::Tool => "Tools",
            TraceFilter::Permission => "Permissions",
        }
    }

    pub fn matches(self, kind: SpanKind) -> bool {
        match self {
            TraceFilter::All => true,
            TraceFilter::Agent => matches!(kind, SpanKind::Agent | SpanKind::Turn),
            TraceFilter::Llm => kind == SpanKind::Llm,
            TraceFilter::Tool => kind == SpanKind::Tool,
            TraceFilter::Permission => matches!(kind, SpanKind::Permission | SpanKind::Waiting),
        }
    }
}

pub(crate) struct Inspector {
    pub path: String,
    pub name: String,
    pub directory: String,
    pub lines: Vec<String>,
    pub snippet: Option<(u32, u32)>,
    pub truncated: bool,
}

/// A single executable entry in the command palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    View(MainView),
    Section(SidebarSection),
    Settings(SettingsTab),
    OpenFolder,
    ToggleTheme,
    NewSession,
    RestartHarness,
    RefreshGit,
    SwitchProject(usize),
    CloseInspector,
}

/// A command plus the presentation metadata used to filter and paint it.
#[derive(Clone)]
pub(crate) struct CommandItem {
    pub command: Command,
    pub label: String,
    pub hint: Option<String>,
    pub icon: IconName,
    pub keywords: String,
}

/// Every whitespace-separated term must match the label, keywords or hint.
pub(crate) fn command_matches(item: &CommandItem, terms: &[&str]) -> bool {
    let haystack = format!(
        "{} {} {}",
        item.label.to_lowercase(),
        item.keywords.to_lowercase(),
        item.hint.as_deref().unwrap_or_default().to_lowercase()
    );
    terms
        .iter()
        .all(|term| haystack.contains(&term.to_lowercase()))
}

pub(crate) struct KestrelApp {
    pub view: MainView,
    pub section: SidebarSection,
    pub projects: Vec<Project>,
    pub selected_project: usize,
    pub sessions: Vec<SessionSummary>,
    pub all_sessions: Vec<SessionSummary>,
    pub fallback_session: SessionSummary,
    pub selected_session: usize,
    pub usages: Vec<ProviderUsage>,
    pub nodes: Vec<AgentNode>,
    pub spans: Vec<TraceSpan>,
    pub composer: Entity<TextareaState>,
    _composer_subscription: Subscription,
    pub search_input: Entity<InputState>,
    _search_subscription: Subscription,

    pub focus_handle: FocusHandle,
    pub command_open: bool,
    pub command_query: Entity<InputState>,
    pub command_text: String,
    pub command_index: usize,
    _command_subscription: Subscription,

    pub settings_open: bool,
    pub settings_tab: SettingsTab,
    pub default_harness: String,
    pub custom_harness_command: String,
    pub custom_command_input: Entity<InputState>,
    _custom_command_subscription: Subscription,
    pub permission_mode: PermissionMode,
    pub project_menu_open: bool,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub word_wrap: bool,
    pub allow_network: bool,

    pub inspector: Option<Inspector>,
    pub selected_span: Option<usize>,
    pub selected_node: Option<usize>,
    pub reasoning_open: bool,
    pub expanded_tool: Option<usize>,
    pub follow_running: bool,
    pub graph_zoom: f32,
    pub trace_zoom: f32,
    pub trace_filter: TraceFilter,

    pub file_tree: FileTree,
    pub search_hits: Vec<(String, u32, String)>,
    pub search_query: String,
    pub notice: Option<String>,

    pub git: git::GitStatus,
    pub git_loading: bool,
    pub commit_input: Entity<InputState>,

    pub store: Option<kestrel_storage::Store>,
    pub harness: harness::HarnessState,
}

impl KestrelApp {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = workspace_root();
        let store = open_store();
        let (projects, mut all_sessions) = match store.as_ref() {
            Some(store) => {
                let mut projects = store.projects().unwrap_or_default();
                if projects.is_empty() {
                    projects = demo::projects(&root)
                        .into_iter()
                        .filter(|project| Path::new(&project.path).is_dir())
                        .collect();
                    if projects.is_empty() {
                        projects = demo::projects(&root);
                    }
                    for project in &projects {
                        let _ = store.upsert_project(project);
                    }
                }
                (projects, store.sessions(None).unwrap_or_default())
            }
            None => (demo::projects(&root), demo::sessions()),
        };
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Kestrel to build, fix, or explain…")
                .rows(2)
        });
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.send_prompt(window, cx);
                }
            },
        );
        let file_tree = FileTree::load(projects[0].path.clone());
        let search_hits = demo::search_hits(&projects[0].path);

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search the whole project…")
                .clean_on_escape()
        });
        let search_subscription = cx.subscribe_in(
            &search_input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.search_query = this.search_input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.run_search(cx),
                _ => {}
            },
        );

        let command_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type a command or search the workspace…")
                .clean_on_escape()
        });
        let command_subscription = cx.subscribe_in(
            &command_query,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.command_text = this.command_query.read(cx).value().to_string();
                    this.command_index = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    let command = this
                        .filtered_commands()
                        .get(this.command_index)
                        .map(|item| item.command);
                    if let Some(command) = command {
                        this.run_command(command, window, cx);
                    }
                }
                _ => {}
            },
        );

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));

        let custom_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("your-agent --acp")
                .default_value("your-agent --acp")
        });
        let custom_command_subscription = cx.subscribe_in(
            &custom_command_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.custom_harness_command =
                        this.custom_command_input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );

        let first_project = projects[0].id.clone();
        let mut sessions: Vec<SessionSummary> = all_sessions
            .iter()
            .filter(|session| session.project_id == first_project)
            .cloned()
            .collect();
        if sessions.is_empty() {
            let session = blank_session(&first_project, "OpenCode", projects[0].branch.clone());
            if let Some(store) = &store {
                let _ = store.upsert_session(&session);
            }
            all_sessions.push(session.clone());
            sessions.push(session);
        }
        let fallback_session = blank_session("none", "OpenCode", None);

        let mut app = Self {
            view: MainView::Chat,
            section: SidebarSection::Files,
            projects,
            selected_project: 0,
            sessions,
            all_sessions,
            fallback_session,
            selected_session: 0,
            usages: demo::usage(),
            nodes: demo::agent_nodes(),
            spans: demo::trace_spans(),
            composer,
            _composer_subscription: composer_subscription,
            search_input,
            _search_subscription: search_subscription,
            focus_handle,
            command_open: false,
            command_query,
            command_text: String::new(),
            command_index: 0,
            _command_subscription: command_subscription,
            settings_open: false,
            settings_tab: SettingsTab::Harnesses,
            default_harness: "OpenCode".into(),
            custom_harness_command: "your-agent --acp".into(),
            custom_command_input,
            _custom_command_subscription: custom_command_subscription,
            permission_mode: PermissionMode::Ask,
            project_menu_open: false,
            auto_connect: true,
            restart_harness: true,
            word_wrap: false,
            allow_network: false,
            inspector: None,
            selected_span: None,
            selected_node: None,
            reasoning_open: false,
            expanded_tool: None,
            follow_running: true,
            graph_zoom: 1.0,
            trace_zoom: 1.0,
            trace_filter: TraceFilter::All,
            file_tree,
            search_hits,
            search_query: String::new(),
            notice: None,
            git: git::GitStatus::default(),
            git_loading: false,
            commit_input,
            store,
            harness: harness::HarnessState::default(),
        };

        let first_session = app.session().id.clone();
        app.load_transcript(&first_session);
        if app.auto_connect {
            app.connect_harness(cx);
        }
        app.refresh_git_status(cx);

        app
    }

    pub(crate) fn project(&self) -> &Project {
        &self.projects[self.selected_project]
    }

    pub(crate) fn session(&self) -> &SessionSummary {
        self.sessions
            .get(self.selected_session)
            .unwrap_or(&self.fallback_session)
    }

    /// Every command currently available to the palette, before filtering.
    pub(crate) fn commands(&self) -> Vec<CommandItem> {
        let mut items = vec![
            CommandItem {
                command: Command::View(MainView::Chat),
                label: "Go to Chat".into(),
                hint: Some("⌘1".into()),
                icon: IconName::MessageSquare,
                keywords: "conversation messages agent".into(),
            },
            CommandItem {
                command: Command::View(MainView::Graph),
                label: "Go to Agent Graph".into(),
                hint: Some("⌘2".into()),
                icon: IconName::Waypoints,
                keywords: "subagents orchestration nodes".into(),
            },
            CommandItem {
                command: Command::View(MainView::Trace),
                label: "Go to Trace".into(),
                hint: Some("⌘3".into()),
                icon: IconName::ChartNoAxesGantt,
                keywords: "waterfall spans timings cost".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Search),
                label: "Search in Project".into(),
                hint: Some("⌘⇧F".into()),
                icon: IconName::Search,
                keywords: "find grep text files".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Files),
                label: "Show Explorer".into(),
                hint: None,
                icon: IconName::Files,
                keywords: "files tree folders".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::SourceControl),
                label: "Show Source Control".into(),
                hint: None,
                icon: IconName::GitBranch,
                keywords: "git changes diff commit".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Sessions),
                label: "Show Sessions".into(),
                hint: None,
                icon: IconName::Clock3,
                keywords: "history runs conversations".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Projects),
                label: "Show Projects".into(),
                hint: None,
                icon: IconName::LayoutGrid,
                keywords: "switch workspace folders".into(),
            },
            CommandItem {
                command: Command::NewSession,
                label: "New Session".into(),
                hint: Some("⌘N".into()),
                icon: IconName::Plus,
                keywords: "start chat conversation".into(),
            },
            CommandItem {
                command: Command::RefreshGit,
                label: "Refresh Source Control".into(),
                hint: None,
                icon: IconName::RefreshCw,
                keywords: "git status changes reload".into(),
            },
            CommandItem {
                command: Command::OpenFolder,
                label: "Open Folder…".into(),
                hint: Some("⌘O".into()),
                icon: IconName::FolderOpen,
                keywords: "add project workspace directory".into(),
            },
            CommandItem {
                command: Command::Settings(SettingsTab::General),
                label: "Open Settings".into(),
                hint: Some("⌘,".into()),
                icon: IconName::Settings2,
                keywords: "preferences configuration".into(),
            },
            CommandItem {
                command: Command::Settings(SettingsTab::Harnesses),
                label: "Configure Harnesses".into(),
                hint: None,
                icon: IconName::Bot,
                keywords: "opencode claude codex acp agent".into(),
            },
            CommandItem {
                command: Command::Settings(SettingsTab::Usage),
                label: "Show Usage & Limits".into(),
                hint: None,
                icon: IconName::ChartNoAxesColumn,
                keywords: "quota subscription tokens cost".into(),
            },
            CommandItem {
                command: Command::ToggleTheme,
                label: "Toggle Light / Dark".into(),
                hint: Some("⌘⇧L".into()),
                icon: IconName::Sun,
                keywords: "appearance theme mode".into(),
            },
            CommandItem {
                command: Command::RestartHarness,
                label: format!("Restart {}", self.default_harness),
                hint: None,
                icon: IconName::RefreshCw,
                keywords: "reconnect harness agent".into(),
            },
            CommandItem {
                command: Command::CloseInspector,
                label: "Close File Inspector".into(),
                hint: Some("esc".into()),
                icon: IconName::X,
                keywords: "hide panel editor".into(),
            },
        ];

        for (index, project) in self.projects.iter().enumerate() {
            items.push(CommandItem {
                command: Command::SwitchProject(index),
                label: format!("Switch to {}", project.name),
                hint: Some("project".into()),
                icon: IconName::Folder,
                keywords: project.path.clone(),
            });
        }

        items
    }

    /// Filter commands by every whitespace-separated term in the query.
    pub(crate) fn filtered_commands(&self) -> Vec<CommandItem> {
        let query = self.command_text.trim().to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            return self.commands();
        }

        self.commands()
            .into_iter()
            .filter(|item| command_matches(item, &terms))
            .collect()
    }

    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_open = true;
        self.command_index = 0;
        self.settings_open = false;
        self.project_menu_open = false;
        self.command_query
            .update(cx, |state, cx| state.set_value("", window, cx));
        let handle = self.command_query.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(crate) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.command_open {
            return;
        }
        self.command_open = false;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn move_command_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.filtered_commands().len();
        if len == 0 {
            self.command_index = 0;
        } else {
            let current = self.command_index.min(len - 1) as isize;
            self.command_index = (current + delta).rem_euclid(len as isize) as usize;
        }
        cx.notify();
    }

    pub(crate) fn run_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_open = false;
        match command {
            Command::View(view) => {
                self.view = view;
                self.settings_open = false;
            }
            Command::Section(section) => {
                self.section = section;
                self.settings_open = false;
                if section == SidebarSection::Search {
                    let handle = self.search_input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                } else if section == SidebarSection::SourceControl && !self.git_loading {
                    self.refresh_git_status(cx);
                }
            }
            Command::Settings(tab) => {
                self.settings_open = true;
                self.settings_tab = tab;
            }
            Command::OpenFolder => self.open_folder(cx),
            Command::ToggleTheme => {
                toggle_mode(window, cx);
            }
            Command::NewSession => self.new_session(cx),
            Command::RestartHarness => self.restart_harness(cx),
            Command::RefreshGit => self.refresh_git_status(cx),
            Command::SwitchProject(index) => self.select_project(index, cx),
            Command::CloseInspector => {
                self.inspector = None;
                self.notice = Some("Closed file inspector".into());
            }
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Open the native macOS folder picker and add the chosen folder.
    pub(crate) fn open_folder(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open a project folder".into()),
        });

        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.add_project(path, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub(crate) fn add_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = path.to_string_lossy().trim_end_matches('/').to_string();
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.path.trim_end_matches('/') == path)
        {
            self.select_project(index, cx);
            self.notice = Some(format!("{} is already open", self.projects[index].name));
            cx.notify();
            return;
        }

        let name = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&path)
            .to_string();
        let branch = git_branch(&path);
        let tone = match self.projects.len() % 6 {
            0 => ProjectTone::Blue,
            1 => ProjectTone::Green,
            2 => ProjectTone::Violet,
            3 => ProjectTone::Amber,
            4 => ProjectTone::Rose,
            _ => ProjectTone::Teal,
        };
        let id = format!("project-{}", self.projects.len() + 1);

        let project = Project {
            id,
            name: name.clone(),
            path,
            branch,
            tone,
            pinned: false,
            trusted: true,
            running_sessions: 0,
        };
        if let Some(store) = &self.store {
            let _ = store.upsert_project(&project);
        }
        self.projects.push(project);
        let index = self.projects.len() - 1;
        self.select_project(index, cx);
        self.notice = Some(format!("Added {name}"));
        cx.notify();
    }

    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        let project_id = self.project().id.clone();
        let branch = self.project().branch.clone();
        let session = blank_session(&project_id, &self.default_harness, branch);
        self.stash_transcript();
        self.persist_session(&session);
        self.all_sessions.push(session.clone());
        self.sessions.push(session.clone());
        self.selected_session = self.sessions.len() - 1;
        self.load_transcript(&session.id);
        self.harness.session_id = None;
        self.harness.running = false;

        if self.harness.status.is_ready() {
            let path = PathBuf::from(&self.project().path);
            if let Some(harness) = self.harness.harness.as_ref() {
                harness.new_session(path);
            }
        } else if self.auto_connect && !self.harness.status.is_busy() {
            self.connect_harness(cx);
        }

        self.view = MainView::Chat;
        self.settings_open = false;
        self.notice = Some("Created a new session".into());
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let platform = keystroke.modifiers.platform;
        let shift = keystroke.modifiers.shift;

        if self.command_open {
            match keystroke.key.as_str() {
                "escape" => {
                    self.close_palette(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "up" => {
                    self.move_command_selection(-1, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" => {
                    self.move_command_selection(1, cx);
                    cx.stop_propagation();
                    return;
                }
                "enter" => {
                    let command = self
                        .filtered_commands()
                        .get(self.command_index)
                        .map(|item| item.command);
                    if let Some(command) = command {
                        self.run_command(command, window, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        if platform {
            let handled = match keystroke.key.as_str() {
                "k" => {
                    self.open_palette(window, cx);
                    true
                }
                "1" => {
                    self.view = MainView::Chat;
                    self.settings_open = false;
                    cx.notify();
                    true
                }
                "2" => {
                    self.view = MainView::Graph;
                    self.settings_open = false;
                    cx.notify();
                    true
                }
                "3" => {
                    self.view = MainView::Trace;
                    self.settings_open = false;
                    cx.notify();
                    true
                }
                "," => {
                    self.settings_open = true;
                    self.settings_tab = SettingsTab::General;
                    cx.notify();
                    true
                }
                "o" => {
                    self.open_folder(cx);
                    true
                }
                "n" => {
                    self.new_session(cx);
                    true
                }
                "f" if shift => {
                    self.section = SidebarSection::Search;
                    self.settings_open = false;
                    let handle = self.search_input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                    cx.notify();
                    true
                }
                "l" if shift => {
                    toggle_mode(window, cx);
                    cx.notify();
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }

        if keystroke.key == "escape" {
            if self.settings_open {
                self.settings_open = false;
                self.focus_handle.focus(window, cx);
                cx.stop_propagation();
                cx.notify();
            } else if self.project_menu_open {
                self.project_menu_open = false;
                cx.stop_propagation();
                cx.notify();
            } else if self.inspector.is_some() {
                self.inspector = None;
                cx.stop_propagation();
                cx.notify();
            }
        }
    }

    pub(crate) fn run_search(&mut self, cx: &mut Context<Self>) {
        let root = self.project().path.clone();
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            self.search_hits = demo::search_hits(&root);
            self.notice = Some("Showing recent matches. Type a query and press Enter.".into());
        } else {
            let hits = search::search_project(&root, &query, 80);
            let count = hits.len();
            self.search_hits = hits;
            self.notice = Some(if count == 0 {
                format!("No matches for “{query}”")
            } else {
                format!("{count} matches for “{query}”")
            });
        }
        cx.notify();
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.projects.len() {
            return;
        }
        self.stash_transcript();
        self.selected_project = index;
        self.project_menu_open = false;
        let path = self.projects[index].path.clone();
        self.file_tree = FileTree::load(path.clone());
        self.search_hits = if path == workspace_root() {
            demo::search_hits(&path)
        } else {
            Vec::new()
        };
        self.search_query.clear();
        self.inspector = None;

        let project_id = self.projects[index].id.clone();
        self.sessions = self
            .all_sessions
            .iter()
            .filter(|session| session.project_id == project_id)
            .cloned()
            .collect();
        if self.sessions.is_empty() {
            let session = blank_session(
                &project_id,
                &self.default_harness,
                self.projects[index].branch.clone(),
            );
            self.persist_session(&session);
            self.all_sessions.push(session.clone());
            self.sessions.push(session);
        }
        self.selected_session = 0;
        let first_session = self.sessions[0].id.clone();
        self.load_transcript(&first_session);
        self.harness.session_id = None;
        self.harness.commands.clear();

        self.notice = Some(format!("Switched to {}", self.projects[index].name));
        if self.auto_connect {
            self.connect_harness(cx);
        } else {
            self.disconnect_harness(cx);
            self.notice = Some(format!("Switched to {}", self.projects[index].name));
        }
        self.refresh_git_status(cx);
        cx.notify();
    }

    /// Read branch and working-tree state off the UI thread.
    pub(crate) fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let path = self.project().path.clone();
        let expected = path.clone();
        self.git_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async move { git::load(&path) })
                .await;
            this.update(cx, |this, cx| {
                if this.project().path == expected {
                    this.git = status;
                    this.git_loading = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn stage_all_changes(&mut self, cx: &mut Context<Self>) {
        let path = self.project().path.clone();
        match git::stage_all(&path) {
            Ok(()) => {
                self.notice = Some("Staged all changes".into());
                self.refresh_git_status(cx);
            }
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn commit_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.commit_input.read(cx).value().to_string();
        let path = self.project().path.clone();
        match git::commit(&path, &message) {
            Ok(()) => {
                self.commit_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.notice = Some("Commit created".into());
                self.refresh_git_status(cx);
            }
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_file(&mut self, path: &str) {
        let root = self.project().path.clone();
        let path_buf = Path::new(path);
        let name = path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        let directory = path_buf
            .parent()
            .and_then(|p| p.strip_prefix(&root).ok())
            .map(|p| {
                if p.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    p.to_string_lossy().to_string()
                }
            })
            .unwrap_or_else(|| ".".to_string());

        let (lines, truncated, snippet) = match fs::read_to_string(path) {
            Ok(contents) => {
                if contents.contains('\0') {
                    (
                        vec!["Binary file — preview is not available.".to_string()],
                        false,
                        None,
                    )
                } else {
                    let total = contents.lines().count();
                    let lines = contents
                        .lines()
                        .take(600)
                        .map(|line| line.replace('\t', "    "))
                        .collect();
                    (lines, total > 600, None)
                }
            }
            Err(error) => (
                vec![format!("Unable to read this file: {error}")],
                false,
                None,
            ),
        };

        self.inspector = Some(Inspector {
            path: path.to_string(),
            name,
            directory,
            lines,
            snippet,
            truncated,
        });
    }

    pub(crate) fn open_search_hit(&mut self, path: &str, line: u32, preview: &str) {
        self.open_file(path);
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.snippet = Some((line, line));
            if inspector.lines.len() < line as usize {
                inspector.lines.push(preview.to_string());
            }
        }
    }

    pub(crate) fn toggle_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        self.file_tree.toggle(path);
        cx.notify();
    }

    pub(crate) fn set_notice(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(text.into());
        cx.notify();
    }

    fn rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut rail = div()
            .w(px(52.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .py(px(9.))
            .gap(px(3.))
            .bg(p.panel)
            .border_r_1()
            .border_color(p.border);

        let items = [
            (
                SidebarSection::Projects,
                IconName::LayoutGrid,
                self.projects.len(),
                false,
            ),
            (SidebarSection::Files, IconName::Files, 0, false),
            (SidebarSection::Search, IconName::Search, 0, false),
            (SidebarSection::SourceControl, IconName::GitBranch, 4, true),
            (
                SidebarSection::Sessions,
                IconName::Clock3,
                self.sessions.len(),
                false,
            ),
        ];

        for (section, icon_name, count, has_badge) in items {
            let active = self.section == section && !self.settings_open;
            let color = if active { p.fg } else { p.muted_fg };
            let mut button = div()
                .id(("rail", section as usize))
                .relative()
                .size(px(36.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.settings_open = false;
                    cx.notify();
                }))
                .child(icon(icon_name, 17.0, color));

            if active {
                button = button.child(
                    div()
                        .absolute()
                        .left(px(-8.))
                        .h(px(16.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(p.fg),
                );
            }
            if has_badge && count > 0 {
                button = button.child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(p.amber)
                        .border_1()
                        .border_color(p.panel),
                );
            }
            rail = rail.child(button);
        }

        rail.child(div().flex_1())
            .child(
                div()
                    .id("rail-usage")
                    .size(px(34.))
                    .rounded(px(8.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(2.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = true;
                        this.settings_tab = SettingsTab::Usage;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(2.))
                            .h(px(13.))
                            .child(div().w(px(3.)).h(px(6.)).rounded(px(1.)).bg(p.violet))
                            .child(div().w(px(3.)).h(px(10.)).rounded(px(1.)).bg(p.blue))
                            .child(div().w(px(3.)).h(px(13.)).rounded(px(1.)).bg(p.teal)),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
                            .text_color(p.muted_fg)
                            .child("62%"),
                    ),
            )
            .child(
                div()
                    .id("rail-settings")
                    .size(px(36.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(self.settings_open, |this| this.bg(p.accent))
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = !this.settings_open;
                        this.settings_tab = SettingsTab::Harnesses;
                        cx.notify();
                    }))
                    .child(icon(
                        IconName::Settings,
                        16.5,
                        if self.settings_open { p.fg } else { p.muted_fg },
                    )),
            )
            .into_any_element()
    }

    fn session_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let project = self.project();
        let session = self.session();
        let status = status_palette(&p, session.status);

        div()
            .h(px(62.))
            .flex_shrink_0()
            .px(px(20.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(p.border)
            .bg(p.bg)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.fg)
                                    .truncate()
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .h(px(19.))
                                    .px(px(7.))
                                    .rounded_full()
                                    .bg(status.1)
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(dot(status.0, 5.0))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(status.0)
                                            .child(session.status.label()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(project.name.clone())
                            .child(div().text_color(p.border_strong).child("·"))
                            .child(icon(IconName::GitBranch, 10.0, p.muted_fg))
                            .child(session.branch.clone().unwrap_or_else(|| "detached".into()))
                            .child(div().text_color(p.border_strong).child("·"))
                            .child(crate::views::chat::message_count_label(
                                self.harness.transcript.len(),
                            ))
                            .child(div().text_color(p.border_strong).child("·"))
                            .child(format_tokens(session.tokens))
                            .child(div().text_color(p.border_strong).child("·"))
                            .child(format!("${:.2}", session.cost_usd)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .h(px(26.))
                            .px(px(9.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.card)
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon(IconName::Sparkles, 12.0, p.violet))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .child(session.model.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .font_family("Geist Mono")
                                    .text_color(p.muted_fg)
                                    .child(session.harness.to_uppercase()),
                            ),
                    )
                    .child(
                        div()
                            .id("header-fork")
                            .size(px(28.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_notice("Session forked into a new draft", cx);
                            }))
                            .child(icon(IconName::GitFork, 14.0, p.muted_fg)),
                    )
                    .child(
                        div()
                            .id("header-more")
                            .size(px(28.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .child(icon(IconName::Ellipsis, 15.0, p.muted_fg)),
                    ),
            )
            .into_any_element()
    }

    fn main_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.view {
            MainView::Chat => views::chat::render(self, window, cx),
            MainView::Graph => views::graph::render(self, window, cx),
            MainView::Trace => views::trace::render(self, window, cx),
        };

        let inspector = self
            .inspector
            .as_ref()
            .map(|inspector| self.inspector_panel(inspector, cx));

        let mut row = div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(content);
        if let Some(inspector) = inspector {
            row = row.child(inspector);
        }
        row.into_any_element()
    }

    fn inspector_panel(&self, inspector: &Inspector, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut code = div()
            .id("inspector-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .py(px(10.));

        for (ix, line) in inspector.lines.iter().enumerate() {
            let number = ix + 1;
            let highlighted = inspector
                .snippet
                .map(|(start, end)| number >= start as usize && number <= end as usize)
                .unwrap_or(false);
            let trimmed = line.trim_start();
            let color = if trimmed.starts_with("//") || trimmed.starts_with('#') {
                p.muted_fg
            } else if highlighted {
                p.fg
            } else {
                p.fg.opacity(0.86)
            };

            code = code.child(
                div()
                    .flex()
                    .items_start()
                    .when(highlighted, |this| this.bg(p.blue_soft))
                    .child(
                        div()
                            .w(px(46.))
                            .flex_shrink_0()
                            .pr(px(10.))
                            .text_right()
                            .text_size(px(10.))
                            .font_family("Geist Mono")
                            .text_color(if highlighted {
                                p.blue
                            } else {
                                p.muted_fg.opacity(0.65)
                            })
                            .child(number.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .pr(px(12.))
                            .text_size(px(11.))
                            .line_height(relative(1.6))
                            .font_family("Geist Mono")
                            .text_color(color)
                            .child(if line.is_empty() {
                                " ".into()
                            } else {
                                line.clone()
                            }),
                    ),
            );
        }

        div()
            .w(px(420.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(p.border)
            .bg(p.card)
            .child(
                div()
                    .h(px(46.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(6.))
                            .bg(p.blue_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::FileCode, 13.0, p.blue)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .truncate()
                                    .child(inspector.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .font_family("Geist Mono")
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(inspector.directory.clone()),
                            ),
                    )
                    .child(
                        div()
                            .id("inspector-close")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector = None;
                                cx.notify();
                            }))
                            .child(icon(IconName::X, 13.0, p.muted_fg)),
                    ),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap_4()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .border_b_2()
                            .border_color(p.fg)
                            .text_size(px(10.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.fg)
                            .child("Preview"),
                    )
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child("Outline"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(9.5))
                            .font_family("Geist Mono")
                            .text_color(p.muted_fg)
                            .child(format!(
                                "{}{}",
                                inspector.lines.len(),
                                if inspector.truncated { "+" } else { "" }
                            )),
                    ),
            )
            .child(code)
            .into_any_element()
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let session = self.session();
        div()
            .h(px(27.))
            .flex_shrink_0()
            .px(px(11.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_t_1()
            .border_color(p.border)
            .bg(p.statusbar)
            .text_size(px(10.))
            .text_color(p.muted_fg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(IconName::GitBranch, 11.0, p.muted_fg))
                    .child(
                        div()
                            .font_family("Geist Mono")
                            .text_color(p.fg.opacity(0.8))
                            .child(session.branch.clone().unwrap_or_else(|| "main".into())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(IconName::ArrowUp, 10.0, p.muted_fg))
                    .child("2")
                    .child(icon(IconName::ArrowDown, 10.0, p.muted_fg))
                    .child("0"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(IconName::CircleCheck, 11.0, p.green))
                    .child("0 problems"),
            )
            .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            .child({
                let (harness_color, _) = crate::views::chat::harness_tone(&p, &self.harness.status);
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(dot(harness_color, 5.0))
                    .child(self.harness.status.label())
            })
            .children(self.notice.as_ref().map(|notice| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(p.blue)
                    .child(icon(IconName::Info, 10.0, p.blue))
                    .child(notice.clone())
            }))
            .child(div().flex_1())
            .child(div().font_family("Geist Mono").child(format!(
                "${:.2} · {} tok",
                session.cost_usd,
                format_tokens(session.tokens)
            )))
            .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(dot(p.violet, 5.0))
                            .child(
                                self.harness
                                    .usage
                                    .map(|usage| format!("{:.0}% context", usage.percent() * 100.0))
                                    .unwrap_or_else(|| "context idle".into()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(dot(p.blue, 5.0))
                            .child(format!("{} cmds", self.harness.commands.len())),
                    ),
            )
            .into_any_element()
    }
}

impl Render for KestrelApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let titlebar = views::titlebar::render(self, window, cx);
        let rail = self.rail(cx);
        let sidebar = views::sidebar::render(self, window, cx);
        let header = self.session_header(cx);
        let main_panel = self.main_panel(window, cx);
        let status_bar = self.status_bar(cx);
        let settings = self
            .settings_open
            .then(|| views::settings::render(self, window, cx));
        let palette = self
            .command_open
            .then(|| views::palette::render(self, window, cx));

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family("Geist")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(titlebar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .child(rail)
                    .child(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .border_l_1()
                            .border_color(p.border)
                            .child(header)
                            .child(main_panel),
                    ),
            )
            .child(status_bar)
            .children(settings)
            .children(palette)
    }
}

impl Drop for KestrelApp {
    fn drop(&mut self) {
        self.save_current_transcript();
        self.save_stashed_transcripts();
    }
}

fn workspace_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").to_string())
}

fn open_store() -> Option<kestrel_storage::Store> {
    let base = std::env::var_os("KESTREL_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Application Support/Kestrel"))
        })?;
    match kestrel_storage::Store::open(base.join("kestrel.db")) {
        Ok(store) => Some(store),
        Err(error) => {
            eprintln!("[storage] could not open database: {error}");
            None
        }
    }
}

fn blank_session(project_id: &str, harness: &str, branch: Option<String>) -> SessionSummary {
    SessionSummary {
        id: format!("ses-{}", uuid::Uuid::now_v7()),
        project_id: project_id.to_string(),
        title: "New session".into(),
        status: SessionStatus::Waiting,
        harness: harness.to_string(),
        model: "Auto".into(),
        branch,
        updated_at: "now".into(),
        cost_usd: 0.0,
        duration_secs: 0,
        tokens: 0,
        summary: "No messages yet. Ask Kestrel to build, fix, or explain something.".into(),
    }
}

fn git_branch(path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

pub(crate) fn tone_color(p: &Palette, tone: ProjectTone) -> Hsla {
    match tone {
        ProjectTone::Blue => p.blue,
        ProjectTone::Green => p.green,
        ProjectTone::Violet => p.violet,
        ProjectTone::Amber => p.amber,
        ProjectTone::Rose => p.red,
        ProjectTone::Teal => p.teal,
    }
}

pub(crate) fn tone_soft(p: &Palette, tone: ProjectTone) -> Hsla {
    match tone {
        ProjectTone::Blue => p.blue_soft,
        ProjectTone::Green => p.green_soft,
        ProjectTone::Violet => p.violet_soft,
        ProjectTone::Amber => p.amber_soft,
        ProjectTone::Rose => p.red_soft,
        ProjectTone::Teal => p.blue_soft,
    }
}

pub(crate) fn status_palette(p: &Palette, status: SessionStatus) -> (Hsla, Hsla) {
    match status {
        SessionStatus::Running => (p.blue, p.blue_soft),
        SessionStatus::Waiting => (p.amber, p.amber_soft),
        SessionStatus::Done => (p.green, p.green_soft),
        SessionStatus::Failed => (p.red, p.red_soft),
        SessionStatus::Interrupted => (p.muted_fg, p.muted),
    }
}

pub(crate) fn agent_status_palette(p: &Palette, status: AgentStatus) -> (Hsla, Hsla) {
    match status {
        AgentStatus::Running => (p.blue, p.blue_soft),
        AgentStatus::Waiting => (p.amber, p.amber_soft),
        AgentStatus::Done => (p.green, p.green_soft),
        AgentStatus::Failed => (p.red, p.red_soft),
        AgentStatus::Stopped => (p.muted_fg, p.muted),
    }
}

pub(crate) fn span_color(p: &Palette, kind: SpanKind) -> Hsla {
    match kind {
        SpanKind::Agent => p.blue,
        SpanKind::Turn => p.fg.opacity(0.72),
        SpanKind::Llm => p.violet,
        SpanKind::Tool => p.teal,
        SpanKind::Permission => p.amber,
        SpanKind::Waiting => p.muted_fg,
        SpanKind::Handoff => p.green,
        SpanKind::Compaction => p.red,
    }
}

pub(crate) fn tool_icon(kind: ToolKind) -> IconName {
    match kind {
        ToolKind::Read => IconName::FileText,
        ToolKind::Edit => IconName::PencilLine,
        ToolKind::Shell => IconName::SquareTerminal,
        ToolKind::Search => IconName::Search,
        ToolKind::Web => IconName::Globe,
        ToolKind::Task => IconName::ListTodo,
        ToolKind::Test => IconName::FlaskConical,
        ToolKind::Plan => IconName::Map,
    }
}

pub(crate) fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

pub(crate) fn format_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, CommandItem, command_matches};
    use kestrel_domain::MainView;
    use kestrel_ui::assets::IconName;

    fn item(label: &str, keywords: &str, hint: Option<&str>) -> CommandItem {
        CommandItem {
            command: Command::View(MainView::Chat),
            label: label.into(),
            hint: hint.map(str::to_string),
            icon: IconName::Search,
            keywords: keywords.into(),
        }
    }

    #[test]
    fn command_filter_is_case_insensitive() {
        let item = item("Toggle theme", "appearance dark light", Some("⌘⇧L"));
        assert!(command_matches(&item, &["theme"]));
        assert!(command_matches(&item, &["TOGGLE"]));
        assert!(!command_matches(&item, &["folder"]));
    }

    #[test]
    fn command_filter_matches_keywords_and_hints() {
        let item = item("Open folder", "workspace project directory", Some("⌘O"));
        assert!(command_matches(&item, &["workspace"]));
        assert!(command_matches(&item, &["⌘o"]));
        assert!(command_matches(&item, &["open", "folder"]));
    }

    #[test]
    fn command_filter_requires_every_term() {
        let item = item("New session", "chat conversation", Some("⌘N"));
        assert!(command_matches(&item, &["new", "session"]));
        assert!(!command_matches(&item, &["new", "theme"]));
    }
}
