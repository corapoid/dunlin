//! Bounded active v1/v2 channel metadata, with separately proposed v3 execution
//! DTOs. The v1/v2 handshake/parsers do not negotiate or accept execution. No
//! DTO establishes verified transport, current authority or a live lease.
use dunlin_domain::hosts::HostCapabilities;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const WORKER_SUBPROTOCOL: &str = "dunlin.worker.v1";
pub const WORKER_PROTOCOL_VERSION: u16 = 1;
pub const WORKER_EXPORT_SUBPROTOCOL: &str = "dunlin.worker.v2";
pub const WORKER_EXPORT_PROTOCOL_VERSION: u16 = 2;
mod execution;
/// Named CLI template metadata only; no existing transport accepts CLI secrets.
pub mod recipe;
pub use execution::{
    AttemptFence, ChannelFence, ExecutionBudgets, ExecutionCancelDisposition, ExecutionCommand,
    ExecutionControllerMessage, ExecutionErrorCode, ExecutionIdentity, ExecutionOutcome,
    ExecutionReceipt, ExecutionRecord, ExecutionStatus, ExecutionTerminalStatus,
    ExecutionWorkerMessage, MAX_EXECUTION_FRAMES_PER_MINUTE, MAX_EXECUTION_INTEGER,
    MAX_EXECUTION_TEXT_BYTES, WORKER_EXECUTION_PROTOCOL_VERSION, WORKER_EXECUTION_SUBPROTOCOL,
    decode_execution_controller_frame, decode_execution_worker_frame, worker_rate_limit,
};
mod exports;
/// Separate persistent-session protocol, negotiated only by explicit v4 opt-in.
pub mod session;
pub use exports::{
    ExportDescriptor, ExportErrorCode, ExportGrant, ExportPermission, ExportPrincipal, ExportScope,
    ExportSummary, MAX_EXPORT_REVISION, PrincipalKind, RepoKind,
};
pub const MAX_WORKER_FRAME_BYTES: usize = 64 * 1024;
const MAX_INTEGER: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerErrorCode {
    InvalidMessage,
    UnsupportedProtocol,
    Unauthorized,
    AuthorityRevoked,
    PeerVerificationFailed,
    ChannelExpired,
    LeaseExpired,
    StaleLease,
    InvalidSequence,
    RateLimited,
    CapacityReached,
    InternalError,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerProtocolError {
    pub code: WorkerErrorCode,
    pub path: String,
    pub message: String,
}
impl std::fmt::Display for WorkerProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}
impl std::error::Error for WorkerProtocolError {}
type Result<T> = std::result::Result<T, WorkerProtocolError>;
fn invalid(path: &str, message: &str) -> WorkerProtocolError {
    WorkerProtocolError {
        code: WorkerErrorCode::InvalidMessage,
        path: path.into(),
        message: message.into(),
    }
}
fn identifier(value: &str, path: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid(path, "A bounded ASCII identifier is required"));
    }
    Ok(())
}
fn nonce(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "nonce",
            "A 256-bit lowercase hexadecimal nonce is required",
        ));
    }
    Ok(())
}
fn positive(value: u64, path: &str) -> Result<()> {
    if value == 0 || value > MAX_INTEGER {
        return Err(invalid(
            path,
            "A positive signed-64-bit integer is required",
        ));
    }
    Ok(())
}
fn challenge_ids(channel: &str, controller: &str, challenge: &str) -> Result<()> {
    identifier(channel, "channel_id")?;
    identifier(controller, "controller_boot_id")?;
    nonce(challenge)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerMessage {
    Hello {
        protocol_version: u16,
        host_id: String,
        workspace_id: String,
        enrollment_epoch: u64,
        credential_revision: u64,
        worker_boot_id: String,
        capabilities: HostCapabilities,
    },
    Ready {
        channel_id: String,
        controller_boot_id: String,
        worker_boot_id: String,
        nonce: String,
    },
    Heartbeat {
        channel_id: String,
        lease_epoch: u64,
        sequence: u64,
    },
    ExportDescribed {
        request_id: String,
        channel_id: String,
        lease_epoch: u64,
        descriptor: ExportDescriptor,
    },
    ExportError {
        request_id: String,
        channel_id: String,
        lease_epoch: u64,
        code: ExportErrorCode,
    },
}
impl WorkerMessage {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Hello {
                protocol_version,
                host_id,
                workspace_id,
                enrollment_epoch,
                credential_revision,
                worker_boot_id,
                capabilities,
            } => {
                if ![
                    WORKER_PROTOCOL_VERSION,
                    WORKER_EXPORT_PROTOCOL_VERSION,
                    WORKER_EXECUTION_PROTOCOL_VERSION,
                    session::WORKER_SESSION_PROTOCOL_VERSION,
                ]
                .contains(protocol_version)
                {
                    return Err(WorkerProtocolError {
                        code: WorkerErrorCode::UnsupportedProtocol,
                        path: "protocol_version".into(),
                        message: "Unsupported worker protocol version".into(),
                    });
                }
                identifier(host_id, "host_id")?;
                identifier(workspace_id, "workspace_id")?;
                positive(*enrollment_epoch, "enrollment_epoch")?;
                positive(*credential_revision, "credential_revision")?;
                identifier(worker_boot_id, "worker_boot_id")?;
                capabilities.validate().map_err(|error| {
                    invalid(&format!("capabilities.{}", error.path), &error.message)
                })
            }
            Self::ExportDescribed {
                request_id,
                channel_id,
                lease_epoch,
                descriptor,
            } => {
                identifier(request_id, "request_id")?;
                identifier(channel_id, "channel_id")?;
                positive(*lease_epoch, "lease_epoch")?;
                descriptor.validate()
            }
            Self::ExportError {
                request_id,
                channel_id,
                lease_epoch,
                ..
            } => {
                identifier(request_id, "request_id")?;
                identifier(channel_id, "channel_id")?;
                positive(*lease_epoch, "lease_epoch")
            }
            Self::Ready {
                channel_id,
                controller_boot_id,
                worker_boot_id,
                nonce,
            } => {
                challenge_ids(channel_id, controller_boot_id, nonce)?;
                identifier(worker_boot_id, "worker_boot_id")
            }
            Self::Heartbeat {
                channel_id,
                lease_epoch,
                sequence,
            } => {
                identifier(channel_id, "channel_id")?;
                positive(*lease_epoch, "lease_epoch")?;
                positive(*sequence, "sequence")
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControllerMessage {
    Challenge {
        channel_id: String,
        controller_boot_id: String,
        nonce: String,
        heartbeat_interval_ms: u64,
        lease_ttl_ms: u64,
    },
    Accepted {
        channel_id: String,
        controller_boot_id: String,
        worker_boot_id: String,
        lease_epoch: u64,
        expires_at_ms: u64,
    },
    HeartbeatAck {
        channel_id: String,
        lease_epoch: u64,
        sequence: u64,
        expires_at_ms: u64,
    },
    DescribeExport {
        request_id: String,
        channel_id: String,
        lease_epoch: u64,
        export_id: String,
        principal: ExportPrincipal,
        expected_export_revision: Option<u64>,
    },
    Error {
        code: WorkerErrorCode,
    },
}
impl ControllerMessage {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Challenge {
                channel_id,
                controller_boot_id,
                nonce,
                heartbeat_interval_ms,
                lease_ttl_ms,
            } => {
                challenge_ids(channel_id, controller_boot_id, nonce)?;
                if !(1000..=5000).contains(heartbeat_interval_ms)
                    || !(5000..=15000).contains(lease_ttl_ms)
                    || heartbeat_interval_ms >= lease_ttl_ms
                {
                    return Err(invalid(
                        "lease_ttl_ms",
                        "Heartbeat interval must be 1–5 seconds and strictly below the 5–15 second lease TTL",
                    ));
                }
                Ok(())
            }
            Self::Accepted {
                channel_id,
                controller_boot_id,
                worker_boot_id,
                lease_epoch,
                expires_at_ms,
            } => {
                identifier(channel_id, "channel_id")?;
                identifier(controller_boot_id, "controller_boot_id")?;
                identifier(worker_boot_id, "worker_boot_id")?;
                positive(*lease_epoch, "lease_epoch")?;
                positive(*expires_at_ms, "expires_at_ms")
            }
            Self::HeartbeatAck {
                channel_id,
                lease_epoch,
                sequence,
                expires_at_ms,
            } => {
                identifier(channel_id, "channel_id")?;
                positive(*lease_epoch, "lease_epoch")?;
                positive(*sequence, "sequence")?;
                positive(*expires_at_ms, "expires_at_ms")
            }
            Self::DescribeExport {
                request_id,
                channel_id,
                lease_epoch,
                export_id,
                principal,
                expected_export_revision,
            } => {
                identifier(request_id, "request_id")?;
                identifier(channel_id, "channel_id")?;
                positive(*lease_epoch, "lease_epoch")?;
                exports::request(export_id, principal, *expected_export_revision)
            }
            Self::Error { .. } => Ok(()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelProofRequest {
    pub channel_id: String,
    pub controller_boot_id: String,
    pub nonce: String,
}
impl ChannelProofRequest {
    pub fn validate(&self) -> Result<()> {
        challenge_ids(&self.channel_id, &self.controller_boot_id, &self.nonce)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelProofResponse {
    pub channel_id: String,
    pub controller_boot_id: String,
    pub worker_boot_id: String,
    pub nonce: String,
    pub host_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
}
impl ChannelProofResponse {
    pub fn validate(&self) -> Result<()> {
        challenge_ids(&self.channel_id, &self.controller_boot_id, &self.nonce)?;
        identifier(&self.worker_boot_id, "worker_boot_id")?;
        identifier(&self.host_id, "host_id")?;
        positive(self.enrollment_epoch, "enrollment_epoch")?;
        positive(self.credential_revision, "credential_revision")
    }
}
fn decode<T: DeserializeOwned>(frame: &str) -> Result<T> {
    if frame.len() > MAX_WORKER_FRAME_BYTES {
        return Err(invalid("frame", "Worker frame exceeds 64 KiB"));
    }
    serde_json::from_str(frame).map_err(|_| invalid("frame", "Invalid worker JSON frame"))
}
pub fn decode_frame(frame: &str) -> Result<WorkerMessage> {
    let message: WorkerMessage = decode(frame)?;
    message.validate()?;
    Ok(message)
}
pub fn decode_controller_frame(frame: &str) -> Result<ControllerMessage> {
    let message: ControllerMessage = decode(frame)?;
    message.validate()?;
    Ok(message)
}
pub fn decode_proof_request(frame: &str) -> Result<ChannelProofRequest> {
    let message: ChannelProofRequest = decode(frame)?;
    message.validate()?;
    Ok(message)
}
pub fn decode_proof_response(frame: &str) -> Result<ChannelProofResponse> {
    let message: ChannelProofResponse = decode(frame)?;
    message.validate()?;
    Ok(message)
}

#[cfg(test)]
mod tests;
