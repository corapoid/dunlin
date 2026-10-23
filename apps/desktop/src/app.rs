use std::fs;
use std::path::{Path, PathBuf};

use kestrel_domain::*;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::diff::{Diff, DiffFile, DiffState};
use kestrel_ui::component::input::{
    Editor, EditorState, InputEvent, InputState, Position, TextareaState,
};
use kestrel_ui::component::{ActiveTheme, ThemeMode};
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Everything Kestrel remembers between launches. Stored as one JSON value in
/// SQLite so new fields can be added without another migration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredSettings {
    pub default_harness: String,
    pub custom_harness_command: String,
    pub permission_mode: PermissionMode,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub code_font_size: f32,
    pub allow_network: bool,
    pub reasoning_open: bool,
    pub dark: bool,
    pub view: MainView,
    pub last_project_id: Option<String>,
    pub last_session_id: Option<String>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            default_harness: "OpenCode".into(),
            custom_harness_command: "your-agent --acp".into(),
            permission_mode: PermissionMode::Ask,
            auto_connect: true,
            restart_harness: true,
            restore_session: true,
            word_wrap: false,
            code_font_size: 13.0,
            allow_network: false,
            reasoning_open: false,
            dark: true,
            view: MainView::Chat,
            last_project_id: None,
            last_session_id: None,
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

#[derive(Clone)]
pub(crate) struct Inspector {
    pub path: String,
    pub name: String,
    pub directory: String,
    pub language: String,
    pub bytes: usize,
    pub snippet: Option<(u32, u32)>,
}

/// A destructive action waiting for confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingDelete {
    Session { id: String, title: String },
    Project { id: String, name: String },
}

/// The read-only unified patch shown for one source-control change.
pub(crate) struct DiffView {
    pub path: String,
    pub name: String,
    pub staged: bool,
    pub state: Entity<DiffState>,
    pub error: Option<String>,
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
    NewFile,
    DeleteSession,
    RemoveProject,
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
    pub pending_delete: Option<PendingDelete>,
    pub diff_view: Option<DiffView>,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub code_font_size: f32,
    pub allow_network: bool,

