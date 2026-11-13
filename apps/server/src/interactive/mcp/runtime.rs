use super::authority::{Authority, SessionPins};
use super::*;
use dunlin_mcp_client::{
    DispatchGuard, Tool,
    negotiated::Client,
    oauth::persistence::{CredentialIntent, CredentialJournal, CredentialRecord, PersistenceError},
};
use dunlin_session_runtime::{
    McpCompletion, McpDispatchPermit, McpInvocation, McpPermissionChallenge, PermissionStatus,
};
use dunlin_vault::oauth::OAuthCredentials;
use std::{collections::BTreeMap, path::PathBuf};

mod input;

type NetResult<T> = std::result::Result<T, PersistenceError>;
type ExtraGuard = Arc<dyn Fn() -> bool + Send + Sync>;
#[derive(Clone)]
struct Entry {
    alias: String,
    tool: Tool,
    connection: usize,
}
pub(crate) struct Broker {
    state: AppState,
    authority: Arc<dyn Authority>,
    pins: SessionPins,
    connections: Vec<BoundConnection>,
    _slot: tokio::sync::OwnedSemaphorePermit,
    lifetime: Instant,
    stop: Arc<AtomicBool>,
    catalog: Mutex<Vec<Entry>>,
    receipts: Mutex<BTreeMap<String, (String, Option<Value>)>>,
}
impl Broker {
    pub(crate) fn check(&self, deadline: Instant) -> Result<()> {
        self.fence(deadline)?;
        self.authority
            .check(&self.state, std::cmp::min(deadline, self.lifetime))?;
        self.journal(|_, _| Ok(()))?;
        self.fence(deadline)
    }
    fn pins(&self) -> &SessionPins {
        &self.pins
    }
    fn fence(&self, deadline: Instant) -> Result<()> {
        if self.stop.load(Ordering::Acquire) || !self.pins.matches(self.authority.pins()) {
            return Err("mcp_scope_stopped_or_changed");
        }
        if Instant::now() >= std::cmp::min(deadline, self.lifetime) {
            return Err("mcp_scope_expired");
        }
        Ok(())
    }
    fn context(
        &self,
        store: &Store,
        in_transaction: bool,
        cut: &dyn super::authority::SqlAuthority,
    ) -> Result<AccessContext> {
        self.fence(self.lifetime)?;
        for connection in &self.connections {
            connection.validate(&self.state, store)?;
        }
        let context = cut.context(store, in_transaction)?;
        store
            .interactive_journal()
            .authorize_dispatch(&context, &self.pins().session, &self.pins().lease)
            .map_err(|_| "mcp_authority_changed")?;
        Ok(context)
    }
    fn journal<T>(
        &self,
        action: impl FnOnce(&Journal<'_>, &AccessContext) -> dunlin_session_runtime::Result<T>,
    ) -> Result<T> {
        self.journal_checked(|_, _| Ok(()), action)
    }
    fn journal_checked<T>(
        &self,
        guard: impl FnOnce(&dyn super::authority::SqlAuthority, &Store) -> Result<()>,
        action: impl FnOnce(&Journal<'_>, &AccessContext) -> dunlin_session_runtime::Result<T>,
    ) -> Result<T> {
        let cut = self.authority.sql(&self.state)?;
        let store = self.state.store.lock().map_err(|_| "store_unavailable")?;
        let context = self.context(&store, false, cut.as_ref())?;
        guard(cut.as_ref(), &store)?;
        let journal = store.interactive_journal();
        let before = journal
            .sequence(&context, &self.pins().session)
            .map_err(|_| "journal_unavailable")?;
        let result = action(&journal, &context).map_err(|_| "journal_or_authority_rejected")?;
        if journal
            .sequence(&context, &self.pins().session)
            .map_err(|_| "journal_unavailable")?
            != before
        {
            self.authority.changed(&store);
        }
        Ok(result)
    }
    pub(crate) async fn prepare(
        state: AppState,
        authority: Arc<dyn Authority>,
        connections: Vec<BoundConnection>,
        lifetime: Instant,
    ) -> Result<Arc<Self>> {
        let pins = authority.pins().clone();
        if connections.is_empty()
            || connections.len() > 8
            || connections.iter().any(|connection| {
                connection.context.user_id != pins.owner
                    || connection.context.workspace_id != pins.workspace
            })
        {
            return Err("mcp_connection_scope_changed");
        }
        let slot = state
            .mcp_oauth
            .runtime_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "mcp_runtime_capacity")?;
        let scope = Arc::new(Self {
            state,
            authority,
            pins,
            connections,
            _slot: slot,
            lifetime,
            stop: Arc::new(AtomicBool::new(false)),
            catalog: Mutex::new(Vec::new()),
            receipts: Mutex::new(BTreeMap::new()),
        });
        let preparing = scope.clone();
        tokio::task::spawn_blocking(move || {
            preparing.catalog(std::cmp::min(
                lifetime,
                Instant::now() + Duration::from_secs(30),
            ))
        })
        .await
        .map_err(|_| "mcp_catalog_unavailable")??;
        Ok(scope)
    }
    fn client<T>(
        self: &Arc<Self>,
        index: usize,
        deadline: Instant,
        extra: Option<ExtraGuard>,
        operation: impl FnOnce(
            &mut Client<'_>,
            &str,
        ) -> std::result::Result<T, dunlin_mcp_client::Error>,
    ) -> NetResult<T> {
        let _slot = self
            .state
            .mcp_oauth
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PersistenceError::Limit)?;
        self.check(deadline)
            .map_err(|_| PersistenceError::Unauthorized)?;
        let bound = self
            .connections
            .get(index)
            .ok_or(PersistenceError::Unauthorized)?;
        let flow = bound
            .provider
            .flow(self.state.mcp_oauth.secrets.as_ref())
            .map_err(|_| PersistenceError::Storage)?;
        self.check(deadline)
            .map_err(|_| PersistenceError::Unauthorized)?;
        let journal = Arc::new(RuntimeJournal {
            scope: self.clone(),
            bound: bound.clone(),
            deadline,
        });
        let credentials =
            OAuthCredentials::new(journal.clone(), self.state.mcp_oauth.secrets.clone());
        let binding = flow.credential_binding();
        let (record, bearer) = {
            let waiting_until = std::cmp::min(
                deadline,
                std::cmp::min(self.lifetime, Instant::now() + Duration::from_secs(5)),
            );
            let _writer = loop {
                if Instant::now() >= waiting_until {
                    return Err(PersistenceError::Conflict);
                }
                self.check(deadline)
                    .map_err(|_| PersistenceError::Unauthorized)?;
                let result = {
                    let store = self
                        .state
                        .store
                        .lock()
                        .map_err(|_| PersistenceError::Storage)?;
                    store.mcp_oauth_writer_lease()
                };
                match result {
                    Err(PersistenceError::Conflict) if Instant::now() < waiting_until => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    result => break result?,
                }
            };
            credentials.access_token_or_refresh(&flow, &bound.context)?
        };
        let guard = DispatchGuard::new(Arc::new({
            let (scope, journal, context, record, binding) = (
                self.clone(),
                journal.clone(),
                bound.context.clone(),
                record.clone(),
                binding.clone(),
            );
            move || {
                scope.check(deadline).is_ok()
                    && extra.as_ref().is_none_or(|check| check())
                    && journal
                        .active(&context, &binding)
                        .is_ok_and(|current| current == record)
            }
        }));
        let mut client = Client::connect_authorized_with_input_support(
            &bound.provider.endpoint,
            &bearer,
            dunlin_mcp_client::negotiated::InputSupport {
                elicitation_form: true,
                elicitation_url: true,
            },
            guard,
        )?;
        let result = operation(&mut client, &bearer);
        let _ = client.close();
        self.check(deadline)
            .map_err(|_| PersistenceError::Unauthorized)?;
        if journal.active(&bound.context, &binding)? != record {
            return Err(PersistenceError::Conflict);
        }
        result.map_err(PersistenceError::from)
    }
    fn catalog(self: &Arc<Self>, deadline: Instant) -> Result<()> {
        let mut entries = Vec::new();
        let mut aliases = BTreeSet::new();
        let mut bytes = 0usize;
        for (index, bound) in self.connections.iter().enumerate() {
            let tools = self
                .client(index, deadline, None, |client, bearer| {
                    let tools = client.tools()?;
                    for tool in &tools {
                        if serde_json::to_string(tool)
                            .map_err(|_| dunlin_mcp_client::Error::Protocol)?
                            .contains(bearer)
                        {
                            return Err(dunlin_mcp_client::Error::Protocol);
                        }
                    }
                    Ok(tools)
                })
                .map_err(|_| "mcp_catalog_unavailable")?;
            for pin in &bound.tools {
                let tool = tools
                    .iter()
                    .find(|t| t.name == pin.name)
                    .ok_or("mcp_tool_unavailable")?;
                if tool
                    .definition_digest()
                    .map_err(|_| "mcp_schema_unavailable")?
                    != pin.definition_digest
                {
                    return Err("mcp_definition_changed");
                }
                let mut hash = Sha256::new();
                hash.update(b"dunlin.mcp.alias.v1\0");
                hash.update(bound.provider.id.as_bytes());
                hash.update([0]);
                hash.update(tool.name.as_bytes());
                let alias = format!("mcp_{}", &hex::encode(hash.finalize())[..32]);
                if !aliases.insert(alias.clone()) {
                    return Err("mcp_alias_collision");
                }
                bytes += serde_json::to_vec(tool)
                    .map_err(|_| "mcp_schema_unavailable")?
                    .len();
                if bytes > 4 * 1024 * 1024 {
                    return Err("mcp_catalog_limit");
                }
                entries.push(Entry {
                    alias,
                    tool: tool.clone(),
                    connection: index,
                });
            }
        }
        entries.sort_by(|a, b| a.alias.cmp(&b.alias));
        self.check(deadline)?;
        *self.catalog.lock().map_err(|_| "mcp_scope_unavailable")? = entries;
        Ok(())
    }
    fn tool_value(&self, entry: &Entry) -> Value {
        let mut value = json!({"name":entry.alias,"description":format!("{} / {}. Execution requires approval in Dunlin.",self.connections[entry.connection].provider.id,entry.tool.name),"inputSchema":entry.tool.input_schema});
        if let Some(output) = &entry.tool.output_schema {
            value["outputSchema"] = output.clone();
        }
        value
    }
    pub(crate) fn validate_reply_limit(&self, maximum: usize) -> Result<()> {
        if !(128..=256 * 1024).contains(&maximum) {
            return Err("mcp_reply_limit");
        }
        let catalog = self.catalog.lock().map_err(|_| "mcp_scope_unavailable")?;
        for entry in catalog.iter() {
            if serde_json::to_vec(&self.tool_value(entry))
                .map_err(|_| "mcp_schema_unavailable")?
                .len()
                > maximum.saturating_sub(128)
            {
                return Err("mcp_catalog_limit");
            }
        }
        Ok(())
    }
    fn tools(&self, params: &Value, maximum: usize) -> Result<Value> {
        if params
            .as_object()
            .is_none_or(|v| v.keys().any(|k| k != "cursor"))
        {
            return Err("mcp_invalid_parameters");
        }
        let start = match params.get("cursor") {
            None => 0,
            Some(Value::String(s)) if s.len() <= 2 => {
                let n = s.parse::<usize>().map_err(|_| "mcp_invalid_cursor")?;
                if n.to_string() != *s {
                    return Err("mcp_invalid_cursor");
                }
                n
            }
            _ => return Err("mcp_invalid_cursor"),
        };
        let catalog = self.catalog.lock().map_err(|_| "mcp_scope_unavailable")?;
        if start > catalog.len() {
            return Err("mcp_invalid_cursor");
        }
        let mut tools = Vec::new();
        let mut bytes = 0;
        let mut end = start;
        for entry in catalog.iter().skip(start) {
            let value = self.tool_value(entry);
            let size = serde_json::to_vec(&value)
                .map_err(|_| "mcp_schema_unavailable")?
                .len();
            if bytes + size > (192 * 1024).min(maximum.saturating_sub(128)) {
                break;
            }
            bytes += size;
            tools.push(value);
            end += 1;
        }
        if start < catalog.len() && end == start {
            return Err("mcp_catalog_limit");
        }
        let mut result = json!({"tools":tools});
        if end < catalog.len() {
            result["nextCursor"] = json!(end.to_string());
        }
        Ok(result)
    }
    fn call(
        self: &Arc<Self>,
        request: &str,
        params: &Value,
        caller_deadline: Instant,
    ) -> Result<Value> {
        if params.as_object().is_none_or(|v| {
            v.keys()
                .any(|k| !matches!(k.as_str(), "name" | "arguments"))
        }) {
            return Err("mcp_invalid_parameters");
        }
        let entry = self
            .catalog
            .lock()
            .map_err(|_| "mcp_scope_unavailable")?
            .iter()
            .find(|entry| Some(entry.alias.as_str()) == params["name"].as_str())
            .cloned()
            .ok_or("mcp_tool_not_allowed")?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let bound = &self.connections[entry.connection];
        let invocation = Arc::new(McpInvocation {
            provider_id: bound.provider.id.clone(),
            connection_id: bound.context.connection_id.clone(),
            connection_revision: bound.context.connection_revision,
            tool_name: entry.tool.name.clone(),
            definition_digest: entry
                .tool
                .definition_digest()
                .map_err(|_| "mcp_schema_unavailable")?,
            arguments: arguments.clone(),
        });
        invocation.validate().map_err(|_| "mcp_arguments_invalid")?;
        dunlin_mcp_client::schema::Schema::compile(&entry.tool.input_schema)
            .and_then(|s| s.validate(&arguments))
            .map_err(|_| "mcp_arguments_invalid")?;
        let active = self.authority.turn()?;
        let turn = active.id;
        let deadline = self.lifetime.min(active.deadline).min(caller_deadline);
        self.check(deadline)?;
        // A fresh validated catalog is required before presenting the action.
        self.client(entry.connection, deadline, None, |client, _| {
            if !client.tools()?.contains(&entry.tool) {
                return Err(dunlin_mcp_client::Error::Unsupported);
            }
            Ok(())
        })
        .map_err(|_| "mcp_definition_changed")?;
        let approval = self.journal(|j, a| {
            j.register_mcp_permission(
                a,
                &self.pins().session,
                &self.pins().lease,
                &turn,
                McpPermissionChallenge {
                    request_id: request.into(),
                    invocation: McpInvocation {
                        provider_id: invocation.provider_id.clone(),
                        connection_id: invocation.connection_id.clone(),
                        connection_revision: invocation.connection_revision,
                        tool_name: invocation.tool_name.clone(),
                        definition_digest: invocation.definition_digest.clone(),
                        arguments: arguments.clone(),
                    },
                    ttl_seconds: deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs()
                        .clamp(1, 120),
                    starter_device: self.pins().starter_device.clone(),
                    binding_revision: self.pins().binding_revision,
                    harness_digest: self.pins().profile_digest.clone(),
                },
            )
        })?;
        loop {
            self.check(deadline)?;
            match self.journal(|j, a| {
                j.mcp_permission_status(
                    a,
                    &self.pins().session,
                    &self.pins().lease,
                    &approval,
                    &invocation,
                )
            })? {
                PermissionStatus::Pending => std::thread::sleep(Duration::from_millis(100)),
                PermissionStatus::Decided(_) => break,
                PermissionStatus::Invalid => return Err("mcp_permission_unavailable"),
            }
        }
        let permit = self.journal(|j, a| {
            j.consume_mcp_permission(
                a,
                &self.pins().session,
                &self.pins().lease,
                approval,
                &invocation,
            )
        })?;
        let Some(permit) = permit else {
            return Ok(
                json!({"isError":true,"content":[{"type":"text","text":"The user denied this MCP call."}]}),
            );
        };
        let permit = Arc::new(permit);
        let devices = Arc::new(Mutex::new(Vec::new()));
        let check: ExtraGuard = Arc::new({
            let (scope, permit, invocation) = (self.clone(), permit.clone(), invocation.clone());
            let devices = devices.clone();
            move || {
                scope.permit(&permit, &invocation).is_ok() && scope.input_devices(&devices).is_ok()
            }
        });
        let result = self.client(entry.connection, deadline, Some(check), |client, bearer| {
            if !client.tools()?.contains(&entry.tool) {
                return Err(dunlin_mcp_client::Error::Unsupported);
            }
            let value = input::Call {
                broker: self,
                permit: &permit,
                invocation: &invocation,
                entry: &entry,
                deadline,
                bearer,
                devices: &devices,
            }
            .run(client, arguments)?;
            if serde_json::to_string(&value)
                .map_err(|_| dunlin_mcp_client::Error::Protocol)?
                .contains(bearer)
            {
                return Err(dunlin_mcp_client::Error::Protocol);
            }
            Ok(value)
        });
        let permit = Arc::try_unwrap(permit).map_err(|_| "mcp_dispatch_unconfirmed")?;
        let (outcome, response) = match result {
            Ok(value) => (McpCompletion::Complete(value.clone()), Ok(value)),
            Err(PersistenceError::OAuth(dunlin_mcp_client::Error::ToolFailed)) => (
                McpCompletion::Failed,
                Ok(
                    json!({"isError":true,"content":[{"type":"text","text":"The MCP tool reported failure. This call was not retried."}]}),
                ),
            ),
            Err(_) => (McpCompletion::Unknown, Err("mcp_outcome_unknown")),
        };
        let unknown = response.is_err();
        let input_devices = devices.lock().map_err(|_| "mcp_input_unavailable")?.clone();
        let completed = self.journal_checked(
            |cut, store| {
                for device in &input_devices {
                    cut.permission_device(store, &self.pins().owner, device)?;
                }
                Ok(())
            },
            |j, a| {
                j.finish_mcp_dispatch(
                    a,
                    &self.pins().session,
                    &self.pins().lease,
                    permit,
                    &invocation,
                    outcome,
                )
            },
        );
        if unknown || completed.is_err() {
            self.authority.interrupt();
        }
        completed?;
        response
    }
    fn permit(&self, permit: &McpDispatchPermit, invocation: &McpInvocation) -> Result<()> {
        let decision = self.journal(|j, a| {
            j.validate_mcp_dispatch(
                a,
                &self.pins().session,
                &self.pins().lease,
                permit,
                invocation,
            )
        })?;
        let cut = self.authority.sql(&self.state)?;
        let store = self.state.store.lock().map_err(|_| "store_unavailable")?;
        self.context(&store, false, cut.as_ref())?;
        cut.permission_device(&store, &self.pins().owner, &decision)
    }
    pub(crate) fn request(
        self: &Arc<Self>,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.request_bounded(id, method, params, self.lifetime, 256 * 1024)
    }
    pub(crate) fn request_bounded(
        self: &Arc<Self>,
        id: &str,
        method: &str,
        params: Value,
        deadline: Instant,
        maximum: usize,
    ) -> Result<Value> {
        if !(128..=256 * 1024).contains(&maximum) {
            return Err("mcp_reply_limit");
        }
        self.check(deadline)?;
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("mcp_request_invalid");
        }
        if method == "tools/list" {
            let result = self.tools(&params, maximum)?;
            self.check(deadline)?;
            return Ok(result);
        }
        if method != "tools/call" {
            return Err("mcp_method_unavailable");
        }
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&params).map_err(|_| "mcp_request_invalid")?,
        ));
        {
            let mut receipts = self.receipts.lock().map_err(|_| "mcp_scope_unavailable")?;
            if let Some((original, result)) = receipts.get(id) {
                if *original != fingerprint {
                    return Err("mcp_request_changed");
                }
                let value = result.clone().ok_or("mcp_request_already_used")?;
                if serde_json::to_vec(&value)
                    .map_err(|_| "mcp_reply_limit")?
                    .len()
                    > maximum
                {
                    return Err("mcp_reply_limit");
                }
                return Ok(value);
            }
            if receipts.len() >= 256 {
                return Err("mcp_request_limit");
            }
            receipts.insert(id.into(), (fingerprint.clone(), None));
        }
        let result = self.call(id, &params, deadline);
        self.check(deadline)?;
        if let Ok(value) = &result {
            if serde_json::to_vec(value)
                .map_err(|_| "mcp_reply_limit")?
                .len()
                > maximum
            {
                self.authority.interrupt();
                return Err("mcp_reply_limit");
            }
            self.receipts
                .lock()
                .map_err(|_| "mcp_scope_unavailable")?
                .insert(id.into(), (fingerprint, Some(value.clone())));
        }
        result
    }
}

