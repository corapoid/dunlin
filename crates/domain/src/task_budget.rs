//! Persistable client-dispatch budgets. Reported cumulative USD is evidence, not
//! a provider billing guarantee; unseen harness-internal child turns are unknown.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cost {
    pub acp: String,
    pub micro_usd: u64,
    pub at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub session: String,
    pub project: String,
    pub root: String,
    pub attempts: u32,
    pub baseline: Option<Cost>,
    pub latest: Option<Cost>,
    pub opaque_children: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub id: String,
    pub label: String,
    pub started_ms: u64,
    pub deadline_ms: u64,
    pub max_attempts: u32,
    pub reported_usd_limit: Option<u64>,
    pub stopped: Option<String>,
    pub members: Vec<Member>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub tasks: Vec<Budget>,
}
pub fn micro_usd(amount: f64) -> Option<u64> {
    if !amount.is_finite() || amount < 0. || amount > 1_000_000_000. {
        None
    } else {
        Some((amount * 1_000_000.).round() as u64)
    }
}
impl Budget {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.label.trim().is_empty()
            || self.label.len() > 256
            || self.members.is_empty()
            || self.members.len() > 8
            || self.max_attempts == 0
            || self.max_attempts > 1000
            || self.deadline_ms <= self.started_ms
            || self.deadline_ms - self.started_ms > 86_400_000
            || self.reported_usd_limit == Some(0)
        {
            return Err("Budget requires 1–1,000 client requests, 1–86,400 seconds, a label and 1–8 scoped sessions".into());
        }
        let mut seen = std::collections::HashSet::new();
        if self.members.iter().any(|m| {
            m.session.is_empty()
                || m.project.is_empty()
                || m.root.is_empty()
                || !seen.insert(&m.session)
        }) {
            return Err("Budget session membership is invalid or duplicated".into());
        }
        Ok(())
    }
    pub fn attempts(&self) -> u32 {
        self.members
            .iter()
            .map(|m| m.attempts)
            .fold(0, u32::saturating_add)
    }
    pub fn spent(&self) -> Option<u64> {
        self.members.iter().try_fold(0u64, |sum, m| {
            if m.opaque_children {
                return None;
            }
            let base = m.baseline.as_ref()?;
            let last = m.latest.as_ref()?;
            if base.acp != last.acp {
                return None;
            }
            sum.checked_add(last.micro_usd.checked_sub(base.micro_usd)?)
        })
    }
    pub fn active_stop(&self, now: u64) -> Option<String> {
        if let Some(reason) = &self.stopped {
            return Some(reason.clone());
        }
        if now < self.started_ms {
            return Some("System clock moved before budget start; reapprove the task".into());
        }
        if now >= self.deadline_ms {
            return Some("Task wall-clock budget expired".into());
        }
        if let Some(limit) = self.reported_usd_limit {
            if self.members.iter().any(|m| m.opaque_children) {
                return Some("Harness child-work cost coverage is unknown".into());
            }
            if self.spent().is_some_and(|spent| spent >= limit) {
                return Some(
                    "Reported cumulative USD budget reached (late reports can exceed this ceiling)"
                        .into(),
                );
            }
            if self.members.iter().any(|m| m.attempts > 0) && self.spent().is_none() {
                return Some("Cumulative USD accounting is no longer verifiable".into());
            }
            if self.members.iter().any(|m| {
                m.attempts > 0
                    && m.latest
                        .as_ref()
                        .is_none_or(|c| now < c.at_ms || now - c.at_ms > 30_000)
            }) {
                return Some("Fresh USD usage is unavailable for active budgeted work".into());
            }
        }
        None
    }
    pub fn gate(
        &self,
        session: &str,
        project: &str,
        root: &str,
        acp: Option<&str>,
        now: u64,
    ) -> Result<(), String> {
        self.continuation_gate(session, project, root, acp, now)?;
        if self.attempts() >= self.max_attempts {
            return Err("Client request budget exhausted; queued text is retained".into());
        }
        Ok(())
    }
    /// Tool permissions belong to the already reserved request, not a new send.
    pub fn continuation_gate(
        &self,
        session: &str,
        project: &str,
        root: &str,
        acp: Option<&str>,
        now: u64,
    ) -> Result<(), String> {
        self.validate()?;
        if let Some(reason) = self.active_stop(now) {
            return Err(reason);
        }
        let member = self
            .members
            .iter()
            .find(|m| m.session == session)
            .ok_or("Session is not assigned to this budget")?;
        if member.project != project || member.root != root {
            return Err("Budget project/root changed; explicit reapproval is required".into());
        }
        if self.reported_usd_limit.is_some() {
            if self.spent().is_none() {
                return Err("USD cost is unknown, not zero. Wait for verified session USD usage or explicitly approve a budget without a USD ceiling.".into());
            }
            let last = member.latest.as_ref().ok_or("No USD observation")?;
            if Some(last.acp.as_str()) != acp || now < last.at_ms || now - last.at_ms > 30_000 {
                return Err("USD observation is stale or belongs to another ACP session".into());
            }
        }
        Ok(())
    }
    pub fn observe(&mut self, session: &str, cost: Option<Cost>) {
        let Some(m) = self.members.iter_mut().find(|m| m.session == session) else {
            return;
        };
        if m.attempts == 0
            && m.baseline
                .as_ref()
                .zip(cost.as_ref())
                .is_none_or(|(a, b)| a.acp != b.acp)
        {
            m.baseline = cost.clone();
        }
        if let (Some(old), Some(new)) = (&m.latest, &cost)
            && old.acp == new.acp
            && new.micro_usd < old.micro_usd
        {
            self.stopped =
                Some("Cumulative USD counter decreased; accounting is no longer verifiable".into());
        }
        m.latest = cost;
    }
}
impl Registry {
    pub fn validate(&self) -> Result<(), String> {
        if self.tasks.len() > 64 {
            return Err("Budget history exceeds 64 tasks".into());
        }
        let mut ids = std::collections::HashSet::new();
        for task in &self.tasks {
            if !ids.insert(&task.id) {
                return Err("Duplicate task budget identity".into());
            }
            task.validate()?;
        }
        Ok(())
    }
    /// Keep release tombstones until every older overlapping membership is gone.
    /// Otherwise deleting a release would silently resurrect an older budget.
    pub fn forget_released(&mut self, id: &str) -> Result<(), String> {
        let index = self
            .tasks
            .iter()
            .position(|t| t.id == id)
            .ok_or("Budget no longer exists")?;
        let task = &self.tasks[index];
        if task.stopped.as_deref() != Some("Released by user") {
            return Err("Explicitly release this task before forgetting its history".into());
        }
        if self.tasks[..index].iter().any(|older| {
            older
                .members
                .iter()
                .any(|m| task.members.iter().any(|n| m.session == n.session))
        }) {
            return Err("Release and forget older overlapping budgets first; the release marker must not revive an older assignment".into());
        }
        self.tasks.remove(index);
        Ok(())
    }
    pub fn task(&self, session: &str) -> Option<&Budget> {
        self.tasks
            .iter()
            .rev()
            .find(|t| t.members.iter().any(|m| m.session == session))
            .filter(|t| t.stopped.as_deref() != Some("Released by user"))
    }
    pub fn task_mut(&mut self, session: &str) -> Option<&mut Budget> {
        self.tasks
            .iter_mut()
            .rev()
            .find(|t| t.members.iter().any(|m| m.session == session))
            .filter(|t| t.stopped.as_deref() != Some("Released by user"))
    }
    pub fn reserve(
        &mut self,
        session: &str,
        project: &str,
        root: &str,
        acp: Option<&str>,
        now: u64,
    ) -> Result<(), String> {
        if let Some(task) = self.task_mut(session) {
            task.gate(session, project, root, acp, now)?;
            let member = task
                .members
                .iter_mut()
                .find(|m| m.session == session)
                .unwrap();
            member.attempts = member
                .attempts
                .checked_add(1)
                .ok_or("Request counter overflow")?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> Budget {
        Budget {
            id: "task".into(),
            label: "Test".into(),
            started_ms: 1000,
            deadline_ms: 61000,
            max_attempts: 2,
            reported_usd_limit: None,
            stopped: None,
            members: vec![
                Member {
                    session: "a".into(),
                    project: "p".into(),
                    root: "/repo".into(),
                    attempts: 0,
                    baseline: None,
                    latest: None,
                    opaque_children: false,
                },
                Member {
                    session: "b".into(),
                    project: "p2".into(),
                    root: "/tree".into(),
                    attempts: 0,
                    baseline: None,
                    latest: None,
                    opaque_children: false,
                },
            ],
        }
    }
    #[test]
    fn shared_requests_and_retries_reserve_before_send_and_survive_restart() {
        let mut r = Registry {
            tasks: vec![budget()],
        };
        r.reserve("a", "p", "/repo", None, 1001).unwrap();
        r.reserve("b", "p2", "/tree", None, 1002).unwrap();
        let mut restored: Registry =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(restored.reserve("a", "p", "/repo", None, 1003).is_err());
        assert_eq!(restored.tasks[0].attempts(), 2);
        assert!(
            restored.tasks[0].active_stop(1003).is_none(),
            "last allowed requests may finish"
        );
    }
    #[test]
    fn unknown_stale_reset_currency_and_child_costs_are_never_zero() {
        let mut b = budget();
        b.reported_usd_limit = Some(2_000_000);
        assert!(b.gate("a", "p", "/repo", Some("acp"), 1001).is_err());
        for session in ["a", "b"] {
            b.observe(
                session,
                Some(Cost {
                    acp: "acp".into(),
                    micro_usd: 500_000,
                    at_ms: 1000,
                }),
            );
        }
        assert_eq!(b.spent(), Some(0));
        assert!(b.gate("a", "p", "/repo", Some("other"), 1001).is_err());
        assert!(b.gate("a", "p", "/repo", Some("acp"), 31001).is_err());
        b.members[0].opaque_children = true;
        assert_eq!(b.spent(), None);
        assert!(b.active_stop(1001).is_some());
        assert_eq!(micro_usd(f64::NAN), None);
        assert_eq!(micro_usd(-1.), None);
    }
    #[test]
    fn time_root_and_cost_counter_reset_fail_closed() {
        let mut b = budget();
        assert!(b.gate("a", "p", "/other", None, 1001).is_err());
        assert!(b.active_stop(61000).is_some());
        b.observe(
            "a",
            Some(Cost {
                acp: "id".into(),
                micro_usd: 100,
                at_ms: 1001,
            }),
        );
        b.observe(
            "a",
            Some(Cost {
                acp: "id".into(),
                micro_usd: 99,
                at_ms: 1002,
            }),
        );
        assert!(b.stopped.is_some());
    }
    #[test]
    fn final_reserved_request_may_grant_tools_but_expiry_and_scope_changes_may_not() {
        let mut r = Registry {
            tasks: vec![budget()],
        };
        r.reserve("a", "p", "/repo", None, 1001).unwrap();
        r.reserve("b", "p2", "/tree", None, 1002).unwrap();
        let t = &r.tasks[0];
        assert!(t.gate("a", "p", "/repo", None, 1003).is_err());
        assert!(t.continuation_gate("a", "p", "/repo", None, 1003).is_ok());
        assert!(t.continuation_gate("a", "p", "/repo", None, 61000).is_err());
        assert!(
            t.continuation_gate("a", "p", "/elsewhere", None, 1003)
                .is_err()
        );
        r.tasks[0].stopped = Some("Stopped by user".into());
        assert!(
            r.tasks[0]
                .continuation_gate("a", "p", "/repo", None, 1003)
                .is_err()
        );
    }
    #[test]
    fn released_tombstone_cannot_be_forgotten_to_revive_an_older_assignment() {
        let old = budget();
        let mut new = old.clone();
        new.id = "new".into();
        new.stopped = Some("Released by user".into());
        let mut r = Registry {
            tasks: vec![old, new],
        };
        assert!(r.task("a").is_none());
        assert!(r.forget_released("new").is_err());
        assert!(r.task("a").is_none());
        assert!(r.forget_released("task").is_err());
        r.tasks[0].stopped = Some("Released by user".into());
        r.forget_released("task").unwrap();
        assert!(r.task("a").is_none());
        r.forget_released("new").unwrap();
        assert!(r.tasks.is_empty());
        let restored: Registry = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(restored.task("a").is_none());
    }
    #[test]
    fn fresh_but_changed_acp_accounting_does_not_authorize_tools() {
        let mut b = budget();
        b.reported_usd_limit = Some(10000);
        for session in ["a", "b"] {
            b.observe(
                session,
                Some(Cost {
                    acp: "initial".into(),
                    micro_usd: 10,
                    at_ms: 1000,
                }),
            );
        }
        b.members[0].attempts = 1;
        b.observe(
            "a",
            Some(Cost {
                acp: "changed".into(),
                micro_usd: 10,
                at_ms: 1001,
            }),
        );
        assert!(b.active_stop(1002).is_some());
        assert!(
            b.continuation_gate("a", "p", "/repo", Some("changed"), 1002)
                .is_err()
        );
    }
}
