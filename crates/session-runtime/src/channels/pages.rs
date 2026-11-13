//! Immutable connection-view history. Pages never change canonical chat state.
use super::*;
use ahp_types::state::Turn;
use std::sync::Arc;

pub const HISTORY_BYTES: usize = 8 * 1024 * 1024;
pub const HISTORY_TURNS: usize = 512;
pub const HISTORY_PAGE_BYTES: usize = 512 * 1024;
pub const HISTORY_PAGE_TURNS: usize = 32;
const SNAPSHOT_BYTES: usize = 900 * 1024;

#[derive(Clone)]
pub struct FrozenChatHistory {
    pub resource: String,
    pub cut: StreamCursor,
    pub bytes: usize,
    turns: Arc<Vec<Turn>>,
    sizes: Arc<Vec<usize>>,
}
pub struct HistoryPage {
    pub turns: Vec<Turn>,
    pub next_before: Option<usize>,
}
pub struct ChannelHistory {
    pub history: FrozenChatHistory,
    pub before: usize,
}
pub struct ChannelPageCut {
    pub cursor: StreamCursor,
    pub snapshots: Vec<Snapshot>,
    pub histories: Vec<ChannelHistory>,
}
impl FrozenChatHistory {
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }
    pub fn page(&self, before: usize) -> Result<HistoryPage> {
        if before == 0 || before > self.turns.len() {
            return Err(Error::Conflict("Invalid history page boundary"));
        }
        let mut start = before;
        let mut bytes = 0;
        while start > 0 && before - start < HISTORY_PAGE_TURNS {
            let size = self.sizes[start - 1] + 1;
            if bytes + size > HISTORY_PAGE_BYTES {
                break;
            }
            bytes += size;
            start -= 1;
        }
        if start == before {
            return Err(Error::Limit(
                "One historical turn exceeds the history page byte budget",
            ));
        }
        Ok(HistoryPage {
            turns: self.turns[start..before].to_vec(),
            next_before: (start > 0).then_some(start),
        })
    }
    pub(super) fn shape(
        mut chat: ChatState,
        cut: &StreamCursor,
        requested: usize,
    ) -> Result<(ChatState, Option<ChannelHistory>)> {
        if requested > 64 {
            return Err(Error::Limit(
                "Chat view turn count must be between 0 and 64",
            ));
        }
        let turns = std::mem::take(&mut chat.turns);
        let mut unique = std::collections::BTreeSet::new();
        if turns.iter().any(|turn| !unique.insert(turn.id.clone())) {
            return Err(Error::Conflict("Duplicate canonical history turn"));
        }
        chat.turns_next_cursor = None;
        let mut bytes = serde_json::to_vec(&chat)?.len() + 128; // opaque cursor/envelope room
        if bytes > SNAPSHOT_BYTES {
            return Err(Error::Limit(
                "Active turn or queued state exceeds the snapshot byte budget",
            ));
        }
        let end = turns.len();
        let sizes = turns
            .iter()
            .map(|turn| serde_json::to_vec(turn).map(|raw| raw.len()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let retained_bytes = sizes.iter().sum::<usize>() + end.saturating_sub(1) + 2;
        if retained_bytes > HISTORY_BYTES || end > HISTORY_TURNS {
            return Err(Error::Limit(
                "Frozen history exceeds the connection history budget",
            ));
        }
        let mut start = end;
        while start > 0 && end - start < requested {
            let size = sizes[start - 1] + 1;
            if bytes + size > SNAPSHOT_BYTES {
                break;
            }
            bytes += size;
            start -= 1;
        }
        if requested > 0 && end > 0 && start == end {
            return Err(Error::Limit(
                "One completed turn exceeds the snapshot byte budget",
            ));
        }
        chat.turns = turns[start..].to_vec();
        if start == 0 {
            return Ok((chat, None));
        }
        let resource = chat.resource.clone();
        Ok((
            chat,
            Some(ChannelHistory {
                history: Self {
                    resource,
                    cut: cut.clone(),
                    bytes: retained_bytes,
                    turns: Arc::new(turns),
                    sizes: Arc::new(sizes),
                },
                before: start,
            }),
        ))
    }
}

impl Journal<'_> {
    /// Reserve a unique serverSeq for a requester-only view action. No history
    /// payload is written to canonical actions or broadcast to other observers.
    /// Global sequence gaps already occur for unsubscribed channels and are valid.
    pub fn reserve_history_sequence(&self, context: &AccessContext, id: &str) -> Result<u64> {
        let tx = self.connection.unchecked_transaction()?;
        self.authorize_channel(context, id)?;
        let current = self.channel_clock()?.sequence;
        let next = current
            .checked_add(1)
            .filter(|value| *value <= 9_007_199_254_740_991)
            .ok_or(Error::Limit("AHP sequence limit reached"))?;
        if self.connection.execute(
            "UPDATE interactive_ahp_clock SET sequence=?1 WHERE singleton=1 AND sequence=?2",
            params![sql_integer(next)?, sql_integer(current)?],
        )? != 1
        {
            return Err(Error::Conflict("AHP history cut changed"));
        }
        tx.commit()?;
        Ok(next)
    }
}
