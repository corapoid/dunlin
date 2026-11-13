//! Version-specific v5 handshake/open shapes; MCP admission also needs a bound broker.
use super::*;
use crate::worker::{WorkerMessage, session::WORKER_SESSION_PROTOCOL_VERSION};
use dunlin_domain::hosts::HostCapabilities;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerLimits {
    pub max_pending: u8,
    pub max_reply_bytes: u32,
}
impl BrokerLimits {
    pub fn validate(&self) -> Result<()> {
        if !(1..=8).contains(&self.max_pending)
            || self.max_reply_bytes == 0
            || self.max_reply_bytes as usize > MAX_REPLY_BYTES
        {
            return Err(invalid("broker_limits", "Unsupported broker budgets"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerHello {
    pub protocol_version: u16,
    pub host_id: String,
    pub workspace_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub worker_boot_id: String,
    pub capabilities: HostCapabilities,
    pub broker_limits: BrokerLimits,
}
impl BrokerHello {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != WORKER_BROKER_PROTOCOL_VERSION {
            return Err(invalid(
                "protocol_version",
                "Explicit worker broker v5 is required",
            ));
        }
        self.broker_limits.validate()?;
        // Reuse bounded identity/capability shape checks only. The validated
        // wire stays v5; this never sends or negotiates a legacy Hello.
        WorkerMessage::Hello {
            protocol_version: WORKER_SESSION_PROTOCOL_VERSION,
            host_id: self.host_id.clone(),
            workspace_id: self.workspace_id.clone(),
            enrollment_epoch: self.enrollment_epoch,
            credential_revision: self.credential_revision,
            worker_boot_id: self.worker_boot_id.clone(),
            capabilities: self.capabilities.clone(),
        }
        .validate()?;
        frame(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Handshake {
    Hello5 { hello: BrokerHello },
}
pub fn decode_broker_hello(bytes: &[u8]) -> Result<BrokerHello> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Broker hello exceeds 64 KiB"));
    }
    let Handshake::Hello5 { hello } =
        serde_json::from_slice(bytes).map_err(|_| invalid("hello", "Invalid v5 broker Hello"))?;
    hello.validate()?;
    Ok(hello)
}
impl BrokerHello {
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        let message = Handshake::Hello5 {
            hello: self.clone(),
        };
        frame(&message)?;
        serde_json::to_string(&message).map_err(|_| invalid("hello", "Cannot encode broker Hello"))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenBrokerSessionCommand {
    pub protocol_version: u16,
    pub open: SessionOpenCommand,
    pub broker_limits: BrokerLimits,
}
impl OpenBrokerSessionCommand {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != WORKER_BROKER_PROTOCOL_VERSION {
            return Err(invalid("protocol_version", "Broker sessions require v5"));
        }
        self.broker_limits.validate()?;
        self.open.validate_mode(true)?;
        if self.open.definition.mcp.is_empty() {
            return Err(invalid(
                "definition.mcp",
                "Broker sessions require an explicit published MCP policy",
            ));
        }
        frame(self)
    }
    pub fn validate_for_channel(
        &self,
        channel: &ChannelFence,
        offered: &BrokerLimits,
    ) -> Result<()> {
        self.validate()?;
        channel.validate()?;
        offered.validate()?;
        if broker_session_open_fingerprint(self)? != self.open.identity.fingerprint {
            return Err(invalid(
                "identity.fingerprint",
                "Broker open payload fingerprint differs",
            ));
        }
        if self.open.channel != *channel {
            return Err(invalid(
                "channel",
                "Broker open belongs to another channel/boot",
            ));
        }
        if self.broker_limits.max_pending > offered.max_pending
            || self.broker_limits.max_reply_bytes > offered.max_reply_bytes
        {
            return Err(invalid(
                "broker_limits",
                "Broker session exceeds negotiated limits",
            ));
        }
        Ok(())
    }
    pub fn validate_receipt(&self, receipt: &SessionReceipt) -> Result<()> {
        self.validate()?;
        if broker_session_open_fingerprint(self)? != self.open.identity.fingerprint {
            return Err(invalid(
                "identity.fingerprint",
                "Broker open payload fingerprint differs",
            ));
        }
        receipt.validate_for_open_mode(&self.open, true)
    }
}
pub fn broker_session_open_fingerprint(command: &OpenBrokerSessionCommand) -> Result<String> {
    command.validate()?;
    let mut command = command.clone();
    command.open.identity.fingerprint.clear();
    hash(b"dunlin.worker.session.v5.open\0", &command)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerSessionControllerMessage {
    OpenBrokerSession {
        command: Box<OpenBrokerSessionCommand>,
    },
}
impl BrokerSessionControllerMessage {
    pub fn validate_for_channel(
        &self,
        channel: &ChannelFence,
        offered: &BrokerLimits,
    ) -> Result<()> {
        let Self::OpenBrokerSession { command } = self;
        command.validate_for_channel(channel, offered)?;
        frame(self)
    }
}
pub fn decode_broker_session_controller_frame(
    bytes: &[u8],
    channel: &ChannelFence,
    offered: &BrokerLimits,
) -> Result<BrokerSessionControllerMessage> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Broker session frame exceeds 64 KiB"));
    }
    let message: BrokerSessionControllerMessage = serde_json::from_slice(bytes)
        .map_err(|_| invalid("frame", "Invalid broker session command"))?;
    message.validate_for_channel(channel, offered)?;
    Ok(message)
}

#[cfg(test)]
mod tests;
