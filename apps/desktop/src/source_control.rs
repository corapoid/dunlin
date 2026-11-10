//! Stable project-owned scroll state and cheap indexes for the virtual change list.
use dunlin_ui::UniformListScrollHandle;
use dunlin_workspace::git::GitChange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Heading { staged: bool, count: usize },
    File(usize),
}

#[derive(Default)]
pub(crate) struct State {
    scope: Option<(String, String)>,
    pub scroll: UniformListScrollHandle,
    pub rows: Vec<Row>,
    #[cfg(debug_assertions)]
    pub rendered: std::ops::Range<usize>,
}

impl State {
    pub fn prepare(&mut self, project: &str, root: &str, changes: &[GitChange]) {
        if self
            .scope
            .as_ref()
            .is_none_or(|(id, path)| id != project || path != root)
        {
            self.scroll = UniformListScrollHandle::new();
            self.scope = Some((project.into(), root.into()));
        }
        self.rows.clear();
        for staged in [true, false] {
            let count = changes.iter().filter(|c| c.staged == staged).count();
            if count == 0 {
                continue;
            }
            self.rows.push(Row::Heading { staged, count });
            self.rows.extend(
                changes
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.staged == staged)
                    .map(|(index, _)| Row::File(index)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_rows_keep_staged_and_worktree_entries_for_the_same_path() {
        let changes = [false, true, false].map(|staged| GitChange {
            path: "same.rs".into(),
            status: 'M',
            additions: 1,
            deletions: 0,
            staged,
        });
        let mut state = State::default();
        state.prepare("project", "/root", &changes);
        assert_eq!(
            state.rows,
            [
                Row::Heading {
                    staged: true,
                    count: 1
                },
                Row::File(1),
                Row::Heading {
                    staged: false,
                    count: 2
                },
                Row::File(0),
                Row::File(2)
            ]
        );
        state.prepare("other", "/other", &[]);
        assert!(state.rows.is_empty());
    }
}
