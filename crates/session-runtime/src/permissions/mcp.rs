//! Host-only MCP approvals. These bind user intent, not OAuth, project or tool
//! authority. The broker must independently revalidate those grants, the schema
//! and the approving device before dispatch and after I/O. No network runs here.
use super::*;
mod input;
pub use input::{MCP_INPUT_SCHEMA, McpInput, McpInputChallenge, McpInputDecision, McpInputScope};

/// Resolved by the broker, never deserialized as an authorization grant.
pub struct McpInvocation {
    pub provider_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub tool_name: String,
    /// Digest of the complete validated catalog definition, including output.
    pub definition_digest: String,
    pub arguments: Value,
}
pub struct McpPermissionChallenge {
    /// Stable within the session, including across reconnect/restart. Reuse is
    /// rejected even after denial, completion or an uncertain outcome.
    pub request_id: String,
    pub invocation: McpInvocation,
    pub ttl_seconds: u64,
    pub starter_device: String,
    pub binding_revision: u64,
    pub harness_digest: String,
}
/// Non-cloneable/non-serializable handle for one immutable challenge.
pub struct McpApproval {
    nonce: String,
    tool: String,
    request: Value,
}
/// Only an approved, atomically consumed challenge can produce this handle.
/// It is not a credential or permission to bypass other broker checks.
pub struct McpDispatchPermit(McpApproval);
pub enum McpCompletion {
    Complete(Value),
    Failed,
    /// Includes lost responses and crashes after dispatch reservation. Never
    /// retry an uncertain operation automatically.
    Unknown,
}
fn bounded(value: &Value, bytes: usize, nodes: usize, depth: usize) -> Result<()> {
    let mut pending = vec![(value, 0)];
    let mut count = 0;
    while let Some((value, level)) = pending.pop() {
        count += 1;
        if count > nodes || level > depth {
            return Err(Error::Limit("MCP approval data limit"));
        }
        match value {
            Value::Array(values) => {
                if count + pending.len() + values.len() > nodes {
                    return Err(Error::Limit("MCP approval data limit"));
                }
                pending.extend(values.iter().map(|v| (v, level + 1)));
            }
            Value::Object(values) => {
                if count + pending.len() + values.len() > nodes {
                    return Err(Error::Limit("MCP approval data limit"));
                }
                pending.extend(values.values().map(|v| (v, level + 1)));
            }
            _ => {}
        }
    }
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(data.len())
                .ok_or(std::io::ErrorKind::InvalidData)?;
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Budget(bytes), value)
        .map_err(|_| Error::Limit("MCP approval data limit"))?;
    Ok(())
}
impl McpInvocation {
    /// Validate identity and bounded arguments before any external operation.
    pub fn validate(&self) -> Result<()> {
        self.value().map(|_| ())
    }

