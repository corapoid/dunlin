use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dunlin_domain::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::diff::{Diff, DiffFile, DiffState};
use dunlin_ui::component::input::{
    Editor, EditorState, InputEvent, InputState, Position, TextareaState,
};
use dunlin_ui::component::{ActiveTheme, Icon, ThemeMode};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use serde::{Deserialize, Serialize};

use crate::{harness, logging, projection, remote, views, watch, window_state};
use dunlin_workspace::tree::FileTree;
use dunlin_workspace::{git, search};

actions!(dunlin, [Quit]);

/// How many transcript items are laid out at once. Long imported sessions grow
/// this window on demand instead of building thousands of views up front.
pub(crate) const CHAT_WINDOW: usize = 120;

mod helpers;
mod navigation;
mod overlays;
pub(crate) use helpers::*;

fn is_composer_submit(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { shift: false, .. })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Design,
    Sidebar,
    Chat,
    Harnesses,
    Skills,
    Security,
    Secrets,
    Extensions,
    Permissions,
    Remote,
    Editor,
    Usage,
    Data,
    Advanced,
}

impl SettingsTab {
    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Design => "Appearance",
            SettingsTab::Sidebar => "Workspace",
            SettingsTab::Chat => "Chat",
            SettingsTab::Harnesses => "Agents",
            SettingsTab::Skills => "Skills",
            SettingsTab::Security => "Security Audits",
            SettingsTab::Secrets => "Credentials",
            SettingsTab::Extensions => "Extensions",
            SettingsTab::Permissions => "Permissions",
            SettingsTab::Remote => "Remote · Tailscale",
            SettingsTab::Editor => "Editor & Terminal",
            SettingsTab::Usage => "Usage & Limits",
            SettingsTab::Data => "Data & Backup",
            SettingsTab::Advanced => "Diagnostics & About",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            SettingsTab::General => IconName::Settings2,
            SettingsTab::Design => IconName::Sun,
            SettingsTab::Sidebar => IconName::PanelLeft,
            SettingsTab::Chat => IconName::MessageSquare,
            SettingsTab::Harnesses => IconName::Bot,
            SettingsTab::Skills => IconName::BookOpen,
            SettingsTab::Security => IconName::ShieldCheck,
            SettingsTab::Secrets => IconName::KeyRound,
            SettingsTab::Extensions => IconName::Puzzle,
            SettingsTab::Permissions => IconName::Lock,
            SettingsTab::Remote => IconName::Cloud,
            SettingsTab::Editor => IconName::Code,
            SettingsTab::Usage => IconName::ChartNoAxesColumn,
            SettingsTab::Data => IconName::Database,
            SettingsTab::Advanced => IconName::Info,
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
pub(crate) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    fn native(self) -> Option<WindowAppearance> {
        match self {
            Self::System => None,
            Self::Light => Some(WindowAppearance::Light),
            Self::Dark => Some(WindowAppearance::Dark),
        }
    }
    pub(crate) fn mode(self, system: WindowAppearance) -> ThemeMode {
        match self {
            Self::System => system.into(),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        }
    }
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
    Secrets,
    Agents,
}

/// Everything Dunlin remembers between launches. Stored as one JSON value in
/// SQLite so new fields can be added without another migration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredSettings {
    pub default_harness: String,
    pub hidden_harnesses: Vec<String>,
    pub custom_harness_command: String,
    pub permission_mode: PermissionMode,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub editor_minimap: bool,
    pub inline_blame: bool,
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
    #[serde(default)]
    pub appearance: Option<Appearance>,
    pub skin: dunlin_ui::Skin,
    pub view: MainView,
    pub section: SidebarSection,
    #[serde(default)]
    pub sidebar_tab: Option<crate::sidebar_layout::Tab>,
    pub sidebar_collapsed: bool,
    pub workspace_navigation_collapsed: bool,
    pub general_sessions_collapsed: bool,
    pub project_sessions_collapsed: bool,
    pub notifications_enabled: bool,
    pub show_shortcut_hints: bool,
    pub show_file_sizes: bool,
    pub last_project_id: Option<String>,
    pub last_session_id: Option<String>,
    pub remote_hosts: Vec<remote::RemoteHost>,
    pub watched_project_folders: Vec<PathBuf>,
    pub collapsed_project_groups: Vec<String>,
    pub project_sort: crate::project_tree::Sort,
    pub project_group_order: Vec<String>,
    pub sidebar_width: f32,
    pub show_project_branches: bool,
    pub project_list: crate::project_catalog::Preferences,
    pub opencode_provider: Option<String>,
    pub opencode_free_consent: bool,
    pub recent_models: Vec<crate::providers::RecentModel>,
    pub group_contexts: HashMap<String, crate::project_groups::GroupContext>,
    pub account_usage_collapsed: bool,
    pub dashboard_settings: crate::dashboard::Settings,
    pub skills: Vec<crate::skills::Skill>,
    pub conversation_skills: HashMap<String, Vec<String>>,
    #[serde(default = "default_security_enabled")]
    pub security_enabled: bool,
    #[serde(default)]
    pub security_triage: Vec<crate::security::TriageRecord>,
    #[serde(default = "default_vault_enabled")]
    pub vault_enabled: bool,
    pub last_ai_target: Option<crate::ai_commit::LastTarget>,
    pub billing_budget_usd: Option<f32>,
    pub billing_alerts: bool,
    pub onboarding_step: u8,
    pub pending_adapter_install: Option<crate::onboarding::Plan>,
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
    #[serde(default = "default_shell_integration")]
    pub shell_integration: bool,
    #[serde(default, skip_serializing, rename = "terminal_ssh_target")]
    pub _legacy_terminal_ssh_target: String,
    #[serde(default, skip_serializing, rename = "terminal_ssh_path")]
    pub _legacy_terminal_ssh_path: String,
    /// Bundled packs are enabled by default; external packs become enabled only
    /// after the user toggles them. This list is the explicit opt-out.
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
    /// External packs require an explicit opt-in. Bundled packs ignore this
    /// list because they are enabled unless disabled above.
    #[serde(default)]
    pub enabled_plugins: Vec<String>,
    /// Per-project overrides. Missing ids inherit the global choice.
    #[serde(default)]
    pub project_disabled_plugins: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub project_enabled_plugins: HashMap<String, Vec<String>>,
    /// Global switch for the spec workspace, intent capture, agent contract
    /// and related commands. Artifacts on disk are never deleted.
    #[serde(default = "default_spec_tools_enabled")]
    pub spec_tools_enabled: bool,
}

pub(crate) fn default_spec_tools_enabled() -> bool {
    true
}

pub(crate) fn default_shell_integration() -> bool {
    true
}

pub(crate) fn default_vault_enabled() -> bool {
    true
}

