//! Root/session views and bounded, complete multi-channel AHP replay.
use super::*;
use ahp::reducers::{ReduceOutcome, apply_action_to_session};
use ahp_types::{
    actions::{ActionEnvelope, StateAction},
    state::{ChatState, ChatSummary, SessionState, SessionSummary, Snapshot, SnapshotState},
};
use serde_json::json;

pub const CHANNEL_SCHEMA: &str = "
ALTER TABLE interactive_ahp_actions ADD COLUMN channel TEXT;
UPDATE interactive_ahp_actions SET channel='ahp-chat:/'||session;
CREATE TABLE interactive_ahp_sessions (
 session TEXT PRIMARY KEY REFERENCES interactive_sessions(id),
 state TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE interactive_ahp_clients (
 device TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
 client TEXT NOT NULL, version TEXT NOT NULL, boot TEXT NOT NULL,
 updated INTEGER NOT NULL DEFAULT (unixepoch()), PRIMARY KEY(device,client)
);
";
#[derive(Clone, Copy)]
pub struct ChannelSubscription<'a> {
    pub context: &'a AccessContext,
    pub session: &'a str,
    pub chat: bool,
}
pub enum ChannelReplay {
    Actions {
        cursor: StreamCursor,
        actions: Vec<ActionEnvelope>,
    },
    SnapshotRequired,
}
pub struct ChannelCut {
    pub cursor: StreamCursor,
    pub snapshots: Vec<Snapshot>,
}
mod pages;
pub use pages::{
    ChannelHistory, ChannelPageCut, FrozenChatHistory, HISTORY_BYTES, HISTORY_TURNS, HistoryPage,
};
fn summary(chat: &ChatState) -> Result<ChatSummary> {
    Ok(serde_json::from_value(
        json!({"resource":chat.resource,"title":chat.title,"status":chat.status,"modifiedAt":chat.modified_at,"interactivity":chat.interactivity}),
    )?)
}
fn initial(chat: &ChatState, phase: &SessionPhase, ready: bool) -> Result<SessionState> {
    let unavailable = chat
        .meta
        .as_ref()
        .is_some_and(|m| m.get("dunlin.projectionUnavailable") == Some(&json!(true)));
    Ok(serde_json::from_value(
        json!({"provider":"dunlin-acp","title":chat.title,"status":if unavailable{2}else{chat.status & !96},"lifecycle":if unavailable{"failed"}else if ready{"ready"}else if *phase==SessionPhase::Interrupted{"failed"}else{"creating"},"activeClients":[],"chats":[summary(chat)?],"defaultChat":chat.resource,"_meta":{"dunlin.runtimePhase":phase,"dunlin.observerOnly":false,"dunlin.projectionUnavailable":unavailable}}),
    )?)
}
impl Journal<'_> {
    pub fn authorize_channel(&self, context: &AccessContext, id: &str) -> Result<()> {
        self.authorized(context, id, Permission::ViewProject)?;
        Ok(())
    }
    pub fn channel_clock(&self) -> Result<StreamCursor> {
        Ok(self.connection.query_row(
            "SELECT stream,sequence FROM interactive_ahp_clock WHERE singleton=1",
            [],
            |r| {
                Ok(StreamCursor {
                    stream: r.get(0)?,
                    sequence: nonnegative(r, 1)?,
                })
            },
        )?)
    }
    /// Store migration inside its existing transaction, before accepting clients.
    pub fn backfill_channels(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict("Channel migration requires a transaction"));
        }
        let ids = {
            let mut q = self
                .connection
                .prepare("SELECT session FROM interactive_ahp_chats ORDER BY session")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for id in ids {
            self.connection.execute("UPDATE interactive_ahp_chats SET state=json_set(state,'$.interactivity','read-only') WHERE session=?1",[&id])?;
            self.project_session(&id, 0, "baseline")?;
        }
        self.connection
            .execute("UPDATE interactive_ahp_clock SET replay_floor=sequence", [])?;
        Ok(())
    }
    pub(super) fn project_session(&self, id: &str, source_seq: u64, kind: &str) -> Result<()> {
        let chat: ChatState = serde_json::from_str(&self.connection.query_row::<String, _, _>(
            "SELECT state FROM interactive_ahp_chats WHERE session=?1",
            [id],
            |r| r.get(0),
        )?)?;
        let phase = self.session(id)?.phase;
        let ready:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_events WHERE session=?1 AND kind='harness_ready')",[id],|r|r.get(0))?;
        let previous = self
            .connection
            .query_row(
                "SELECT state FROM interactive_ahp_sessions WHERE session=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let mut state = if let Some(raw) = &previous {
            serde_json::from_str::<SessionState>(raw)?
        } else {
            initial(&chat, &phase, ready)?
        };
        let mut actions = Vec::new();
        if previous.is_some() && state.chats.first() != Some(&summary(&chat)?) {
            actions.push(serde_json::from_value::<StateAction>(json!({"type":"session/chatUpdated","chat":chat.resource,"changes":{"title":chat.title,"status":chat.status,"modifiedAt":chat.modified_at}}))?);
        }
        if kind == "harness_ready" {
            actions.push(serde_json::from_value(json!({"type":"session/ready"}))?);
        }
        if matches!(kind, "runtime_interrupted" | "recovery_required") && !ready {
            actions.push(serde_json::from_value(json!({"type":"session/creationFailed","error":{"errorType":"runtime_interrupted","message":"The runtime stopped before becoming ready."}}))?);
        }
        let unavailable = chat
            .meta
            .as_ref()
            .is_some_and(|m| m.get("dunlin.projectionUnavailable") == Some(&json!(true)));
        let (pending, unconfirmed) = self.cancellation_counts(id)?;
        let (permission_pending, permission_unconfirmed) = self.permission_counts(id)?;
        let meta = json!({"dunlin.runtimePhase":phase,"dunlin.observerOnly":false,"dunlin.projectionUnavailable":unavailable,"dunlin.cancellationPending":pending>0,"dunlin.cancellationUnconfirmed":unconfirmed>0,"dunlin.permissionPending":permission_pending>0,"dunlin.permissionUnconfirmed":permission_unconfirmed>0});
        if previous.is_some() && serde_json::to_value(&state.meta)? != meta {
            actions.push(serde_json::from_value(
                json!({"type":"session/metaChanged","_meta":meta}),
            )?);
        }
        for (ordinal, action) in actions.iter().enumerate() {
            if apply_action_to_session(&mut state, action) != ReduceOutcome::Applied {
                return Err(Error::Conflict("Session reducer rejected projection"));
            }
            self.connection
                .execute("UPDATE interactive_ahp_clock SET sequence=sequence+1", [])?;
            let seq = self.channel_clock()?.sequence;
            self.connection.execute("INSERT INTO interactive_ahp_actions(server_seq,session,source_seq,ordinal,action,channel) VALUES(?1,?2,?3,?4,?5,?6)",params![sql_integer(seq)?,id,sql_integer(source_seq)?,1000+ordinal as i64,serde_json::to_string(action)?,format!("ahp-session:/{id}")])?;
        }
        // The wire specification defines this aggregate; SDK 1.0.0's session
        // reducer updates chat summaries but does not derive the top-level bits.
        // Consumers also receive root/sessionSummaryChanged with the aggregate.
        state.status = if unavailable
            || phase == SessionPhase::Interrupted
            || unconfirmed > 0
            || permission_unconfirmed > 0
            || state.lifecycle == ahp_types::state::SessionLifecycle::Failed
        {
            2
        } else {
            (state.status & 96) | (chat.status & !96)
        };
        let added: i64 = actions
            .iter()
            .map(|a| serde_json::to_string(a).map(|s| s.len() as i64))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        self.connection.execute("UPDATE interactive_ahp_chats SET action_count=action_count+?2,action_bytes=action_bytes+?3 WHERE session=?1",params![id,actions.len() as i64,added])?;
        let (count, bytes): (i64, i64) = self.connection.query_row(
            "SELECT action_count,action_bytes FROM interactive_ahp_chats WHERE session=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let recovery = matches!(
            kind,
            "runtime_interrupted"
                | "recovery_required"
                | "cancellation_settled"
                | "permission_lifecycle"
                | "baseline"
        );
        if count > if recovery { 4096 } else { 4064 }
            || bytes
                > if recovery {
                    16 * 1024 * 1024
                } else {
                    16 * 1024 * 1024 - 131072
                }
        {
            return Err(Error::Limit("AHP channel journal limit reached"));
        }
        let created=self.connection.query_row("SELECT json_extract(payload,'$._recorded_at') FROM interactive_events WHERE session=?1 AND kind='session_created'",[id],|r|r.get::<_,Option<String>>(0))?.unwrap_or_else(||chat.modified_at.clone());
        self.connection.execute("INSERT INTO interactive_ahp_sessions VALUES(?1,?2,?3) ON CONFLICT(session) DO UPDATE SET state=excluded.state",params![id,serde_json::to_string(&state)?,created])?;
        Ok(())
    }
    pub fn channel_session(&self, context: &AccessContext, id: &str) -> Result<SessionState> {
        self.projected_chat(context, id)?;
        let raw: String = self.connection.query_row(
            "SELECT state FROM interactive_ahp_sessions WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn channel_summary(&self, context: &AccessContext, id: &str) -> Result<SessionSummary> {
        self.authorize_channel(context, id)?;
        let (raw, created): (String, String) = self.connection.query_row(
            "SELECT state,created_at FROM interactive_ahp_sessions WHERE session=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let state: SessionState = serde_json::from_str(&raw)?;
        let chat = state
            .chats
            .first()
            .ok_or(Error::Conflict("Missing default chat summary"))?;
        Ok(serde_json::from_value(
            json!({"provider":state.provider,"resource":format!("ahp-session:/{id}"),"title":state.title,"status":state.status,"createdAt":created,"modifiedAt":chat.modified_at,"chats":[{"resource":chat.resource,"title":chat.title,"status":chat.status,"interactivity":chat.interactivity}],"defaultChat":chat.resource,"_meta":state.meta}),
        )?)
    }
    pub fn channel_snapshots(
        &self,
        subscriptions: &[ChannelSubscription<'_>],
    ) -> Result<ChannelCut> {
        let cut =
            self.channel_snapshots_with_views(subscriptions, &std::collections::BTreeMap::new())?;
        Ok(ChannelCut {
            cursor: cut.cursor,
            snapshots: cut.snapshots,
        })
    }
    pub fn channel_snapshots_with_views(
        &self,
        subscriptions: &[ChannelSubscription<'_>],
        views: &std::collections::BTreeMap<String, usize>,
    ) -> Result<ChannelPageCut> {
        if subscriptions.len() > 32 {
            return Err(Error::Limit("Channel subscription limit"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let cursor = self.channel_clock()?;
        let mut snapshots = Vec::new();
        let mut bytes = 0;
        let mut histories = Vec::new();
        let mut history_bytes = 0usize;
        let mut history_turns = 0usize;
        for s in subscriptions {
            let (resource, state) = if s.chat {
                let chat = self.projected_chat(s.context, s.session)?;
                let chat = if let Some(requested) = views.get(&chat.resource) {
                    let (chat, history) = FrozenChatHistory::shape(chat, &cursor, *requested)?;
                    if let Some(history) = history {
                        history_bytes = history_bytes
                            .checked_add(history.history.bytes)
                            .ok_or(Error::Limit("Multi-channel history byte limit"))?;
                        history_turns = history_turns
                            .checked_add(history.history.turn_count())
                            .ok_or(Error::Limit("Multi-channel history turn limit"))?;
                        if history_bytes > HISTORY_BYTES || history_turns > HISTORY_TURNS {
                            return Err(Error::Limit(
                                "Multi-channel frozen history budget reached",
                            ));
                        }
                        histories.push(history);
                    }
                    chat
                } else {
                    chat
                };
                (chat.resource.clone(), SnapshotState::Chat(Box::new(chat)))
            } else {
                (
                    format!("ahp-session:/{}", s.session),
                    SnapshotState::Session(Box::new(self.channel_session(s.context, s.session)?)),
                )
            };
            let snapshot = Snapshot {
                resource,
                state,
                from_seq: sql_integer(cursor.sequence)?,
            };
            bytes += serde_json::to_vec(&snapshot)?.len();
            if bytes > 900 * 1024 {
                return Err(Error::Limit("AHP snapshot requires pagination"));
            }
            snapshots.push(snapshot);
        }
        tx.commit()?;
        Ok(ChannelPageCut {
            cursor,
            snapshots,
            histories,
        })
    }
    pub fn channel_replay(
        &self,
        subscriptions: &[ChannelSubscription<'_>],
        after: u64,
    ) -> Result<ChannelReplay> {
        if subscriptions.len() > 32 {
            return Err(Error::Limit("Channel subscription limit"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let cursor = self.channel_clock()?;
        let floor = self.connection.query_row(
            "SELECT replay_floor FROM interactive_ahp_clock",
            [],
            |r| nonnegative(r, 0),
        )?;
        let mut channels = std::collections::BTreeSet::new();
        for s in subscriptions {
            self.authorized(s.context, s.session, Permission::ViewProject)?;
            channels.insert(format!(
                "{}{}",
                if s.chat {
                    "ahp-chat:/"
                } else {
                    "ahp-session:/"
                },
                s.session
            ));
        }
        if after < floor || after > cursor.sequence {
            return Ok(ChannelReplay::SnapshotRequired);
        }
        if channels.is_empty() {
            return Ok(ChannelReplay::Actions {
                cursor,
                actions: vec![],
            });
        }
        let placeholders = (0..channels.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT server_seq,channel,action,origin,rejection_reason FROM interactive_ahp_actions WHERE server_seq>?1 AND channel IN ({placeholders}) ORDER BY server_seq LIMIT 129"
        );
        let mut params = vec![rusqlite::types::Value::Integer(sql_integer(after)?)];
        params.extend(channels.into_iter().map(rusqlite::types::Value::Text));
        let mut q = self.connection.prepare(&sql)?;
        let mut rows = q.query(rusqlite::params_from_iter(params))?;
        let mut actions = Vec::new();
        let mut bytes = 0;
        while let Some(r) = rows.next()? {
            let action = ActionEnvelope {
                server_seq: nonnegative(r, 0)?,
                channel: r.get(1)?,
                action: serde_json::from_str(&r.get::<_, String>(2)?)?,
                origin: r
                    .get::<_, Option<String>>(3)?
                    .map(|raw| serde_json::from_str(&raw))
                    .transpose()?,
                rejection_reason: r.get(4)?,
            };
            bytes += serde_json::to_vec(&action)?.len();
            if actions.len() >= 128 || bytes > 900 * 1024 {
                return Ok(ChannelReplay::SnapshotRequired);
            }
            actions.push(action);
        }
        drop(rows);
        drop(q);
        tx.commit()?;
        Ok(ChannelReplay::Actions { cursor, actions })
    }
}

#[cfg(test)]
mod tests;
