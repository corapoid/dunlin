//! Durable human input for an already approved MCP invocation. Opaque MCP
//! continuation state stays in the live client; this ledger cannot resume HTTP.
use super::*;
use ahp_types::state::ChatInputQuestion;
use ahp_types::{
    actions::ChatInputCompletedAction,
    state::{ChatInputRequest, ChatInputResponseKind},
};

pub const MCP_INPUT_SCHEMA: &str = "
CREATE TABLE interactive_mcp_inputs (
 id TEXT PRIMARY KEY, session TEXT NOT NULL REFERENCES interactive_sessions(id),
 nonce TEXT NOT NULL REFERENCES interactive_permission_challenges(nonce),
 request TEXT NOT NULL, constraints TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','decided','consumed','invalid')),
 decision TEXT, device TEXT,
 UNIQUE(session,id)
);
CREATE INDEX interactive_mcp_input_parent ON interactive_mcp_inputs(session,nonce,state);
";
/// Host-created display data and flat form schema, or null for URL input.
/// Never requestState. URL decisions carry no answers or content.
pub struct McpInputChallenge {
    pub request: ChatInputRequest,
    pub schema: Value,
}
pub struct McpInputScope<'a> {
    pub permit: &'a McpDispatchPermit,
    pub invocation: &'a McpInvocation,
}
/// Neither wire data nor a resumable credential; only the live broker owns it.
pub struct McpInput {
    id: String,
    nonce: String,
}
impl McpInput {
    pub fn id(&self) -> &str {
        &self.id
    }
}
/// Deliberately not Debug/Serialize: do not log human form content.
pub struct McpInputDecision {
    pub response: ChatInputResponseKind,
    pub content: Option<Value>,
    pub device: PermissionDecision,
}
impl Journal<'_> {
    fn mcp_inputs_present(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='interactive_mcp_inputs')",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn register_mcp_input(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        scope: McpInputScope<'_>,
        mut challenge: McpInputChallenge,
    ) -> Result<McpInput> {
        let tx = self.connection.unchecked_transaction()?;
        self.validate_mcp_dispatch(context, id, lease, scope.permit, scope.invocation)?;
        if challenge.request.answers.is_some() {
            return Err(Error::Conflict("MCP input must be fresh"));
        }
        if let Some(target) = &challenge.request.url {
            let raw = target.as_str();
            let parsed =
                url::Url::parse(raw).map_err(|_| Error::Conflict("Invalid MCP input URL"))?;
            if challenge.request.questions.is_some()
                || !challenge.schema.is_null()
                || raw.len() > 4096
                || raw
                    .bytes()
                    .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
                || parsed.scheme() != "https"
                || parsed.host().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(Error::Conflict("Invalid MCP input URL"));
            }
        } else if challenge
            .request
            .questions
            .as_ref()
            .is_none_or(|q| q.len() > 32)
        {
            return Err(Error::Conflict("MCP input must be a fresh form"));
        }
        let mut names = std::collections::BTreeSet::new();
        for q in challenge.request.questions.iter().flatten() {
            if matches!(q, ChatInputQuestion::Unknown(_)) {
                return Err(Error::Conflict("Unsupported MCP input question"));
            }
            let q = serde_json::to_value(q)?;
            let name = q["id"]
                .as_str()
                .filter(|v| sid(v, 128))
                .ok_or(Error::Conflict("Invalid MCP input question identity"))?;
            if !names.insert(name.to_owned()) {
                return Err(Error::Conflict("Duplicate MCP input question identity"));
            }
        }
        bounded(
            &serde_json::to_value(&challenge.request)?,
            48 * 1024,
            4096,
            16,
        )?;
        bounded(&challenge.schema, 16 * 1024, 1024, 16)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM interactive_mcp_inputs WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        if count >= 256 {
            return Err(Error::Limit("MCP input ledger capacity"));
        }
        let request_id = format!("dunlin:mcp-input:{}", uuid::Uuid::now_v7());
        challenge.request.id = request_id.clone();
        tx.execute("INSERT INTO interactive_mcp_inputs(id,session,nonce,request,constraints,state) VALUES(?1,?2,?3,?4,?5,'pending')",params![request_id,id,scope.permit.0.nonce,serde_json::to_string(&challenge.request)?,serde_json::to_string(&challenge.schema)?])?;
        self.event(
            id,
            "permission_lifecycle",
            json!({"actions":[{"type":"chat/inputRequested","request":challenge.request}]}),
            false,
        )?;
        tx.commit()?;
        self.changed();
        Ok(McpInput {
            id: request_id,
            nonce: scope.permit.0.nonce.clone(),
        })
    }
    pub(crate) fn input_rejection(&self, id: &str, request: &str) -> Result<Option<&'static str>> {
        if !self.mcp_inputs_present()? {
            return Ok(Some("MCP input is unavailable"));
        }
        let pending:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM interactive_mcp_inputs WHERE session=?1 AND id=?2 AND state='pending')",params![id,request],|r|r.get(0))?;
        Ok((!pending).then_some("MCP input is no longer pending"))
    }
    pub(crate) fn mcp_input_answers_valid(
        &self,
        id: &str,
        action: &ChatInputCompletedAction,
        validate: &dyn Fn(&Value, &Value) -> Result<()>,
    ) -> Result<bool> {
        let (raw,schema):(String,String)=self.connection.query_row("SELECT request,constraints FROM interactive_mcp_inputs WHERE session=?1 AND id=?2 AND state='pending'",params![id,action.request_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let request = serde_json::from_str(&raw)?;
        let schema = serde_json::from_str(&schema)?;
        let Ok(content) = input_content(&request, action) else {
            return Ok(false);
        };
        Ok(content
            .as_ref()
            .is_none_or(|content| validate(&schema, content).is_ok()))
    }
    pub(crate) fn apply_mcp_input_decision(
        &self,
        context: &AccessContext,
        id: &str,
        device: &str,
        action: &ChatInputCompletedAction,
        validate: &dyn Fn(&Value, &Value) -> Result<()>,
    ) -> Result<ahp_types::actions::StateAction> {
        let (nonce,raw,schema):(String,String,String)=self.connection.query_row("SELECT nonce,request,constraints FROM interactive_mcp_inputs WHERE session=?1 AND id=?2 AND state='pending'",params![id,action.request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let ticket = self
            .ticket(id, &nonce)?
            .ok_or(Error::Conflict("MCP input parent missing"))?;
        let session = self.session(id)?;
        let lease = Lease {
            epoch: session.epoch,
            instance: session
                .runtime
                .ok_or(Error::Conflict("MCP input runtime missing"))?,
        };
        if ticket.state != "consumed"
            || ticket.request["origin"] != MCP_ORIGIN
            || !self.validate_ticket(context, id, &lease, &nonce, &ticket)?
        {
            return Err(Error::Conflict("MCP input authority changed"));
        }
        let content = input_content(&serde_json::from_str(&raw)?, action)?;
        if let Some(content) = &content {
            validate(&serde_json::from_str(&schema)?, content)?;
        }
        let changed=self.connection.execute("UPDATE interactive_mcp_inputs SET state='decided',decision=?3,device=?4 WHERE session=?1 AND id=?2 AND state='pending'",params![id,action.request_id,serde_json::to_string(action)?,device])?;
        if changed != 1 {
            return Err(Error::Conflict("MCP input already decided"));
        }
        Ok(ahp_types::actions::StateAction::ChatInputCompleted(
            action.clone(),
        ))
    }
    /// Poll and atomically consume once, rechecking the original permit. This
    /// never dispatches a continuation or recreates the client's Pending handle.
    pub fn take_mcp_input(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        scope: McpInputScope<'_>,
        input: &McpInput,
    ) -> Result<Option<McpInputDecision>> {
        let tx = self.connection.unchecked_transaction()?;
        self.validate_mcp_dispatch(context, id, lease, scope.permit, scope.invocation)?;
        if input.nonce != scope.permit.0.nonce {
            return Err(Error::Conflict("MCP input parent changed"));
        }
        let (state,raw,decision,device):(String,String,Option<String>,Option<String>)=tx.query_row("SELECT state,request,decision,device FROM interactive_mcp_inputs WHERE session=?1 AND id=?2 AND nonce=?3",params![id,input.id,input.nonce],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if state == "pending" {
            return Ok(None);
        }
        if state != "decided" {
            return Err(Error::Conflict("MCP input cannot be consumed"));
        }
        let action: ChatInputCompletedAction =
            serde_json::from_str(&decision.ok_or(Error::Conflict("MCP input decision missing"))?)?;
        let content = input_content(&serde_json::from_str(&raw)?, &action)?;
        tx.execute("UPDATE interactive_mcp_inputs SET state='consumed' WHERE session=?1 AND id=?2 AND state='decided'",params![id,input.id])?;
        tx.commit()?;
        Ok(Some(McpInputDecision {
            response: action.response,
            content,
            device: PermissionDecision {
                nonce: input.id.clone(),
                device_id: device.ok_or(Error::Conflict("MCP input device missing"))?,
                approved: action.response == ChatInputResponseKind::Accept,
                option_id: None,
            },
        }))
    }
    pub(in crate::permissions) fn invalidate_mcp_inputs(
        &self,
        id: &str,
        nonce: &str,
    ) -> Result<Vec<Value>> {
        if !self.mcp_inputs_present()? {
            return Ok(vec![]);
        }
        let mut q=self.connection.prepare("SELECT id FROM interactive_mcp_inputs WHERE session=?1 AND nonce=?2 AND state='pending'")?;
        let ids = q
            .query_map(params![id, nonce], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.connection.execute("UPDATE interactive_mcp_inputs SET state='invalid',decision=NULL WHERE session=?1 AND nonce=?2 AND state IN ('pending','decided')",params![id,nonce])?;
        Ok(ids
            .into_iter()
            .map(|id| json!({"type":"chat/inputCompleted","requestId":id,"response":"cancel"}))
            .collect())
    }
}
/// Decode only submitted, typed answers. The adapter must additionally validate
/// the resulting content with the exact compiled MCP FormSchema before commit.
fn input_content(
    request: &ChatInputRequest,
    action: &ChatInputCompletedAction,
) -> Result<Option<Value>> {
    let fail = || Error::Conflict("Invalid MCP input answers");
    let answers = action.answers.as_ref();
    if request.url.is_some() {
        if answers.is_some() {
            return Err(fail());
        }
        return Ok(None);
    }
    if action.response != ChatInputResponseKind::Accept {
        if answers.is_some_and(|a| !a.is_empty()) {
            return Err(fail());
        }
        return Ok(None);
    }
    let questions = request.questions.as_ref().ok_or_else(fail)?;
    let mut content = serde_json::Map::new();
    for (key, answer) in answers.into_iter().flatten() {
        let question = questions
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .find(|q| q["id"] == *key)
            .ok_or_else(fail)?;
        let a = serde_json::to_value(answer)?;
        if a["state"] != "submitted" || a["value"].get("freeformValues").is_some() {
            return Err(fail());
        }
        let kind = match question["kind"].as_str() {
            Some("text") => "text",
            Some("number" | "integer") => "number",
            Some("boolean") => "boolean",
            Some("single-select") => "selected",
            Some("multi-select") => "selected-many",
            _ => return Err(fail()),
        };
        if a["value"]["kind"] != kind {
            return Err(fail());
        }
        content.insert(key.clone(), a["value"]["value"].clone());
    }
    let content = Value::Object(content);
    bounded(&content, 64 * 1024, 1024, 4)?;
    Ok(Some(content))
}

#[cfg(test)]
mod tests;