pub(crate) fn default_security_enabled() -> bool {
    true
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
            hidden_harnesses: Vec::new(),
            custom_harness_command: "your-agent --acp".into(),
            permission_mode: PermissionMode::Ask,
            auto_connect: true,
            restart_harness: true,
            restore_session: true,
            word_wrap: false,
            editor_minimap: true,
            inline_blame: false,
            autosave: false,
            notifications_enabled: true,
            show_shortcut_hints: true,
            show_file_sizes: true,
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
            appearance: Some(Appearance::System),
            skin: dunlin_ui::Skin::Default,
            view: MainView::Chat,
            section: SidebarSection::Files,
            sidebar_tab: None,
            sidebar_collapsed: false,
            workspace_navigation_collapsed: false,
            general_sessions_collapsed: false,
            project_sessions_collapsed: false,
            last_project_id: None,
            last_session_id: None,
            remote_hosts: Vec::new(),
            watched_project_folders: Vec::new(),
            collapsed_project_groups: Vec::new(),
            project_sort: Default::default(),
            project_group_order: Vec::new(),
            sidebar_width: crate::sidebar_layout::DEFAULT_WIDTH,
            show_project_branches: true,
            project_list: Default::default(),
            opencode_provider: Some(crate::providers::GO.into()),
            opencode_free_consent: false,
            recent_models: Vec::new(),
            group_contexts: HashMap::new(),
            account_usage_collapsed: false,
            dashboard_settings: Default::default(),
            skills: crate::skills::defaults(),
            conversation_skills: HashMap::new(),
            security_enabled: true,
            security_triage: Vec::new(),
            vault_enabled: true,
            last_ai_target: None,
            billing_budget_usd: None,
            billing_alerts: true,
            onboarding_step: 0,
            pending_adapter_install: None,
            density: UiDensity::default(),
            reduce_motion: false,
            run_history: Vec::new(),
            bottom_panel: BottomPanel::Hidden,
            bottom_panel_height: default_bottom_panel_height(),
            bottom_panel_width: default_bottom_panel_width(),
            terminal_dock: TerminalDock::Bottom,
            shell_integration: default_shell_integration(),
            _legacy_terminal_ssh_target: String::new(),
            _legacy_terminal_ssh_path: String::new(),
            disabled_plugins: Vec::new(),
            enabled_plugins: Vec::new(),
            project_disabled_plugins: HashMap::new(),
            project_enabled_plugins: HashMap::new(),
            spec_tools_enabled: true,
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

#[derive(Clone, Debug)]
pub(crate) enum ContextTarget {
    Project(usize),
    Session(String),
    Tab(crate::workspace_panes::TabKey),
}

#[derive(Clone, Debug)]
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
    NewChat,
    NewChatInRepo,
    NewChatInProject(usize),
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
    ChooseBranch,
    ToggleTerminal,
    ToggleRun,
    Impact,
    Compare,
    Spec,
    CaptureIntent,
    AgentContract,
    Subtasks,
    Budgets,
    Coordination,
    Checks,
    Context,
    Review,
    SaveClipboardArtifact,
    SaveFileArtifact,
    Pack(usize),
    SwitchProject(usize),
    SearchResult(usize),
    Artifacts,
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
    pub navigation: navigation::History,
    pub sidebar_tab: crate::sidebar_layout::Tab,
    pub global_search: crate::global_search::State,
    pub workspace_tools: crate::workspace_tools::State,
    pub project_search: crate::project_search::State,
    pub section: SidebarSection,
    pub projects: Vec<Project>,
    pub local_import_running: bool,
    pub project_import: crate::project_import::State,
    pub backup: crate::backup::State,
    pub watched_project_folders: Vec<PathBuf>,
    pub session_list: crate::session_list::State,
    pub user_profile: crate::user_profile::State,
    pub connection_map: crate::connection_map::State,
    pub remote_connection: crate::remote_connection::State,
    pub group_cleanup: crate::group_cleanup::State,
    pub session_import: crate::session_import::State,
    pub fallback_project: Project,
    pub project_groups: Vec<ProjectGroup>,
    pub collapsed_project_groups: Vec<String>,
    pub project_sort: crate::project_tree::Sort,
    pub project_group_order: Vec<String>,
    pub editing_project_tree: bool,
    pub sidebar_width: f32,
    pub show_project_branches: bool,
    pub project_list: crate::project_catalog::Preferences,
    pub project_catalog: crate::project_catalog::State,
    pub sidebar_resize: Option<(f32, f32)>,
    pub project_group_editor: Option<usize>,
    pub group_contexts: HashMap<String, crate::project_groups::GroupContext>,
    pub pending_group_context: Option<crate::project_groups::GroupContext>,
    pub repo_summaries: crate::project_groups::RepoSummaries,
    pub branch_ui: crate::branch_picker::State,
    pub pending_git_action: Option<crate::git_actions::Pending>,
    pub project_sessions_running_only: bool,
    pub git_history: crate::git_history::State,
    pub spec: crate::spec::State,
    pub knowledge: crate::knowledge::State,
    pub plugins: crate::plugins::State,
    pub task_budgets: crate::task_budgets::State,
    pub coordination: crate::coordination::State,
    pub subtask_worktrees: crate::subtask_worktrees::State,
    pub comparisons: crate::comparisons::State,
    pub overlay_window: crate::overlay_window::State,
    pub project_icons: crate::project_icons::State,
    pub parked_runtimes: HashMap<String, harness::ParkedRuntime>,
    pub project_session_selections: HashMap<String, String>,
    navigation_generation: u64,
    pub mcp_operations: std::collections::HashSet<String>,
    pub dashboard: crate::dashboard::State,
    pub skills: Vec<crate::skills::Skill>,
    pub conversation_skills: HashMap<String, Vec<String>>,
    pub security_enabled: bool,
    pub skills_ui: crate::skills::Ui,
    pub skills_import_url: Entity<InputState>,
    pub security_check_argv: Entity<InputState>,
    pub security: crate::security::State,
    pub vault: crate::vault::State,
    pub last_ai_target: Option<crate::ai_commit::LastTarget>,
    pub ai_commit: crate::ai_commit::State,
    pub billing: Option<dunlin_usage::billing::Summary>,
    pub billing_ranges: Vec<dunlin_usage::billing::Summary>,
    pub billing_range: dunlin_usage::billing::TimeRange,
    pub billing_loading: bool,
    pub billing_error: Option<String>,
    pub billing_checked: Option<std::time::Instant>,
    pub billing_budget_usd: Option<f32>,
    pub billing_alerts: bool,
    pub billing_sync: Vec<crate::usage::BillingSync>,
    pub billing_syncing: bool,
    pub remote_billing: Vec<crate::remote::RemoteBillingEvidence>,
    pub remote_billing_loading: bool,
    pub remote_billing_error: Option<String>,
    pub onboarding_step: u8,
    pub onboarding_show_all_harnesses: bool,
    pub onboarding: crate::onboarding::State,
    pub harness_setup: Option<String>,
    pub harness_setup_notice: Option<String>,
    pub harness_scanning: bool,
    pub harness_checks: HashMap<u64, harness::HarnessStatus>,
    pub permissions_refreshing: bool,
    pub factory_resetting: bool,
    pub density: UiDensity,
    pub appearance: Appearance,
    pub skin: dunlin_ui::Skin,
    skin_window_background: Option<WindowBackgroundAppearance>,
    pub settings_offset: Point<Pixels>,
    pub settings_drag: Option<(Point<Pixels>, Point<Pixels>)>,
    pub reduce_motion: bool,
    pub acp_log_text: Option<String>,
    pub acp_log_error: Option<String>,
    pub acp_log_loading: bool,
    pub acp_log_exporting: bool,
    pub qa_report: Option<String>,
    pub qa_error: Option<String>,
    pub terminal: crate::terminal::State,
    pub run: crate::run::State,
    pub shell_integration: bool,
    pub bottom_panel: BottomPanel,
    pub bottom_panel_height: f32,
    pub bottom_panel_width: f32,
    pub terminal_dock: TerminalDock,
    pub terminal_resize: Option<(f32, f32)>,
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
    /// Tab stop and active-descendant owner for the graph tree. Keeping
    /// one composite focus handle means arrow keys never steal the search box.
    pub activity_focus: FocusHandle,
    pub command_open: bool,
    pub command_epoch: u64,
    pub command_query: Entity<InputState>,
    pub command_projects_only: bool,
    pub command_text: String,
    pub command_index: usize,
    pub command_scroll: ScrollHandle,
    _command_subscription: Subscription,

    pub settings_open: bool,
    pub settings_ui: views::settings::State,
    pub settings_tab: SettingsTab,
    pub default_harness: String,
    pub hidden_harnesses: Vec<String>,
    pub custom_harness_command: String,
    pub permission_mode: PermissionMode,
    pub project_menu_open: bool,
    pub project_creation: crate::project_creation::State,
    pub sidebar_collapsed: bool,
    pub workspace_navigation_collapsed: bool,
    pub general_sessions_collapsed: bool,
    pub project_sessions_collapsed: bool,
    pub pending_delete: Option<PendingDelete>,
    pub diff_view: Option<DiffView>,
    pub auto_connect: bool,
    pub restart_harness: bool,
    pub restore_session: bool,
    pub word_wrap: bool,
    pub editor_minimap: bool,
    pub inline_blame: bool,
    pub autosave: bool,
    pub notifications_enabled: bool,
    pub show_shortcut_hints: bool,
    pub show_file_sizes: bool,
    pub code_font_size: f32,
    pub ui_font_family: String,
    pub code_font_family: String,
    pub available_fonts: Vec<String>,
    pub remote_hosts: Vec<remote::RemoteHost>,
    pub remote_tokens: HashMap<String, String>,
    pub server_panel: crate::server_panel::State,
    pub remote_sessions_loading: bool,
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
    pub selected_node: Option<String>,
    pub activity: crate::activity::State,
    pub reasoning_open: bool,
    pub final_answer_only: bool,
    pub stream_responses: bool,
    pub chat_timestamps: bool,
    pub chat_duration: bool,
    pub chat_logo: bool,
    pub expanded_tool: Option<usize>,
    pub chat_visible_items: usize,
    pub chat_scroll: crate::chat_scroll::ChatScroll,

    pub file_tree: FileTree,
    file_watcher: Option<watch::ProjectWatcher>,
    watcher_root: Option<String>,
    pub search_hits: Vec<search::SearchHit>,
    pub search_query: String,
    pub search_loading: bool,
    pub notice: Option<String>,

    pub git: git::GitStatus,
    pub git_loading: bool,
    pub source_control: crate::source_control::State,
    pub commit_input: Entity<InputState>,

    pub store: Option<dunlin_storage::Store>,
    pub storage_error: RefCell<Option<String>>,
    pub harness: harness::HarnessState,
    pub harness_discovery: Vec<crate::harness_discovery::Detected>,
    pub provider_catalog: crate::providers::Catalog,
    pub mcp: crate::mcp::Catalog,
    pub provider_auth: Option<crate::provider_auth::AuthDialog>,
    pub free_endpoint_dialog: Option<crate::free_endpoint_consent::Dialog>,
    pub opencode_free_consent: bool,
    pub opencode_provider: Option<String>,
    pub recent_models: Vec<crate::providers::RecentModel>,
}

