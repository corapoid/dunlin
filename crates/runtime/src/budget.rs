//! Client-dispatch budget state. Persistence is an explicit adapter: a successful
//! durable write is required before the in-memory ledger authorizes a new send.
use dunlin_domain::task_budget::{Budget, Registry};

pub const KEY: &str = "task-budget-ledgers-v1";

#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub session: &'a str,
    pub project: &'a str,
    pub root: &'a str,
    pub acp: Option<&'a str>,
}

#[derive(Default)]
pub struct State {
    pub registry: Registry,
    pub error: Option<String>,
}

impl State {
    pub fn load(read: impl FnOnce(&str) -> Result<Option<Registry>, String>) -> Self {
        let result = read(KEY)
            .map(|registry| registry.unwrap_or_default())
            .and_then(|registry| {
                registry.validate()?;
                Ok(registry)
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
            Err(error) => Self {
                registry: Registry::default(),
                error: Some(error),
            },
        }
    }

    pub fn commit(
        &mut self,
        next: Registry,
        persist: impl FnOnce(&str, &Registry) -> Result<(), String>,
    ) -> Result<(), String> {
        next.validate()?;
        if let Err(error) = persist(KEY, &next) {
            self.error = Some(error.clone());
            return Err(error);
        }
        self.registry = next;
        self.error = None;
        Ok(())
    }

    pub fn gate(&self, scope: Scope<'_>, now: u64) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if let Some(task) = self.registry.task(scope.session) {
            task.gate(scope.session, scope.project, scope.root, scope.acp, now)?;
        }
        Ok(())
    }

    pub fn continuation_gate(&self, scope: Scope<'_>, now: u64) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if let Some(task) = self.registry.task(scope.session) {
            task.continuation_gate(scope.session, scope.project, scope.root, scope.acp, now)?;
        }
        Ok(())
    }

    pub fn reserve(
        &mut self,
        scope: Scope<'_>,
        now: u64,
        persist: impl FnOnce(&str, &Registry) -> Result<(), String>,
    ) -> Result<(), String> {
        self.gate(scope, now)?;
        if self.registry.task(scope.session).is_none() {
            return Ok(());
        }
        let mut next = self.registry.clone();
        next.reserve(scope.session, scope.project, scope.root, scope.acp, now)?;
        self.commit(next, persist)
    }

    pub fn approve(
        &mut self,
        budget: Budget,
        persist: impl FnOnce(&str, &Registry) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.error.is_some() {
            return Err("Repair/reload budget storage before approving new work".into());
        }
        budget.validate()?;
        if self.registry.tasks.len() >= 64 {
            return Err(
                "Budget history limit reached; release and remove obsolete tasks first".into(),
            );
        }
        let mut next = self.registry.clone();
        next.tasks.push(budget);
        self.commit(next, persist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::task_budget::{Cost, Member};

    fn scope() -> Scope<'static> {
        Scope {
            session: "s",
            project: "p",
            root: "/project",
            acp: Some("acp"),
        }
    }
    fn budget() -> Budget {
        Budget {
            id: "b".into(),
            label: "Task".into(),
            started_ms: 1,
            deadline_ms: 10000,
            max_attempts: 1,
            reported_usd_limit: None,
            stopped: None,
            members: vec![Member {
                session: "s".into(),
                project: "p".into(),
                root: "/project".into(),
                attempts: 0,
                baseline: None,
                latest: None,
                opaque_children: false,
            }],
        }
    }
    fn loaded() -> State {
        State::load(|key| {
            assert_eq!(key, KEY);
            Ok(Some(Registry {
                tasks: vec![budget()],
            }))
        })
    }

    #[test]
    fn reservation_is_durable_before_memory_changes_and_blocks_the_next_send() {
        let mut state = loaded();
        state
            .reserve(scope(), 2, |key, next| {
                assert_eq!(key, KEY);
                assert_eq!(next.tasks[0].attempts(), 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(state.registry.tasks[0].attempts(), 1);
        assert!(state.gate(scope(), 3).is_err());
        assert!(state.continuation_gate(scope(), 3).is_ok());
        assert!(
            state
                .reserve(scope(), 3, |_, _| panic!(
                    "must not persist a rejected request"
                ))
                .is_err()
        );
    }

    #[test]
    fn failed_persistence_keeps_the_old_ledger_and_denies_further_work() {
        let mut state = loaded();
        assert!(
            state
                .reserve(scope(), 2, |_, _| Err("disk full".into()))
                .is_err()
        );
        assert_eq!(state.registry.tasks[0].attempts(), 0);
        assert_eq!(state.error.as_deref(), Some("disk full"));
        assert!(state.gate(scope(), 3).is_err());
        assert!(state.continuation_gate(scope(), 3).is_err());
        assert!(
            state
                .approve(budget(), |_, _| panic!(
                    "must not approve after a storage failure"
                ))
                .is_err()
        );
    }

    #[test]
    fn corrupt_or_unavailable_storage_is_not_an_empty_unrestricted_ledger() {
        let missing = State::load(|_| Err("storage unavailable".into()));
        assert!(missing.gate(scope(), 2).is_err());
        let mut invalid = budget();
        invalid.max_attempts = 0;
        let corrupt = State::load(|_| {
            Ok(Some(Registry {
                tasks: vec![invalid],
            }))
        });
        assert!(corrupt.gate(scope(), 2).is_err());
        assert!(corrupt.registry.tasks.is_empty());
    }

    #[test]
    fn restart_invalidates_old_cost_observations_without_resetting_attempts() {
        let mut task = budget();
        task.reported_usd_limit = Some(1000);
        task.members[0].attempts = 1;
        let cost = Cost {
            acp: "acp".into(),
            micro_usd: 10,
            at_ms: 2,
        };
        task.members[0].baseline = Some(cost.clone());
        task.members[0].latest = Some(cost);
        let state = State::load(|_| Ok(Some(Registry { tasks: vec![task] })));
        assert_eq!(state.registry.tasks[0].attempts(), 1);
        assert!(state.registry.tasks[0].members[0].latest.is_none());
        assert!(state.continuation_gate(scope(), 3).is_err());
    }

    #[test]
    fn scope_and_deadline_are_checked_before_persistence() {
        let mut state = loaded();
        assert!(
            state
                .reserve(
                    Scope {
                        root: "/different",
                        ..scope()
                    },
                    2,
                    |_, _| panic!("wrong root")
                )
                .is_err()
        );
        assert!(
            state
                .reserve(scope(), 10000, |_, _| panic!("expired"))
                .is_err()
        );
        assert_eq!(state.registry.tasks[0].attempts(), 0);
    }

    #[test]
    fn unbudgeted_requests_do_not_write_a_ledger() {
        let mut state = State::load(|_| Ok(None));
        state
            .reserve(scope(), 2, |_, _| panic!("no budget to reserve"))
            .unwrap();
    }
}
