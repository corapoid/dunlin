//! Private session bindings are data, never OAuth or dispatch authority.
use super::*;
use crate::config::Config;
use dunlin_domain::agent::AgentDefinition;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024;

#[derive(Clone, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionPin {
    provider_id: String,
    connection_id: String,
    revision: u64,
    configuration: String,
}
#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema_version: u16,
    owner: String,
    workspace: String,
    profile_digest: String,
    connections: Vec<ConnectionPin>,
}
/// Cannot be deserialized. A stored snapshot is only input to fresh account and
/// operator-configuration checks. It never recreates a session/dispatch permit.
pub(crate) struct BoundSession {
    snapshot: Snapshot,
    connections: Vec<BoundConnection>,
}
impl BoundSession {
    /// Shape/scope only: negative recovery never recreates a live binding.
    pub(crate) fn validate_snapshot(
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        encoded: &str,
    ) -> Result<()> {
        Self::snapshot_data(owner, workspace, definition, encoded).map(|_| ())
    }
    fn snapshot_data(
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        encoded: &str,
    ) -> Result<Snapshot> {
        if encoded.len() > MAX_SNAPSHOT_BYTES {
            return Err("invalid_mcp_binding_snapshot");
        }
        let snapshot: Snapshot =
            serde_json::from_str(encoded).map_err(|_| "invalid_mcp_binding_snapshot")?;
        snapshot.validate()?;
        definition.validate().map_err(|_| "invalid_agent_profile")?;
        if snapshot.owner != owner
            || snapshot.workspace != workspace
            || snapshot.profile_digest
                != crate::worker_execution::profile_digest(definition)
                    .map_err(|_| "invalid_agent_profile")?
        {
            return Err("mcp_binding_scope_changed");
        }
        Ok(snapshot)
    }
    pub(crate) fn capture(
        config: &Config,
        store: &Store,
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        selections: &[Selection],
    ) -> Result<Self> {
        definition.validate().map_err(|_| "invalid_agent_profile")?;
        let connections = bind_connections(
            config,
            store,
            owner,
            workspace,
            &definition.mcp,
            selections,
            false,
        )?;
        let digest = crate::worker_execution::profile_digest(definition)
            .map_err(|_| "invalid_agent_profile")?;
        if connections
            .iter()
            .any(|c| matches!(&c.backend, Backend::Scoped(g, _) if g.profile.digest != digest))
        {
            return Err("mcp_secret_profile_changed");
        }
        let snapshot = Snapshot {
            schema_version: 1,
            owner: owner.into(),
            workspace: workspace.into(),
            profile_digest: crate::worker_execution::profile_digest(definition)
                .map_err(|_| "invalid_agent_profile")?,
            connections: connections.iter().map(BoundConnection::pin).collect(),
        };
        let bound = Self {
            snapshot,
            connections,
        };
        bound.encode()?;
        Ok(bound)
    }
    pub(crate) fn encode(&self) -> Result<String> {
        self.snapshot.validate()?;
        let encoded =
            serde_json::to_string(&self.snapshot).map_err(|_| "invalid_mcp_binding_snapshot")?;
        if encoded.len() > MAX_SNAPSHOT_BYTES {
            return Err("invalid_mcp_binding_snapshot");
        }
        Ok(encoded)
    }
    pub(crate) fn restore(
        config: &Config,
        store: &Store,
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        encoded: &str,
    ) -> Result<Self> {
        Self::restore_mode(config, store, owner, workspace, definition, encoded, false)
    }
    /// Live guard only. Refreshing is handled by the credential journal; this
    /// never returns a binding or a dispatch permit to its caller.
    pub(crate) fn validate_live_snapshot(
        config: &Config,
        store: &Store,
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        encoded: &str,
    ) -> Result<()> {
        Self::restore_mode(config, store, owner, workspace, definition, encoded, true).map(|_| ())
    }
    fn restore_mode(
        config: &Config,
        store: &Store,
        owner: &str,
        workspace: &str,
        definition: &AgentDefinition,
        encoded: &str,
        live: bool,
    ) -> Result<Self> {
        let snapshot = Self::snapshot_data(owner, workspace, definition, encoded)?;
        let selections = snapshot
            .connections
            .iter()
            .map(|pin| Selection {
                provider_id: pin.provider_id.clone(),
                connection_id: pin.connection_id.clone(),
                revision: pin.revision,
            })
            .collect::<Vec<_>>();
        let connections = bind_connections(
            config,
            store,
            owner,
            workspace,
            &definition.mcp,
            &selections,
            live,
        )?;
        if connections.iter().any(|c| matches!(&c.backend, Backend::Scoped(g, _) if g.profile.digest != snapshot.profile_digest)) { return Err("mcp_secret_profile_changed"); }
        if connections
            .iter()
            .map(BoundConnection::pin)
            .zip(&snapshot.connections)
            .any(|(fresh, saved)| {
                fresh.provider_id != saved.provider_id
                    || fresh.connection_id != saved.connection_id
                    || fresh.revision != saved.revision
                    || fresh.configuration != saved.configuration
            })
        {
            return Err("mcp_provider_changed");
        }
        Ok(Self {
            snapshot,
            connections,
        })
    }
    pub(crate) fn validate_target(
        &self,
        target: &dunlin_vault::scoped::Binding,
        profile: &dunlin_domain::agent::AgentProfileRef,
    ) -> Result<()> {
        if self.connections.iter().any(|c| matches!(&c.backend, Backend::Scoped(g,v) if g.profile != *profile || v.binding != *target)) {
            return Err("mcp_secret_target_changed");
        }
        Ok(())
    }
    pub(crate) fn into_connections(self) -> Vec<BoundConnection> {
        self.connections
    }
}
impl Snapshot {
    fn validate(&self) -> Result<()> {
        fn digest(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        if self.schema_version != 1 || self.connections.len() > 8 || !digest(&self.profile_digest) {
            return Err("invalid_mcp_binding_snapshot");
        }
        let scope = AuthContext {
            user_id: self.owner.clone(),
            workspace_id: self.workspace.clone(),
            connection_id: "scope".into(),
            connection_revision: 1,
        };
        scope
            .validate()
            .map_err(|_| "invalid_mcp_binding_snapshot")?;
        let mut previous: Option<&str> = None;
        for pin in &self.connections {
            if pin.provider_id.is_empty()
                || pin.provider_id.len() > 64
                || !pin
                    .provider_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !digest(&pin.configuration)
                || previous.is_some_and(|id| id >= pin.provider_id.as_str())
            {
                return Err("invalid_mcp_binding_snapshot");
            }
            previous = Some(&pin.provider_id);
            AuthContext {
                connection_id: pin.connection_id.clone(),
                connection_revision: pin.revision,
                ..scope.clone()
            }
            .validate()
            .map_err(|_| "invalid_mcp_binding_snapshot")?;
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    pub provider_id: String,
    pub connection_id: String,
    pub revision: u64,
}
#[derive(Clone)]
pub(crate) struct BoundConnection {
    pub(super) context: AuthContext,
    pub(super) backend: Backend,
    pub(super) tools: Vec<AgentMcpTool>,
}
#[derive(Clone)]
pub(crate) enum Backend {
    OAuth(Box<Provider>),
    Scoped(
        Box<crate::store::scoped_secrets::Grant>,
        Box<dunlin_vault::scoped::Version>,
    ),
}
impl BoundConnection {
    pub(super) fn provider_id(&self) -> &str {
        match &self.backend {
            Backend::OAuth(p) => &p.id,
            Backend::Scoped(g, _) => &g.provider_id,
        }
    }
    pub(super) fn oauth(&self) -> Result<&Provider> {
        match &self.backend {
            Backend::OAuth(p) => Ok(p),
            _ => Err("mcp_oauth_binding_required"),
        }
    }
    fn configuration(&self) -> String {
        match &self.backend {
            Backend::OAuth(p) => p.digest(),
            Backend::Scoped(g, v) => {
                use sha2::{Digest, Sha256};
                // Private fingerprint; neither the envelope reference nor its hash is exposed.
                hex::encode(Sha256::digest(
                    serde_json::to_vec(
                        &json!({"domain":"dunlin.scoped-binding.v1","grant":g,"binding":v.binding,"reference":v.reference,"envelope_sha256":v.sha256}),
                    )
                    .expect("serializable binding"),
                ))
            }
        }
    }

    fn pin(&self) -> ConnectionPin {
        ConnectionPin {
            provider_id: self.provider_id().into(),
            connection_id: self.context.connection_id.clone(),
            revision: self.context.connection_revision,
            configuration: self.configuration(),
        }
    }
    pub fn identity(&self) -> Value {
        json!({"provider_id":self.provider_id(),"connection_id":self.context.connection_id,
            "revision":self.context.connection_revision,"configuration":self.configuration()})
    }
    pub fn validate(&self, state: &AppState, store: &Store) -> Result<()> {
        self.validate_config(&state.config, store)
    }
    pub(crate) fn validate_config(&self, config: &Config, store: &Store) -> Result<()> {
        if let Backend::Scoped(saved, version) = &self.backend {
            let (grant, current) = store
                .scoped_secret_selection(
                    &self.context.user_id,
                    &self.context.workspace_id,
                    &saved.id,
                    saved.revision,
                )
                .map_err(|_| "mcp_secret_grant_changed")?;
            if grant.profile != saved.profile
                || grant.read_only != saved.read_only
                || grant.provider_id != saved.provider_id
                || current != **version
            {
                return Err("mcp_secret_grant_changed");
            }
            return Ok(());
        }
        let provider = self.oauth()?;
        if !config
            .mcp_oauth
            .iter()
            .any(|p| p.id == provider.id && p.digest() == provider.digest())
        {
            return Err("mcp_provider_changed");
        }
        let connection = store
            .mcp_oauth_connection(
                &self.context.user_id,
                &self.context.workspace_id,
                &self.context.connection_id,
            )
            .map_err(|_| "mcp_connection_unavailable")?
            .ok_or("mcp_connection_unavailable")?;
        if connection.revision != self.context.connection_revision
            || connection.provider_id.as_deref() != Some(provider.id.as_str())
            || matches!(
                connection.status,
                ConnectionStatus::Disabled | ConnectionStatus::AuthorizationIncomplete
            )
        {
            return Err("mcp_connection_changed");
        }
        Ok(())
    }
}
fn bind_connections(
    config: &Config,
    store: &Store,
    user: &str,
    workspace: &str,
    tools: &[AgentMcpTool],
    selections: &[Selection],
    live: bool,
) -> Result<Vec<BoundConnection>> {
    let providers = tools
        .iter()
        .map(|t| t.provider_id.as_str())
        .collect::<BTreeSet<_>>();
    if selections.len() > 8 || selections.len() != providers.len() {
        return Err("mcp_connections_required");
    }
    let mut seen = BTreeSet::new();
    let mut bound = Vec::new();
    for selected in selections {
        if !providers.contains(selected.provider_id.as_str())
            || !seen.insert(selected.provider_id.as_str())
        {
            return Err("mcp_connection_selection_invalid");
        }
        let context = AuthContext {
            user_id: user.into(),
            workspace_id: workspace.into(),
            connection_id: selected.connection_id.clone(),
            connection_revision: selected.revision,
        };
        context
            .validate()
            .map_err(|_| "mcp_connection_selection_invalid")?;
        if dunlin_vault::scoped::tool::is_provider(&selected.provider_id) {
            let (grant, version) = store
                .scoped_secret_selection(
                    user,
                    workspace,
                    &selected.connection_id,
                    selected.revision,
                )
                .map_err(|_| "mcp_secret_grant_unavailable")?;
            let pins: Vec<_> = tools
                .iter()
                .filter(|t| t.provider_id == selected.provider_id)
                .cloned()
                .collect();
            let tool = dunlin_vault::scoped::tool::http();
            if grant.provider_id != selected.provider_id
                || pins.len() != 1
                || pins[0].name != tool.name
                || pins[0].definition_digest
                    != tool
                        .definition_digest()
                        .map_err(|_| "mcp_schema_unavailable")?
            {
                return Err("mcp_secret_profile_changed");
            }
            bound.push(BoundConnection {
                context,
                backend: Backend::Scoped(Box::new(grant), Box::new(version)),
                tools: pins,
            });
            continue;
        }
        let provider = config
            .mcp_oauth
            .iter()
            .find(|p| p.id == selected.provider_id)
            .cloned()
            .ok_or("mcp_provider_unavailable")?;
        let connection = store
            .mcp_oauth_connection(user, workspace, &selected.connection_id)
            .map_err(|_| "mcp_connection_unavailable")?
            .ok_or("mcp_connection_unavailable")?;
        if connection.revision != selected.revision
            || !(matches!(connection.status, ConnectionStatus::Connected)
                || (live && matches!(connection.status, ConnectionStatus::ReauthorizationRequired)))
            || connection.provider_id.as_deref() != Some(selected.provider_id.as_str())
        {
            return Err("mcp_connection_changed");
        }
        let value = BoundConnection {
            context,
            backend: Backend::OAuth(Box::new(provider)),
            tools: tools
                .iter()
                .filter(|t| t.provider_id == selected.provider_id)
                .cloned()
                .collect(),
        };
        value.validate_config(config, store)?;
        bound.push(value);
    }
    bound.sort_by(|a, b| a.provider_id().cmp(b.provider_id()));
    Ok(bound)
}

#[cfg(test)]
mod tests;