impl DunlinApp {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        views::chat_controls::init_mode_keys(cx);
        // The kit root paints an opaque theme surface unless explicitly overridden.
        window.defer(cx, |window, cx| {
            dunlin_ui::component::Root::update(window, cx, |root, _, cx| {
                root.style().background = Some(transparent_black().into());
                cx.notify();
            });
        });
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
        let vault_start = stored.vault_enabled && store.is_some();
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
            kind: Default::default(),
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
                .placeholder("Ask…")
                .rows(2)
                .submit_on_enter(true)
        });
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::Focus | InputEvent::Blur | InputEvent::Change
                ) {
                    // The compound composer owns its frame and send-action state.
                    cx.notify();
                }
                if is_composer_submit(event) && this.composer == *input {
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
                        total_size: 0,
                    }
                } else {
                    FileTree::load(project.path.clone())
                }
            })
            .unwrap_or_else(|| FileTree {
                root: String::new(),
                entries: Vec::new(),
                error: None,
                total_size: 0,
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
                .placeholder("Search graph…")
                .clean_on_escape()
        });
        let activity_search_subscription = cx.subscribe_in(
            &activity_search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.activity_query = this.activity_search.read(cx).value().to_string();
                    this.activity_normalize_selection(cx);
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
            InputState::new(window, cx).placeholder("Type a command or search the workspace…")
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
        let skills_import_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/owner/repo/tree/main/skills/my-skill")
        });
        let security_check_argv = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("JSON argv, e.g. [\"cargo\",\"test\",\"--quiet\"]")
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
                    this.command_scroll.set_offset(point(px(0.), px(0.)));
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
        let activity_focus = cx.focus_handle();
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
        let task_budgets = crate::task_budgets::load(store.as_ref());
        let subtask_worktrees = crate::subtask_worktrees::State::load(store.as_ref());
        let comparisons = crate::comparisons::State::load(store.as_ref());

        let mut app = Self {
            view: stored.view,
            view_epoch: 0,
            navigation: Default::default(),
            sidebar_tab: stored.sidebar_tab.unwrap_or_else(|| {
                if projects.is_empty() {
                    crate::sidebar_layout::Tab::Projects
                } else {
                    crate::sidebar_layout::Tab::from_section(stored.section)
                }
            }),
            global_search: Default::default(),
            workspace_tools: Default::default(),
            project_search: Default::default(),
            section: if stored.dashboard_settings.start_at_dashboard {
                SidebarSection::Dashboard
            } else if stored.section == SidebarSection::Dashboard
                || stored.section == SidebarSection::Security
            {
                SidebarSection::Projects
            } else {
                stored.section
            },
            projects,
            local_import_running: false,
            project_import: Default::default(),
            backup: Default::default(),
            watched_project_folders: stored.watched_project_folders.clone(),
            session_list: Default::default(),
            user_profile: Default::default(),
            connection_map: Default::default(),
            remote_connection: Default::default(),
            group_cleanup: Default::default(),
            session_import: crate::session_import::State::load(store.as_ref()),
            fallback_project,
            project_groups,
            collapsed_project_groups: stored.collapsed_project_groups.clone(),
            project_sort: stored.project_sort,
            project_group_order: stored.project_group_order.clone(),
            editing_project_tree: false,
            sidebar_width: stored.sidebar_width,
            show_project_branches: stored.show_project_branches,
            project_list: stored.project_list,
            project_catalog: Default::default(),
            sidebar_resize: None,
            project_group_editor: None,
            group_contexts: stored.group_contexts.clone(),
            pending_group_context: None,
            repo_summaries: Default::default(),
            branch_ui: Default::default(),
            pending_git_action: None,
            project_sessions_running_only: false,
            git_history: Default::default(),
            spec: Default::default(),
            knowledge: Default::default(),
            plugins: crate::plugins::State::new(
                stored.disabled_plugins.clone(),
                stored.enabled_plugins.clone(),
                stored.project_disabled_plugins.clone(),
                stored.project_enabled_plugins.clone(),
            )
            .with_legacy_spec_tools(stored.spec_tools_enabled),
            task_budgets,
            coordination: crate::coordination::State::start(),
            subtask_worktrees,
            comparisons,
            overlay_window: Default::default(),
            project_icons: Default::default(),
            parked_runtimes: HashMap::new(),
            project_session_selections: HashMap::new(),
            navigation_generation: 0,
            mcp_operations: Default::default(),
            dashboard: Default::default(),
            skills: stored.skills.clone(),
            conversation_skills: stored.conversation_skills.clone(),
            security_enabled: stored.security_enabled,
            skills_ui: Default::default(),
            skills_import_url,
            security_check_argv,
            security: crate::security::State {
                triage: stored.security_triage.clone(),
                ..Default::default()
            },
            vault: crate::vault::State::start(vault_start),
            last_ai_target: stored.last_ai_target.clone(),
            ai_commit: Default::default(),
            billing: None,
            billing_ranges: Vec::new(),
            billing_range: Default::default(),
            billing_loading: false,
            billing_error: None,
            billing_checked: None,
            billing_budget_usd: stored.billing_budget_usd,
            billing_alerts: stored.billing_alerts,
            billing_sync: Vec::new(),
            billing_syncing: false,
            remote_billing: Vec::new(),
            remote_billing_loading: false,
            remote_billing_error: None,
            acp_log_text: None,
            acp_log_error: None,
            acp_log_loading: false,
            acp_log_exporting: false,
            qa_report: None,
            qa_error: None,
            onboarding_step: stored.onboarding_step,
            onboarding_show_all_harnesses: false,
            onboarding: crate::onboarding::State::restore(stored.pending_adapter_install.clone()),
            harness_setup: None,
            harness_setup_notice: None,
            harness_scanning: false,
            harness_checks: HashMap::new(),
            permissions_refreshing: false,
            factory_resetting: false,
            density: stored.density,
            skin: stored.skin,
            skin_window_background: None,
            appearance: stored.appearance.unwrap_or(if stored.dark {
                Appearance::Dark
            } else {
                Appearance::Light
            }),
            settings_offset: point(px(0.), px(0.)),
            settings_drag: None,
            reduce_motion: stored.reduce_motion,
            terminal: crate::terminal::State::new(cx),
            run: crate::run::State::new(stored.run_history.clone()),
            bottom_panel: stored.bottom_panel,
            bottom_panel_height: stored.bottom_panel_height,
            bottom_panel_width: stored.bottom_panel_width,
            terminal_dock: stored.terminal_dock,
            shell_integration: stored.shell_integration,
            terminal_resize: None,
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
            activity_focus,
            command_open: false,
            command_epoch: 0,
            command_query,
            command_projects_only: false,
            command_text: String::new(),
            command_index: 0,
            command_scroll: ScrollHandle::new(),
            _command_subscription: command_subscription,
            settings_open: false,
            settings_ui: views::settings::State::default(),
            settings_tab: SettingsTab::Harnesses,
            default_harness: stored.default_harness.clone(),
            // The agent catalog no longer has a visibility preference. Restore
            // previously hidden installed adapters to the picker on upgrade.
            hidden_harnesses: Vec::new(),
            custom_harness_command: stored.custom_harness_command.clone(),
            permission_mode: stored.permission_mode,
            project_menu_open: false,
            project_creation: Default::default(),
            sidebar_collapsed: stored.sidebar_collapsed,
            workspace_navigation_collapsed: stored.workspace_navigation_collapsed,
            general_sessions_collapsed: stored.general_sessions_collapsed,
            project_sessions_collapsed: stored.project_sessions_collapsed,
            pending_delete: None,
            diff_view: None,
            auto_connect: stored.auto_connect,
            restart_harness: stored.restart_harness,
            restore_session: stored.restore_session,
            word_wrap: stored.word_wrap,
            editor_minimap: stored.editor_minimap,
            inline_blame: stored.inline_blame,
            autosave: stored.autosave,
            notifications_enabled: stored.notifications_enabled,
            show_shortcut_hints: stored.show_shortcut_hints,
            show_file_sizes: stored.show_file_sizes,
            code_font_size: stored.code_font_size,
            ui_font_family,
            code_font_family,
            available_fonts,
            remote_hosts: stored.remote_hosts.clone(),
            server_panel: Default::default(),
            remote_tokens: HashMap::new(),
            remote_sessions_loading: false,
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
            selected_node: None,
            activity: Default::default(),
            reasoning_open: stored.reasoning_open,
            final_answer_only: stored.final_answer_only,
            stream_responses: stored.stream_responses,
            chat_timestamps: stored.chat_timestamps,
            chat_duration: stored.chat_duration,
            chat_logo: stored.chat_logo,
            expanded_tool: None,
            chat_visible_items: CHAT_WINDOW,
            chat_scroll: crate::chat_scroll::ChatScroll::default(),
            file_tree,
            file_watcher: None,
            watcher_root: None,
            search_hits,
            search_query: String::new(),
            search_loading: false,
            notice: None,
            git: git::GitStatus::default(),
            git_loading: false,
            source_control: Default::default(),
            commit_input,
            store,
            storage_error: RefCell::new(storage_error),
            harness: harness::HarnessState::default(),
            harness_discovery: crate::harness_discovery::discover(),
            provider_catalog: crate::providers::Catalog::default(),
            mcp: crate::mcp::Catalog::default(),
            provider_auth: None,
            free_endpoint_dialog: None,
            opencode_free_consent: stored.opencode_free_consent,
            opencode_provider: stored.opencode_provider.clone(),
            recent_models: stored.recent_models.clone(),
        };

        cx.set_window_appearance(app.appearance.native());
        dunlin_ui::theme::set_skin(app.skin, cx);
        dunlin_ui::set_motion_preference(app.reduce_motion, cx);
        dunlin_ui::theme::set_mode(
            app.appearance.mode(cx.window_appearance()),
            Some(window),
            cx,
        );
        dunlin_ui::theme::set_code_font_size(stored.code_font_size, cx);
        dunlin_ui::theme::set_font_families(&app.ui_font_family, &app.code_font_family, cx);
        dunlin_ui::scale::set_density(match app.density {
            UiDensity::Compact => dunlin_ui::scale::Density::Compact,
            UiDensity::Comfortable => dunlin_ui::scale::Density::Comfortable,
        });
        cx.observe_window_appearance(window, |this, window, cx| {
            if this.appearance == Appearance::System {
                dunlin_ui::theme::set_mode_animated(
                    this.appearance.mode(window.appearance()),
                    window,
                    this.reduce_motion,
                    cx,
                );
                cx.notify();
            }
        })
        .detach();

        let first_session = app.session().id.clone();
        app.load_transcript(&first_session);
        app.refresh_vault(cx);
        app.spawn_vault_approvals(window, cx);

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
        // The first project is already selected before restore. Its Git status
        // still needs a background refresh so the branch/remote status bar is
        // accurate instead of showing its default empty state.
        if !restored_project && !app.project().is_remote() && !app.project().path.is_empty() {
            app.refresh_git_status(cx);
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
        } else if stored.section == SidebarSection::Dashboard
            || stored.section == SidebarSection::Security
        {
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
        app.assign_imported_projects(cx);
        app.restart_file_watcher();
        app.refresh_project_icons(cx);
        if !restored_project && !app.projects.is_empty() {
            app.refresh_git_status(cx);
        }
        app.persist_settings(cx);
        app.refresh_usage(cx);
        app.poll_host_presence(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.check_open_file(cx);
                        this.poll_file_watcher(cx);
                        this.poll_project_folders(cx);
                        this.refresh_project_icons(cx);
                        this.poll_task_budgets(cx);
                        this.poll_coordination(cx);
                        this.poll_host_presence(cx);
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
                if !this.factory_resetting && this.last_window_geometry != Some(geometry) {
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
            Some(cx.observe_window_activation(window, |this, window, cx| {
                this.set_app_active(window.is_window_active() || this.overlay_window.active, cx);
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
        project.is_not_downloaded() || (!project.is_remote() && !Path::new(&project.path).is_dir())
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
        if self.project().is_remote()
            && let harness::HarnessStatus::Ready { agent, .. } = &self.harness.status
        {
            return agent.clone();
        }
        self.sessions
            .get(self.selected_session)
            .map(|session| session.harness.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.effective_harness())
    }

    pub(crate) fn effective_permission_mode(&self) -> PermissionMode {
        self.effective_permission_mode_for(&self.session().id)
    }

    pub(crate) fn effective_permission_mode_for(&self, session_id: &str) -> PermissionMode {
        if let Some(context) = self.group_contexts.get(session_id) {
            return context.permission_mode(
                &self.projects,
                &self.project_groups,
                self.permission_mode,
            );
        }
        let project = self
            .all_sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| {
                self.projects
                    .iter()
                    .find(|project| project.id == session.project_id)
            })
            .unwrap_or_else(|| self.project());
        let mode = self
            .resolved_project_settings(project)
            .map(|settings| settings.permission_mode.value)
            .unwrap_or(PermissionMode::ReadOnly);
        let autonomy = (!project.is_remote() && project.trusted)
            .then(|| dunlin_artifacts::contract_autonomy(std::path::Path::new(&project.path)))
            .flatten();
        crate::spec::permission_ceiling(mode, autonomy)
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
        if project.is_not_downloaded() {
            return Err("Repository not downloaded · click its download icon in Projects".into());
        }
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
        let mut active_sessions: std::collections::HashSet<_> =
            self.parked_runtimes.keys().cloned().collect();
        if self.harness.harness.is_some() {
            active_sessions.insert(self.session().id.clone());
        }
        for row in &rows {
            let summary = remote_session_summary(row, &project_id, &host_id);
            if let Some(existing) = self
                .all_sessions
                .iter_mut()
                .find(|session| session.id == summary.id)
            {
                existing.title = summary.title;
                existing.updated_at = summary.updated_at;
                if !active_sessions.contains(&existing.id) {
                    existing.status = SessionStatus::Idle;
                }
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
                && !live.contains(&session.id)
                && !active_sessions.contains(&session.id))
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
            self.harness.pending_remote_session = Some(PendingRemoteSession::New);
            if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            }
            self.notice = Some("No remote sessions yet · starting a new one".into());
        } else {
            let preferred = self
                .restore_session_id
                .take()
                .or_else(|| self.project_session_selections.get(&project_id).cloned());
            let index = preferred
                .as_deref()
                .and_then(|id| self.sessions.iter().position(|session| session.id == id))
                .unwrap_or(0);
            self.selected_session = index;
            let session = self.sessions[index].clone();
            if self.restore_runtime(&session.id) {
                self.refresh_mcp(cx);
                self.refresh_providers(cx);
                self.ensure_usage(cx);
                cx.notify();
                return;
            }
            self.load_transcript(&session.id);
            let acp_id = session
                .acp_session_id
                .clone()
                .unwrap_or_else(|| session.id.clone());
            self.harness.pending_remote_session =
                Some(PendingRemoteSession::Load { session_id: acp_id });
            if self.harness.status.is_ready() {
                let action = self.harness.pending_remote_session.take().unwrap();
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
            .get_mut(self.selected_session)
            .filter(|session| session.id.starts_with("pending-"))
        {
            let pending_id = existing.id.clone();
            if let Some(context) = self.group_contexts.remove(&pending_id) {
                self.group_contexts.insert(session_id.to_string(), context);
            }
            if let Some(skills) = self.conversation_skills.remove(&pending_id) {
                self.conversation_skills
                    .insert(session_id.to_string(), skills);
            }
            existing.id = session_id.to_string();
            existing.acp_session_id = Some(session_id.to_string());
            existing.status = SessionStatus::Idle;
            let stored = existing.clone();
            if let Some(all) = self
                .all_sessions
                .iter_mut()
                .find(|session| session.id == pending_id)
            {
                all.id = session_id.to_string();
                all.acp_session_id = Some(session_id.to_string());
                all.status = SessionStatus::Idle;
            }
            self.rekey_chat_tab(&pending_id, session_id);
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
            workspace: Some(self.project().path.clone()),
            id: session_id.to_string(),
            project_id,
            title: "New session".into(),
            status: SessionStatus::Idle,
            harness: "Remote".into(),
            model: "Auto".into(),
            branch: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
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
            self.harness.pending_remote_session = Some(action);
            return;
        };
        let cwd = PathBuf::from(&self.project().path);
        let roots = self.session_additional_directories();
        self.harness.local_session_loading = true;
        match action {
            PendingRemoteSession::New => {
                let name = self.session_harness();
                let model = self
                    .harness
                    .inherited_model
                    .as_ref()
                    .and_then(|model| model.id_for(&name))
                    .map(str::to_owned)
                    .or_else(|| {
                        self.resolved_project_settings(self.project())
                            .ok()
                            .and_then(|settings| settings.default_model(&name).map(str::to_owned))
                    });
                self.harness.pending_model = model.clone();
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
        let mut items =
            vec![
            CommandItem {
                command: Command::Section(SidebarSection::Dashboard),
                label: "Dashboard".into(), hint: None,
                icon: IconName::ChartNoAxesColumn,
                keywords: "home overview activity recent projects".into(),
            },
            CommandItem {
                command: Command::Artifacts, label: "Artifacts & Evidence".into(), hint: None,
                icon: IconName::FileText, keywords: "artifact evidence files knowledge".into(),
            },
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
                command: Command::View(MainView::Timeline),
                label: "Go to File Timeline".into(),
                hint: Some("⌘3".into()),
                icon: IconName::ChartNoAxesGantt,
                keywords: "files edits changes additions deletions timeline".into(),
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
                command: Command::Spec,
                label: "Open Spec Workspace".into(),
                hint: None,
                icon: IconName::ListTodo,
                keywords: "intent spec plan tasks constitution sdd spec-kit cockpit".into(),
            },
            CommandItem {
                command: Command::CaptureIntent,
                label: "Capture Intent…".into(),
                hint: None,
                icon: IconName::FilePlus,
                keywords: "intent why problem outcome brainstorm draft".into(),
            },
            CommandItem {
                command: Command::AgentContract,
                label: "Open Agent Contract".into(),
                hint: None,
                icon: IconName::ShieldCheck,
                keywords: "agent contract behaviour values boundaries autonomy".into(),
            },
            CommandItem {
                command: Command::Subtasks,
                label: "Subtask Worktrees".into(),
                hint: None,
                icon: IconName::GitBranch,
                keywords: "task board agent worktree branch isolation".into(),
            },
            CommandItem {
                command: Command::Budgets,
                label: "Open Task Budgets".into(),
                hint: None,
                icon: IconName::Gauge,
                keywords: "limits cost request time".into(),
            },
            CommandItem {
                command: Command::Coordination,
                label: "Open Agent Coordination".into(),
                hint: None,
                icon: IconName::Bot,
                keywords:
                    "agents peers inbox messages wait dependencies continuations orchestration"
                        .into(),
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
                command: Command::Section(SidebarSection::Security),
                label: "Show Security".into(),
                hint: None,
                icon: IconName::ShieldCheck,
                keywords: "security audit vulnerability skill cloudflare codex".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Sessions),
                label: "Show Sessions".into(),
                hint: None,
                icon: IconName::Clock3,
                keywords: "history runs conversations".into(),
            },
            CommandItem {
                command: Command::Section(SidebarSection::Knowledge),
                label: "Show Knowledge".into(),
                hint: None,
                icon: IconName::BookOpen,
                keywords: "intent concept decision graph evidence proposals".into(),
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
                command: Command::NewChat,
                label: "New Standalone Chat".into(),
                hint: None,
                icon: IconName::MessageSquare,
                keywords: "scratch workspace chat space no repository isolated history".into(),
            },
            CommandItem {
                command: Command::NewChatInRepo,
                label: "New Chat in Repository…".into(),
                hint: None,
                icon: IconName::FolderOpen,
                keywords: "standalone chat explicit context repository consent".into(),
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
                label: "Add Projects…".into(),
                hint: Some("⌘O".into()),
                icon: IconName::FolderOpen,
                keywords:
                    "add project workspace directory folder import clone github repository filter"
                        .into(),
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
                label: "Configure Agents".into(),
                hint: None,
                icon: IconName::Bot,
                keywords: "opencode claude codex acp agent harness adapter".into(),
            },
            CommandItem {
                command: Command::Settings(SettingsTab::Usage),
                label: "Show Usage & Limits".into(),
                hint: None,
                icon: IconName::ChartNoAxesColumn,
                keywords: "quota subscription tokens cost".into(),
            },
            CommandItem {
                command: Command::SaveClipboardArtifact,
                label: "Save Clipboard as Artifact".into(),
                hint: None,
                icon: IconName::FilePlus,
                keywords: "artifact evidence clipboard capture".into(),
            },
            CommandItem {
                command: Command::SaveFileArtifact,
                label: "Save Current File as Artifact".into(),
                hint: None,
                icon: IconName::FilePlus,
                keywords: "artifact evidence file capture".into(),
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
        if !self.spec_tools_enabled() {
            items.retain(|item| {
                !matches!(
                    item.command,
                    Command::Spec | Command::CaptureIntent | Command::AgentContract
                )
            });
        }

        for (index, entry) in self.pack_command_entries().into_iter().enumerate() {
            items.push(CommandItem {
                command: Command::Pack(index),
                label: format!("{} · {}", entry.pack_name, entry.label),
                hint: Some("pack".into()),
                icon: IconName::Puzzle,
                keywords: format!(
                    "{} {} {}",
                    entry.pack_id, entry.command_id, entry.description
                ),
            });
        }

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
        for (index, project) in self.projects.iter().enumerate() {
            if project.is_chat_space() {
                continue;
            }
            items.push(CommandItem {
                command: Command::SwitchProject(index),
                label: format!("Switch to {}", project.name),
                hint: Some("project".into()),
                icon: IconName::Folder,
                keywords: project.path.clone(),
            });
            if !project.is_remote() {
                items.push(CommandItem {
                    command: Command::NewChatInProject(index),
                    label: format!("New Chat in {}", project.name),
                    hint: Some("chat context".into()),
                    icon: IconName::MessageSquare,
                    keywords: format!("standalone chat repository context {}", project.path),
                });
            }
        }

        if !self.security_enabled {
            items
                .retain(|item| !matches!(item.command, Command::Section(SidebarSection::Security)));
        }
        items
    }

    /// Filter commands by every whitespace-separated term in the query.
    pub(crate) fn filtered_commands(&self) -> Vec<CommandItem> {
        let query = self.command_text.trim().to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut items: Vec<_> = self
            .commands()
            .into_iter()
            .filter(|item| {
                !self.command_projects_only || matches!(item.command, Command::SwitchProject(_))
            })
            .filter(|item| {
                (!self.command_projects_only
                    && item.command == Command::Section(SidebarSection::Dashboard))
                    || command_matches(item, &terms)
            })
            .collect();
        if !self.command_projects_only {
            for item in views::settings::search_entries(&query) {
                if !items
                    .iter()
                    .any(|existing| existing.command == item.command)
                {
                    items.push(item);
                }
            }
            items.extend(self.global_search.items(&terms));
            items.sort_by_key(|item| crate::global_search::rank(item, terms.is_empty()));
            items.truncate(80);
        }
        items
    }

    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_command_palette(false, window, cx);
    }

    pub(crate) fn open_project_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_command_palette(true, window, cx);
    }

    fn show_command_palette(
        &mut self,
        projects_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation_blocked() {
            return;
        }
        self.command_projects_only = projects_only;
        if !projects_only {
            self.refresh_global_search();
        }
        if !self.command_open {
            self.command_epoch = self.command_epoch.wrapping_add(1);
        }
        self.command_open = true;
        self.command_text.clear();
        self.command_index = 0;
        self.command_scroll.set_offset(point(px(0.), px(0.)));
        self.settings_open = false;
        self.project_menu_open = false;
        self.command_query.update(cx, |state, cx| {
            state.set_placeholder(
                if projects_only {
                    "Search projects…"
                } else {
                    "Search projects, sessions, files and settings…"
                },
                window,
                cx,
            );
            state.set_value("", window, cx);
        });
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
            let offset = (self.command_index as f32 * 49. + 48. + 10. - 330.).max(0.);
            self.command_scroll.set_offset(point(px(0.), -px(offset)));
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
                self.set_view(view, cx);
            }
            Command::Section(section) => {
                if section == SidebarSection::Knowledge {
                    self.open_workspace_tool(crate::workspace_tools::Tool::Knowledge, window, cx);
                    return;
                }
                if section == SidebarSection::Security {
                    self.open_workspace_tool(crate::workspace_tools::Tool::Security, window, cx);
                    return;
                }
                if section == SidebarSection::Security && !self.security_enabled {
                    self.settings_open = true;
                    self.settings_tab = SettingsTab::Security;
                    return;
                }
                self.section = section;
                if matches!(
                    section,
                    SidebarSection::Projects
                        | SidebarSection::Sessions
                        | SidebarSection::Files
                        | SidebarSection::Search
                ) {
                    self.sidebar_tab = crate::sidebar_layout::Tab::from_section(section);
                    self.sidebar_collapsed = false;
                }
                self.settings_open = false;
                self.persist_settings(cx);
                if section == SidebarSection::Knowledge {
                    self.refresh_knowledge(cx);
                } else if section == SidebarSection::Search {
                    let handle = self.search_input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                } else if section == SidebarSection::Files {
                    self.sidebar_collapsed = false;
                    self.focus_explorer(window, cx);
                } else if section == SidebarSection::SourceControl {
                    self.open_git_history(window, cx);
                    if !self.git_loading {
                        self.refresh_git_status(cx);
                    }
                } else if section == SidebarSection::Security {
                    self.security.checked_project = None;
                }
            }
            Command::Settings(tab) => {
                self.settings_open = true;
                self.settings_tab = tab;
            }
            Command::SearchResult(index) => self.open_global_search_result(index, window, cx),
            Command::Artifacts => {
                self.open_workspace_tool(crate::workspace_tools::Tool::Artifacts, window, cx);
            }
            Command::OpenFolder => self.open_folder(cx),
            Command::AddRemoteProject => self.open_remote_picker(cx),
            Command::NewProjectGroup => self.open_new_group_prompt(None, window, cx),
            Command::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                self.persist_settings(cx);
            }
            Command::ToggleTheme => {
                self.toggle_appearance(window, cx);
                window.activate_window();
                self.persist_settings(cx);
            }
            Command::NewSession => self.new_session(cx),
            Command::NewChat => self.new_chat_space(cx),
            Command::NewChatInRepo => self.new_chat_space_in_repo(window, cx),
            Command::NewChatInProject(index) => self.new_chat_space_in_project(index, window, cx),
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
            Command::ChooseBranch => {
                self.open_branch_picker(&self.project().id.clone(), window, cx)
            }
            Command::ToggleTerminal => self.toggle_bottom_panel(BottomPanel::Terminal, window, cx),
            Command::ToggleRun => self.toggle_bottom_panel(BottomPanel::Run, window, cx),
            Command::Impact => self.open_impact_map(window, cx),
            Command::Compare => self.open_comparisons(window, cx),
            Command::Spec => {
                if self.spec_tools_enabled() {
                    self.open_spec_workspace(window, cx)
                } else {
                    self.file_notice(
                        "Spec, intent and agent-contract tools are disabled in Settings → Extensions",
                        cx,
                    )
                }
            }
            Command::CaptureIntent => {
                if self.spec_tools_enabled() {
                    self.capture_intent(window, cx)
                } else {
                    self.file_notice("Intent capture is disabled in Settings → Extensions", cx)
                }
            }
            Command::AgentContract => {
                if self.spec_tools_enabled() {
                    self.open_agent_contract(window, cx)
                } else {
                    self.file_notice(
                        "The agent contract is disabled in Settings → Extensions",
                        cx,
                    )
                }
            }
            Command::Subtasks => self.open_subtask_worktrees(window, cx),
            Command::Budgets => self.open_task_budgets(window, cx),
            Command::Coordination => self.toggle_bottom_panel(BottomPanel::Agents, window, cx),
            Command::Checks => self.open_checks(window, cx),
            Command::Context => self.open_context_inspector(window, cx),
            Command::Review => self.open_task_review(window, cx),
            Command::SaveClipboardArtifact => self.save_clipboard_artifact(cx),
            Command::SaveFileArtifact => self.save_file_artifact(cx),
            Command::Pack(index) => {
                let entry = self.pack_command_entries().into_iter().nth(index);
                if let Some(entry) = entry {
                    self.run_pack_command(&entry.pack_id, &entry.command_id, window, cx);
                } else {
                    self.file_notice("That pack command is no longer available", cx);
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
        if !matches!(
            command,
            Command::SearchResult(_)
                | Command::Section(SidebarSection::Files | SidebarSection::Search)
        ) {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Local imports start in the system folder picker, without an intermediate sheet.
    pub(crate) fn open_folder(&mut self, cx: &mut Context<Self>) {
        if self.navigation_blocked() || self.factory_resetting {
            return;
        }
        if self.local_import_running
            || self.project_import.busy
            || self.backup.busy
            || self.backup.cloning
        {
            self.file_notice("Wait for the current import to finish", cx);
            return;
        }
        self.command_open = false;
        self.context_menu = None;
        self.project_menu_open = false;
        self.settings_open = false;
        self.remote_picker = None;
        self.chat_menu_open = false;
        self.branch_ui.dismiss();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add projects from local drive".into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |this, cx| {
                if this.factory_resetting || this.navigation_blocked() {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        this.import_local_folders(paths, cx)
                    }
                    Ok(Err(error)) => this.file_notice(error.to_string(), cx),
                    _ => {}
                }
            });
        })
        .detach();
    }

    /// Re-point a local project whose folder was moved or deleted.
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
        self.settings_ui.project_focus = Some(cx.focus_handle());
        self.reset_project_color_picker();
        self.group_settings_draft = None;
        self.project_runtime_draft = None;
        self.context_menu = None;
        self.project_menu_open = false;
        self.project_settings_target = Some(index);
        if let Some(project) = self.projects.get(index).cloned()
            && let Some(remote) = project.remote
        {
            self.sync_remote_project_settings(&remote.host_id, &remote.project_id, &project.id, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_project_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_ui.project_focus = None;
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
        self.sync_vault_read_only();
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

    pub(crate) fn pair_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let pair = remote::pair(&url_for_task, &code, "Dunlin on macOS")?;
                    #[cfg(debug_assertions)]
                    crate::remote_e2e::ui::journal_keychain(&host_id_for_task)?;
                    remote::save_token(&host_id_for_task, &pair.token)?;
                    Ok::<_, String>((pair, host_id_for_task))
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.remote_loading = false;
                this.remote_code_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                match result {
                    Ok((pair, host_id)) => {
                        let managed_id = host_id.clone();
                        this.remote_tokens.insert(host_id.clone(), pair.token);
                        this.remote_hosts.push(remote::RemoteHost {
                            id: host_id,
                            name,
                            url,
                            user_name: pair.user.name,
                            user_role: pair.user.role,
                            device_id: pair.device_id,
                        });
                        this.poll_host_presence(cx);
                        this.persist_settings(cx);
                        this.notice = Some("Paired this Mac with the relay".into());
                        this.remote_connection.open = false;
                        this.manage_server(&managed_id, window, cx);
                    }
                    Err(error) => {
                        this.remote_connection.error = Some(error.clone());
                        this.set_notice(error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn remove_remote_host(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.server_panel.host.as_deref() == Some(id) {
            if let Some(cancel) = &self.server_panel.transfer {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.server_panel = Default::default();
        }
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
        self.poll_host_presence(cx);
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
            kind: Default::default(),
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
        self.remote_picker = None;
        self.settings_open = false;
        self.select_project(index, cx);
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
        let Some(expected) = self.projects.iter().find(|p| p.id == local_id).cloned() else {
            return;
        };
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
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { remote::project_settings(&url, &token, &remote_id) })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(settings) => {
                    this.apply_remote_project_settings(&expected, settings, cx);
                }
                Err(error) => logging::warn(&format!("remote settings: {error}")),
            });
        })
        .detach();
    }

    fn apply_remote_project_settings(
        &mut self,
        expected: &Project,
        settings: ProjectSettings,
        cx: &mut Context<Self>,
    ) {
        // Never replace the baseline underneath an open or saving form.
        if self
            .project_runtime_draft
            .as_ref()
            .is_some_and(|draft| draft.belongs_to(&expected.id))
        {
            return;
        }
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == expected.id)
            && crate::project_runtime_settings::refresh_remote_defaults(project, expected, settings)
        {
            if let Some(store) = &self.store {
                let _ = store.upsert_project(project);
            }
            cx.notify();
        }
    }

    /// Create a standalone scratch chat. It has its own project/history id and
    /// never appears as a repository project in the Projects tree.
    pub(crate) fn new_chat_space(&mut self, cx: &mut Context<Self>) {
        let Some(base) = data_dir() else {
            self.set_notice(
                "Local data directory is unavailable; cannot create a scratch chat",
                cx,
            );
            return;
        };
        let id = format!("chat-{}", uuid::Uuid::now_v7());
        let path = base.join("chat-spaces").join(&id);
        if let Err(error) = fs::create_dir_all(&path) {
            self.set_notice(
                format!("Could not create the scratch workspace: {error}"),
                cx,
            );
            return;
        }
        let count = self
            .projects
            .iter()
            .filter(|project| project.is_chat_space() && project.remote.is_none())
            .count();
        let name = if count == 0 {
            "Chat".to_string()
        } else {
            format!("Chat {}", count + 1)
        };
        self.create_chat_space(id, name, path, None, cx);
    }

    /// Create a standalone chat whose explicit, consented workspace is one
    /// repository. It is still a chat space: separate history and no silent cwd
    /// change for any existing project session.
    pub(crate) fn new_chat_space_in_repo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a repository context for a standalone chat".into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let display = path.to_string_lossy().to_string();
            let _ = this.update_in(cx, |this, window, cx| {
                this.confirm_chat_context(path, display, window, cx)
            });
        })
        .detach();
    }

    /// Reuse an already configured local repository as the explicit context of
    /// a new chat space. The repository project itself is not modified.
    fn new_chat_space_in_project(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.get(index) else {
            return;
        };
        if project.is_chat_space() || project.is_remote() {
            self.set_notice("Choose a local repository project as chat context", cx);
            return;
        }
        let path = PathBuf::from(&project.path);
        let display = project.path.clone();
        self.confirm_chat_context(path, display, window, cx);
    }

    fn confirm_chat_context(
        &mut self,
        path: PathBuf,
        display: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Use this folder as explicit chat context?",
            Some(&format!(
                "Standalone chat workspace: {display}\n\nA new chat space with isolated history will be created. Its tools can access this folder under the current permission mode; this is not a security sandbox. Existing project sessions never change their cwd."
            )),
            &["Cancel", "Create chat context"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn_in(window, async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let id = format!("chat-{}", uuid::Uuid::now_v7());
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Chat".into());
                this.create_chat_space(id, format!("Chat · {name}"), path, Some(display), cx);
            });
        })
        .detach();
    }

    fn create_chat_space(
        &mut self,
        id: String,
        name: String,
        path: PathBuf,
        repo_context: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut project = Project {
            id,
            name,
            path: path.to_string_lossy().to_string(),
            branch: None,
            tone: ProjectTone::Teal,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: ProjectKind::ChatSpace,
            remote: None,
            group_id: None,
            settings: Default::default(),
        };
        if let Some(context) = repo_context {
            project
                .settings
                .environment
                .insert("DUNLIN_CHAT_CONTEXT".into(), context);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project(&project)
        {
            self.report_storage_error("save the chat space", error);
            return;
        }
        self.projects.push(project.clone());
        self.assign_imported_projects(cx);
        let index = self.projects.len() - 1;
        self.select_project_internal(index, true, cx);
        if self.sessions.is_empty() {
            self.new_session(cx);
        }
        self.section = SidebarSection::Projects;
        self.set_view(MainView::Chat, cx);
        self.set_notice(
            "Created a standalone chat space with isolated history and no repository project entry",
            cx,
        );
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            self.set_notice("Open a project first", cx);
            return;
        }
        if self.harness.local_session_loading
            || self.harness.session_metadata_pending
            || self.harness.status.is_busy()
            || self.harness.model_changing
        {
            self.set_notice(
                if self.harness.model_changing {
                    "Wait for the model change to finish"
                } else {
                    "Wait for the harness to finish connecting"
                },
                cx,
            );
            return;
        }
        let new_harness = if self.project().is_remote() {
            self.session_harness()
        } else {
            self.effective_harness()
        };
        let inherited_model = self.inherited_new_session_model(&new_harness);
        let project_id = self.project().id.clone();
        if let Some(remote_project) = self.project().remote.clone() {
            self.park_current_runtime(cx);
            self.harness.inherited_model = inherited_model.clone();
            self.reset_mcp_for_navigation();
            let pending = SessionSummary {
                workspace: Some(self.project().path.clone()),
                id: format!("pending-{}", uuid::Uuid::now_v7()),
                project_id,
                title: "New session".into(),
                status: SessionStatus::Idle,
                harness: new_harness,
                model: inherited_model
                    .as_ref()
                    .map(|model| model.choice.name.clone())
                    .unwrap_or_else(|| "Auto".into()),
                branch: None,
                updated_at: chrono::Utc::now().to_rfc3339(),
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
            self.restore_chat_ui();
            if let Some(context) = self.pending_group_context.take() {
                self.group_contexts.insert(pending.id.clone(), context);
            }
            self.harness.transcript.clear();
            self.chat_visible_items = CHAT_WINDOW;
            self.harness.session_id = None;
            self.harness.running = false;
            self.selected_node = None;
            self.harness.pending_remote_session = Some(PendingRemoteSession::New);
            let group_roots = self.session_remote_roots();

            if !group_roots.is_empty() && self.harness.harness.is_some() {
                // The relay sandbox and ACP scope were fixed when the current
                // connection opened. A group root list requires a fresh
                // connection so a prompt cannot see a wider/narrower set than
                // the approved manifest.
                self.disconnect_harness(cx);
            }
            if self.harness.status.is_ready() {
                let action = self.harness.pending_remote_session.take().unwrap();
                self.start_remote_session(action, cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.connect_harness(cx);
            } else {
                self.notice = Some("Connect to the relay to start a session".into());
            }

            self.set_view(MainView::Chat, cx);
            self.ensure_usage(cx);
            cx.notify();
            return;
        }

        let branch = self.project().branch.clone();
        let mut session = blank_session(&project_id, &new_harness, branch);
        if let Some(model) = &inherited_model {
            session.model = model.choice.name.clone();
        }
        session.workspace = Some(self.project().path.clone());
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
        self.harness.inherited_model = inherited_model;
        self.harness.session_id = None;
        self.harness.running = false;

        if self.auto_connect {
            self.connect_harness(cx);
        }

        self.set_view(MainView::Chat, cx);
        self.selected_node = None;
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
        self.request_delete_session_id(&session.id.clone(), cx);
    }

    pub(crate) fn request_delete_session_id(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.all_sessions.iter().find(|s| s.id == id) else {
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
        if self.session_import.loading {
            self.set_notice(
                "Wait for the session import to finish before clearing history",
                cx,
            );
            return;
        }
        self.pending_delete = Some(PendingDelete::History {
            count: self.all_sessions.len() + self.session_import.pending.len(),
        });
        cx.notify();
    }

    pub(crate) fn request_discard_file(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_git_action(
            dunlin_workspace::git::operations::Action::Discard {
                paths: vec![path.into()],
            },
            window,
            cx,
        );
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
            None => {}
        }
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.all_sessions.is_empty() && self.session_import.pending.is_empty() {
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
        self.session_import.pending.clear();
        self.session_import.notices.clear();
        self.sessions.clear();
        self.selected_node = None;
        self.expanded_tool = None;

        if self.projects.is_empty() {
            self.harness.transcript.clear();
            self.set_notice("Cleared conversation history", cx);
            return;
        }
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
        let Some(session) = self.all_sessions.iter().find(|session| session.id == id) else {
            return;
        };
        let index = self.sessions.iter().position(|session| session.id == id);
        if self.harness.local_session_loading || self.harness.status.is_busy() {
            self.set_notice("Wait for the harness to finish connecting", cx);
            return;
        }
        let remote_origin = match &session.origin {
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

        let active = index == Some(self.selected_session);
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
        self.all_sessions.retain(|session| session.id != id);
        self.group_contexts.remove(id);
        self.conversation_skills.remove(id);
        self.forget_transcript(id);
        let Some(index) = index else {
            self.notice = Some("Deleted session".into());
            self.persist_settings(cx);
            cx.notify();
            return;
        };
        self.sessions.remove(index);
        let remote_project = self.project().is_remote();
        if self.sessions.is_empty() {
            if remote_project {
                self.harness.transcript.clear();
                self.harness.pending_remote_session = Some(PendingRemoteSession::New);
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
            if self.restore_runtime(&session.id) {
                self.refresh_providers(cx);
                self.refresh_mcp(cx);
                self.persist_settings(cx);
                cx.notify();
                return;
            }
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
                self.harness.pending_remote_session =
                    Some(PendingRemoteSession::Load { session_id: acp_id });
                if self.harness.status.is_ready() {
                    let action = self.harness.pending_remote_session.take().unwrap();
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
        if self
            .workbench
            .tabs
            .iter()
            .any(|tab| tab.project == id && tab.saving)
        {
            self.file_notice("Wait for pending saves before removing the project", cx);
            return;
        }
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
            self.conversation_skills.remove(session_id);
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
            self.harness.pending_remote_session = None;
            self.remote_sessions_loading = false;
            self.close_inspector();
            self.diff_view = None;
            self.explorer.search.clear(&mut self.file_tree);
            self.explorer.selected = None;
            self.explorer.scroll = ScrollHandle::new();
            self.file_tree = FileTree {
                root: String::new(),
                entries: Vec::new(),
                error: None,
                total_size: 0,
            };
            self.restart_file_watcher();
            self.git = git::GitStatus::default();
            self.global_search = Default::default();
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

    fn activity_selected(&self) -> Option<&str> {
        match self.view {
            MainView::Graph => self.selected_node.as_deref(),
            _ => None,
        }
    }

    fn set_activity_selection(&mut self, id: Option<String>) {
        if self.view == MainView::Graph {
            self.selected_node = id;
        }
    }

    pub(crate) fn activity_visible_ids(&self) -> Vec<String> {
        match self.view {
            MainView::Graph => {
                let session = self.session();
                let all = projection::nodes(&self.harness.transcript, self.project(), session);
                let nodes = crate::activity::graph_nodes(all, &self.activity.agents_expanded);
                crate::activity::filter_graph_nodes(nodes, &self.activity_query)
                    .into_iter()
                    .map(|node| node.id)
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Keep focus on a real row after search, filters, collapse or a streaming
    /// update. The previous code could leave `aria-activedescendant` pointing at
    /// a node that was no longer rendered.
    pub(crate) fn activity_normalize_selection(&mut self, cx: &mut Context<Self>) {
        let ids = self.activity_visible_ids();
        let next = crate::activity::normalize_selection(self.activity_selected(), &ids);
        self.set_activity_selection(next);
        cx.notify();
    }

    pub(crate) fn activity_move_selection(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.activity_visible_ids();
        let next = crate::activity::move_selection(self.activity_selected(), &ids, delta);
        self.set_activity_selection(next);
        self.activity_focus.focus(window, cx);
        self.activity_reveal_selection(cx);
        cx.notify();
    }

    pub(crate) fn activity_select_edge(
        &mut self,
        last: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.activity_visible_ids();
        let next = if last { ids.last() } else { ids.first() }.cloned();
        self.set_activity_selection(next);
        self.activity_focus.focus(window, cx);
        self.activity_reveal_selection(cx);
        cx.notify();
    }

    /// Scroll the selected graph node into view. Virtualization is
    /// only honest when keyboard selection cannot move focus to an offscreen row.
    fn activity_reveal_selection(&mut self, cx: &mut Context<Self>) {
        if self.view == MainView::Graph {
            let Some(id) = self.selected_node.clone() else {
                return;
            };
            let all = projection::nodes(&self.harness.transcript, self.project(), self.session());
            let nodes = crate::activity::graph_nodes(all, &self.activity.agents_expanded);
            let nodes = crate::activity::filter_graph_nodes(nodes, &self.activity_query);
            let Some(node) = nodes.iter().find(|node| node.id == id) else {
                return;
            };
            let (x, y, width, height) =
                crate::activity::scaled_node_rect(node, self.activity.graph_zoom);
            let viewport = self.activity.graph_scroll.bounds().size;
            let viewport_width = f32::from(viewport.width);
            let viewport_height = f32::from(viewport.height);
            if viewport_width <= 0.0 || viewport_height <= 0.0 {
                return;
            }
            let max = self.activity.graph_scroll.max_offset();
            let target_x = (x + width / 2.0 - viewport_width / 2.0).clamp(0.0, f32::from(max.x));
            let target_y = (y + height / 2.0 - viewport_height / 2.0).clamp(0.0, f32::from(max.y));
            self.activity.graph_centered = true;
            self.activity
                .graph_scroll
                .set_offset(point(px(-target_x), px(-target_y)));
        }
        cx.notify();
    }

    pub(crate) fn activity_toggle_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            MainView::Graph => {
                if let Some(id) = self.selected_node.clone()
                    && !self.activity.agents_expanded.remove(&id)
                {
                    self.activity.agents_expanded.insert(id);
                }
            }
            _ => return,
        }
        self.activity_focus.focus(window, cx);
        self.activity_reveal_selection(cx);
        cx.notify();
    }

    pub(crate) fn activity_expand_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            MainView::Graph => {
                let Some(id) = self.selected_node.clone() else {
                    return;
                };
                let all =
                    projection::nodes(&self.harness.transcript, self.project(), self.session());
                let has_children = all.iter().any(|node| node.parent.as_deref() == Some(&id));
                if !has_children {
                    return;
                }
                if self.activity.agents_expanded.contains(&id) {
                    let nodes = crate::activity::graph_nodes(all, &self.activity.agents_expanded);
                    let nodes = crate::activity::filter_graph_nodes(nodes, &self.activity_query);
                    let next = nodes
                        .iter()
                        .find(|node| node.parent.as_deref() == Some(&id))
                        .map(|node| node.id.clone());
                    if let Some(next) = next {
                        self.set_activity_selection(Some(next));
                    }
                } else {
                    self.activity.agents_expanded.insert(id);
                }
            }
            _ => return,
        }
        self.activity_focus.focus(window, cx);
        self.activity_reveal_selection(cx);
        cx.notify();
    }

    pub(crate) fn activity_collapse_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            MainView::Graph => {
                let Some(id) = self.selected_node.clone() else {
                    return;
                };
                if self.activity.agents_expanded.remove(&id) {
                    // The selected agent is still visible after collapsing.
                } else {
                    let all =
                        projection::nodes(&self.harness.transcript, self.project(), self.session());
                    let parent = all
                        .iter()
                        .find(|node| node.id == id)
                        .and_then(|node| node.parent.clone());
                    let visible = self.activity_visible_ids();
                    let next = parent
                        .filter(|parent| visible.iter().any(|id| id == parent))
                        .or_else(|| visible.first().cloned());
                    self.set_activity_selection(next);
                }
            }
            _ => return,
        }
        self.activity_focus.focus(window, cx);
        self.activity_reveal_selection(cx);
        cx.notify();
    }

    pub(crate) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.onboarding.setup.is_some() {
            cx.stop_propagation();
            return;
        }
        if self.factory_resetting {
            cx.stop_propagation();
            return;
        }
        if self.group_cleanup.dialog.is_some() {
            if event.keystroke.key == "escape" {
                self.close_cleanup(window, cx);
                cx.stop_propagation();
            }
            // Focused checkboxes/buttons own Space, Enter and Tab. Enter on the
            // dialog itself must not confirm cleanup.
            return;
        }
        if self.user_profile.menu_open {
            return;
        }
        if self.free_endpoint_dialog.is_some() {
            if event.keystroke.key == "escape" {
                self.close_free_endpoint_consent(cx);
            }
            // Let focused switches and buttons own Space/Enter/Tab.
            return;
        }
        if event.keystroke.key == "escape"
            && !self.has_modal_overlay()
            && (self.project_menu_open
                || self.context_menu.is_some()
                || self.remote_picker.is_some())
        {
            self.project_menu_open = false;
            self.project_creation.editing = false;
            self.context_menu = None;
            self.remote_picker = None;
            self.focus_handle.focus(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.remote_connection.open {
            if event.keystroke.key == "escape" {
                self.close_remote_connection(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if self.project_menu_open && self.project_creation.editing {
            if event.keystroke.key == "enter" {
                self.confirm_inline_project(cx);
                if !self.project_creation.editing {
                    self.focus_handle.focus(window, cx);
                }
                cx.stop_propagation();
            }
            return; // The inline name owns typing, selection and clipboard shortcuts.
        }
        if self.project_group_editor.is_some() {
            if event.keystroke.key == "escape" {
                self.project_group_editor = None;
                self.focus_handle.focus(window, cx);
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
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
                self.request_close_project_settings(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if self.branch_ui.is_open() {
            if event.keystroke.key == "escape"
                || self
                    .branch_ui
                    .input_focus(cx)
                    .is_some_and(|focus| focus.is_focused(window))
            {
                self.branch_picker_key(&event.keystroke.key, window, cx);
            }
            return;
        }
        if self.provider_auth.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_provider_auth(window, cx);
                    cx.stop_propagation();
                }
                "enter"
                    if self.focus_handle.is_focused(window)
                        || self.provider_auth_input_focused(window, cx) =>
                {
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

        if self.settings_open && platform {
            match keystroke.key.as_str() {
                "," => self.request_settings_location(Some(SettingsTab::General), window, cx),
                "w" => self.request_settings_location(None, window, cx),
                "f" => {
                    if let Some(search) = &self.settings_ui.search {
                        search.read(cx).focus_handle(cx).focus(window, cx);
                    }
                }
                _ => return, // Let focused inputs own copy/paste, undo and selection.
            }
            cx.stop_propagation();
            return;
        }

        if self.project_import.open {
            if keystroke.key == "escape" {
                self.project_import.dismiss();
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }

        if self.editor_conflict.is_some() {
            if keystroke.key.as_str() == "escape" {
                self.cancel_editor_conflict(cx);
                cx.stop_propagation();
            }
            return;
        }

        if self.pending_editor.is_some() {
            // A focused action owns Enter/Space; only the dialog's default focus submits here.
            match keystroke.key.as_str() {
                "escape" => {
                    self.cancel_pending_editor(cx);
                    cx.stop_propagation();
                }
                "enter" if self.focus_handle.is_focused(window) => {
                    self.save_pending_editor(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }

        if self.group_prompt.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.cancel_group_prompt(cx);
                    cx.stop_propagation();
                    return;
                }
                "enter"
                    if self.focus_handle.is_focused(window)
                        || self
                            .group_input
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window) =>
                {
                    self.submit_group_prompt(window, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => return,
            }
        }

        if self.queue_open && keystroke.key.as_str() == "escape" {
            self.queue_open = false;
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if self.pending_git_action.is_some() {
            if keystroke.key == "escape" {
                self.cancel_git_action(window, cx);
            } else if matches!(keystroke.key.as_str(), "tab" | "enter" | "space") {
                return;
            }
            cx.stop_propagation();
            return;
        }

        if self.pending_delete.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.cancel_delete(cx);
                    cx.stop_propagation();
                }
                "enter" if self.focus_handle.is_focused(window) => {
                    self.confirm_delete(cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
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

        if self.project_search_key_down(event, window, cx)
            || self.explorer_key_down(event, window, cx)
        {
            cx.stop_propagation();
            return;
        }

        if (self.bottom_panel == BottomPanel::Terminal
            || self.showing_dependency_terminal()
            || matches!(
                self.current_workspace_tab(),
                Some(crate::workspace_panes::TabKey::Terminal(_))
            ))
            && self.terminal.focus.is_focused(window)
            && !self.settings_open
            && self.terminal_handle_key(event, window, cx)
        {
            cx.stop_propagation();
            return;
        }

        if keystroke.key == "tab"
            && shift
            && !platform
            && !keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && !event.is_held
            && self.cycle_chat_mode(window, cx)
        {
            cx.stop_propagation();
            return;
        }

        if !platform
            && self.view == MainView::Graph
            && !self.has_modal_overlay()
            && (self.activity_focus.is_focused(window) || self.focus_handle.is_focused(window))
            && !self
                .activity_search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            match keystroke.key.as_str() {
                "up" => {
                    self.activity_move_selection(-1, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" => {
                    self.activity_move_selection(1, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "pageup" => {
                    self.activity_move_selection(-10, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "pagedown" => {
                    self.activity_move_selection(10, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "home" => {
                    self.activity_select_edge(false, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "end" => {
                    self.activity_select_edge(true, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "enter" | "space" => {
                    self.activity_toggle_selection(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "right" => {
                    self.activity_expand_selection(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "left" => {
                    self.activity_collapse_selection(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.activity_focus.focus(window, cx);
                    self.selected_node = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        if platform {
            let handled = match keystroke.key.as_str() {
                "[" | "]" if !shift => {
                    self.navigate_history(keystroke.key == "]", window, cx);
                    true
                }
                "=" | "+" if !self.has_modal_overlay() => {
                    crate::workspace_text::adjust(0.1, cx);
                    cx.notify();
                    true
                }
                "-" if !self.has_modal_overlay() => {
                    crate::workspace_text::adjust(-0.1, cx);
                    cx.notify();
                    true
                }
                "0" if !self.has_modal_overlay() => {
                    crate::workspace_text::reset(cx);
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
                "p" if keystroke.modifiers.alt => {
                    self.open_project_search(window, cx);
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
                    self.set_view(MainView::Timeline, cx);
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
                "s" if self.workbench.show_file
                    && self.inspector.is_some()
                    && !self.git_history.is_active_for(self.project()) =>
                {
                    self.save_editor(cx);
                    true
                }
                "w" if let Some(key) = self.current_workspace_tab() => {
                    self.request_close_workspace_tab(&key, window, cx);
                    true
                }
                "n" => {
                    self.new_session(cx);
                    true
                }
                "f" if shift => {
                    self.section = SidebarSection::Search;
                    self.sidebar_collapsed = false;
                    self.settings_open = false;
                    let handle = self.search_input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                    cx.notify();
                    true
                }
                "l" if shift => {
                    self.toggle_appearance(window, cx);
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
                self.request_settings_location(None, window, cx);
                cx.stop_propagation();
                cx.notify();
            } else if self.project_menu_open {
                self.project_menu_open = false;
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
        self.toggle_project_trust_at(self.selected_project, cx);
    }

    pub(crate) fn toggle_project_trust_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .projects
            .get(index)
            .is_some_and(|p| p.is_not_downloaded())
        {
            self.file_notice("Download this repository before granting trust", cx);
            return;
        }
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
            if index == self.selected_project && self.auto_connect && !self.harness.status.is_busy()
            {
                self.connect_harness(cx);
            }
        } else {
            if index == self.selected_project {
                self.interrupt_live_turn(cx);
                self.disconnect_harness(cx);
            }
            self.stop_project_runtimes(&stored.id, cx);
            self.revoke_project_coordination(&stored.id, cx);
            self.notice = Some(format!("Untrusted {name} · harness stopped"));
        }
        cx.notify();
    }

    pub(crate) fn set_app_active(&mut self, active: bool, cx: &mut Context<Self>) {
        let returning = active && !self.app_active;
        self.app_active = active;
        if returning {
            self.refresh_permissions(cx);
            if self.projects.is_empty() {
                self.harness_discovery = crate::harness_discovery::discover();
            }
            cx.notify();
        }
    }

    pub(crate) fn refresh_permissions(&mut self, cx: &mut Context<Self>) {
        if self.permissions_refreshing {
            return;
        }
        self.permissions_refreshing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .spawn(async { crate::platform::refresh_permission_statuses() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.permissions_refreshing = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn request_notifications(&mut self, cx: &mut Context<Self>) {
        self.notice = Some("Waiting for macOS notification permission…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let result = crate::platform::request_notification_authorization();
                    crate::platform::refresh_permission_statuses();
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(state) => this.set_notice(format!("Notifications: {}", state.label()), cx),
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
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
        if self.projects[index].is_not_downloaded() {
            self.file_notice(
                "Repository not downloaded · click its download icon in Projects",
                cx,
            );
            return;
        }
        if self.editor_dirty {
            self.pending_editor = Some(PendingEditor::SwitchProject {
                id: self.projects[index].id.clone(),
            });
            cx.notify();
            return;
        }
        // The first imported project also has index 0, but no workspace is loaded yet.
        if index == self.selected_project && self.file_tree.root == self.projects[index].path {
            return;
        }
        self.cancel_ai_commit_preparation();
        self.park_current_runtime(cx);
        self.reset_mcp_for_navigation();
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        self.workspace_tools.current = None;
        self.remote_sessions_loading = false;
        self.selected_project = index;
        self.explorer.selected = None;
        self.explorer.search.clear(&mut self.file_tree);
        self.explorer.scroll = ScrollHandle::new();
        self.explorer.creation = None;
        self.explorer.expand_after_scan.clear();
        self.new_file_prompt = false;
        self.git = git::GitStatus::default();
        self.project_menu_open = false;
        let project = self.projects[index].clone();
        self.restart_file_watcher();
        let path = project.path.clone();
        self.file_tree = FileTree {
            root: path.clone(),
            entries: Vec::new(),
            error: None,
            total_size: 0,
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
        self.harness.pending_remote_session = None;

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
            let settings_baseline = project.clone();
            let navigation_generation = self.navigation_generation;
            cx.spawn(async move |this, cx| {
                let (settings, result) = cx
                    .background_executor()
                    .spawn(async move {
                        let settings = remote::project_settings(&url, &token, &remote_id);
                        let sessions = remote::sessions(&url, &token, &remote_id);
                        (settings, sessions)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    if this.project().id != expected_project
                        || this.navigation_generation != navigation_generation
                    {
                        return;
                    }
                    match settings {
                        Ok(settings) => {
                            this.apply_remote_project_settings(&settings_baseline, settings, cx)
                        }
                        Err(error) => logging::warn(&format!("remote settings: {error}")),
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
            self.ensure_usage(cx);
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
            self.refresh_project_summaries(cx);
        }
        self.ensure_usage(cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Load the explorer through the active local or remote backend.
    pub(crate) fn refresh_file_tree(&mut self, cx: &mut Context<Self>) {
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_tree.error = Some(error);
                self.explorer.tree_loading = false;
                cx.notify();
                return;
            }
        };
        let expected = self.project().path.clone();
        let project = self.project().id.clone();
        self.explorer.tree_generation = self.explorer.tree_generation.wrapping_add(1);
        let generation = self.explorer.tree_generation;
        self.explorer.tree_loading = true;
        cx.notify();
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
                if current {
                    this.explorer.tree_loading = false;
                }
                match result {
                    Ok(mut tree) => {
                        if current || this.file_tree.entries.is_empty() {
                            // Use live expansion state: a query or Escape may have changed
                            // it while the filesystem/relay scan was running.
                            let mut expanded = this.file_tree.expanded_paths();
                            expanded.extend(this.explorer.expand_after_scan.iter().cloned());
                            tree.restore_expanded(&expanded);
                            this.explorer.expand_after_scan.clear();
                            this.explorer.search.refresh(&mut tree);
                            this.file_tree = tree;
                            this.select_explorer_search_match();
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
            self.run.checked = None;
            self.refresh_file_tree(cx);
            self.refresh_git_status(cx);
            if self.spec.is_open_for(self.project()) {
                self.refresh_spec_workspace(cx);
            }
        }
    }

    /// Read branch and working-tree state off the UI thread.
    pub(crate) fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        self.refresh_project_summaries(cx);
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

    /// Jump from a timeline edit to the transcript entry that produced it.
    pub(crate) fn open_timeline_in_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        self.set_view(MainView::Chat, cx);
        self.chat_visible_items = self
            .chat_visible_items
            .max(self.harness.transcript.len().saturating_sub(index));
        self.expanded_tool = Some(index).filter(|index| {
            matches!(
                self.harness.transcript.get(*index),
                Some(harness::ChatItem::Tool { .. } | harness::ChatItem::Child { .. })
            )
        });
        self.chat_scroll.reveal_item(index);
        self.set_notice("Opened the edit in chat", cx);
    }

    /// Open the recorded patch, never today's working-tree version of the file.
    pub(crate) fn open_timeline_diff(&mut self, path: &str, patch: &str, cx: &mut Context<Self>) {
        match DiffFile::parse(patch) {
            Ok(files) if !files.is_empty() => {
                self.diff_state
                    .update(cx, |state, cx| state.set_files(files, cx));
                self.diff_view = Some(DiffView {
                    request: uuid::Uuid::now_v7(),
                    path: path.into(),
                    name: Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                        .into(),
                    staged: false,
                    state: self.diff_state.clone(),
                    error: None,
                });
                cx.notify();
            }
            Ok(_) => self.set_notice("This edit has no text diff", cx),
            Err(error) => {
                self.set_notice(format!("Could not read the recorded patch: {error}"), cx)
            }
        }
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
        fn task_error(error: String) -> String {
            let lower = error.to_ascii_lowercase();
            if lower.contains("not possible to fast-forward")
                || lower.contains("diverged")
                || lower.contains("conflict")
                || lower.contains("would be overwritten")
            {
                "Pull stopped: local and remote changes conflict. Resolve them manually; nothing was merged."
                    .into()
            } else if lower.contains("non-fast-forward")
                || lower.contains("fetch first")
                || lower.contains("rejected")
            {
                "Push rejected: the remote branch has newer commits. Pull first.".into()
            } else {
                error
            }
        }
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
                        this.repo_summaries.invalidate();
                        this.refresh_project_summaries(cx);
                        this.refresh_file_tree(cx);
                        this.refresh_git_status(cx);
                    }
                    Err(error) => this.set_notice(task_error(error), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    pub(crate) fn set_skin(&mut self, skin: dunlin_ui::Skin, cx: &mut Context<Self>) {
        self.skin = skin;
        dunlin_ui::theme::set_skin(skin, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        cx.set_window_appearance(appearance.native());
        dunlin_ui::theme::set_mode_animated(
            appearance.mode(cx.window_appearance()),
            window,
            self.reduce_motion,
            cx,
        );
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn toggle_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            Appearance::Light
        } else {
            Appearance::Dark
        };
        self.set_appearance(next, window, cx);
    }

    /// Switch the primary surface, restarting the title-bar animation so the
    /// active tab visibly settles into place.
    pub(crate) fn set_view(&mut self, view: MainView, cx: &mut Context<Self>) {
        self.remember_workspace_tab();
        self.workspace_tools.current = None;
        self.workbench.terminal = None;
        self.ensure_chat_tab();
        self.git_history.active = false;
        self.spec.active = false;
        if matches!(
            self.section,
            SidebarSection::Dashboard
                | SidebarSection::Security
                | SidebarSection::SourceControl
                | SidebarSection::Knowledge
        ) {
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
        self.remember_workspace_tab();
        cx.notify();
    }

    /// Snapshot the settings that survive a restart.
    pub(crate) fn stored_settings(&self, cx: &App) -> StoredSettings {
        StoredSettings {
            default_harness: self.default_harness.clone(),
            hidden_harnesses: self.hidden_harnesses.clone(),
            custom_harness_command: self.custom_harness_command.clone(),
            permission_mode: self.permission_mode,
            auto_connect: self.auto_connect,
            restart_harness: self.restart_harness,
            restore_session: self.restore_session,
            word_wrap: self.word_wrap,
            editor_minimap: self.editor_minimap,
            inline_blame: self.inline_blame,
            autosave: self.autosave,
            notifications_enabled: self.notifications_enabled,
            show_shortcut_hints: self.show_shortcut_hints,
            show_file_sizes: self.show_file_sizes,
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
            appearance: Some(self.appearance),
            skin: self.skin,
            view: self.view,
            section: self.section,
            sidebar_tab: Some(self.sidebar_tab),
            last_project_id: self
                .projects
                .get(self.selected_project)
                .map(|project| project.id.clone()),
            last_session_id: self
                .sessions
                .get(self.selected_session)
                .map(|session| session.id.clone()),
            remote_hosts: self.remote_hosts.clone(),
            watched_project_folders: self.watched_project_folders.clone(),
            collapsed_project_groups: self
                .project_search
                .saved_groups(&self.collapsed_project_groups)
                .to_vec(),
            project_sort: self.project_sort,
            project_group_order: self.project_group_order.clone(),
            sidebar_width: self.sidebar_width,
            show_project_branches: self.show_project_branches,
            project_list: self.project_list.clone(),
            sidebar_collapsed: self.sidebar_collapsed,
            workspace_navigation_collapsed: self.workspace_navigation_collapsed,
            general_sessions_collapsed: self.general_sessions_collapsed,
            project_sessions_collapsed: self.project_sessions_collapsed,
            opencode_provider: self.opencode_provider.clone(),
            opencode_free_consent: self.opencode_free_consent,
            recent_models: self.recent_models.clone(),
            group_contexts: self.group_contexts.clone(),
            account_usage_collapsed: self.account_usage_collapsed,
            dashboard_settings: self.dashboard_settings.clone(),
            skills: self.skills.clone(),
            conversation_skills: self.conversation_skills.clone(),
            security_enabled: self.security_enabled,
            security_triage: self.security.triage.clone(),
            vault_enabled: self.vault.requested_enabled,
            last_ai_target: self.last_ai_target.clone(),
            billing_budget_usd: self.billing_budget_usd,
            billing_alerts: self.billing_alerts,
            onboarding_step: self.onboarding_step,
            pending_adapter_install: self.onboarding.plan.clone(),
            density: self.density,
            reduce_motion: self.reduce_motion,
            run_history: self.run.history.clone(),
            bottom_panel: self.bottom_panel,
            bottom_panel_height: self.bottom_panel_height,
            bottom_panel_width: self.bottom_panel_width,
            terminal_dock: self.terminal_dock,
            shell_integration: self.shell_integration,
            _legacy_terminal_ssh_target: String::new(),
            _legacy_terminal_ssh_path: String::new(),
            disabled_plugins: self.plugins.disabled(),
            enabled_plugins: self.plugins.enabled(),
            project_disabled_plugins: self.plugins.project_disabled(),
            project_enabled_plugins: self.plugins.project_enabled(),
            spec_tools_enabled: true,
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
        if self.active_file_tab().is_some_and(|t| {
            t.kind != dunlin_workspace::preview::PreviewKind::Text || t.loading || t.saving
        }) {
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
                        if explicit {this.editor_conflict=None;this.finish_pending_editor(cx);}
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
            self.finish_pending_editor(cx);
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
        if self.active_file_tab().is_some_and(|tab| tab.saving) {
            self.file_notice("Wait for the file to finish saving before closing it", cx);
            return;
        }
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

    pub(crate) fn save_pending_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_editor(cx);
        if !self.editor_dirty {
            self.finish_pending_editor(cx);
        }
    }

    pub(crate) fn discard_pending_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.active_file_tab().is_some_and(|tab| tab.saving) {
            self.file_notice("Wait for the pending save before discarding the buffer", cx);
            return;
        }
        if let Some(id) = self.workbench.active.take() {
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::File(id.clone()));
            self.workbench.tabs.retain(|tab| tab.id != id);
        }
        self.editor_dirty = false;
        self.finish_pending_editor(cx);
    }

    pub(crate) fn finish_pending_editor(&mut self, cx: &mut Context<Self>) {
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
        if let Some(tab) = self.active_file_tab().filter(|tab| tab.untitled) {
            let id = tab.id.clone();
            self.save_untitled_file(&id, cx);
            return;
        }
        self.save_editor_internal(true, self.editor_mtime, cx);
    }

    fn save_editor_internal(
        &mut self,
        announce: bool,
        expected_mtime: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if self.active_file_tab().is_some_and(|tab| tab.untitled) {
            return;
        }
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
        if let Some(id) = self.active_file_tab().map(|tab| tab.id.clone()) {
            self.save_file_tab(&id, expected_mtime, announce, cx);
        }
    }

    pub(crate) fn cancel_editor_conflict(&mut self, cx: &mut Context<Self>) {
        self.editor_conflict = None;
        cx.notify();
    }

    pub(crate) fn reload_editor_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_history.running || self.active_file_tab().is_some_and(|tab| tab.saving) {
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_history.running || self.active_file_tab().is_some_and(|tab| tab.saving) {
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
            self.finish_pending_editor(cx);
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
        if self.active_file_tab().is_some_and(|tab| tab.untitled) {
            return;
        }
        if self.git_history.running || self.active_file_tab().is_some_and(|t| t.git_changed) {
            return;
        }
        if self.active_file_tab().is_some_and(|t| {
            t.kind != dunlin_workspace::preview::PreviewKind::Text || t.loading || t.saving
        }) {
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
                        || this.editor_dirty
                        || this.editor_conflict.is_some()
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
                        // This is an observation, not the version of our buffer.
                        // Only a successful reload may advance the save baseline.
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
        // Do not retain a pasted credential in a hidden form or panel.
        if !self.settings_open {
            self.vault.ui.draft = None;
            self.vault.ui.recipe_draft = None;
            self.vault.ui.operation_draft = None;
        }
        if self.settings_open || self.bottom_panel != BottomPanel::Secrets {
            self.vault.ui.panel.value = None;
        }
        self.sync_vault_read_only();
        self.prepare_markdown_visual(window, cx);
        self.git_history.prune(&self.projects, &self.remote_hosts);
        self.spec.prune(&self.projects);
        self.record_navigation();
        let tabs = &self.workbench.tabs;
        self.workbench
            .preview_cache
            .retain(|id| tabs.iter().any(|tab| tab.id == id));
        self.refresh_minimap(cx);
        self.refresh_inline_blame(cx);
        if let Some(draft) = self.workbench.pending_chat_draft.take() {
            let id = self.session().id.clone();
            self.composer = self.ensure_pane_composer(&id, window, cx);
            self.composer
                .update(cx, |state, cx| state.set_value(draft, window, cx));
        }
        let geometry = window_state::from_bounds(window.window_bounds());
        if !self.factory_resetting && self.last_window_geometry != Some(geometry) {
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
        self.advance_onboarding(cx);
        let p = palette(cx);
        let welcome = self.onboarding_visible();
        let reveal = self
            .onboarding
            .reveal
            .map(|started| (started.elapsed().as_secs_f32() / 0.4).min(1.))
            .unwrap_or(1.);
        let reveal = if self.reduce_motion || cx.reduce_motion() {
            1.
        } else {
            1. - (1. - reveal).powi(3)
        };
        let show_sidebar = !welcome && !self.sidebar_collapsed;
        let (sidebar_fraction, moving) = self.sidebar_motion.sample(show_sidebar);
        if moving || (!welcome && reveal < 1.) {
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        }
        let titlebar = views::titlebar::render(self, window, cx);
        let sidebar = if welcome {
            div().into_any_element()
        } else {
            views::sidebar::render(self, window, cx)
        };
        let main_panel = self.main_panel(window, cx);
        let dock = (!welcome && self.bottom_panel != BottomPanel::Hidden)
            .then(|| self.bottom_panel_view(window, cx));
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
        let status_bar = if welcome {
            None
        } else {
            Some(self.status_bar(cx))
        };
        let detached = crate::overlay_window::prepare(self, window, cx);
        let overlays =
            (!detached && self.has_overlay()).then(|| self.render_overlay_content(window, cx));
        let toast = (!self.has_overlay())
            .then(|| self.toast.as_ref().map(|_| self.toast_overlay(cx)))
            .flatten();

        let chrome = chrome_background(p, crate::platform::reduce_transparency());
        let material = self
            .skin
            .window_background(crate::platform::reduce_transparency());
        if self.skin_window_background != Some(material) {
            window.set_background_appearance(material);
            self.skin_window_background = Some(material);
        }

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(chrome)
            .opacity(if welcome { 1. } else { reveal })
            .text_color(p.fg)
            .font_family(dunlin_ui::theme::ui_font())
            .children(grain_layer(p, 0.))
            .child(self.overlay_backdrop.begin())
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Back, window, cx| {
                    this.navigate_history(false, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Forward, window, cx| {
                    this.navigate_history(true, window, cx);
                }),
            )
            .when(
                !self.factory_resetting && self.onboarding.setup.is_none(),
                |root| {
                    root.on_action(
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
                            this.sidebar_tab = crate::sidebar_layout::Tab::Projects;
                            cx.notify();
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &crate::native_menu::Security, _, cx| {
                            if !this.dismiss_history_for_menu(cx) {
                                return;
                            }
                            this.settings_open = true;
                            this.settings_tab = SettingsTab::Security;
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
                    .on_action(cx.listener(
                        |this, _: &crate::native_menu::ProjectSearch, window, cx| {
                            this.open_project_search(window, cx);
                        },
                    ))
                    .on_action(
                        cx.listener(|this, _: &crate::native_menu::IncreaseText, _, cx| {
                            if !this.has_modal_overlay() {
                                crate::workspace_text::adjust(0.1, cx);
                                cx.notify();
                            }
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &crate::native_menu::DecreaseText, _, cx| {
                            if !this.has_modal_overlay() {
                                crate::workspace_text::adjust(-0.1, cx);
                                cx.notify();
                            }
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &crate::native_menu::ResetText, _, cx| {
                            if !this.has_modal_overlay() {
                                crate::workspace_text::reset(cx);
                                cx.notify();
                            }
                        }),
                    )
                    .when(!self.projects.is_empty(), |root| {
                        root.on_action(cx.listener(
                            |this, _: &crate::native_menu::NewSession, _, cx| {
                                if !this.dismiss_history_for_menu(cx) {
                                    return;
                                }
                                this.new_session(cx)
                            },
                        ))
                        .on_action(cx.listener(|this, _: &crate::native_menu::SaveAll, _, cx| {
                            this.save_all_tabs(cx);
                        }))
                        .on_action(cx.listener(
                            |this, _: &crate::native_menu::ProjectSettings, _, cx| {
                                if !this.dismiss_history_for_menu(cx) {
                                    return;
                                }
                                this.open_project_settings(this.selected_project, cx)
                            },
                        ))
                        .on_action(cx.listener(
                            |this, _: &crate::native_menu::SwitchBranch, window, cx| {
                                if !this.dismiss_history_for_menu(cx) {
                                    return;
                                }
                                this.open_branch_picker(&this.project().id.clone(), window, cx)
                            },
                        ))
                        .on_action(cx.listener(
                            |this, _: &crate::native_menu::RefreshGit, _, cx| {
                                this.refresh_git_status(cx)
                            },
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
                },
            )
            .capture_key_down(cx.listener(Self::on_key_down))
            .key_context(
                if self.view == MainView::Chat
                    && matches!(
                        self.current_workspace_tab(),
                        Some(crate::workspace_panes::TabKey::Chat(_))
                    )
                    && !self.has_overlay()
                {
                    "DunlinChat"
                } else {
                    "Dunlin"
                },
            )
            .capture_action(cx.listener(
                |this, _: &dunlin_ui::component::input::OutdentInline, window, cx| {
                    if !this.cycle_chat_mode(window, cx) {
                        cx.propagate();
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &views::chat_controls::NextAgentMode, window, cx| {
                    if !this.cycle_chat_mode(window, cx) {
                        cx.propagate();
                    }
                },
            ))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.terminal_split_drag(event, cx) {
                    return;
                }
                if this.bottom_panel == crate::app::BottomPanel::Terminal
                    || this.showing_dependency_terminal()
                {
                    this.terminal_pointer_move(event, cx);
                }
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
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.finish_terminal_resize(cx);
                    this.terminal_finish_split_drag(cx);
                    this.terminal_pointer_up(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.finish_terminal_resize(cx);
                    this.terminal_finish_split_drag(cx);
                    this.terminal_pointer_up(cx);
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
                    .pr(px(dunlin_ui::scale::WORKSPACE_INSET))
                    .pl(px(dunlin_ui::scale::WORKSPACE_INSET))
                    .when(!welcome && sidebar_fraction > 0., |this| {
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
                        workspace_surface(p)
                            .id("workspace-surface")
                            .relative()
                            .child(workspace)
                            .children({
                                #[cfg(debug_assertions)]
                                {
                                    crate::workspace_ui_smoke::probe("workspace-surface")
                                }
                                #[cfg(not(debug_assertions))]
                                {
                                    None::<AnyElement>
                                }
                            }),
                    ),
            )
            .children(status_bar)
            .when(self.projects.is_empty(), |root| {
                root.child(
                    div()
                        .h(px(dunlin_ui::scale::WORKSPACE_INSET))
                        .flex_shrink_0(),
                )
            })
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
        if handle
            .read_with(cx, |this, _| this.factory_resetting)
            .unwrap_or(false)
        {
            return false;
        }
        if handle
            .read_with(cx, |this, _| this.session_import.applying)
            .unwrap_or(false)
        {
            let _ = handle.update(cx, |this, cx| {
                this.file_notice("Wait for the session import to finish before quitting", cx)
            });
            return false;
        }
        if handle
            .read_with(cx, |this, _| {
                this.workbench.tabs.iter().any(|tab| tab.saving)
            })
            .unwrap_or(false)
        {
            let _ = handle.update(cx, |this, cx| {
                this.file_notice("Wait for pending file saves before quitting", cx)
            });
            return false;
        }
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

        let answers = if dirty {
            vec![
                PromptButton::new("Save and Quit"),
                PromptButton::new("Discard and Quit"),
                PromptButton::cancel("Cancel"),
            ]
        } else {
            vec![
                PromptButton::cancel("Keep Running"),
                PromptButton::new("Quit Anyway"),
            ]
        };
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
            &answers,
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
            if dirty && answer == Ok(0) {
                let _ = handle.update(cx, |this, cx| {
                    this.save_tabs_then(None, cx, move |this, cx| {
                        if this.has_dirty_tabs()
                            || this.branch_ui.running()
                            || this.git_history.running
                            || this.session_import.applying
                            || this.explorer.busy
                            || this.group_cleanup.busy
                        {
                            return;
                        }
                        confirmed.set(true);
                        cx.quit();
                    });
                });
                return;
            }
            let quit = answer == Ok(1);
            if quit {
                if handle
                    .read_with(cx, |this, _| {
                        this.branch_ui.running()
                            || this.git_history.running
                            || this.session_import.applying
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
        // Global actions can arrive inside this window's dispatch borrow.
        let guard = guard.clone();
        cx.defer(move |cx| {
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
    });
}

#[cfg(test)]
mod tests;
