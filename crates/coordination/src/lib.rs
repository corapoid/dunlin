//! Harness-neutral coordination. Runtime facts, agent claims and human grants
//! are deliberately separate. No tool in this crate executes a prompt or Git.
pub mod mcp;
pub mod service;
mod store;

use serde::{Deserialize, Serialize};
pub use store::Store;

pub const MAX_RUNS: usize = 2048;
pub const MAX_ITEMS: usize = 512;
pub const MAX_TEXT: usize = 16 * 1024;
pub const MAX_DEPS: usize = 64;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Authority is local, or a relay user namespace. Never accept it from a tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub authority: String,
    pub project: String,
}
impl Scope {
    fn validate(&self) -> Result<(), String> {
        text(&self.authority, 256)?;
        text(&self.project, 256)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub root: String,
    pub harness: String,
    pub model: String,
    pub mode: String,
    pub permission: String,
    /// Opaque configuration digest; never persist process environment values.
    pub configuration: String,
}
impl Execution {
    fn validate(&self) -> Result<(), String> {
        text(&self.root, 4096)?;
        text(&self.harness, 512)?;
        text(&self.model, 512)?;
        text(&self.mode, 512)?;
        text(&self.permission, 64)?;
        text(&self.configuration, 128)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    WaitingPermission,
    TurnFinished,
    Failed,
    Cancelled,
    Unknown,
}
impl RunStatus {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingPermission)
    }
    pub fn from_stop(reason: &str) -> Self {
        match reason {
            "end_turn" => Self::TurnFinished,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        text(&self.summary, MAX_TEXT)?;
        if self.evidence.len() > 32 || self.blockers.len() > 32 {
            return Err("Report list limit exceeded".into());
        }
        for v in self.evidence.iter().chain(&self.blockers) {
            text(v, 2048)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub session: String,
    pub scope: Scope,
    pub execution: Execution,
    pub title: String,
    pub status: RunStatus,
    /// Relay cannot observe unsent prompts on other devices: unknown is null.
    pub queued_prompts: Option<usize>,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub stop_reason: Option<String>,
    /// A claim by the agent, not an independently verified success verdict.
    pub report: Option<Report>,
    pub continuation: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    ReportedSucceeded,
    ReportedFailed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub scope: Scope,
    pub owner: String,
    pub title: String,
    pub runs: Vec<String>,
    pub status: TaskStatus,
    pub report: Option<Report>,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Dependency {
    Run(String),
    Task(String),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationStatus {
    PendingApproval,
    Waiting,
    Dispatching,
    Dispatched,
    Cancelled,
    NeedsAttention,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Continuation {
    pub id: String,
    pub scope: Scope,
    pub session: String,
    pub requester_run: String,
    pub execution: Execution,
    pub prompt: String,
    pub dependencies: Vec<Dependency>,
    pub status: ContinuationStatus,
    pub created_ms: u64,
    pub deadline_ms: u64,
    pub approved_ms: Option<u64>,
    pub reason: Option<String>,
    pub dispatched_run: Option<String>,
    #[serde(default)]
    pub executor_device: Option<String>,
    #[serde(default)]
    pub executor_connection: Option<String>,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub scope: Scope,
    pub from: String,
    pub to: String,
    pub body: String,
    pub created_ms: u64,
    pub read_ms: Option<u64>,
    pub key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Audit {
    pub at_ms: u64,
    pub scope: Scope,
    pub actor: String,
    pub action: String,
    pub subject: String,
}

/// Added only by a human/native surface, never through an agent tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorktreeLink {
    pub project: String,
    pub root: String,
    pub parent: String,
    pub parent_root: String,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub runs: Vec<Run>,
    pub tasks: Vec<Task>,
    pub continuations: Vec<Continuation>,
    pub messages: Vec<Message>,
    pub audit: Vec<Audit>,
    #[serde(default)]
    pub worktree_links: Vec<WorktreeLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub scope: Scope,
    pub device: String,
    pub ledger: Ledger,
}
impl Ledger {
    pub fn validate(&self) -> Result<(), String> {
        if self.runs.len() > MAX_RUNS
            || self.tasks.len() > MAX_ITEMS
            || self.continuations.len() > MAX_ITEMS
            || self.messages.len() > MAX_ITEMS
            || self.audit.len() > 4096
            || self.worktree_links.len() > 64
        {
            return Err(
                "Coordination history limit reached; archive finished history first".into(),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for link in &self.worktree_links {
            if link.project == link.parent
                || !ids.insert(&link.project)
                || self.worktree_links.iter().any(|l| l.project == link.parent)
            {
                return Err("Worktree links must be unique, explicit, one-level grants".into());
            }
            text(&link.root, 4096)?;
            text(&link.parent_root, 4096)?;
        }
        ids.clear();
        for run in &self.runs {
            run.scope.validate()?;
            run.execution.validate()?;
            if !ids.insert(&run.id) {
                return Err("Duplicate run identity".into());
            }
            text(&run.id, 128)?;
            text(&run.session, 256)?;
            text(&run.title, 512)?;
            if let Some(report) = &run.report {
                report.validate()?;
            }
        }
        ids.clear();
        for c in &self.continuations {
            c.scope.validate()?;
            c.execution.validate()?;
            text(&c.id, 128)?;
            text(&c.session, 256)?;
            text(&c.key, 128)?;
            if !ids.insert(&c.id) || c.dependencies.len() > MAX_DEPS {
                return Err("Invalid continuation identity/dependencies".into());
            }
            text(&c.prompt, MAX_TEXT)?;
            if c.deadline_ms <= c.created_ms || c.deadline_ms - c.created_ms > 86_400_000 {
                return Err("Invalid continuation deadline".into());
            }
            self.check_dependencies(&c.scope, &c.session, &c.dependencies)?;
            if !self
                .runs
                .iter()
                .any(|r| r.id == c.requester_run && r.scope == c.scope && r.session == c.session)
            {
                return Err("Continuation requester is unavailable".into());
            }
        }
        ids.clear();
        for t in &self.tasks {
            if !ids.insert(&t.id) {
                return Err("Duplicate task identity".into());
            }
            t.scope.validate()?;
            text(&t.id, 128)?;
            text(&t.owner, 256)?;
            text(&t.key, 128)?;
            text(&t.title, 512)?;
            for id in &t.runs {
                if !self
                    .runs
                    .iter()
                    .any(|r| &r.id == id && r.scope == t.scope && r.session == t.owner)
                {
                    return Err("Invalid task membership".into());
                }
            }
            if let Some(r) = &t.report {
                r.validate()?;
            }
        }
        ids.clear();
        for m in &self.messages {
            if !ids.insert(&m.id) {
                return Err("Duplicate message identity".into());
            }
            m.scope.validate()?;
            text(&m.id, 128)?;
            text(&m.from, 256)?;
            text(&m.to, 256)?;
            text(&m.key, 128)?;
            text(&m.body, MAX_TEXT)?;
        }
        Ok(())
    }
    pub fn audit(&mut self, scope: &Scope, actor: &str, action: &str, subject: &str) {
        if self.audit.len() == 4096 {
            self.audit.remove(0);
        }
        self.audit.push(Audit {
            at_ms: now_ms(),
            scope: scope.clone(),
            actor: actor.into(),
            action: action.into(),
            subject: subject.into(),
        });
    }
    pub fn current_run(&self, scope: &Scope, session: &str) -> Option<&Run> {
        self.runs
            .iter()
            .rev()
            .find(|r| &r.scope == scope && r.session == session)
    }
    pub fn check_dependencies(
        &self,
        scope: &Scope,
        session: &str,
        deps: &[Dependency],
    ) -> Result<(), String> {
        if deps.len() > MAX_DEPS {
            return Err("At most 64 dependencies are allowed".into());
        }
        let mut seen = std::collections::HashSet::new();
        for dep in deps {
            let (id, owner) = match dep {
                Dependency::Run(id) => (
                    id,
                    self.runs
                        .iter()
                        .find(|r| &r.id == id && &r.scope == scope)
                        .map(|r| &r.session),
                ),
                Dependency::Task(id) => (
                    id,
                    self.tasks
                        .iter()
                        .find(|t| &t.id == id && &t.scope == scope)
                        .map(|t| &t.owner),
                ),
            };
            let owner = owner.ok_or("Dependency is unavailable in this project")?;
            if owner == session || !seen.insert(id) {
                return Err("Self or duplicate dependency refused".into());
            }
        }
        // Edges point only to already-started runs, or tasks owned by another
        // session. Reject session wait cycles too (including open task cycles).
        fn visit(
            l: &Ledger,
            scope: &Scope,
            node: &str,
            target: &str,
            seen: &mut std::collections::HashSet<String>,
        ) -> bool {
            if node == target {
                return true;
            }
            if !seen.insert(node.into()) {
                return false;
            }
            l.continuations
                .iter()
                .filter(|c| {
                    &c.scope == scope
                        && c.session == node
                        && matches!(
                            c.status,
                            ContinuationStatus::PendingApproval | ContinuationStatus::Waiting
                        )
                })
                .flat_map(|c| &c.dependencies)
                .any(|d| {
                    let owner = match d {
                        Dependency::Run(id) => l
                            .runs
                            .iter()
                            .find(|r| &r.id == id && r.status.active())
                            .map(|r| r.session.as_str()),
                        Dependency::Task(id) => l
                            .tasks
                            .iter()
                            .find(|t| &t.id == id && t.status == TaskStatus::Open)
                            .map(|t| t.owner.as_str()),
                    };
                    owner.is_some_and(|o| visit(l, scope, o, target, seen))
                })
        }
        for dep in deps {
            let owner = match dep {
                Dependency::Run(id) => self
                    .runs
                    .iter()
                    .find(|r| &r.id == id && r.status.active())
                    .map(|r| r.session.as_str()),
                Dependency::Task(id) => self
                    .tasks
                    .iter()
                    .find(|t| &t.id == id && t.status == TaskStatus::Open)
                    .map(|t| t.owner.as_str()),
            };
            if owner
                .is_some_and(|owner| visit(self, scope, owner, session, &mut Default::default()))
            {
                return Err("Dependency cycle refused".into());
            }
        }
        Ok(())
    }
    pub fn readiness(&self, c: &Continuation, now: u64) -> Result<bool, String> {
        if now < c.created_ms || now >= c.deadline_ms {
            return Err(
                "Continuation expired or system clock moved backwards; reapproval required".into(),
            );
        }
        let requester = self
            .runs
            .iter()
            .find(|r| r.id == c.requester_run)
            .ok_or("Requester run is unavailable")?;
        let mut waiting = requester.status.active();
        if !waiting && requester.status != RunStatus::TurnFinished {
            return Err("The requesting turn did not finish normally".into());
        }
        for d in &c.dependencies {
            match d {
                Dependency::Run(id) => {
                    let r = self
                        .runs
                        .iter()
                        .find(|r| &r.id == id)
                        .ok_or("Dependency disappeared")?;
                    if r.status.active() {
                        waiting = true;
                        continue;
                    }
                    if r.status != RunStatus::TurnFinished {
                        return Err(format!("Dependency {id} ended as {:?}", r.status));
                    }
                }
                Dependency::Task(id) => {
                    let t = self
                        .tasks
                        .iter()
                        .find(|t| &t.id == id)
                        .ok_or("Task disappeared")?;
                    match t.status {
                        TaskStatus::Open => waiting = true,
                        TaskStatus::ReportedFailed => {
                            return Err(format!("Task {id} reported failure"));
                        }
                        TaskStatus::ReportedSucceeded => {
                            if t.runs.iter().any(|id| {
                                self.runs.iter().any(|r| &r.id == id && r.status.active())
                            }) {
                                waiting = true;
                                continue;
                            }
                            if t.runs.last().is_none_or(|id| {
                                !self
                                    .runs
                                    .iter()
                                    .any(|r| &r.id == id && r.status == RunStatus::TurnFinished)
                            }) {
                                return Err(format!(
                                    "Task {id} did not finish its reporting run normally"
                                ));
                            }
                        }
                    }
                }
            }
        }
        // Do not jump ahead of a new user turn, even in the same conversation.
        if self
            .current_run(&c.scope, &c.session)
            .is_none_or(|r| r.id != c.requester_run)
        {
            return Err("The target conversation advanced; reapprove this continuation".into());
        }
        Ok(!waiting)
    }
    pub fn recover(&mut self) {
        for r in &mut self.runs {
            if r.status.active() {
                r.status = RunStatus::Unknown;
                r.stop_reason = Some("Runtime observation lost on restart".into());
                r.updated_ms = now_ms();
            }
        }
        for c in &mut self.continuations {
            if matches!(
                c.status,
                ContinuationStatus::Waiting | ContinuationStatus::Dispatching
            ) {
                c.status = ContinuationStatus::NeedsAttention;
                c.reason = Some("Application restarted; verify runtime and explicitly reapprove. An uncertain wire send is never replayed.".into());
            }
        }
    }
}

pub(crate) fn text(s: &str, max: usize) -> Result<(), String> {
    if s.trim().is_empty() || s.len() > max || s.contains('\0') {
        Err(format!("Text must contain 1–{max} bytes and no NUL"))
    } else {
        Ok(())
    }
}
