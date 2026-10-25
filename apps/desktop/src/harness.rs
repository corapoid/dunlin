//! Live ACP harness state and the bridge between the worker thread and GPUI.
//!
//! The UI never touches protocol I/O directly. It keeps a transcript, forwards
//! prompts to [`AcpHarness`], and folds streaming events back into that
//! transcript. Permission prompts round-trip through the same path.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kestrel_agent::{
    AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate, PermissionChoice, PermissionKind,
    PlanItem, PlanStatus, ToolCallDelta, ToolCallView,
};
use kestrel_domain::{SessionStatus, ToolStatus};
use kestrel_ui::*;
use serde::{Deserialize, Serialize};

use crate::app::{KestrelApp, PendingRemoteSession};
use crate::logging;

/// One rendered entry in the conversation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum ChatItem {
    User {
        text: String,
        #[serde(default)]
        at_ms: u64,
    },
    AgentText {
        text: String,
        streaming: bool,
        #[serde(default)]
        at_ms: u64,
    },
    Thought {
        text: String,
        streaming: bool,
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

pub(crate) struct HarnessState {
    pub harness: Option<AcpHarness>,
    pub status: HarnessStatus,
    pub transcript: Vec<ChatItem>,
    transcripts: HashMap<String, Vec<ChatItem>>,
    pub usage: Option<HarnessUsage>,
    pub commands: Vec<String>,
    pub running: bool,
    pub session_id: Option<String>,
    pending_prompts: VecDeque<QueuedPrompt>,
    next_prompt_id: u64,
    prompt_sent: bool,
    generation: u64,
    pub(crate) started_tools: HashMap<String, Instant>,
    restarts: u8,
}

impl Default for HarnessState {
    fn default() -> Self {
        Self {
            harness: None,
            status: HarnessStatus::Offline,
            transcript: Vec::new(),
            transcripts: HashMap::new(),
            usage: None,
            commands: Vec::new(),
            running: false,
            session_id: None,
            pending_prompts: VecDeque::new(),
            next_prompt_id: 0,
            prompt_sent: false,
            generation: 0,
            started_tools: HashMap::new(),
            restarts: 0,
        }
    }
}

impl HarnessState {
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

impl KestrelApp {
    /// Start a fresh ACP process or remote connection for the selected project.
    pub(crate) fn connect_harness(&mut self, cx: &mut Context<Self>) {
        if self.harness.status.is_busy() {
            return;
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

        let command = std::env::var("KESTREL_HARNESS_COMMAND")
            .ok()
            .filter(|command| !command.trim().is_empty())
            .unwrap_or_else(|| {
                harness_command(&self.default_harness, &self.custom_harness_command)
            });
        self.start_harness(HarnessConfig::new(command, cwd), None, cx);
    }

    fn start_harness(
        &mut self,
        config: HarnessConfig,
        remote_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.harness.generation += 1;
        let generation = self.harness.generation;
        if let Some(previous) = self.harness.harness.take() {
            previous.shutdown();
        }

        let description = config.description();
        logging::info(&format!(
            "starting harness: {description} · cwd {}",
            config.cwd.display()
        ));
        let harness = AcpHarness::spawn(config);
        let events = harness.events().clone();
        self.harness.harness = Some(harness);
        self.harness.status = HarnessStatus::Starting;
        self.harness.session_id = None;
        self.harness.running = false;
        self.harness.commands.clear();
        self.notice = Some(match remote_name {
            Some(name) => format!("Connecting to {name}…"),
            None => format!("Starting {description}…"),
        });

        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let keep_going = this.update(cx, |this, cx| {
                    if this.harness.generation != generation {
                        return false;
                    }
                    this.handle_harness_event(event, cx);
                    true
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
        self.interrupt_live_turn(cx);
        logging::info("disconnecting harness");
        self.harness.generation += 1;
        if let Some(harness) = self.harness.harness.take() {
            harness.shutdown();
        }
        self.harness.status = HarnessStatus::Offline;
        self.harness.session_id = None;
        self.harness.running = false;
        self.notice = Some("Harness disconnected".into());
        cx.notify();
    }

    pub(crate) fn restart_harness(&mut self, cx: &mut Context<Self>) {
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

    /// Send the composer contents to the active harness, connecting lazily.
    pub(crate) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
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
        let title = session_title(&text);
        let summary = session_summary(&text);
        let was_running = self.harness.running;
        self.harness.transcript.push(ChatItem::User {
            text: text.clone(),
            at_ms: now_ms(),
        });
        self.harness.running = true;
        if !was_running {
            self.harness.prompt_sent = false;
        }
        self.harness.enqueue_prompt(text);
        self.update_active_session(|session| {
            session.status = SessionStatus::Running;
            session.summary = summary;
            if session.title == "New session" {
                session.title = title;
            }
        });
        self.save_current_transcript();

        if self.harness.status.is_ready() && self.harness.session_id.is_some() {
            self.dispatch_pending_prompt();
        } else if !self.harness.status.is_busy() && !self.remote_sessions_loading {
            self.connect_harness(cx);
        }
    }

    pub(crate) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
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
        let current_id = self.session().id.clone();
        let current = std::mem::take(&mut self.harness.transcript);
        if let Some(store) = &self.store {
            let _ = store.save_transcript(&current_id, &current);
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
        let loaded = self.harness.transcripts.remove(session_id).or_else(|| {
            self.store.as_ref().and_then(|store| {
                store
                    .load_transcript::<Vec<ChatItem>>(session_id)
                    .ok()
                    .flatten()
            })
        });
        self.harness.transcript = loaded.unwrap_or_default();
        self.harness.started_tools.clear();
        self.harness.pending_prompts.clear();
        self.harness.prompt_sent = false;
        self.harness.running = false;
        self.harness.usage = None;
    }

    /// Switch the visible conversation and start a fresh ACP session for it.
    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.sessions.len() || index == self.selected_session {
            return;
        }
        if self.remote_sessions_loading {
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
            let path = PathBuf::from(&self.project().path);
            if let Some(harness) = self.harness.harness.as_ref() {
                harness.new_session(path);
            }
        } else if self.auto_connect && !self.harness.status.is_busy() {
            self.connect_harness(cx);
        }

        self.notice = Some(format!("Opened “{title}”"));
        self.persist_settings(cx);
        cx.notify();
    }

    fn is_current_session(&self, session_id: &str) -> bool {
        self.harness.session_id.as_deref() == Some(session_id)
    }

    /// Notify only when the window is in the background and the user kept
    /// desktop notifications enabled. The Dock badge is updated separately so
    /// it stays visible while the app is active too.
    fn notify_if_background(&self, title: &str, body: &str) {
        if self.notifications_enabled && !self.app_active {
            crate::platform::notify(title, body);
        }
    }

    /// Rescan the explorer and flag a clean editor for reload after a turn.
    fn refresh_workspace_after_turn(&mut self, cx: &mut Context<Self>) {
        self.refresh_file_tree(cx);
        self.editor_stale = self.inspector.is_some() && !self.editor_dirty;
    }

    fn dispatch_pending_prompt(&mut self) {
        if self.harness.prompt_sent || self.harness.session_id.is_none() {
            return;
        }
        let Some(prompt) = self.harness.pending_prompts.pop_front() else {
            return;
        };
        let text = prompt.text;
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
            .map(|harness| harness.prompt(text.clone()))
            .unwrap_or(false);
        if sent {
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
        if std::env::var_os("KESTREL_TRACE_HARNESS").is_some() {
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
                if self.project().is_remote()
                    && let Some(action) = self.pending_remote_session.take()
                {
                    self.start_remote_session(action, cx);
                } else if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt();
                }
            }
            HarnessEvent::SessionCreated { session_id } => {
                if self.project().is_remote() {
                    self.create_remote_session_summary(&session_id, cx);
                } else {
                    let stored = self.sessions.get_mut(self.selected_session).map(|session| {
                        session.acp_session_id = Some(session_id.clone());
                        session.clone()
                    });
                    if let Some(session) = stored {
                        self.persist_session(&session);
                    }
                }
                self.harness.session_id = Some(session_id);
                if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt();
                }
            }
            HarnessEvent::SessionLoaded { session_id } => {
                self.harness.session_id = Some(session_id);
                self.update_active_session(|session| {
                    if matches!(
                        session.status,
                        SessionStatus::Waiting | SessionStatus::Running
                    ) {
                        session.status = SessionStatus::Done;
                    }
                });
                if !self.harness.pending_prompts.is_empty() {
                    self.dispatch_pending_prompt();
                }
            }
            HarnessEvent::Update {
                session_id, update, ..
            } => {
                if self.is_current_session(&session_id) {
                    self.apply_update(update);
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
                    self.handle_permission(request_id, title.clone(), options);
                    self.notify_if_background(
                        "Permission requested",
                        &format!("{title} · {session_title}"),
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
            } => {
                if self.is_current_session(&session_id) {
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
                    );
                    self.save_current_transcript();
                    self.refresh_workspace_after_turn(cx);
                    self.refresh_git_status(cx);
                    self.dispatch_pending_prompt();
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
                self.harness.transcript.push(ChatItem::Error {
                    message: message.clone(),
                    at_ms: now_ms(),
                });
                self.notify_if_background("Kestrel needs attention", &message);
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

                if self.restart_harness && self.harness.restarts < 1 {
                    self.harness.restarts += 1;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1200))
                            .await;
                        this.update(cx, |this, cx| this.connect_harness(cx)).ok();
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    fn apply_update(&mut self, update: HarnessUpdate) {
        match update {
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
        let auto = match self.permission_mode {
            crate::app::PermissionMode::AutoApprove => options
                .iter()
                .find(|option| option.kind == PermissionKind::AllowAlways)
                .or_else(|| {
                    options
                        .iter()
                        .find(|option| option.kind == PermissionKind::AllowOnce)
                })
                .cloned(),
            crate::app::PermissionMode::ReadOnly => options
                .iter()
                .find(|option| {
                    matches!(
                        option.kind,
                        PermissionKind::RejectAlways | PermissionKind::RejectOnce
                    )
                })
                .cloned(),
            crate::app::PermissionMode::Ask => None,
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
        for item in self.harness.transcript.iter_mut() {
            match item {
                ChatItem::AgentText { streaming, .. } | ChatItem::Thought { streaming, .. } => {
                    *streaming = false;
                }
                _ => {}
            }
        }
    }

    pub(crate) fn update_active_session(
        &mut self,
        update: impl FnOnce(&mut kestrel_domain::SessionSummary),
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

    pub(crate) fn persist_session(&self, session: &kestrel_domain::SessionSummary) {
        if let Some(store) = &self.store {
            let _ = store.upsert_session(session);
        }
    }

    pub(crate) fn save_current_transcript(&mut self) {
        let session_id = self.session().id.clone();
        let transcript = self.harness.transcript.clone();
        if let Some(store) = &self.store {
            let _ = store.save_transcript(&session_id, &transcript);
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
                let _ = store.save_transcript(session_id, transcript);
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
    match (kind, transcript.last_mut()) {
        (
            StreamKind::Message,
            Some(ChatItem::AgentText {
                text: body,
                streaming,
                ..
            }),
        ) if *streaming => {
            body.push_str(&text);
            return;
        }
        (
            StreamKind::Thought,
            Some(ChatItem::Thought {
                text: body,
                streaming,
                ..
            }),
        ) if *streaming => {
            body.push_str(&text);
            return;
        }
        _ => {}
    }

    match kind {
        StreamKind::Message => transcript.push(ChatItem::AgentText {
            text,
            streaming: true,
            at_ms,
        }),
        StreamKind::Thought => transcript.push(ChatItem::Thought {
            text,
            streaming: true,
            at_ms,
        }),
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

/// Check whether the executable named by an ACP command is on `PATH`.
pub(crate) fn command_available(command: &str) -> bool {
    let Some(executable) = command.split_whitespace().next() else {
        return false;
    };
    if executable.contains('/') {
        return Path::new(executable).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|directory| {
                let candidate = directory.join(executable);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
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
        ChatItem, HarnessState, command_available, harness_command, session_summary, session_title,
        transcript_markdown,
    };

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
        assert!(!command_available("kestrel-definitely-not-installed"));
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
                at_ms: 1,
            },
            ChatItem::AgentText {
                text: "Done".into(),
                streaming: false,
                at_ms: 2,
            },
            ChatItem::Plan {
                items: vec![kestrel_agent::PlanItem {
                    content: "Read files".into(),
                    status: kestrel_agent::PlanStatus::Completed,
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
