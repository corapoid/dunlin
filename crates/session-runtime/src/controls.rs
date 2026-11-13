//! Queued message controls and durable, fenced cancellation acknowledgement.
use super::*;
use crate::dispatch::identifier;
use ahp_types::{
    actions::StateAction,
    state::{MessageKind, PendingMessageKind},
};
use serde_json::json;
use std::collections::BTreeSet;

pub const CONTROL_SCHEMA: &str = "
CREATE TABLE interactive_cancellation_intents (
 session TEXT NOT NULL REFERENCES interactive_sessions(id),
 turn_id TEXT NOT NULL REFERENCES interactive_turns(id), epoch INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('requested','confirmed','unconfirmed')),
 evidence TEXT, duration_ms INTEGER,
 PRIMARY KEY(session,turn_id)
);
";
#[derive(Clone, Copy, Debug, Default)]
pub struct DispatchAdmission<'a> {
    pub rejection: Option<&'a str>,
    pub elapsed_ms: Option<u64>,
}
impl Journal<'_> {
    fn controls_present(&self) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='interactive_cancellation_intents')",[],|r|r.get(0))?)
    }
    pub(super) fn cancellation_known(&self, id: &str, turn: &str) -> Result<bool> {
        if !self.controls_present()? {
            return Ok(false);
        }
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND turn_id=?2)",params![id,turn],|r|r.get(0))?)
    }
    pub(super) fn cancellation_pending(&self, id: &str, turn: &str) -> Result<bool> {
        if !self.controls_present()? {
            return Ok(false);
        }
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND turn_id=?2 AND state='requested')",params![id,turn],|r|r.get(0))?)
    }
    pub(super) fn cancellation_counts(&self, id: &str) -> Result<(i64, i64)> {
        if !self.controls_present()? {
            return Ok((0, 0));
        }
        Ok(self.connection.query_row("SELECT coalesce(sum(state='requested'),0),coalesce(sum(state='unconfirmed'),0) FROM interactive_cancellation_intents WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?)
    }
    pub(super) fn unconfirm_cancellations(&self, id: &str) -> Result<()> {
        if self.controls_present()? {
            self.connection.execute("UPDATE interactive_cancellation_intents SET state='unconfirmed',evidence='runtime_interrupted' WHERE session=?1 AND state='requested'",[id])?;
        }
        Ok(())
    }
    pub fn cancellation_requested(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
    ) -> Result<bool> {
        self.leased(context, id, lease)?;
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_cancellation_intents WHERE session=?1 AND turn_id=?2 AND epoch=?3 AND state='requested')",params![id,turn,sql_integer(lease.epoch)?],|r|r.get(0))?)
    }
    pub fn settle_cancellation(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        evidence: &str,
        duration_ms: u64,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.leased(context, id, lease)?;
        let changed = self.settle_cancellation_inner(id, lease, turn, evidence, duration_ms)?;
        tx.commit()?;
        if changed {
            self.changed();
        }
        Ok(())
    }
    pub(super) fn settle_cancellation_inner(
        &self,
        id: &str,
        lease: &Lease,
        turn: &str,
        evidence: &str,
        duration_ms: u64,
    ) -> Result<bool> {
        if evidence.is_empty() || evidence.len() > 256 || duration_ms > i64::MAX as u64 {
            return Err(Error::Limit("Invalid cancellation evidence"));
        }
        let state:Option<String>=self.connection.query_row("SELECT state FROM interactive_cancellation_intents WHERE session=?1 AND turn_id=?2 AND epoch=?3",params![id,turn,sql_integer(lease.epoch)?],|r|r.get(0)).optional()?;
        if state.as_deref() == Some("confirmed") {
            return Ok(false);
        }
        if state.as_deref() != Some("requested") {
            return Err(Error::Conflict(
                "Cancellation was not requested for this lease",
            ));
        }
        let n=self.connection.execute("UPDATE interactive_turns SET phase='cancelled' WHERE session=?1 AND id=?2 AND phase IN ('running','dispatching')",params![id,turn])?;
        if n != 1 {
            return Err(Error::Conflict("Cancellation target is not active"));
        }
        self.connection.execute("UPDATE interactive_cancellation_intents SET state='confirmed',evidence=?4,duration_ms=?5 WHERE session=?1 AND turn_id=?2 AND epoch=?3",params![id,turn,sql_integer(lease.epoch)?,evidence,sql_integer(duration_ms)?])?;
        self.event(
            id,
            "cancellation_settled",
            json!({"turn_id":turn,"evidence":evidence,"duration_ms":duration_ms}),
            true,
        )?;
        Ok(true)
    }
    fn message_turn(&self, id: &str, message: &str) -> Result<Option<(String, String)>> {
        Ok(self.connection.query_row("SELECT t.id,t.phase FROM interactive_turns t LEFT JOIN interactive_message_ids m ON m.turn_id=t.id AND m.session=t.session WHERE t.session=?1 AND coalesce(m.message_id,t.id)=?2",params![id,message],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)
    }
    pub(super) fn control_rejection(
        &self,
        id: &str,
        action: &StateAction,
    ) -> Result<Option<&'static str>> {
        let invalid = Some("Unsupported or invalid chat control");
        match action {
            StateAction::ChatToolCallConfirmed(_) => self.permission_rejection(id, action),
            StateAction::ChatInputCompleted(a) => self.input_rejection(id, &a.request_id),
            StateAction::ChatPendingMessageSet(a) => {
                let value = serde_json::to_value(action)?;
                if a.kind != PendingMessageKind::Queued
                    || !identifier(&a.id)
                    || a.message.origin.kind != MessageKind::User
                    || a.message.text.trim().is_empty()
                    || a.message.text.len() > 32768
                    || value
                        != json!({"type":"chat/pendingMessageSet","kind":"queued","id":a.id,"message":{"text":a.message.text,"origin":{"kind":"user"}}})
                {
                    return Ok(invalid);
                }
                if let Some((_, phase)) = self.message_turn(id, &a.id)? {
                    return Ok(if phase == "queued" {
                        None
                    } else {
                        Some("Message is no longer queued")
                    });
                }
                let (total,queued):(i64,i64)=self.connection.query_row("SELECT count(*),coalesce(sum(phase='queued'),0) FROM interactive_turns WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
                Ok(if total >= 256 || queued >= 32 {
                    Some("Session turn queue limit reached")
                } else {
                    None
                })
            }
            StateAction::ChatPendingMessageRemoved(a) => {
                if a.kind != PendingMessageKind::Queued || !identifier(&a.id) {
                    return Ok(invalid);
                }
                Ok(match self.message_turn(id, &a.id)? {
                    Some((_, phase)) if phase != "queued" => Some("Message is no longer queued"),
                    _ => None,
                })
            }
            StateAction::ChatQueuedMessagesReordered(a) => Ok(
                if a.order.len() > 32
                    || a.order.iter().any(|s| !identifier(s))
                    || a.order.iter().collect::<BTreeSet<_>>().len() != a.order.len()
                {
                    invalid
                } else {
                    None
                },
            ),
            StateAction::ChatTurnCancelled(a) => {
                let active:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase IN ('running','dispatching'))",params![id,a.turn_id],|r|r.get(0))?;
                let chat = self.projected_chat_state(id)?;
                Ok(
                    if !active
                        || chat.active_turn.as_ref().is_none_or(|t| t.id != a.turn_id)
                        || self.cancellation_known(id, &a.turn_id)?
                    {
                        Some("Turn is no longer cancellable")
                    } else {
                        None
                    },
                )
            }
            _ => Ok(invalid),
        }
    }
    fn projected_chat_state(&self, id: &str) -> Result<ahp_types::state::ChatState> {
        let raw: String = self.connection.query_row(
            "SELECT state FROM interactive_ahp_chats WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub(super) fn apply_control(
        &self,
        id: &str,
        action: &StateAction,
        elapsed_ms: Option<u64>,
    ) -> Result<StateAction> {
        match action {
            StateAction::ChatPendingMessageSet(a) => {
                if let Some((turn, _)) = self.message_turn(id, &a.id)? {
                    self.connection.execute("UPDATE interactive_turns SET prompt=?3,fingerprint=?4 WHERE session=?1 AND id=?2 AND phase='queued'",params![id,turn,a.message.text,hex::encode(Sha256::digest(a.message.text.as_bytes()))])?;
                } else {
                    let turn = format!("turn-{}", uuid::Uuid::now_v7());
                    let key = format!("ahp:{}", uuid::Uuid::now_v7());
                    let position:u64=self.connection.query_row("SELECT coalesce(max(position),0)+1 FROM interactive_turns WHERE session=?1",[id],|r|nonnegative(r,0))?;
                    self.connection.execute(
                        "INSERT INTO interactive_turns VALUES(?1,?2,?3,?4,?5,'queued',?6)",
                        params![
                            turn,
                            id,
                            key,
                            hex::encode(Sha256::digest(a.message.text.as_bytes())),
                            a.message.text,
                            sql_integer(position)?
                        ],
                    )?;
                    self.connection.execute(
                        "INSERT INTO interactive_message_ids VALUES(?1,?2,?3)",
                        params![id, a.id, turn],
                    )?;
                }
            }
            StateAction::ChatPendingMessageRemoved(a) => {
                if let Some((turn, _)) = self.message_turn(id, &a.id)? {
                    self.connection.execute("UPDATE interactive_turns SET phase='cancelled' WHERE session=?1 AND id=?2 AND phase='queued'",params![id,turn])?;
                }
            }
            StateAction::ChatQueuedMessagesReordered(a) => {
                let mut q=self.connection.prepare("SELECT t.id,coalesce(m.message_id,t.id) FROM interactive_turns t LEFT JOIN interactive_message_ids m ON m.turn_id=t.id AND m.session=t.session WHERE t.session=?1 AND t.phase='queued' ORDER BY t.position")?;
                let queued = q
                    .query_map([id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                drop(q);
                let mut ordered = Vec::new();
                for requested in &a.order {
                    if let Some((turn, _)) = queued.iter().find(|(_, message)| message == requested)
                    {
                        ordered.push(turn.clone());
                    }
                }
                for (turn, _) in &queued {
                    if !ordered.contains(turn) {
                        ordered.push(turn.clone());
                    }
                }
                let messages: Vec<_> = ordered
                    .iter()
                    .map(|turn| {
                        queued
                            .iter()
                            .find(|(id, _)| id == turn)
                            .map(|(_, message)| message.clone())
                            .ok_or(Error::Conflict("Missing ordered message"))
                    })
                    .collect::<Result<_>>()?;
                for (index, turn) in ordered.iter().enumerate() {
                    self.connection.execute(
                        "UPDATE interactive_turns SET position=?3 WHERE session=?1 AND id=?2",
                        params![id, turn, index as i64 + 1],
                    )?;
                }
                // Pinned SDK sorts omitted IDs; supply the complete host order
                // so the official reducer and durable FIFO remain identical.
                return Ok(serde_json::from_value(
                    json!({"type":"chat/queuedMessagesReordered","order":messages}),
                )?);
            }
            StateAction::ChatTurnCancelled(a) => {
                self.invalidate_permissions_inner(id, None, Some(&a.turn_id), "turn_cancelled")?;
                let duration = elapsed_ms.unwrap_or(0);
                if duration > i64::MAX as u64 {
                    return Err(Error::Limit("Invalid cancellation duration"));
                }
                let epoch = self.session(id)?.epoch;
                self.connection.execute("INSERT INTO interactive_cancellation_intents(session,turn_id,epoch,state) VALUES(?1,?2,?3,'requested')",params![id,a.turn_id,sql_integer(epoch)?])?;
                return Ok(serde_json::from_value(
                    json!({"type":"chat/turnCancelled","turnId":a.turn_id,"duration":duration,"_meta":{"dunlin.durationUnknown":elapsed_ms.is_none(),"dunlin.cancellationPending":true}}),
                )?);
            }
            _ => return Err(Error::Conflict("Unsupported chat control")),
        }
        Ok(action.clone())
    }
}
#[cfg(test)]
mod tests;
