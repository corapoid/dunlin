//! Version-specific Hello at the authenticated socket boundary. Negotiating
//! transport does not grant tools or select a private OAuth connection.
use super::*;
use dunlin_relay_protocol::worker::session::broker as wire;
mod producer;
pub(crate) use producer::{BrokerRegistration, SessionBroker};
pub(super) use producer::{Exchange, Registry};

pub(super) struct ChannelHello {
    pub protocol_version: u16,
    pub host_id: String,
    pub workspace_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub worker_boot_id: String,
    pub capabilities: dunlin_domain::hosts::HostCapabilities,
    pub broker_offer: Option<wire::BrokerLimits>,
}
pub(super) async fn receive_hello(
    socket: &mut WebSocket,
    rate: &mut Rate,
    version: u16,
) -> Result<ChannelHello> {
    if version != wire::WORKER_BROKER_PROTOCOL_VERSION {
        let WorkerMessage::Hello {
            protocol_version,
            host_id,
            workspace_id,
            enrollment_epoch,
            credential_revision,
            worker_boot_id,
            capabilities,
        } = receive(socket, rate).await?
        else {
            return Err(WorkerErrorCode::InvalidMessage);
        };
        return Ok(ChannelHello {
            protocol_version,
            host_id,
            workspace_id,
            enrollment_epoch,
            credential_revision,
            worker_boot_id,
            capabilities,
            broker_offer: None,
        });
    }
    loop {
        let frame = socket
            .recv()
            .await
            .ok_or(WorkerErrorCode::ChannelExpired)?
            .map_err(|_| WorkerErrorCode::InvalidMessage)?;
        rate.frame_for_protocol(version)?;
        match frame {
            Message::Text(text) => {
                let hello = wire::decode_broker_hello(text.as_bytes())
                    .map_err(|_| WorkerErrorCode::InvalidMessage)?;
                return Ok(ChannelHello {
                    protocol_version: hello.protocol_version,
                    host_id: hello.host_id,
                    workspace_id: hello.workspace_id,
                    enrollment_epoch: hello.enrollment_epoch,
                    credential_revision: hello.credential_revision,
                    worker_boot_id: hello.worker_boot_id,
                    capabilities: hello.capabilities,
                    broker_offer: Some(hello.broker_limits),
                });
            }
            Message::Ping(_) | Message::Pong(_) => {}
            _ => return Err(WorkerErrorCode::InvalidMessage),
        }
    }
}
/// Decode only after negotiated v5. Correlation alone grants no tools/account.
pub(super) fn request(
    text: &str,
    transport: &sessions::SessionTransport,
) -> Result<Option<wire::BrokerRequest>> {
    if transport.broker_offer().is_none() {
        return Ok(None);
    }
    let Ok(wire::BrokerWorkerMessage::McpRequest { request }) =
        wire::decode_broker_worker_frame(text.as_bytes())
    else {
        return Ok(None);
    };
    request
        .validate_for_channel(transport.fence())
        .map_err(|_| WorkerErrorCode::InvalidMessage)?;
    Ok(Some(request))
}
pub(super) async fn send_reply(
    socket: &mut WebSocket,
    reply: &wire::BrokerControllerMessage,
) -> Result<()> {
    let encoded = serde_json::to_string(reply).map_err(|_| WorkerErrorCode::InternalError)?;
    tokio::time::timeout(SEND, socket.send(Message::Text(encoded.into())))
        .await
        .map_err(|_| WorkerErrorCode::ChannelExpired)?
        .map_err(|_| WorkerErrorCode::ChannelExpired)?;
    Ok(())
}
