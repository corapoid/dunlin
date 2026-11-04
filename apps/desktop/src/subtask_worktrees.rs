//! Durable task-board registry for one worktree per agent subtask.
use crate::app::DunlinApp;
use dunlin_workspace::subtask::Registry;

const KEY: &str = "subtask-worktrees-v1";

#[derive(Default)]
pub(crate) struct State {
    pub registry: Registry,
    pub error: Option<String>,
}

impl State {
    pub(crate) fn load(store: Option<&dunlin_storage::Store>) -> Self {
        let result = store
            .ok_or("Storage unavailable; existing subtask worktrees cannot be verified".to_string())
            .and_then(|store| {
                store
                    .load_setting::<Registry>(KEY)
                    .map_err(|_| "Could not read the subtask worktree registry".into())
            })
            .map(|registry| registry.unwrap_or_default())
            .and_then(|registry| {
                registry.validate()?;
                Ok(registry)
            });
        match result {
            Ok(registry) => Self {
                registry,
                error: None,
            },
            Err(error) => Self {
                registry: Registry::default(),
                error: Some(error),
            },
        }
    }
}

impl DunlinApp {
    pub(crate) fn persist_subtask_worktrees(&mut self, next: Registry) -> Result<(), String> {
        next.validate()?;
        let result = self
            .store
            .as_ref()
            .ok_or("Subtask worktree storage unavailable")
            .and_then(|store| {
                store
                    .save_setting(KEY, &next)
                    .map_err(|_| "Could not persist the subtask worktree registry")
            });
        if let Err(error) = result {
            self.subtask_worktrees.error = Some(error.into());
            return Err(error.into());
        }
        self.subtask_worktrees.registry = next;
        self.subtask_worktrees.error = None;
        Ok(())
    }
}