struct RuntimeJournal {
    scope: Arc<Broker>,
    bound: BoundConnection,
    deadline: Instant,
}
impl RuntimeJournal {
    fn scoped<T>(
        &self,
        context: &AuthContext,
        operation: impl FnOnce(&dyn CredentialJournal) -> NetResult<T>,
    ) -> NetResult<T> {
        if *context != self.bound.context
            || self.scope.stop.load(Ordering::Acquire)
            || Instant::now() >= self.deadline
        {
            return Err(PersistenceError::Unauthorized);
        }
        let cut = self
            .scope
            .authority
            .sql(&self.scope.state)
            .map_err(|_| PersistenceError::Unauthorized)?;
        let store = self
            .scope
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?;
        let check = |store: &Store, c: &AuthContext| {
            if *c != self.bound.context
                || self.scope.stop.load(Ordering::Acquire)
                || Instant::now() >= self.deadline
            {
                return Err(PersistenceError::Unauthorized);
            }
            self.scope
                .context(store, true, cut.as_ref())
                .map(|_| ())
                .map_err(|_| PersistenceError::Unauthorized)
        };
        operation(&store.mcp_oauth_journal(&check))
    }
}
impl CredentialJournal for RuntimeJournal {
    fn active(&self, c: &AuthContext, b: &str) -> NetResult<CredentialRecord> {
        self.scoped(c, |j| j.active(c, b))
    }
    fn begin_install(&self, _: &AuthContext, _: &str) -> NetResult<CredentialIntent> {
        Err(PersistenceError::Unauthorized)
    }
    fn begin_refresh(
        &self,
        c: &AuthContext,
        b: &str,
        r: &CredentialRecord,
    ) -> NetResult<CredentialIntent> {
        self.scoped(c, |j| j.begin_refresh(c, b, r))
    }
    fn prepare_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> NetResult<()> {
        self.scoped(c, |j| j.prepare_write(c, b, i, h))
    }
    fn claim_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> NetResult<()> {
        self.scoped(c, |j| j.claim_write(c, b, i, h))
    }
    fn acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> NetResult<()> {
        if *c != self.bound.context {
            return Err(PersistenceError::Unauthorized);
        }
        self.scope
            .state
            .store
            .lock()
            .map_err(|_| PersistenceError::Storage)?
            .mcp_oauth_acknowledge_write(c, b, i, h)
    }
    fn commit(&self, c: &AuthContext, b: &str, i: &CredentialIntent, h: &str) -> NetResult<()> {
        self.scoped(c, |j| j.commit(c, b, i, h))
    }
    fn disable(&self, _: &AuthContext) -> NetResult<u64> {
        Err(PersistenceError::Unauthorized)
    }
}

