//! Personal catalog inspection is an observation, never a tool execution grant.
use super::{
    Service,
    config::Provider,
    journal::{BrowserJournal, BrowserProof},
};
use crate::{api::AppState, store::mcp_oauth::ConnectionStatus};
use dunlin_mcp_client::{
    DispatchGuard,
    negotiated::Client,
    oauth::{
        AuthContext,
        persistence::{CredentialJournal, PersistenceError},
    },
};
use dunlin_vault::oauth::OAuthCredentials;
use serde::Serialize;
use std::sync::Arc;

/// Server-only admission, not deserializable from a browser/runner.
pub(crate) struct CatalogScope {
    pub context: AuthContext,
    pub proof: BrowserProof,
    pub membership_revision: u64,
    pub provider: Provider,
}
#[derive(Serialize)]
pub(crate) struct Catalog {
    pub connection_id: String,
    pub revision: u64,
    pub protocol_version: String,
    pub tools: Vec<String>,
    pub tool_pins: Vec<dunlin_domain::agent::AgentMcpTool>,
}
impl Service {
    pub(crate) fn inspect_catalog(
        &self,
        state: AppState,
        scope: CatalogScope,
    ) -> Result<Catalog, PersistenceError> {
        let context = scope.context;
        let journal = Arc::new(BrowserJournal {
            state,
            proof: scope.proof,
            membership_revision: Some(scope.membership_revision),
        });
        {
            let store = journal
                .state
                .store
                .lock()
                .map_err(|_| PersistenceError::Storage)?;
            journal.validate(&store, &context)?;
            let row = store
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id,
                )?
                .ok_or(PersistenceError::Unauthorized)?;
            if row.revision != context.connection_revision
                || !matches!(row.status, ConnectionStatus::Connected)
                || row.provider_id.as_deref() != Some(scope.provider.id.as_str())
            {
                return Err(PersistenceError::Conflict);
            }
        }
        let flow = scope
            .provider
            .flow(self.secrets.as_ref())
            .map_err(|_| PersistenceError::Storage)?;
        let credentials = OAuthCredentials::new(journal.clone(), self.secrets.clone());
        let binding = flow.credential_binding();
        let (record, bearer) = {
            let _writer = journal.writer_lease(&context)?;
            credentials.access_token_or_refresh(&flow, &context)?
        };
        let guard = DispatchGuard::new(Arc::new({
            let journal = journal.clone();
            let context = context.clone();
            let binding = binding.clone();
            let record = record.clone();
            move || {
                journal
                    .active(&context, &binding)
                    .is_ok_and(|current| current == record)
            }
        }));
        let mut client = Client::connect_authorized(&scope.provider.endpoint, &bearer, guard)?;
        let protocol_version = client
            .protocol_version()
            .ok_or(PersistenceError::Conflict)?
            .to_owned();
        let tools = client.tools();
        let _ = client.close();
        let tools = tools?;
        let tool_pins = tools
            .iter()
            .filter(|tool| {
                !tool.name.trim().is_empty()
                    && !tool.name.contains(['*', '?'])
                    && !tool.name.chars().any(char::is_control)
            })
            .map(|tool| {
                Ok(dunlin_domain::agent::AgentMcpTool {
                    provider_id: scope.provider.id.clone(),
                    name: tool.name.clone(),
                    definition_digest: tool.definition_digest()?,
                })
            })
            .collect::<Result<Vec<_>, dunlin_mcp_client::Error>>()?;
        let catalog = Catalog {
            connection_id: context.connection_id.clone(),
            revision: context.connection_revision,
            protocol_version,
            tools: tools.into_iter().map(|tool| tool.name).collect(),
            tool_pins,
        };
        if serde_json::to_vec(&catalog)
            .map_err(|_| PersistenceError::Storage)?
            .len()
            > 262144
        {
            return Err(PersistenceError::Limit);
        }
        if journal.active(&context, &binding)? != record {
            return Err(PersistenceError::Conflict);
        }
        Ok(catalog)
    }
}
