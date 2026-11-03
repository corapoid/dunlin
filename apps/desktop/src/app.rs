use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dunlin_domain::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::diff::{Diff, DiffFile, DiffState};
use dunlin_ui::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, Position, TextareaState,
};
use dunlin_ui::component::{ActiveTheme, Icon, ThemeMode, tooltip::Tooltip};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use serde::{Deserialize, Serialize};

use crate::{harness, logging, projection, remote, views, watch, window_state};
use dunlin_workspace::tree::FileTree;
use dunlin_workspace::{git, search};

actions!(dunlin, [Quit]);

mod helpers;
mod overlays;
pub(crate) use helpers::*;

fn is_composer_submit(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { shift: false, .. })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Design,
    Chat,
    Harnesses,
    Skills,
    Permissions,
    Remote,
    Editor,
    Usage,
    Advanced,
}

impl SettingsTab {
    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Design => "Design",
            SettingsTab::Chat => "Chat",
            SettingsTab::Harnesses => "Harnesses",
            SettingsTab::Skills => "Skills",
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
            SettingsTab::Design => IconName::SlidersHorizontal,
            SettingsTab::Chat => IconName::MessageSquare,
            SettingsTab::Harnesses => IconName::Bot,
            SettingsTab::Skills => IconName::BookOpen,
            SettingsTab::Permissions => IconName::ShieldCheck,
            SettingsTab::Remote => IconName::Cloud,
            SettingsTab::Editor => IconName::Code,
            SettingsTab::Usage => IconName::ChartNoAxesColumn,
            SettingsTab::Advanced => IconName::SlidersHorizontal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum UiDensity {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum TerminalDock {
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum BottomPanel {
    #[default]
    Hidden,
    Terminal,
    Run,
}

/// Everything Dunlin remembers between launches. Stored as one JSON value in
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
    pub editor_minimap: bool,
    pub autosave: bool,
    pub code_font_size: f32,
    pub ui_font_family: String,
    pub code_font_family: String,
    pub reasoning_open: bool,
    pub final_answer_only: bool,
    pub stream_responses: bool,
    pub chat_timestamps: bool,
    pub chat_duration: bool,
    pub chat_logo: bool,
    pub dark: bool,
    pub view: MainView,
    pub default_view: MainView,
    pub section: SidebarSection,
    pub sidebar_collapsed: bool,
    pub notifications_enabled: bool,
    pub show_shortcut_hints: bool,
    pub last_project_id: Option<String>,
    pub last_session_id: Option<String>,
    pub remote_hosts: Vec<remote::RemoteHost>,
    pub collapsed_project_groups: Vec<String>,
    pub project_view: crate::project_tree::View,
    pub project_sort: crate::project_tree::Sort,
    pub sidebar_width: f32,
    pub show_project_branches: bool,
    pub opencode_provider: Option<String>,
    pub recent_models: Vec<crate::providers::RecentModel>,
    pub group_contexts: HashMap<String, crate::project_groups::GroupContext>,
    pub account_usage_collapsed: bool,
    pub dashboard_settings: crate::dashboard::Settings,
    pub skills: Vec<crate::skills::Skill>,
    pub last_ai_target: Option<crate::ai_commit::LastTarget>,
    pub billing_budget_usd: Option<f32>,
    pub billing_alerts: bool,
    pub onboarding_step: u8,
    pub density: UiDensity,
    pub reduce_motion: bool,
    #[serde(default)]
    pub run_history: Vec<crate::run::RunRecord>,
    #[serde(default)]
    pub bottom_panel: BottomPanel,
    #[serde(default = "default_bottom_panel_height")]
    pub bottom_panel_height: f32,
    #[serde(default = "default_bottom_panel_width")]
    pub bottom_panel_width: f32,
    #[serde(default)]
    pub terminal_dock: TerminalDock,
}

pub(crate) fn default_bottom_panel_height() -> f32 {
    300.
}

pub(crate) fn default_bottom_panel_width() -> f32 {
    420.
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
            editor_minimap: true,
            autosave: false,
            notifications_enabled: true,
            show_shortcut_hints: true,
            code_font_size: 13.0,
            ui_font_family: "Geist".into(),
            code_font_family: "Geist Mono".into(),
            reasoning_open: false,
            final_answer_only: false,
            stream_responses: true,
            chat_timestamps: false,
            chat_duration: false,
            chat_logo: true,
            dark: true,
            view: MainView::Chat,
            default_view: MainView::Chat,
            section: SidebarSection::Files,
            sidebar_collapsed: false,
            last_project_id: None,
            last_session_id: None,
            remote_hosts: Vec::new(),
            collapsed_project_groups: Vec::new(),
            project_view: Default::default(),
            project_sort: Default::default(),
            sidebar_width: crate::sidebar_layout::DEFAULT_WIDTH,
            show_project_branches: true,
            opencode_provider: Some(crate::providers::GO.into()),
            recent_models: Vec::new(),
            group_contexts: HashMap::new(),
            account_usage_collapsed: false,
            dashboard_settings: Default::default(),
            skills: crate::skills::defaults(),
            last_ai_target: None,
            billing_budget_usd: None,
            billing_alerts: true,
            onboarding_step: 0,
            density: UiDensity::default(),
            reduce_motion: false,
            run_history: Vec::new(),
            bottom_panel: BottomPanel::Hidden,
            bottom_panel_height: default_bottom_panel_height(),
            bottom_panel_width: default_bottom_panel_width(),
            terminal_dock: TerminalDock::Bottom,
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
    pub request: uuid::Uuid,
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
    SwitchProject { id: String },
    RemoveProject { id: String },
}

/// A save was rejected because the file changed on disk or on the relay.
#[derive(Clone)]
pub(crate) struct EditorConflict {
    pub path: String,
    pub message: String,
}

/// The create/rename dialog for a project group. `assign_project` is set when
/// the dialog was opened from a project's "Move to group" picker.
pub(crate) struct GroupPrompt {
    pub id: Option<String>,
    pub assign_project: Option<usize>,
    pub parent_id: Option<String>,
}

/// A short-lived action shown in the corner. The first use is undoing a group
/// deletion without reviving a whole session/project history.
pub(crate) struct Toast {
    pub id: uuid::Uuid,
    pub message: String,
    pub action_label: String,
    pub action: ToastAction,
}

impl Toast {
    pub(crate) fn is_expiration_target(&self, id: uuid::Uuid) -> bool {
        self.id == id && matches!(self.action, ToastAction::Info)
    }
}

pub(crate) enum ToastAction {
    Info,
    RestoreGroup {
        group: Box<ProjectGroup>,
        project_ids: Vec<String>,
        child_ids: Vec<String>,
    },
}
impl ToastAction {
    pub(crate) fn member_moved(&mut self, id: &str, is_group: bool) {
        if let Self::RestoreGroup {
            project_ids,
            child_ids,
            ..
        } = self
        {
            if is_group {
                child_ids.retain(|candidate| candidate != id);
            } else {
                project_ids.retain(|candidate| candidate != id);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ContextTarget {
    Project(usize),
    Session(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContextMenuState {
    pub target: ContextTarget,
    pub position: Point<Pixels>,
}

/// A single executable entry in the command palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    View(MainView),
    Section(SidebarSection),
    Settings(SettingsTab),
    OpenFolder,
    AddRemoteProject,
    NewProjectGroup,
    ToggleSidebar,
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
    ChooseBranch,
    ToggleTerminal,
    ToggleRun,
    Impact,
    Compare,
    Budgets,
    Checks,
    Context,
    Review,
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

pub(crate) struct DunlinApp {
    pub view: MainView,
    pub view_epoch: u64,
    pub default_view: MainView,
    pub section: SidebarSection,
    pub projects: Vec<Project>,
    pub local_import_running: bool,
    pub session_list: crate::session_list::State,
    pub session_import: crate::session_import::State,
    pub design_section: crate::views::settings::DesignSection,
    pub fallback_project: Project,
    pub project_groups: Vec<ProjectGroup>,
    pub collapsed_project_groups: Vec<String>,
    pub project_view: crate::project_tree::View,
    pub project_sort: crate::project_tree::Sort,
    pub sidebar_width: f32,
    pub show_project_branches: bool,
    pub sidebar_resize: Option<(f32, f32)>,
    pub project_group_editor: Option<usize>,
    pub group_contexts: HashMap<String, crate::project_groups::GroupContext>,
    pub pending_group_context: Option<crate::project_groups::GroupContext>,
    pub repo_summaries: crate::project_groups::RepoSummaries,
    pub branch_ui: crate::branch_picker::State,
    pub git_history: crate::git_history::State,
    pub task_budgets: crate::task_budgets::State,
    pub comparisons: crate::comparisons::State,
    pub overlay_window: crate::overlay_window::State,
    pub project_icons: crate::project_icons::State,
    pub parked_runtimes: HashMap<String, harness::ParkedRuntime>,
    pub project_session_selections: HashMap<String, String>,
    navigation_generation: u64,
    pub mcp_operations: std::collections::HashSet<String>,
    pub dashboard: crate::dashboard::State,
    pub skills: Vec<crate::skills::Skill>,
    pub skills_ui: crate::skills::Ui,
    pub last_ai_target: Option<crate::ai_commit::LastTarget>,
    pub ai_commit: crate::ai_commit::State,
    pub billing: Option<dunlin_usage::billing::Summary>,
    pub billing_loading: bool,
    pub billing_error: Option<String>,
    pub billing_checked: Option<std::time::Instant>,
    pub billing_budget_usd: Option<f32>,
    pub billing_alerts: bool,
    pub onboarding_step: u8,
    pub density: UiDensity,
    pub reduce_motion: bool,
    pub terminal: crate::terminal::State,
    pub run: crate::run::State,
    pub bottom_panel: BottomPanel,
    pub bottom_panel_height: f32,
    pub bottom_panel_width: f32,
    pub terminal_dock: TerminalDock,
    pub terminal_resize: Option<(f32, f32)>,
    pub terminal_drag: bool,
    sidebar_motion: crate::effects::SidebarMotion,
    pub(crate) overlay_backdrop: crate::effects::OverlayBackdrop,
    pub dashboard_settings: crate::dashboard::Settings,
    pub account_usage_collapsed: bool,
    pub group_prompt: Option<GroupPrompt>,
    pub toast: Option<Toast>,
    pub context_menu: Option<ContextMenuState>,
    pub project_settings_target: Option<usize>,
    pub project_runtime_draft: Option<crate::project_runtime_settings::Draft>,
    pub group_settings_draft: Option<crate::group_settings::Draft>,
    pub group_input: Entity<InputState>,
    _group_subscription: Subscription,
    pub selected_project: usize,
    pub sessions: Vec<SessionSummary>,
    pub all_sessions: Vec<SessionSummary>,
    pub fallback_session: SessionSummary,
    pub selected_session: usize,
    pub restore_session_id: Option<String>,
    pub usages: Vec<ProviderUsage>,
    pub usage_cache: crate::usage::Cache,
    pub remote_usages: HashMap<String, Vec<ProviderUsage>>,
    pub usage_loading: bool,
    pub usage_scope: Option<crate::usage::Scope>,
    pub usage_request: Option<crate::usage::Scope>,
    usage_generation: u64,
    pub composer: Entity<TextareaState>,
    _composer_subscription: Subscription,
    pub queue_open: bool,
    pub chat_menu_open: bool,
    pub search_input: Entity<InputState>,
    _search_subscription: Subscription,
    pub activity_search: Entity<InputState>,
    pub activity_query: String,
    _activity_search_subscription: Subscription,
    pub terminal_search: Entity<InputState>,
    pub terminal_query: String,
    _terminal_search_subscription: Subscription,

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
    pub sidebar_collapsed: bool,
    pub pending_delete: Option<PendingDelete>,
    pub diff_view: Option<DiffView>,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub editor_minimap: bool,
    pub autosave: bool,
    pub notifications_enabled: bool,
    pub show_shortcut_hints: bool,
    pub code_font_size: f32,
    pub ui_font_family: String,
    pub code_font_family: String,
    pub available_fonts: Vec<String>,
    pub remote_hosts: Vec<remote::RemoteHost>,
    pub remote_tokens: HashMap<String, String>,
    pub remote_sessions_loading: bool,
    pub pending_remote_session: Option<PendingRemoteSession>,
    pub pending_model: Option<String>,
    pub remote_url_input: Entity<InputState>,
    pub remote_code_input: Entity<InputState>,
    pub remote_name_input: Entity<InputState>,
    pub remote_loading: bool,
    pub remote_picker: Option<RemotePicker>,
    pub new_file_prompt: bool,
    pub explorer: crate::explorer::State,
    pub new_file_input: Entity<InputState>,
    _new_file_subscription: Subscription,

    pub inspector: Option<Inspector>,
    pub workbench: crate::workbench::State,
    pub editor: Entity<EditorState>,
    pub editor_preview: Entity<EditorState>,
    _editor_subscription: Subscription,
    pub diff_state: Entity<DiffState>,
    pub editor_dirty: bool,
    pub editor_stale: bool,
    pub editor_readonly: bool,
    pub editor_mtime: Option<u64>,
    pub autosave_generation: u64,
    pub editor_conflict: Option<EditorConflict>,
    pub app_active: bool,
    last_dock_badge: Option<String>,
    last_window_geometry: Option<window_state::WindowGeometry>,
    _activation_subscription: Option<Subscription>,
    _window_bounds_subscription: Option<Subscription>,
    pub pending_editor: Option<PendingEditor>,
    pub selected_span: Option<String>,
    pub selected_node: Option<String>,
    pub activity: crate::activity::State,
    pub reasoning_open: bool,
    pub final_answer_only: bool,
    pub stream_responses: bool,
    pub chat_timestamps: bool,
    pub chat_duration: bool,
    pub chat_logo: bool,
    pub expanded_tool: Option<usize>,
    pub chat_scroll: crate::chat_scroll::ChatScroll,
    pub trace_filter: TraceFilter,

    pub file_tree: FileTree,
    file_watcher: Option<watch::ProjectWatcher>,
    watcher_root: Option<String>,
    pub search_hits: Vec<search::SearchHit>,
    pub search_query: String,
    pub search_loading: bool,
    pub notice: Option<String>,

    pub git: git::GitStatus,
    pub git_loading: bool,
    pub branches: Vec<String>,
    pub commit_input: Entity<InputState>,

    pub store: Option<dunlin_storage::Store>,
    pub storage_error: RefCell<Option<String>>,
    pub harness: harness::HarnessState,
    pub harness_discovery: Vec<crate::harness_discovery::Detected>,
    pub provider_catalog: crate::providers::Catalog,
    pub mcp: crate::mcp::Catalog,
    pub provider_auth: Option<crate::provider_auth::AuthDialog>,
    pub opencode_provider: Option<String>,
    pub recent_models: Vec<crate::providers::RecentModel>,
}

impl DunlinApp {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut store, mut storage_error) = match open_store() {
            Ok(store) => (Some(store), None),
            Err(error) => (None, Some(error)),
        };
        logging::info(&format!("data {}", data_dir_display()));
        // Preferences are separate from projects/sessions: a stale enum or a
        // corrupt preference must not make all local history unreadable. The
        // defaults are used and the store stays writable so the next save
        // repairs the preference row; project/history read failures still fail
        // closed below.
        let stored = match store
            .as_ref()
            .map(|store| store.load_setting::<StoredSettings>("app"))
        {
            Some(Ok(Some(stored))) => stored,
            Some(Ok(None)) | None => StoredSettings::default(),
            Some(Err(error)) => {
                let message = format!(
                    "Some saved preferences could not be read: {error}. Defaults were used; projects and history were not discarded."
                );
                logging::error(&message);
                storage_error = Some(message);
                StoredSettings::default()
            }
        };
        let snapshot = store.as_ref().and_then(|store| {
            match (
                store.project_groups(),
                store.projects(),
                store.sessions(None),
            ) {
                (Ok(groups), Ok(projects), Ok(sessions)) => Some((groups, projects, sessions)),
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    let message = format!(
                        "Could not load local data: {error}. Your saved history may be unavailable."
                    );
                    logging::error(&message);
                    storage_error = Some(message);
                    None
                }
            }
        });
        if snapshot.is_none() {
            // Do not overwrite history with an empty installation after a
            // failed read.
            store = None;
        }
        let (project_groups, projects, mut all_sessions) = snapshot.unwrap_or_default();
        let fallback_project = Project {
            id: "none".into(),
            name: "Welcome".into(),
            path: String::new(),
            branch: None,
            tone: ProjectTone::Blue,
            pinned: false,
            trusted: false,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        };
        // A live turn cannot survive a restart, so clear statuses that imply one.
        for session in &mut all_sessions {
            // Older versions saved unused drafts as Waiting. Only repair them
            // after a successful history read, never from a failed read.
            let empty_history = is_unused_draft(session)
                && store.as_ref().is_some_and(|store| {
                    store
                        .load_transcript::<Vec<harness::ChatItem>>(&session.id)
                        .is_ok_and(|items| items.is_none_or(|items| items.is_empty()))
                });
            let recovered = recovered_session_status(session, empty_history);
            if recovered != session.status {
                session.status = recovered;
                if let Some(store) = &store
                    && let Err(error) = store.upsert_session(session)
                {
                    storage_error =
                        Some(format!("Could not save recovered session status: {error}"));
                }
            }
        }
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Dunlin to build, fix, or explain…")
                .rows(2)
                .submit_on_enter(true)
        });
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if is_composer_submit(event) {
                    this.send_prompt(window, cx);
                }
            },
        );
        let file_tree = projects
            .first()
            .map(|project| {
                if project.is_remote() {
                    FileTree {
                        root: project.path.clone(),
                        entries: Vec::new(),
                        error: None,
                    }
                } else {
                    FileTree::load(project.path.clone())
                }
            })
            .unwrap_or_else(|| FileTree {
                root: String::new(),
                entries: Vec::new(),
                error: None,
            });
        let search_hits = Vec::new();

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

        let activity_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search graph / trace…")
                .clean_on_escape()
        });
        let activity_search_subscription = cx.subscribe_in(
            &activity_search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.activity_query = this.activity_search.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );
        let terminal_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find in visible terminal…")
                .clean_on_escape()
        });
        let terminal_search_subscription = cx.subscribe_in(
            &terminal_search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.terminal_query = this.terminal_search.read(cx).value().to_string();
                    cx.notify();
                }
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
            cx.new(|cx| InputState::new(window, cx).placeholder("Name or relative path"));
        let group_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Group name")
                .clean_on_escape()
        });
        let group_subscription = cx.subscribe_in(
            &group_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_group_prompt(window, cx);
                }
            },
        );
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

        let editor_preview = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language("rust").soft_wrap(stored.word_wrap);
            state.set_value("pub fn route_permission(\n    child: &SessionId,\n    request: PermissionRequest,\n) -> Result<()> {\n    self.projection\n        .active_turn(child)?\n        .send(request)?;\n    Ok(())\n}", window, cx);
            state
        });
        let available_fonts = cx.text_system().all_font_names();
        let ui_font_family = available_font(&stored.ui_font_family, "Geist", &available_fonts);
        let code_font_family =
            available_font(&stored.code_font_family, "Geist Mono", &available_fonts);

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

        let mut sessions: Vec<SessionSummary> = projects
            .first()
            .map(|project| {
                all_sessions
                    .iter()
                    .filter(|session| {
                        session.project_id == project.id
                            && (!project.is_remote() || session.origin.is_remote())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if let Some(project) = projects.first()
            && sessions.is_empty()
            && !project.is_remote()
        {
            let harness = dunlin_domain::resolve_project_settings(
                project,
                &project_groups,
                &stored.default_harness,
                stored.permission_mode,
            )
            .map(|settings| settings.harness.value)
            .unwrap_or_else(|_| stored.default_harness.clone());
            let session = blank_session(&project.id, &harness, project.branch.clone());
            if let Some(store) = &store
                && let Err(error) = store.upsert_session(&session)
            {
                storage_error = Some(format!("Could not save the new session: {error}"));
            }
            all_sessions.push(session.clone());
            sessions.push(session);
        }
        let fallback_session = blank_session("none", &stored.default_harness, None);
        let task_budgets = crate::task_budgets::State::load(store.as_ref());
        let comparisons = crate::comparisons::State::load(store.as_ref());

        let mut app = Self {
            view: stored.view,
            view_epoch: 0,
            default_view: stored.default_view,
            section: if stored.dashboard_settings.start_at_dashboard {
                SidebarSection::Dashboard
            } else if stored.section == SidebarSection::Dashboard {
                SidebarSection::Projects
            } else {
                stored.section
            },
            projects,
            local_import_running: false,
            session_list: Default::default(),
            session_import: Default::default(),
            design_section: Default::default(),
            fallback_project,
            project_groups,
            collapsed_project_groups: stored.collapsed_project_groups.clone(),
            project_view: stored.project_view,
            project_sort: stored.project_sort,
            sidebar_width: stored.sidebar_width,
            show_project_branches: stored.show_project_branches,
            sidebar_resize: None,
            project_group_editor: None,
            group_contexts: stored.group_contexts.clone(),
            pending_group_context: None,
            repo_summaries: Default::default(),
            branch_ui: Default::default(),
            git_history: Default::default(),
            task_budgets,
            comparisons,
            overlay_window: Default::default(),
            project_icons: Default::default(),
            parked_runtimes: HashMap::new(),
            project_session_selections: HashMap::new(),
            navigation_generation: 0,
            mcp_operations: Default::default(),
            dashboard: Default::default(),
            skills: stored.skills.clone(),
            skills_ui: Default::default(),
            last_ai_target: stored.last_ai_target.clone(),
            ai_commit: Default::default(),
            billing: None,
            billing_loading: false,
            billing_error: None,
            billing_checked: None,
            billing_budget_usd: stored.billing_budget_usd,
            billing_alerts: stored.billing_alerts,
            onboarding_step: stored.onboarding_step,
            density: stored.density,
            reduce_motion: stored.reduce_motion,
            terminal: crate::terminal::State::new(cx),
            run: crate::run::State::new(stored.run_history.clone()),
            bottom_panel: stored.bottom_panel,
            bottom_panel_height: stored.bottom_panel_height,
            bottom_panel_width: stored.bottom_panel_width,
            terminal_dock: stored.terminal_dock,
            terminal_resize: None,
            terminal_drag: false,
            sidebar_motion: Default::default(),
            overlay_backdrop: crate::effects::for_window(window, cx),
            dashboard_settings: stored.dashboard_settings.clone(),
            account_usage_collapsed: stored.account_usage_collapsed,
            group_prompt: None,
            toast: None,
            context_menu: None,
            project_settings_target: None,
            project_runtime_draft: None,
            group_settings_draft: None,
            group_input,
            _group_subscription: group_subscription,
            selected_project: 0,
            sessions,
            all_sessions,
            fallback_session,
            selected_session: 0,
            restore_session_id: if stored.restore_session {
                stored.last_session_id.clone()
            } else {
                None
            },
            usages: Vec::new(),
            usage_cache: Default::default(),
            remote_usages: HashMap::new(),
            usage_loading: false,
            usage_scope: None,
            usage_request: None,
            usage_generation: 0,
            composer,
            _composer_subscription: composer_subscription,
            queue_open: false,
            chat_menu_open: false,
            search_input,
            _search_subscription: search_subscription,
            activity_search,
            activity_query: String::new(),
            _activity_search_subscription: activity_search_subscription,
            terminal_search,
            terminal_query: String::new(),
            _terminal_search_subscription: terminal_search_subscription,
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
            sidebar_collapsed: stored.sidebar_collapsed,
            pending_delete: None,
            diff_view: None,
            auto_connect: stored.auto_connect,
            restart_harness: stored.restart_harness,
            restore_session: stored.restore_session,
            word_wrap: stored.word_wrap,
            editor_minimap: stored.editor_minimap,
            autosave: stored.autosave,
            notifications_enabled: stored.notifications_enabled,
            show_shortcut_hints: stored.show_shortcut_hints,
            code_font_size: stored.code_font_size,
            ui_font_family,
            code_font_family,
            available_fonts,
            remote_hosts: stored.remote_hosts.clone(),
            remote_tokens: HashMap::new(),
            remote_sessions_loading: false,
            pending_remote_session: None,
            pending_model: None,
            remote_url_input,
            remote_code_input,
            remote_name_input,
            remote_loading: false,
            remote_picker: None,
            new_file_prompt: false,
            explorer: Default::default(),
            new_file_input,
            _new_file_subscription: new_file_subscription,
            inspector: None,
            workbench: Default::default(),
            editor,
            editor_preview,
            _editor_subscription: editor_subscription,
            diff_state,
            editor_dirty: false,
            editor_stale: false,
            editor_readonly: false,
            editor_mtime: None,
            autosave_generation: 0,
            editor_conflict: None,
            app_active: true,
            last_dock_badge: None,
            last_window_geometry: None,
            _activation_subscription: None,
            _window_bounds_subscription: None,
            pending_editor: None,
            selected_span: None,
            selected_node: None,
            activity: Default::default(),
            reasoning_open: stored.reasoning_open,
            final_answer_only: stored.final_answer_only,
            stream_responses: stored.stream_responses,
            chat_timestamps: stored.chat_timestamps,
            chat_duration: stored.chat_duration,
            chat_logo: stored.chat_logo,
            expanded_tool: None,
            chat_scroll: crate::chat_scroll::ChatScroll::default(),
            trace_filter: TraceFilter::All,
            file_tree,
            file_watcher: None,
            watcher_root: None,
            search_hits,
            search_query: String::new(),
            search_loading: false,
            notice: None,
            git: git::GitStatus::default(),
            git_loading: false,
            branches: Vec::new(),
            commit_input,
            store,
            storage_error: RefCell::new(storage_error),
            harness: harness::HarnessState::default(),
            harness_discovery: crate::harness_discovery::discover(),
            provider_catalog: crate::providers::Catalog::default(),
            mcp: crate::mcp::Catalog::default(),
            provider_auth: None,
            opencode_provider: stored.opencode_provider.clone(),
            recent_models: stored.recent_models.clone(),
        };

        dunlin_ui::theme::set_mode(
            if stored.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
        dunlin_ui::theme::set_code_font_size(stored.code_font_size, cx);
        dunlin_ui::theme::set_font_families(&app.ui_font_family, &app.code_font_family, cx);
        dunlin_ui::scale::set_density(match app.density {
            UiDensity::Compact => dunlin_ui::scale::Density::Compact,
            UiDensity::Comfortable => dunlin_ui::scale::Density::Comfortable,
        });

        let first_session = app.session().id.clone();
        app.load_transcript(&first_session);

        let mut restored_project = false;
        // Restore the selection before starting a process. Otherwise a process
        // for the first session can finish connecting after another is selected.
        let auto_connect = app.auto_connect;
        app.auto_connect = false;
        if let Some(index) = stored
            .last_project_id
            .as_deref()
            .and_then(|id| app.projects.iter().position(|project| project.id == id))
        {
            restored_project = index != app.selected_project;
            app.select_project_internal(index, false, cx);
        }
        if app.restore_session
            && let Some(index) = stored
                .last_session_id
                .as_deref()
                .and_then(|id| app.sessions.iter().position(|session| session.id == id))
        {
            app.select_session(index, cx);
        }
        // Older versions could save the project but not the newly selected
        // conversation when Chat was already open. Recover only an unused,
        // unavailable harness stub, never a conversation with real history.
        if !std::env::var("DUNLIN_HARNESS_COMMAND")
            .ok()
            .is_some_and(|command| !command.trim().is_empty())
        {
            let preferred = app.effective_harness();
            let selected_command =
                harness::harness_command(&app.session_harness(), &app.custom_harness_command);
            let preferred_command =
                harness::harness_command(&preferred, &app.custom_harness_command);
            if let Some(index) = startup_session_recovery(
                &app.sessions,
                app.selected_session,
                &app.harness.transcript,
                &preferred,
                harness::command_available(&selected_command),
                harness::command_available(&preferred_command),
                app.project().is_remote(),
            ) {
                app.select_session(index, cx);
                logging::info(
                    "restored the latest conversation for the available default harness; unused unavailable-harness history preserved",
                );
            }
        }
        app.auto_connect = auto_connect;
        app.section = if stored.dashboard_settings.start_at_dashboard {
            SidebarSection::Dashboard
        } else if stored.section == SidebarSection::Dashboard {
            SidebarSection::Projects
        } else {
            stored.section
        };

        if app.auto_connect
            && !app.harness.status.is_busy()
            && !app.remote_sessions_loading
            && !app.project_path_missing()
        {
            app.connect_harness(cx);
        }
        app.restart_file_watcher();
        app.refresh_project_icons(cx);
        if !restored_project && !app.projects.is_empty() {
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
                    .update(cx, |this, cx| {
                        this.check_open_file(cx);
                        this.poll_file_watcher(cx);
                        this.refresh_project_icons(cx);
                        this.poll_task_budgets(cx);
                        this.advance_comparisons(cx);
                        this.resume_visible_queue(cx);
                        this.advance_ai_commit(cx);
                        if this.section == SidebarSection::Dashboard {
                            this.refresh_dashboard(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        app._window_bounds_subscription =
            Some(cx.observe_window_bounds(window, |this, window, cx| {
                let geometry = window_state::from_bounds(window.window_bounds());
                if this.last_window_geometry != Some(geometry) {
                    this.last_window_geometry = Some(geometry);
                    window_state::save(geometry);
                }
                // The attached transparent plane follows owner resizing as
                // well as movement; cached views do not rerender on moves.
                if this.has_modal_overlay() {
                    cx.notify();
                }
            }));
        app.app_active = window.is_window_active();
        app._activation_subscription =
            Some(cx.observe_window_activation(window, |this, window, _| {
                this.app_active = window.is_window_active() || this.overlay_window.active;
            }));

        if let Ok(prompt) = std::env::var("DUNLIN_AUTOPROMPT")
            && !prompt.trim().is_empty()
        {
            app.composer
                .update(cx, |state, cx| state.set_value(prompt, window, cx));
            app.send_prompt(window, cx);
        }

        install_close_guard(window, cx);

        app
    }

    pub(crate) fn project(&self) -> &Project {
        self.projects
            .get(self.selected_project)
            .unwrap_or(&self.fallback_project)
    }

    pub(crate) fn navigation_epoch(&self) -> u64 {
        self.navigation_generation
    }

    pub(crate) fn project_path_missing(&self) -> bool {
        let project = self.project();
        !project.is_remote() && !Path::new(&project.path).is_dir()
    }

    pub(crate) fn effective_harness(&self) -> String {
        self.resolved_project_settings(self.project())
            .map(|settings| settings.harness.value)
            .unwrap_or_else(|_| self.default_harness.clone())
    }

    pub(crate) fn resolved_project_settings(
        &self,
        project: &Project,
    ) -> Result<dunlin_domain::ResolvedSettings, String> {
        dunlin_domain::resolve_project_settings(
            project,
            &self.project_groups,
            &self.default_harness,
            self.permission_mode,
        )
    }

    /// Existing conversations retain their harness even if the project default
    /// changes. ACP session ids are not interchangeable between harnesses.
    pub(crate) fn session_harness(&self) -> String {
        self.sessions
            .get(self.selected_session)
            .map(|session| session.harness.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.effective_harness())
    }

    pub(crate) fn effective_permission_mode(&self) -> PermissionMode {
        if let Some(context) = self.group_contexts.get(&self.session().id) {
            return context.permission_mode(
                &self.projects,
                &self.project_groups,
                self.permission_mode,
            );
        }
        self.resolved_project_settings(self.project())
            .map(|settings| settings.permission_mode.value)
            .unwrap_or(PermissionMode::ReadOnly)
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

    pub(crate) fn remote_workspace_for_project(
        &mut self,
        project: &Project,
    ) -> Result<remote::RemoteWorkspace, String> {
        let remote_project = project
            .remote
            .clone()
            .ok_or_else(|| "Project is not remote".to_string())?;
        let host = self
            .remote_host(&remote_project.host_id)
            .ok_or_else(|| "Remote server is not configured".to_string())?;
        let token = self.remote_token(&host.id)?;
        Ok(remote::RemoteWorkspace {
            host,
            project: remote_project,
            token,
        })
    }

    pub(crate) fn backend(&mut self) -> Result<remote::WorkspaceBackend, String> {
        let project = self.project().clone();
        if let Some(remote_project) = project.remote.clone() {
            let host = self
                .remote_host(&remote_project.host_id)
                .ok_or_else(|| "Remote server is not configured".to_string())?;
            let token = self.remote_token(&host.id)?;
            Ok(remote::WorkspaceBackend::Remote(Box::new(
                remote::RemoteWorkspace {
                    host,
                    project: remote_project,
                    token,
                },
            )))
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
                existing.status = SessionStatus::Idle;
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
            self.restore_session_id = None;
            self.harness.transcript.clear();
            self.pending_remote_session = Some(PendingRemoteSession::New);
            if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
            self.notice = Some("No remote sessions yet · starting a new one".into());
        } else {
            let preferred = self.restore_session_id.take();
            let index = preferred
                .as_deref()
                .and_then(|id| self.sessions.iter().position(|session| session.id == id))
                .unwrap_or(0);
            self.selected_session = index;
            let session = self.sessions[index].clone();
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
            let pending_id = existing.id.clone();
            if let Some(context) = self.group_contexts.remove(&pending_id) {
                self.group_contexts.insert(session_id.to_string(), context);
            }
            existing.id = session_id.to_string();
            existing.acp_session_id = Some(session_id.to_string());
            existing.status = SessionStatus::Idle;
            let stored = existing.clone();
            if let Some(all) = self
                .all_sessions
                .iter_mut()
                .find(|session| session.id.starts_with("pending-"))
            {
                all.id = session_id.to_string();
                all.acp_session_id = Some(session_id.to_string());
                all.status = SessionStatus::Idle;
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
            status: SessionStatus::Idle,
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
        let roots = self.session_additional_directories();
        match action {
            PendingRemoteSession::New => {
                let model = self
                    .resolved_project_settings(self.project())
                    .ok()
                    .and_then(|settings| {
                        settings
                            .default_model(&self.session_harness())
                            .map(str::to_string)
                    });
                self.pending_model = model.clone();
                harness.new_session_with_roots(cwd, roots, model);
            }
            PendingRemoteSession::Load { session_id } => {
                self.harness.transcript.clear();
                self.harness.started_tools.clear();
                self.harness.running = false;
                self.harness.usage = None;
                // `session/load` replays history as notifications before the
                // response arrives, so make the session current first.
                self.harness.session_id = Some(session_id.clone());
                harness.load_session_with_roots(session_id, cwd, roots);
            }
        }
        cx.notify();
    }

    /// Number of live turns in a project, derived from session state.
    pub(crate) fn running_sessions_for(&self, project_id: &str) -> usize {
        self.live_agents()
            .iter()
            .filter(|(project, _, state)| project == project_id && *state != "Connecting")
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
                command: Command::ToggleTerminal,
                label: "Toggle Terminal Panel".into(),
                hint: Some("⌘J".into()),
                icon: IconName::Terminal,
                keywords: "shell pty console bottom panel".into(),
            },
            CommandItem {
                command: Command::ToggleRun,
                label: "Toggle Run Panel".into(),
                hint: Some("⌘⇧J".into()),
                icon: IconName::SquareTerminal,
                keywords: "manifest scripts commands bottom panel".into(),
            },
            CommandItem {
                command: Command::Impact,
                label: "Open Impact Map".into(),
                hint: None,
                icon: IconName::Network,
                keywords: "references dependencies tests rust ast".into(),
            },
            CommandItem {
                command: Command::Compare,
                label: "Compare Solutions".into(),
                hint: None,
                icon: IconName::GitCompare,
                keywords: "a/b worktrees agents budget".into(),
            },
            CommandItem {
                command: Command::Budgets,
                label: "Open Task Budgets".into(),
                hint: None,
                icon: IconName::Gauge,
                keywords: "limits cost request time".into(),
            },
            CommandItem {
                command: Command::Checks,
                label: "Open Checks / Problems".into(),
                hint: None,
                icon: IconName::CircleCheck,
                keywords: "cargo test diagnostics commands".into(),
            },
            CommandItem {
                command: Command::Context,
                label: "Inspect Context".into(),
                hint: None,
                icon: IconName::FileText,
                keywords: "prompt environment provenance decisions".into(),
            },
            CommandItem {
                command: Command::Review,
                label: "Open Task Review".into(),
                hint: None,
                icon: IconName::FileDiff,
                keywords: "checkpoint diff restore review".into(),
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
                command: Command::NewProjectGroup,
                label: "New Project Group…".into(),
                hint: None,
                icon: IconName::FolderTree,
                keywords: "section folder private work group projects".into(),
            },
            CommandItem {
                command: Command::ToggleSidebar,
                label: "Toggle Sidebar".into(),
                hint: Some("⌘B".into()),
                icon: IconName::PanelLeft,
                keywords: "hide show sidebar layout".into(),
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
                label: format!("Restart {}", self.session_harness()),
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

        items.push(CommandItem {
            command: Command::ChooseBranch,
            label: "Switch branch…".into(),
            hint: Some("git".into()),
            icon: IconName::GitBranch,
            keywords: "checkout branch choose search".into(),
        });
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
                self.set_view(view, cx);
            }
            Command::Section(section) => {
                self.section = section;
                self.settings_open = false;
                self.persist_settings(cx);
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
            Command::NewProjectGroup => self.open_new_group_prompt(None, window, cx),
            Command::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                self.persist_settings(cx);
            }
            Command::ToggleTheme => {
                toggle_mode(window, cx);
                window.activate_window();
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
                    if self.harness.running {
                        self.set_notice(
                            "Wait for the current turn to finish before refreshing sessions",
                            cx,
                        );
                    } else {
                        self.select_project(self.selected_project, cx);
                    }
                } else {
                    self.set_notice("Only remote projects have relay sessions", cx);
                }
            }
            Command::GitPull => self.git_pull(cx),
            Command::GitPush => self.git_push(cx),
            Command::GitAmend => self.amend_commit(cx),
            Command::SwitchBranch(index) => {
                if let Some(branch) = self.branches.get(index).cloned() {
                    self.switch_branch(&branch, window, cx);
                }
            }
            Command::ChooseBranch => {
                self.open_branch_picker(&self.project().id.clone(), window, cx)
            }
            Command::ToggleTerminal => self.toggle_bottom_panel(BottomPanel::Terminal, window, cx),
            Command::ToggleRun => self.toggle_bottom_panel(BottomPanel::Run, window, cx),
            Command::Impact => self.open_impact_map(window, cx),
            Command::Compare => self.open_comparisons(window, cx),
            Command::Budgets => self.open_task_budgets(window, cx),
            Command::Checks => self.open_checks(window, cx),
            Command::Context => self.open_context_inspector(window, cx),
            Command::Review => self.open_task_review(window, cx),
            Command::CopyTrace => {
                let session = self.session().clone();
                let spans = projection::spans(&self.harness.transcript, &session);
                if spans.is_empty() {
                    self.set_notice("No trace to copy yet", cx);
                } else {
                    let json = serde_json::to_string_pretty(&spans).unwrap_or_default();
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                    self.set_notice("Copied trace JSON", cx);
                }
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

    /// Open the native macOS folder picker and add the chosen folder.
    pub(crate) fn open_folder(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Open project folders or a folder containing repositories".into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);

        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = receiver.await {
                let _ = this.update(cx, |this, cx| {
                    if paths.len() == 1 {
                        this.add_project(paths.remove(0), cx);
                    } else if !paths.is_empty() {
                        this.import_local_folders(paths, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Re-point a local project whose folder was moved or deleted.
    pub(crate) fn locate_project(&mut self, cx: &mut Context<Self>) {
        self.locate_project_at(self.selected_project, cx);
    }

    pub(crate) fn locate_project_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Locate the project folder".into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.relocate_project(index, path, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn relocate_project(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(project) = self.projects.get(index) {
            let id = project.id.clone();
            if (index == self.selected_project && self.editor_dirty)
                || self
                    .workbench
                    .tabs
                    .iter()
                    .any(|tab| tab.project == id && tab.dirty)
            {
                self.file_notice(
                    "Save or close dirty tabs before relocating their project",
                    cx,
                );
                return;
            }
            self.workbench.tabs.retain(|tab| tab.project != id);
            self.stop_project_runtimes(&id, cx);
            if index == self.selected_project {
                self.disconnect_harness(cx);
                self.close_inspector();
            }
        }
        let path = path.to_string_lossy().trim_end_matches('/').to_string();
        if !Path::new(&path).is_dir() {
            self.set_notice("That path is not a folder", cx);
            return;
        }
        let Some(project) = self.projects.get_mut(index) else {
            return;
        };
        project.path = path.clone();
        project.branch = git_branch(&path);
        let stored = project.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project(&stored)
        {
            self.report_storage_error("save project location", error);
        }
        let active = index == self.selected_project;
        self.notice = Some(format!("Located {}", stored.name));
        self.persist_settings(cx);
        if active {
            self.restart_file_watcher();
            self.refresh_file_tree(cx);
            self.refresh_git_status(cx);
            if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn add_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.import_local_folders(vec![path], cx);
    }

    /// Open the small create/rename dialog for a project section.
    pub(crate) fn open_new_group_prompt(
        &mut self,
        assign_project: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_group_editor = None;
        self.group_prompt = Some(GroupPrompt {
            id: None,
            assign_project,
            parent_id: None,
        });
        self.group_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn open_rename_group_prompt(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .project_groups
            .iter()
            .find(|group| group.id == id)
            .cloned()
        else {
            return;
        };
        self.project_group_editor = None;
        self.group_prompt = Some(GroupPrompt {
            id: Some(group.id),
            assign_project: None,
            parent_id: group.parent_id,
        });
        self.group_input.update(cx, |state, cx| {
            state.set_value(group.name, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn cancel_group_prompt(&mut self, cx: &mut Context<Self>) {
        self.group_prompt = None;
        cx.notify();
    }

    pub(crate) fn submit_group_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.group_prompt.take() else {
            return;
        };
        let name = self.group_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.group_prompt = Some(prompt);
            self.set_notice("Give the group a name", cx);
            return;
        }
        let editing_id = prompt.id.as_deref();
        if self.project_groups.iter().any(|group| {
            Some(group.id.as_str()) != editing_id
                && group.parent_id == prompt.parent_id
                && group.name.eq_ignore_ascii_case(&name)
        }) {
            self.group_prompt = Some(prompt);
            self.set_notice(format!("A group named {name} already exists"), cx);
            return;
        }

        if let Some(id) = &prompt.id {
            let Some(index) = self.project_groups.iter().position(|group| &group.id == id) else {
                self.set_notice("The group no longer exists", cx);
                return;
            };
            let mut group = self.project_groups[index].clone();
            group.name = name.clone();
            if let Some(store) = &self.store
                && let Err(error) = store.upsert_project_group(&group)
            {
                self.group_prompt = Some(prompt);
                self.set_notice(error.to_string(), cx);
                return;
            }
            self.project_groups[index] = group;
            self.notice = Some(format!("Renamed the group to {name}"));
        } else {
            let tone = match self.project_groups.len() % 6 {
                0 => ProjectTone::Blue,
                1 => ProjectTone::Green,
                2 => ProjectTone::Violet,
                3 => ProjectTone::Amber,
                4 => ProjectTone::Rose,
                _ => ProjectTone::Teal,
            };
            let group = ProjectGroup {
                id: format!("group-{}", uuid::Uuid::now_v7()),
                name: name.clone(),
                tone,
                parent_id: prompt.parent_id.clone(),
                settings: Default::default(),
            };
            if let Err(error) = group.validate_parent(&self.project_groups) {
                self.group_prompt = Some(prompt);
                self.file_notice(error, cx);
                return;
            }
            if let Some(store) = &self.store
                && let Err(error) = store.upsert_project_group(&group)
            {
                self.group_prompt = Some(prompt);
                self.file_notice(error.to_string(), cx);
                return;
            }
            self.project_groups.push(group.clone());
            if let Some(parent) = &group.parent_id {
                self.collapsed_project_groups.retain(|id| id != parent);
            }
            if let Some(index) = prompt.assign_project {
                self.assign_project_group(index, Some(group.id), cx);
            }
            self.notice = Some(format!("Created the group {name}"));
        }

        self.group_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.group_prompt = None;
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn delete_project_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(group) = self
            .project_groups
            .iter()
            .find(|group| group.id == id)
            .cloned()
        else {
            return;
        };
        let project_ids: Vec<String> = self
            .projects
            .iter()
            .filter(|project| project.group_id.as_deref() == Some(id))
            .map(|project| project.id.clone())
            .collect();
        let child_ids: Vec<_> = self
            .project_groups
            .iter()
            .filter(|g| g.parent_id.as_deref() == Some(id))
            .map(|g| g.id.clone())
            .collect();
        if let Some(store) = &self.store
            && let Err(error) = store.delete_project_group(id)
        {
            self.file_notice(error.to_string(), cx);
            return;
        }
        for project in &mut self.projects {
            if project.group_id.as_deref() == Some(id) {
                project.group_id = group.parent_id.clone();
            }
        }
        self.project_groups.retain(|group| group.id != id);
        for child in self
            .project_groups
            .iter_mut()
            .filter(|g| g.parent_id.as_deref() == Some(id))
        {
            child.parent_id = group.parent_id.clone();
        }
        self.collapsed_project_groups.retain(|group| group != id);
        self.group_prompt = None;
        self.persist_settings(cx);
        self.notice =
            Some("Deleted the group · projects and subgroups were promoted one level".into());
        self.toast = Some(Toast {
            id: uuid::Uuid::now_v7(),
            message: format!("Deleted group {}", group.name),
            action_label: "Undo".into(),
            action: ToastAction::RestoreGroup {
                group: Box::new(group),
                project_ids,
                child_ids,
            },
        });
        cx.notify();
    }

    pub(crate) fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
        cx.notify();
    }

    pub(crate) fn apply_toast(&mut self, cx: &mut Context<Self>) {
        let Some(toast) = self.toast.take() else {
            return;
        };
        match toast.action {
            ToastAction::Info => {}
            ToastAction::RestoreGroup {
                group,
                mut project_ids,
                mut child_ids,
            } => {
                let mut group = *group;
                let promoted_parent = group.parent_id.clone();
                if group
                    .parent_id
                    .as_ref()
                    .is_some_and(|id| !self.project_groups.iter().any(|g| &g.id == id))
                {
                    group.parent_id = None;
                }
                if self
                    .project_groups
                    .iter()
                    .any(|candidate| candidate.id == group.id)
                {
                    self.set_notice("The group already exists", cx);
                    return;
                }
                project_ids.retain(|id| {
                    self.projects
                        .iter()
                        .any(|p| &p.id == id && p.group_id == promoted_parent)
                });
                let mut restored_groups = self.project_groups.clone();
                restored_groups.push(group.clone());
                child_ids.retain(|id| {
                    if let Some(child) = restored_groups
                        .iter()
                        .find(|g| &g.id == id && g.parent_id == promoted_parent)
                    {
                        let mut child = child.clone();
                        child.parent_id = Some(group.id.clone());
                        child.validate_parent(&restored_groups).is_ok()
                    } else {
                        false
                    }
                });
                if let Some(store) = &self.store
                    && let Err(error) = store.restore_project_group(
                        &group,
                        promoted_parent.as_deref(),
                        &project_ids,
                        &child_ids,
                    )
                {
                    self.set_notice(error.to_string(), cx);
                    return;
                }
                self.project_groups.push(group.clone());
                for project in &mut self.projects {
                    if project_ids.iter().any(|id| id == &project.id)
                        && project.group_id == promoted_parent
                    {
                        project.group_id = Some(group.id.clone());
                    }
                }
                for index in 0..self.project_groups.len() {
                    let child = &self.project_groups[index];
                    if child_ids.contains(&child.id) && child.parent_id == promoted_parent {
                        let mut child = child.clone();
                        child.parent_id = Some(group.id.clone());
                        if child.validate_parent(&self.project_groups).is_ok() {
                            self.project_groups[index] = child;
                        }
                    }
                }
                self.collapsed_project_groups.retain(|id| id != &group.id);
                self.notice = Some(format!("Restored group {}", group.name));
            }
        }
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn toggle_project_group(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .collapsed_project_groups
            .iter()
            .position(|group| group == id)
        {
            self.collapsed_project_groups.remove(index);
        } else {
            self.collapsed_project_groups.push(id.to_string());
        }
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn open_project_settings(&mut self, index: usize, cx: &mut Context<Self>) {
        self.group_settings_draft = None;
        self.project_runtime_draft = None;
        self.context_menu = None;
        self.project_menu_open = false;
        self.project_settings_target = Some(index);
        cx.notify();
    }

    pub(crate) fn close_project_settings(&mut self, cx: &mut Context<Self>) {
        self.project_runtime_draft = None;
        self.project_settings_target = None;
        cx.notify();
    }

    pub(crate) fn store_project_settings(&mut self, index: usize) {
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let stored = project.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project(&stored)
        {
            self.report_storage_error("save project settings", error);
        }
    }

    pub(crate) fn set_project_harness(
        &mut self,
        index: usize,
        harness: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = self.projects.get_mut(index) {
            project.settings.harness = harness;
        }
        self.store_project_settings(index);
        self.persist_settings(cx);
        self.notice = Some("Project default harness updated · applies to new conversations".into());
        cx.notify();
    }

    pub(crate) fn set_project_permission(
        &mut self,
        index: usize,
        mode: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = self.projects.get_mut(index) {
            project.settings.permission_mode = mode;
        }
        self.store_project_settings(index);
        self.persist_settings(cx);
        self.notice = Some("Project permission mode updated".into());
        cx.notify();
    }

    pub(crate) fn reset_project_settings(&mut self, index: usize, cx: &mut Context<Self>) {
        self.project_runtime_draft = None;
        if let Some(project) = self.projects.get_mut(index) {
            project.settings = ProjectSettings::default();
        }
        self.store_project_settings(index);
        self.persist_settings(cx);
        self.notice = Some("Project overrides cleared".into());
        cx.notify();
    }

    pub(crate) fn assign_project_group(
        &mut self,
        index: usize,
        group_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if group_id
            .as_ref()
            .is_some_and(|id| !self.project_groups.iter().any(|g| &g.id == id))
        {
            return false;
        }
        let Some(project) = self.projects.get(index) else {
            return false;
        };
        let mut project = project.clone();
        project.group_id = group_id;
        let validation = self
            .resolved_project_settings(&project)
            .and_then(|settings| {
                if project.is_remote() {
                    Ok(())
                } else {
                    settings.process_environment().map(|_| ())
                }
            });
        if let Err(error) = validation {
            self.set_notice(error, cx);
            return false;
        }
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project(&project)
        {
            self.set_notice(error.to_string(), cx);
            return false;
        }
        self.projects[index] = project.clone();
        if let Some(toast) = &mut self.toast {
            toast.action.member_moved(&project.id, false);
        }
        self.project_group_editor = None;
        self.persist_settings(cx);
        self.notice = Some(format!(
            "Moved {} to {} · inherited defaults and client approvals updated",
            project.name,
            project
                .group_id
                .as_ref()
                .and_then(|id| self.project_groups.iter().find(|group| &group.id == id))
                .map(|group| group.name.as_str())
                .unwrap_or("No group")
        ));
        cx.notify();
        true
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
        if let Err(error) = remote::validate_relay_url(&url) {
            self.set_notice(error, cx);
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
                    let pair = remote::pair(&url_for_task, &code, "Dunlin on macOS")?;
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
                            user_role: pair.user.role,
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
        if self
            .project()
            .remote
            .as_ref()
            .is_some_and(|project| project.host_id == id)
        {
            self.interrupt_live_turn(cx);
            self.disconnect_harness(cx);
        }
        self.remote_hosts.retain(|host| host.id != id);
        self.remote_tokens.remove(id);
        self.remote_usages.remove(id);
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
            group_id: None,
            settings: Default::default(),
        };
        if let Some(store) = &self.store {
            let _ = store.upsert_project(&project);
        }
        self.projects.push(project);
        let index = self.projects.len() - 1;
        let local_id = self.projects[index].id.clone();
        self.remote_picker = None;
        self.settings_open = false;
        self.select_project(index, cx);
        self.sync_remote_project_settings(host_id, &info.id, &local_id, cx);
        self.notice = Some(format!("Added remote project {}", info.name));
        cx.notify();
    }

    fn sync_remote_project_settings(
        &mut self,
        host_id: &str,
        remote_id: &str,
        local_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.remote_host(host_id) else {
            return;
        };
        let token = match self.remote_token(&host.id) {
            Ok(token) => token,
            Err(error) => {
                logging::warn(&format!("remote settings: {error}"));
                return;
            }
        };
        let url = host.url;
        let remote_id = remote_id.to_string();
        let local_id = local_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { remote::project_settings(&url, &token, &remote_id) })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(settings) => {
                    if let Some(project) = this.projects.iter_mut().find(|p| p.id == local_id)
                        && project.is_remote()
                    {
                        project.settings = settings;
                        if let Some(store) = &this.store {
                            let _ = store.upsert_project(project);
                        }
                        cx.notify();
                    }
                }
                Err(error) => logging::warn(&format!("remote settings: {error}")),
            });
        })
        .detach();
    }

    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            self.set_notice("Open a project first", cx);
            return;
        }
        if self.harness.local_session_loading || self.harness.status.is_busy() {
            self.set_notice("Wait for the harness to finish connecting", cx);
            return;
        }
        let project_id = self.project().id.clone();
        if let Some(remote_project) = self.project().remote.clone() {
            self.interrupt_live_turn(cx);
            self.stash_transcript();
            self.sessions
                .retain(|session| !session.id.starts_with("pending-"));
            self.all_sessions
                .retain(|session| !session.id.starts_with("pending-"));
            let pending = SessionSummary {
                id: format!("pending-{}", uuid::Uuid::now_v7()),
                project_id,
                title: "New session".into(),
                status: SessionStatus::Idle,
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
            if let Some(context) = self.pending_group_context.take() {
                self.group_contexts.insert(pending.id.clone(), context);
            }
            self.harness.transcript.clear();
            self.harness.session_id = None;
            self.harness.running = false;
            self.selected_node = None;
            self.selected_span = None;
            self.pending_remote_session = Some(PendingRemoteSession::New);
            let group_roots = self.session_remote_roots();

            if !group_roots.is_empty() && self.harness.harness.is_some() {
                // The relay sandbox and ACP scope were fixed when the current
                // connection opened. A group root list requires a fresh
                // connection so a prompt cannot see a wider/narrower set than
                // the approved manifest.
                self.disconnect_harness(cx);
            }
            if self.harness.status.is_ready() {
                let action = self.pending_remote_session.take().unwrap();
                self.start_remote_session(action, cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            } else {
                self.notice = Some("Connect to the relay to start a session".into());
            }

            self.set_view(self.default_view, cx);
            self.session_menu_open = false;
            self.ensure_usage(cx);
            cx.notify();
            return;
        }

        let branch = self.project().branch.clone();
        let session = blank_session(&project_id, &self.effective_harness(), branch);
        self.park_current_runtime(cx);
        self.reset_mcp_for_navigation();
        self.persist_session(&session);
        self.all_sessions.push(session.clone());
        self.sessions.push(session.clone());
        self.selected_session = self.sessions.len() - 1;
        if let Some(context) = self.pending_group_context.take() {
            self.group_contexts.insert(session.id.clone(), context);
        }
        self.load_transcript(&session.id);
        self.harness.session_id = None;
        self.harness.running = false;

        if self.auto_connect {
            self.connect_harness(cx);
        }

        self.set_view(self.default_view, cx);
        self.selected_node = None;
        self.selected_span = None;
        self.session_menu_open = false;
        self.notice = Some("Created a new session".into());
        self.ensure_usage(cx);
        // set_view only saves when the view changes; a new conversation must
        // also become the startup selection when we were already in Chat.
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        self.disconnect_harness(cx);
        let background: Vec<_> = self.parked_runtimes.keys().cloned().collect();
        for id in background {
            self.stop_parked_runtime(&id);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.clear_sessions()
        {
            self.report_storage_error("clear conversation history", error);
            cx.notify();
            return;
        }
        self.forget_all_transcripts();
        self.group_contexts.clear();
        self.all_sessions.clear();
        self.sessions.clear();
        self.selected_span = None;
        self.selected_node = None;
        self.expanded_tool = None;

        let project_id = self.project().id.clone();
        let branch = self.project().branch.clone();
        let session = blank_session(&project_id, &self.effective_harness(), branch);
        self.persist_session(&session);
        self.all_sessions.push(session.clone());
        self.sessions.push(session.clone());
        self.selected_session = 0;
        self.harness.session_id = None;
        self.load_transcript(&session.id);

        if self.harness.status.is_ready() {
            self.activate_local_session(cx);
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
        if self.harness.local_session_loading || self.harness.status.is_busy() {
            self.set_notice("Wait for the harness to finish connecting", cx);
            return;
        }
        let remote_origin = match &self.sessions[index].origin {
            SessionOrigin::Remote { host_id } => Some(host_id.clone()),
            SessionOrigin::Local => None,
        };
        if let Some(host_id) = remote_origin {
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
            if let Err(error) = remote::delete_session(&host.url, &token, id)
                && !error.contains("404")
            {
                self.set_notice(format!("Could not delete the remote session: {error}"), cx);
                return;
            }
        }

        let active = index == self.selected_session;
        self.stop_parked_runtime(id);
        if active && !self.project().is_remote() {
            self.disconnect_harness(cx);
        }
        if active && self.harness.running {
            self.cancel_turn(cx);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.delete_session(id)
        {
            self.report_storage_error("delete session", error);
            cx.notify();
            return;
        }
        self.sessions.remove(index);
        self.all_sessions.retain(|session| session.id != id);
        self.group_contexts.remove(id);
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
                let session = blank_session(&project_id, &self.effective_harness(), branch);
                self.persist_session(&session);
                self.all_sessions.push(session.clone());
                self.sessions.push(session);
            }
        }
        self.selected_session =
            session_index_after_delete(index, self.selected_session, self.sessions.len());

        if active && !self.sessions.is_empty() {
            let session = self.session().clone();
            if !session.origin.is_remote() {
                if !self.restore_runtime(&session.id) {
                    self.load_transcript(&session.id);
                    if self.auto_connect {
                        self.connect_harness(cx);
                    }
                }
                self.refresh_providers(cx);
                self.refresh_mcp(cx);
                self.notice = Some("Deleted session".into());
                self.persist_settings(cx);
                cx.notify();
                return;
            }
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
                self.activate_local_session(cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
        }

        self.notice = Some("Deleted session".into());
        self.persist_settings(cx);
        cx.notify();
    }

    fn delete_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.projects.iter().position(|project| project.id == id) else {
            return;
        };
        let active = index == self.selected_project;
        if self.workbench.tabs.iter().any(|tab| {
            tab.project == id && tab.dirty && Some(&tab.id) != self.workbench.active.as_ref()
        }) {
            self.file_notice(
                "Save or close this project's dirty tabs before removing it",
                cx,
            );
            return;
        }
        if active && self.editor_dirty {
            self.pending_editor = Some(PendingEditor::RemoveProject { id: id.to_string() });
            cx.notify();
            return;
        }
        self.stop_project_runtimes(id, cx);
        if active {
            if self.harness.running {
                self.cancel_turn(cx);
            }
            self.stash_transcript();
            self.disconnect_harness(cx);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.delete_project(id)
        {
            self.report_storage_error("remove project", error);
            if active {
                let session_id = self.session().id.clone();
                self.load_transcript(&session_id);
            }
            cx.notify();
            return;
        }

        let removed_sessions: Vec<String> = self
            .all_sessions
            .iter()
            .filter(|session| session.project_id == id)
            .map(|session| session.id.clone())
            .collect();
        self.all_sessions.retain(|session| session.project_id != id);
        for session_id in &removed_sessions {
            self.group_contexts.remove(session_id);
            self.forget_transcript(session_id);
        }
        self.project_group_editor = None;
        self.project_settings_target = None;
        self.project_runtime_draft = None;
        self.context_menu = None;
        let removed = self.projects.remove(index);
        self.workbench.tabs.retain(|tab| tab.project != removed.id);

        if self.projects.is_empty() {
            self.disconnect_harness(cx);
            self.forget_all_transcripts();
            self.harness.transcript.clear();
            self.sessions.clear();
            self.selected_project = 0;
            self.selected_session = 0;
            self.pending_remote_session = None;
            self.remote_sessions_loading = false;
            self.close_inspector();
            self.diff_view = None;
            self.file_tree = FileTree {
                root: String::new(),
                entries: Vec::new(),
                error: None,
            };
            self.restart_file_watcher();
            self.git = git::GitStatus::default();
            self.branches.clear();
            self.search_hits.clear();
        } else if active {
            let next = index.min(self.projects.len() - 1);
            self.selected_project = usize::MAX;
            self.sessions.clear();
            self.selected_session = 0;
            self.harness = Default::default();
            self.select_project(next, cx);
        } else if index < self.selected_project {
            self.selected_project -= 1;
        }

        self.notice = Some(format!("Removed {}", removed.name));
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn activity_visible_ids(&self) -> Vec<String> {
        match self.view {
            MainView::Graph => {
                let session = self.session();
                let all = projection::nodes(&self.harness.transcript, self.project(), session);
                let nodes = crate::activity::graph_nodes(&all, &self.activity.agents_expanded);
                crate::activity::filter_graph_nodes(&nodes, &self.activity_query)
                    .into_iter()
                    .map(|node| node.id)
                    .collect()
            }
            MainView::Trace => {
                let spans = projection::spans(&self.harness.transcript, self.session());
                let spans = crate::activity::filter_spans(&spans, &self.activity_query);
                crate::activity::trace_rows(
                    &spans,
                    &self.activity.spans_expanded,
                    self.trace_filter,
                )
                .into_iter()
                .map(|span| span.id)
                .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn activity_move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ids = self.activity_visible_ids();
        if ids.is_empty() {
            return;
        }
        let current = match self.view {
            MainView::Graph => self.selected_node.as_ref(),
            MainView::Trace => self.selected_span.as_ref(),
            _ => None,
        };
        let index = current
            .and_then(|id| ids.iter().position(|candidate| candidate == id))
            .map(|index| {
                if delta < 0 {
                    index.saturating_sub(delta.unsigned_abs())
                } else {
                    (index + delta as usize).min(ids.len() - 1)
                }
            })
            .unwrap_or(if delta < 0 { ids.len() - 1 } else { 0 });
        match self.view {
            MainView::Graph => self.selected_node = Some(ids[index].clone()),
            MainView::Trace => self.selected_span = Some(ids[index].clone()),
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn activity_toggle_selection(&mut self, cx: &mut Context<Self>) {
        match self.view {
            MainView::Graph => {
                if let Some(id) = self.selected_node.clone()
                    && !self.activity.agents_expanded.remove(&id)
                {
                    self.activity.agents_expanded.insert(id);
                }
            }
            MainView::Trace => {
                if let Some(id) = self.selected_span.clone()
                    && id != "root"
                    && !self.activity.spans_expanded.remove(&id)
                {
                    self.activity.spans_expanded.insert(id);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    pub(crate) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.group_settings_draft.is_some() {
            if event.keystroke.key == "escape" {
                self.close_group_settings(cx);
                self.focus_handle.focus(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if self.project_settings_target.is_some() {
            if event.keystroke.key == "escape" {
                self.close_project_settings(cx);
                self.focus_handle.focus(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if self.branch_ui.is_open() {
            self.branch_picker_key(&event.keystroke.key, window, cx);
            return;
        }
        if self.provider_auth.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_provider_auth(window, cx);
                    cx.stop_propagation();
                }
                "enter" => {
                    self.submit_provider_auth(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        // Native composer dropdowns own Escape/arrows while open. Do not let
        // the app's capture handler close the editor behind a focused menu.
        if self.chat_menu_open && self.view == MainView::Chat && !self.settings_open {
            return;
        }
        let keystroke = &event.keystroke;
        let platform = keystroke.modifiers.platform;
        let shift = keystroke.modifiers.shift;

        if self.editor_conflict.is_some() {
            if keystroke.key.as_str() == "escape" {
                self.cancel_editor_conflict(cx);
            }
            cx.stop_propagation();
            return;
        }

        if self.pending_editor.is_some() {
            match keystroke.key.as_str() {
                "escape" => self.cancel_pending_editor(cx),
                "enter" => self.save_pending_editor(window, cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }

        if self.group_prompt.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.cancel_group_prompt(cx);
                    cx.stop_propagation();
                    return;
                }
                "enter" => {
                    self.submit_group_prompt(window, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        if self.queue_open && keystroke.key.as_str() == "escape" {
            self.queue_open = false;
            cx.stop_propagation();
            cx.notify();
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

        if self.bottom_panel == BottomPanel::Terminal
            && self.terminal.focus.is_focused(window)
            && !self.settings_open
            && !self.git_history.is_active_for(self.project())
            && self.terminal_handle_key(event, window, cx)
        {
            cx.stop_propagation();
            return;
        }

        if !platform
            && matches!(self.view, MainView::Graph | MainView::Trace)
            && !self
                .activity_search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            match keystroke.key.as_str() {
                "up" => {
                    self.activity_move_selection(-1, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" => {
                    self.activity_move_selection(1, cx);
                    cx.stop_propagation();
                    return;
                }
                "enter" | "space" => {
                    self.activity_toggle_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "right" => {
                    if let Some(id) = self
                        .selected_node
                        .clone()
                        .or_else(|| self.selected_span.clone())
                    {
                        if self.view == MainView::Graph {
                            self.activity.agents_expanded.insert(id);
                        } else {
                            self.activity.spans_expanded.insert(id);
                        }
                        cx.notify();
                    }
                    cx.stop_propagation();
                    return;
                }
                "left" => {
                    if let Some(id) = self
                        .selected_node
                        .take()
                        .or_else(|| self.selected_span.take())
                    {
                        self.activity.agents_expanded.remove(&id);
                        self.activity.spans_expanded.remove(&id);
                        cx.notify();
                    }
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.selected_node = None;
                    self.selected_span = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        if platform {
            let handled = match keystroke.key.as_str() {
                "=" | "+" if matches!(self.view, MainView::Graph | MainView::Trace) => {
                    if self.view == MainView::Graph {
                        self.activity.graph_zoom = (self.activity.graph_zoom + 0.1).clamp(0.5, 1.8);
                    } else {
                        self.activity.trace_zoom = (self.activity.trace_zoom + 0.1).clamp(0.6, 1.8);
                    }
                    cx.notify();
                    true
                }
                "-" if matches!(self.view, MainView::Graph | MainView::Trace) => {
                    if self.view == MainView::Graph {
                        self.activity.graph_zoom = (self.activity.graph_zoom - 0.1).clamp(0.5, 1.8);
                    } else {
                        self.activity.trace_zoom = (self.activity.trace_zoom - 0.1).clamp(0.6, 1.8);
                    }
                    cx.notify();
                    true
                }
                "0" if matches!(self.view, MainView::Graph | MainView::Trace) => {
                    self.activity.graph_zoom = 1.;
                    self.activity.trace_zoom = 1.;
                    cx.notify();
                    true
                }
                "j" => {
                    self.toggle_bottom_panel(
                        if shift {
                            BottomPanel::Run
                        } else {
                            BottomPanel::Terminal
                        },
                        window,
                        cx,
                    );
                    true
                }
                "k" => {
                    self.open_palette(window, cx);
                    true
                }
                "1" => {
                    self.set_view(MainView::Chat, cx);
                    true
                }
                "2" => {
                    self.set_view(MainView::Graph, cx);
                    true
                }
                "3" => {
                    self.set_view(MainView::Trace, cx);
                    true
                }
                "b" => {
                    self.sidebar_collapsed = !self.sidebar_collapsed;
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
                "s" if shift => {
                    self.save_all_tabs(cx);
                    true
                }
                "s" if self.inspector.is_some()
                    && !self.git_history.is_active_for(self.project()) =>
                {
                    self.save_editor(cx);
                    true
                }
                "w" if self.git_history.is_active_for(self.project()) => {
                    self.close_git_history(cx);
                    self.focus_handle.focus(window, cx);
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
                    window.activate_window();
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
            } else if self.git_history.is_active_for(self.project()) {
                self.show_chat_tab(window, cx);
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
            self.search_hits = Vec::new();
            self.search_loading = false;
            self.notice = Some("Type a query and press Enter.".into());
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

    pub(crate) fn toggle_project_pin(&mut self, cx: &mut Context<Self>) {
        self.toggle_project_pin_at(self.selected_project, cx);
    }

    pub(crate) fn toggle_project_pin_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get_mut(index) else {
            return;
        };
        project.pinned = !project.pinned;
        let pinned = project.pinned;
        let name = project.name.clone();
        let stored = project.clone();
        if let Some(store) = &self.store {
            let _ = store.upsert_project(&stored);
        }
        self.notice = Some(if pinned {
            format!("Pinned {name}")
        } else {
            format!("Unpinned {name}")
        });
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
            self.stop_project_runtimes(&stored.id, cx);
            self.notice = Some(format!("Untrusted {name} · harness stopped"));
        }
        cx.notify();
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.section == SidebarSection::Dashboard {
            self.section = SidebarSection::Projects;
        }
        self.select_project_internal(index, true, cx);
        if index < self.projects.len() && self.selected_project == index {
            self.remember_dashboard_project(cx);
        }
        cx.notify();
    }

    pub(crate) fn select_project_internal(
        &mut self,
        index: usize,
        connect: bool,
        cx: &mut Context<Self>,
    ) {
        if self.explorer.busy {
            self.file_notice("Wait for the file operation before switching projects", cx);
            return;
        }
        if index >= self.projects.len() {
            return;
        }
        if self.editor_dirty {
            self.pending_editor = Some(PendingEditor::SwitchProject {
                id: self.projects[index].id.clone(),
            });
            cx.notify();
            return;
        }
        if index == self.selected_project {
            return;
        }
        self.cancel_ai_commit_preparation();
        self.park_current_runtime(cx);
        self.reset_mcp_for_navigation();
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        self.remote_sessions_loading = false;
        self.selected_project = index;
        self.explorer.selected = None;
        self.explorer.creation = None;
        self.explorer.expand_after_scan.clear();
        self.new_file_prompt = false;
        self.git = git::GitStatus::default();
        self.project_menu_open = false;
        self.session_menu_open = false;
        let project = self.projects[index].clone();
        self.restart_file_watcher();
        let path = project.path.clone();
        self.file_tree = FileTree {
            root: path.clone(),
            entries: Vec::new(),
            error: None,
        };
        self.refresh_file_tree(cx);
        self.search_hits = Vec::new();
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
            let navigation_generation = self.navigation_generation;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { remote::sessions(&url, &token, &remote_id) })
                    .await;
                this.update(cx, |this, cx| {
                    if this.project().id != expected_project
                        || this.navigation_generation != navigation_generation
                    {
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
            self.refresh_usage(cx);
            self.notice = Some(format!("Switched to {}", project.name));
            self.persist_settings(cx);
            cx.notify();
            return;
        }

        if self.sessions.is_empty() {
            let session = blank_session(
                &project_id,
                &self.effective_harness(),
                project.branch.clone(),
            );
            self.persist_session(&session);
            self.all_sessions.push(session.clone());
            self.sessions.push(session);
        }
        self.selected_session = self
            .sessions
            .iter()
            .position(|s| self.project_session_selections.get(&project_id) == Some(&s.id))
            .unwrap_or(0);
        let first_session = self.sessions[0].id.clone();
        let first_session = self
            .sessions
            .get(self.selected_session)
            .map(|s| s.id.clone())
            .unwrap_or(first_session);
        let restored = self.restore_runtime(&first_session);
        if !restored {
            self.load_transcript(&first_session);
        }

        self.notice = Some(format!("Switched to {}", project.name));
        let missing = !project.is_remote() && !Path::new(&project.path).is_dir();
        if missing {
            self.disconnect_harness(cx);
            self.notice = Some(format!(
                "{} is missing · use Locate folder… in the project menu",
                project.name
            ));
        } else if restored {
            self.ensure_usage(cx);
            self.refresh_mcp(cx);
            self.refresh_providers(cx);
            self.resume_visible_queue(cx);
        } else if self.auto_connect && connect {
            self.connect_harness(cx);
        } else {
            self.disconnect_harness(cx);
            self.notice = Some(format!("Switched to {}", project.name));
        }
        if !missing {
            self.refresh_git_status(cx);
        }
        self.refresh_usage(cx);
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
        let mut expanded = self.file_tree.expanded_paths();
        expanded.extend(self.explorer.expand_after_scan.iter().cloned());
        let expected = self.project().path.clone();
        let project = self.project().id.clone();
        self.explorer.tree_generation = self.explorer.tree_generation.wrapping_add(1);
        let generation = self.explorer.tree_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.tree() })
                .await;
            this.update(cx, |this, cx| {
                if this.project().path != expected || this.project().id != project {
                    return;
                }
                // Watcher bursts can start a newer scan while an older one is
                // still running. Never let a stale completion replace a newer
                // populated tree, but do apply it when the current tree is
                // empty; otherwise continuous edits can starve the explorer.
                let current = this.explorer.tree_generation == generation;
                match result {
                    Ok(mut tree) => {
                        if current || this.file_tree.entries.is_empty() {
                            tree.restore_expanded(&expanded);
                            this.explorer.expand_after_scan.clear();
                            this.file_tree = tree;
                        }
                    }
                    Err(error) => {
                        if current || this.file_tree.entries.is_empty() {
                            this.file_tree.error = Some(error);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Rebuild the debounced file watcher after a project switch.
    pub(crate) fn restart_file_watcher(&mut self) {
        let project = self.project().clone();
        if self.watcher_root.as_deref() == Some(project.path.as_str())
            && (self.file_watcher.is_some() || project.is_remote())
        {
            return;
        }
        self.file_watcher = None;
        self.watcher_root = Some(project.path.clone());
        if !project.is_remote() {
            self.file_watcher = watch::ProjectWatcher::start(Path::new(&project.path));
        }
    }

    /// Poll the watcher once per second and refresh the surfaces that depend on
    /// the working tree. The watcher already debounces bursts.
    pub(crate) fn poll_file_watcher(&mut self, cx: &mut Context<Self>) {
        let changed = self
            .file_watcher
            .as_mut()
            .map(watch::ProjectWatcher::poll)
            .unwrap_or(false);
        if changed {
            self.invalidate_project_icon();
            self.refresh_file_tree(cx);
            self.refresh_git_status(cx);
        }
    }

    /// Read branch and working-tree state off the UI thread.
    pub(crate) fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        self.invalidate_impact_map();
        self.invalidate_minimap_baselines();
        let expected = self.project().path.clone();
        let project = self.project().id.clone();
        self.explorer.git_generation = self.explorer.git_generation.wrapping_add(1);
        let generation = self.explorer.git_generation;
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.git.not_repository = false;
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
                if this.project().path == expected
                    && this.project().id == project
                    && this.explorer.git_generation == generation
                {
                    match status {
                        Ok(status) => {
                            if let Some(error) = &status.error {
                                logging::warn(&format!("git status: {error}"));
                            }
                            this.git = status;
                        }
                        Err(error) => {
                            this.git.not_repository = false;
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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

        let request = uuid::Uuid::now_v7();
        let project = self.project().id.clone();
        let root = self.project().path.clone();
        self.diff_state
            .update(cx, |state, cx| state.set_files(Vec::new(), cx));
        self.diff_view = Some(DiffView {
            request,
            path: path.into(),
            name,
            staged,
            state: self.diff_state.clone(),
            error: Some("Loading changes…".into()),
        });
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let patch = backend.git_file_diff(&path, staged)?;
                    let files = DiffFile::parse(&patch)
                        .map_err(|e| format!("Could not read the patch: {e}"))?;
                    if files.is_empty() {
                        return Err("No changes to show for this side of the index".into());
                    }
                    Ok(files)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.project().id != project
                    || this.project().path != root
                    || this
                        .diff_view
                        .as_ref()
                        .is_none_or(|view| view.request != request)
                {
                    return;
                }
                match result {
                    Ok(files) => {
                        this.diff_state
                            .update(cx, |state, cx| state.set_files(files, cx));
                        if let Some(view) = &mut this.diff_view {
                            view.error = None;
                        }
                    }
                    Err(e) => {
                        if let Some(view) = &mut this.diff_view {
                            view.error = Some(e);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Ask macOS for files to reference in the next prompt.
    pub(crate) fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_picked_paths(false, window, cx);
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
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
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

    /// Jump from a trace span to the transcript entry that produced it.
    pub(crate) fn open_span_in_chat(&mut self, span_id: &str, cx: &mut Context<Self>) {
        self.set_view(MainView::Chat, cx);
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        self.run_git_task(
            "Pulling…",
            "Pulled the latest changes",
            |backend| backend.git_pull(),
            cx,
        );
    }

    pub(crate) fn git_push(&mut self, cx: &mut Context<Self>) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        let project = self.project().id.clone();
        let root = self.project().path.clone();
        self.begin_git_mutation(&project);
        self.git_loading = true;
        self.notice = Some(label);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { operation(backend) })
                .await;
            this.update(cx, |this, cx| {
                this.finish_git_mutation(&project);
                if this.project().id != project || this.project().path != root {
                    return;
                }
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

    pub(crate) fn switch_branch(
        &mut self,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker_filtered(branch, window, cx);
    }

    pub(crate) fn amend_commit(&mut self, cx: &mut Context<Self>) {
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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

    /// Switch the primary surface, restarting the title-bar animation so the
    /// active tab visibly settles into place.
    pub(crate) fn set_view(&mut self, view: MainView, cx: &mut Context<Self>) {
        self.git_history.active = false;
        if self.section == SidebarSection::Dashboard {
            self.section = SidebarSection::Projects;
        }
        self.workbench.show_file = false;
        if self.view != view {
            self.chat_menu_open = false;
            self.view = view;
            self.view_epoch = self.view_epoch.wrapping_add(1);
            self.persist_settings(cx);
        }
        self.settings_open = false;
        cx.notify();
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
            editor_minimap: self.editor_minimap,
            autosave: self.autosave,
            notifications_enabled: self.notifications_enabled,
            show_shortcut_hints: self.show_shortcut_hints,
            code_font_size: self.code_font_size,
            ui_font_family: self.ui_font_family.clone(),
            code_font_family: self.code_font_family.clone(),
            reasoning_open: self.reasoning_open,
            final_answer_only: self.final_answer_only,
            stream_responses: self.stream_responses,
            chat_timestamps: self.chat_timestamps,
            chat_duration: self.chat_duration,
            chat_logo: self.chat_logo,
            dark: cx.theme().is_dark(),
            view: self.view,
            default_view: self.default_view,
            section: self.section,
            last_project_id: self
                .projects
                .get(self.selected_project)
                .map(|project| project.id.clone()),
            last_session_id: self
                .sessions
                .get(self.selected_session)
                .map(|session| session.id.clone()),
            remote_hosts: self.remote_hosts.clone(),
            collapsed_project_groups: self.collapsed_project_groups.clone(),
            project_view: self.project_view,
            project_sort: self.project_sort,
            sidebar_width: self.sidebar_width,
            show_project_branches: self.show_project_branches,
            sidebar_collapsed: self.sidebar_collapsed,
            opencode_provider: self.opencode_provider.clone(),
            recent_models: self.recent_models.clone(),
            group_contexts: self.group_contexts.clone(),
            account_usage_collapsed: self.account_usage_collapsed,
            dashboard_settings: self.dashboard_settings.clone(),
            skills: self.skills.clone(),
            last_ai_target: self.last_ai_target.clone(),
            billing_budget_usd: self.billing_budget_usd,
            billing_alerts: self.billing_alerts,
            onboarding_step: self.onboarding_step,
            density: self.density,
            reduce_motion: self.reduce_motion,
            run_history: self.run.history.clone(),
            bottom_panel: self.bottom_panel,
            bottom_panel_height: self.bottom_panel_height,
            bottom_panel_width: self.bottom_panel_width,
            terminal_dock: self.terminal_dock,
        }
    }

    pub(crate) fn persist_settings(&self, cx: &App) {
        if let Some(store) = &self.store
            && let Err(error) = store.save_setting("app", &self.stored_settings(cx))
        {
            self.report_storage_error("save settings", error);
        }
    }

    pub(crate) fn report_storage_error(&self, operation: &str, error: impl std::fmt::Display) {
        let message = format!("Could not {operation}: {error}. Your history may not be saved.");
        logging::error(&message);
        *self.storage_error.borrow_mut() = Some(message);
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
                label: "Session context (not account allowance)".into(),
                used_percent: usage.percent() * 100.0,
                resets_in: "current turn".into(),
                source: UsageSource::Session,
            }],
        })
    }

    pub(crate) fn invalidate_usage(&mut self) {
        self.usage_cache.clear();
        self.usages.clear();
        self.usage_scope = None;
        self.usage_request = None;
        self.usage_loading = false;
        self.usage_generation = self.usage_generation.wrapping_add(1);
    }

    pub(crate) fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        let scope = self.usage_target();
        if self.usage_request.as_ref() == Some(&scope) {
            return;
        }
        self.usage_generation = self.usage_generation.wrapping_add(1);
        let generation = self.usage_generation;
        self.usage_request = None;
        self.usage_loading = false;
        if let Some(remote_project) = self.project().remote.clone() {
            let Some(host) = self.remote_host(&remote_project.host_id) else {
                return;
            };
            if crate::usage::restricted_to_admin(&host.user_role) {
                self.usage_scope = Some(scope.clone());
                self.usage_cache.record(
                    scope.clone(),
                    vec![scope.unavailable(
                        "Admin-only",
                        "Provider usage is restricted to relay administrators.",
                    )],
                );
                cx.notify();
                return;
            }
            let token = match self.remote_token(&host.id) {
                Ok(token) => token,
                Err(error) => {
                    self.set_notice(error, cx);
                    return;
                }
            };
            let host_id = host.id.clone();
            let url = host.url.clone();
            self.usage_loading = true;
            self.usage_request = Some(scope.clone());
            cx.notify();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { remote::usage(&url, &token) })
                    .await;
                this.update(cx, |this, cx| {
                    if this.usage_generation != generation {
                        return;
                    }
                    this.usage_loading = false;
                    this.usage_request = None;
                    this.usage_scope = Some(scope.clone());
                    match result {
                        Ok(usages) => {
                            this.usage_cache
                                .record(scope.clone(), crate::usage::select(&scope, &usages));
                            logging::info(&format!(
                                "remote usage refreshed · {} providers",
                                usages.len()
                            ));
                            this.remote_usages.insert(host_id, usages);
                        }
                        Err(error) => {
                            let admin_only = crate::usage::admin_only_error(&error);
                            this.usage_cache.record(
                                scope.clone(),
                                vec![scope.unavailable(
                                    if admin_only {
                                        "Admin-only"
                                    } else {
                                        "Usage request failed"
                                    },
                                    if admin_only {
                                        "Provider usage is restricted to relay administrators."
                                    } else {
                                        "Could not read relay account limits. Refresh to retry."
                                    },
                                )],
                            );
                            this.remote_usages.remove(&host_id);
                            if admin_only {
                                logging::info("remote usage: admin-only");
                            } else {
                                logging::warn(&format!("remote usage: {error}"));
                                this.set_notice(error, cx);
                            }
                        }
                    }
                    this.ensure_usage(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        self.usage_loading = true;
        self.usage_request = Some(scope.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let target = scope.clone();
            let usages = cx
                .background_executor()
                .spawn(async move { crate::usage::probe(&target) })
                .await;
            this.update(cx, |this, cx| {
                if this.usage_generation != generation {
                    return;
                }
                logging::info(&format!("usage refreshed · {} providers", usages.len()));
                this.usage_cache.record(scope.clone(), usages.clone());
                this.usages = usages;
                this.usage_loading = false;
                this.usage_scope = Some(scope);
                this.usage_request = None;
                this.ensure_usage(cx);
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
        self.open_workbench_file(path, line, window, cx);
    }

    pub(crate) fn jump_to_line(&mut self, line: u32, window: &mut Window, cx: &mut Context<Self>) {
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
        self.reload_editor_file(false, window, cx);
    }

    fn reload_editor_file(&mut self, explicit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_history.running
            || (!explicit && self.active_file_tab().is_some_and(|t| t.git_changed))
        {
            return;
        }
        if self
            .active_file_tab()
            .is_some_and(|t| t.kind != dunlin_workspace::preview::PreviewKind::Text || t.loading)
        {
            return;
        }
        if self.editor_dirty && !explicit {
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
            let expected_project = self.project().id.clone();
            let expected_mtime = self.editor_mtime;
            let expected_text = self.editor.read(cx).text().clone();
            let reload_identity = self
                .active_file_tab()
                .map(|t| (t.id.clone(), t.reload_generation));
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { backend.read_file(&path) })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    if this.git_history.running
                        || (!explicit && this.active_file_tab().is_some_and(|t| t.git_changed))
                        || this
                            .active_file_tab()
                            .map(|t| (t.id.clone(), t.reload_generation))
                            != reload_identity
                        || (!explicit && this.editor_dirty)
                        || *this.editor.read(cx).text()!=expected_text
                        || this.project().id != expected_project
                        || this.editor_mtime != expected_mtime
                        || !this
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
                        this.record_saved_editor_text(file.contents.clone());
                        this.editor.update(cx, |state, cx| {
                            if *state.text() != file.contents {
                                state.set_value(file.contents, window, cx);
                            }
                            state.set_highlighter(language.clone(), cx);
                            state.set_soft_wrap(wrap, window, cx);
                        });
                        if let Some(inspector) = this.inspector.as_mut() {
                            inspector.bytes = file.bytes as usize;
                            inspector.language = language;
                        }
                        if explicit {this.editor_conflict=None;this.finish_pending_editor(window,cx);}
                    } else if explicit {
                        this.file_notice("Could not reload the file. Its buffer, unsaved changes and Git review guard are preserved.",cx);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        let file = match self.backend().and_then(|backend| backend.read_file(&path)) {
            Ok(file) => file,
            Err(_) => {
                if explicit {
                    self.file_notice("Could not reload the file. Its buffer, unsaved changes and Git review guard are preserved.",cx);
                }
                return;
            }
        };
        if file.binary {
            return;
        }
        self.editor_mtime = Some(file.mtime_ms);
        let contents = file.contents;

        let language = editor_language(&path).to_string();
        let bytes = contents.len();
        let wrap = self.word_wrap;
        self.record_saved_editor_text(contents.clone());
        self.editor.update(cx, |state, cx| {
            if *state.text() != contents {
                state.set_value(contents, window, cx);
            }
            state.set_highlighter(language.clone(), cx);
            state.set_soft_wrap(wrap, window, cx);
        });
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.bytes = bytes;
            inspector.language = language;
        }
        if explicit {
            self.editor_conflict = None;
            self.finish_pending_editor(window, cx);
        }
    }

    /// Apply editor preferences that live in the GPUI Kit input state.
    pub(crate) fn apply_editor_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wrap = self.word_wrap;
        self.editor
            .update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
        self.editor_preview
            .update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
    }

    pub(crate) fn select_font(&mut self, family: &str, code: bool, cx: &mut Context<Self>) {
        if !self.available_fonts.iter().any(|name| name == family) {
            return;
        }
        if code {
            self.code_font_family = family.into();
        } else {
            self.ui_font_family = family.into();
        }
        dunlin_ui::theme::set_font_families(&self.ui_font_family, &self.code_font_family, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Step the code font size and apply it to editors and diffs immediately.
    pub(crate) fn adjust_code_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.code_font_size = (self.code_font_size + delta).clamp(10.0, 20.0);
        dunlin_ui::theme::set_code_font_size(self.code_font_size, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Close the inspector, discarding any unsaved editor buffer.
    /// Prompt before closing when the buffer has unsaved edits.
    pub(crate) fn request_close_inspector(&mut self, cx: &mut Context<Self>) {
        if self.git_history.is_active_for(self.project()) {
            self.close_git_history(cx);
            return;
        }
        if !self.workbench.show_file {
            return;
        }
        if self.editor_dirty {
            self.pending_editor = Some(PendingEditor::Close);
            cx.notify();
        } else {
            self.close_active_tab();
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
        if let Some(id) = self.workbench.active.take() {
            self.workbench.tabs.retain(|tab| tab.id != id);
        }
        self.editor_dirty = false;
        self.finish_pending_editor(window, cx);
    }

    fn finish_pending_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_editor.take() else {
            return;
        };
        match pending {
            PendingEditor::Close => self.close_active_tab(),
            PendingEditor::SwitchProject { id } => {
                if let Some(index) = self.projects.iter().position(|project| project.id == id) {
                    self.select_project(index, cx);
                }
            }
            PendingEditor::RemoveProject { id } => self.delete_project(&id, cx),
        }
        cx.notify();
    }

    pub(crate) fn close_inspector(&mut self) {
        self.stash_editor_tab();
        self.workbench.active = None;
        self.workbench.show_file = false;
        if self.editor_dirty {
            self.notice = Some("Unsaved changes discarded".into());
        }
        self.inspector = None;
        self.editor_dirty = false;
        self.editor_readonly = false;
        self.editor_mtime = None;
        self.editor_conflict = None;
        self.pending_editor = None;
    }

    pub(crate) fn save_editor(&mut self, cx: &mut Context<Self>) {
        self.save_editor_internal(true, self.editor_mtime, cx);
    }

    fn save_editor_internal(
        &mut self,
        announce: bool,
        expected_mtime: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_dirty {
            return;
        }
        if self.active_file_tab().is_some_and(|t| t.git_changed) && expected_mtime.is_some() {
            if announce {
                self.editor_conflict = self.active_file_tab().and_then(|t| t.conflict.clone());
                self.file_notice("Git changed this file. Reload/compare or explicitly confirm overwrite before saving.",cx);
            }
            return;
        }
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
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
        match backend.write_file(&inspector.path, &value, expected_mtime) {
            Ok(stat) => {
                self.record_saved_editor_text(value);
                self.remember_dashboard_project(cx);
                self.editor_dirty = false;
                self.editor_mtime = Some(stat.mtime_ms);
                self.editor_conflict = None;
                if announce {
                    self.notice = Some(format!("Saved {}", inspector.name));
                }
                self.refresh_git_status(cx);
            }
            Err(error) if error.starts_with("conflict:") => {
                self.editor_conflict = Some(EditorConflict {
                    path: inspector.path.clone(),
                    message: error,
                });
                self.notice = Some(format!(
                    "{} changed elsewhere · reload or overwrite",
                    inspector.name
                ));
            }
            Err(error) => {
                if announce {
                    self.notice = Some(format!("Could not save {}: {error}", inspector.name));
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_editor_conflict(&mut self, cx: &mut Context<Self>) {
        self.editor_conflict = None;
        cx.notify();
    }

    pub(crate) fn reload_editor_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_history.running {
            return;
        }
        if let Some(id) = self.workbench.active.clone()
            && let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id)
        {
            tab.reload_generation = tab.reload_generation.wrapping_add(1);
        }
        self.editor_stale = false;
        self.editor_conflict = None;
        self.reload_editor_file(true, window, cx);
        cx.notify();
    }

    pub(crate) fn overwrite_editor_conflict(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_history.running {
            return;
        }
        if self.active_file_tab().is_some_and(|t| t.git_changed) {
            self.editor_dirty = true;
            if let Some(id) = self.workbench.active.clone()
                && let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id)
            {
                tab.dirty = true;
            }
        }
        self.editor_conflict = None;
        self.save_editor_internal(true, None, cx);
        if !self.editor_dirty {
            self.finish_pending_editor(window, cx);
        }
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
                    let expected = this.editor_mtime;
                    this.save_editor_internal(false, expected, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Detect external edits to the open file and reload a clean buffer.
    pub(crate) fn check_open_file(&mut self, cx: &mut Context<Self>) {
        if self.git_history.running || self.active_file_tab().is_some_and(|t| t.git_changed) {
            return;
        }
        if self
            .active_file_tab()
            .is_some_and(|t| t.kind != dunlin_workspace::preview::PreviewKind::Text || t.loading)
        {
            return;
        }
        if self.editor_dirty || self.editor_stale || self.editor_conflict.is_some() {
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
            let reload_identity = self
                .active_file_tab()
                .map(|t| (t.id.clone(), t.reload_generation));
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { backend.stat_file(&path) })
                    .await;
                this.update(cx, |this, cx| {
                    if this.git_history.running
                        || this.active_file_tab().is_some_and(|t| t.git_changed)
                        || this
                            .active_file_tab()
                            .map(|t| (t.id.clone(), t.reload_generation))
                            != reload_identity
                        || !this
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
}

impl Render for DunlinApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_markdown_visual(window, cx);
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let tabs = &self.workbench.tabs;
        self.workbench
            .preview_cache
            .retain(|id| tabs.iter().any(|tab| tab.id == id));
        self.refresh_minimap(cx);
        self.refresh_inline_blame(cx);
        let geometry = window_state::from_bounds(window.window_bounds());
        if self.last_window_geometry != Some(geometry) {
            self.last_window_geometry = Some(geometry);
            window_state::save(geometry);
        }
        if self.editor_stale
            && !self.git_history.running
            && !self.active_file_tab().is_some_and(|t| t.git_changed)
        {
            self.editor_stale = false;
            self.reload_open_editor(window, cx);
        }
        let waiting = self
            .all_sessions
            .iter()
            .filter(|session| session.status == SessionStatus::Waiting)
            .count();
        let badge = (waiting > 0).then(|| waiting.to_string());
        if self.last_dock_badge != badge {
            crate::platform::set_dock_badge(badge.as_deref());
            self.last_dock_badge = badge;
        }
        let p = palette(cx);
        let show_sidebar = !self.sidebar_collapsed && !self.projects.is_empty();
        let (sidebar_fraction, moving) = self.sidebar_motion.sample(show_sidebar);
        if moving {
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        }
        let titlebar = views::titlebar::render(self, window, cx);
        let rail = self.rail(cx);
        let sidebar = views::sidebar::render(self, window, cx);
        let main_panel = self.main_panel(window, cx);
        let dock =
            (self.bottom_panel != BottomPanel::Hidden).then(|| self.bottom_panel_view(window, cx));
        let dock_position = self.terminal_dock;
        let workspace = match (dock, dock_position) {
            (Some(dock), TerminalDock::Bottom) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .child(main_panel)
                .child(dock)
                .into_any_element(),
            (Some(dock), TerminalDock::Left) => div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .child(dock)
                .child(main_panel)
                .into_any_element(),
            (Some(dock), TerminalDock::Right) => div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .child(main_panel)
                .child(dock)
                .into_any_element(),
            (None, _) => main_panel,
        };
        let status_bar = if self.projects.is_empty() {
            None
        } else {
            Some(self.status_bar(cx))
        };
        let detached = crate::overlay_window::prepare(self, window, cx);
        let overlays = (!detached && self.has_modal_overlay())
            .then(|| self.render_overlay_content(window, cx));
        let toast = (!self.has_modal_overlay())
            .then(|| self.toast.as_ref().map(|_| self.toast_overlay(cx)))
            .flatten();

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family(dunlin_ui::theme::ui_font())
            .child(self.overlay_backdrop.begin())
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Settings, _, cx| {
                    if !this.dismiss_history_for_menu(cx) {
                        return;
                    }
                    this.branch_ui.dismiss();
                    this.settings_open = true;
                    this.settings_tab = SettingsTab::General;
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::native_menu::OpenFolders, _, cx| {
                    this.open_folder(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::native_menu::Home, _, cx| {
                if !this.dismiss_history_for_menu(cx) {
                    return;
                }
                this.section = SidebarSection::Dashboard;
                this.settings_open = false;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Projects, _, cx| {
                    if !this.dismiss_history_for_menu(cx) {
                        return;
                    }
                    this.section = SidebarSection::Projects;
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::native_menu::ToggleSidebar, _, cx| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                    this.persist_settings(cx);
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Commands, window, cx| {
                    if !this.dismiss_history_for_menu(cx) {
                        return;
                    }
                    this.open_palette(window, cx)
                }),
            )
            .when(!self.projects.is_empty(), |root| {
                root.on_action(
                    cx.listener(|this, _: &crate::native_menu::NewSession, _, cx| {
                        if !this.dismiss_history_for_menu(cx) {
                            return;
                        }
                        this.new_session(cx)
                    }),
                )
                .on_action(cx.listener(|this, _: &crate::native_menu::SaveAll, _, cx| {
                    this.save_all_tabs(cx);
                }))
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::ProjectSettings, _, cx| {
                        if !this.dismiss_history_for_menu(cx) {
                            return;
                        }
                        this.open_project_settings(this.selected_project, cx)
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::SwitchBranch, window, cx| {
                        if !this.dismiss_history_for_menu(cx) {
                            return;
                        }
                        this.open_branch_picker(&this.project().id.clone(), window, cx)
                    }),
                )
                .on_action(cx.listener(
                    |this, _: &crate::native_menu::RefreshGit, _, cx| this.refresh_git_status(cx),
                ))
            })
            .when(
                !self.projects.is_empty() && self.project().trusted,
                |root| {
                    root.on_action(cx.listener(
                        |this, _: &crate::native_menu::GitHistory, window, cx| {
                            this.open_git_history(window, cx)
                        },
                    ))
                },
            )
            .capture_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some((start, initial)) = this.sidebar_resize {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.finish_sidebar_resize(cx);
                        return;
                    }
                    this.sidebar_width = crate::sidebar_layout::width(
                        initial + f32::from(event.position.x) - start,
                        f32::from(window.viewport_size().width),
                    );
                    cx.notify();
                }
                if let Some((start, initial)) = this.terminal_resize {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.finish_terminal_resize(cx);
                        return;
                    }
                    let viewport_width = f32::from(window.viewport_size().width);
                    let viewport_height = f32::from(window.viewport_size().height);
                    match this.terminal_dock {
                        crate::app::TerminalDock::Bottom => {
                            this.bottom_panel_height = (initial + start
                                - f32::from(event.position.y))
                            .clamp(160., (viewport_height * 0.62).max(160.));
                        }
                        crate::app::TerminalDock::Left => {
                            this.bottom_panel_width = (initial + f32::from(event.position.x)
                                - start)
                                .clamp(260., (viewport_width * 0.55).max(260.));
                        }
                        crate::app::TerminalDock::Right => {
                            this.bottom_panel_width = (initial
                                - (f32::from(event.position.x) - start))
                                .clamp(260., (viewport_width * 0.55).max(260.));
                        }
                    }
                    cx.notify();
                }
                if this.terminal_drag {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.finish_terminal_resize(cx);
                        return;
                    }
                    let viewport_width = f32::from(window.viewport_size().width);
                    let viewport_height = f32::from(window.viewport_size().height);
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let target = if y > viewport_height * 0.78 {
                        crate::app::TerminalDock::Bottom
                    } else if x < viewport_width * 0.18 {
                        crate::app::TerminalDock::Left
                    } else if x > viewport_width * 0.82 {
                        crate::app::TerminalDock::Right
                    } else {
                        this.terminal_dock
                    };
                    if target != this.terminal_dock {
                        this.terminal_dock = target;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.finish_terminal_resize(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.finish_terminal_resize(cx);
                }),
            )
            .child(titlebar)
            .children(self.storage_error.borrow().as_ref().map(|message| {
                div()
                    .px(px(16.))
                    .py(px(8.))
                    .bg(p.red_soft)
                    .text_color(p.red)
                    .text_size(px(12.))
                    .child(message.clone())
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .child(rail)
                    .when(sidebar_fraction > 0., |this| {
                        this.child(
                            div()
                                .w(px(crate::sidebar_layout::width(
                                    self.sidebar_width,
                                    f32::from(window.viewport_size().width),
                                ) * sidebar_fraction))
                                .h_full()
                                .flex_shrink_0()
                                .overflow_hidden()
                                .child(sidebar),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .border_l_1()
                            .border_color(p.border)
                            .child(workspace),
                    ),
            )
            .children(status_bar)
            .children(overlays)
            .children(toast)
            .child(self.overlay_backdrop.finish())
    }
}

impl Drop for DunlinApp {
    fn drop(&mut self) {
        self.save_and_shutdown_background_agents();
        if self.harness.running
            && let Some(adapter) = &self.harness.harness
        {
            adapter.cancel();
        }
        if !self.projects.is_empty() {
            self.save_current_transcript();
        }
        self.save_stashed_transcripts();
    }
}

/// Ask before closing the window while an ACP turn is still streaming. The
/// turn cannot survive the process, so the only safe options are to wait or to
/// cancel explicitly.
pub(crate) fn install_close_guard(window: &mut Window, cx: &mut Context<DunlinApp>) {
    let handle = cx.entity().downgrade();
    let confirmed = Rc::new(Cell::new(false));
    let prompting = Rc::new(Cell::new(false));
    let guard = Rc::new(RefCell::new(move |window: &mut Window, cx: &mut App| {
        let (dirty, busy, checkout) = handle
            .read_with(cx, |this, _| {
                (
                    this.has_dirty_tabs(),
                    this.has_running_agents(),
                    this.branch_ui.running() || this.git_history.running,
                )
            })
            .unwrap_or((false, false, false));
        if checkout {
            let _ = handle.update(cx, |this, cx| {
                this.file_notice(
                    "Wait for the repository operation to finish before quitting",
                    cx,
                )
            });
            return false;
        }
        if (!dirty && !busy) || confirmed.get() {
            return true;
        }
        if prompting.replace(true) {
            return false;
        }

        let receiver = window.prompt(
            PromptLevel::Warning,
            if dirty {
                "You have unsaved changes"
            } else {
                "An agent turn is still running"
            },
            Some(if busy {
                "Quitting cancels the running agent turn. Save any edited file before quitting."
            } else {
                "Save your edited file before quitting? Discarding cannot be undone."
            }),
            if dirty {
                &["Save and Quit", "Discard and Quit", "Cancel"]
            } else {
                &["Keep Running", "Quit Anyway"]
            },
            cx,
        );
        let backdrop = handle
            .read_with(cx, |this, _| this.overlay_backdrop.clone())
            .unwrap_or_default();
        let receiver = backdrop.track_native(receiver, cx);
        let confirmed = confirmed.clone();
        let prompting = prompting.clone();
        let handle = handle.clone();
        cx.spawn(async move |cx| {
            let answer = receiver.await;
            prompting.set(false);
            let quit = if dirty && answer == Ok(0) {
                handle
                    .update(cx, |this, cx| this.save_all_tabs(cx))
                    .unwrap_or(false)
            } else {
                answer == Ok(1)
            };
            if quit {
                if handle
                    .read_with(cx, |this, _| {
                        this.branch_ui.running() || this.git_history.running
                    })
                    .unwrap_or(false)
                {
                    return;
                }
                confirmed.set(true);
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
        false
    }));
    let close_guard = guard.clone();
    window.on_window_should_close(cx, move |window, cx| (close_guard.borrow_mut())(window, cx));
    let window_handle = window.window_handle();
    App::on_action(cx, move |_: &Quit, cx| {
        let result = window_handle.update(cx, |_, window, cx| {
            if (guard.borrow_mut())(window, cx) {
                cx.quit();
            }
        });
        if result.is_err() {
            // The last window may already have been closed cleanly.
            cx.quit();
        }
    });
}

#[cfg(test)]
mod tests;
