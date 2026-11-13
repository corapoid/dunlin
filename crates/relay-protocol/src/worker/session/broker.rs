//! Proposed broker messages for an explicitly negotiated v5 transport.
//! Shape/digests are correlation, never authority or an OAuth credential.
//! Hello5 requires explicit transport opt-in. The controller must resolve
//! private connections itself and attest the actual live channel/session/turn.
use super::*;
use serde_json::Value;
mod open;
pub use open::{
    BrokerHello, BrokerLimits, BrokerSessionControllerMessage, OpenBrokerSessionCommand,
    broker_session_open_fingerprint, decode_broker_hello, decode_broker_session_controller_frame,
};

pub const WORKER_BROKER_PROTOCOL_VERSION: u16 = 5;
pub const WORKER_BROKER_SUBPROTOCOL: &str = "dunlin.worker.v5";
pub const MAX_ARGUMENT_BYTES: usize = 8 * 1024;
pub const MAX_REPLY_BYTES: usize = 256 * 1024;
pub const MAX_REPLY_CHUNK_BYTES: usize = 4 * 1024;
pub const MAX_REPLY_CHUNKS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerScope {
    Catalog { session: SessionFence },
    Turn { turn: TurnFence },
}
impl BrokerScope {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Catalog { session } => session.validate(),
            Self::Turn { turn } => turn.validate(),
        }
    }
    pub fn session(&self) -> &SessionFence {
        match self {
            Self::Catalog { session } => session,
            Self::Turn { turn } => &turn.session,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerOperation {
    ToolsList {
        cursor: Option<String>,
    },
    ToolsCall {
        name: String,
        arguments_json: String,
    },
}
fn arguments(value: &str) -> Result<()> {
    if value.len() > MAX_ARGUMENT_BYTES {
        return Err(invalid("arguments_json", "MCP arguments exceed 8 KiB"));
    }
    let value: Value = serde_json::from_str(value)
        .map_err(|_| invalid("arguments_json", "Invalid arguments JSON"))?;
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> bool {
        *nodes += 1;
        if depth > 32 || *nodes > 1024 {
            return false;
        }
        match value {
            Value::Object(values) => values.values().all(|v| visit(v, depth + 1, nodes)),
            Value::Array(values) => values.iter().all(|v| visit(v, depth + 1, nodes)),
            _ => true,
        }
    }
    if !value.is_object() || !visit(&value, 0, &mut 0) {
        return Err(invalid(
            "arguments_json",
            "Bounded object arguments are required",
        ));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerRequest {
    pub scope: BrokerScope,
    pub request_id: String,
    pub operation: BrokerOperation,
}
impl BrokerRequest {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        identifier(&self.request_id, "request_id")?;
        match (&self.scope, &self.operation) {
            (BrokerScope::Catalog { .. }, BrokerOperation::ToolsList { cursor }) => {
                if cursor.as_ref().is_some_and(|s| {
                    s.len() > 128 || s.is_empty() || s.chars().any(char::is_control)
                }) {
                    return Err(invalid("cursor", "Invalid catalog cursor"));
                }
            }
            (
                BrokerScope::Turn { .. },
                BrokerOperation::ToolsCall {
                    name,
                    arguments_json,
                },
            ) => {
                let alias = name
                    .strip_prefix("mcp_")
                    .ok_or_else(|| invalid("name", "A host-issued MCP alias is required"))?;
                if alias.len() != 32
                    || !alias
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(invalid("name", "Invalid host-issued alias"));
                }
                arguments(arguments_json)?;
            }
            _ => {
                return Err(invalid(
                    "scope",
                    "Tools calls require an exact turn; catalog reads require a session",
                ));
            }
        }
        frame(self)
    }
    pub fn validate_for_channel(&self, channel: &ChannelFence) -> Result<()> {
        self.validate()?;
        channel.validate()?;
        if self.scope.session().channel != *channel {
            return Err(invalid(
                "channel",
                "Broker request belongs to another live channel/boot",
            ));
        }
        Ok(())
    }
    pub fn validate_for_turn(&self, turn: &TurnFence) -> Result<()> {
        self.validate()?;
        turn.validate()?;
        if !matches!(&self.scope,BrokerScope::Turn{turn:actual} if actual==turn) {
            return Err(invalid("turn", "Broker request belongs to another turn"));
        }
        Ok(())
    }
    pub fn reference(&self) -> Result<BrokerRequestRef> {
        self.validate()?;
        Ok(BrokerRequestRef {
            scope: self.scope.clone(),
            request_id: self.request_id.clone(),
            fingerprint: hash(b"dunlin.worker.broker.v5.request\0", self)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerRequestRef {
    pub scope: BrokerScope,
    pub request_id: String,
    pub fingerprint: String,
}
impl BrokerRequestRef {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        identifier(&self.request_id, "request_id")?;
        digest(&self.fingerprint)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerReplyChunk {
    pub request: BrokerRequestRef,
    pub index: u16,
    pub chunks: u16,
    pub total_bytes: u32,
    pub sha256: String,
    pub data: String,
}
impl BrokerReplyChunk {
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        digest(&self.sha256)?;
        if self.chunks == 0
            || usize::from(self.chunks) > MAX_REPLY_CHUNKS
            || self.index >= self.chunks
            || self.total_bytes == 0
            || self.total_bytes as usize > MAX_REPLY_BYTES
            || self.total_bytes as usize > usize::from(self.chunks) * MAX_REPLY_CHUNK_BYTES
            || self.total_bytes < u32::from(self.chunks)
            || self.data.is_empty()
            || self.data.len() > MAX_REPLY_CHUNK_BYTES
            || self.data.len() > self.total_bytes as usize
        {
            return Err(invalid("reply", "Invalid bounded broker reply chunk"));
        }
        frame(self)
    }
    pub fn validate_for_request(&self, request: &BrokerRequest) -> Result<()> {
        self.validate()?;
        if self.request != request.reference()? {
            return Err(invalid(
                "request",
                "Broker response belongs to another request",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerError {
    Denied,
    Stale,
    CapacityReached,
    Unavailable,
    OutcomeUnknown,
    InvalidRequest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerWorkerMessage {
    McpRequest { request: BrokerRequest },
}
impl BrokerWorkerMessage {
    pub fn validate(&self) -> Result<()> {
        let Self::McpRequest { request } = self;
        request.validate()?;
        frame(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerControllerMessage {
    McpReplyChunk {
        chunk: BrokerReplyChunk,
    },
    McpRejected {
        request: BrokerRequestRef,
        code: BrokerError,
    },
}
impl BrokerControllerMessage {
    pub fn validate_for_request(&self, request: &BrokerRequest) -> Result<()> {
        match self {
            Self::McpReplyChunk { chunk } => chunk.validate_for_request(request)?,
            Self::McpRejected {
                request: reference, ..
            } => {
                reference.validate()?;
                if *reference != request.reference()? {
                    return Err(invalid(
                        "request",
                        "Broker rejection belongs to another request",
                    ));
                }
            }
        }
        frame(self)
    }
}
pub fn decode_broker_worker_frame(bytes: &[u8]) -> Result<BrokerWorkerMessage> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Broker frame exceeds 64 KiB"));
    }
    let message: BrokerWorkerMessage = serde_json::from_slice(bytes)
        .map_err(|_| invalid("frame", "Invalid broker worker message"))?;
    message.validate()?;
    Ok(message)
}
pub fn decode_broker_controller_frame(
    bytes: &[u8],
    request: &BrokerRequest,
) -> Result<BrokerControllerMessage> {
    if bytes.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Broker frame exceeds 64 KiB"));
    }
    let message: BrokerControllerMessage = serde_json::from_slice(bytes)
        .map_err(|_| invalid("frame", "Invalid broker controller message"))?;
    message.validate_for_request(request)?;
    Ok(message)
}
/// Pure bounded encoding, without network, replay, credentials or authority.
pub fn chunk_reply(request: &BrokerRequest, payload: &str) -> Result<Vec<BrokerReplyChunk>> {
    let reference = request.reference()?;
    if payload.is_empty() || payload.len() > MAX_REPLY_BYTES {
        return Err(invalid("reply", "Broker reply exceeds 256 KiB"));
    }
    serde_json::from_str::<Value>(payload)
        .map_err(|_| invalid("reply", "Broker reply must be JSON"))?;
    let sha256 = hex::encode(Sha256::digest(payload.as_bytes()));
    let mut parts = Vec::new();
    let mut start = 0;
    while start < payload.len() {
        let mut end = (start + MAX_REPLY_CHUNK_BYTES).min(payload.len());
        while !payload.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(payload[start..end].to_owned());
        start = end;
    }
    if parts.len() > MAX_REPLY_CHUNKS {
        return Err(invalid("reply", "Too many broker chunks"));
    }
    let count = parts.len() as u16;
    parts
        .into_iter()
        .enumerate()
        .map(|(index, data)| {
            let chunk = BrokerReplyChunk {
                request: reference.clone(),
                index: index as u16,
                chunks: count,
                total_bytes: payload.len() as u32,
                sha256: sha256.clone(),
                data,
            };
            chunk.validate()?;
            Ok(chunk)
        })
        .collect()
}
/// One operation only. Any mismatch/duplicate/reorder poisons the assembly;
/// callers still need fresh transport/export/lease authority before delivery.
pub struct ReplyAssembly {
    request: BrokerRequestRef,
    header: Option<(u16, u32, String)>,
    next: u16,
    bytes: Vec<u8>,
    closed: bool,
}
impl ReplyAssembly {
    pub fn new(request: &BrokerRequest) -> Result<Self> {
        Ok(Self {
            request: request.reference()?,
            header: None,
            next: 0,
            bytes: Vec::new(),
            closed: false,
        })
    }
    pub fn push(&mut self, chunk: &BrokerReplyChunk) -> Result<Option<String>> {
        let result = self.accept(chunk);
        if result.is_err() {
            self.closed = true;
            self.bytes.clear();
        }
        result
    }
    fn accept(&mut self, chunk: &BrokerReplyChunk) -> Result<Option<String>> {
        if self.closed {
            return Err(invalid("reply", "Broker reply assembly is closed"));
        }
        chunk.validate()?;
        if chunk.request != self.request || chunk.index != self.next {
            return Err(invalid("reply", "Uncorrelated or reordered broker chunk"));
        }
        let header = (chunk.chunks, chunk.total_bytes, chunk.sha256.clone());
        if self.header.as_ref().is_some_and(|prior| *prior != header) {
            return Err(invalid("reply", "Broker reply header changed"));
        }
        if self.header.is_none() {
            self.bytes.reserve(chunk.total_bytes as usize);
            self.header = Some(header);
        }
        if self.bytes.len() + chunk.data.len() > chunk.total_bytes as usize {
            return Err(invalid("reply", "Broker reply exceeds declared size"));
        }
        self.bytes.extend_from_slice(chunk.data.as_bytes());
        self.next += 1;
        if self.next < chunk.chunks {
            return Ok(None);
        }
        self.closed = true;
        if self.bytes.len() != chunk.total_bytes as usize
            || hex::encode(Sha256::digest(&self.bytes)) != chunk.sha256
        {
            return Err(invalid("reply", "Broker reply digest/length mismatch"));
        }
        let payload = String::from_utf8(std::mem::take(&mut self.bytes))
            .map_err(|_| invalid("reply", "Invalid broker reply UTF-8"))?;
        serde_json::from_str::<Value>(&payload)
            .map_err(|_| invalid("reply", "Invalid broker reply JSON"))?;
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests;
