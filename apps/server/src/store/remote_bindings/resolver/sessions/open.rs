//! Versioned storage keeps legacy JSON bytes and separates the MCP open hash.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RemoteOpen {
    Broker(Box<wire::broker::OpenBrokerSessionCommand>),
    Legacy(Box<wire::SessionOpenCommand>),
}
impl std::ops::Deref for RemoteOpen {
    type Target = wire::SessionOpenCommand;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Legacy(open) => open,
            Self::Broker(command) => &command.open,
        }
    }
}
impl RemoteOpen {
    pub(super) fn validate(&self) -> Result<()> {
        valid(match self {
            Self::Legacy(open) => open.validate(),
            Self::Broker(open) => open.validate(),
        })
    }
    pub(super) fn fingerprint(&self) -> Result<String> {
        valid(match self {
            Self::Legacy(open) => wire::session_open_fingerprint(open),
            Self::Broker(open) => wire::broker::broker_session_open_fingerprint(open),
        })
    }
    pub(super) fn validate_receipt(&self, receipt: &wire::SessionReceipt) -> Result<()> {
        valid(match self {
            Self::Legacy(open) => receipt.validate_for_open(open),
            Self::Broker(open) => open.validate_receipt(receipt),
        })
    }
    pub(super) fn limits(&self) -> Option<&wire::broker::BrokerLimits> {
        match self {
            Self::Legacy(_) => None,
            Self::Broker(open) => Some(&open.broker_limits),
        }
    }
    pub(super) fn set_fingerprint(&mut self) -> Result<()> {
        let fingerprint = self.fingerprint()?;
        match self {
            Self::Legacy(open) => open.identity.fingerprint = fingerprint,
            Self::Broker(open) => open.open.identity.fingerprint = fingerprint,
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn into_legacy(self) -> Result<wire::SessionOpenCommand> {
        match self {
            Self::Legacy(open) => Ok(*open),
            Self::Broker(_) => Err(StoreError::Limit(
                "MCP dispatch requires the scoped v5 runtime broker".into(),
            )),
        }
    }
}
