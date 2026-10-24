use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use kestrel_domain::*;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::diff::{Diff, DiffFile, DiffState};
use kestrel_ui::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, Position, TextareaState,
};
use kestrel_ui::component::{ActiveTheme, ThemeMode};
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;
use serde::{Deserialize, Serialize};

use crate::{demo, harness, logging, projection, remote, usage, views, window_state};
use kestrel_workspace::tree::FileTree;
use kestrel_workspace::{git, search};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Harnesses,
    Permissions,
    Remote,
    Editor,
    Usage,
    Advanced,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 7] = [
        SettingsTab::General,
        SettingsTab::Harnesses,
        SettingsTab::Permissions,
        SettingsTab::Remote,
        SettingsTab::Editor,
        SettingsTab::Usage,
        SettingsTab::Advanced,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Harnesses => "Harnesses",
            SettingsTab::Permissions => "Permissions",
            SettingsTab::Remote => "Remote",
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
            SettingsTab::Remote => IconName::Cloud,
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
    pub autosave: bool,
    pub code_font_size: f32,
    pub allow_network: bool,
    pub reasoning_open: bool,
    pub dark: bool,
    pub view: MainView,
    pub default_view: MainView,
    pub last_project_id: Option<String>,
    pub last_session_id: Option<String>,
    pub remote_hosts: Vec<remote::RemoteHost>,
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
            autosave: false,
            code_font_size: 13.0,
            allow_network: false,
            reasoning_open: false,
            dark: true,
            view: MainView::Chat,
            default_view: MainView::Chat,
            last_project_id: None,
            last_session_id: None,
            remote_hosts: Vec::new(),
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
    History { count: usize },
    DiscardFile { path: String },
}

/// The read-only unified patch shown for one source-control change.
pub(crate) struct DiffView {
    pub path: String,
    pub name: String,
    pub staged: bool,
    pub state: Entity<DiffState>,
    pub error: Option<String>,
}

/// What to do after the user resolves an unsaved editor buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingEditor {
    Close,
    Open { path: String, line: Option<u32> },
}

/// A single executable entry in the command palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    View(MainView),
    Section(SidebarSection),
    Settings(SettingsTab),
    OpenFolder,
    AddRemoteProject,
    ToggleTheme,
    NewSession,
    NewFile,
    DeleteSession,
    ClearHistory,
    RemoveProject,
    RestartHarness,
    RefreshGit,
    RefreshExplorer,
    RefreshSessions,
    GitPull,
    GitPush,
    GitAmend,
    SwitchBranch(usize),
    CopyTrace,
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

#[derive(Clone, Debug)]
pub(crate) enum PendingRemoteSession {
    New,
    Load { session_id: String },
}

pub(crate) struct RemotePicker {
    pub host_id: String,
    pub host_name: String,
    pub projects: Vec<remote::RemoteProjectInfo>,
    pub loading: bool,
    pub error: Option<String>,
}

pub(crate) struct KestrelApp {
    pub view: MainView,
    pub default_view: MainView,
    pub section: SidebarSection,
    pub projects: Vec<Project>,
    pub selected_project: usize,
    pub sessions: Vec<SessionSummary>,
    pub all_sessions: Vec<SessionSummary>,
    pub fallback_session: SessionSummary,
    pub selected_session: usize,
    pub usages: Vec<ProviderUsage>,
    pub usage_loading: bool,
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
    pub session_menu_open: bool,
    pub pending_delete: Option<PendingDelete>,
    pub diff_view: Option<DiffView>,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub autosave: bool,
    pub code_font_size: f32,
    pub allow_network: bool,
    pub remote_hosts: Vec<remote::RemoteHost>,
    pub remote_tokens: HashMap<String, String>,
    pub remote_sessions_loading: bool,
    pub pending_remote_session: Option<PendingRemoteSession>,
    pub remote_url_input: Entity<InputState>,
    pub remote_code_input: Entity<InputState>,
    pub remote_name_input: Entity<InputState>,
    pub remote_loading: bool,
    pub remote_picker: Option<RemotePicker>,
    pub new_file_prompt: bool,
    pub new_file_input: Entity<InputState>,
    _new_file_subscription: Subscription,

    pub inspector: Option<Inspector>,
    pub editor: Entity<EditorState>,
    _editor_subscription: Subscription,
    pub diff_state: Entity<DiffState>,
    pub editor_dirty: bool,
    pub editor_stale: bool,
    pub editor_readonly: bool,
    pub editor_mtime: Option<u64>,
    pub autosave_generation: u64,
    last_window_geometry: Option<window_state::WindowGeometry>,
    _window_bounds_subscription: Option<Subscription>,
    pub pending_editor: Option<PendingEditor>,
    pub selected_span: Option<usize>,
    pub selected_node: Option<usize>,
    pub reasoning_open: bool,
    pub expanded_tool: Option<usize>,
    pub follow_running: bool,
    pub graph_zoom: f32,
    pub trace_zoom: f32,
    pub trace_filter: TraceFilter,

    pub file_tree: FileTree,
    pub search_hits: Vec<search::SearchHit>,
    pub search_query: String,
    pub search_loading: bool,
    pub notice: Option<String>,

    pub git: git::GitStatus,
    pub git_loading: bool,
    pub branches: Vec<String>,
    pub commit_input: Entity<InputState>,

    pub store: Option<kestrel_storage::Store>,
    pub harness: harness::HarnessState,
}