pub(in crate::interactive) struct Peer {
    pub configs: Vec<Value>,
    pub paths: Vec<PathBuf>,
    stop: Arc<AtomicBool>,
    #[cfg(unix)]
    _directory: Option<tempfile::TempDir>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Peer {
    pub async fn start(state: &AppState, task: &Task, lifetime: Instant) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        if task.mcp.is_empty() {
            return Ok(Self {
                configs: vec![],
                paths: vec![],
                stop,
                #[cfg(unix)]
                _directory: None,
            });
        }
        #[cfg(not(unix))]
        {
            return Err("mcp_runtime_platform_unsupported");
        }
        #[cfg(unix)]
        {
            use std::os::unix::{fs::PermissionsExt, net::UnixListener};
            let scope = Broker::prepare(
                state.clone(),
                Arc::new(super::authority::Local::new(task)?),
                task.mcp.clone(),
                lifetime,
            )
            .await?;
            let stop = scope.stop.clone();
            let directory = tempfile::Builder::new()
                .prefix("dunlin-mcp-")
                .tempdir_in("/tmp")
                .map_err(|_| "mcp_socket_unavailable")?;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "mcp_socket_unavailable")?;
            let socket = directory.path().join("rpc.sock");
            let listener = UnixListener::bind(&socket).map_err(|_| "mcp_socket_unavailable")?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "mcp_socket_unavailable")?;
            listener
                .set_nonblocking(true)
                .map_err(|_| "mcp_socket_unavailable")?;
            let token = crate::auth::random_token();
            let token_hash = crate::auth::hash_token(&token);
            let executable = proxy_executable()?;
            let configs = vec![
                json!({"name":"dunlin-session-tools","command":executable,"args":["--mcp-tools"],"env":[{"name":"DUNLIN_MCP_SOCKET","value":socket},{"name":"DUNLIN_MCP_TOKEN","value":token}]}),
            ];
            let paths = vec![socket, executable];
            std::thread::Builder::new().name("dunlin-mcp-session".into()).spawn(move||{
                while !scope.stop.load(Ordering::Acquire) && Instant::now()<lifetime {
                    let(mut stream,_)=match listener.accept(){Ok(value)=>value,Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(20));continue;},Err(_)=>break};
                    let _=stream.set_read_timeout(Some(Duration::from_secs(2)));let _=stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let result=(||->Result<Value>{
                        let value=super::proxy::read_frame(&mut stream).map_err(|_|"mcp_frame_invalid")?;
                        let request:super::proxy::Request=serde_json::from_value(value).map_err(|_|"mcp_frame_invalid")?;
                        if request.token.len()>128 || crate::auth::hash_token(&request.token)!=token_hash {return Err("mcp_scope_unauthorized");}
                        scope.request(&request.request_id,&request.method,request.params)
                    })();
                    let response=match result{Ok(value)=>json!({"result":value}),Err(_)=>json!({"error":"MCP request rejected or outcome unavailable. It was not retried."})};
                    let mut packet=Vec::new();
                    if super::proxy::write_frame(&mut packet,&response).is_err(){continue;}
                    if scope.check(lifetime).is_err(){
                        packet.clear();
                        if super::proxy::write_frame(&mut packet,&json!({"error":"MCP session authority is no longer available."})).is_err(){continue;}
                    }
                    use std::io::Write;
                    let _=stream.write_all(&packet).and_then(|_|stream.flush());
                }
            }).map_err(|_|"mcp_runtime_capacity")?;
            Ok(Self {
                configs,
                paths,
                stop,
                _directory: Some(directory),
            })
        }
    }
}
#[cfg(unix)]
pub(crate) fn proxy_executable() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("DUNLIN_RELAY_E2E_BIN") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().map_err(|_| "mcp_proxy_unavailable")
}

#[cfg(test)]
mod tests;
