//! Preserve the original versioned command through asynchronous native jobs.
use super::*;
use dunlin_relay_protocol::worker::{
    ChannelFence,
    session::{
        SessionRequestRef,
        broker::{BrokerLimits, OpenBrokerSessionCommand},
    },
};

pub(super) enum Control {
    Legacy(Box<Request>),
    Broker(Box<OpenBrokerSessionCommand>),
}
impl Control {
    fn channel(&self) -> &ChannelFence {
        match self {
            Self::Legacy(request) => request.channel(),
            Self::Broker(command) => &command.open.channel,
        }
    }
    fn reference(&self) -> SessionRequestRef {
        match self {
            Self::Legacy(request) => request.reference(),
            Self::Broker(command) => SessionRequestRef::Open {
                identity: command.open.identity.clone(),
            },
        }
    }
    pub fn key(&self) -> Result<String, &'static str> {
        serde_json::to_string(&self.reference()).map_err(|_| "invalid_session_request")
    }
    pub fn validate(
        &self,
        fence: &ChannelFence,
        offer: Option<&BrokerLimits>,
    ) -> Result<(), &'static str> {
        match self {
            Self::Legacy(request) => request
                .validate_for_channel(fence)
                .map_err(|_| "worker_session_fence_mismatch"),
            Self::Broker(command) => command
                .validate_for_channel(fence, offer.ok_or("worker_broker_channel_required")?)
                .map_err(|_| "worker_session_fence_mismatch"),
        }
    }
    pub fn rejected(&self, code: Code) -> Reply {
        Reply::Rejected {
            channel: self.channel().clone(),
            request: self.reference(),
            code,
        }
    }
    pub fn accepts(&self, reply: &Reply) -> bool {
        match self {
            Self::Legacy(request) => reply.validate_for_request(request).is_ok(),
            Self::Broker(command) => match reply {
                Reply::SessionAccepted { receipt } => command.validate_receipt(receipt).is_ok(),
                Reply::Rejected {
                    channel, request, ..
                } => {
                    reply.validate().is_ok()
                        && channel == self.channel()
                        && *request == self.reference()
                }
                _ => false,
            },
        }
    }
}
