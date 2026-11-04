//! Budget ledger lives outside tabs/runtimes, survives navigation/restart, and is
//! durably reserved BEFORE each client wire send, including retries/queued sends.
use crate::app::DunlinApp;
use dunlin_domain::task_budget::{Budget, Cost, Registry};
use dunlin_ui::*;
const KEY: &str = "task-budget-ledgers-v1";
#[derive(Default)]
pub(crate) struct State {
    pub registry: Registry,
    pub error: Option<String>,
}
impl State {
    pub fn load(store: Option<&dunlin_storage::Store>) -> Self {
        let result = store
            .ok_or("Storage unavailable; existing task budgets cannot be verified".to_string())
            .and_then(|s| {
                s.load_setting::<Registry>(KEY)
                    .map_err(|_| "Could not read task budget ledger".into())
            })
            .map(|r| r.unwrap_or_default())
            .and_then(|r| {
                r.validate()?;
                Ok(r)
            });
        match result {
            Ok(mut registry) => {
                for task in &mut registry.tasks {
                    for member in &mut task.members {
                        member.latest = None;
                    }
                }
                Self {
                    registry,
                    error: None,
                }
            }
            Err(e) => Self {
                registry: Registry::default(),
                error: Some(e),
            },
        }
    }
}
impl DunlinApp {
    pub(crate) fn persist_task_budgets(&mut self, next: Registry) -> Result<(), String> {
        next.validate()?;
        let result = self
            .store
            .as_ref()
            .ok_or("Budget storage unavailable")
            .and_then(|s| {
                s.save_setting(KEY, &next).map_err(
                    |_| "Budget reservation could not be persisted; no new prompt is authorized",
                )
            });
        if let Err(e) = result {
            self.task_budgets.error = Some(e.into());
            return Err(e.into());
        }
        self.task_budgets.registry = next;
        self.task_budgets.error = None;
        Ok(())
    }
    pub(crate) fn budget_gate(&self) -> Result<(), String> {
        if let Some(e) = &self.task_budgets.error {
            return Err(e.clone());
        }
        if let Some(task) = self.task_budgets.registry.task(&self.session().id) {
            task.gate(
                &self.session().id,
                &self.project().id,
                &self.project().path,
                self.harness.session_id.as_deref(),
                crate::harness::now_ms(),
            )?;
        }
        Ok(())
    }
    pub(crate) fn budget_continuation_gate(&self) -> Result<(), String> {
        if let Some(e) = &self.task_budgets.error {
            return Err(e.clone());
        }
        if let Some(task) = self.task_budgets.registry.task(&self.session().id) {
            task.continuation_gate(
                &self.session().id,
                &self.project().id,
                &self.project().path,
                self.harness.session_id.as_deref(),
                crate::harness::now_ms(),
            )?;
        }
        Ok(())
    }
    pub(crate) fn reserve_task_request(&mut self, cx: &mut Context<Self>) -> bool {
        let result = (|| {
            self.budget_gate()?;
            if self
                .task_budgets
                .registry
                .task(&self.session().id)
                .is_none()
            {
                return Ok(());
            }
            let mut next = self.task_budgets.registry.clone();
            next.reserve(
                &self.session().id,
                &self.project().id,
                &self.project().path,
                self.harness.session_id.as_deref(),
                crate::harness::now_ms(),
            )?;
            self.persist_task_budgets(next)
        })();
        if let Err(e) = result {
            self.file_notice(format!("Task budget paused sending: {e}"), cx);
            false
        } else {
            true
        }
    }
    pub(crate) fn approve_task_budget(&mut self, budget: Budget) -> Result<(), String> {
        if self.task_budgets.error.is_some() {
            return Err("Repair/reload budget storage before approving new work".into());
        }
        budget.validate()?;
        let mut next = self.task_budgets.registry.clone();
        if next.tasks.len() >= 64 {
            return Err(
                "Budget history limit reached; release and remove obsolete tasks first".into(),
            );
        }
        next.tasks.push(budget);
        self.persist_task_budgets(next)?;
        if let Some(session) = &self.harness.vault_session {
            session.set_allowed(true);
        }
        Ok(())
    }
    pub(crate) fn observe_task_cost(&mut self, cost: Option<f64>, opaque_child: bool) {
        let now = crate::harness::now_ms();
        let session = self.session().id.clone();
        if !opaque_child {
            self.harness.reported_cost = cost
                .and_then(dunlin_domain::task_budget::micro_usd)
                .map(|amount| Cost {
                    acp: self.harness.session_id.clone().unwrap_or_default(),
                    micro_usd: amount,
                    at_ms: now,
                })
                .filter(|c| !c.acp.is_empty());
        }
        if self.task_budgets.registry.task(&session).is_none() {
            return;
        }
        let mut next = self.task_budgets.registry.clone();
        let task = next.task_mut(&session).unwrap();
        if opaque_child {
            if let Some(m) = task.members.iter_mut().find(|m| m.session == session) {
                m.opaque_children = true;
            }
        } else {
            task.observe(&session, self.harness.reported_cost.clone());
        }
        let _ = self.persist_task_budgets(next);
    }
    pub(crate) fn poll_task_budgets(&mut self, cx: &mut Context<Self>) {
        let now = crate::harness::now_ms();
        let mut next = self.task_budgets.registry.clone();
        let mut changed = false;
        for task in &mut next.tasks {
            if task.stopped.is_none()
                && let Some(reason) = task.active_stop(now)
            {
                task.stopped = Some(reason);
                changed = true;
            }
        }
        if changed {
            let _ = self.persist_task_budgets(next);
            cx.notify();
        }
        let reason = |session: &str| -> Option<String> {
            self.task_budgets.error.clone().or_else(|| {
                self.task_budgets
                    .registry
                    .task(session)
                    .and_then(|task| task.active_stop(now))
            })
        };
        let active = reason(&self.session().id);
        let parked = self
            .parked_runtimes
            .keys()
            .filter_map(|id| reason(id).map(|r| (id.clone(), r)))
            .collect::<Vec<_>>();
        if let Some(reason) = active
            && self.harness.running
        {
            self.harness.pause_for_budget(&reason);
            self.update_active_session(|s| s.status = dunlin_domain::SessionStatus::Interrupted);
            self.save_current_transcript();
            self.file_notice(
                format!("Task budget: {reason}. Cancellation requested; queued text retained."),
                cx,
            );
        }
        for (id, reason) in parked {
            if let Some(runtime) = self.parked_runtimes.get_mut(&id)
                && runtime.state.running
            {
                runtime.state.pause_for_budget(&reason);
                if let Some(s) = self.all_sessions.iter_mut().find(|s| s.id == id) {
                    s.status = dunlin_domain::SessionStatus::Interrupted;
                    let s = s.clone();
                    self.persist_session(&s);
                }
                cx.notify();
            }
        }
    }
}