    pub inspector: Option<Inspector>,
    pub editor: Entity<EditorState>,
    _editor_subscription: Subscription,
    pub diff_state: Entity<DiffState>,
    pub editor_dirty: bool,
    pub editor_readonly: bool,
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
        let stored = store
            .as_ref()
            .and_then(|store| store.load_setting::<StoredSettings>("app").ok().flatten())
            .unwrap_or_default();
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
        // A live turn cannot survive a restart, so clear statuses that imply one.
        for session in &mut all_sessions {
            if matches!(
                session.status,
                SessionStatus::Running | SessionStatus::Waiting
            ) {
                session.status = SessionStatus::Interrupted;
                if let Some(store) = &store {
                    let _ = store.upsert_session(session);
                }
            }
        }
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

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("plaintext")
                .soft_wrap(stored.word_wrap)
        });
        let editor_subscription =
            cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.editor_dirty = true;
                    cx.notify();
                }
            });

        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));

        let diff_state = cx.new(|cx| DiffState::new(Vec::new(), cx));

        let custom_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("your-agent --acp")
                .default_value(stored.custom_harness_command.clone())
        });
        let custom_command_subscription = cx.subscribe_in(
            &custom_command_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.custom_harness_command =
                        this.custom_command_input.read(cx).value().to_string();
                    this.persist_settings(cx);
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
            let session = blank_session(
                &first_project,
                &stored.default_harness,
                projects[0].branch.clone(),
            );
            if let Some(store) = &store {
                let _ = store.upsert_session(&session);
            }
            all_sessions.push(session.clone());
            sessions.push(session);
        }
        let fallback_session = blank_session("none", &stored.default_harness, None);

        let mut app = Self {
            view: stored.view,
            section: SidebarSection::Files,
            projects,
            selected_project: 0,
            sessions,
            all_sessions,
            fallback_session,
            selected_session: 0,
            usages: demo::usage(),
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
            default_harness: stored.default_harness.clone(),
            custom_harness_command: stored.custom_harness_command.clone(),
            custom_command_input,
            _custom_command_subscription: custom_command_subscription,
            permission_mode: stored.permission_mode,
            project_menu_open: false,
            pending_delete: None,
            diff_view: None,
            auto_connect: stored.auto_connect,
            restart_harness: stored.restart_harness,
            restore_session: stored.restore_session,
            word_wrap: stored.word_wrap,
            code_font_size: stored.code_font_size,
            allow_network: stored.allow_network,
            inspector: None,
            editor,
            _editor_subscription: editor_subscription,
            diff_state,
            editor_dirty: false,
            editor_readonly: false,
            selected_span: None,
            selected_node: None,
            reasoning_open: stored.reasoning_open,
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

        kestrel_ui::theme::set_mode(
            if stored.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
        kestrel_ui::theme::set_code_font_size(stored.code_font_size, cx);

        let first_session = app.session().id.clone();
        app.load_transcript(&first_session);

        let mut restored_project = false;
        if app.restore_session {
            if let Some(index) = stored
                .last_project_id
                .as_deref()
                .and_then(|id| app.projects.iter().position(|project| project.id == id))
            {
                app.select_project(index, cx);
                restored_project = true;
            }
            if let Some(index) = stored
                .last_session_id
                .as_deref()
                .and_then(|id| app.sessions.iter().position(|session| session.id == id))
            {
                app.select_session(index, cx);
            }
        }

        if app.auto_connect && !app.harness.status.is_busy() {
            app.connect_harness(cx);
        }
        if !restored_project {
            app.refresh_git_status(cx);
        }
        app.persist_settings(cx);

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
                command: Command::NewFile,
                label: "New File…".into(),
                hint: None,
                icon: IconName::FilePlus,
                keywords: "create add file explorer".into(),
            },
            CommandItem {
                command: Command::DeleteSession,
                label: "Delete Current Session".into(),
                hint: None,
                icon: IconName::Trash,
                keywords: "remove delete conversation history".into(),
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

        if self.projects.len() > 1 {
            items.push(CommandItem {
                command: Command::RemoveProject,
                label: "Remove Current Project".into(),
                hint: None,
                icon: IconName::Trash,
                keywords: "delete remove workspace folder".into(),
            });
        }

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
                self.persist_settings(cx);
            }
            Command::NewSession => self.new_session(cx),
            Command::NewFile => self.new_file(window, cx),
            Command::DeleteSession => self.request_delete_session(self.selected_session, cx),
            Command::RemoveProject => self.request_delete_project(self.selected_project, cx),
            Command::RestartHarness => self.restart_harness(cx),
            Command::RefreshGit => self.refresh_git_status(cx),
            Command::SwitchProject(index) => self.select_project(index, cx),
            Command::CloseInspector => {
                let dirty = self.editor_dirty;
                self.close_inspector();
                if !dirty {
                    self.notice = Some("Closed file inspector".into());
                }
            }
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Ask macOS for a path inside the project, then create or open it.
    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.project().path.clone();
        let receiver = cx.prompt_for_new_path(Path::new(&root), Some("untitled.txt"));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, window, cx| this.create_file(path, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn create_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.project().path.clone();
        if !path_inside(Path::new(&root), &path) {
            self.set_notice("Choose a location inside the current project", cx);
            return;
        }

        let existed = path.exists();
        if !existed && let Err(error) = fs::write(&path, "") {
            self.set_notice(format!("Could not create the file: {error}"), cx);
            return;
        }

        let file = path.to_string_lossy().to_string();
        self.file_tree = FileTree::load(root);
        self.open_file(&file, window, cx);
        self.notice = Some(if existed {
            format!("Opened {file}")
        } else {
            format!("Created {file}")
        });
        self.refresh_git_status(cx);
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
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.add_project(path, cx)).ok();
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
        self.persist_settings(cx);
        cx.notify();
    }

    /// Ask for confirmation before deleting a session.
    pub(crate) fn request_delete_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(index) else {
            return;
        };
        self.pending_delete = Some(PendingDelete::Session {
            id: session.id.clone(),
            title: session.title.clone(),
        });
        cx.notify();
    }

    /// Ask for confirmation before removing a project.
    pub(crate) fn request_delete_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.projects.len() <= 1 {
            self.set_notice("Keep at least one project in Kestrel", cx);
            return;
        }
        let Some(project) = self.projects.get(index) else {
            return;
        };
        self.pending_delete = Some(PendingDelete::Project {
            id: project.id.clone(),
            name: project.name.clone(),
        });
        cx.notify();
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.pending_delete = None;
        cx.notify();
    }

    pub(crate) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        match self.pending_delete.take() {
            Some(PendingDelete::Session { id, .. }) => self.delete_session(&id, cx),
            Some(PendingDelete::Project { id, .. }) => self.delete_project(&id, cx),
            None => {}
        }
    }

    fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.sessions.iter().position(|session| session.id == id) else {
            return;
        };
        let active = index == self.selected_session;
        if active && self.harness.running {
            self.cancel_turn(cx);
        }
        self.sessions.remove(index);
        self.all_sessions.retain(|session| session.id != id);
        if let Some(store) = &self.store {
            let _ = store.delete_session(id);
        }
        self.forget_transcript(id);
        if self.sessions.is_empty() {
            let project_id = self.project().id.clone();
            let branch = self.project().branch.clone();
            let session = blank_session(&project_id, &self.default_harness, branch);
            self.persist_session(&session);
            self.all_sessions.push(session.clone());
            self.sessions.push(session);
        }
        self.selected_session =
            session_index_after_delete(index, self.selected_session, self.sessions.len());

        if active {
            let session_id = self.session().id.clone();
            self.harness.session_id = None;
            self.load_transcript(&session_id);
            if self.harness.status.is_ready() {
                let path = PathBuf::from(&self.project().path);
                if let Some(harness) = self.harness.harness.as_ref() {
                    harness.new_session(path);
                }
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
        }

        self.notice = Some("Deleted session".into());
        self.persist_settings(cx);
        cx.notify();
    }

    fn delete_project(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.projects.len() <= 1 {
            self.set_notice("Keep at least one project in Kestrel", cx);
            return;
        }
        let Some(index) = self.projects.iter().position(|project| project.id == id) else {
            return;
        };
        let active = index == self.selected_project;
        if active {
            if self.harness.running {
                self.cancel_turn(cx);
            }
            self.stash_transcript();
        }

        let removed_sessions: Vec<String> = self
            .all_sessions
            .iter()
            .filter(|session| session.project_id == id)
            .map(|session| session.id.clone())
            .collect();
        self.all_sessions.retain(|session| session.project_id != id);
        for session_id in &removed_sessions {
            self.forget_transcript(session_id);
        }
        if let Some(store) = &self.store {
            let _ = store.delete_project(id);
        }
        let removed = self.projects.remove(index);

        if active {
            self.selected_project = index.min(self.projects.len() - 1);
            let project_id = self.project().id.clone();
            self.sessions = self
                .all_sessions
                .iter()
                .filter(|session| session.project_id == project_id)
                .cloned()
                .collect();
            if self.sessions.is_empty() {
                let branch = self.project().branch.clone();
                let session = blank_session(&project_id, &self.default_harness, branch);
                self.persist_session(&session);
                self.all_sessions.push(session.clone());
                self.sessions.push(session);
            }
            self.selected_session = 0;
            let session_id = self.sessions[0].id.clone();
            let path = self.project().path.clone();
            self.harness.session_id = None;
            self.harness.commands.clear();
            self.load_transcript(&session_id);
            self.file_tree = FileTree::load(path.clone());
            self.search_query.clear();
            self.search_hits = if path == workspace_root() {
                demo::search_hits(&path)
            } else {
                Vec::new()
            };
            self.close_inspector();
            if self.auto_connect {
                self.connect_harness(cx);
            } else {
                self.disconnect_harness(cx);
            }
            self.refresh_git_status(cx);
        } else if index < self.selected_project {
            self.selected_project -= 1;
        }

        self.notice = Some(format!("Removed {}", removed.name));
        self.persist_settings(cx);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let platform = keystroke.modifiers.platform;
        let shift = keystroke.modifiers.shift;

        if self.pending_delete.is_some() {
            match keystroke.key.as_str() {
                "escape" => self.cancel_delete(cx),
                "enter" => self.confirm_delete(cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }

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
                "s" if self.inspector.is_some() => {
                    self.save_editor(cx);
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
                    self.persist_settings(cx);
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
            } else if self.diff_view.is_some() {
                self.close_diff(window, cx);
                cx.stop_propagation();
            } else if self.inspector.is_some() {
                self.close_inspector();
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
        self.close_inspector();

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
        self.persist_settings(cx);
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

    pub(crate) fn stage_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let path = self.project().path.clone();
        match git::stage_file(&path, file) {
            Ok(()) => {
                self.notice = Some(format!("Staged {file}"));
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
    }

    pub(crate) fn unstage_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let path = self.project().path.clone();
        match git::unstage_file(&path, file) {
            Ok(()) => {
                self.notice = Some(format!("Unstaged {file}"));
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
    }

    /// Show the read-only unified patch for one source-control change.
    pub(crate) fn open_diff(&mut self, path: &str, staged: bool, cx: &mut Context<Self>) {
        let root = self.project().path.clone();
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_string();

        match git::file_diff(&root, path, staged) {
            Ok(patch) => match DiffFile::parse(&patch) {
                Ok(files) if !files.is_empty() => {
                    self.diff_state
                        .update(cx, |state, cx| state.set_files(files, cx));
                    self.diff_view = Some(DiffView {
                        path: path.to_string(),
                        name,
                        staged,
                        state: self.diff_state.clone(),
                        error: None,
                    });
                    cx.notify();
                }
                Ok(_) => self.set_notice("No changes to show", cx),
                Err(error) => self.set_notice(format!("Could not read the patch: {error}"), cx),
            },
            Err(error) => {
                self.diff_state
                    .update(cx, |state, cx| state.set_files(Vec::new(), cx));
                self.diff_view = Some(DiffView {
                    path: path.to_string(),
                    name,
                    staged,
                    state: self.diff_state.clone(),
                    error: Some(error),
                });
                cx.notify();
            }
        }
    }

    /// Jump from a trace span to the transcript entry that produced it.
    pub(crate) fn open_span_in_chat(&mut self, span_id: &str, cx: &mut Context<Self>) {
        self.view = MainView::Chat;
        self.settings_open = false;
        self.expanded_tool = span_id
            .strip_prefix("item-")
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| {
                matches!(
                    self.harness.transcript.get(*index),
                    Some(harness::ChatItem::Tool { .. })
                )
            });
        self.set_notice("Opened the trace in chat", cx);
    }

    pub(crate) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.diff_view.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
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

    /// Snapshot the settings that survive a restart.
    pub(crate) fn stored_settings(&self, cx: &App) -> StoredSettings {
        StoredSettings {
            default_harness: self.default_harness.clone(),
            custom_harness_command: self.custom_harness_command.clone(),
            permission_mode: self.permission_mode,
            auto_connect: self.auto_connect,
            restart_harness: self.restart_harness,
            restore_session: self.restore_session,
            word_wrap: self.word_wrap,
            code_font_size: self.code_font_size,
            allow_network: self.allow_network,
            reasoning_open: self.reasoning_open,
            dark: cx.theme().is_dark(),
            view: self.view,
            last_project_id: self
                .projects
                .get(self.selected_project)
                .map(|project| project.id.clone()),
            last_session_id: self
                .sessions
                .get(self.selected_session)
                .map(|session| session.id.clone()),
        }
    }

    pub(crate) fn persist_settings(&self, cx: &App) {
        if let Some(store) = &self.store {
            let _ = store.save_setting("app", &self.stored_settings(cx));
        }
    }

    /// Live context-window usage reported by the active ACP session.
    pub(crate) fn session_usage(&self) -> Option<ProviderUsage> {
        let usage = self.harness.usage?;
        let session = self.session();
        let harness = if session.harness.is_empty() {
            self.default_harness.clone()
        } else {
            session.harness.clone()
        };
        Some(ProviderUsage {
            provider: harness,
            plan: "ACP".into(),
            accent: ProjectTone::Teal,
            detail: format!(
                "{} of {} tokens · ${:.2} this session",
                format_tokens(usage.used),
                format_tokens(usage.size),
                session.cost_usd
            ),
            windows: vec![UsageWindow {
                label: "Context".into(),
                used_percent: usage.percent() * 100.0,
                resets_in: "current turn".into(),
                source: UsageSource::Live,
            }],
        })
    }

    pub(crate) fn open_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_dirty {
            self.notice = Some("Unsaved changes discarded".into());
        }
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

        let language = editor_language(path).to_string();
        let bytes = fs::metadata(path)
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0);
        let (contents, readonly) = match fs::read_to_string(path) {
            Ok(contents) if contents.contains('\0') => {
                ("Binary file — editing is disabled.".to_string(), true)
            }
            Ok(contents) => (contents, false),
            Err(error) => (format!("Unable to read this file: {error}"), true),
        };

        self.editor.update(cx, |state, cx| {
            state.set_value(contents, window, cx);
            state.set_highlighter(language.clone(), cx);
            state.set_soft_wrap(self.word_wrap, window, cx);
            state.focus(window, cx);
        });
        self.editor_readonly = readonly;
        self.editor_dirty = false;
        self.inspector = Some(Inspector {
            path: path.to_string(),
            name,
            directory,
            language,
            bytes,
            snippet: None,
        });
        cx.notify();
    }

    /// Apply editor preferences that live in the GPUI Kit input state.
    pub(crate) fn apply_editor_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wrap = self.word_wrap;
        self.editor
            .update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
    }

    /// Step the code font size and apply it to editors and diffs immediately.
    pub(crate) fn adjust_code_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.code_font_size = (self.code_font_size + delta).clamp(10.0, 20.0);
        kestrel_ui::theme::set_code_font_size(self.code_font_size, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Close the inspector, discarding any unsaved editor buffer.
    pub(crate) fn close_inspector(&mut self) {
        if self.editor_dirty {
            self.notice = Some("Unsaved changes discarded".into());
        }
        self.inspector = None;
        self.editor_dirty = false;
        self.editor_readonly = false;
    }

    pub(crate) fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(inspector) = self.inspector.clone() else {
            return;
        };
        if self.editor_readonly {
            self.notice = Some(format!("{} is read-only", inspector.name));
            cx.notify();
            return;
        }
        let value = self.editor.read(cx).value().to_string();
        match fs::write(&inspector.path, value) {
            Ok(()) => {
                self.editor_dirty = false;
                self.notice = Some(format!("Saved {}", inspector.name));
                self.refresh_git_status(cx);
            }
            Err(error) => {
                self.notice = Some(format!("Could not save {}: {error}", inspector.name));
            }
        }
        cx.notify();
    }

    pub(crate) fn open_search_hit(
        &mut self,
        path: &str,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file(path, window, cx);
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.snippet = Some((line, line));
        }
        self.editor.update(cx, |state, cx| {
            state.set_cursor_position(Position::new(line.saturating_sub(1), 0), window, cx);
        });
        cx.notify();
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

    fn diff_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(view) = self.diff_view.as_ref() else {
            return div().into_any_element();
        };
        let path = view.path.clone();
        let open_path = path.clone();
        let name = view.name.clone();
        let staged = view.staged;
        let error = view.error.clone();

        let body = match error {
            Some(error) => div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(7.))
                .child(icon(IconName::FileDiff, 24.0, p.muted_fg))
                .child(div().text_size(px(11.5)).text_color(p.fg).child(error))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child("This file cannot be previewed as a patch."),
                )
                .into_any_element(),
            None => div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(Diff::new(&view.state).line_number(true).soft_wrap(false))
                .into_any_element(),
        };

        div()
            .id("diff-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(p.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx)))
            .child(
                div()
                    .id("diff-dialog")
                    .w(px(880.))
                    .h(px(560.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(52.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(p.border)
                            .child(
                                div()
                                    .size(px(28.))
                                    .flex_shrink_0()
                                    .rounded(px(7.))
                                    .bg(if staged { p.green_soft } else { p.amber_soft })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(
                                        IconName::FileDiff,
                                        14.0,
                                        if staged { p.green } else { p.amber },
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .truncate()
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .font_family("Geist Mono")
                                            .text_color(p.muted_fg)
                                            .truncate()
                                            .child(path.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(19.))
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .bg(if staged { p.green_soft } else { p.amber_soft })
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(8.5))
                                            .font_family("Geist Mono")
                                            .text_color(if staged { p.green } else { p.amber })
                                            .child(if staged { "STAGED" } else { "WORKTREE" }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("diff-open-file")
                                    .h(px(26.))
                                    .px(px(9.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_diff(window, cx);
                                        this.open_file(&open_path, window, cx);
                                    }))
                                    .child(icon(IconName::ExternalLink, 10.5, p.muted_fg))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(p.fg)
                                            .child("Open file"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("diff-close")
                                    .size(px(26.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_diff(window, cx)
                                    }))
                                    .child(icon(IconName::X, 12.0, p.muted_fg)),
                            ),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn delete_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(pending) = self.pending_delete.clone() else {
            return div().into_any_element();
        };
        let (title, body, action) = match &pending {
            PendingDelete::Session { title, .. } => (
                "Delete session?".to_string(),
                format!(
                    "“{title}” and its transcript will be removed from this Mac. This cannot be undone."
                ),
                "Delete session",
            ),
            PendingDelete::Project { name, .. } => (
                "Remove project?".to_string(),
                format!(
                    "“{name}” will be removed from Kestrel. The folder stays on disk, but its sessions and transcripts will be deleted."
                ),
                "Remove project",
            ),
        };

        div()
            .id("delete-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(p.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx)))
            .child(
                div()
                    .id("delete-dialog")
                    .w(px(392.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .flex_shrink_0()
                                    .rounded(px(8.))
                                    .bg(p.red_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Trash, 14.0, p.red)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .line_height(relative(1.5))
                                            .text_color(p.muted_fg)
                                            .child(body),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(7.))
                            .child(
                                div()
                                    .id("delete-cancel")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx)))
                                    .child(
                                        div().text_size(px(10.5)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("delete-confirm")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .bg(p.red)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.88))
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx)))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(white())
                                            .child(action),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn inspector_panel(&self, inspector: &Inspector, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let dirty = self.editor_dirty;
        let readonly = self.editor_readonly;
        let snippet = inspector.snippet;

        div()
            .w(px(560.))
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
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.fg)
                                            .truncate()
                                            .child(inspector.name.clone()),
                                    )
                                    .when(dirty, |this| {
                                        this.child(div().size(px(5.)).rounded_full().bg(p.amber))
                                    }),
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
                            .id("inspector-save")
                            .h(px(24.))
                            .px(px(7.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .when(readonly, |this| this.opacity(0.5))
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_editor(cx);
                            }))
                            .child(icon(IconName::Save, 11.5, p.muted_fg))
                            .child(div().text_size(px(9.5)).text_color(p.fg).child("Save"))
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .font_family("Geist Mono")
                                    .text_color(p.muted_fg.opacity(0.7))
                                    .child("⌘S"),
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
                                this.close_inspector();
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
                            .child("Editor"),
                    )
                    .child(
                        div()
                            .h(px(16.))
                            .px(px(5.))
                            .rounded(px(3.))
                            .bg(p.muted)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(8.))
                                    .font_family("Geist Mono")
                                    .text_color(p.muted_fg)
                                    .child(inspector.language.clone()),
                            ),
                    )
                    .when_some(snippet, |this, (line, _)| {
                        this.child(
                            div()
                                .h(px(16.))
                                .px(px(5.))
                                .rounded(px(3.))
                                .bg(p.blue_soft)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(8.))
                                        .font_family("Geist Mono")
                                        .text_color(p.blue)
                                        .child(format!("line {line}")),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(9.))
                            .font_family("Geist Mono")
                            .text_color(p.muted_fg)
                            .child(if dirty {
                                "unsaved changes".to_string()
                            } else if readonly {
                                "read-only".to_string()
                            } else {
                                format_bytes(inspector.bytes)
                            }),
                    ),
            )
            .child(
                div().flex_1().min_h(px(0.)).overflow_hidden().child(
                    Editor::new(&self.editor)
                        .appearance(false)
                        .bordered(false)
                        .readonly(readonly)
                        .h_full(),
                ),
            )
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
        let confirm = self.pending_delete.as_ref().map(|_| self.delete_modal(cx));
        let diff = self.diff_view.as_ref().map(|_| self.diff_modal(cx));

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
            .children(confirm)
            .children(diff)
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

