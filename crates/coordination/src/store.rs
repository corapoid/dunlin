use crate::*;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// One bounded, versioned ledger per coordinator authority. The caller holds a
/// process lock around read/modify/commit, including MCP snapshot registration.
pub struct Store {
    connection: Connection,
    ledger: Ledger,
    revision: u64,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        Self::initialize(connection)
    }
    pub fn memory() -> Result<Self, String> {
        Self::initialize(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }
    fn initialize(connection: Connection) -> Result<Self, String> {
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS coordination_ledger (id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL, data TEXT NOT NULL);").map_err(|e| e.to_string())?;
        let row: Option<(u32, String)> = connection
            .query_row(
                "SELECT version,data FROM coordination_ledger WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let ledger = match row {
            Some((1, value)) if value.len() <= 64 * 1024 * 1024 => {
                serde_json::from_str::<Ledger>(&value).map_err(|e| e.to_string())?
            }
            None => Ledger::default(),
            _ => {
                return Err(
                    "Unsupported or oversized coordination ledger; refusing to overwrite it".into(),
                );
            }
        };
        ledger.validate()?;
        Ok(Self {
            connection,
            ledger,
            revision: 0,
        })
    }
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn mutate<T>(
        &mut self,
        f: impl FnOnce(&mut Ledger) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut next = self.ledger.clone();
        let result = f(&mut next)?;
        next.validate()?;
        let data = serde_json::to_string(&next).map_err(|e| e.to_string())?;
        if data.len() > 64 * 1024 * 1024 {
            return Err("Coordination ledger size limit reached".into());
        }
        self.connection.execute("INSERT INTO coordination_ledger(id,version,data) VALUES(1,1,?1) ON CONFLICT(id) DO UPDATE SET version=1,data=excluded.data", [data]).map_err(|e| e.to_string())?;
        self.ledger = next;
        self.revision = self.revision.wrapping_add(1);
        Ok(result)
    }
    pub fn recover(&mut self) -> Result<(), String> {
        self.mutate(|l| {
            l.recover();
            Ok(())
        })
    }
    pub fn start_run(&mut self, mut run: Run) -> Result<String, String> {
        self.mutate(|l| {
            if l.runs
                .iter()
                .any(|r| r.scope == run.scope && r.session == run.session && r.status.active())
            {
                return Err("This session already has an active run".into());
            }
            if l.runs.iter().any(|r| r.id == run.id) {
                return Err("Duplicate run".into());
            }
            run.status = RunStatus::Running;
            let id = run.id.clone();
            for task in l.tasks.iter_mut().filter(|t| {
                t.scope == run.scope && t.owner == run.session && t.status == TaskStatus::Open
            }) {
                task.runs.push(id.clone());
            }
            l.audit(&run.scope, &run.session, "run_started", &id);
            l.runs.push(run);
            Ok(id)
        })
    }
    pub fn finish_run(
        &mut self,
        id: &str,
        status: RunStatus,
        reason: &str,
        queued: Option<usize>,
        report: Option<Report>,
    ) -> Result<(), String> {
        self.mutate(|l| {
            let run = l
                .runs
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Run is unavailable")?;
            // Late duplicate/stale worker events cannot rewrite terminal facts.
            if !run.status.active() {
                return Ok(());
            }
            run.status = status;
            run.stop_reason = Some(reason.chars().take(2048).collect());
            run.queued_prompts = queued;
            run.updated_ms = now_ms();
            if run.report.is_none() {
                run.report = report;
            }
            let (scope, session) = (run.scope.clone(), run.session.clone());
            l.audit(&scope, &session, "run_finished", id);
            Ok(())
        })
    }
    pub fn approve(
        &mut self,
        id: &str,
        execution: &Execution,
        deadline_ms: u64,
    ) -> Result<(), String> {
        self.mutate(|l| {
            let current = l
                .continuations
                .iter()
                .find(|c| c.id == id)
                .ok_or("Continuation unavailable")?
                .clone();
            if !matches!(
                current.status,
                ContinuationStatus::PendingApproval | ContinuationStatus::NeedsAttention
            ) {
                return Err("Continuation cannot be approved in this state".into());
            }
            if &current.execution != execution {
                return Err("Execution configuration changed; create a new continuation".into());
            }
            if current.dispatched_run.is_some() {
                return Err(
                    "An uncertain/dispatched send must be reconciled, never replayed".into(),
                );
            }
            if l.continuations.iter().any(|c| {
                c.id != current.id
                    && c.scope == current.scope
                    && c.session == current.session
                    && matches!(
                        c.status,
                        ContinuationStatus::PendingApproval
                            | ContinuationStatus::Waiting
                            | ContinuationStatus::Dispatching
                    )
            }) {
                return Err("Resolve the other continuation for this conversation first".into());
            }
            l.check_dependencies(&current.scope, &current.session, &current.dependencies)?;
            let c = l.continuations.iter_mut().find(|c| c.id == id).unwrap();
            c.created_ms = now_ms();
            c.deadline_ms = deadline_ms;
            c.approved_ms = Some(now_ms());
            c.status = ContinuationStatus::Waiting;
            c.reason = None;
            l.audit(&current.scope, "human", "continuation_approved", id);
            Ok(())
        })
    }
    pub fn set_continuation(
        &mut self,
        id: &str,
        status: ContinuationStatus,
        reason: Option<String>,
    ) -> Result<(), String> {
        self.mutate(|l| {
            let c = l
                .continuations
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Continuation unavailable")?;
            if c.status == ContinuationStatus::Dispatched {
                return Err("Already dispatched; cancel its run instead".into());
            }
            c.status = status;
            c.reason = reason;
            let scope = c.scope.clone();
            l.audit(&scope, "human/runtime", "continuation_state_changed", id);
            Ok(())
        })
    }
    /// The reservation and run identity are durable before touching the wire.
    pub fn reserve_continuation(&mut self, id: &str, run: Run) -> Result<String, String> {
        self.mutate(|l| {
            let c = l
                .continuations
                .iter()
                .find(|c| c.id == id)
                .ok_or("Continuation unavailable")?
                .clone();
            if c.status != ContinuationStatus::Waiting || !l.readiness(&c, now_ms())? {
                return Err("Continuation is not ready".into());
            }
            if c.execution != run.execution
                || c.scope != run.scope
                || c.session != run.session
                || run.continuation.as_deref() != Some(id)
            {
                return Err("Continuation execution scope changed".into());
            }
            if l.runs.iter().any(|r| {
                r.id == run.id
                    || (r.scope == run.scope && r.session == run.session && r.status.active())
            }) {
                return Err("Run identity/session is already active".into());
            }
            let run_id = run.id.clone();
            for task in l.tasks.iter_mut().filter(|t| {
                t.scope == run.scope && t.owner == run.session && t.status == TaskStatus::Open
            }) {
                task.runs.push(run_id.clone());
            }
            let c = l.continuations.iter_mut().find(|c| c.id == id).unwrap();
            c.status = ContinuationStatus::Dispatching;
            c.dispatched_run = Some(run_id.clone());
            l.audit(&run.scope, &run.session, "continuation_reserved", id);
            l.runs.push(run);
            Ok(run_id)
        })
    }
    pub fn confirm_dispatch(&mut self, id: &str) -> Result<(), String> {
        self.mutate(|l| {
            let c = l
                .continuations
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Continuation unavailable")?;
            if c.status != ContinuationStatus::Dispatching {
                return Err("No wire reservation".into());
            }
            c.status = ContinuationStatus::Dispatched;
            Ok(())
        })
    }
    /// Archive only terminal, unreferenced history; never erase dedupe keys for
    /// live work or leave dangling dependencies. This is an explicit UI action.
    pub fn archive(&mut self) -> Result<(), String> {
        self.archive_scope(None)
    }
    pub fn archive_scope(&mut self, scope: Option<&Scope>) -> Result<(), String> {
        self.mutate(|l| {
            l.continuations.retain(|c| {
                scope.is_some_and(|scope| &c.scope != scope)
                    || !matches!(
                        c.status,
                        ContinuationStatus::Dispatched | ContinuationStatus::Cancelled
                    )
                    || c.dispatched_run
                        .as_ref()
                        .is_some_and(|id| l.runs.iter().any(|r| &r.id == id && r.status.active()))
            });
            l.messages
                .retain(|m| scope.is_some_and(|scope| &m.scope != scope) || m.read_ms.is_none());
            l.tasks.retain(|t| {
                scope.is_some_and(|scope| &t.scope != scope)
                    || t.status == TaskStatus::Open
                    || l.continuations
                        .iter()
                        .any(|c| c.dependencies.contains(&Dependency::Task(t.id.clone())))
            });
            let mut latest = std::collections::HashSet::new();
            let latest_ids: std::collections::HashSet<_> = l
                .runs
                .iter()
                .rev()
                .filter(|r| {
                    latest.insert((
                        r.scope.clone().authority,
                        r.scope.project.clone(),
                        r.session.clone(),
                    ))
                })
                .map(|r| r.id.clone())
                .collect();
            l.runs.retain(|r| {
                scope.is_some_and(|scope| &r.scope != scope)
                    || r.status.active()
                    || latest_ids.contains(&r.id)
                    || l.tasks.iter().any(|t| t.runs.contains(&r.id))
                    || l.continuations.iter().any(|c| {
                        c.requester_run == r.id
                            || c.dependencies.contains(&Dependency::Run(r.id.clone()))
                    })
            });
            Ok(())
        })
    }
}
