//! In-memory search snapshot. Opening search never scans disk or contacts a server.
use crate::app::{Command, CommandItem, DunlinApp, command_matches};
use dunlin_domain::SidebarSection;
use dunlin_ui::{assets::IconName, *};

#[derive(Clone)]
enum Target {
    Session(String),
    File { project: String, path: String },
}

struct Entry {
    target: Target,
    label: String,
    keywords: String,
    detail: String,
    kind: &'static str,
}

#[derive(Default)]
pub(crate) struct State {
    entries: Vec<Entry>,
}

impl State {
    pub(crate) fn detail(&self, index: usize) -> Option<String> {
        self.entries.get(index).map(|entry| entry.detail.clone())
    }
    pub(crate) fn items(&self, terms: &[&str]) -> Vec<CommandItem> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !terms.is_empty() || entry.kind == "session")
            .map(|(index, entry)| CommandItem {
                command: Command::SearchResult(index),
                label: entry.label.clone(),
                hint: Some(entry.kind.into()),
                keywords: entry.keywords.clone(),
                icon: if entry.kind == "session" {
                    IconName::MessageSquare
                } else {
                    IconName::FileText
                },
            })
            .filter(|item| command_matches(item, terms))
            .take(if terms.is_empty() { 8 } else { 40 })
            .collect()
    }
}

pub(crate) fn rank(item: &CommandItem, _suggestions: bool) -> usize {
    match item.command {
        Command::Section(SidebarSection::Dashboard) => 0,
        Command::SwitchProject(_) => 1,
        Command::SearchResult(_) => 2,
        Command::Settings(_) => 3,
        Command::Section(_) | Command::Artifacts | Command::Spec => 4,
        _ => 5,
    }
}

impl DunlinApp {
    pub(crate) fn refresh_global_search(&mut self) {
        let mut sessions: Vec<_> = self
            .all_sessions
            .iter()
            .filter(|session| self.session_visible(session))
            .collect();
        sessions.sort_by_cached_key(|session| {
            std::cmp::Reverse(chrono::DateTime::parse_from_rfc3339(&session.updated_at).ok())
        });
        let mut entries: Vec<_> = sessions
            .into_iter()
            .map(|session| Entry {
                target: Target::Session(session.id.clone()),
                label: session.title.clone(),
                detail: format!(
                    "{} · {}",
                    self.projects
                        .iter()
                        .find(|project| project.id == session.project_id)
                        .map(|project| project.name.as_str())
                        .unwrap_or("Session"),
                    session.harness
                ),
                keywords: format!(
                    "{} {} {}",
                    session.harness,
                    session.summary,
                    self.projects
                        .iter()
                        .find(|project| project.id == session.project_id)
                        .map(|project| project.name.as_str())
                        .unwrap_or("")
                ),
                kind: "session",
            })
            .collect();
        let project = self.project();
        if self.file_tree.root == project.path {
            let mut pending: Vec<_> = self.file_tree.entries.iter().rev().collect();
            while let Some(entry) = pending.pop() {
                if entry.is_dir {
                    pending.extend(entry.children.iter().rev());
                } else {
                    entries.push(Entry {
                        target: Target::File {
                            project: project.id.clone(),
                            path: entry.path.clone(),
                        },
                        label: entry.name.clone(),
                        detail: entry.path.clone(),
                        keywords: format!("{} {}", project.name, entry.path),
                        kind: "file",
                    });
                }
            }
        }
        self.global_search.entries = entries;
    }

    pub(crate) fn open_global_search_result(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self
            .global_search
            .entries
            .get(index)
            .map(|entry| entry.target.clone())
        else {
            return;
        };
        match target {
            Target::Session(id) => {
                if self.open_session_id(&id, cx) {
                    self.set_view(dunlin_domain::MainView::Chat, cx);
                }
                self.focus_handle.focus(window, cx);
            }
            Target::File { project, path } => {
                // A background project switch cannot retarget a stale result.
                if self.project().id == project {
                    self.open_file(&path, window, cx);
                } else {
                    self.file_notice("Project changed. Search again to open this file.", cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SettingsTab;

    #[test]
    fn dashboard_is_always_the_first_suggestion() {
        let item = |command| CommandItem {
            command,
            label: "Example".into(),
            keywords: String::new(),
            hint: None,
            icon: IconName::Search,
        };
        let mut items = vec![
            item(Command::SearchResult(0)),
            item(Command::Settings(SettingsTab::General)),
            item(Command::Section(SidebarSection::Dashboard)),
            item(Command::SwitchProject(0)),
        ];
        for suggestions in [true, false] {
            items.sort_by_key(|item| rank(item, suggestions));
            assert_eq!(
                items[0].command,
                Command::Section(SidebarSection::Dashboard)
            );
        }
    }

    #[test]
    fn suggestions_include_recent_sessions_but_files_require_a_query() {
        let state = State {
            entries: vec![
                Entry {
                    target: Target::Session("session-id".into()),
                    label: "Fix sidebar".into(),
                    keywords: "Dunlin".into(),
                    detail: "Dunlin · OpenCode".into(),
                    kind: "session",
                },
                Entry {
                    target: Target::File {
                        project: "project".into(),
                        path: "/project/sidebar.rs".into(),
                    },
                    label: "sidebar.rs".into(),
                    keywords: "Dunlin /project/sidebar.rs".into(),
                    detail: "/project/sidebar.rs".into(),
                    kind: "file",
                },
            ],
        };
        assert_eq!(state.items(&[]).len(), 1);
        let results = state.items(&["sidebar"]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].command, Command::SearchResult(1));
        assert!(state.items(&["missing"]).is_empty());
    }
}
