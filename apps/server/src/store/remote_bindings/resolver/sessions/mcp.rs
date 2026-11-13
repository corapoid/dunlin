//! Private admission inputs; only the wire-safe OpenBrokerSession crosses WSS.
use super::*;
use crate::{
    config::Config,
    interactive::mcp::{BoundSession, Selection},
    worker_channel::SessionTransport,
};

pub(crate) struct RemoteMcpAdmission<'a> {
    pub config: &'a Config,
    pub selections: &'a [Selection],
    pub transport: SessionTransport,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct McpPins {
    pub snapshot: String,
    pub limits: wire::broker::BrokerLimits,
}
pub(super) struct McpPreparation {
    config: Config,
    pub transport: SessionTransport,
    pins: McpPins,
}
pub(super) enum McpInput<'a> {
    Fresh(RemoteMcpAdmission<'a>),
    Pinned(&'a McpPreparation),
    Live {
        config: &'a Config,
        transport: &'a SessionTransport,
        pins: &'a McpPins,
    },
}
impl McpInput<'_> {
    pub(super) fn prepare(
        self,
        store: &Store,
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        candidate: &RemoteTargetCandidate,
    ) -> Result<McpPreparation> {
        let (config, transport, snapshot) = match self {
            Self::Fresh(input) => {
                let bound = BoundSession::capture(
                    input.config,
                    store,
                    owner,
                    workspace,
                    definition,
                    input.selections,
                )
                .map_err(|_| denied())?;
                (
                    input.config.clone(),
                    input.transport,
                    bound.encode().map_err(|_| denied())?,
                )
            }
            Self::Pinned(previous) => {
                let bound = BoundSession::restore(
                    &previous.config,
                    store,
                    owner,
                    workspace,
                    definition,
                    &previous.pins.snapshot,
                )
                .map_err(|_| denied())?;
                (
                    previous.config.clone(),
                    previous.transport.clone(),
                    bound.encode().map_err(|_| denied())?,
                )
            }
            Self::Live {
                config,
                transport,
                pins,
            } => {
                BoundSession::validate_live_snapshot(
                    config,
                    store,
                    owner,
                    workspace,
                    definition,
                    &pins.snapshot,
                )
                .map_err(|_| denied())?;
                if transport.broker_offer() != Some(&pins.limits) {
                    return Err(denied());
                }
                (config.clone(), transport.clone(), pins.snapshot.clone())
            }
        };
        if definition.mcp.is_empty()
            || !config.worker_controller.as_ref().is_some_and(|c| {
                c.execution_enabled
                    && c.persistent_sessions
                    && c.mcp_broker
                    && c.node_id == candidate.controller_node_id
            })
            || transport.host_id() != candidate.host_id()
            || transport.identity().workspace_id() != workspace
        {
            return Err(denied());
        }
        let limits = transport.broker_offer().cloned().ok_or_else(denied)?;
        transport.attest_in_transaction(
            store,
            transport.identity(),
            transport.lease(),
            transport.fence(),
        )?;
        let pins = McpPins { snapshot, limits };
        Ok(McpPreparation {
            config,
            transport,
            pins,
        })
    }
}
impl McpPreparation {
    pub(super) fn pins(&self) -> McpPins {
        self.pins.clone()
    }
}
