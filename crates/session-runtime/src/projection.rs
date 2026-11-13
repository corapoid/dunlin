//! ACP journal -> typed AHP chat actions, reduced by the pinned official SDK.
//! This module does not implement an AHP server or authorize client actions.
use super::*;
use ahp::reducers::{ReduceOutcome, apply_action_to_chat};
use ahp_types::{
    actions::{ActionEnvelope, StateAction},
    state::{ChatState, ResponsePart},
};
use serde_json::{Value, json};

pub const PROJECTION_SCHEMA: &str = "
CREATE TABLE interactive_ahp_clock (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1), stream TEXT NOT NULL,
 sequence INTEGER NOT NULL CHECK(sequence>=0 AND sequence<=9007199254740991),
 replay_floor INTEGER NOT NULL DEFAULT 0 CHECK(replay_floor>=0 AND replay_floor<=sequence)
);
INSERT INTO interactive_ahp_clock VALUES(1,lower(hex(randomblob(16))),0,0);
CREATE TABLE interactive_ahp_chats (
 session TEXT PRIMARY KEY REFERENCES interactive_sessions(id),
 source_seq INTEGER NOT NULL, state TEXT NOT NULL,
 action_count INTEGER NOT NULL DEFAULT 0, action_bytes INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE interactive_ahp_actions (
 server_seq INTEGER PRIMARY KEY, session TEXT NOT NULL,
 source_seq INTEGER NOT NULL, ordinal INTEGER NOT NULL, action TEXT NOT NULL,
 UNIQUE(session,source_seq,ordinal),
 FOREIGN KEY(session,source_seq) REFERENCES interactive_events(session,seq) ON DELETE RESTRICT
);
CREATE INDEX interactive_ahp_by_session ON interactive_ahp_actions(session,server_seq);
";
fn typed(value: Value) -> Result<StateAction> {
    Ok(serde_json::from_value(value)?)
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or(Error::Conflict("Invalid projection input"))
}
fn blank(id: &str, at: &str) -> Result<ChatState> {
    Ok(serde_json::from_value(
        json!({"resource":format!("ahp-chat:/{id}"),"title":"Session","status":1,"interactivity":"full","modifiedAt":at,"turns":[]}),
    )?)
}
fn message(prompt: String) -> Value {
    json!({"text":prompt,"origin":{"kind":"user"}})
}
fn terminal(turn: &str, phase: &str, payload: &Value) -> Result<StateAction> {
    let duration = payload["duration_ms"]
        .as_u64()
        .and_then(|v| i64::try_from(v).ok());
    let meta = json!({"dunlin.durationUnknown":duration.is_none(),"dunlin.outcome":phase});
    let mut action = match phase {
        "completed" => json!({"type":"chat/turnComplete"}),
        "cancelled" => json!({"type":"chat/turnCancelled"}),
        _ => {
            json!({"type":"chat/error","part":{"kind":"error","error":{"errorType":if phase=="unknown"{"unknown_outcome"}else{"turn_failed"},"message":if phase=="unknown"{"The outcome of this turn is unknown. It was not replayed."}else{"The agent did not complete this turn."}},"resumable":false}})
        }
    };
    action["turnId"] = json!(turn);
    action["duration"] = json!(duration.unwrap_or(0));
    action["_meta"] = meta;
    typed(action)
}
fn chunk(
    state: &ChatState,
    turn: &str,
    content: &str,
    reasoning: bool,
) -> Result<Vec<StateAction>> {
    let active = state
        .active_turn
        .as_ref()
        .filter(|t| t.id == turn)
        .ok_or(Error::Conflict("Projection has no active turn"))?;
    let kind = if reasoning { "reasoning" } else { "markdown" };
    // Adjacent chunks share a part; a tool/reasoning boundary preserves ordering.
    let last = active
        .response_parts
        .last()
        .and_then(|p| match (reasoning, p) {
            (false, ResponsePart::Markdown(p)) => Some(p.id.clone()),
            (true, ResponsePart::Reasoning(p)) => Some(p.id.clone()),
            _ => None,
        });
    let mut actions = Vec::new();
    let part = last.unwrap_or_else(|| format!("{turn}-part-{}", active.response_parts.len()));
    if !matches!(active.response_parts.last(),Some(ResponsePart::Markdown(_)) if !reasoning)
        && !matches!(active.response_parts.last(),Some(ResponsePart::Reasoning(_)) if reasoning)
    {
        actions.push(typed(json!({"type":"chat/responsePart","turnId":turn,"part":{"kind":kind,"id":part,"content":""}}))?);
    }
    actions.push(typed(json!({"type":if reasoning{"chat/reasoning"}else{"chat/delta"},"turnId":turn,"partId":part,"content":content}))?);
    Ok(actions)
}
impl Journal<'_> {
    /// Store migration only, inside the same transaction as PROJECTION_SCHEMA.
    /// Old source events are folded into a snapshot baseline, never replayed as
    /// if the former server had already emitted AHP actions.
    pub fn backfill_projection(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Projection migration requires a transaction",
            ));
        }
        let existing: i64 =
            self.connection
                .query_row("SELECT count(*) FROM interactive_ahp_chats", [], |r| {
                    r.get(0)
                })?;
        if existing != 0 {
            return Err(Error::Conflict("Projection already initialized"));
        }
        let baseline: String =
            self.connection
                .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                    r.get(0)
                })?;
        let mut after = 0;
        loop {
            // Close each read cursor before savepoint rollback. Batches bound
            // migration memory instead of loading every transcript at once.
            let records = {
                let mut q=self.connection.prepare("SELECT x.server_seq,e.session,e.seq,e.kind,e.payload FROM interactive_stream_events x JOIN interactive_events e ON e.session=x.session AND e.seq=x.session_seq WHERE x.server_seq>?1 ORDER BY x.server_seq LIMIT 64")?;
                q.query_map([sql_integer(after)?], |r| {
                    Ok((
                        nonnegative(r, 0)?,
                        r.get::<_, String>(1)?,
                        nonnegative(r, 2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
            };
            if records.is_empty() {
                break;
            }
            for (global, id, seq, kind, raw) in records {
                let mut payload: Value = serde_json::from_str(&raw)?;
                if payload.get("_recorded_at").is_none() {
                    payload["_recorded_at"] = json!(baseline);
                    payload["_timestamp_imported"] = json!(true);
                }
                self.connection
                    .execute_batch("SAVEPOINT ahp_backfill_record")?;
                match self.project_event(&id, seq, &kind, &payload) {
                    Ok(()) => self
                        .connection
                        .execute_batch("RELEASE ahp_backfill_record")?,
                    Err(Error::Conflict(
                        "ACP content projection unavailable" | "ACP update projection unavailable",
                    ))
                    | Err(Error::Limit(_)) => {
                        self.connection.execute_batch(
                            "ROLLBACK TO ahp_backfill_record; RELEASE ahp_backfill_record",
                        )?;
                        self.quarantine_projection(&id, seq, &baseline)?;
                    }
                    Err(error) => return Err(error),
                }
                after = global;
            }
        }
        self.connection
            .execute("UPDATE interactive_ahp_clock SET replay_floor=sequence", [])?;
        Ok(())
    }
    fn quarantine_projection(&self, id: &str, seq: u64, at: &str) -> Result<()> {
        let mut state = blank(id, at)?;
        state.meta = Some(serde_json::Map::from_iter([(
            "dunlin.projectionUnavailable".into(),
            json!(true),
        )]));
        self.connection
            .execute("DELETE FROM interactive_ahp_actions WHERE session=?1", [id])?;
        self.connection.execute("INSERT INTO interactive_ahp_chats(session,source_seq,state) VALUES(?1,?2,?3) ON CONFLICT(session) DO UPDATE SET source_seq=excluded.source_seq,state=excluded.state,action_count=0,action_bytes=0",params![id,sql_integer(seq)?,serde_json::to_string(&state)?])?;
        let channels:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='interactive_ahp_sessions')",[],|r|r.get(0))?;
        if channels {
            self.project_session(id, seq, "projection_unavailable")?;
        }
        Ok(())
    }
    fn prompt_for(&self, id: &str, turn: &str) -> Result<String> {
        Ok(self.connection.query_row(
            "SELECT prompt FROM interactive_turns WHERE session=?1 AND id=?2",
            params![id, turn],
            |r| r.get(0),
        )?)
    }
    fn actions_for(
        &self,
        id: &str,
        state: &ChatState,
        kind: &str,
        p: &Value,
    ) -> Result<Vec<StateAction>> {
        let turn = p.get("turn_id").and_then(Value::as_str);
        if matches!(kind, "agent_output" | "agent_update" | "turn_phase")
            && let Some(turn) = turn
            && self.cancellation_known(id, turn)?
            && state
                .active_turn
                .as_ref()
                .is_none_or(|active| active.id != turn)
        {
            return Ok(vec![]);
        }
        match kind {
            "permission_registered" | "permission_lifecycle" => {
                Ok(serde_json::from_value(p["actions"].clone())?)
            }
            "client_control" => Ok(vec![serde_json::from_value(p["action"].clone())?]),
            "cancellation_settled" => Ok(vec![]),
            "session_created" | "runtime_acquired" | "harness_ready" => Ok(vec![]),
            "turn_queued" => {
                let t = turn.ok_or(Error::Conflict("Missing queued turn"))?;
                Ok(vec![typed(
                    json!({"type":"chat/pendingMessageSet","kind":"queued","id":self.pending_message_id(id,t)?,"message":message(self.prompt_for(id,t)?)}),
                )?])
            }
            "turn_dispatching" => {
                let t = turn.ok_or(Error::Conflict("Missing dispatched turn"))?;
                if state.active_turn.is_some() {
                    return Err(Error::Conflict("Projection already has an active turn"));
                }
                Ok(vec![typed(
                    json!({"type":"chat/turnStarted","turnId":t,"startedAt":text(p,"_recorded_at")?,"message":message(self.prompt_for(id,t)?),"queuedMessageId":self.pending_message_id(id,t)?,"_meta":{"dunlin.timestampImported":p["_timestamp_imported"]==true}}),
                )?])
            }
            "turn_phase" => {
                let phase = text(p, "phase")?;
                if phase == "running" {
                    return Ok(vec![]);
                }
                Ok(vec![terminal(
                    turn.ok_or(Error::Conflict("Missing terminal turn"))?,
                    phase,
                    p,
                )?])
            }
            "agent_output" => chunk(
                state,
                turn.ok_or(Error::Conflict("Missing output turn"))?,
                text(p, "text")?,
                false,
            ),
            "agent_update" => {
                let t = turn.ok_or(Error::Conflict("Missing update turn"))?;
                let update = &p["update"];
                match text(update, "sessionUpdate")? {
                    "agent_message_chunk" | "agent_thought_chunk" => {
                        if update["content"]["type"] != "text" {
                            return Err(Error::Conflict("ACP content projection unavailable"));
                        }
                        chunk(
                            state,
                            t,
                            text(&update["content"], "text")?,
                            update["sessionUpdate"] == "agent_thought_chunk",
                        )
                    }
                    "tool_call" | "tool_call_update" => tool_actions(state, t, update),
                    // Configuration is checked by the actor; these are not chat content.
                    "config_option_update"
                    | "current_mode_update"
                    | "available_commands_update"
                    | "session_info_update" => Ok(vec![]),
                    _ => Err(Error::Conflict("ACP update projection unavailable")),
                }
            }
            "runtime_interrupted" | "recovery_required" => {
                if let Some(t) = &state.active_turn {
                    Ok(vec![terminal(&t.id, "unknown", p)?])
                } else {
                    Ok(vec![])
                }
            }
            _ => Err(Error::Conflict("Unknown journal projection event")),
        }
    }
    pub(super) fn project_event(
        &self,
        id: &str,
        source_seq: u64,
        kind: &str,
        payload: &Value,
    ) -> Result<()> {
        let row=self.connection.query_row("SELECT source_seq,state,action_count,action_bytes FROM interactive_ahp_chats WHERE session=?1",[id],|r|Ok((nonnegative(r,0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?;
        let (mut state, count, bytes) = match row {
            Some((previous, raw, count, bytes)) => {
                if previous.checked_add(1) != Some(source_seq) {
                    return Err(Error::Conflict("Projection source gap"));
                }
                (serde_json::from_str::<ChatState>(&raw)?, count, bytes)
            }
            None => {
                if kind != "session_created" || source_seq != 1 {
                    return Err(Error::Conflict("Missing projection baseline"));
                }
                let mut state = blank(id, text(payload, "_recorded_at")?)?;
                if payload["_timestamp_imported"] == true {
                    state.meta = Some(serde_json::Map::from_iter([(
                        "dunlin.timestampImported".into(),
                        json!(true),
                    )]));
                }
                (state, 0, 0)
            }
        };
        let unavailable = state
            .meta
            .as_ref()
            .is_some_and(|m| m.get("dunlin.projectionUnavailable") == Some(&json!(true)));
        let actions = if unavailable {
            vec![]
        } else {
            self.actions_for(id, &state, kind, payload)?
        };
        let recovery = matches!(
            kind,
            "runtime_interrupted"
                | "recovery_required"
                | "cancellation_settled"
                | "permission_lifecycle"
        );
        let mut added_bytes = 0i64;
        for (ordinal, action) in actions.iter().enumerate() {
            let outcome = apply_action_to_chat(&mut state, action);
            if outcome != ReduceOutcome::Applied
                && !(kind == "client_control" && outcome == ReduceOutcome::NoOp)
            {
                return Err(Error::Conflict("AHP reducer rejected projection"));
            }
            let raw = serde_json::to_string(action)?;
            added_bytes = added_bytes.saturating_add(raw.len() as i64);
            self.connection.execute(
                "UPDATE interactive_ahp_clock SET sequence=sequence+1 WHERE singleton=1",
                [],
            )?;
            let seq: u64 = self.connection.query_row(
                "SELECT sequence FROM interactive_ahp_clock WHERE singleton=1",
                [],
                |r| nonnegative(r, 0),
            )?;
            let channels:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('interactive_ahp_actions') WHERE name='channel')",[],|r|r.get(0))?;
            if !channels {
                self.connection.execute(
                    "INSERT INTO interactive_ahp_actions VALUES(?1,?2,?3,?4,?5)",
                    params![
                        sql_integer(seq)?,
                        id,
                        sql_integer(source_seq)?,
                        ordinal as i64,
                        raw
                    ],
                )?;
            } else {
                self.connection.execute(
                "INSERT INTO interactive_ahp_actions(server_seq,session,source_seq,ordinal,action,channel) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    sql_integer(seq)?,
                    id,
                    sql_integer(source_seq)?,
                    ordinal as i64,
                    raw,format!("ahp-chat:/{id}")
                ],
            )?;
            }
        }
        let limit = if recovery {
            16 * 1024 * 1024
        } else {
            16 * 1024 * 1024 - 131072
        };
        if count + actions.len() as i64 > if recovery { 4096 } else { 4064 }
            || bytes + added_bytes > limit
        {
            return Err(Error::Limit("AHP projection journal limit reached"));
        }
        let raw = serde_json::to_string(&state)?;
        if raw.len()
            > if recovery {
                8 * 1024 * 1024
            } else {
                8 * 1024 * 1024 - 65536
            }
        {
            return Err(Error::Limit("AHP projection snapshot limit reached"));
        }
        self.connection.execute("INSERT INTO interactive_ahp_chats(session,source_seq,state,action_count,action_bytes) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session) DO UPDATE SET source_seq=excluded.source_seq,state=excluded.state,action_count=excluded.action_count,action_bytes=excluded.action_bytes",params![id,sql_integer(source_seq)?,raw,count+actions.len() as i64,bytes+added_bytes])?;
        let channels:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='interactive_ahp_sessions')",[],|r|r.get(0))?;
        if channels {
            self.project_session(id, source_seq, kind)?;
        }
        Ok(())
    }
    pub fn projected_chat(&self, context: &AccessContext, id: &str) -> Result<ChatState> {
        self.authorized(context, id, Permission::ViewProject)?;
        let raw: String = self.connection.query_row(
            "SELECT state FROM interactive_ahp_chats WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        let state: ChatState = serde_json::from_str(&raw)?;
        if state
            .meta
            .as_ref()
            .is_some_and(|m| m.get("dunlin.projectionUnavailable") == Some(&json!(true)))
        {
            return Err(Error::Conflict(
                "Legacy session requires a compatible projection adapter",
            ));
        }
        Ok(state)
    }
    /// A typed chat snapshot and shared AHP action high-water mark from one
    /// transaction. The transport must subscribe to invalidations before this cut.
    pub fn projected_snapshot(
        &self,
        context: &AccessContext,
        id: &str,
    ) -> Result<ahp_types::state::Snapshot> {
        let tx = self.connection.unchecked_transaction()?;
        let state = self.projected_chat(context, id)?;
        let seq = self.connection.query_row(
            "SELECT sequence FROM interactive_ahp_clock WHERE singleton=1",
            [],
            |r| nonnegative(r, 0),
        )?;
        let snapshot = ahp_types::state::Snapshot {
            resource: state.resource.clone(),
            state: ahp_types::state::SnapshotState::Chat(Box::new(state)),
            from_seq: sql_integer(seq)?,
        };
        tx.commit()?;
        Ok(snapshot)
    }
    /// Internal inspection for the adapter. Full bounded multi-channel reconnect
    /// and snapshot/subscription cuts are separate from this per-session page.
    pub fn projected_actions(
        &self,
        context: &AccessContext,
        id: &str,
        after: u64,
    ) -> Result<Vec<ActionEnvelope>> {
        self.projected_chat(context, id)?;
        let (floor, last) = self.connection.query_row(
            "SELECT replay_floor,sequence FROM interactive_ahp_clock WHERE singleton=1",
            [],
            |r| Ok((nonnegative(r, 0)?, nonnegative(r, 1)?)),
        )?;
        if after < floor || after > last {
            return Err(Error::Conflict("AHP action cursor requires a snapshot"));
        }
        let mut q=self.connection.prepare("SELECT server_seq,action,origin,rejection_reason FROM interactive_ahp_actions WHERE session=?1 AND channel LIKE 'ahp-chat:/%' AND server_seq>?2 ORDER BY server_seq LIMIT 128")?;
        q.query_map(params![id, sql_integer(after)?], |r| {
            Ok((
                nonnegative(r, 0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .map(|row| {
            let (seq, raw, origin, rejection_reason) = row?;
            Ok(ActionEnvelope {
                channel: format!("ahp-chat:/{id}"),
                action: serde_json::from_str(&raw)?,
                server_seq: seq,
                origin: origin.map(|raw| serde_json::from_str(&raw)).transpose()?,
                rejection_reason,
            })
        })
        .collect()
    }
}

/// Project agent-managed tool reports only. This cannot approve or execute a
/// tool. not-needed means no additional AHP client confirmation is requested
/// for an already reported operation; metadata preserves the evidence boundary.
/// It never represents a user-action/setting decision made by this projection.
fn tool_actions(state: &ChatState, turn: &str, update: &Value) -> Result<Vec<StateAction>> {
    let active = state
        .active_turn
        .as_ref()
        .filter(|t| t.id == turn)
        .ok_or(Error::Conflict("Projection has no active turn"))?;
    let id = text(update, "toolCallId")?;
    if id.starts_with(crate::permissions::MCP_TOOL_PREFIX) {
        return Err(Error::Conflict("ACP cannot report a host MCP tool"));
    }
    if id.is_empty() || id.len() > 256 {
        return Err(Error::Limit("Invalid ACP tool identity"));
    }
    let previous = active.response_parts.iter().find_map(|p| {
        if let ResponsePart::ToolCall(p) = p {
            let v = serde_json::to_value(&p.tool_call).ok()?;
            if v["toolCallId"] == id {
                return Some(v);
            }
        }
        None
    });
    if previous.as_ref().is_some_and(|t| {
        t["_meta"]["dunlin.permissionNonce"].is_string()
            && matches!(t["status"].as_str(), Some("cancelled" | "completed"))
    }) {
        return Ok(vec![]);
    }
    let host_meta = previous
        .as_ref()
        .and_then(|t| t["_meta"].as_object())
        .filter(|m| m.contains_key("dunlin.permissionNonce"))
        .cloned();
    let mut actions = Vec::new();
    let current = if let Some(previous) = previous {
        if update["sessionUpdate"] == "tool_call" {
            return Err(Error::Conflict("Duplicate ACP tool identity"));
        }
        previous
    } else {
        if update["sessionUpdate"] != "tool_call" {
            return Err(Error::Conflict("ACP tool update precedes tool call"));
        }
        let title = text(update, "title")?;
        actions.push(typed(json!({"type":"chat/toolCallStart","turnId":turn,"toolCallId":id,"toolName":update["kind"].as_str().unwrap_or("other"),"displayName":title,"_meta":{"dunlin.acpReport":update}}))?);
        json!({"status":"streaming"})
    };
    let status = update["status"]
        .as_str()
        .unwrap_or(if current["status"] == "streaming" {
            "pending"
        } else {
            "in_progress"
        });
    if !matches!(status, "pending" | "in_progress" | "completed" | "failed") {
        return Err(Error::Conflict("Unknown ACP tool state"));
    }
    if !matches!(current["status"].as_str(), Some("streaming" | "running")) {
        return Err(Error::Conflict("ACP tool update after terminal state"));
    }
    if current["status"] == "streaming" {
        if let Some(input) = update.get("rawInput") {
            actions.push(typed(json!({"type":"chat/toolCallDelta","turnId":turn,"toolCallId":id,"_meta":{"dunlin.rawInput":input,"dunlin.acpReport":update}}))?);
        }
        if status != "pending" {
            let tool_input = update
                .get("rawInput")
                .map(serde_json::to_string)
                .transpose()?
                .or(current["_meta"]
                    .get("dunlin.rawInput")
                    .map(serde_json::to_string)
                    .transpose()?);
            let mut ready = json!({"type":"chat/toolCallReady","turnId":turn,"toolCallId":id,"invocationMessage":update["title"].as_str().or_else(||current["displayName"].as_str()).unwrap_or("Tool invocation"),"confirmed":"not-needed","_meta":{"dunlin.confirmationEvidence":"agent-report-only","dunlin.acpReport":update}});
            if let Some(input) = tool_input {
                ready["toolInput"] = json!(input);
            }
            actions.push(typed(ready)?);
        }
    } else if status == "pending" {
        return Err(Error::Conflict("ACP tool execution regressed"));
    }
    let mut content = Vec::new();
    if let Some(parts) = update.get("content") {
        for part in parts
            .as_array()
            .ok_or(Error::Conflict("Invalid ACP tool content"))?
        {
            if part["type"] == "content" && part["content"]["type"] == "text" {
                content.push(json!({"type":"text","text":text(&part["content"],"text")?}));
            }
            // Non-text diff/resource data remains in structuredContent's full
            // ACP report. Do not invent a nonstandard wire content variant.
        }
    }
    if matches!(status, "completed" | "failed") {
        actions.push(typed(json!({"type":"chat/toolCallComplete","turnId":turn,"toolCallId":id,"result":{"success":status=="completed","pastTenseMessage":update["title"].as_str().unwrap_or("Tool finished"),"content":content,"structuredContent":{"dunlin.acpReport":update}},"_meta":{"dunlin.acpReport":update}}))?);
    } else if status == "in_progress" && update.get("content").is_some() {
        actions.push(typed(json!({"type":"chat/toolCallContentChanged","turnId":turn,"toolCallId":id,"content":content,"_meta":{"dunlin.acpReport":update}}))?);
    }
    if let Some(host) = host_meta {
        for action in &mut actions {
            let mut value = serde_json::to_value(&*action)?;
            let mut meta = value["_meta"].as_object().cloned().unwrap_or_default();
            for (key, value) in &host {
                if key.starts_with("dunlin.permission")
                    || key == "dunlin.humanApproved"
                    || key == "dunlin.confirmationEvidence"
                {
                    meta.insert(key.clone(), value.clone());
                }
            }
            value["_meta"] = Value::Object(meta);
            *action = serde_json::from_value(value)?;
        }
    }
    Ok(actions)
}

#[cfg(test)]
mod frontend_oracle;
#[cfg(test)]
mod tests;
