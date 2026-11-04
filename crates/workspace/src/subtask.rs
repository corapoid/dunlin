//! Durable registry for one worktree per agent subtask.
//!
//! Creating the tree is still an explicit two-phase Git operation. This module
//! only records what was approved so the task board can reopen/review it and so
//! stale entries are never silently reused after the worktree disappears.
use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtaskWorktree {
    pub id: String,
    pub project_id: String,
    pub root: String,
    pub task: String,
    /// Optional `T###` link from a spec task. Empty for manually created trees.
    #[serde(default)]
    pub task_id: String,
    pub name: String,
    pub branch: String,
    pub path: String,
    pub base: String,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    #[serde(default)]
    pub entries: Vec<SubtaskWorktree>,
}

impl Registry {
    pub fn validate(&self) -> Result<(), String> {
        if self.entries.len() > MAX_ENTRIES {
            return Err("Subtask worktree registry limit reached".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for entry in &self.entries {
            if entry.id.is_empty()
                || entry.id.len() > 80
                || entry.task.trim().is_empty()
                || entry.task.len() > 256
                || entry.task.chars().any(char::is_control)
                || entry.name.is_empty()
                || entry.name.len() > 80
                || entry.root.is_empty()
                || entry.path.is_empty()
                || entry.branch.is_empty()
                || entry.branch.len() > 200
            {
                return Err("Invalid subtask worktree record".into());
            }
            if !ids.insert(&entry.id) || !paths.insert(&entry.path) {
                return Err("Duplicate subtask worktree record".into());
            }
        }
        Ok(())
    }

    pub fn add(&mut self, entry: SubtaskWorktree) -> Result<(), String> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err("Subtask worktree registry limit reached".into());
        }
        if self
            .entries
            .iter()
            .any(|existing| existing.path == entry.path || existing.id == entry.id)
        {
            return Err("This subtask worktree is already recorded".into());
        }
        self.entries.push(entry);
        self.validate()
    }

    pub fn remove(&mut self, id: &str) -> Option<SubtaskWorktree> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        Some(self.entries.remove(index))
    }

    pub fn for_project(&self, project_id: &str) -> impl Iterator<Item = &SubtaskWorktree> {
        self.entries
            .iter()
            .filter(move |entry| entry.project_id == project_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, path: &str) -> SubtaskWorktree {
        SubtaskWorktree {
            id: id.into(),
            project_id: "project".into(),
            root: "/repo".into(),
            task: "Review the API".into(),
            task_id: "T001".into(),
            name: "review-api".into(),
            branch: "dunlin/subtask/review-api".into(),
            path: path.into(),
            base: "abc".into(),
            created_ms: 1,
        }
    }

    #[test]
    fn registry_rejects_duplicate_paths_and_survives_roundtrip() {
        let mut registry = Registry::default();
        registry
            .add(entry("one", "/repo/.git/dunlin-worktrees/subtask-one"))
            .unwrap();
        assert!(
            registry
                .add(entry("two", "/repo/.git/dunlin-worktrees/subtask-one"))
                .is_err()
        );
        assert!(
            registry
                .add(entry("one", "/repo/.git/dunlin-worktrees/subtask-two"))
                .is_err()
        );
        let json = serde_json::to_string(&registry).unwrap();
        let restored: Registry = serde_json::from_str(&json).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.for_project("project").count(), 1);
        assert!(restored.for_project("other").next().is_none());
        assert_eq!(restored.entries[0].branch, "dunlin/subtask/review-api");
    }
}
