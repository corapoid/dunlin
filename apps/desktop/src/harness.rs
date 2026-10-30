//! Live ACP harness state and the bridge between the worker thread and GPUI.
//!
//! The UI never touches protocol I/O directly. It keeps a transcript, forwards
//! prompts to [`AcpHarness`], and folds streaming events back into that
//! transcript. Permission prompts round-trip through the same path.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dunlin_agent::{
    AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate, PermissionChoice, PermissionKind,
    PlanItem, PlanStatus, ToolCallDelta, ToolCallView,
};
use dunlin_domain::{PermissionMode, SessionStatus, ToolStatus};
use dunlin_ui::*;
use serde::{Deserialize, Serialize};

use crate::app::{DunlinApp, PendingRemoteSession};
use crate::logging;

/// One rendered entry in the conversation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct TurnTiming {
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub usage: Option<dunlin_agent::TokenUsage>,
    #[serde(default)]
    pub usage_before: Option<dunlin_agent::TokenUsage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum ChatItem {
    User {
        text: String,
        #[serde(default)]
        timing: Option<TurnTiming>,
        #[serde(default)]
        at_ms: u64,
    },
    AgentText {
        text: String,
        streaming: bool,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Thought {
        text: String,
        streaming: bool,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Tool {
        view: ToolCallView,
        duration_ms: u64,
        #[serde(default)]
        at_ms: u64,
    },
    Plan {
        items: Vec<PlanItem>,
        #[serde(default)]
        at_ms: u64,
    },
    Permission {
        request_id: String,
        title: String,
        options: Vec<PermissionChoice>,
        resolved: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Error {
        message: String,
        #[serde(default)]
        at_ms: u64,
    },
    Child {
        id: String,
        parent_id: String,
        title: Option<String>,
        items: Vec<ChatItem>,
        #[serde(default)]
        at_ms: u64,
    },
}

/// Wall-clock timestamp used to project transcripts into traces.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Render a conversation as Markdown for the clipboard.
pub(crate) fn transcript_markdown(items: &[ChatItem]) -> String {
    let mut out = String::new();
    for item in items {
        match item {
            ChatItem::User { text, .. } => {
                out.push_str("## You\n\n");
                out.push_str(text);
                out.push_str("\n\n");
            }
            ChatItem::AgentText { text, .. } => {
                out.push_str("## Agent\n\n");
                out.push_str(text);
                out.push_str("\n\n");
            }
            ChatItem::Thought { text, .. } => {
                out.push_str("### Reasoning\n\n");
                out.push_str(text);
                out.push_str("\n\n");
            }
            ChatItem::Tool {
                view, duration_ms, ..
            } => {
                out.push_str(&format!(
                    "- **{}** · {} · {} ms\n",
                    view.title,
                    view.status.label(),
                    duration_ms
                ));
            }
            ChatItem::Plan { items, .. } => {
                out.push_str("### Plan\n\n");
                for step in items {
                    let mark = match step.status {
                        PlanStatus::Pending => "[ ]",
                        PlanStatus::InProgress => "[~]",
                        PlanStatus::Completed => "[x]",
                    };
                    out.push_str(&format!("{} {}\n", mark, step.content));
                }
                out.push('\n');
            }
            ChatItem::Permission {
                title, resolved, ..
            } => {
                out.push_str(&format!(
                    "- **Permission** · {} · {}\n",
                    title,
                    resolved.as_deref().unwrap_or("pending")
                ));
            }
            ChatItem::Error { message, .. } => {
                out.push_str(&format!("> **Error:** {message}\n\n"));
            }
            ChatItem::Child {
                title, id, items, ..
            } => {
                out.push_str(&format!(
                    "### Subagent · {}\n\n{}\n\n",
                    title.as_deref().unwrap_or(id),
                    transcript_markdown(items)
                ));
            }
        }
    }
    out.trim_end().to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HarnessStatus {
    Offline,
    Starting,
    Ready { agent: String, version: String },
    Failed(String),
    Exited(String),
}

impl HarnessStatus {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, HarnessStatus::Ready { .. })
    }

    pub(crate) fn is_busy(&self) -> bool {
        matches!(self, HarnessStatus::Starting)
    }

    pub(crate) fn label(&self) -> String {
        match self {
            HarnessStatus::Offline => "Harness offline".into(),
            HarnessStatus::Starting => "Connecting harness…".into(),
            HarnessStatus::Ready { agent, .. } => format!("{agent} ready"),
            HarnessStatus::Failed(_) => "Harness failed".into(),
            HarnessStatus::Exited(_) => "Harness exited".into(),
        }
    }

    pub(crate) fn detail(&self) -> Option<String> {
        match self {
            HarnessStatus::Failed(reason) | HarnessStatus::Exited(reason) => Some(reason.clone()),
            HarnessStatus::Ready { agent, version } => Some(format!("{agent} {version} · ACP")),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HarnessUsage {
    pub used: u64,
    pub size: u64,
}

/// A prompt waiting for the current turn to finish. The id is stable so the
/// queue panel can edit, remove and reorder entries without relying on text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueuedPrompt {
    pub id: u64,
    pub text: String,
    pub created_at_ms: u64,
}

impl HarnessUsage {
    pub(crate) fn percent(&self) -> f32 {
        if self.size == 0 {
            0.0
        } else {
            (self.used as f32 / self.size as f32).clamp(0.0, 1.0)
        }
    }
}

pub(crate) struct ParkedRuntime {
    pub project: String,
    pub root: String,
    pub state: HarnessState,
}

fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(crate) struct HarnessState {
    pub harness: Option<AcpHarness>,
    pub status: HarnessStatus,
    pub transcript: Vec<ChatItem>,
    transcripts: HashMap<String, Vec<ChatItem>>,
    pub usage: Option<HarnessUsage>,
    pub commands: Vec<String>,
    connected_harness: Option<String>,
    pub models: Option<dunlin_agent::SessionModels>,
    pub modes: Option<dunlin_agent::SessionModes>,
    pub model_changing: bool,
    pub mode_changing: bool,
    pub missing_command: Option<String>,
    pub local_session_loading: bool,
    pub running: bool,
    pub session_id: Option<String>,
    pending_prompts: VecDeque<QueuedPrompt>,
    next_prompt_id: u64,
    prompt_sent: bool,
    active_turn: Option<(usize, Instant)>,
    token_baseline: Option<dunlin_agent::TokenUsage>,
    generation: u64,
    pub(crate) started_tools: HashMap<String, Instant>,
    restarts: u8,
}

impl Default for HarnessState {
    fn default() -> Self {
        Self {
            harness: None,
            status: HarnessStatus::Offline,
            models: None,
            modes: None,
            model_changing: false,
            mode_changing: false,
            missing_command: None,
            local_session_loading: false,
            transcript: Vec::new(),
            transcripts: HashMap::new(),
            usage: None,
            commands: Vec::new(),
            connected_harness: None,
            running: false,
            session_id: None,
            pending_prompts: VecDeque::new(),
            next_prompt_id: 0,
            prompt_sent: false,
            active_turn: None,
            token_baseline: None,
            generation: 0,
            started_tools: HashMap::new(),
            restarts: 0,
        }
    }
}

impl HarnessState {
    fn finish_turn_timing(&mut self, at_ms: u64) {
        if let Some((index, start)) = self.active_turn.take()
            && let Some(ChatItem::User {
                timing: Some(timing),
                ..
            }) = self.transcript.get_mut(index)
        {
            timing.finished_at_ms = Some(at_ms);
            timing.elapsed_ms = Some(start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        }
    }

    fn push_error(&mut self, message: String) {
        if matches!(self.transcript.last(), Some(ChatItem::Error { message: previous, .. }) if previous == &message)
        {
            return;
        }
        self.transcript.push(ChatItem::Error {
            message,
            at_ms: now_ms(),
        });
    }

    pub(crate) fn queued_prompts(&self) -> Vec<QueuedPrompt> {
        self.pending_prompts.iter().cloned().collect()
    }

    pub(crate) fn remove_queued_prompt(&mut self, id: u64) -> Option<QueuedPrompt> {
        self.pending_prompts
            .iter()
            .position(|prompt| prompt.id == id)
            .and_then(|index| self.pending_prompts.remove(index))
    }

    pub(crate) fn move_queued_prompt(&mut self, id: u64, delta: isize) -> bool {
        let Some(index) = self
            .pending_prompts
            .iter()
            .position(|prompt| prompt.id == id)
        else {
            return false;
        };
        let target = (index as isize + delta)
            .clamp(0, self.pending_prompts.len().saturating_sub(1) as isize)
            as usize;
        if target == index {
            return false;
        }
        let prompt = match self.pending_prompts.remove(index) {
            Some(prompt) => prompt,
            None => return false,
        };
        self.pending_prompts.insert(target, prompt);
        true
    }

    fn enqueue_prompt(&mut self, text: String) -> QueuedPrompt {
        self.next_prompt_id = self.next_prompt_id.wrapping_add(1);
        let prompt = QueuedPrompt {
            id: self.next_prompt_id,
            text,
            created_at_ms: now_ms(),
        };
        self.pending_prompts.push_back(prompt.clone());
        prompt
    }

    pub(crate) fn clear_queued_prompts(&mut self) -> usize {
        let count = self.pending_prompts.len();
        self.pending_prompts.clear();
        count
    }
}

impl DunlinApp {
    /// Start a fresh ACP process or remote connection for the selected project.
    pub(crate) fn connect_harness(&mut self, cx: &mut Context<Self>) {
        if self.harness.status.is_busy() {
            return;
        }
        // Navigation is never blocked by the process budget. Idle background
        // adapters can be evicted; running/waiting agents must not be killed.
        if self
            .parked_runtimes
            .values()
            .filter(|r| r.state.harness.is_some())
            .count()
            >= 7
        {
            let idle = self
                .parked_runtimes
                .iter()
                .find(|(_, r)| {
                    r.state.harness.is_some() && !r.state.running && !r.state.status.is_busy()
                })
                .map(|(id, _)| id.clone());
            if let Some(id) = idle {
                self.stop_parked_runtime(&id);
            } else {
                self.set_notice("Eight agents are already connected. Finish an agent before connecting another.", cx);
                return;
            }
        }

        let project = self.project().clone();
        if !project.trusted {
            let reason = format!(
                "{} is not trusted. Trust it from the project menu first.",
                project.path
            );
            self.harness.status = HarnessStatus::Failed(reason.clone());
            self.notice = Some(reason);
            cx.notify();
            return;
        }

        if let Some(remote_project) = project.remote.clone() {
            let Some(host) = self.remote_host(&remote_project.host_id) else {
                let reason = "Remote server is not configured".to_string();
                self.harness.status = HarnessStatus::Failed(reason.clone());
                self.notice = Some(reason);
                cx.notify();
                return;
            };
            let token = match self.remote_token(&host.id) {
                Ok(token) => token,
                Err(error) => {
                    self.harness.status = HarnessStatus::Failed(error.clone());
                    self.notice = Some(error);
                    cx.notify();
                    return;
                }
            };
            logging::info(&format!(
                "connecting to relay {} · project {}",
                host.url, remote_project.project_id
            ));
            if self.pending_remote_session.is_none() && !self.remote_sessions_loading {
                let current = self.session().clone();
                self.pending_remote_session = Some(match current.acp_session_id {
                    Some(session_id) => PendingRemoteSession::Load { session_id },
                    None => PendingRemoteSession::New,
                });
            }
            self.start_harness(
                HarnessConfig::remote(
                    host.url.clone(),
                    token,
                    remote_project.project_id.clone(),
                    PathBuf::from(&project.path),
                ),
                Some(host.name),
                cx,
            );
            return;
        }

        let cwd = PathBuf::from(&project.path);
        if !cwd.is_dir() {
            let reason = format!("Folder is not available: {}", project.path);
            self.harness.status = HarnessStatus::Failed(reason.clone());
            self.notice = Some(reason);
            cx.notify();
            return;
        }

        let command = std::env::var("DUNLIN_HARNESS_COMMAND")
            .ok()
            .filter(|command| !command.trim().is_empty())
            .unwrap_or_else(|| {
                harness_command(&self.session_harness(), &self.custom_harness_command)
            });
        if !command_available(&command) {
            let reason = format!(
                "{} needs an installed ACP adapter: {command}. Install it or choose an available harness.",
                self.session_harness()
            );
            // Configuration problems are not conversation messages and cannot
            // be fixed by repeatedly spawning the same missing executable.
            self.disconnect_harness(cx);
            self.harness.missing_command = Some(command);
            self.harness.pending_prompts.clear();
            logging::warn(&reason);
            self.notice = Some(reason);
            cx.notify();
            return;
        }
        let command = crate::harness_discovery::resolved_command(&command).unwrap_or(command);
        let mut config = HarnessConfig::new(command, cwd);
        if let Some(path) = crate::harness_discovery::child_path() {
            config.process_env.insert("PATH".into(), path);
        }
        config.resume_session_id = self.session().acp_session_id.clone();
        self.start_harness(config, None, cx);
    }

    fn start_harness(
        &mut self,
        config: HarnessConfig,
        remote_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.harness.generation = next_generation();
        let generation = self.harness.generation;
        self.harness.missing_command = None;
        if let Some(previous) = self.harness.harness.take() {
            previous.shutdown();
        }

        let description = config.description();
        logging::info(&format!(
            "starting harness: {description} · cwd {}",
            config.cwd.display()
        ));
        self.harness.local_session_loading = !config.is_remote();
        self.harness.connected_harness = (!config.is_remote()).then(|| self.session_harness());
        self.harness.session_id = config.resume_session_id.clone();
        self.harness.models = None;
        self.harness.modes = None;
        self.harness.mode_changing = false;
        self.harness.model_changing = false;
        let harness = AcpHarness::spawn(config);
        let events = harness.events().clone();
        self.harness.harness = Some(harness);
        self.harness.status = HarnessStatus::Starting;
        self.harness.running = false;
        self.harness.commands.clear();
        self.notice = Some(match remote_name {
            Some(name) => format!("Connecting to {name}…"),
            None => format!("Starting {description}…"),
        });

        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let keep_going = this.update(cx, |this, cx| {
                    this.route_harness_event(generation, event, cx)
                });
                if !matches!(keep_going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    pub(crate) fn disconnect_harness(&mut self, cx: &mut Context<Self>) {
        self.cancel_ai_commit_preparation();
        self.interrupt_live_turn(cx);
        logging::info("disconnecting harness");
        self.harness.generation = next_generation();
        if let Some(harness) = self.harness.harness.take() {
            harness.shutdown();
        }
        self.harness.status = HarnessStatus::Offline;
        self.harness.missing_command = None;
        self.harness.connected_harness = None;
        self.harness.models = None;
        self.harness.modes = None;
        self.harness.mode_changing = false;
        self.harness.model_changing = false;
        self.harness.local_session_loading = false;
        self.harness.session_id = None;
        self.harness.running = false;
        self.notice = Some("Harness disconnected".into());
        cx.notify();
    }

    pub(crate) fn restart_harness(&mut self, cx: &mut Context<Self>) {
        self.cancel_ai_commit_preparation();
        self.interrupt_live_turn(cx);
        if let Some(harness) = self.harness.harness.take() {
            harness.shutdown();
        }
        self.harness.status = HarnessStatus::Offline;
        self.harness.session_id = None;
        self.harness.running = false;
        self.harness.restarts = 0;
        self.connect_harness(cx);
    }

    pub(crate) fn has_running_agents(&self) -> bool {
        self.harness.running || self.parked_runtimes.values().any(|r| r.state.running)
    }

    pub(crate) fn live_agents(&self) -> Vec<(String, String, &'static str)> {
        let label = |state: &HarnessState, waiting: bool| {
            if waiting && state.running {
                Some("Waiting for permission")
            } else if state.running {
                Some("Working")
            } else if state.status.is_busy() || state.local_session_loading {
                Some("Connecting")
            } else {
                None
            }
        };
        let mut rows = Vec::new();
        if let Some(label) = label(
            &self.harness,
            self.session().status == SessionStatus::Waiting,
        ) {
            rows.push((self.project().id.clone(), self.session().id.clone(), label));
        }
        for (id, runtime) in &self.parked_runtimes {
            let waiting = self
                .all_sessions
                .iter()
                .any(|s| &s.id == id && s.status == SessionStatus::Waiting);
            if let Some(label) = label(&runtime.state, waiting) {
                rows.push((runtime.project.clone(), id.clone(), label));
            }
        }
        rows
    }

    pub(crate) fn park_current_runtime(&mut self, cx: &mut Context<Self>) {
        self.branch_ui.dismiss();
        self.cancel_ai_commit_preparation();
        if self.sessions.get(self.selected_session).is_none() {
            return;
        }
        if self.project().is_remote() {
            // Relay leases still use the existing single-connection lifecycle.
            self.disconnect_harness(cx);
            self.stash_transcript();
            return;
        }
        self.save_current_transcript();
        let id = self.session().id.clone();
        self.project_session_selections
            .insert(self.project().id.clone(), id.clone());
        if self.harness.harness.is_none() && !self.harness.running && !self.harness.status.is_busy()
        {
            self.harness = Default::default();
            return;
        }
        let runtime = ParkedRuntime {
            project: self.project().id.clone(),
            root: self.project().path.clone(),
            state: std::mem::take(&mut self.harness),
        };
        self.parked_runtimes.insert(id, runtime);
    }

    pub(crate) fn restore_runtime(&mut self, id: &str) -> bool {
        let Some(runtime) = self.parked_runtimes.remove(id) else {
            return false;
        };
        if runtime.project != self.project().id || runtime.root != self.project().path {
            return false;
        }
        self.harness = runtime.state;
        true
    }

    pub(crate) fn stop_parked_runtime(&mut self, id: &str) {
        if let Some(mut runtime) = self.parked_runtimes.remove(id) {
            runtime.state.finish_turn_timing(now_ms());
            if let Some(adapter) = &runtime.state.harness {
                if runtime.state.running {
                    adapter.cancel();
                }
                adapter.shutdown();
            }
            if let Some(store) = &self.store
                && let Err(error) = store.save_transcript(id, &runtime.state.transcript)
            {
                self.report_storage_error("save background conversation", error);
            }
            if runtime.state.running
                && let Some(session) = self.all_sessions.iter_mut().find(|s| s.id == id)
            {
                session.status = SessionStatus::Interrupted;
                let session = session.clone();
                self.persist_session(&session);
            }
        }
    }

    pub(crate) fn stop_project_runtimes(&mut self, project: &str, cx: &mut Context<Self>) {
        if self.project().id == project
            || self
                .group_contexts
                .get(&self.session().id)
                .is_some_and(|context| context.roots.iter().any(|r| r.id == project))
        {
            self.disconnect_harness(cx);
        }
        let ids: Vec<_> = self
            .parked_runtimes
            .iter()
            .filter(|(id, r)| {
                r.project == project
                    || self
                        .group_contexts
                        .get(*id)
                        .is_some_and(|context| context.roots.iter().any(|r| r.id == project))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.stop_parked_runtime(&id);
        }
    }

    pub(crate) fn save_and_shutdown_background_agents(&self) {
        if let Some(store) = &self.store {
            for (id, runtime) in &self.parked_runtimes {
                if let Err(error) = store.save_transcript(id, &runtime.state.transcript) {
                    self.report_storage_error("save background conversation", error);
                }
            }
        }
        for runtime in self.parked_runtimes.values() {
            if let Some(adapter) = &runtime.state.harness {
                if runtime.state.running {
                    adapter.cancel();
                }
                adapter.shutdown();
            }
        }
    }

    /// Background queues pause at turn boundaries. Resume only after returning
    /// to this session and revalidating its provider/model and MCP state.
    pub(crate) fn resume_visible_queue(&mut self, cx: &mut Context<Self>) {
        if !self.harness.running
            && !self.harness.pending_prompts.is_empty()
            && self.harness.status.is_ready()
            && !self.harness.local_session_loading
            && !self.harness.model_changing
            && !self.harness.mode_changing
            && !self.mcp_busy()
            && (!self.local_opencode()
                || self
                    .harness
                    .models
                    .as_ref()
                    .is_some_and(|m| self.visible_models().iter().any(|c| c.id == m.current)))
        {
            self.harness.running = true;
            self.update_active_session(|s| s.status = SessionStatus::Running);
            self.dispatch_pending_prompt(cx);
        }
    }

    pub(crate) fn background_transcripts(&self) -> HashMap<String, Vec<ChatItem>> {
        self.parked_runtimes
            .iter()
            .map(|(id, runtime)| (id.clone(), runtime.state.transcript.clone()))
            .collect()
    }

    fn route_harness_event(
        &mut self,
        generation: u64,
        event: HarnessEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.harness.generation == generation {
            self.handle_harness_event(event, cx);
            return true;
        }
        let Some(id) = self
            .parked_runtimes
            .iter()
            .find(|(_, r)| r.state.generation == generation)
            .map(|(id, _)| id.clone())
        else {
            return false;
        };
        let mut runtime = self.parked_runtimes.remove(&id).unwrap();
        let Some(project_index) = self
            .projects
            .iter()
            .position(|p| p.id == runtime.project && p.path == runtime.root && p.trusted)
        else {
            return false;
        };
        let Some(session) = self.all_sessions.iter().find(|s| s.id == id).cloned() else {
            return false;
        };
        // A synchronous scope only: no UI rendering or async continuation can
        // observe this selection. Reduced background handling never refreshes
        // visible editors/providers/MCP or dispatches queued prompts.
        let project = std::mem::replace(&mut self.selected_project, project_index);
        let sessions = std::mem::replace(&mut self.sessions, vec![session]);
        let selected = std::mem::replace(&mut self.selected_session, 0);
        let notice = self.notice.clone();
        std::mem::swap(&mut self.harness, &mut runtime.state);
        self.handle_background_event(event, cx);
        std::mem::swap(&mut self.harness, &mut runtime.state);
        self.selected_project = project;
        self.sessions = sessions;
        self.selected_session = selected;
        for session in &mut self.sessions {
            if let Some(updated) = self.all_sessions.iter().find(|s| s.id == session.id) {
                *session = updated.clone();
            }
        }
        self.notice = notice;
        self.parked_runtimes.insert(id, runtime);
        cx.notify();
        true
    }

    fn handle_background_event(&mut self, event: HarnessEvent, cx: &mut Context<Self>) {
        match event {
            HarnessEvent::Ready { agent, version } => {
                self.harness.status = HarnessStatus::Ready { agent, version }
            }
            HarnessEvent::SessionCreated { session_id } => {
                self.harness.token_baseline = Some(Default::default());
                self.harness.local_session_loading = false;
                self.harness.session_id = Some(session_id.clone());
                self.update_active_session(|s| s.acp_session_id = Some(session_id));
            }
            HarnessEvent::SessionLoaded { session_id } if self.is_current_session(&session_id) => {
                self.harness.local_session_loading = false;
                self.harness.session_id = Some(session_id);
                self.harness.token_baseline = None;
            }
            HarnessEvent::Update { session_id, update }
                if self.is_current_session(&session_id) && !self.harness.local_session_loading =>
            {
                self.apply_update(update)
            }
            HarnessEvent::Models { session_id, state } if self.is_current_session(&session_id) => {
                self.apply_models(state)
            }
            HarnessEvent::Modes { session_id, state } if self.is_current_session(&session_id) => {
                self.apply_modes(state)
            }
            HarnessEvent::ModelChangeFailed { message, .. } => {
                self.harness.model_changing = false;
                self.harness.push_error(message);
            }
            HarnessEvent::ModeChangeFailed { message, .. } => {
                self.harness.mode_changing = false;
                self.harness.push_error(message);
            }
            HarnessEvent::PermissionRequested {
                request_id,
                session_id,
                title,
                options,
            } => {
                if self.is_current_session(&session_id) && self.group_context_ready(cx) {
                    self.handle_permission(request_id, title, options);
                } else if let Some(adapter) = &self.harness.harness {
                    adapter.respond_permission(request_id, None);
                }
            }
            HarnessEvent::PromptFinished {
                session_id,
                stop_reason,
                usage,
            } if self.is_current_session(&session_id) => {
                if let Some((index, _)) = self.harness.active_turn
                    && let Some(ChatItem::User {
                        timing: Some(t), ..
                    }) = self.harness.transcript.get_mut(index)
                {
                    t.usage = usage;
                }
                self.harness.token_baseline = usage;
                self.harness.running = false;
                self.harness.prompt_sent = false;
                self.finish_streaming();
                self.update_active_session(|s| {
                    s.status = if stop_reason == "cancelled" {
                        SessionStatus::Interrupted
                    } else {
                        SessionStatus::Done
                    }
                });
                self.save_current_transcript();
                self.notify_if_background("Agent finished", &self.session().title, cx);
            }
            HarnessEvent::SessionLoadFailed { message, .. } | HarnessEvent::Error { message } => {
                self.harness.local_session_loading = false;
                self.harness.running = false;
                self.harness.prompt_sent = false;
                self.harness.pending_prompts.clear();
                self.finish_streaming();
                self.harness.push_error(message.clone());
                self.harness.status = HarnessStatus::Failed(message);
                self.update_active_session(|s| s.status = SessionStatus::Failed);
                self.save_current_transcript();
            }
            HarnessEvent::Exited { reason } => {
                self.harness.local_session_loading = false;
                self.harness.running = false;
                self.harness.prompt_sent = false;
                self.harness.pending_prompts.clear();
                self.finish_streaming();
                self.harness.harness = None;
                self.harness.session_id = None;
                self.harness.status = HarnessStatus::Exited(reason);
                self.update_active_session(|s| s.status = SessionStatus::Failed);
                self.save_current_transcript();
            }
            _ => {}
        }
    }

    /// Send the composer contents to the active harness, connecting lazily.
    pub(crate) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_commit_preparing() {
            self.set_notice(
                "Wait for AI commit preparation or cancel it before sending another request",
                cx,
            );
            return;
        }
        if self.mcp_busy() {
            self.set_notice(
                "Wait for MCP selection to finish updating before sending",
                cx,
            );
            return;
        }
        if self.harness.local_session_loading
            || self.harness.model_changing
            || self.harness.mode_changing
        {
            self.set_notice(
                "Wait for the session or model to finish loading before sending",
                cx,
            );
            return;
        }
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self.guard_ai_commit_agent_send(cx)
            || !self.group_context_ready(cx)
            || !self.check_provider_model(cx)
        {
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.submit_prompt(text, cx);
        cx.notify();
    }

    pub(crate) fn reorder_queued_prompt(&mut self, id: u64, delta: isize, cx: &mut Context<Self>) {
        if self.harness.move_queued_prompt(id, delta) {
            cx.notify();
        }
    }

    pub(crate) fn remove_queued_prompt(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.harness.remove_queued_prompt(id).is_some() {
            if self.harness.queued_prompts().is_empty() {
                self.queue_open = false;
            }
            cx.notify();
        }
    }

    pub(crate) fn clear_queued_prompts(&mut self, cx: &mut Context<Self>) {
        let count = self.harness.clear_queued_prompts();
        if count > 0 {
            self.queue_open = false;
            self.notice = Some(format!(
                "Cleared {count} queued {}",
                if count == 1 { "prompt" } else { "prompts" }
            ));
            cx.notify();
        }
    }

    pub(crate) fn edit_queued_prompt(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.harness.remove_queued_prompt(id) else {
            return;
        };
        self.composer.update(cx, |state, cx| {
            state.set_value(prompt.text, window, cx);
            state.focus(window, cx);
        });
        self.queue_open = false;
        cx.notify();
    }

    /// Re-run a previous user prompt as a new turn.
    pub(crate) fn retry_prompt(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        self.send_prompt(window, cx);
    }

    /// Load a previous prompt into the composer for editing without sending it.
    pub(crate) fn edit_prompt(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        self.queue_open = false;
        self.notice = Some("Editing a previous prompt · send to re-run it".into());
        cx.notify();
    }

    /// Append a user message, update session metadata and dispatch it.
    fn submit_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        if !self.guard_ai_commit_agent_send(cx)
            || !self.group_context_ready(cx)
            || !self.check_provider_model(cx)
        {
            return;
        }
        if self.harness.local_session_loading
            || self.harness.model_changing
            || self.harness.mode_changing
        {
            self.set_notice(
                "Wait for the session or model to finish loading before sending",
                cx,
            );
            return;
        }
        let title = session_title(&text);
        self.remember_dashboard_project(cx);
        let summary = session_summary(&text);
        let was_running = self.harness.running;
        let queued = self.harness.enqueue_prompt(text.clone());
        self.chat_scroll.jump_to_latest();
        self.harness.transcript.push(ChatItem::User {
            text: text.clone(),
            at_ms: queued.created_at_ms,
            timing: None,
        });
        self.harness.running = true;
        if !was_running {
            self.harness.prompt_sent = false;
        }
        self.update_active_session(|session| {
            session.status = SessionStatus::Running;
            session.summary = summary;
            if session.title == "New session" {
                session.title = title;
            }
        });
        self.save_current_transcript();

        if self.harness.status.is_ready() && self.harness.session_id.is_some() {
            self.dispatch_pending_prompt(cx);
        } else if !self.harness.status.is_busy() && !self.remote_sessions_loading {
            self.connect_harness(cx);
        }
    }

    pub(crate) fn submit_automation_prompt(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        // Automation may only dispatch immediately, never enter a queue which
        // could resume later under a different model or after cancellation.
        if self.harness.running
            || !self.harness.status.is_ready()
            || self.harness.harness.is_none()
            || self.harness.session_id.is_none()
            || !self.harness.pending_prompts.is_empty()
            || self.mcp_busy()
        {
            return false;
        }
        self.submit_prompt(text, cx);
        self.harness.prompt_sent
    }

    pub(crate) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        self.cancel_ai_commit_preparation();
        self.interrupt_live_turn(cx);
        self.notice = Some("Cancellation requested".into());
        cx.notify();
    }

    /// Stop any in-flight turn and clear the session's live status.
    ///
    /// Called before switching projects, sessions or harnesses so the previous
    /// conversation is not left marked as running forever.
    pub(crate) fn interrupt_live_turn(&mut self, cx: &mut Context<Self>) {
        let live = self.harness.running || !self.harness.pending_prompts.is_empty();
        if !live {
            return;
        }
        if self.harness.running
            && self.harness.session_id.is_some()
            && let Some(harness) = self.harness.harness.as_ref()
        {
            harness.cancel();
        }
        self.harness.running = false;
        self.harness.prompt_sent = false;
        self.harness.pending_prompts.clear();
        self.finish_streaming();
        self.update_active_session(|session| {
            if matches!(
                session.status,
                SessionStatus::Running | SessionStatus::Waiting
            ) {
                session.status = SessionStatus::Interrupted;
            }
        });
        cx.notify();
    }

    pub(crate) fn resolve_permission(
        &mut self,
        request_id: &str,
        option_id: Option<String>,
        label: String,
        cx: &mut Context<Self>,
    ) {
        if !self.group_context_ready(cx) {
            return;
        }
        if option_id.as_ref().is_some_and(|id| self.effective_permission_mode() == PermissionMode::ReadOnly && self.harness.transcript.iter().any(|item| matches!(item, ChatItem::Permission { request_id: request, options, .. } if request == request_id && options.iter().any(|o| &o.id == id && matches!(o.kind, PermissionKind::AllowOnce | PermissionKind::AllowAlways))))) {
            self.set_notice("Read-only policy does not allow approving this operation", cx); return;
        }
        if let Some(harness) = self.harness.harness.as_ref() {
            harness.respond_permission(request_id.to_string(), option_id);
        }
        for item in self.harness.transcript.iter_mut() {
            if let ChatItem::Permission {
                request_id: id,
                resolved,
                ..
            } = item
                && id == request_id
            {
                *resolved = Some(label.clone());
                break;
            }
        }
        self.harness.running = true;
        self.update_active_session(|session| {
            session.status = SessionStatus::Running;
        });
        self.save_current_transcript();
        self.notice = Some(label);
        cx.notify();
    }

    pub(crate) fn stash_transcript(&mut self) {
        if self.sessions.get(self.selected_session).is_none() {
            self.harness.transcript.clear();
            return;
        }
        let current_id = self.session().id.clone();
        let current = std::mem::take(&mut self.harness.transcript);
        if let Some(store) = &self.store
            && let Err(error) = store.save_transcript(&current_id, &current)
        {
            self.report_storage_error("save conversation", error);
        }
        self.upload_remote_transcript(&current_id, &current);
        self.harness.transcripts.insert(current_id, current);
    }

    /// Drop a cached transcript without touching the store.
    pub(crate) fn forget_transcript(&mut self, session_id: &str) {
        self.harness.transcripts.remove(session_id);
    }

    /// Drop every cached transcript, for a full history reset.
    pub(crate) fn forget_all_transcripts(&mut self) {
        self.harness.transcripts.clear();
    }

    pub(crate) fn load_transcript(&mut self, session_id: &str) {
        self.harness.missing_command = None;
        self.selected_node = None;
        self.selected_span = None;
        let loaded = self.harness.transcripts.remove(session_id).or_else(|| {
            self.store.as_ref().and_then(|store| {
                match store.load_transcript::<Vec<ChatItem>>(session_id) {
                    Ok(transcript) => transcript,
                    Err(error) => {
                        self.report_storage_error("load conversation", error);
                        None
                    }
                }
            })
        });
        if let Some(items) = &loaded {
            self.session_list.record(session_id, items);
        }
        self.harness.transcript = loaded.unwrap_or_default();
        self.harness.started_tools.clear();
        self.harness.active_turn = None;
        self.harness.pending_prompts.clear();
        self.harness.prompt_sent = false;
        self.harness.running = false;
        self.harness.usage = None;
        self.harness.models = None;
        self.harness.modes = None;
        self.harness.mode_changing = false;
        self.harness.model_changing = false;
        self.harness.local_session_loading = false;
    }

    /// Switch the visible conversation and resume its ACP context when available.
    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.sessions.len() || index == self.selected_session {
            return;
        }
        self.cancel_ai_commit_preparation();
        if !self.project().is_remote() {
            self.park_current_runtime(cx);
            self.reset_mcp_for_navigation();
            self.selected_session = index;
            let id = self.session().id.clone();
            if !self.restore_runtime(&id) {
                self.load_transcript(&id);
                if self.auto_connect {
                    self.connect_harness(cx);
                }
            }
            self.session_menu_open = false;
            self.refresh_mcp(cx);
            self.refresh_providers(cx);
            self.resume_visible_queue(cx);
            self.ensure_usage(cx);
            self.persist_settings(cx);
            cx.notify();
            return;
        }
        if self.remote_sessions_loading {
            return;
        }
        if self.harness.local_session_loading || self.harness.status.is_busy() {
            self.set_notice("Wait for the harness to finish connecting", cx);
            return;
        }
        self.session_menu_open = false;
        self.interrupt_live_turn(cx);
        self.stash_transcript();
        self.selected_session = index;
        let session_id = self.session().id.clone();
        let title = self.session().title.clone();
        self.load_transcript(&session_id);
        self.harness.session_id = None;
        self.update_active_session(|session| {
            if session.status == SessionStatus::Running {
                session.status = SessionStatus::Interrupted;
            }
        });

        if self.project().is_remote() {
            let acp_id = self
                .session()
                .acp_session_id
                .clone()
                .unwrap_or_else(|| self.session().id.clone());
            if self.harness.status.is_ready() {
                self.start_remote_session(PendingRemoteSession::Load { session_id: acp_id }, cx);
            } else if self.auto_connect && !self.harness.status.is_busy() {
                self.pending_remote_session =
                    Some(PendingRemoteSession::Load { session_id: acp_id });
                self.connect_harness(cx);
            }
        } else if self.harness.status.is_ready() {
            self.activate_local_session(cx);
        } else if self.auto_connect && !self.harness.status.is_busy() {
            self.connect_harness(cx);
        }

        self.notice = Some(format!("Opened “{title}”"));
        self.ensure_usage(cx);
        self.persist_settings(cx);
        cx.notify();
    }

    fn is_current_session(&self, session_id: &str) -> bool {
        self.harness.session_id.as_deref() == Some(session_id)
    }

    pub(crate) fn activate_local_session(&mut self, cx: &mut Context<Self>) {
        if self.harness.connected_harness.as_deref() != Some(self.session_harness().as_str()) {
            self.restart_harness(cx);
            return;
        }
        let path = PathBuf::from(&self.project().path);
        let existing = self.session().acp_session_id.clone();
        self.harness.local_session_loading = true;
        self.harness.session_id = existing.clone();
        self.harness.models = None;
        self.harness.modes = None;
        self.harness.mode_changing = false;
        self.harness.model_changing = false;
        if let Some(harness) = self.harness.harness.as_ref() {
            if let Some(id) = existing {
                harness.load_session(id, path);
            } else {
                harness.new_session(path);
            }
        }
    }

    /// Notify only when the window is in the background and the user kept
    /// desktop notifications enabled. The Dock badge is updated separately so
    /// it stays visible while the app is active too.
    fn notify_if_background(&self, title: &str, body: &str, cx: &App) {
        if self.notifications_enabled && !self.app_active {
            crate::platform::notify(cx, title, body);
        }
    }

    /// Rescan the explorer and flag a clean editor for reload after a turn.
    fn refresh_workspace_after_turn(&mut self, cx: &mut Context<Self>) {
        self.refresh_file_tree(cx);
        self.editor_stale = self.inspector.is_some() && !self.editor_dirty;
    }

    fn dispatch_pending_prompt(&mut self, cx: &mut Context<Self>) {
        if self.harness.prompt_sent
            || self.harness.session_id.is_none()
            || self.harness.local_session_loading
            || self.harness.pending_prompts.is_empty()
            || self.mcp_busy()
        {
            return;
        }
        // Recheck at the wire boundary as well: a resumed session or later ACP
        // update must not turn a queued Free prompt into a paid-model send.
        if !self.guard_ai_commit_agent_send(cx)
            || !self.group_context_ready(cx)
            || !self.check_provider_model(cx)
        {
            self.harness.running = false;
            self.update_active_session(|session| session.status = SessionStatus::Interrupted);
            return;
        }
        let Some(prompt) = self.harness.pending_prompts.pop_front() else {
            return;
        };
        let text = prompt.text;
        let wire_text = self
            .group_contexts
            .get(&self.session().id)
            .map(|context| context.prompt(&text))
            .unwrap_or_else(|| text.clone());
        let title = session_title(&text);
        self.update_active_session(|session| {
            if session.title == "New session" {
                session.title = title.clone();
            }
        });
        let sent = self
            .harness
            .harness
            .as_ref()
            .map(|harness| harness.prompt(wire_text))
            .unwrap_or(false);
        if sent {
            let index = self.harness.transcript.iter().position(|item| matches!(item, ChatItem::User { text: body, at_ms, timing: None } if body == &text && *at_ms == prompt.created_at_ms));
            if let Some(index) = index {
                if let ChatItem::User { timing, .. } = &mut self.harness.transcript[index] {
                    *timing = Some(TurnTiming {
                        started_at_ms: now_ms(),
                        finished_at_ms: None,
                        elapsed_ms: None,
                        usage: None,
                        // Consume the baseline: a failed/interrupted request with no
                        // final counters must not be charged to a later request.
                        usage_before: self.harness.token_baseline.take(),
                    });
                }
                self.harness.active_turn = Some((index, Instant::now()));
            }
            // A harness without model metadata must invalidate the previous
            // target too, not silently reuse an older harness/model for commits.
            self.last_ai_target = Some(crate::ai_commit::LastTarget {
                harness: self.session_harness(),
                model: self
                    .harness
                    .models
                    .as_ref()
                    .map(|models| models.current.clone())
                    .unwrap_or_default(),
                provider: self.selected_provider().map(str::to_string),
                remote: self.project().is_remote(),
            });
            if let Some(models) = &self.harness.models {
                let id = models.current.clone();
                let scope = self.model_scope();
                crate::providers::remember(&mut self.recent_models, &scope, &id);
            }
            self.persist_settings(cx);
            self.harness.prompt_sent = true;
            self.harness.running = true;
            self.update_active_session(|session| {
                if session.status != SessionStatus::Running {
                    session.status = SessionStatus::Running;
                }
            });
        } else {
            self.harness.transcript.push(ChatItem::Error {
                message: "Could not send the prompt: the harness is not running.".into(),
                at_ms: now_ms(),
            });
            self.harness.running = false;
            self.harness.prompt_sent = false;
            self.harness.pending_prompts.clear();
        }
    }

    fn handle_harness_event(&mut self, event: HarnessEvent, cx: &mut Context<Self>) {
        if std::env::var_os("DUNLIN_TRACE_HARNESS").is_some() {
            eprintln!("[harness] {event:?}");
        }
        logging::harness_event(&event);
        match event {
            HarnessEvent::Ready { agent, version } => {
                self.harness.status = HarnessStatus::Ready {
                    agent: agent.clone(),
                    version: version.clone(),
                };
                self.harness.restarts = 0;
                self.notice = Some(format!("{agent} {version} connected over ACP"));
                self.refresh_providers(cx);
                self.refresh_usage(cx);
                if self.project().is_remote()
                    && let Some(action) = self.pending_remote_session.take()
                {
                    self.start_remote_session(action, cx);
                } else if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt(cx);
                }
            }
            HarnessEvent::SessionCreated { session_id } => {
                self.harness.token_baseline = Some(Default::default());
                self.harness.local_session_loading = false;
                if self.provider_catalog.providers.is_empty()
                    || self.provider_catalog.error.is_some()
                {
                    self.refresh_providers(cx);
                }
                if self.project().is_remote() {
                    self.create_remote_session_summary(&session_id, cx);
                } else {
                    self.update_active_session(|session| {
                        session.acp_session_id = Some(session_id.clone());
                    });
                }
                self.harness.session_id = Some(session_id);
                self.refresh_mcp(cx);
                self.persist_settings(cx);
                if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt(cx);
                }
            }
            HarnessEvent::SessionLoaded { session_id } => {
                if !self.is_current_session(&session_id) {
                    return;
                }
                // Work may have happened on another client since our last snapshot.
                self.harness.token_baseline = None;
                self.harness.local_session_loading = false;
                self.harness.session_id = Some(session_id);
                self.refresh_mcp(cx);
                if self.provider_catalog.providers.is_empty()
                    || self.provider_catalog.error.is_some()
                {
                    self.refresh_providers(cx);
                }
                self.update_active_session(|session| {
                    if matches!(
                        session.status,
                        SessionStatus::Waiting | SessionStatus::Running
                    ) {
                        session.status = SessionStatus::Done;
                    }
                });
                if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt(cx);
                }
            }
            HarnessEvent::Update {
                session_id, update, ..
            } => {
                if self.is_current_session(&session_id) && !self.harness.local_session_loading {
                    self.apply_update(update);
                }
            }
            HarnessEvent::SessionLoadFailed {
                session_id,
                message,
            } => {
                if self.is_current_session(&session_id) {
                    self.harness.local_session_loading = false;
                    self.harness.session_id = None;
                    self.harness.running = false;
                    self.harness.prompt_sent = false;
                    self.harness.pending_prompts.clear();
                    self.update_active_session(|session| {
                        session.status = SessionStatus::Interrupted
                    });
                    self.notice = Some(message);
                }
            }
            HarnessEvent::Models { session_id, state } => {
                if self.is_current_session(&session_id) {
                    self.apply_models(state);
                    self.ensure_usage(cx);
                }
            }
            HarnessEvent::ModelChangeFailed {
                session_id,
                message,
            } => {
                if self.is_current_session(&session_id) {
                    self.harness.model_changing = false;
                    self.notice = Some(message);
                }
            }
            HarnessEvent::Modes { session_id, state } => {
                if self.is_current_session(&session_id) {
                    self.apply_modes(state);
                }
            }
            HarnessEvent::ModeChangeFailed {
                session_id,
                message,
            } => {
                if self.is_current_session(&session_id) {
                    self.harness.mode_changing = false;
                    self.notice = Some(message);
                }
            }
            HarnessEvent::PermissionRequested {
                request_id,
                session_id,
                title,
                options,
            } => {
                if self.is_current_session(&session_id) {
                    let session_title = self.session().title.clone();
                    if !self.group_context_ready(cx) {
                        if let Some(adapter) = &self.harness.harness {
                            adapter.respond_permission(request_id, None);
                        }
                        return;
                    }
                    self.handle_permission(request_id, title.clone(), options);
                    self.notify_if_background(
                        "Permission requested",
                        &format!("{title} · {session_title}"),
                        cx,
                    );
                } else if let Some(harness) = self.harness.harness.as_ref() {
                    // A prompt from a session we left behind must not block the
                    // agent forever; decline it and let the cancellation settle.
                    harness.respond_permission(request_id, None);
                }
            }
            HarnessEvent::PromptFinished {
                session_id,
                stop_reason,
                usage,
            } => {
                if self.is_current_session(&session_id) {
                    if let Some((index, _)) = self.harness.active_turn
                        && let Some(ChatItem::User {
                            timing: Some(timing),
                            ..
                        }) = self.harness.transcript.get_mut(index)
                    {
                        timing.usage = usage;
                    }
                    self.harness.token_baseline = usage;
                    self.harness.running = false;
                    self.harness.prompt_sent = false;
                    self.finish_streaming();
                    let cancelled = stop_reason == "cancelled";
                    let session_title = self.session().title.clone();
                    self.update_active_session(|session| {
                        session.status = if cancelled {
                            SessionStatus::Interrupted
                        } else {
                            SessionStatus::Done
                        };
                    });
                    self.notice = Some(if cancelled {
                        "Turn cancelled".into()
                    } else {
                        "Turn complete".into()
                    });
                    self.notify_if_background(
                        if cancelled {
                            "Turn cancelled"
                        } else {
                            "Turn complete"
                        },
                        &session_title,
                        cx,
                    );
                    self.save_current_transcript();
                    self.refresh_workspace_after_turn(cx);
                    self.refresh_git_status(cx);
                    self.refresh_usage(cx);
                    self.refresh_mcp(cx);
                    self.dispatch_pending_prompt(cx);
                }
            }
            HarnessEvent::Error { message } => {
                let was_running = self.harness.running;
                self.harness.running = false;
                self.harness.prompt_sent = false;
                self.harness.pending_prompts.clear();
                self.finish_streaming();
                if message.contains("Could not load session") {
                    self.harness.session_id = None;
                }
                if message.contains("Could not create session") {
                    self.sessions
                        .retain(|session| !session.id.starts_with("pending-"));
                    self.all_sessions
                        .retain(|session| !session.id.starts_with("pending-"));
                    self.selected_session = 0;
                }
                self.harness.push_error(message.clone());
                self.notify_if_background("Dunlin needs attention", &message, cx);
                if matches!(self.harness.status, HarnessStatus::Starting) {
                    self.harness.status = HarnessStatus::Failed(message.clone());
                    self.update_active_session(|session| {
                        session.status = SessionStatus::Failed;
                    });
                } else if was_running {
                    self.update_active_session(|session| {
                        session.status = SessionStatus::Failed;
                    });
                }
                self.save_current_transcript();
                self.notice = Some(if message.contains("Session is active on another device") {
                    "Session is active on another device · use Take over session".into()
                } else {
                    message
                });
                self.refresh_workspace_after_turn(cx);
                self.refresh_git_status(cx);
            }
            HarnessEvent::Exited { reason } => {
                let retry = should_restart(
                    &self.harness.status,
                    self.restart_harness,
                    self.harness.restarts,
                );
                self.harness.connected_harness = None;
                self.harness.models = None;
                self.harness.model_changing = false;
                self.harness.modes = None;
                self.harness.mode_changing = false;
                self.harness.local_session_loading = false;
                self.harness.harness = None;
                self.harness.session_id = None;
                self.harness.running = false;
                self.harness.prompt_sent = false;
                self.harness.pending_prompts.clear();
                self.finish_streaming();
                self.harness.status = HarnessStatus::Exited(reason.clone());
                self.update_active_session(|session| {
                    session.status = SessionStatus::Failed;
                });
                self.save_current_transcript();
                self.notice = Some(format!("Harness exited: {reason}"));

                if retry {
                    self.harness.restarts += 1;
                    let generation = self.harness.generation;
                    let project = self.project().id.clone();
                    let session = self.session().id.clone();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1200))
                            .await;
                        this.update(cx, |this, cx| {
                            if this.harness.generation == generation
                                && this.project().id == project
                                && this.session().id == session
                                && matches!(this.harness.status, HarnessStatus::Exited(_))
                            {
                                this.connect_harness(cx);
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    fn apply_update(&mut self, update: HarnessUpdate) {
        match update {
            HarnessUpdate::MessageChunk { id, text, thought } => {
                append_identified(
                    &mut self.harness.transcript,
                    if thought {
                        StreamKind::Thought
                    } else {
                        StreamKind::Message
                    },
                    text,
                    now_ms(),
                    Some(id),
                );
            }
            HarnessUpdate::Child {
                id,
                parent_id,
                title,
                update,
            } => {
                append_child(
                    &mut self.harness.transcript,
                    id,
                    parent_id,
                    title,
                    *update,
                    now_ms(),
                );
            }
            HarnessUpdate::UserMessage(text) => {
                self.harness.transcript.push(ChatItem::User {
                    text,
                    timing: None,
                    at_ms: now_ms(),
                });
            }
            HarnessUpdate::Configuration { models, modes } => {
                if models.is_some() {
                    self.apply_models(models);
                }
                if modes.is_some() {
                    self.apply_modes(modes);
                }
            }
            HarnessUpdate::AgentMessage(text) => {
                append_stream(
                    &mut self.harness.transcript,
                    StreamKind::Message,
                    text,
                    now_ms(),
                );
            }
            HarnessUpdate::AgentThought(text) => {
                append_stream(
                    &mut self.harness.transcript,
                    StreamKind::Thought,
                    text,
                    now_ms(),
                );
            }
            HarnessUpdate::ToolCall(view) => {
                self.harness
                    .started_tools
                    .insert(view.id.clone(), Instant::now());
                self.harness.transcript.push(ChatItem::Tool {
                    view,
                    duration_ms: 0,
                    at_ms: now_ms(),
                });
            }
            HarnessUpdate::ToolCallUpdate(delta) => self.apply_tool_delta(delta),
            HarnessUpdate::Plan(items) => {
                if let Some(ChatItem::Plan {
                    items: existing, ..
                }) = self
                    .harness
                    .transcript
                    .iter_mut()
                    .rev()
                    .find(|item| matches!(item, ChatItem::Plan { .. }))
                {
                    *existing = items;
                } else {
                    self.harness.transcript.push(ChatItem::Plan {
                        items,
                        at_ms: now_ms(),
                    });
                }
            }
            HarnessUpdate::Usage {
                used,
                size,
                cost_usd,
            } => {
                self.harness.usage = Some(HarnessUsage { used, size });
                self.update_active_session(|session| {
                    session.tokens = used;
                    if let Some(cost) = cost_usd {
                        session.cost_usd = cost;
                    }
                });
            }
            HarnessUpdate::AvailableCommands(commands) => {
                self.harness.commands = commands;
            }
        }
    }

    fn apply_models(&mut self, mut state: Option<dunlin_agent::SessionModels>) {
        if let Some(models) = state.as_mut() {
            if models.choices.is_empty()
                && let Some(previous) = self.harness.models.as_ref()
                && previous.config_id == models.config_id
            {
                models.choices = previous.choices.clone();
            }
            let label = models
                .choices
                .iter()
                .find(|choice| choice.id == models.current)
                .map(|choice| choice.name.clone())
                .unwrap_or_else(|| models.current.clone());
            if self.harness.model_changing {
                self.notice = Some(format!("Model changed to {label}"));
            }
            self.update_active_session(|session| session.model = label);
        }
        self.harness.models = state;
        self.harness.model_changing = false;
    }

    pub(crate) fn chat_controls_busy(&self) -> bool {
        self.mcp_busy()
            || self.harness.running
            || self.harness.status.is_busy()
            || self.harness.local_session_loading
            || self.harness.model_changing
            || self.harness.mode_changing
            || self.remote_sessions_loading
    }

    fn apply_modes(&mut self, mut state: Option<dunlin_agent::SessionModes>) {
        if let Some(modes) = state.as_mut() {
            if modes.choices.is_empty()
                && let Some(previous) = &self.harness.modes
            {
                modes.choices = previous.choices.clone();
                modes.config_id = previous.config_id.clone();
            }
            if self.harness.mode_changing {
                self.notice = Some(format!("Mode changed to {}", modes.label()));
            }
        }
        self.harness.modes = state;
        self.harness.mode_changing = false;
    }

    pub(crate) fn select_mode(&mut self, mode_id: &str, cx: &mut Context<Self>) {
        if self.chat_controls_busy() {
            return;
        }
        let Some(modes) = &self.harness.modes else {
            return;
        };
        if modes.current == mode_id {
            return;
        }
        if !modes.choices.iter().any(|choice| choice.id == mode_id) {
            return;
        }
        if let (Some(harness), Some(session_id)) = (&self.harness.harness, &self.harness.session_id)
        {
            if harness.set_mode(
                session_id.clone(),
                modes.config_id.clone(),
                mode_id.to_string(),
            ) {
                self.harness.mode_changing = true;
                self.notice = Some("Changing mode…".into());
            } else {
                self.notice = Some("Could not change mode: the harness is disconnected".into());
            }
        }
        cx.notify();
    }

    pub(crate) fn select_chat_harness(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.chat_controls_busy()
            || self.project().is_remote()
            || self.projects.is_empty()
            || self.session_harness() == name
        {
            return;
        }
        // Agent session ids cannot be moved between different harnesses.
        // Keep the current conversation intact and explicitly start a new one.
        self.disconnect_harness(cx);
        self.set_project_harness(self.selected_project, Some(name.to_string()), cx);
        self.new_session(cx);
        if !self.auto_connect {
            self.connect_harness(cx);
        }
    }

    pub(crate) fn select_model(&mut self, model_id: &str, cx: &mut Context<Self>) {
        if self.chat_controls_busy() {
            return;
        }
        if !self
            .visible_models()
            .iter()
            .any(|choice| choice.id == model_id)
        {
            return;
        }
        let Some(models) = self.harness.models.as_ref() else {
            return;
        };
        if models.current == model_id {
            return;
        }
        if !models.choices.iter().any(|choice| choice.id == model_id) {
            return;
        }
        if let (Some(harness), Some(session_id)) = (&self.harness.harness, &self.harness.session_id)
        {
            if harness.set_model(
                session_id.clone(),
                models.config_id.clone(),
                model_id.to_string(),
            ) {
                self.harness.model_changing = true;
                self.notice = Some("Changing model…".into());
            } else {
                self.notice = Some("Could not change model: the harness is disconnected".into());
            }
        }
        cx.notify();
    }

    fn apply_tool_delta(&mut self, delta: ToolCallDelta) {
        let finished = delta
            .status
            .map(|status| matches!(status, ToolStatus::Done | ToolStatus::Failed))
            .unwrap_or(false);

        for item in self.harness.transcript.iter_mut().rev() {
            let ChatItem::Tool {
                view, duration_ms, ..
            } = item
            else {
                continue;
            };
            if view.id != delta.id {
                continue;
            }
            if let Some(title) = delta.title {
                view.title = title;
            }
            if let Some(kind) = delta.kind {
                view.kind = kind;
            }
            if let Some(status) = delta.status {
                view.status = status;
            }
            if let Some(detail) = delta.detail {
                view.detail = detail;
            }
            if let Some(additions) = delta.additions {
                view.additions = additions;
            }
            if let Some(deletions) = delta.deletions {
                view.deletions = deletions;
            }
            if let Some(diff) = delta.diff {
                view.diff = Some(diff);
            }
            if finished && let Some(started) = self.harness.started_tools.remove(&delta.id) {
                *duration_ms = started.elapsed().as_millis() as u64;
            }
            return;
        }
    }

    fn handle_permission(
        &mut self,
        request_id: String,
        title: String,
        options: Vec<PermissionChoice>,
    ) {
        let auto = match self.effective_permission_mode() {
            PermissionMode::AutoApprove => options
                .iter()
                .find(|option| option.kind == PermissionKind::AllowAlways)
                .or_else(|| {
                    options
                        .iter()
                        .find(|option| option.kind == PermissionKind::AllowOnce)
                })
                .cloned(),
            PermissionMode::ReadOnly => options
                .iter()
                .find(|option| {
                    matches!(
                        option.kind,
                        PermissionKind::RejectAlways | PermissionKind::RejectOnce
                    )
                })
                .cloned(),
            PermissionMode::Ask => None,
        };

        if let Some(choice) = auto {
            let label = if matches!(
                choice.kind,
                PermissionKind::RejectAlways | PermissionKind::RejectOnce
            ) {
                format!("Denied · {}", choice.label)
            } else {
                format!("Allowed · {}", choice.label)
            };
            if let Some(harness) = self.harness.harness.as_ref() {
                harness.respond_permission(request_id.clone(), Some(choice.id.clone()));
            }
            self.harness.transcript.push(ChatItem::Permission {
                request_id,
                title,
                options,
                resolved: Some(label.clone()),
                at_ms: now_ms(),
            });
            self.save_current_transcript();
            self.notice = Some(label);
            return;
        }

        self.harness.transcript.push(ChatItem::Permission {
            request_id,
            title,
            options,
            resolved: None,
            at_ms: now_ms(),
        });
        self.update_active_session(|session| {
            session.status = SessionStatus::Waiting;
        });
        self.save_current_transcript();
        self.notice = Some("Permission requested".into());
    }

    fn finish_streaming(&mut self) {
        self.harness.finish_turn_timing(now_ms());
        fn finish(items: &mut [ChatItem]) {
            for item in items {
                match item {
                    ChatItem::AgentText { streaming, .. } | ChatItem::Thought { streaming, .. } => {
                        *streaming = false;
                    }
                    ChatItem::Child { items, .. } => finish(items),
                    _ => {}
                }
            }
        }
        finish(&mut self.harness.transcript);
    }

    pub(crate) fn update_active_session(
        &mut self,
        update: impl FnOnce(&mut dunlin_domain::SessionSummary),
    ) {
        let Some(session) = self.sessions.get_mut(self.selected_session) else {
            return;
        };
        update(session);
        let stored = session.clone();
        if let Some(entry) = self
            .all_sessions
            .iter_mut()
            .find(|candidate| candidate.id == stored.id)
        {
            *entry = stored.clone();
        }
        self.persist_session(&stored);
    }

    pub(crate) fn persist_session(&self, session: &dunlin_domain::SessionSummary) {
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_session(session)
        {
            self.report_storage_error("save session", error);
        }
    }

    pub(crate) fn save_current_transcript(&mut self) {
        if crate::session_list::has_user_message(&self.harness.transcript)
            || !crate::session_list::historical_evidence(self.session())
        {
            self.session_list
                .record(&self.session().id.clone(), &self.harness.transcript);
        }
        if self.sessions.get(self.selected_session).is_none() {
            return;
        }
        let session_id = self.session().id.clone();
        let transcript = self.harness.transcript.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.save_transcript(&session_id, &transcript)
        {
            self.report_storage_error("save conversation", error);
        }
        self.upload_remote_transcript(&session_id, &transcript);
    }

    /// Mirror the rendered transcript to the relay so another device or a
    /// backup can recover the conversation even when OpenCode is unavailable.
    fn upload_remote_transcript(&mut self, session_id: &str, transcript: &[ChatItem]) {
        if transcript.is_empty() {
            return;
        }
        let Some(session) = self
            .all_sessions
            .iter()
            .find(|session| {
                session.id == session_id || session.acp_session_id.as_deref() == Some(session_id)
            })
            .cloned()
        else {
            return;
        };
        let Some(host_id) = session.origin.host_id().map(str::to_string) else {
            return;
        };
        let Some(host) = self.remote_host(&host_id) else {
            return;
        };
        let Ok(token) = self.remote_token(&host_id) else {
            return;
        };
        let Ok(payload) = serde_json::to_value(transcript) else {
            return;
        };
        let session_id = session
            .acp_session_id
            .clone()
            .unwrap_or_else(|| session.id.clone());
        std::thread::spawn(move || {
            if let Err(error) =
                crate::remote::save_transcript(&host.url, &token, &session_id, &payload)
            {
                crate::logging::warn(&format!("remote transcript: {error}"));
            }
        });
    }

    pub(crate) fn save_stashed_transcripts(&self) {
        if let Some(store) = &self.store {
            for (session_id, transcript) in &self.harness.transcripts {
                if let Err(error) = store.save_transcript(session_id, transcript) {
                    self.report_storage_error("save conversation", error);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum StreamKind {
    Message,
    Thought,
}

fn append_stream(transcript: &mut Vec<ChatItem>, kind: StreamKind, text: String, at_ms: u64) {
    append_identified(transcript, kind, text, at_ms, None);
}

fn append_identified(
    transcript: &mut Vec<ChatItem>,
    kind: StreamKind,
    text: String,
    at_ms: u64,
    id: Option<String>,
) {
    if let Some(id) = &id {
        for item in transcript.iter_mut().rev() {
            if matches!(item, ChatItem::User { .. }) {
                break;
            }
            match (kind, item) {
                (
                    StreamKind::Message,
                    ChatItem::AgentText {
                        text: body,
                        message_id: Some(existing),
                        ..
                    },
                )
                | (
                    StreamKind::Thought,
                    ChatItem::Thought {
                        text: body,
                        message_id: Some(existing),
                        ..
                    },
                ) if existing == id => {
                    body.push_str(&text);
                    return;
                }
                _ => {}
            }
        }
    }
    match (kind, transcript.last_mut()) {
        (
            StreamKind::Message,
            Some(ChatItem::AgentText {
                text: body,
                streaming,
                message_id,
                ..
            }),
        ) if *streaming && *message_id == id => {
            body.push_str(&text);
            return;
        }
        (
            StreamKind::Thought,
            Some(ChatItem::Thought {
                text: body,
                streaming,
                message_id,
                ..
            }),
        ) if *streaming && *message_id == id => {
            body.push_str(&text);
            return;
        }
        _ => {}
    }

    match kind {
        StreamKind::Message => transcript.push(ChatItem::AgentText {
            text,
            streaming: true,
            message_id: id,
            at_ms,
        }),
        StreamKind::Thought => transcript.push(ChatItem::Thought {
            text,
            streaming: true,
            message_id: id,
            at_ms,
        }),
    }
}

fn append_child(
    transcript: &mut Vec<ChatItem>,
    id: String,
    parent_id: String,
    title: Option<String>,
    update: HarnessUpdate,
    at_ms: u64,
) {
    if !transcript
        .iter()
        .any(|item| matches!(item, ChatItem::Child { id:existing, .. } if existing == &id))
    {
        transcript.push(ChatItem::Child {
            id: id.clone(),
            parent_id,
            title: title.clone(),
            items: Vec::new(),
            at_ms,
        });
    }
    let Some(ChatItem::Child {
        items,
        title: current,
        ..
    }) = transcript
        .iter_mut()
        .find(|item| matches!(item, ChatItem::Child { id:existing, .. } if existing == &id))
    else {
        return;
    };
    if title.is_some() {
        *current = title;
    }
    match update {
        HarnessUpdate::AgentMessage(text) => append_stream(items, StreamKind::Message, text, at_ms),
        HarnessUpdate::AgentThought(text) => append_stream(items, StreamKind::Thought, text, at_ms),
        HarnessUpdate::MessageChunk { id, text, thought } => append_identified(
            items,
            if thought {
                StreamKind::Thought
            } else {
                StreamKind::Message
            },
            text,
            at_ms,
            Some(id),
        ),
        HarnessUpdate::ToolCall(view) => items.push(ChatItem::Tool {
            view,
            duration_ms: 0,
            at_ms,
        }),
        HarnessUpdate::ToolCallUpdate(delta) => {
            if let Some(ChatItem::Tool { view, .. }) = items
                .iter_mut()
                .find(|item| matches!(item, ChatItem::Tool {view,..} if view.id == delta.id))
            {
                if let Some(title) = delta.title {
                    view.title = title;
                }
                if let Some(status) = delta.status {
                    view.status = status;
                }
                if let Some(detail) = delta.detail {
                    view.detail = detail;
                }
            }
        }
        HarnessUpdate::Plan(plan) => items.push(ChatItem::Plan { items: plan, at_ms }),
        HarnessUpdate::Child {
            id,
            parent_id,
            title,
            update,
        } => append_child(items, id, parent_id, title, *update, at_ms),
        _ => {}
    }
}

fn session_title(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or(prompt).trim();
    let mut title = String::new();
    for word in line.split_whitespace() {
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
        if title.chars().count() > 52 {
            break;
        }
    }
    if title.is_empty() {
        "New session".into()
    } else if title.chars().count() > 52 {
        format!("{}…", title.chars().take(52).collect::<String>())
    } else {
        title
    }
}

/// Collapse a prompt into the one-line description shown on graph and trace roots.
fn session_summary(prompt: &str) -> String {
    let text = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 180 {
        format!("{}…", text.chars().take(180).collect::<String>())
    } else {
        text
    }
}

pub(crate) fn harness_command(name: &str, custom: &str) -> String {
    match name {
        "OpenCode" => "opencode acp".into(),
        "Claude Code" => "claude-agent-acp".into(),
        "OpenAI Codex" => "codex-acp".into(),
        "Custom ACP" => {
            let custom = custom.trim();
            if custom.is_empty() {
                "your-agent --acp".into()
            } else {
                custom.to_string()
            }
        }
        other => other.to_string(),
    }
}

fn should_restart(status: &HarnessStatus, enabled: bool, attempts: u8) -> bool {
    enabled && attempts < 1 && status.is_ready()
}

/// Check whether the executable named by an ACP command is on `PATH`.
pub(crate) fn command_available(command: &str) -> bool {
    crate::harness_discovery::resolved_command(command).is_some()
}

pub(crate) fn plan_status_label(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Pending => "queued",
        PlanStatus::InProgress => "in progress",
        PlanStatus::Completed => "done",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatItem, HarnessState, HarnessStatus, command_available, harness_command, session_summary,
        session_title, should_restart, transcript_markdown,
    };

    #[test]
    fn runtime_generations_are_unique_across_rapid_switches_and_restarts() {
        let generations: std::collections::HashSet<_> =
            (0..10_000).map(|_| super::next_generation()).collect();
        assert_eq!(generations.len(), 10_000);
        assert!(!generations.contains(&HarnessState::default().generation));
    }

    #[test]
    fn parking_preserves_live_timing_queue_and_models_instead_of_cancelling() {
        let mut active = HarnessState {
            generation: super::next_generation(),
            running: true,
            session_id: Some("owner-session".into()),
            ..Default::default()
        };
        active.enqueue_prompt("queued follow-up".into());
        active.active_turn = Some((0, std::time::Instant::now()));
        active
            .started_tools
            .insert("tool".into(), std::time::Instant::now());
        let owner = active.generation;
        let mut parked = std::mem::take(&mut active);
        assert_ne!(active.generation, owner);
        assert!(parked.running);
        assert!(parked.active_turn.is_some());
        active.generation = super::next_generation();
        assert_ne!(active.generation, parked.generation);
        std::mem::swap(&mut active, &mut parked);
        assert_eq!(active.session_id.as_deref(), Some("owner-session"));
        assert!(active.started_tools.contains_key("tool"));
        assert_eq!(active.queued_prompts()[0].text, "queued follow-up");
    }

    #[test]
    fn turn_timing_is_persistent_and_only_finishes_the_dispatched_prompt() {
        let mut state = HarnessState::default();
        state.transcript.push(ChatItem::User {
            text: "active".into(),
            at_ms: 100,
            timing: Some(super::TurnTiming {
                started_at_ms: 200,
                finished_at_ms: None,
                elapsed_ms: None,
                usage: None,
                usage_before: None,
            }),
        });
        state.transcript.push(ChatItem::User {
            text: "queued".into(),
            at_ms: 300,
            timing: None,
        });
        state.active_turn = Some((
            0,
            std::time::Instant::now() - std::time::Duration::from_millis(1200),
        ));
        state.finish_turn_timing(1500);
        assert!(
            matches!(&state.transcript[0], ChatItem::User { timing: Some(timing), .. } if timing.finished_at_ms == Some(1500) && timing.elapsed_ms.unwrap() >= 1200)
        );
        assert!(matches!(
            &state.transcript[1],
            ChatItem::User { timing: None, .. }
        ));
        state.finish_turn_timing(2000);
        let restored: Vec<ChatItem> =
            serde_json::from_str(&serde_json::to_string(&state.transcript).unwrap()).unwrap();
        assert!(
            matches!(&restored[0], ChatItem::User { timing: Some(timing), .. } if timing.finished_at_ms == Some(1500))
        );
        let legacy: Vec<ChatItem> =
            serde_json::from_str(r#"[{"User":{"text":"legacy"}}]"#).unwrap();
        assert!(matches!(
            &legacy[0],
            ChatItem::User {
                timing: None,
                at_ms: 0,
                ..
            }
        ));
    }

    #[test]
    fn interleaved_reasoning_and_child_chunks_do_not_split_or_pollute_root_markdown() {
        use dunlin_agent::HarnessUpdate;
        let mut transcript = Vec::new();
        super::append_identified(
            &mut transcript,
            super::StreamKind::Message,
            "# Wea".into(),
            1,
            Some("root-msg".into()),
        );
        super::append_child(
            &mut transcript,
            "child".into(),
            "root".into(),
            Some("City".into()),
            HarnessUpdate::AgentMessage("child answer".into()),
            2,
        );
        super::append_identified(
            &mut transcript,
            super::StreamKind::Thought,
            "thinking".into(),
            3,
            Some("thought".into()),
        );
        super::append_identified(
            &mut transcript,
            super::StreamKind::Message,
            "ther\n\n**Done**".into(),
            4,
            Some("root-msg".into()),
        );
        assert_eq!(transcript.len(), 3);
        assert!(
            matches!(&transcript[0],ChatItem::AgentText {text,at_ms:1,..} if text=="# Weather\n\n**Done**")
        );
        assert!(
            matches!(&transcript[1],ChatItem::Child {items,..} if matches!(&items[0],ChatItem::AgentText {text,..} if text=="child answer"))
        );
        let restored: Vec<ChatItem> =
            serde_json::from_str(&serde_json::to_string(&transcript).unwrap()).unwrap();
        assert!(
            matches!(&restored[0],ChatItem::AgentText {message_id:Some(id),..} if id=="root-msg")
        );
    }

    #[test]
    fn startup_failures_are_not_retried_as_runtime_crashes() {
        for status in [
            HarnessStatus::Offline,
            HarnessStatus::Starting,
            HarnessStatus::Failed("missing adapter".into()),
            HarnessStatus::Exited("startup failed".into()),
        ] {
            assert!(!should_restart(&status, true, 0));
        }
        let ready = HarnessStatus::Ready {
            agent: "Mock".into(),
            version: "1".into(),
        };
        assert!(should_restart(&ready, true, 0));
        assert!(!should_restart(&ready, true, 1));
        assert!(!should_restart(&ready, false, 0));
    }

    #[test]
    fn duplicate_connection_errors_do_not_accumulate_between_messages() {
        let mut state = HarnessState::default();
        state.push_error("missing adapter".into());
        state.push_error("missing adapter".into());
        assert_eq!(state.transcript.len(), 1);
        state.transcript.push(ChatItem::User {
            text: "try again".into(),
            timing: None,
            at_ms: 1,
        });
        state.push_error("missing adapter".into());
        state.push_error("missing adapter".into());
        assert_eq!(state.transcript.len(), 3);
        assert!(matches!(&state.transcript[1], ChatItem::User { text, .. } if text == "try again"));
        state.push_error("different error".into());
        assert_eq!(state.transcript.len(), 4);
    }

    #[test]
    fn title_uses_the_first_line() {
        assert_eq!(
            session_title("Fix the login flow\nand add tests"),
            "Fix the login flow"
        );
    }

    #[test]
    fn title_collapses_whitespace() {
        assert_eq!(
            session_title("  review   my   changes  "),
            "review my changes"
        );
    }

    #[test]
    fn summary_collapses_and_limits_prompt_text() {
        assert_eq!(
            session_summary("  Review\n the   working tree  "),
            "Review the working tree"
        );
        let long = session_summary(&"a".repeat(300));
        assert_eq!(long.chars().count(), 181);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn title_falls_back_for_empty_prompts() {
        assert_eq!(session_title("   \n\t"), "New session");
    }

    #[test]
    fn title_truncates_long_prompts() {
        let title = session_title(&"a".repeat(80));
        assert_eq!(title.chars().count(), 53);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn custom_harness_command_is_used_verbatim() {
        assert_eq!(
            harness_command("Custom ACP", "  /tmp/my-agent --acp  "),
            "/tmp/my-agent --acp"
        );
        assert_eq!(harness_command("Custom ACP", "   "), "your-agent --acp");
        assert_eq!(harness_command("OpenCode", "ignored"), "opencode acp");
    }

    #[test]
    fn detects_executables_on_path() {
        assert!(command_available("sh"));
        assert!(command_available("\"/bin/sh\" -c true"));
        assert!(command_available("'/bin/sh' -c true"));
        assert!(!command_available("\"unterminated"));
        assert!(!command_available("\"\""));
        assert!(!command_available("dunlin-definitely-not-installed"));
    }

    #[test]
    fn transcripts_without_timestamps_still_deserialize() {
        let json = r#"[
            {"User":{"text":"hello"}},
            {"AgentText":{"text":"hi","streaming":false}},
            {"Tool":{"view":{"id":"t1","title":"Read","kind":"Read","status":"Done","detail":"","locations":[],"additions":0,"deletions":0},"duration_ms":12}}
        ]"#;
        let items: Vec<ChatItem> = serde_json::from_str(json).unwrap();
        assert_eq!(items.len(), 3);
        assert!(super::now_ms() > 0);
        assert!(matches!(&items[0], ChatItem::User { at_ms: 0, .. }));
        assert!(matches!(&items[1], ChatItem::AgentText { at_ms: 0, .. }));
        assert!(matches!(&items[2], ChatItem::Tool { at_ms: 0, .. }));
    }

    #[test]
    fn transcripts_export_as_markdown() {
        let items = vec![
            ChatItem::User {
                text: "Fix it".into(),
                timing: None,
                at_ms: 1,
            },
            ChatItem::AgentText {
                text: "Done".into(),
                streaming: false,
                message_id: None,
                at_ms: 2,
            },
            ChatItem::Plan {
                items: vec![dunlin_agent::PlanItem {
                    content: "Read files".into(),
                    status: dunlin_agent::PlanStatus::Completed,
                }],
                at_ms: 3,
            },
            ChatItem::Error {
                message: "boom".into(),
                at_ms: 4,
            },
        ];
        let markdown = transcript_markdown(&items);
        assert!(markdown.contains("## You\n\nFix it"));
        assert!(markdown.contains("## Agent\n\nDone"));
        assert!(markdown.contains("[x] Read files"));
        assert!(markdown.contains("**Error:** boom"));
    }

    #[test]
    fn queue_reorders_and_removes_entries() {
        let mut state = HarnessState::default();
        let first = state.enqueue_prompt("first".into());
        let second = state.enqueue_prompt("second".into());
        assert_eq!(state.queued_prompts().len(), 2);
        assert!(state.move_queued_prompt(second.id, -1));
        assert_eq!(state.queued_prompts()[0].id, second.id);
        assert!(state.remove_queued_prompt(first.id).is_some());
        assert_eq!(state.queued_prompts().len(), 1);
        assert_eq!(state.clear_queued_prompts(), 1);
        assert!(state.queued_prompts().is_empty());
    }

    #[test]
    fn streaming_chunks_keep_the_first_timestamp() {
        let mut transcript = Vec::new();
        super::append_stream(
            &mut transcript,
            super::StreamKind::Message,
            "hel".into(),
            10,
        );
        super::append_stream(&mut transcript, super::StreamKind::Message, "lo".into(), 20);
        assert_eq!(transcript.len(), 1);
        assert!(matches!(
            &transcript[0],
            ChatItem::AgentText {
                text,
                at_ms: 10,
                ..
            } if text == "hello"
        ));
    }
}