    fn value(&self) -> Result<Value> {
        if self.provider_id.is_empty()
            || self.provider_id.len() > 64
            || !self
                .provider_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !sid(&self.connection_id, 128)
            || !self
                .connection_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
            || self.connection_revision == 0
            || self.connection_revision > 9_007_199_254_740_991
            || !sid(&self.tool_name, 256)
            || self.definition_digest.len() != 64
            || !self
                .definition_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !self.arguments.is_object()
        {
            return Err(Error::Limit("Invalid MCP invocation identity"));
        }
        bounded(&self.arguments, 8192, 1024, 32)?;
        Ok(
            json!({"provider_id":self.provider_id,"connection_id":self.connection_id,
            "connection_revision":self.connection_revision,"tool_name":self.tool_name,
            "definition_digest":self.definition_digest,"arguments":self.arguments}),
        )
    }
}
impl Journal<'_> {
    pub fn register_mcp_permission(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        turn: &str,
        challenge: McpPermissionChallenge,
    ) -> Result<McpApproval> {
        if !sid(&challenge.request_id, 128) {
            return Err(Error::Limit("Invalid MCP request identity"));
        }
        let invocation = challenge.invocation.value()?;
        let tool = format!("{MCP_TOOL_PREFIX}{}", uuid::Uuid::now_v7());
        let request =
            json!({"origin":MCP_ORIGIN,"request_id":challenge.request_id,"invocation":invocation});
        let nonce=self.register_permission_for(context,id,lease,turn,PermissionChallenge {
            request_id:request.clone(),
            tool_call:json!({"toolCallId":tool,"title":format!("MCP {} / {}",challenge.invocation.provider_id,challenge.invocation.tool_name),"kind":"mcp","rawInput":invocation}),
            options:json!([{"optionId":"mcp.allow.once","kind":"allow_once","name":"Approve this MCP call once"},{"optionId":"mcp.deny.once","kind":"reject_once","name":"Deny"}]),
            ttl_seconds:challenge.ttl_seconds,starter_device:challenge.starter_device,
            binding_revision:challenge.binding_revision,harness_digest:challenge.harness_digest,
        },true)?;
        Ok(McpApproval {
            nonce,
            tool,
            request,
        })
    }
    fn pin_mcp(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        approval: &McpApproval,
        invocation: &McpInvocation,
    ) -> Result<()> {
        self.leased(context, id, lease)?;
        let ticket = self
            .ticket(id, &approval.nonce)?
            .ok_or(Error::Conflict("MCP approval missing"))?;
        if ticket.tool != approval.tool
            || ticket.request != approval.request
            || approval.request["origin"] != MCP_ORIGIN
            || approval.request["invocation"] != invocation.value()?
        {
            return Err(Error::Conflict("MCP approval invocation changed"));
        }
        Ok(())
    }
    pub fn mcp_permission_status(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        approval: &McpApproval,
        invocation: &McpInvocation,
    ) -> Result<PermissionStatus> {
        self.pin_mcp(context, id, lease, approval, invocation)?;
        self.permission_status(context, id, lease, &approval.nonce)
    }
    /// None is a denied decision, durably invalidated without a dispatch permit.
    pub fn consume_mcp_permission(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        approval: McpApproval,
        invocation: &McpInvocation,
    ) -> Result<Option<McpDispatchPermit>> {
        self.pin_mcp(context, id, lease, &approval, invocation)?;
        let status = self.permission_status(context, id, lease, &approval.nonce)?;
        if let PermissionStatus::Decided(ref decision) = status
            && !decision.approved
        {
            self.invalidate_permission(context, id, lease, &approval.nonce, "mcp_denied")?;
            return Ok(None);
        }
        let decision = self.consume_permission_for(context, id, lease, &approval.nonce, true)?;
        if !decision.approved || decision.option_id.as_deref() != Some("mcp.allow.once") {
            return Err(Error::Conflict("MCP decision cannot authorize dispatch"));
        }
        // Keep device evidence separate from the immutable request pin.
        Ok(Some(McpDispatchPermit(approval)))
    }
    pub fn validate_mcp_dispatch(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        permit: &McpDispatchPermit,
        invocation: &McpInvocation,
    ) -> Result<PermissionDecision> {
        self.pin_mcp(context, id, lease, &permit.0, invocation)?;
        self.validate_permission_delivery_for(context, id, lease, &permit.0.nonce, true)?;
        self.ticket(id, &permit.0.nonce)?
            .and_then(|t| t.decision)
            .filter(|d| d.approved && d.option_id.as_deref() == Some("mcp.allow.once"))
            .ok_or(Error::Conflict("MCP approval decision missing"))
    }
    /// Commit only a definite response as completed. Unknown outcomes retain a
    /// durable uncertainty marker; the request identity cannot be registered again.
    pub fn finish_mcp_dispatch(
        &self,
        context: &AccessContext,
        id: &str,
        lease: &Lease,
        permit: McpDispatchPermit,
        invocation: &McpInvocation,
        outcome: McpCompletion,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.validate_mcp_dispatch(context, id, lease, &permit, invocation)?;
        if matches!(outcome, McpCompletion::Unknown) {
            self.invalidate_permissions_inner(
                id,
                Some(&permit.0.nonce),
                None,
                "mcp_outcome_unknown",
            )?;
        } else {
            let (success, message, result) = match outcome {
                McpCompletion::Complete(result) => {
                    bounded(&result, 48 * 1024, 4096, 32)?;
                    (true, "MCP call completed", Some(result))
                }
                McpCompletion::Failed => (false, "MCP call failed", None),
                McpCompletion::Unknown => unreachable!(),
            };
            let ticket = self
                .ticket(id, &permit.0.nonce)?
                .ok_or(Error::Conflict("MCP approval missing"))?;
            self.connection.execute("UPDATE interactive_permission_challenges SET state='delivered' WHERE session=?1 AND nonce=?2 AND state='consumed'",params![id,permit.0.nonce])?;
            let mut actions = self.invalidate_mcp_inputs(id, &permit.0.nonce)?;
            actions.push(json!({
                "type":"chat/toolCallComplete","turnId":ticket.turn,"toolCallId":permit.0.tool,
                "result":{"success":success,"pastTenseMessage":message,"structuredContent":{"dunlin.mcp.result":result}},
                "_meta":{"dunlin.permissionNonce":permit.0.nonce,"dunlin.confirmationEvidence":"human-one-use","dunlin.mcpOutcome":if success{"complete"}else{"failed"}}
            }));
            self.event(
                id,
                "permission_lifecycle",
                json!({"nonce":permit.0.nonce,"state":"delivered","actions":actions}),
                true,
            )?;
        }
        tx.commit()?;
        self.changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
