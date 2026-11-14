//! Proposed private CLI v6 metadata. No value is carried, persisted or accepted
//! by this codec. Negotiation/admission never follows from a DTO or digest alone.
use crate::worker::session::broker::{BrokerHello, BrokerLimits, OpenBrokerSessionCommand};
use crate::worker::session::{SessionOpenCommand, SessionReceipt, TurnFence};
use crate::worker::{
    ChannelFence, ExportDescriptor, ExportPermission, MAX_WORKER_FRAME_BYTES, PrincipalKind,
    Result, invalid,
};
use dunlin_domain::agent::{AgentDefinition, AgentMcpTool};
use dunlin_domain::hosts::HostCapabilities;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub const WORKER_RECIPE_PROTOCOL_VERSION: u16 = 6;
pub const WORKER_RECIPE_SUBPROTOCOL: &str = "dunlin.worker.v6";
pub const MAX_PRIVATE_SECRET_BYTES: u32 = 16 * 1024;
pub const MAX_PRIVATE_REPLY_BYTES: u32 = 256 * 1024;
fn digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("digest", "A lowercase SHA-256 value is required"));
    }
    Ok(())
}
fn frame<T: Serialize>(value: &T) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|_| invalid("frame", "Cannot encode private metadata"))?
        .len()
        > MAX_WORKER_FRAME_BYTES
    {
        return Err(invalid("frame", "Private metadata exceeds 64 KiB"));
    }
    Ok(())
}
fn hash<T: Serialize>(domain: &[u8], value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("fingerprint", "Cannot encode private metadata"))?;
    let mut sha = Sha256::new();
    sha.update(domain);
    sha.update(bytes);
    Ok(hex::encode(sha.finalize()))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeLimits {
    pub max_pending: u8,
    pub max_secret_bytes: u32,
    pub max_reply_bytes: u32,
    pub max_timeout_ms: u32,
}
impl RecipeLimits {
    pub fn validate(&self) -> Result<()> {
        if !(1..=4).contains(&self.max_pending)
            || !(1..=MAX_PRIVATE_SECRET_BYTES).contains(&self.max_secret_bytes)
            || !(1024..=MAX_PRIVATE_REPLY_BYTES).contains(&self.max_reply_bytes)
            || !(1..=120000).contains(&self.max_timeout_ms)
        {
            return Err(invalid(
                "recipe_limits",
                "Unsupported private recipe budgets",
            ));
        }
        Ok(())
    }
    pub fn validate_selection(&self, offered: &Self, broker: &BrokerLimits) -> Result<()> {
        self.validate()?;
        offered.validate()?;
        broker.validate()?;
        if self.max_pending > offered.max_pending
            || self.max_secret_bytes > offered.max_secret_bytes
            || self.max_reply_bytes > offered.max_reply_bytes
            || self.max_timeout_ms > offered.max_timeout_ms
            || self.max_pending > broker.max_pending
            || self.max_reply_bytes > broker.max_reply_bytes
        {
            return Err(invalid(
                "recipe_limits",
                "Selection exceeds the negotiated private/broker budgets",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeHello {
    pub protocol_version: u16,
    pub host_id: String,
    pub workspace_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub worker_boot_id: String,
    pub capabilities: HostCapabilities,
    pub broker_limits: BrokerLimits,
    pub recipe_limits: RecipeLimits,
}
impl RecipeHello {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != WORKER_RECIPE_PROTOCOL_VERSION {
            return Err(invalid(
                "protocol_version",
                "Explicit private recipe v6 is required",
            ));
        }
        // Shape checks only; this never sends or negotiates an inner Hello5.
        BrokerHello {
            protocol_version: 5,
            host_id: self.host_id.clone(),
            workspace_id: self.workspace_id.clone(),
            enrollment_epoch: self.enrollment_epoch,
            credential_revision: self.credential_revision,
            worker_boot_id: self.worker_boot_id.clone(),
            capabilities: self.capabilities.clone(),
            broker_limits: self.broker_limits.clone(),
        }
        .validate()?;
        self.recipe_limits
            .validate_selection(&self.recipe_limits, &self.broker_limits)?;
        frame(self)
    }
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        let msg = Hello::Hello6 {
            hello: Box::new(self.clone()),
        };
        frame(&msg)?;
        serde_json::to_string(&msg).map_err(|_| invalid("hello", "Cannot encode Hello6"))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Hello {
    Hello6 { hello: Box<RecipeHello> },
}
pub fn decode_hello(bytes: &[u8]) -> Result<RecipeHello> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Hello6 exceeds 64 KiB"));
    }
    let Hello::Hello6 { hello } =
        serde_json::from_slice(bytes).map_err(|_| invalid("hello", "Invalid private Hello6"))?;
    hello.validate()?;
    Ok(*hello)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRecipeSessionCommand {
    pub protocol_version: u16,
    pub open: SessionOpenCommand,
    pub broker_limits: BrokerLimits,
    pub recipe_limits: RecipeLimits,
}
impl OpenRecipeSessionCommand {
    fn broker_shape(&self) -> OpenBrokerSessionCommand {
        OpenBrokerSessionCommand {
            protocol_version: 5,
            open: self.open.clone(),
            broker_limits: self.broker_limits.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != WORKER_RECIPE_PROTOCOL_VERSION {
            return Err(invalid("protocol_version", "Private sessions require v6"));
        }
        self.broker_shape().validate()?;
        self.recipe_limits
            .validate_selection(&self.recipe_limits, &self.broker_limits)?;
        frame(self)
    }
    pub fn validate_for_channel(
        &self,
        channel: &ChannelFence,
        broker: &BrokerLimits,
        recipes: &RecipeLimits,
    ) -> Result<()> {
        self.validate()?;
        channel.validate()?;
        broker.validate()?;
        self.recipe_limits
            .validate_selection(recipes, &self.broker_limits)?;
        if self.open.channel != *channel
            || self.broker_limits.max_pending > broker.max_pending
            || self.broker_limits.max_reply_bytes > broker.max_reply_bytes
            || open_fingerprint(self)? != self.open.identity.fingerprint
        {
            return Err(invalid(
                "open",
                "Private open differs from the channel, negotiated offer or fingerprint",
            ));
        }
        Ok(())
    }
    pub fn validate_receipt(&self, receipt: &SessionReceipt) -> Result<()> {
        self.validate()?;
        if open_fingerprint(self)? != self.open.identity.fingerprint {
            return Err(invalid("fingerprint", "Private open fingerprint differs"));
        }
        // Only receipt shape/pins; no v5 admission, renewal or authority conversion.
        receipt.validate()?;
        if receipt.identity != self.open.identity
            || receipt.fence.channel != self.open.channel
            || receipt.fence.controller_session_id != self.open.controller_session_id
            || receipt.budgets != self.open.budgets
        {
            return Err(invalid(
                "receipt",
                "Private session receipt differs from its open",
            ));
        }
        Ok(())
    }
}
pub fn open_fingerprint(command: &OpenRecipeSessionCommand) -> Result<String> {
    command.validate()?;
    let mut copy = command.clone();
    copy.open.identity.fingerprint.clear();
    hash(b"dunlin.worker.session.v6.open\0", &copy)
}
/// Safe correlation metadata for a future private command. The actual value
/// must use a separate bounded ephemeral payload and never enter this type.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeOperation {
    pub protocol_version: u16,
    pub nonce: String,
    pub turn: TurnFence,
    pub descriptor: ExportDescriptor,
    pub tool: AgentMcpTool,
    pub recipe: String,
    pub template_digest: String,
    pub credential_stamp: String,
    pub params: BTreeMap<String, String>,
    pub timeout_ms: u32,
    pub max_reply_bytes: u32,
    pub fingerprint: String,
}
impl std::fmt::Debug for RecipeOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecipeOperation")
            .field("nonce", &self.nonce)
            .field("recipe", &self.recipe)
            .field("parameter_count", &self.params.len())
            .finish()
    }
}
impl RecipeOperation {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != WORKER_RECIPE_PROTOCOL_VERSION {
            return Err(invalid("protocol_version", "Private operations require v6"));
        }
        digest(&self.nonce)?;
        digest(&self.template_digest)?;
        digest(&self.credential_stamp)?;
        digest(&self.fingerprint)?;
        digest(&self.tool.definition_digest)?;
        let suffix = self
            .tool
            .provider_id
            .strip_prefix("dunlin-secret-")
            .ok_or_else(|| invalid("tool", "A scoped recipe tool is required"))?;
        if self.tool.name != "recipe"
            || suffix.is_empty()
            || self.tool.provider_id.len() > 64
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(invalid("tool", "Invalid scoped recipe pin"));
        }
        self.turn.validate()?;
        self.descriptor.validate()?;
        if self.descriptor.grant.principal.kind != PrincipalKind::Human
            || !self
                .descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Execute)
        {
            return Err(invalid(
                "grant",
                "An explicit Human Read and Execute grant is required",
            ));
        }
        if self.recipe.is_empty()
            || self.recipe.len() > 64
            || !self
                .recipe
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
            || self.params.len() > 16
            || self.params.iter().any(|(k, v)| {
                k.is_empty()
                    || k.len() > 48
                    || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || v.len() > 4096
                    || v.contains(['\0', '\r', '\n'])
            })
            || !(1..=120000).contains(&self.timeout_ms)
            || !(1024..=MAX_PRIVATE_REPLY_BYTES).contains(&self.max_reply_bytes)
        {
            return Err(invalid(
                "recipe_operation",
                "Invalid bounded private operation",
            ));
        }
        if serde_json::to_vec(&self.params)
            .map_err(|_| invalid("params", "Cannot encode parameters"))?
            .len()
            > 8 * 1024
        {
            return Err(invalid("params", "Private parameters exceed 8 KiB"));
        }
        frame(self)
    }
    pub fn validate_for_turn(
        &self,
        turn: &TurnFence,
        descriptor: &ExportDescriptor,
        definition: &AgentDefinition,
        limits: &RecipeLimits,
    ) -> Result<()> {
        self.validate()?;
        turn.validate()?;
        descriptor.validate()?;
        limits.validate()?;
        definition
            .validate()
            .map_err(|_| invalid("profile", "Invalid pinned profile definition"))?;
        if !definition.allows_mcp(
            &self.tool.provider_id,
            &self.tool.name,
            &self.tool.definition_digest,
        ) {
            return Err(invalid(
                "tool",
                "Private operation is outside the pinned profile allowlist",
            ));
        }
        if self.turn != *turn
            || self.descriptor != *descriptor
            || self.timeout_ms > limits.max_timeout_ms
            || self.max_reply_bytes > limits.max_reply_bytes
            || operation_fingerprint(self)? != self.fingerprint
        {
            return Err(invalid(
                "recipe_operation",
                "Private operation differs from its live scope, limits or fingerprint",
            ));
        }
        Ok(())
    }
}
pub fn operation_fingerprint(operation: &RecipeOperation) -> Result<String> {
    operation.validate()?;
    let mut copy = operation.clone();
    copy.fingerprint.clear();
    hash(b"dunlin.worker.recipe.operation.v1\0", &copy)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrivateControllerMetadata {
    OpenRecipeSession {
        command: Box<OpenRecipeSessionCommand>,
    },
    PrepareRecipe {
        operation: Box<RecipeOperation>,
    },
}
/// Decode metadata only. Callers must then validate against the actual channel,
/// session/turn, policy and durable admission; this is never a dispatch permit.
pub fn decode_metadata(bytes: &[u8]) -> Result<PrivateControllerMetadata> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Private metadata exceeds 64 KiB"));
    }
    let msg: PrivateControllerMetadata =
        serde_json::from_slice(bytes).map_err(|_| invalid("frame", "Invalid private metadata"))?;
    match &msg {
        PrivateControllerMetadata::OpenRecipeSession { command } => {
            command.validate()?;
            if open_fingerprint(command)? != command.open.identity.fingerprint {
                return Err(invalid("fingerprint", "Private open fingerprint differs"));
            }
        }
        PrivateControllerMetadata::PrepareRecipe { operation } => {
            operation.validate()?;
            if operation_fingerprint(operation)? != operation.fingerprint {
                return Err(invalid(
                    "fingerprint",
                    "Private operation fingerprint differs",
                ));
            }
        }
    }
    frame(&msg)?;
    Ok(msg)
}
#[cfg(test)]
mod tests;