fn editor_language(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "toml" => "toml",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "css" => "css",
        "html" | "htm" => "html",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "sql" => "sql",
        "swift" => "swift",
        _ => "plaintext",
    }
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

/// Pick the session to show after one has been removed at `removed`.
pub(crate) fn session_index_after_delete(
    removed: usize,
    selected: usize,
    len_after: usize,
) -> usize {
    if len_after == 0 {
        0
    } else if removed < selected {
        selected - 1
    } else if removed == selected {
        removed.min(len_after - 1)
    } else {
        selected
    }
}

/// Whether a new file sits inside `root`, resolving symlinks on its parent.
pub(crate) fn path_inside(root: &Path, path: &Path) -> bool {
    let parent = path.parent().unwrap_or(root);
    match (parent.canonicalize(), root.canonicalize()) {
        (Ok(parent), Ok(root)) => parent.starts_with(root),
        _ => false,
    }
}

pub(crate) fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
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
    use super::{
        Command, CommandItem, PermissionMode, StoredSettings, command_matches, editor_language,
        format_bytes, path_inside, session_index_after_delete,
    };
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

    #[test]
    fn stored_settings_fill_defaults_for_missing_fields() {
        let settings: StoredSettings =
            serde_json::from_str(r#"{"default_harness":"Claude Code"}"#).unwrap();
        assert_eq!(settings.default_harness, "Claude Code");
        assert_eq!(settings.permission_mode, PermissionMode::Ask);
        assert!(settings.auto_connect);
        assert!(settings.restore_session);
        assert_eq!(settings.code_font_size, 13.0);
        assert_eq!(settings.view, MainView::Chat);
        assert_eq!(settings.last_project_id, None);
    }

    #[test]
    fn editor_language_uses_known_extensions() {
        assert_eq!(editor_language("src/main.rs"), "rust");
        assert_eq!(editor_language("README.md"), "markdown");
        assert_eq!(editor_language("ui/theme.tsx"), "tsx");
        assert_eq!(editor_language("Cargo.toml"), "toml");
        assert_eq!(editor_language("LICENSE"), "plaintext");
    }

    #[test]
    fn byte_sizes_are_human_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn deleting_a_session_keeps_a_sensible_selection() {
        // Removing the active session selects the next one, or the last one.
        assert_eq!(session_index_after_delete(1, 1, 3), 1);
        assert_eq!(session_index_after_delete(2, 2, 2), 1);
        // Removing an earlier session shifts the selection down.
        assert_eq!(session_index_after_delete(0, 2, 2), 1);
        // Removing a later session leaves the selection alone.
        assert_eq!(session_index_after_delete(3, 1, 3), 1);
        // A fresh session is created when the list becomes empty.
        assert_eq!(session_index_after_delete(0, 0, 1), 0);
    }

    #[test]
    fn new_files_must_stay_inside_the_project() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("kestrel-paths-{unique}"));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let outside = std::env::temp_dir().join(format!("kestrel-outside-{unique}.txt"));

        assert!(path_inside(&root, &root.join("src/main.rs")));
        assert!(path_inside(&root, &root.join("README.md")));
        assert!(!path_inside(&root, &outside));

        let _ = std::fs::remove_dir_all(&root);
    }
}
