//! Shared domain vocabulary for Kestrel.
//!
//! These types are deliberately UI-agnostic: they are persisted by the storage
//! layer, produced by the ACP adapter and projected into chat, graph and trace
//! views. Keeping them in one crate prevents the UI from inventing a second
//! model for the same session.

use serde::{Deserialize, Serialize};

pub type Id = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MainView {
    Chat,
    Graph,
    Trace,
}

impl MainView {
    pub const ALL: [MainView; 3] = [MainView::Chat, MainView::Graph, MainView::Trace];

    pub fn label(self) -> &'static str {
        match self {
            MainView::Chat => "Chat",
            MainView::Graph => "Graph",
            MainView::Trace => "Trace",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarSection {
    Projects,
    Files,
    Search,
    SourceControl,
    Sessions,
}

impl SidebarSection {
    pub const ALL: [SidebarSection; 5] = [
        SidebarSection::Projects,
        SidebarSection::Files,
        SidebarSection::Search,
        SidebarSection::SourceControl,
        SidebarSection::Sessions,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SidebarSection::Projects => "Projects",
            SidebarSection::Files => "Explorer",
            SidebarSection::Search => "Search",
            SidebarSection::SourceControl => "Source Control",
            SidebarSection::Sessions => "Sessions",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectTone {
    Blue,
    Green,
    Violet,
    Amber,
    Rose,
    Teal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
    pub tone: ProjectTone,
    pub pinned: bool,
    pub trusted: bool,
    pub running_sessions: usize,
}

impl Project {
    pub fn path_tail(&self) -> &str {
        self.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    Waiting,
    Done,
    Failed,
    Interrupted,
}

impl SessionStatus {
    pub fn label(self) -> &'static str {
        match self {
            SessionStatus::Running => "Running",
            SessionStatus::Waiting => "Waiting",
            SessionStatus::Done => "Done",
            SessionStatus::Failed => "Failed",
            SessionStatus::Interrupted => "Interrupted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: Id,
    pub project_id: Id,
    pub title: String,
    pub status: SessionStatus,
    pub harness: String,
    pub model: String,
    pub branch: Option<String>,
    pub updated_at: String,
    pub cost_usd: f64,
    pub duration_secs: u64,
    pub tokens: u64,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageSource {
    Live,
    Session,
    Estimated,
    Unavailable,
}

impl UsageSource {
    pub fn label(self) -> &'static str {
        match self {
            UsageSource::Live => "live",
            UsageSource::Session => "session",
            UsageSource::Estimated => "estimated",
            UsageSource::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f32,
    pub resets_in: String,
    pub source: UsageSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub plan: String,
    pub accent: ProjectTone,
    pub windows: Vec<UsageWindow>,
    pub detail: String,
}

impl ProviderUsage {
    pub fn primary(&self) -> Option<&UsageWindow> {
        self.windows.first()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Running => "Running",
            AgentStatus::Waiting => "Waiting",
            AgentStatus::Done => "Done",
            AgentStatus::Failed => "Failed",
            AgentStatus::Stopped => "Stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Orchestrator,
    Subagent,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentNode {
    pub id: Id,
    pub parent: Option<Id>,
    pub title: String,
    pub task: String,
    pub model: String,
    pub harness: String,
    pub status: AgentStatus,
    pub kind: AgentKind,
    pub tools: Vec<ToolKind>,
    pub cost_usd: f64,
    pub duration_secs: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    Edit,
    Shell,
    Search,
    Web,
    Task,
    Test,
    Plan,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Read => "Read",
            ToolKind::Edit => "Edit",
            ToolKind::Shell => "Shell",
            ToolKind::Search => "Search",
            ToolKind::Web => "Web",
            ToolKind::Task => "Task",
            ToolKind::Test => "Test",
            ToolKind::Plan => "Plan",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanKind {
    Agent,
    Turn,
    Llm,
    Tool,
    Permission,
    Waiting,
    Handoff,
    Compaction,
}

impl SpanKind {
    pub fn label(self) -> &'static str {
        match self {
            SpanKind::Agent => "agent",
            SpanKind::Turn => "turn",
            SpanKind::Llm => "llm",
            SpanKind::Tool => "tool",
            SpanKind::Permission => "permission",
            SpanKind::Waiting => "waiting",
            SpanKind::Handoff => "handoff",
            SpanKind::Compaction => "compaction",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanStatus {
    Running,
    Done,
    Failed,
    Waiting,
}

impl SpanStatus {
    pub fn label(self) -> &'static str {
        match self {
            SpanStatus::Running => "running",
            SpanStatus::Done => "done",
            SpanStatus::Failed => "failed",
            SpanStatus::Waiting => "waiting",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceSpan {
    pub id: Id,
    pub parent: Option<Id>,
    pub name: String,
    pub agent: String,
    pub kind: SpanKind,
    pub status: SpanStatus,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub depth: u32,
    pub cost_usd: f64,
    pub tokens: u64,
    pub detail: String,
}

impl TraceSpan {
    pub fn end_ms(&self) -> u64 {
        self.start_ms + self.duration_ms
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Waiting,
    Done,
    Failed,
}

impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            ToolStatus::Running => "running",
            ToolStatus::Waiting => "waiting",
            ToolStatus::Done => "done",
            ToolStatus::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Id,
    pub kind: ToolKind,
    pub title: String,
    pub target: String,
    pub status: ToolStatus,
    pub detail: String,
    pub duration_ms: u64,
    pub additions: u32,
    pub deletions: u32,
}