impl KestrelApp {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = workspace_root();
        let store = open_store();
        logging::info(&format!("workspace {root} · data {}", data_dir_display()));
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
        let file_tree = if projects[0].is_remote() {
            FileTree {
                root: projects[0].path.clone(),
                entries: Vec::new(),
                error: None,
            }
        } else {
            FileTree::load(projects[0].path.clone())
        };
        let search_hits = if projects[0].is_remote() {
            Vec::new()
        } else {
            demo::search_hits(&projects[0].path)
        };

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
                    this.search_loading = false;
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
        let remote_url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://host.tailnet.ts.net"));
        let remote_code_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Pairing code"));
        let remote_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Server name"));
        let new_file_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("src/new-file.rs"));
        let new_file_subscription = cx.subscribe_in(
            &new_file_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create_remote_file(window, cx);
                }
            },
        );
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
                    if this.autosave {
                        this.schedule_autosave(cx);
                    }
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
        let first_is_remote = projects[0].is_remote();
        let mut sessions: Vec<SessionSummary> = all_sessions
            .iter()
            .filter(|session| {
                session.project_id == first_project
                    && (!first_is_remote || session.origin.is_remote())
            })
            .cloned()
            .collect();
        if sessions.is_empty() && !first_is_remote {
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
            default_view: stored.default_view,
            section: SidebarSection::Files,
            projects,
            selected_project: 0,
            sessions,
            all_sessions,
            fallback_session,
            selected_session: 0,
            usages: usage::fallback(),
            usage_loading: false,
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
            session_menu_open: false,
            pending_delete: None,
            diff_view: None,
            auto_connect: stored.auto_connect,
            restart_harness: stored.restart_harness,
            restore_session: stored.restore_session,
            word_wrap: stored.word_wrap,
            autosave: stored.autosave,
            code_font_size: stored.code_font_size,
            allow_network: stored.allow_network,
            remote_hosts: stored.remote_hosts.clone(),
            remote_tokens: HashMap::new(),
            remote_sessions_loading: false,
            pending_remote_session: None,
            remote_url_input,
            remote_code_input,
            remote_name_input,
            remote_loading: false,
            remote_picker: None,
            new_file_prompt: false,
            new_file_input,
            _new_file_subscription: new_file_subscription,
            inspector: None,
            editor,
            _editor_subscription: editor_subscription,
            diff_state,
            editor_dirty: false,
            editor_stale: false,
            editor_readonly: false,
            editor_mtime: None,
            autosave_generation: 0,
            last_window_geometry: None,
            _window_bounds_subscription: None,
            pending_editor: None,
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
            search_loading: false,
            notice: None,
            git: git::GitStatus::default(),
            git_loading: false,
            branches: Vec::new(),
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

        if app.auto_connect && !app.harness.status.is_busy() && !app.remote_sessions_loading {
            app.connect_harness(cx);
        }
        if !restored_project {
            app.refresh_git_status(cx);
        }
        app.persist_settings(cx);
        app.refresh_usage(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if this
                    .update(cx, |this, cx| this.check_open_file(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        app._window_bounds_subscription =
            Some(cx.observe_window_bounds(window, |this, window, _| {
                let geometry = window_state::from_bounds(window.window_bounds());
                if this.last_window_geometry != Some(geometry) {
                    this.last_window_geometry = Some(geometry);
                    window_state::save(geometry);
                }
            }));

        if let Ok(prompt) = std::env::var("KESTREL_AUTOPROMPT")
            && !prompt.trim().is_empty()
        {
            app.composer
                .update(cx, |state, cx| state.set_value(prompt, window, cx));
            app.send_prompt(window, cx);
        }

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

    pub(crate) fn remote_host(&self, id: &str) -> Option<remote::RemoteHost> {
        self.remote_hosts.iter().find(|host| host.id == id).cloned()
    }

    pub(crate) fn remote_token(&mut self, host_id: &str) -> Result<String, String> {
        if let Some(token) = self.remote_tokens.get(host_id) {
            return Ok(token.clone());
        }
        let token = remote::load_token(host_id)?
            .ok_or_else(|| "Pair this device with the relay again".to_string())?;
        self.remote_tokens
            .insert(host_id.to_string(), token.clone());
        Ok(token)
    }

    pub(crate) fn backend(&mut self) -> Result<remote::WorkspaceBackend, String> {
        let project = self.project().clone();
        if let Some(remote_project) = project.remote.clone() {
            let host = self
                .remote_host(&remote_project.host_id)
                .ok_or_else(|| "Remote server is not configured".to_string())?;
            let token = self.remote_token(&host.id)?;
            Ok(remote::WorkspaceBackend::Remote(remote::RemoteWorkspace {
                host,
                project: remote_project,
                token,
            }))
        } else {
            Ok(remote::WorkspaceBackend::Local { root: project.path })
        }
    }

    fn import_remote_sessions(
        &mut self,
        rows: Vec<remote::RemoteSessionInfo>,
        cx: &mut Context<Self>,
    ) {
        let project_id = self.project().id.clone();
        let Some(host_id) = self
            .project()
            .remote
            .as_ref()
            .map(|project| project.host_id.clone())
        else {
            return;
        };
        let mut updates = Vec::new();
        for row in &rows {
            let summary = remote_session_summary(row, &project_id, &host_id);
            if let Some(existing) = self
                .all_sessions
                .iter_mut()
                .find(|session| session.id == summary.id)
            {
                existing.title = summary.title;
                existing.updated_at = summary.updated_at;
                existing.status = SessionStatus::Waiting;
                existing.origin = summary.origin;
                existing.acp_session_id = summary.acp_session_id;
                updates.push(existing.clone());
            } else {
                self.all_sessions.push(summary.clone());
                updates.push(summary);
            }
        }
        for session in updates {
            self.persist_session(&session);
        }

        let live: std::collections::HashSet<String> =
            rows.iter().map(|row| row.id.clone()).collect();
        self.all_sessions.retain(|session| {
            !(session.project_id == project_id
                && session.origin.is_remote()
                && !live.contains(&session.id))
        });
        self.sessions = self
            .all_sessions
            .iter()
            .filter(|session| session.project_id == project_id && session.origin.is_remote())
            .cloned()
            .collect();
        self.remote_sessions_loading = false;

        if self.sessions.is_empty() {
            self.selected_session = 0;
            self.harness.transcript.clear();
            self.pending_remote_session = Some(PendingRemoteSession::New);
            if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
            self.notice = Some("No remote sessions yet · starting a new one".into());
        } else {
            self.selected_session = 0;
            let session = self.sessions[0].clone();
            self.load_transcript(&session.id);
            let acp_id = session
                .acp_session_id
                .clone()
                .unwrap_or_else(|| session.id.clone());
            self.pending_remote_session = Some(PendingRemoteSession::Load { session_id: acp_id });
            if self.harness.status.is_ready() {
                let action = self.pending_remote_session.take().unwrap();
                self.start_remote_session(action, cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
            self.notice = Some(format!(
                "Loaded {} remote session{}",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            ));
        }
        cx.notify();
    }

    pub(crate) fn create_remote_session_summary(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        let project_id = self.project().id.clone();
        let host_id = self
            .project()
            .remote
            .as_ref()
            .map(|project| project.host_id.clone())
            .unwrap_or_default();

        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|session| session.id.starts_with("pending-"))
        {
            existing.id = session_id.to_string();
            existing.acp_session_id = Some(session_id.to_string());
            existing.status = SessionStatus::Waiting;
            let stored = existing.clone();
            if let Some(all) = self
                .all_sessions
                .iter_mut()
                .find(|session| session.id.starts_with("pending-"))
            {
                all.id = session_id.to_string();
                all.acp_session_id = Some(session_id.to_string());
                all.status = SessionStatus::Waiting;
            }
            self.persist_session(&stored);
            self.selected_session = self
                .sessions
                .iter()
                .position(|session| session.id == session_id)
                .unwrap_or(0);
            self.harness.transcript.clear();
            self.notice = Some("Remote session created".into());
            cx.notify();
            return;
        }

        if let Some(existing) = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
        {
            self.selected_session = existing;
            self.notice = Some("Remote session created".into());
            cx.notify();
            return;
        }

        let session = SessionSummary {
            id: session_id.to_string(),
            project_id,
            title: "New session".into(),
            status: SessionStatus::Waiting,
            harness: "Remote".into(),
            model: "Auto".into(),
            branch: None,
            updated_at: "now".into(),
            cost_usd: 0.0,
            duration_secs: 0,
            tokens: 0,
            summary: "Remote session on the relay.".into(),
            origin: SessionOrigin::Remote {
                host_id: host_id.clone(),
            },
            acp_session_id: Some(session_id.to_string()),
        };
        self.persist_session(&session);
        self.all_sessions.push(session.clone());
        self.sessions.push(session.clone());
        self.selected_session = self.sessions.len() - 1;
        self.harness.transcript.clear();
        self.notice = Some("Remote session created".into());
        cx.notify();
    }

    pub(crate) fn start_remote_session(
        &mut self,
        action: PendingRemoteSession,
        cx: &mut Context<Self>,
    ) {
        let Some(harness) = self.harness.harness.as_ref() else {
            self.pending_remote_session = Some(action);
            return;
        };
        let cwd = PathBuf::from(&self.project().path);
        match action {
            PendingRemoteSession::New => {
                harness.new_session(cwd);
            }
            PendingRemoteSession::Load { session_id } => {
                self.harness.transcript.clear();
                self.harness.started_tools.clear();
                self.harness.running = false;
                self.harness.usage = None;
                // `session/load` replays history as notifications before the
                // response arrives, so make the session current first.
                self.harness.session_id = Some(session_id.clone());
                harness.load_session(session_id, cwd);
            }
        }
        cx.notify();
    }

    /// Number of live turns in a project, derived from session state.
    pub(crate) fn running_sessions_for(&self, project_id: &str) -> usize {
        self.all_sessions
            .iter()
            .filter(|session| {
                session.project_id == project_id && session.status == SessionStatus::Running
            })
            .count()
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
                command: Command::ClearHistory,
                label: "Clear Conversation History…".into(),
                hint: None,
                icon: IconName::Trash,
                keywords: "remove delete all sessions transcripts reset".into(),
            },
            CommandItem {
                command: Command::RefreshGit,
                label: "Refresh Source Control".into(),
                hint: None,
                icon: IconName::RefreshCw,
                keywords: "git status changes reload".into(),
            },
            CommandItem {
                command: Command::RefreshExplorer,
                label: "Refresh Explorer".into(),
                hint: None,
                icon: IconName::RefreshCw,
                keywords: "files tree reload rescan".into(),
            },
            CommandItem {
                command: Command::RefreshSessions,
                label: "Refresh Remote Sessions".into(),
                hint: None,
                icon: IconName::Cloud,
                keywords: "relay phone sessions sync".into(),
            },
            CommandItem {
                command: Command::GitPull,
                label: "Git Pull (fast-forward)".into(),
                hint: None,
                icon: IconName::ArrowDownToLine,
                keywords: "source control fetch merge remote".into(),
            },
            CommandItem {
                command: Command::GitPush,
                label: "Git Push".into(),
                hint: None,
                icon: IconName::ArrowUpFromLine,
                keywords: "source control upload remote branch".into(),
            },
            CommandItem {
                command: Command::GitAmend,
                label: "Amend Last Commit".into(),
                hint: None,
                icon: IconName::GitCommitHorizontal,
                keywords: "git rewrite message staged".into(),
            },
            CommandItem {
                command: Command::OpenFolder,
                label: "Open Folder…".into(),
                hint: Some("⌘O".into()),
                icon: IconName::FolderOpen,
                keywords: "add project workspace directory".into(),
            },
            CommandItem {
                command: Command::AddRemoteProject,
                label: "Add Remote Project…".into(),
                hint: None,
                icon: IconName::Cloud,
                keywords: "relay server remote project tailscale".into(),
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
                command: Command::CopyTrace,
                label: "Copy Trace JSON".into(),
                hint: None,
                icon: IconName::Copy,
                keywords: "export spans waterfall diagnostics".into(),
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

        for (index, branch) in self.branches.iter().take(16).enumerate() {
            items.push(CommandItem {
                command: Command::SwitchBranch(index),
                label: format!("Switch to branch {branch}"),
                hint: Some("git".into()),
                icon: IconName::GitBranch,
                keywords: format!("checkout branch {branch}"),
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
        self.branches = self
            .backend()
            .ok()
            .and_then(|backend| backend.git_branches().ok())
            .unwrap_or_default();
        self.command_open = true;
        self.command_index = 0;
        self.settings_open = false;
        self.project_menu_open = false;
        self.session_menu_open = false;
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
        self.session_menu_open = false;
        match command {
            Command::View(view) => {
                self.view = view;
                self.settings_open = false;
                self.persist_settings(cx);
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
            Command::AddRemoteProject => self.open_remote_picker(cx),
            Command::ToggleTheme => {
                toggle_mode(window, cx);
                self.persist_settings(cx);
            }
            Command::NewSession => self.new_session(cx),
            Command::NewFile => self.new_file(window, cx),
            Command::DeleteSession => self.request_delete_session(self.selected_session, cx),
            Command::ClearHistory => self.request_clear_history(cx),
            Command::RemoveProject => self.request_delete_project(self.selected_project, cx),
            Command::RestartHarness => self.restart_harness(cx),
            Command::RefreshGit => self.refresh_git_status(cx),
            Command::RefreshExplorer => {
                self.refresh_file_tree(cx);
                self.notice = Some("Refreshed the explorer".into());
                cx.notify();
            }
            Command::RefreshSessions => {
                if self.project().is_remote() {
                    self.select_project(self.selected_project, cx);
                } else {
                    self.set_notice("Only remote projects have relay sessions", cx);
                }
            }
            Command::GitPull => self.git_pull(cx),
            Command::GitPush => self.git_push(cx),
            Command::GitAmend => self.amend_commit(cx),
            Command::SwitchBranch(index) => {
                if let Some(branch) = self.branches.get(index).cloned() {
                    self.switch_branch(&branch, cx);
                }
            }
            Command::CopyTrace => {
                let session = self.session().clone();
                let spans = if self.harness.transcript.is_empty() {
                    demo::trace_spans()
                } else {
                    projection::spans(&self.harness.transcript, &session)
                };
                let json = serde_json::to_string_pretty(&spans).unwrap_or_default();
                cx.write_to_clipboard(ClipboardItem::new_string(json));
                self.set_notice("Copied trace JSON", cx);
            }
            Command::SwitchProject(index) => self.select_project(index, cx),
            Command::CloseInspector => {
                self.request_close_inspector(cx);
                if self.inspector.is_none() {
                    self.notice = Some("Closed file inspector".into());
                }
            }
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Ask macOS for a path inside the project, then create or open it.
    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project().is_remote() {
            self.new_file_prompt = true;
            self.new_file_input.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
            cx.notify();
            return;
        }
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

    fn create_remote_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let relative = self.new_file_input.read(cx).value().trim().to_string();
        if relative.is_empty() {
            self.set_notice("Enter a file path", cx);
            return;
        }
        if Path::new(&relative).is_absolute()
            || relative
                .split(['/', '\\'])
                .any(|component| component == "..")
        {
            self.set_notice("Use a relative path inside the project", cx);
            return;
        }
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.create_file(&relative) {
            Ok(_) => {
                let root = self.project().path.clone();
                let full = format!(
                    "{}/{}",
                    root.trim_end_matches('/'),
                    relative.trim_start_matches('/')
                );
                self.new_file_prompt = false;
                self.new_file_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.refresh_file_tree(cx);
                self.open_file(&full, window, cx);
                self.notice = Some(format!("Created {relative}"));
            }
            Err(error) => self.set_notice(error, cx),
        }
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
        if let Some(index) = self.projects.iter().position(|project| {
            project.remote.is_none() && project.path.trim_end_matches('/') == path
        }) {
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
        let id = format!("project-{}", uuid::Uuid::now_v7());

        let project = Project {
            id,
            name: name.clone(),
            path,
            branch,
            tone,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: None,
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

    pub(crate) fn pair_remote(&mut self, cx: &mut Context<Self>) {
        if self.remote_loading {
            return;
        }
        let url = self
            .remote_url_input
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        let code = self.remote_code_input.read(cx).value().trim().to_string();
        let mut name = self.remote_name_input.read(cx).value().trim().to_string();
        if url.is_empty() || code.is_empty() {
            self.set_notice("Enter the relay URL and a pairing code", cx);
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.set_notice("The relay URL must start with http:// or https://", cx);
            return;
        }
        if name.is_empty() {
            name = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("Relay")
                .to_string();
        }
        let host_id = format!("host-{}", uuid::Uuid::now_v7());
        let host_id_for_task = host_id.clone();
        let url_for_task = url.clone();
        self.remote_loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let pair = remote::pair(&url_for_task, &code, "Kestrel on macOS")?;
                    remote::save_token(&host_id_for_task, &pair.token)?;
                    Ok::<_, String>((pair, host_id_for_task))
                })
                .await;
            this.update(cx, |this, cx| {
                this.remote_loading = false;
                match result {
                    Ok((pair, host_id)) => {
                        this.remote_tokens.insert(host_id.clone(), pair.token);
                        this.remote_hosts.push(remote::RemoteHost {
                            id: host_id,
                            name,
                            url,
                            user_name: pair.user.name,
                            device_id: pair.device_id,
                        });
                        this.persist_settings(cx);
                        this.notice = Some("Paired this Mac with the relay".into());
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn remove_remote_host(&mut self, id: &str, cx: &mut Context<Self>) {
        self.remote_hosts.retain(|host| host.id != id);
        self.remote_tokens.remove(id);
        remote::delete_token(id);
        self.persist_settings(cx);
        self.set_notice("Removed the remote server", cx);
    }

    pub(crate) fn open_remote_picker(&mut self, cx: &mut Context<Self>) {
        if self.remote_hosts.is_empty() {
            self.settings_open = true;
            self.settings_tab = SettingsTab::Remote;
            self.set_notice("Pair a relay before adding a remote project", cx);
            return;
        }
        let host_id = self
            .project()
            .remote
            .as_ref()
            .map(|project| project.host_id.clone())
            .or_else(|| self.remote_hosts.first().map(|host| host.id.clone()))
            .unwrap();
        self.open_remote_picker_for(&host_id, cx);
    }

    pub(crate) fn open_remote_picker_for(&mut self, host_id: &str, cx: &mut Context<Self>) {
        let Some(host) = self.remote_host(host_id) else {
            self.set_notice("Remote server is not configured", cx);
            return;
        };
        let token = match self.remote_token(&host.id) {
            Ok(token) => token,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        self.remote_picker = Some(RemotePicker {
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            projects: Vec::new(),
            loading: true,
            error: None,
        });
        let url = host.url.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { remote::projects(&url, &token) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(picker) = this.remote_picker.as_mut() {
                    picker.loading = false;
                    match result {
                        Ok(projects) => picker.projects = projects,
                        Err(error) => picker.error = Some(error),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn add_remote_project(
        &mut self,
        host_id: &str,
        info: &remote::RemoteProjectInfo,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.projects.iter().position(|project| {
            project
                .remote
                .as_ref()
                .is_some_and(|remote| remote.host_id == host_id && remote.project_id == info.id)
        }) {
            self.remote_picker = None;
            self.select_project(index, cx);
            return;
        }
        let tone = match self.projects.len() % 6 {
            0 => ProjectTone::Blue,
            1 => ProjectTone::Green,
            2 => ProjectTone::Violet,
            3 => ProjectTone::Amber,
            4 => ProjectTone::Rose,
            _ => ProjectTone::Teal,
        };
        let project = Project {
            id: format!("project-{}", uuid::Uuid::now_v7()),
            name: info.name.clone(),
            path: info.path.clone(),
            branch: None,
            tone,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: Some(RemoteProject {
                host_id: host_id.to_string(),
                project_id: info.id.clone(),
                path: info.path.clone(),
            }),
        };
        if let Some(store) = &self.store {
            let _ = store.upsert_project(&project);
        }
        self.projects.push(project);
        let index = self.projects.len() - 1;
        self.remote_picker = None;
        self.select_project(index, cx);
        self.notice = Some(format!("Added remote project {}", info.name));
        cx.notify();
    }

    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        let project_id = self.project().id.clone();
        if let Some(remote_project) = self.project().remote.clone() {
            self.interrupt_live_turn(cx);
            self.stash_transcript();
            let pending = SessionSummary {
                id: format!("pending-{}", uuid::Uuid::now_v7()),
                project_id,
                title: "New session".into(),
                status: SessionStatus::Waiting,
                harness: "Remote".into(),
                model: "Auto".into(),
                branch: None,
                updated_at: "now".into(),
                cost_usd: 0.0,
                duration_secs: 0,
                tokens: 0,
                summary: "Remote session on the relay.".into(),
                origin: SessionOrigin::Remote {
                    host_id: remote_project.host_id.clone(),
                },
                acp_session_id: None,
            };
            self.sessions.push(pending.clone());
            self.all_sessions.push(pending.clone());
            self.selected_session = self.sessions.len() - 1;
            self.harness.transcript.clear();
            self.harness.session_id = None;
            self.harness.running = false;
            self.pending_remote_session = Some(PendingRemoteSession::New);

            if self.harness.status.is_ready() {
                let action = self.pending_remote_session.take().unwrap();
                self.start_remote_session(action, cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            } else {
                self.notice = Some("Connect to the relay to start a session".into());
            }

            self.view = self.default_view;
            self.settings_open = false;
            self.session_menu_open = false;
            self.persist_settings(cx);
            cx.notify();
            return;
        }

        let branch = self.project().branch.clone();
        let session = blank_session(&project_id, &self.default_harness, branch);
        self.interrupt_live_turn(cx);
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

        self.view = self.default_view;
        self.settings_open = false;
        self.session_menu_open = false;
        self.notice = Some("Created a new session".into());
        self.persist_settings(cx);
        cx.notify();
    }

    /// Ask for confirmation before deleting a session.
    pub(crate) fn request_delete_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(index) else {
            return;
        };
        self.session_menu_open = false;
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

    /// Ask for confirmation before clearing all conversations.
    pub(crate) fn request_clear_history(&mut self, cx: &mut Context<Self>) {
        self.session_menu_open = false;
        self.pending_delete = Some(PendingDelete::History {
            count: self.all_sessions.len(),
        });
        cx.notify();
    }

    pub(crate) fn request_discard_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.pending_delete = Some(PendingDelete::DiscardFile {
            path: path.to_string(),
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
            Some(PendingDelete::History { .. }) => self.clear_history(cx),
            Some(PendingDelete::DiscardFile { path }) => self.discard_file(&path, cx),
            None => {}
        }
    }

    fn discard_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_discard(path) {
            Ok(()) => {
                self.notice = Some(format!("Discarded changes in {path}"));
                if self.inspector.as_ref().is_some_and(|i| i.path == path) {
                    self.editor_stale = true;
                }
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.all_sessions.is_empty() {
            self.set_notice("No conversation history to clear", cx);
            return;
        }
        if self.project().is_remote() {
            self.set_notice(
                "Remote history lives on the relay; delete sessions individually",
                cx,
            );
            return;
        }
        self.interrupt_live_turn(cx);
        if let Some(store) = &self.store {
            let _ = store.clear_sessions();
        }
        self.forget_all_transcripts();
        self.all_sessions.clear();
        self.sessions.clear();
        self.selected_span = None;
        self.selected_node = None;
        self.expanded_tool = None;

        let project_id = self.project().id.clone();
        let branch = self.project().branch.clone();
        let session = blank_session(&project_id, &self.default_harness, branch);
        self.persist_session(&session);
        self.all_sessions.push(session.clone());
        self.sessions.push(session.clone());
        self.selected_session = 0;
        self.harness.session_id = None;
        self.load_transcript(&session.id);

        if self.harness.status.is_ready() {
            let path = PathBuf::from(&self.project().path);
            if let Some(harness) = self.harness.harness.as_ref() {
                harness.new_session(path);
            }
        } else if self.auto_connect && !self.harness.status.is_busy() {
            self.connect_harness(cx);
        }

        self.settings_open = false;
        self.notice = Some("Cleared conversation history".into());
        self.persist_settings(cx);
        cx.notify();
    }

    fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.sessions.iter().position(|session| session.id == id) else {
            return;
        };
        let remote_origin = match &self.sessions[index].origin {
            SessionOrigin::Remote { host_id } => Some(host_id.clone()),
            SessionOrigin::Local => None,
        };
        if let Some(host_id) = remote_origin
            && let Some(host) = self.remote_host(&host_id)
            && let Ok(token) = self.remote_token(&host_id)
        {
            let url = host.url.clone();
            let session_id = id.to_string();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { remote::delete_session(&url, &token, &session_id) })
                    .await;
                this.update(cx, |this, cx| {
                    if let Err(error) = result {
                        this.set_notice(error, cx);
                    }
                })
                .ok();
            })
            .detach();
        }

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
        let remote_project = self.project().is_remote();
        if self.sessions.is_empty() {
            if remote_project {
                self.harness.transcript.clear();
                self.pending_remote_session = Some(PendingRemoteSession::New);
                if self.auto_connect && !self.harness.status.is_busy() {
                    self.connect_harness(cx);
                }
            } else {
                let project_id = self.project().id.clone();
                let branch = self.project().branch.clone();
                let session = blank_session(&project_id, &self.default_harness, branch);
                self.persist_session(&session);
                self.all_sessions.push(session.clone());
                self.sessions.push(session);
            }
        }
        self.selected_session =
            session_index_after_delete(index, self.selected_session, self.sessions.len());

        if active && !self.sessions.is_empty() {
            let session = self.session().clone();
            self.harness.session_id = None;
            self.load_transcript(&session.id);
            if session.origin.is_remote() {
                let acp_id = session
                    .acp_session_id
                    .clone()
                    .unwrap_or_else(|| session.id.clone());
                self.pending_remote_session =
                    Some(PendingRemoteSession::Load { session_id: acp_id });
                if self.harness.status.is_ready() {
                    let action = self.pending_remote_session.take().unwrap();
                    self.start_remote_session(action, cx);
                } else if self.auto_connect && !self.harness.status.is_busy() {
                    self.connect_harness(cx);
                }
            } else if self.harness.status.is_ready() {
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
            self.select_project(self.selected_project, cx);
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

        if self.pending_editor.is_some() {
            match keystroke.key.as_str() {
                "escape" => self.cancel_pending_editor(cx),
                "enter" => self.save_pending_editor(window, cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }

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
                    self.persist_settings(cx);
                    cx.notify();
                    true
                }
                "2" => {
                    self.view = MainView::Graph;
                    self.settings_open = false;
                    self.persist_settings(cx);
                    cx.notify();
                    true
                }
                "3" => {
                    self.view = MainView::Trace;
                    self.settings_open = false;
                    self.persist_settings(cx);
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
            } else if self.session_menu_open {
                self.session_menu_open = false;
                cx.stop_propagation();
                cx.notify();
            } else if self.diff_view.is_some() {
                self.close_diff(window, cx);
                cx.stop_propagation();
            } else if self.inspector.is_some() {
                self.request_close_inspector(cx);
                cx.stop_propagation();
            }
        }
    }

    pub(crate) fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            self.search_hits = if self.project().is_remote() {
                Vec::new()
            } else {
                demo::search_hits(&self.project().path)
            };
            self.search_loading = false;
            self.notice = Some("Showing recent matches. Type a query and press Enter.".into());
            cx.notify();
            return;
        }

        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        self.search_loading = true;
        self.notice = Some(format!("Searching for “{query}”…"));
        let expected = self.project().path.clone();
        cx.spawn(async move |this, cx| {
            let match_query = query.clone();
            let hits = cx
                .background_executor()
                .spawn(async move { backend.search(&query, 80) })
                .await;
            this.update(cx, |this, cx| {
                if this.search_query.trim() != match_query || this.project().path != expected {
                    return;
                }
                match hits {
                    Ok(hits) => {
                        let count = hits.len();
                        this.search_hits = hits;
                        this.notice = Some(if count == 0 {
                            format!("No matches for “{match_query}”")
                        } else {
                            format!("{count} matches for “{match_query}”")
                        });
                    }
                    Err(error) => {
                        this.search_hits = Vec::new();
                        this.notice = Some(error);
                    }
                }
                this.search_loading = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn toggle_project_trust(&mut self, cx: &mut Context<Self>) {
        let index = self.selected_project;
        let Some(project) = self.projects.get_mut(index) else {
            return;
        };
        project.trusted = !project.trusted;
        let trusted = project.trusted;
        let name = project.name.clone();
        let stored = project.clone();
        if let Some(store) = &self.store {
            let _ = store.upsert_project(&stored);
        }
        if trusted {
            self.notice = Some(format!("Trusted {name} · harness may start"));
            if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
        } else {
            self.interrupt_live_turn(cx);
            self.disconnect_harness(cx);
            self.notice = Some(format!("Untrusted {name} · harness stopped"));
        }
        cx.notify();
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.projects.len() {
            return;
        }
        self.interrupt_live_turn(cx);
        self.stash_transcript();
        self.selected_project = index;
        self.project_menu_open = false;
        self.session_menu_open = false;
        let project = self.projects[index].clone();
        let path = project.path.clone();
        self.file_tree = if project.is_remote() {
            FileTree {
                root: path.clone(),
                entries: Vec::new(),
                error: None,
            }
        } else {
            FileTree::load(path.clone())
        };
        self.search_hits = if !project.is_remote() && path == workspace_root() {
            demo::search_hits(&path)
        } else {
            Vec::new()
        };
        self.search_query.clear();
        self.search_loading = false;
        self.close_inspector();

        let project_id = project.id.clone();
        self.sessions = self
            .all_sessions
            .iter()
            .filter(|session| {
                session.project_id == project_id
                    && (!project.is_remote() || session.origin.is_remote())
            })
            .cloned()
            .collect();
        self.selected_session = 0;
        self.harness.session_id = None;
        self.harness.commands.clear();
        self.pending_remote_session = None;

        if let Some(remote_project) = project.remote.clone() {
            self.remote_sessions_loading = true;
            self.disconnect_harness(cx);
            if let Some(session) = self.sessions.first() {
                let session_id = session.id.clone();
                self.load_transcript(&session_id);
            } else {
                self.harness.transcript.clear();
            }
            let Some(host) = self.remote_host(&remote_project.host_id) else {
                self.remote_sessions_loading = false;
                self.set_notice("Remote server is not configured", cx);
                return;
            };
            let token = match self.remote_token(&host.id) {
                Ok(token) => token,
                Err(error) => {
                    self.remote_sessions_loading = false;
                    self.set_notice(error, cx);
                    return;
                }
            };
            let url = host.url.clone();
            let remote_id = remote_project.project_id.clone();
            let expected_project = project.id.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { remote::sessions(&url, &token, &remote_id) })
                    .await;
                this.update(cx, |this, cx| {
                    if this.project().id != expected_project {
                        return;
                    }
                    match result {
                        Ok(rows) => this.import_remote_sessions(rows, cx),
                        Err(error) => {
                            this.remote_sessions_loading = false;
                            this.set_notice(error, cx);
                        }
                    }
                })
                .ok();
            })
            .detach();
            self.refresh_file_tree(cx);
            self.refresh_git_status(cx);
            self.notice = Some(format!("Switched to {}", project.name));
            self.persist_settings(cx);
            cx.notify();
            return;
        }

        if self.sessions.is_empty() {
            let session = blank_session(&project_id, &self.default_harness, project.branch.clone());
            self.persist_session(&session);
            self.all_sessions.push(session.clone());
            self.sessions.push(session);
        }
        self.selected_session = 0;
        let first_session = self.sessions[0].id.clone();
        self.load_transcript(&first_session);
        self.harness.session_id = None;
        self.harness.commands.clear();

        self.notice = Some(format!("Switched to {}", project.name));
        if self.auto_connect {
            self.connect_harness(cx);
        } else {
            self.disconnect_harness(cx);
            self.notice = Some(format!("Switched to {}", project.name));
        }
        self.refresh_git_status(cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Load the explorer through the active local or remote backend.
    pub(crate) fn refresh_file_tree(&mut self, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_tree.error = Some(error);
                cx.notify();
                return;
            }
        };
        let expanded = self.file_tree.expanded_paths();
        let expected = self.project().path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.tree() })
                .await;
            this.update(cx, |this, cx| {
                if this.project().path != expected {
                    return;
                }
                match result {
                    Ok(mut tree) => {
                        tree.restore_expanded(&expanded);
                        this.file_tree = tree;
                    }
                    Err(error) => {
                        this.file_tree.error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Read branch and working-tree state off the UI thread.
    pub(crate) fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let expected = self.project().path.clone();
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.git.error = Some(error);
                self.git_loading = false;
                cx.notify();
                return;
            }
        };
        self.git_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async move { backend.git_status() })
                .await;
            this.update(cx, |this, cx| {
                if this.project().path == expected {
                    match status {
                        Ok(status) => {
                            if let Some(error) = &status.error {
                                logging::warn(&format!("git status: {error}"));
                            }
                            this.git = status;
                        }
                        Err(error) => {
                            this.git.error = Some(error);
                        }
                    }
                    this.git_loading = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn stage_all_changes(&mut self, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_stage_all() {
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
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_stage_file(file) {
            Ok(()) => {
                self.notice = Some(format!("Staged {file}"));
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
    }

    pub(crate) fn unstage_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_unstage_file(file) {
            Ok(()) => {
                self.notice = Some(format!("Unstaged {file}"));
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
        }
    }

    /// Show the read-only unified patch for one source-control change.
    pub(crate) fn open_diff(&mut self, path: &str, staged: bool, cx: &mut Context<Self>) {
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_string();
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };

        match backend.git_file_diff(path, staged) {
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

    /// Ask macOS for files to reference in the next prompt.
    pub(crate) fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_picked_paths(false, window, cx);
    }

    /// Pick one file and insert an @mention into the composer.
    pub(crate) fn mention_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_picked_paths(true, window, cx);
    }

    fn insert_picked_paths(&mut self, mention: bool, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.project().path.clone();
        let options = PathPromptOptions {
            files: true,
            directories: false,
            multiple: !mention,
            prompt: Some(if mention {
                "Mention a file".into()
            } else {
                "Attach files".into()
            }),
        };
        let receiver = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && !paths.is_empty()
            {
                this.update_in(cx, |this, window, cx| {
                    let remote = this.project().is_remote();
                    let backend = match this.backend() {
                        Ok(backend) => backend,
                        Err(error) => {
                            this.set_notice(error, cx);
                            return;
                        }
                    };
                    let mut inserted = Vec::new();
                    for path in &paths {
                        match backend.upload_file(path) {
                            Ok(uploaded) => {
                                inserted.push(if remote {
                                    uploaded
                                } else {
                                    display_path(&root, path)
                                });
                            }
                            Err(error) => {
                                this.set_notice(error, cx);
                                return;
                            }
                        }
                    }
                    let mut text = this.composer.read(cx).value().to_string();
                    if mention {
                        if !text.is_empty() && !text.ends_with(char::is_whitespace) {
                            text.push(' ');
                        }
                        for path in &inserted {
                            text.push('@');
                            text.push_str(path);
                            text.push(' ');
                        }
                    } else {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str("Attached files:\n");
                        for path in &inserted {
                            text.push_str("- ");
                            text.push_str(path);
                            text.push('\n');
                        }
                    }
                    this.composer.update(cx, |state, cx| {
                        state.set_value(text, window, cx);
                        state.focus(window, cx);
                    });
                    this.notice = Some(format!(
                        "{} {}",
                        if mention { "Mentioned" } else { "Attached" },
                        inserted.len()
                    ));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Start a slash command, using the commands advertised over ACP.
    pub(crate) fn open_composer_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.harness.commands.is_empty() {
            self.set_notice("This harness has not advertised any slash commands", cx);
            return;
        }
        let mut text = self.composer.read(cx).value().to_string();
        if !text.is_empty() && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        text.push('/');
        self.composer.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        self.notice = Some(format!("Commands: {}", self.harness.commands.join(" · ")));
        cx.notify();
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
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_commit(&message) {
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

    pub(crate) fn git_pull(&mut self, cx: &mut Context<Self>) {
        self.run_git_task(
            "Pulling…",
            "Pulled the latest changes",
            |backend| backend.git_pull(),
            cx,
        );
    }

    pub(crate) fn git_push(&mut self, cx: &mut Context<Self>) {
        self.run_git_task(
            "Pushing…",
            "Pushed the current branch",
            |backend| backend.git_push(),
            cx,
        );
    }

    fn run_git_task<F>(
        &mut self,
        label: impl Into<String>,
        success: impl Into<String>,
        operation: F,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(remote::WorkspaceBackend) -> Result<(), String> + Send + 'static,
    {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let label = label.into();
        let success = success.into();
        self.git_loading = true;
        self.notice = Some(label);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { operation(backend) })
                .await;
            this.update(cx, |this, cx| {
                this.git_loading = false;
                match result {
                    Ok(()) => {
                        this.notice = Some(success);
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn switch_branch(&mut self, branch: &str, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let branch = branch.to_string();
        self.git_loading = true;
        self.notice = Some(format!("Switching to {branch}…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let switch = branch.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend.git_switch(&switch) })
                .await;
            this.update(cx, |this, cx| {
                this.git_loading = false;
                match result {
                    Ok(()) => {
                        this.close_inspector();
                        this.refresh_file_tree(cx);
                        this.notice = Some(format!("Switched to {branch}"));
                        this.refresh_git_status(cx);
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn amend_commit(&mut self, cx: &mut Context<Self>) {
        let message = self.commit_input.read(cx).value().to_string();
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        match backend.git_amend(&message) {
            Ok(()) => {
                self.notice = Some("Amended the last commit".into());
                self.refresh_git_status(cx);
            }
            Err(error) => self.set_notice(error, cx),
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
            autosave: self.autosave,
            code_font_size: self.code_font_size,
            allow_network: self.allow_network,
            reasoning_open: self.reasoning_open,
            dark: cx.theme().is_dark(),
            view: self.view,
            default_view: self.default_view,
            last_project_id: self
                .projects
                .get(self.selected_project)
                .map(|project| project.id.clone()),
            last_session_id: self
                .sessions
                .get(self.selected_session)
                .map(|session| session.id.clone()),
            remote_hosts: self.remote_hosts.clone(),
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

    pub(crate) fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        if self.usage_loading {
            return;
        }
        self.usage_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let usages = cx
                .background_executor()
                .spawn(async move { usage::probe_all() })
                .await;
            this.update(cx, |this, cx| {
                for usage in &usages {
                    if usage.primary().is_none() {
                        logging::warn(&format!(
                            "usage {} unavailable: {}",
                            usage.provider, usage.detail
                        ));
                    }
                }
                logging::info(&format!("usage refreshed · {} providers", usages.len()));
                this.usages = usages;
                this.usage_loading = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn open_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_at(path, None, window, cx);
    }

    pub(crate) fn open_file_at(
        &mut self,
        path: &str,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.inspector.as_ref().is_some_and(|i| i.path == path) {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
            if let Some(line) = line {
                self.jump_to_line(line, window, cx);
            }
            cx.notify();
            return;
        }
        if self.editor_dirty {
            self.pending_editor = Some(PendingEditor::Open {
                path: path.to_string(),
                line,
            });
            cx.notify();
            return;
        }
        if self.project().is_remote() {
            self.open_file_remote(path, line, window, cx);
        } else {
            self.open_file_now(path, window, cx);
            if let Some(line) = line {
                self.jump_to_line(line, window, cx);
            }
        }
    }

    fn open_file_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        let metadata = fs::metadata(path).ok();
        let bytes = metadata
            .as_ref()
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0);
        self.editor_mtime = metadata.as_ref().map(metadata_mtime_ms);
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

    fn open_file_remote(
        &mut self,
        path: &str,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.project().path.clone();
        let path_buf = Path::new(path);
        let name = path_buf
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_string();
        let directory = path_buf
            .parent()
            .and_then(|parent| parent.strip_prefix(&root).ok())
            .map(|parent| {
                if parent.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    parent.to_string_lossy().to_string()
                }
            })
            .unwrap_or_else(|| ".".to_string());
        let language = editor_language(path).to_string();
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        self.editor_readonly = true;
        self.editor_dirty = false;
        self.editor_mtime = None;
        self.editor.update(cx, |state, cx| {
            state.set_value("Loading…", window, cx);
            state.set_highlighter(language.clone(), cx);
            state.set_soft_wrap(self.word_wrap, window, cx);
            state.focus(window, cx);
        });
        self.inspector = Some(Inspector {
            path: path.to_string(),
            name,
            directory,
            language: language.clone(),
            bytes: 0,
            snippet: None,
        });
        let file_path = path.to_string();
        let expected_path = file_path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.read_file(&file_path) })
                .await;
            this.update_in(cx, |this, window, cx| {
                if !this
                    .inspector
                    .as_ref()
                    .is_some_and(|inspector| inspector.path == expected_path)
                {
                    return;
                }
                match result {
                    Ok(file) => {
                        this.editor_readonly = file.readonly;
                        this.editor_mtime = Some(file.mtime_ms);
                        let wrap = this.word_wrap;
                        this.editor.update(cx, |state, cx| {
                            state.set_value(file.contents, window, cx);
                            state.set_highlighter(language.clone(), cx);
                            state.set_soft_wrap(wrap, window, cx);
                            state.focus(window, cx);
                        });
                        if let Some(inspector) = this.inspector.as_mut() {
                            inspector.bytes = file.bytes as usize;
                        }
                        if let Some(line) = line {
                            this.jump_to_line(line, window, cx);
                        }
                    }
                    Err(error) => {
                        this.editor_readonly = true;
                        this.editor.update(cx, |state, cx| {
                            state.set_value(
                                format!("Unable to read this file: {error}"),
                                window,
                                cx,
                            );
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn jump_to_line(&mut self, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.snippet = Some((line, line));
        }
        self.editor.update(cx, |state, cx| {
            state.set_cursor_position(Position::new(line.saturating_sub(1), 0), window, cx);
        });
    }

    /// Re-read the open file after the agent finishes a turn.
    ///
    /// Skipped while the buffer has unsaved edits so local work is never lost.
    fn reload_open_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_dirty {
            return;
        }
        let Some(inspector) = self.inspector.as_ref() else {
            return;
        };
        let path = inspector.path.clone();
        if self.project().is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(_) => return,
            };
            let language = editor_language(&path).to_string();
            let expected_path = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { backend.read_file(&path) })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    if !this
                        .inspector
                        .as_ref()
                        .is_some_and(|inspector| inspector.path == expected_path)
                    {
                        return;
                    }
                    if let Ok(file) = result {
                        this.editor_readonly = file.readonly;
                        this.editor_mtime = Some(file.mtime_ms);
                        let wrap = this.word_wrap;
                        this.editor.update(cx, |state, cx| {
                            state.set_value(file.contents, window, cx);
                            state.set_highlighter(language.clone(), cx);
                            state.set_soft_wrap(wrap, window, cx);
                        });
                        if let Some(inspector) = this.inspector.as_mut() {
                            inspector.bytes = file.bytes as usize;
                            inspector.language = language;
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            return;
        };
        if contents.contains('\0') {
            return;
        }
        self.editor_mtime = fs::metadata(&path)
            .ok()
            .map(|metadata| metadata_mtime_ms(&metadata));

        let language = editor_language(&path).to_string();
        let bytes = contents.len();
        let wrap = self.word_wrap;
        self.editor.update(cx, |state, cx| {
            state.set_value(contents, window, cx);
            state.set_highlighter(language.clone(), cx);
            state.set_soft_wrap(wrap, window, cx);
        });
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.bytes = bytes;
            inspector.language = language;
        }
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
    /// Prompt before closing when the buffer has unsaved edits.
    pub(crate) fn request_close_inspector(&mut self, cx: &mut Context<Self>) {
        if self.editor_dirty {
            self.pending_editor = Some(PendingEditor::Close);
            cx.notify();
        } else {
            self.close_inspector();
            cx.notify();
        }
    }

    pub(crate) fn cancel_pending_editor(&mut self, cx: &mut Context<Self>) {
        self.pending_editor = None;
        cx.notify();
    }

    pub(crate) fn save_pending_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_editor(cx);
        if !self.editor_dirty {
            self.finish_pending_editor(window, cx);
        }
    }

    pub(crate) fn discard_pending_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_dirty = false;
        self.finish_pending_editor(window, cx);
    }

    fn finish_pending_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_editor.take() else {
            return;
        };
        match pending {
            PendingEditor::Close => self.close_inspector(),
            PendingEditor::Open { path, line } => {
                self.open_file_at(&path, line, window, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn close_inspector(&mut self) {
        if self.editor_dirty {
            self.notice = Some("Unsaved changes discarded".into());
        }
        self.inspector = None;
        self.editor_dirty = false;
        self.editor_readonly = false;
        self.editor_mtime = None;
        self.pending_editor = None;
    }

    pub(crate) fn save_editor(&mut self, cx: &mut Context<Self>) {
        self.save_editor_internal(true, cx);
    }

    fn save_editor_internal(&mut self, announce: bool, cx: &mut Context<Self>) {
        let Some(inspector) = self.inspector.clone() else {
            return;
        };
        if self.editor_readonly {
            if announce {
                self.notice = Some(format!("{} is read-only", inspector.name));
            }
            cx.notify();
            return;
        }
        let value = self.editor.read(cx).value().to_string();
        if self.project().is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(error) => {
                    if announce {
                        self.notice = Some(error);
                    }
                    cx.notify();
                    return;
                }
            };
            match backend.write_file(&inspector.path, &value) {
                Ok(stat) => {
                    self.editor_dirty = false;
                    self.editor_mtime = Some(stat.mtime_ms);
                    if announce {
                        self.notice = Some(format!("Saved {}", inspector.name));
                    }
                    self.refresh_git_status(cx);
                }
                Err(error) => {
                    if announce {
                        self.notice = Some(format!("Could not save {}: {error}", inspector.name));
                    }
                }
            }
            cx.notify();
            return;
        }
        match fs::write(&inspector.path, value) {
            Ok(()) => {
                self.editor_dirty = false;
                self.editor_mtime = fs::metadata(&inspector.path)
                    .ok()
                    .map(|metadata| metadata_mtime_ms(&metadata));
                if announce {
                    self.notice = Some(format!("Saved {}", inspector.name));
                }
                self.refresh_git_status(cx);
            }
            Err(error) => {
                if announce {
                    self.notice = Some(format!("Could not save {}: {error}", inspector.name));
                }
            }
        }
        cx.notify();
    }

    /// Debounce a quiet autosave after the buffer changes.
    pub(crate) fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let generation = self.autosave_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(700))
                .await;
            this.update(cx, |this, cx| {
                if this.autosave_generation == generation && this.autosave && this.editor_dirty {
                    this.save_editor_internal(false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Detect external edits to the open file and reload a clean buffer.
    fn check_open_file(&mut self, cx: &mut Context<Self>) {
        if self.editor_dirty || self.editor_stale {
            return;
        }
        let Some(inspector) = self.inspector.as_ref() else {
            return;
        };
        let path = inspector.path.clone();
        if self.project().is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(_) => return,
            };
            let known = self.editor_mtime;
            let expected_path = path.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { backend.stat_file(&path) })
                    .await;
                this.update(cx, |this, cx| {
                    if !this
                        .inspector
                        .as_ref()
                        .is_some_and(|inspector| inspector.path == expected_path)
                    {
                        return;
                    }
                    if let Ok(stat) = result
                        && this.editor_mtime == known
                        && known != Some(stat.mtime_ms)
                    {
                        this.editor_mtime = Some(stat.mtime_ms);
                        this.editor_stale = true;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        let Ok(modified) = fs::metadata(&path).map(|metadata| metadata_mtime_ms(&metadata)) else {
            return;
        };
        if self.editor_mtime != Some(modified) {
            self.editor_mtime = Some(modified);
            self.editor_stale = true;
            cx.notify();
        }
    }

    pub(crate) fn open_search_hit(
        &mut self,
        path: &str,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_at(path, Some(line), window, cx);
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
                            .id("header-more")
                            .aria_label("Session menu")
                            .size(px(28.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(self.session_menu_open, |this| this.bg(p.accent))
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session_menu_open = !this.session_menu_open;
                                cx.notify();
                            }))
                            .child(icon(IconName::Ellipsis, 15.0, p.muted_fg)),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn takeover_current_session(&mut self, cx: &mut Context<Self>) {
        let session = self.session().clone();
        let Some(host_id) = session.origin.host_id().map(str::to_string) else {
            self.set_notice("Only remote sessions can be taken over", cx);
            return;
        };
        let Some(host) = self.remote_host(&host_id) else {
            self.set_notice("Remote server is not configured", cx);
            return;
        };
        let token = match self.remote_token(&host_id) {
            Ok(token) => token,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let session_id = session
            .acp_session_id
            .clone()
            .unwrap_or_else(|| session.id.clone());
        let session_for_load = session_id.clone();
        let expected_project = self.project().id.clone();
        self.session_menu_open = false;
        self.notice = Some("Taking over the remote session…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { remote::takeover_session(&host.url, &token, &session_id) })
                .await;
            this.update(cx, |this, cx| {
                if this.project().id != expected_project {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.pending_remote_session = Some(PendingRemoteSession::Load {
                            session_id: session_for_load,
                        });
                        this.disconnect_harness(cx);
                        this.connect_harness(cx);
                        this.notice = Some("Took over the remote session".into());
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn session_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);

        fn item(
            id: &'static str,
            icon_name: IconName,
            label: &'static str,
            hint: Option<&'static str>,
            danger: bool,
            cx: &mut Context<KestrelApp>,
            action: fn(&mut KestrelApp, &mut Window, &mut Context<KestrelApp>),
        ) -> AnyElement {
            let p = palette(cx);
            div()
                .id(id)
                .h(px(30.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(if danger { p.red_soft } else { p.hover }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    action(this, window, cx);
                }))
                .child(icon(
                    icon_name,
                    12.5,
                    if danger { p.red } else { p.muted_fg },
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .text_color(if danger { p.red } else { p.fg })
                        .child(label),
                )
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(hint)
                }))
                .into_any_element()
        }

        let takeover = self.session().origin.is_remote().then(|| {
            item(
                "session-menu-takeover",
                IconName::Cloud,
                "Take over session",
                None,
                false,
                cx,
                |this, _, cx| {
                    this.session_menu_open = false;
                    this.takeover_current_session(cx);
                },
            )
        });

        div()
            .id("session-menu-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_click(cx.listener(|this, _, _, cx| {
                this.session_menu_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("session-menu")
                    .absolute()
                    .top(px(100.))
                    .right(px(14.))
                    .w(px(190.))
                    .p(px(5.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(item(
                        "session-menu-new",
                        IconName::Plus,
                        "New session",
                        Some("⌘N"),
                        false,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            this.new_session(cx);
                        },
                    ))
                    .child(item(
                        "session-menu-copy",
                        IconName::Copy,
                        "Copy transcript",
                        Some("MD"),
                        false,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            if this.harness.transcript.is_empty() {
                                this.set_notice("Nothing to copy yet", cx);
                                return;
                            }
                            let markdown = harness::transcript_markdown(&this.harness.transcript);
                            cx.write_to_clipboard(ClipboardItem::new_string(markdown));
                            this.set_notice("Copied the transcript as Markdown", cx);
                        },
                    ))
                    .children(takeover)
                    .child(div().h(px(1.)).my(px(3.)).bg(p.border))
                    .child(item(
                        "session-menu-delete",
                        IconName::Trash,
                        "Delete session…",
                        None,
                        true,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            this.request_delete_session(this.selected_session, cx);
                        },
                    )),
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

    fn editor_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(pending) = self.pending_editor.as_ref() else {
            return div().into_any_element();
        };
        let name = self
            .inspector
            .as_ref()
            .map(|inspector| inspector.name.clone())
            .unwrap_or_else(|| "this file".into());
        let next = match pending {
            PendingEditor::Close => "closing the editor".to_string(),
            PendingEditor::Open { path, .. } => format!(
                "opening {}",
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path)
            ),
        };

        div()
            .id("editor-prompt-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(p.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_pending_editor(cx)))
            .child(
                div()
                    .id("editor-prompt")
                    .w(px(400.))
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
                                    .bg(p.amber_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::FileCode, 14.0, p.amber)),
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
                                            .child("Unsaved changes"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .line_height(relative(1.5))
                                            .text_color(p.muted_fg)
                                            .child(format!(
                                                "Save changes to “{name}” before {next}?"
                                            )),
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
                                    .id("editor-cancel")
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
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_pending_editor(cx);
                                    }))
                                    .child(
                                        div().text_size(px(10.5)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("editor-discard")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.red_soft))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_pending_editor(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(p.red)
                                            .child("Discard"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("editor-save")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.9))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_pending_editor(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child("Save"),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
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

    fn remote_picker_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(picker) = self.remote_picker.as_ref() else {
            return div().into_any_element();
        };
        let host_id = picker.host_id.clone();
        let host_name = picker.host_name.clone();
        let loading = picker.loading;
        let error = picker.error.clone();
        let projects = picker.projects.clone();

        let mut list = div().flex().flex_col().gap(px(2.));
        if loading {
            list = list.child(
                div()
                    .py(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .child(icon(IconName::LoaderCircle, 13.0, p.muted_fg))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .child("Loading projects…"),
                    ),
            );
        } else if let Some(error) = error {
            list = list.child(
                div()
                    .py(px(18.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(p.red_soft)
                    .text_size(px(10.))
                    .text_color(p.red)
                    .child(error),
            );
        } else if projects.is_empty() {
            list = list.child(
                div()
                    .py(px(24.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.fg)
                            .child("No projects on this relay"),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(p.muted_fg)
                            .child("Add one with `kestrel-relay project add`."),
                    ),
            );
        } else {
            for info in &projects {
                let info = info.clone();
                let info_for_click = info.clone();
                let host = host_id.clone();
                list = list.child(
                    div()
                        .id(format!("remote-project:{}", info.id))
                        .h(px(42.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_remote_project(&host, &info_for_click, cx);
                        }))
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(6.))
                                .bg(p.blue_soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(IconName::Folder, 12.0, p.blue)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.fg)
                                        .child(info.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(8.5))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg)
                                        .truncate()
                                        .child(info.path.clone()),
                                ),
                        )
                        .child(icon(IconName::ChevronRight, 11.0, p.muted_fg)),
                );
            }
        }

        div()
            .id("remote-picker-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(p.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.remote_picker = None;
                cx.notify();
            }))
            .child(
                div()
                    .id("remote-picker")
                    .w(px(440.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .bg(p.blue_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Cloud, 13.0, p.blue)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child("Add remote project"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.))
                                            .text_color(p.muted_fg)
                                            .child(host_name),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("remote-picker-close")
                                    .size(px(24.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.remote_picker = None;
                                        cx.notify();
                                    }))
                                    .child(icon(IconName::X, 11.0, p.muted_fg)),
                            ),
                    )
                    .child(
                        div()
                            .id("remote-picker-list")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    fn new_file_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        div()
            .id("new-file-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(p.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_file_prompt = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("new-file-modal")
                    .w(px(380.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child("New remote file"),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(p.muted_fg)
                            .child("Path relative to the project root."),
                    )
                    .child(
                        div()
                            .h(px(30.))
                            .px(px(7.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.new_file_input)
                                    .appearance(false)
                                    .bordered(false),
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
                                    .id("new-file-cancel")
                                    .h(px(27.))
                                    .px(px(9.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.new_file_prompt = false;
                                        cx.notify();
                                    }))
                                    .child(
                                        div().text_size(px(10.)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("new-file-create")
                                    .h(px(27.))
                                    .px(px(9.))
                                    .rounded(px(5.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_remote_file(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child("Create"),
                                    ),
                            ),
                    ),
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
            PendingDelete::History { count } => (
                "Clear conversation history?".to_string(),
                format!(
                    "{count} sessions and their transcripts will be deleted from this Mac. Projects and settings stay. This cannot be undone."
                ),
                "Clear history",
            ),
            PendingDelete::DiscardFile { path } => (
                "Discard file changes?".to_string(),
                format!(
                    "“{path}” will be restored to the last committed version. This cannot be undone."
                ),
                "Discard changes",
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
                            .aria_label("Close editor")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.request_close_inspector(cx);
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
        let geometry = window_state::from_bounds(window.window_bounds());
        if self.last_window_geometry != Some(geometry) {
            self.last_window_geometry = Some(geometry);
            window_state::save(geometry);
        }
        if self.editor_stale {
            self.editor_stale = false;
            self.reload_open_editor(window, cx);
        }
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
        let remote_picker = self
            .remote_picker
            .as_ref()
            .map(|_| self.remote_picker_modal(cx));
        let new_file_modal = self.new_file_prompt.then(|| self.new_file_modal(cx));
        let session_menu = self.session_menu_open.then(|| self.session_menu(cx));
        let editor_prompt = self.pending_editor.as_ref().map(|_| self.editor_prompt(cx));

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
            .children(session_menu)
            .children(settings)
            .children(palette)
            .children(confirm)
            .children(diff)
            .children(remote_picker)
            .children(new_file_modal)
            .children(editor_prompt)
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

pub(crate) fn data_dir() -> Option<PathBuf> {
    std::env::var_os("KESTREL_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Application Support/Kestrel"))
        })
}

pub(crate) fn data_dir_display() -> String {
    let Some(path) = data_dir() else {
        return "unavailable".into();
    };
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    path.to_string_lossy().to_string()
}

/// Approximate on-disk size of the SQLite database and its WAL files.
pub(crate) fn storage_size() -> u64 {
    let Some(directory) = data_dir() else {
        return 0;
    };
    ["kestrel.db", "kestrel.db-wal", "kestrel.db-shm"]
        .iter()
        .filter_map(|name| std::fs::metadata(directory.join(name)).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn open_store() -> Option<kestrel_storage::Store> {
    let base = data_dir()?;
    match kestrel_storage::Store::open(base.join("kestrel.db")) {
        Ok(store) => Some(store),
        Err(error) => {
            logging::error(&format!("could not open database: {error}"));
            eprintln!("[storage] could not open database: {error}");
            None
        }
    }
}

fn remote_session_summary(
    row: &remote::RemoteSessionInfo,
    project_id: &str,
    host_id: &str,
) -> SessionSummary {
    SessionSummary {
        id: row.id.clone(),
        project_id: project_id.to_string(),
        title: row.title.clone(),
        status: SessionStatus::Waiting,
        harness: "Remote".into(),
        model: "Auto".into(),
        branch: None,
        updated_at: relative_epoch(row.updated_at),
        cost_usd: 0.0,
        duration_secs: 0,
        tokens: 0,
        summary: "Remote session on the relay.".into(),
        origin: SessionOrigin::Remote {
            host_id: host_id.to_string(),
        },
        acp_session_id: Some(row.id.clone()),
    }
}

fn relative_epoch(epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let delta = (now - epoch).max(0);
    if delta < 60 {
        "now".into()
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

fn metadata_mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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
        origin: SessionOrigin::Local,
        acp_session_id: None,
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

/// Prefer a project-relative path in prompts; keep absolute paths outside it.
pub(crate) fn display_path(root: &str, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
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
        Command, CommandItem, PermissionMode, StoredSettings, command_matches, display_path,
        editor_language, format_bytes, path_inside, session_index_after_delete,
    };
    use kestrel_domain::MainView;
    use kestrel_ui::assets::IconName;
    use std::path::Path;

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
        assert!(!settings.autosave);
        assert_eq!(settings.view, MainView::Chat);
        assert_eq!(settings.default_view, MainView::Chat);
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
    fn prompts_use_relative_paths_inside_the_project() {
        assert_eq!(
            display_path("/work/kestrel", Path::new("/work/kestrel/src/main.rs")),
            "src/main.rs"
        );
        assert_eq!(
            display_path("/work/kestrel", Path::new("/tmp/notes.md")),
            "/tmp/notes.md"
        );
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
