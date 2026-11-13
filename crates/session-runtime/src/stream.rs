//! Durable order across interactive journals. This is an internal source log,
//! not an AHP action stream: projection may allocate multiple wire actions.
use super::*;
use std::collections::BTreeSet;

/// Applied after the original journal schema, including on an existing v17 DB.
/// Old journals had no cross-session order. Backfilled rows form a snapshot
/// baseline, never a fabricated replay history.
pub const STREAM_SCHEMA: &str = "
CREATE TABLE interactive_stream_clock (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 stream TEXT NOT NULL,
 sequence INTEGER NOT NULL CHECK(sequence>=0 AND sequence<=9007199254740991),
 replay_floor INTEGER NOT NULL CHECK(replay_floor>=0 AND replay_floor<=sequence)
);
CREATE TABLE interactive_stream_events (
 server_seq INTEGER PRIMARY KEY CHECK(server_seq>0),
 session TEXT NOT NULL,
 session_seq INTEGER NOT NULL,
 UNIQUE(session,session_seq),
 FOREIGN KEY(session,session_seq) REFERENCES interactive_events(session,seq) ON DELETE RESTRICT
);
CREATE INDEX interactive_stream_by_session ON interactive_stream_events(session,server_seq);
INSERT INTO interactive_stream_events(server_seq,session,session_seq)
 SELECT row_number() OVER (ORDER BY session,seq),session,seq FROM interactive_events;
INSERT INTO interactive_stream_clock
 SELECT 1,lower(hex(randomblob(16))),count(*),count(*) FROM interactive_stream_events;
CREATE TRIGGER interactive_event_stream AFTER INSERT ON interactive_events BEGIN
 UPDATE interactive_stream_clock SET sequence=sequence+1 WHERE singleton=1;
 SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Interactive stream clock unavailable') END;
 INSERT INTO interactive_stream_events(server_seq,session,session_seq)
 SELECT sequence,NEW.session,NEW.seq FROM interactive_stream_clock WHERE singleton=1;
