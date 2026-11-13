//! The socket queue owns the original versioned command, never a v4 projection
//! of an MCP open. A reply must correlate against that exact command.
use super::*;
use wire::broker::{BrokerLimits, BrokerSessionControllerMessage, OpenBrokerSessionCommand};

#[derive(Clone)]
pub(super) enum Control {
    Legacy(Box<Request>),
    Broker(Box<OpenBrokerSessionCommand>),
}
impl Control {
    pub fn open(&self) -> Option<&wire::SessionOpenCommand> {
        match self {
            Self::Legacy(request) => match request.as_ref() {
                Request::OpenSession { command } => Some(command),
                _ => None,
            },
            Self::Broker(command) => Some(&command.open),
        }
    }
    pub fn validate_for_channel(
        &self,
        channel: &ChannelFence,
        offered: Option<&BrokerLimits>,
    ) -> std::result::Result<(), TransportError> {
        match self {
            Self::Legacy(request) => request
                .validate_for_channel(channel)
                .map_err(|_| TransportError::InvalidResponse),
            Self::Broker(command) => BrokerSessionControllerMessage::OpenBrokerSession {
                command: command.clone(),
            }
            .validate_for_channel(channel, offered.ok_or(TransportError::Unavailable)?)
            .map_err(|_| TransportError::InvalidResponse),
        }
    }
    pub fn key(&self) -> Result<String> {
        match self {
            Self::Legacy(request) => request_key(request),
            Self::Broker(command) => serde_json::to_string(&(
                &command.open.channel,
                wire::SessionRequestRef::Open {
                    identity: command.open.identity.clone(),
                },
            ))
            .map_err(|_| WorkerErrorCode::InvalidMessage),
        }
    }
    pub fn accepts(&self, reply: &Reply) -> bool {
        match self {
            Self::Legacy(request) => reply.validate_for_request(request).is_ok(),
            Self::Broker(command) => {
                // Rejections must validate the original fingerprint too; an
                // otherwise well-shaped command is not a correlation proof.
                if reply.validate().is_err()
                    || command
                        .validate_for_channel(&command.open.channel, &command.broker_limits)
                        .is_err()
                {
                    return false;
                }
                match reply {
                    Reply::SessionAccepted { receipt } => command.validate_receipt(receipt).is_ok(),
                    Reply::Rejected {
                        channel, request, ..
                    } => {
                        channel == &command.open.channel
                            && *request
                                == (wire::SessionRequestRef::Open {
                                    identity: command.open.identity.clone(),
                                })
                    }
                    _ => false,
                }
            }
        }
    }
    pub fn encode(&self) -> Result<String> {
        match self {
            Self::Legacy(request) => {
                request
                    .validate()
                    .map_err(|_| WorkerErrorCode::InvalidMessage)?;
                serde_json::to_string(request)
            }
            Self::Broker(command) => {
                let message = BrokerSessionControllerMessage::OpenBrokerSession {
                    command: command.clone(),
                };
                message
                    .validate_for_channel(&command.open.channel, &command.broker_limits)
                    .map_err(|_| WorkerErrorCode::InvalidMessage)?;
                serde_json::to_string(&message)
            }
        }
        .map_err(|_| WorkerErrorCode::InternalError)
    }
}
