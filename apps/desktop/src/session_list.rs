//! Visibility only. Drafts stay persisted and selectable internally; no sessions
//! or transcripts are deleted to make the sidebar look empty.
use crate::{app::DunlinApp, harness::ChatItem};
use dunlin_domain::SessionSummary;
use dunlin_ui::Context;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct State {
    values: HashMap<String, bool>,
    checked: HashSet<String>,
    busy: bool,
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
        self.sessions
            .iter()
            .filter(|s| self.session_visible(s))
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
            .sessions
            .iter()
            .filter(|s| {
                !historical_evidence(s)
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
    use super::{State, has_user_message, historical_evidence};
    use crate::harness::ChatItem;
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
