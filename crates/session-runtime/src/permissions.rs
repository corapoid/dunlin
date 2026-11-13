//! One-use, host-owned ACP permission challenges. This is not workflow approval.
use super::*;
use ahp_types::{actions::StateAction, commands::DispatchActionParams, state::ChatState};
use serde_json::{Value, json};
use std::collections::BTreeSet;
mod mcp;
pub use mcp::{
    MCP_INPUT_SCHEMA, McpApproval, McpCompletion, McpDispatchPermit, McpInput, McpInputChallenge,
    McpInputDecision, McpInputScope, McpInvocation, McpPermissionChallenge,
};
pub(crate) const MCP_TOOL_PREFIX: &str = "dunlin:mcp:";
const MCP_ORIGIN: &str = "dunlin.mcp.v1";

pub const PERMISSION_SCHEMA:&str="
CREATE TABLE interactive_permission_challenges (
 nonce TEXT PRIMARY KEY, session TEXT NOT NULL REFERENCES interactive_sessions(id),
 turn_id TEXT NOT NULL REFERENCES interactive_turns(id), tool_id TEXT NOT NULL,
 epoch INTEGER NOT NULL, runtime TEXT NOT NULL, authority TEXT NOT NULL,
 request_digest TEXT NOT NULL, request TEXT NOT NULL, tool TEXT NOT NULL, options TEXT NOT NULL,
 starter_device TEXT NOT NULL, binding_revision INTEGER NOT NULL, harness_digest TEXT NOT NULL,
 expires INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','decided','consumed','delivered','invalid','unknown')),
 decision_device TEXT, decision_client TEXT, decision_seq INTEGER, decision TEXT,
 reason TEXT
);
CREATE INDEX interactive_permission_scope ON interactive_permission_challenges(session,turn_id,tool_id,state);
";
#[derive(Clone, Debug)]
pub struct PermissionChallenge {
    pub request_id: Value,
    pub tool_call: Value,
    pub options: Value,
    pub ttl_seconds: u64,
    pub starter_device: String,
    pub binding_revision: u64,
    pub harness_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionDecision {
    pub nonce: String,
    pub device_id: String,
    pub approved: bool,
    pub option_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionStatus {
    Pending,
    Decided(PermissionDecision),
    Invalid,
}
struct Ticket {
    turn: String,
    tool: String,
    epoch: u64,
    runtime: String,
    authority: AuthorityStamp,
    expires: i64,
    state: String,
    options: Value,
    decision: Option<PermissionDecision>,
    request: Value,
}
fn sid(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
impl Journal<'_> {
    fn permission_table(&self) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='interactive_permission_challenges')",[],|r|r.get(0))?)
    }
    fn ticket(&self, id: &str, nonce: &str) -> Result<Option<Ticket>> {
        let row=self.connection.query_row("SELECT turn_id,tool_id,epoch,runtime,authority,expires,state,options,decision,request FROM interactive_permission_challenges WHERE session=?1 AND nonce=?2",params![id,nonce],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,nonnegative(r,2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,String>(9)?))).optional()?;
        row.map(
            |(
                turn,
                tool,
                epoch,
                runtime,
                authority,
                expires,
                state,
                options,
                decision,
                request,
            )| {
                Ok(Ticket {
                    turn,
                    tool,
                    epoch,
                    runtime,
                    authority: serde_json::from_str(&authority)?,
                    expires,
                    state,
                    options: serde_json::from_str(&options)?,
                    decision: decision.map(|s| serde_json::from_str(&s)).transpose()?,
                    request: serde_json::from_str(&request)?,
                })
            },
        )
        .transpose()
    }
    fn permission_chat(&self, id: &str) -> Result<ChatState> {
        let raw: String = self.connection.query_row(
            "SELECT state FROM interactive_ahp_chats WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    }
    fn permission_tool(&self, id: &str, turn: &str, tool: &str) -> Result<Option<Value>> {
        let chat = self.permission_chat(id)?;
        Ok(chat.active_turn.filter(|t| t.id == turn).and_then(|t| {
            t.response_parts.into_iter().find_map(|p| {
                let v = serde_json::to_value(p).ok()?;
                if v["kind"] == "toolCall" && v["toolCall"]["toolCallId"] == tool {
                    Some(v["toolCall"].clone())
                } else {
                    None
                }
            })
        }))
    }
    fn validate_ticket(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
        t: &Ticket,
    ) -> Result<bool> {
        self.leased(context, id, lease)?;
        context.revalidate(&t.authority, Permission::CreateSession)?;
        let now: i64 = self
            .connection
            .query_row("SELECT unixepoch()", [], |r| r.get(0))?;
        let running:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='running')",params![id,t.turn],|r|r.get(0))?;
        if t.epoch != lease.epoch
            || t.runtime != lease.instance
            || t.expires <= now
            || !running
            || self.cancellation_known(id, &t.turn)?
        {
            return Ok(false);
        }
        Ok(self
            .permission_tool(id, &t.turn, &t.tool)?
            .is_some_and(|tool| tool["_meta"]["dunlin.permissionNonce"] == nonce))
    }
    pub fn register_permission(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        challenge: PermissionChallenge,
    ) -> Result<String> {
        self.register_permission_for(context, id, lease, turn, challenge, false)
    }
    fn register_permission_for(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        challenge: PermissionChallenge,
        mcp: bool,
    ) -> Result<String> {
        let tx = self.connection.unchecked_transaction()?;
        let session = self.leased(context, id, lease)?;
        let request_ok = if mcp {
            challenge.request_id["origin"] == MCP_ORIGIN
                && challenge.request_id["request_id"]
                    .as_str()
                    .is_some_and(|s| sid(s, 128))
        } else {
            challenge.request_id.as_str().is_some_and(|s| sid(s, 128))
                || challenge.request_id.as_i64().is_some()
        };
        let size=serde_json::to_vec(&json!({"request":challenge.request_id,"tool":challenge.tool_call,"options":challenge.options}))?.len();
        if !request_ok
            || size > 32768
            || !(1..=300).contains(&challenge.ttl_seconds)
            || !sid(&challenge.starter_device, 128)
            || !sid(&challenge.harness_digest, 128)
            || challenge.binding_revision > i64::MAX as u64
        {
            return Err(Error::Limit("Invalid permission challenge"));
        }
        let tool = challenge.tool_call["toolCallId"]
            .as_str()
            .filter(|s| sid(s, 256))
            .ok_or(Error::Conflict("Permission tool identity missing"))?;
        let title = challenge.tool_call["title"]
            .as_str()
            .filter(|s| sid(s, 256))
            .ok_or(Error::Conflict("Permission tool title missing"))?;
        if tool.starts_with(MCP_TOOL_PREFIX) != mcp {
            return Err(Error::Conflict(
                "Permission origin does not match tool identity",
            ));
        }
        if mcp {
            let duplicate: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM interactive_permission_challenges WHERE session=?1 AND json_extract(request,'$.origin')=?2 AND json_extract(request,'$.request_id')=?3)",params![id,MCP_ORIGIN,challenge.request_id["request_id"].as_str()],|r|r.get(0))?;
            if duplicate {
                return Err(Error::Conflict("MCP request identity was already used"));
            }
        }
        let running:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM interactive_turns WHERE session=?1 AND id=?2 AND phase='running')",params![id,turn],|r|r.get(0))?;
        let chat = self.permission_chat(id)?;
        if !running
            || chat.active_turn.as_ref().is_none_or(|a| a.id != turn)
            || self.cancellation_known(id, turn)?
        {
            return Err(Error::Conflict("Permission has no running turn"));
        }
        if challenge
            .tool_call
            .get("status")
            .is_some_and(|v| !matches!(v.as_str(), Some("pending" | "in_progress")))
        {
            return Err(Error::Conflict(
                "Permission tool report is already terminal",
            ));
        }
        let existing = self.permission_tool(id, turn, tool)?;
        if existing.as_ref().is_some_and(|t| {
            !matches!(
                t["status"].as_str(),
                Some("streaming" | "running" | "pending-confirmation")
            )
        }) {
            return Err(Error::Conflict("Permission tool is terminal"));
        }
        let blocked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM interactive_permission_challenges WHERE session=?1 AND turn_id=?2 AND tool_id=?3 AND state='consumed')",params![id,turn,tool],|r|r.get(0))?;
        if blocked {
            return Err(Error::Conflict("Permission delivery outcome is pending"));
        }
        let (all,own,pending):(i64,i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(session=?1),0),coalesce(sum(session=?1 AND state IN ('pending','decided','consumed')),0) FROM interactive_permission_challenges",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if all >= 8192 || own >= 256 || pending >= 8 {
            return Err(Error::Limit("Permission challenge capacity"));
        }
        let nonce = uuid::Uuid::now_v7().to_string();
        let source = challenge
            .options
            .as_array()
            .filter(|a| a.len() <= 32)
            .ok_or(Error::Limit("Invalid permission options"))?;
        let mut seen = BTreeSet::new();
        let mut options = Vec::new();
        for (index, option) in source.iter().enumerate() {
            let raw = option["optionId"]
                .as_str()
                .filter(|s| sid(s, 128))
                .ok_or(Error::Conflict("Invalid permission option identity"))?;
            if !seen.insert(raw) {
                return Err(Error::Conflict("Duplicate permission option"));
            }
            let kind = option["kind"]
                .as_str()
                .ok_or(Error::Conflict("Invalid permission option kind"))?;
            if matches!(kind, "allow_always" | "reject_always") {
                continue;
            }
            if !matches!(kind, "allow_once" | "reject_once") {
                return Err(Error::Conflict("Unsupported permission option"));
            }
            let label = option["name"]
                .as_str()
                .filter(|s| sid(s, 256))
                .ok_or(Error::Conflict("Invalid permission option label"))?;
            options.push(json!({"id":format!("permission:{nonce}:{index}"),"label":label,"kind":if kind=="allow_once"{"approve"}else{"deny"},"raw":raw}));
        }
        if !options.iter().any(|o| o["kind"] == "deny") {
            options.push(json!({"id":format!("permission:{nonce}:deny"),"label":"Deny","kind":"deny","raw":null}));
        }
        let canonical = serde_json::to_vec(
            &json!({"request":challenge.request_id,"tool":challenge.tool_call,"options":challenge.options}),
        )?;
        let digest = hex::encode(Sha256::digest(canonical));
        tx.execute("UPDATE interactive_permission_challenges SET state='invalid',reason='superseded' WHERE session=?1 AND turn_id=?2 AND tool_id=?3 AND state IN ('pending','decided')",params![id,turn,tool])?;
        tx.execute("INSERT INTO interactive_permission_challenges(nonce,session,turn_id,tool_id,epoch,runtime,authority,request_digest,request,tool,options,starter_device,binding_revision,harness_digest,expires,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,unixepoch()+?15,'pending')",params![nonce,id,turn,tool,sql_integer(lease.epoch)?,lease.instance,serde_json::to_string(session.authority.as_ref().ok_or(PolicyError::StaleAuthority)?)?,digest,serde_json::to_string(&challenge.request_id)?,serde_json::to_string(&challenge.tool_call)?,serde_json::to_string(&options)?,challenge.starter_device,sql_integer(challenge.binding_revision)?,challenge.harness_digest,sql_integer(challenge.ttl_seconds)?])?;
        let mut actions = Vec::new();
        if existing.is_none() {
            actions.push(json!({"type":"chat/toolCallStart","turnId":turn,"toolCallId":tool,"toolName":challenge.tool_call["kind"].as_str().unwrap_or("other"),"displayName":title}));
        }
        let offered: Vec<_> = options
            .iter()
            .map(|o| json!({"id":o["id"],"label":o["label"],"kind":o["kind"]}))
            .collect();
        let mut ready = json!({"type":"chat/toolCallReady","turnId":turn,"toolCallId":tool,"invocationMessage":title,"options":offered,"_meta":{"dunlin.permissionNonce":nonce,"dunlin.permissionDigest":digest,"dunlin.confirmationEvidence":"host-permission-challenge"}});
        if let Some(input) = challenge.tool_call.get("rawInput") {
            ready["toolInput"] = json!(serde_json::to_string(input)?);
        }
        actions.push(ready);
        self.event(
            id,
            "permission_registered",
            json!({"actions":actions,"nonce":nonce}),
            false,
        )?;
        tx.commit()?;
        self.changed();
        Ok(nonce)
    }
    pub(super) fn permission_rejection(
        &self,
        id: &str,
        action: &StateAction,
    ) -> Result<Option<&'static str>> {
        let StateAction::ChatToolCallConfirmed(a) = action else {
            return Ok(Some("Unsupported permission action"));
        };
        let value = serde_json::to_value(action)?;
        if value
            != json!({"type":"chat/toolCallConfirmed","turnId":a.turn_id,"toolCallId":a.tool_call_id,"approved":a.approved,"selectedOptionId":a.selected_option_id})
        {
            return Ok(Some(
                "Only exact one-use permission decisions are supported",
            ));
        }
        let Some(selected) = a.selected_option_id.as_deref() else {
            return Ok(Some("Permission option is required"));
        };
        let Some(nonce) = selected
            .strip_prefix("permission:")
            .and_then(|s| s.split_once(':').map(|(n, _)| n))
        else {
            return Ok(Some("Invalid permission nonce"));
        };
        let Some(t) = self.ticket(id, nonce)? else {
            return Ok(Some("Permission is unavailable"));
        };
        let now: i64 = self
            .connection
            .query_row("SELECT unixepoch()", [], |r| r.get(0))?;
        let valid = t.state == "pending"
            && t.expires > now
            && t.turn == a.turn_id
            && t.tool == a.tool_call_id
            && !self.cancellation_known(id, &t.turn)?
            && self
                .permission_tool(id, &t.turn, &t.tool)?
                .is_some_and(|tool| {
                    tool["status"] == "pending-confirmation"
                        && tool["_meta"]["dunlin.permissionNonce"] == nonce
                })
            && t.options.as_array().is_some_and(|options| {
                options
                    .iter()
                    .any(|o| o["id"] == selected && (o["kind"] == "approve") == a.approved)
            });
        Ok(if valid {
            None
        } else {
            Some("Permission is no longer pending or option does not match")
        })
    }
    pub(super) fn apply_permission_decision(
        &self,
        context: &AccessContext,
        id: &str,
        device: &str,
        client: &str,
        p: &DispatchActionParams,
    ) -> Result<StateAction> {
        let StateAction::ChatToolCallConfirmed(a) = &p.action else {
            return Err(Error::Conflict("Invalid permission decision"));
        };
        let selected = a
            .selected_option_id
            .as_deref()
            .ok_or(Error::Conflict("Missing permission option"))?;
        let nonce = selected
            .strip_prefix("permission:")
            .and_then(|s| s.split_once(':').map(|(n, _)| n))
            .ok_or(Error::Conflict("Invalid permission nonce"))?;
        let t = self
            .ticket(id, nonce)?
            .ok_or(Error::Conflict("Missing permission"))?;
        context.revalidate(&t.authority, Permission::CreateSession)?;
        let session = self.session(id)?;
        let lease = Lease {
            epoch: session.epoch,
            instance: session
                .runtime
                .clone()
                .ok_or(Error::Conflict("Missing runtime"))?,
        };
        if !self.validate_ticket(context, id, &lease, nonce, &t)? {
            return Err(Error::Conflict("Stale permission authority"));
        }
        let option = t
            .options
            .as_array()
            .and_then(|options| options.iter().find(|o| o["id"] == selected))
            .ok_or(Error::Conflict("Missing permission option"))?;
        let decision = PermissionDecision {
            nonce: nonce.into(),
            device_id: device.into(),
            approved: a.approved,
            option_id: option["raw"].as_str().map(str::to_owned),
        };
        let n=self.connection.execute("UPDATE interactive_permission_challenges SET state='decided',decision_device=?3,decision_client=?4,decision_seq=?5,decision=?6 WHERE session=?1 AND nonce=?2 AND state='pending'",params![id,nonce,device,client,p.client_seq,serde_json::to_string(&decision)?])?;
        if n != 1 {
            return Err(Error::Conflict("Permission already decided"));
        }
        let mut canonical = json!({"type":"chat/toolCallConfirmed","turnId":a.turn_id,"toolCallId":a.tool_call_id,"approved":a.approved,"selectedOptionId":selected,"_meta":{"dunlin.permissionNonce":nonce,"dunlin.confirmationEvidence":"human-one-use","dunlin.humanApproved":a.approved}});
        if a.approved {
            canonical["confirmed"] = json!("user-action");
        } else {
            canonical["reason"] = json!("denied");
        }
        Ok(serde_json::from_value(canonical)?)
    }
    pub fn permission_status(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
    ) -> Result<PermissionStatus> {
        let Some(t) = self.ticket(id, nonce)? else {
            self.leased(context, id, lease)?;
            return Ok(PermissionStatus::Invalid);
        };
        if !self.validate_ticket(context, id, lease, nonce, &t)? {
            return Ok(PermissionStatus::Invalid);
        }
        Ok(match t.state.as_str() {
            "pending" => PermissionStatus::Pending,
            "decided" => t
                .decision
                .map(PermissionStatus::Decided)
                .unwrap_or(PermissionStatus::Invalid),
            _ => PermissionStatus::Invalid,
        })
    }
    pub fn consume_permission(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
    ) -> Result<PermissionDecision> {
        self.consume_permission_for(context, id, lease, nonce, false)
    }
    fn consume_permission_for(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
        mcp: bool,
    ) -> Result<PermissionDecision> {
        let tx = self.connection.unchecked_transaction()?;
        let t = self
            .ticket(id, nonce)?
            .ok_or(Error::Conflict("Missing permission"))?;
        if (t.request["origin"] == MCP_ORIGIN) != mcp
            || t.state != "decided"
            || !self.validate_ticket(context, id, lease, nonce, &t)?
        {
            return Err(Error::Conflict("Permission cannot be consumed"));
        }
        let decision = t
            .decision
            .ok_or(Error::Conflict("Permission lacks decision"))?;
        self.connection.execute("UPDATE interactive_permission_challenges SET state='consumed' WHERE session=?1 AND nonce=?2 AND state='decided'",params![id,nonce])?;
        tx.commit()?;
        Ok(decision)
    }
    pub fn validate_permission_delivery(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
    ) -> Result<()> {
        self.validate_permission_delivery_for(context, id, lease, nonce, false)
    }
    fn validate_permission_delivery_for(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
        mcp: bool,
    ) -> Result<()> {
        let t = self
            .ticket(id, nonce)?
            .ok_or(Error::Conflict("Missing permission"))?;
        if (t.request["origin"] == MCP_ORIGIN) != mcp
            || t.state != "consumed"
            || !self.validate_ticket(context, id, lease, nonce, &t)?
        {
            return Err(Error::Conflict("Permission delivery is no longer valid"));
        }
        Ok(())
    }
    pub fn permission_delivered(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.validate_permission_delivery(context, id, lease, nonce)?;
        self.connection.execute("UPDATE interactive_permission_challenges SET state='delivered' WHERE session=?1 AND nonce=?2 AND state='consumed'",params![id,nonce])?;
        self.event(
            id,
            "permission_lifecycle",
            json!({"actions":[],"nonce":nonce,"state":"delivered"}),
            true,
        )?;
        tx.commit()?;
        self.changed();
        Ok(())
    }
    pub fn invalidate_permission(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        nonce: &str,
        reason: &str,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.leased(context, id, lease)?;
        self.invalidate_permissions_inner(id, Some(nonce), None, reason)?;
        tx.commit()?;
        self.changed();
        Ok(())
    }
    pub(super) fn invalidate_permissions_inner(
        &self,
        id: &str,
        nonce: Option<&str>,
        turn: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        if !self.permission_table()? {
            return Ok(());
        }
        if reason.len() > 256 {
            return Err(Error::Limit("Permission invalidation reason limit"));
        }
        let mut q=self.connection.prepare("SELECT nonce,turn_id,tool_id,state FROM interactive_permission_challenges WHERE session=?1 AND (?2 IS NULL OR nonce=?2) AND (?3 IS NULL OR turn_id=?3) AND state IN ('pending','decided','consumed')")?;
        let rows = q
            .query_map(params![id, nonce, turn], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        if rows.is_empty() {
            return Ok(());
        }
        let mut actions = Vec::new();
        for (nonce, turn, tool, state) in rows {
            actions.extend(self.invalidate_mcp_inputs(id, &nonce)?);
            let unknown = state == "consumed";
            self.connection.execute("UPDATE interactive_permission_challenges SET state=?3,reason=?4 WHERE session=?1 AND nonce=?2",params![id,nonce,if unknown{"unknown"}else{"invalid"},reason])?;
            if let Some(current) = self.permission_tool(id, &turn, &tool)? {
                if current["_meta"]["dunlin.permissionNonce"] != nonce {
                    continue;
                }
                let meta = json!({"dunlin.permissionNonce":nonce,"dunlin.permissionRevoked":true,"dunlin.permissionUnconfirmed":unknown,"dunlin.permissionReason":reason});
                match current["status"].as_str(){Some("pending-confirmation")=>actions.push(json!({"type":"chat/toolCallConfirmed","turnId":turn,"toolCallId":tool,"approved":false,"reason":"skipped","_meta":meta})),Some("running")=>actions.push(json!({"type":"chat/toolCallComplete","turnId":turn,"toolCallId":tool,"result":{"success":false,"pastTenseMessage":"Permission delivery stopped","error":{"message":"Permission delivery outcome is unavailable"}},"_meta":meta})),_=>{}}
            }
        }
        // Even an empty action batch updates the session's durable permission flags.
        self.event(
            id,
            "permission_lifecycle",
            json!({"actions":actions,"reason":reason}),
            true,
        )?;
        Ok(())
    }
    pub(super) fn permission_counts(&self, id: &str) -> Result<(i64, i64)> {
        if !self.permission_table()? {
            return Ok((0, 0));
        }
        Ok(self.connection.query_row("SELECT coalesce(sum(state IN ('pending','decided','consumed')),0),coalesce(sum(state='unknown'),0) FROM interactive_permission_challenges WHERE session=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?)
    }
}
#[cfg(test)]
mod tests;
