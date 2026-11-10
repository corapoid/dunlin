//! Budget ledger lives outside tabs/runtimes, survives navigation/restart, and is
//! durably reserved BEFORE each client wire send, including retries/queued sends.
use crate::app::DunlinApp;
use dunlin_domain::task_budget::{Budget, Cost, Registry};
use dunlin_runtime::budget::Scope;
pub(crate) use dunlin_runtime::budget::State;
use dunlin_ui::*;

pub(crate) fn load(store: Option<&dunlin_storage::Store>) -> State {
    State::load(|key| {
        store
            .ok_or("Storage unavailable; existing task budgets cannot be verified".to_string())
            .and_then(|s| {
                s.load_setting::<Registry>(key)
                    .map_err(|_| "Could not read task budget ledger".into())
            })
    })
}

fn persist(
    store: Option<&dunlin_storage::Store>,
    key: &str,
    registry: &Registry,
) -> Result<(), String> {
    store
        .ok_or("Budget storage unavailable".to_string())?
        .save_setting(key, registry)
        .map_err(|_| {
            "Budget reservation could not be persisted; no new prompt is authorized".into()
        })
}

impl DunlinApp {
    pub(crate) fn persist_task_budgets(&mut self, next: Registry) -> Result<(), String> {
        self.task_budgets
            .commit(next, |key, next| persist(self.store.as_ref(), key, next))
    }
    pub(crate) fn budget_gate(&self) -> Result<(), String> {
        self.task_budgets
            .gate(self.budget_scope(), crate::harness::now_ms())
    }
    pub(crate) fn budget_continuation_gate(&self) -> Result<(), String> {
        self.task_budgets
            .continuation_gate(self.budget_scope(), crate::harness::now_ms())
    }
    fn budget_scope(&self) -> Scope<'_> {
        Scope {
            session: &self.session().id,
            project: &self.project().id,
            root: &self.project().path,
            acp: self.harness.session_id.as_deref(),
        }
    }
    pub(crate) fn reserve_task_request(&mut self, cx: &mut Context<Self>) -> bool {
        let session = self.session().id.clone();
        let project = self.project().clone();
        let scope = Scope {
            session: &session,
            project: &project.id,
            root: &project.path,
            acp: self.harness.session_id.as_deref(),
        };
        let result = self
            .task_budgets
            .reserve(scope, crate::harness::now_ms(), |key, next| {
                persist(self.store.as_ref(), key, next)
            });
        if let Err(e) = result {
            self.file_notice(format!("Task budget paused sending: {e}"), cx);
            false
        } else {
            true
        }
    }
    pub(crate) fn approve_task_budget(&mut self, budget: Budget) -> Result<(), String> {
        self.task_budgets
            .approve(budget, |key, next| persist(self.store.as_ref(), key, next))?;
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
