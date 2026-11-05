//! Session scopes, visibility and cross-project navigation. Drafts stay persisted
//! and selectable internally; filtering never deletes sessions or transcripts.
use crate::{app::DunlinApp, harness::ChatItem};
use dunlin_domain::{Project, SessionSummary};
use dunlin_ui::Context;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct State {
    values: HashMap<String, bool>,
    checked: HashSet<String>,
    busy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    General,
    Project,
}

fn scope(project: &Project, group_context: bool, current_project: &str) -> Option<Scope> {
    if project.is_chat_space() || group_context {
        Some(Scope::General)
    } else if project.id == current_project {
        Some(Scope::Project)
    } else {
        None
    }
}

pub(crate) fn has_user_message(items: &[ChatItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item, ChatItem::User { text, .. } if !text.trim().is_empty()))
}
pub(crate) fn historical_evidence(session: &SessionSummary) -> bool {
    session.title != "New session"
        || session.tokens > 0
        || session.duration_secs > 0
        || session.cost_usd > 0.0
}
impl State {
    pub(crate) fn record(&mut self, id: &str, items: &[ChatItem]) {
        let started = has_user_message(items);
        // A temporarily unloaded transcript is not evidence that an established
        // conversation became a draft. Session IDs never get reused.
        let previous = self.values.get(id).copied().unwrap_or(false);
        self.values.insert(id.into(), started || previous);
    }
}
impl DunlinApp {
    pub(crate) fn session_scope(&self, session: &SessionSummary) -> Option<Scope> {
        let project = self.projects.iter().find(|p| p.id == session.project_id)?;
        if project.is_remote() && !session.origin.is_remote() {
            return None;
        }
        scope(
            project,
            self.group_contexts.contains_key(&session.id),
            &self.project().id,
        )
    }

    pub(crate) fn open_session_id(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(project) = self
            .all_sessions
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| self.projects.iter().position(|p| p.id == s.project_id))
        else {
            return false;
        };
        if self.selected_project != project {
            self.select_project(project, cx);
        }
        if self.selected_project != project {
            return false;
        }
        if self.project().is_remote() && self.remote_sessions_loading {
            self.restore_session_id = Some(id.into());
            return true;
        }
        let Some(index) = self.sessions.iter().position(|s| s.id == id) else {
            return false;
        };
        self.select_session(index, cx);
        self.session().id == id
    }

    pub(crate) fn session_visible(&self, session: &SessionSummary) -> bool {
        if self.session().id == session.id && has_user_message(&self.harness.transcript) {
            return true;
        }
        // Existing titles/recorded usage are evidence of an established session,
        // even when a legacy/remote transcript is incomplete or unavailable.
        if historical_evidence(session) {
            return true;
        }
        self.session_list
            .values
            .get(&session.id)
            .copied()
            .unwrap_or_else(|| historical_evidence(session))
    }
    pub(crate) fn visible_session_count(&self) -> usize {
        self.all_sessions
            .iter()
            .filter(|s| self.session_scope(s).is_some() && self.session_visible(s))
            .count()
    }
    pub(crate) fn refresh_session_list(&mut self, cx: &mut Context<Self>) {
        if self.session_list.busy {
            return;
        }
        if has_user_message(&self.harness.transcript) || !historical_evidence(self.session()) {
            self.session_list
                .record(&self.session().id.clone(), &self.harness.transcript);
        }
        let ids: Vec<_> = self
            .all_sessions
            .iter()
            .filter(|s| {
                self.session_scope(s).is_some()
                    && !historical_evidence(s)
                    && !self.session_list.values.contains_key(&s.id)
                    && !self.session_list.checked.contains(&s.id)
            })
            .map(|s| s.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        self.session_list.checked.extend(ids.iter().cloned());
        self.session_list.busy = true;
        let path = self.store.as_ref().and_then(|s| s.database_path());
        cx.spawn(async move |this, cx| {
            let rows = cx
                .background_executor()
                .spawn(async move {
                    let store = path.and_then(|path| dunlin_storage::Store::read_only(path).ok());
                    ids.into_iter()
                        .map(|id| {
                            let started = store
                                .as_ref()
                                .and_then(|s| {
                                    s.load_transcript::<Vec<ChatItem>>(&id).ok().flatten()
                                })
                                .map(|items| has_user_message(&items));
                            (id, started)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.session_list.busy = false;
                for (id, started) in rows {
                    if let Some(started) = started
                        && this.all_sessions.iter().any(|s| s.id == id)
                        && !this.session_list.values.get(&id).copied().unwrap_or(false)
                    {
                        this.session_list.values.insert(id, started);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{Scope, State, has_user_message, historical_evidence, scope};
    use crate::harness::ChatItem;

    #[test]
    fn general_sessions_survive_project_switches_without_duplicating_project_history() {
        let mut project = dunlin_domain::Project {
            id: "repo".into(),
            name: "Repository".into(),
            path: "/repo".into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: Some("work".into()),
            settings: Default::default(),
        };
        // Membership alone does not turn an ordinary repository session into a group conversation.
        assert_eq!(scope(&project, false, "repo"), Some(Scope::Project));
        assert_eq!(scope(&project, false, "other"), None);
        for selected in ["repo", "other"] {
            assert_eq!(scope(&project, true, selected), Some(Scope::General));
        }
        project.kind = dunlin_domain::ProjectKind::ChatSpace;
        for selected in ["repo", "other"] {
            assert_eq!(scope(&project, false, selected), Some(Scope::General));
            assert_eq!(scope(&project, true, selected), Some(Scope::General));
        }
    }

    #[test]
    fn blank_and_failed_connection_drafts_are_hidden_but_a_first_user_message_is_enough() {
        let session = crate::app::blank_session("project", "OpenCode", None);
        assert!(!historical_evidence(&session));
        assert!(!has_user_message(&[]));
        assert!(!has_user_message(&[ChatItem::Error {
            message: "adapter unavailable".into(),
            at_ms: 0
        }]));
        assert!(!has_user_message(&[ChatItem::User {
            text: "  ".into(),
            timing: None,
            at_ms: 0
        }]));
        let mut state = State::default();
        state.record(&session.id, &[]);
        assert!(!state.values[&session.id]);
        state.record(
            &session.id,
            &[ChatItem::User {
                text: "Hello".into(),
                timing: None,
                at_ms: 0,
            }],
        );
        assert!(state.values[&session.id]);
        state.record(&session.id, &[]);
        assert!(state.values[&session.id]);
    }
    #[test]
    fn legacy_history_stays_visible_when_transcripts_are_temporarily_unavailable() {
        let mut session = crate::app::blank_session("project", "OpenCode", None);
        session.title = "Actual conversation".into();
        assert!(historical_evidence(&session));
        session.title = "New session".into();
        session.tokens = 12;
        assert!(historical_evidence(&session));
        let before = serde_json::to_string(&session).unwrap();
        let mut state = State::default();
        state.record(
            &session.id,
            &[ChatItem::User {
                text: "Old request".into(),
                timing: None,
                at_ms: 0,
            }],
        );
        assert_eq!(before, serde_json::to_string(&session).unwrap());
    }
}