END;
";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StreamCursor {
    pub stream: String,
    pub sequence: u64,
}
/// A caller must reload each context under the Store lock before invoking any
/// stream operation. Possessing a cursor or subscription is never authority.
pub struct Subscription<'a> {
    pub context: &'a AccessContext,
    pub session: &'a str,
}
#[derive(Debug, Serialize)]
pub struct StreamSnapshot {
    pub cursor: StreamCursor,
    pub sessions: Vec<(Session, Vec<Turn>)>,
}
#[derive(Debug, Serialize)]
pub struct StreamEvent {
    pub server_sequence: u64,
    pub session: String,
    pub event: Event,
}
#[derive(Debug)]
pub enum StreamReplay {
    /// All matching records through this cursor, not a partially replayed page.
    Events {
        cursor: StreamCursor,
        events: Vec<StreamEvent>,
    },
    /// Unknown stream, future cursor, missing history, or response over budget.
    /// The adapter must obtain fresh snapshots instead of silently skipping data.
    SnapshotRequired,
}
const SUBSCRIPTIONS: usize = 32;
const REPLAY_EVENTS: usize = 128;
const REPLAY_BYTES: usize = 1024 * 1024;
impl Journal<'_> {
    fn stream_clock(&self) -> Result<(StreamCursor, u64)> {
        Ok(self.connection.query_row(
            "SELECT stream,sequence,replay_floor FROM interactive_stream_clock WHERE singleton=1",
            [],
            |r| {
                Ok((
                    StreamCursor {
                        stream: r.get(0)?,
                        sequence: nonnegative(r, 1)?,
                    },
                    nonnegative(r, 2)?,
                ))
            },
        )?)
    }
    fn authorize_subscriptions(&self, subscriptions: &[Subscription<'_>]) -> Result<()> {
        if subscriptions.is_empty() || subscriptions.len() > SUBSCRIPTIONS {
            return Err(Error::Limit("Invalid interactive subscription count"));
        }
        let mut ids = BTreeSet::new();
        for subscription in subscriptions {
            if !ids.insert(subscription.session) {
                return Err(Error::Conflict("Duplicate interactive subscription"));
            }
            self.authorized(
                subscription.context,
                subscription.session,
                Permission::ViewProject,
            )?;
        }
        Ok(())
    }
    /// One SQLite read transaction establishes a shared cut across all sessions.
    pub fn snapshot_stream(&self, subscriptions: &[Subscription<'_>]) -> Result<StreamSnapshot> {
        let tx = self.connection.unchecked_transaction()?;
        self.authorize_subscriptions(subscriptions)?;
        let (cursor, _) = self.stream_clock()?;
        let mut prompt_bytes = 0i64;
        for subscription in subscriptions {
            let size:i64 = self.connection.query_row("SELECT coalesce(sum(length(CAST(prompt AS BLOB))),0) FROM interactive_turns WHERE session=?1",[subscription.session],|r|r.get(0))?;
            prompt_bytes = prompt_bytes.saturating_add(size);
            if prompt_bytes > REPLAY_BYTES as i64 {
                return Err(Error::Limit("Interactive snapshot requires pagination"));
            }
        }
        let sessions = subscriptions
            .iter()
            .map(|s| self.snapshot_rows(s.context, s.session))
            .collect::<Result<Vec<_>>>()?;
        let snapshot = StreamSnapshot { cursor, sessions };
        if serde_json::to_vec(&snapshot)?.len() > REPLAY_BYTES {
            return Err(Error::Limit("Interactive snapshot requires pagination"));
        }
        tx.commit()?;
        Ok(snapshot)
    }
    /// Bounded, complete replay for the explicit subscriptions only. A future
    /// transport must attach its invalidation receiver BEFORE taking this cut.
    pub fn replay_stream(
        &self,
        subscriptions: &[Subscription<'_>],
        after: &StreamCursor,
    ) -> Result<StreamReplay> {
        let tx = self.connection.unchecked_transaction()?;
        self.authorize_subscriptions(subscriptions)?;
        let (cursor, floor) = self.stream_clock()?;
        if after.stream != cursor.stream
            || after.sequence < floor
            || after.sequence > cursor.sequence
        {
            return Ok(StreamReplay::SnapshotRequired);
        }
        let placeholders = (0..subscriptions.len())
            .map(|i| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(",");
        // Only placeholder numbers are formatted; identifiers are bound values.
        let sql = format!(
            "SELECT x.server_seq,e.session,e.seq,e.kind,e.payload FROM interactive_stream_events x JOIN interactive_events e ON e.session=x.session AND e.seq=x.session_seq WHERE x.server_seq>?1 AND x.server_seq<=?2 AND x.session IN ({placeholders}) ORDER BY x.server_seq LIMIT {}",
            REPLAY_EVENTS + 1
        );
        let mut params = vec![
            rusqlite::types::Value::Integer(sql_integer(after.sequence)?),
            rusqlite::types::Value::Integer(sql_integer(cursor.sequence)?),
        ];
        params.extend(
            subscriptions
                .iter()
                .map(|s| rusqlite::types::Value::Text(s.session.into())),
        );
        let mut query = self.connection.prepare(&sql)?;
        let mut rows = query.query(rusqlite::params_from_iter(params))?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(4)?;
            if events.len() >= REPLAY_EVENTS {
                return Ok(StreamReplay::SnapshotRequired);
            }
            let event = StreamEvent {
                server_sequence: nonnegative(row, 0)?,
                session: row.get(1)?,
                event: Event {
                    sequence: nonnegative(row, 2)?,
                    kind: row.get(3)?,
                    payload: serde_json::from_str(&raw)?,
                },
            };
            bytes = bytes.saturating_add(serde_json::to_vec(&event)?.len());
            if bytes > REPLAY_BYTES - 4096 {
                return Ok(StreamReplay::SnapshotRequired);
            }
            events.push(event);
        }
        drop(rows);
        drop(query);
        tx.commit()?;
        Ok(StreamReplay::Events { cursor, events })
    }
}

#[cfg(test)]
mod tests;
