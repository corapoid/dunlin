//! A session's private connection bindings and host-owned MCP execution scope.
//! No OAuth credentials or keyring references are sent to the harness.
use super::*;
use crate::mcp_oauth::config::Provider;
use crate::store::{Store, mcp_oauth::ConnectionStatus};
use dunlin_domain::agent::AgentMcpTool;
use dunlin_mcp_client::oauth::AuthContext;
use std::collections::BTreeSet;

mod bindings;
pub(crate) mod forms;
mod urls;
pub(crate) use bindings::{BoundConnection, BoundSession, Selection};
pub(super) fn bind(
    state: &AppState,
    store: &Store,
    owner: &str,
    workspace: &str,
    definition: &dunlin_domain::agent::AgentDefinition,
    selections: &[Selection],
) -> Result<Vec<BoundConnection>> {
    BoundSession::capture(
        &state.config,
        store,
        owner,
        workspace,
        definition,
        selections,
    )
    .map(BoundSession::into_connections)
}

pub(crate) mod authority;
pub(crate) mod runtime;
pub(super) use runtime::Peer;
#[cfg(unix)]
pub(crate) mod proxy;
pub(crate) fn run_stdio() -> i32 {
    #[cfg(unix)]
    {
        if proxy::run().is_ok() { 0 } else { 1 }
    }
    #[cfg(not(unix))]
    {
        1
    }
}
