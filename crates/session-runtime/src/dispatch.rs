//! Durable client dispatch identity and atomic queue acceptance.
use super::*;
use ahp_types::{
    actions::{ActionEnvelope, ActionOrigin},
    commands::DispatchActionParams,
};
use serde_json::json;

pub const DISPATCH_SCHEMA: &str = "
ALTER TABLE interactive_ahp_actions ADD COLUMN origin TEXT;
ALTER TABLE interactive_ahp_actions ADD COLUMN rejection_reason TEXT;
CREATE TABLE interactive_client_dispatch (
 device TEXT NOT NULL, client TEXT NOT NULL, client_seq INTEGER NOT NULL,
 fingerprint TEXT NOT NULL, envelope TEXT NOT NULL,
 PRIMARY KEY(device,client,client_seq)
);
CREATE TABLE interactive_message_ids (
 session TEXT NOT NULL REFERENCES interactive_sessions(id),
 message_id TEXT NOT NULL, turn_id TEXT NOT NULL UNIQUE REFERENCES interactive_turns(id),
 PRIMARY KEY(session,message_id)
);
";
pub(super) fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}
impl Journal<'_> {
    /// Revalidate actor admission against its exact current fencing lease.
    /// This reads only session authority, never the transcript or prompt queue.
    pub fn authorize_dispatch(
        &self,
        context: &AccessContext,
        session: &str,
        lease: &Lease,
    ) -> Result<()> {
        self.leased(context, session, lease).map(|_| ())
    }

    /// Migration 21, inside the store migration transaction. No stale read-only
    /// snapshots may be resumed through the old replay baseline.
    pub fn backfill_dispatch(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict("Dispatch migration requires a transaction"));
        }
        let mut q = self
            .connection
            .prepare("SELECT session,state FROM interactive_ahp_chats")?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        for (id, raw) in rows {
            let mut state: ahp_types::state::ChatState = serde_json::from_str(&raw)?;
            state.interactivity = Some(ahp_types::state::ChatInteractivity::Full);
            self.connection.execute(
                "UPDATE interactive_ahp_chats SET state=?2 WHERE session=?1",
                params![id, serde_json::to_string(&state)?],
            )?;
            let raw: String = self.connection.query_row(
                "SELECT state FROM interactive_ahp_sessions WHERE session=?1",
                [&id],
                |r| r.get(0),
            )?;
            let mut session: ahp_types::state::SessionState = serde_json::from_str(&raw)?;
            session
                .meta
                .get_or_insert_with(Default::default)
                .insert("dunlin.observerOnly".into(), json!(false));
            for chat in &mut session.chats {
                if chat.resource == state.resource {
                    chat.interactivity = state.interactivity;
                }
            }
            self.connection.execute(
                "UPDATE interactive_ahp_sessions SET state=?2 WHERE session=?1",
                params![id, serde_json::to_string(&session)?],
            )?;
        }
        self.connection
            .execute("UPDATE interactive_ahp_clock SET replay_floor=sequence", [])?;
        Ok(())
    }

    pub(super) fn pending_message_id(&self, session: &str, turn: &str) -> Result<String> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='interactive_message_ids')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(turn.into());
        }
        Ok(self
            .connection
            .query_row(
                "SELECT message_id FROM interactive_message_ids WHERE session=?1 AND turn_id=?2",
                params![session, turn],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| turn.into()))
    }
    /// Accept a queued user text or persist its rejection. All durable effects
    /// share one transaction; callers supply a freshly evaluated actor gate.
    pub fn dispatch_text(
        &self,
        context: &AccessContext,
        session: &str,
        device: &str,
        client: &str,
        params: &DispatchActionParams,
        rejection: Option<&str>,
    ) -> Result<ActionEnvelope> {
        self.dispatch_action(
            context,
            session,
            device,
            client,
            params,
            DispatchAdmission {
                rejection,
                elapsed_ms: None,
            },
        )
    }
    pub fn dispatch_action(
        &self,
        context: &AccessContext,
        session: &str,
        device: &str,
        client: &str,
        params: &DispatchActionParams,
        admission: DispatchAdmission<'_>,
    ) -> Result<ActionEnvelope> {
        self.dispatch_action_with_mcp_input(
            context,
            session,
            device,
            client,
            params,
            admission,
            &|_, _| Err(Error::Conflict("MCP form validator is unavailable")),
        )
    }
    /// Form acceptance requires a pure validator of the pinned schema and
    /// decoded content, within this SQL transaction. No I/O or re-entrant SQL.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_action_with_mcp_input(
        &self,
        context: &AccessContext,
        session: &str,
        device: &str,
        client: &str,
        params: &DispatchActionParams,
        admission: DispatchAdmission<'_>,
        validate: &dyn Fn(&serde_json::Value, &serde_json::Value) -> Result<()>,
    ) -> Result<ActionEnvelope> {
        let tx = self.connection.unchecked_transaction()?;
        self.authorized(context, session, Permission::CreateSession)?;
        if !identifier(device)
            || !identifier(client)
            || !(0..=9_007_199_254_740_991).contains(&params.client_seq)
            || params.channel != format!("ahp-chat:/{session}")
        {
            return Err(Error::Conflict("Invalid dispatch identity or channel"));
        }
        // serde_json's default map ordering canonicalizes nested typed payloads.
        let canonical = serde_json::to_vec(&serde_json::to_value(params)?)?;
        if canonical.len() > 65536 {
            return Err(Error::Limit("Dispatch payload limit"));
        }
        let fingerprint = hex::encode(Sha256::digest(&canonical));
        let previous: Option<(String,String)> = tx.query_row("SELECT fingerprint,envelope FROM interactive_client_dispatch WHERE device=?1 AND client=?2 AND client_seq=?3", params![device,client,params.client_seq], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((digest, envelope)) = previous {
            if digest != fingerprint {
                return Err(Error::Conflict(
                    "Client sequence reused with another action",
                ));
            }
            return Ok(serde_json::from_str(&envelope)?);
        }
        let (all, own): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(device=?1),0) FROM interactive_client_dispatch",
            [device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if all >= 8192 || own >= 1024 {
            return Err(Error::Limit("Dispatch ledger capacity reached"));
        }
        let mut reason = admission.rejection.map(str::to_owned);
        if let Some(invalid) = self.control_rejection(session, &params.action)? {
            reason = Some(invalid.into());
        }
        if reason.is_none()
            && let ahp_types::actions::StateAction::ChatInputCompleted(action) = &params.action
            && !self.mcp_input_answers_valid(session, action, validate)?
        {
            reason = Some("MCP form response does not match its schema".into());
        }
        let origin = ActionOrigin {
            client_id: client.into(),
            client_seq: params.client_seq,
        };
        let envelope = if let Some(reason) = reason {
            let reason: String = reason.chars().take(256).collect();
            let raw = serde_json::to_string(&params.action)?;
            let (count, bytes): (i64, i64) = tx.query_row(
                "SELECT action_count,action_bytes FROM interactive_ahp_chats WHERE session=?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if count >= 4064 || bytes + raw.len() as i64 > 16 * 1024 * 1024 - 131072 {
                return Err(Error::Limit("AHP dispatch rejection limit"));
            }
            tx.execute("UPDATE interactive_ahp_clock SET sequence=sequence+1", [])?;
            let seq = self.channel_clock()?.sequence;
            let source = self.session(session)?.sequence;
            tx.execute("INSERT INTO interactive_ahp_actions(server_seq,session,source_seq,ordinal,action,channel,origin,rejection_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![sql_integer(seq)?,session,sql_integer(source)?,-sql_integer(seq)?,raw,params.channel,serde_json::to_string(&origin)?,reason])?;
            tx.execute("UPDATE interactive_ahp_chats SET action_count=action_count+1,action_bytes=action_bytes+?2 WHERE session=?1",params![session,raw.len() as i64])?;
            ActionEnvelope {
                channel: params.channel.clone(),
                action: params.action.clone(),
                server_seq: seq,
                origin: Some(origin),
                rejection_reason: Some(reason),
            }
        } else {
            let current = self.session(session)?;
            if current.phase != SessionPhase::Active || current.runtime.is_none() {
                return Err(Error::Conflict("Runtime lease is not active"));
            }
            context.revalidate(
                current
                    .authority
                    .as_ref()
                    .ok_or(PolicyError::StaleAuthority)?,
                Permission::CreateSession,
            )?;
            let action = if matches!(
                params.action,
                ahp_types::actions::StateAction::ChatToolCallConfirmed(_)
            ) {
                self.apply_permission_decision(context, session, device, client, params)?
            } else if let ahp_types::actions::StateAction::ChatInputCompleted(action) =
                &params.action
            {
                self.apply_mcp_input_decision(context, session, device, action, validate)?
            } else {
                self.apply_control(session, &params.action, admission.elapsed_ms)?
            };
            self.event(session, "client_control", json!({"action":action}), false)?;
            let source = self.session(session)?.sequence;
            let seq = tx.query_row("SELECT server_seq FROM interactive_ahp_actions WHERE session=?1 AND source_seq=?2 AND ordinal=0",params![session,sql_integer(source)?],|r|nonnegative(r,0))?;
            tx.execute(
                "UPDATE interactive_ahp_actions SET origin=?2 WHERE server_seq=?1",
                params![sql_integer(seq)?, serde_json::to_string(&origin)?],
            )?;
            ActionEnvelope {
                channel: params.channel.clone(),
                action,
                server_seq: seq,
                origin: Some(origin),
                rejection_reason: None,
            }
        };
        tx.execute(
            "INSERT INTO interactive_client_dispatch VALUES(?1,?2,?3,?4,?5)",
            params![
                device,
                client,
                params.client_seq,
                fingerprint,
                serde_json::to_string(&envelope)?
            ],
        )?;
        tx.commit()?;
        self.changed();
        Ok(envelope)
    }
}
#[cfg(test)]
mod tests;
