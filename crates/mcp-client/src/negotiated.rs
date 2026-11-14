//! Modern-first Streamable HTTP with an explicit MCP 2025 adapter.
//! Discovery and catalog metadata are untrusted observations, not grants.
use crate::{Error, MAX_BYTES, MAX_TOOLS, Tool, metadata};
use dunlin_http::{Agent, AgentBuilder, Response};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};
mod cache;
mod continuation;
pub use continuation::{
    ElicitationAction, ElicitationResponse, FormElicitation, InputSupport, Outcome, Pending,
    UrlElicitation,
};
mod subscriptions;
pub use subscriptions::{Change, Subscription, SubscriptionFilter};

pub struct Client<'a> {
    transport: Option<Transport<'a>>,
}

enum Transport<'a> {
    Modern(Modern<'a>),
    Legacy(crate::Client<'a>),
}

struct CatalogTool {
    headers: metadata::ToolHeaders,
    validation: crate::schema::ToolValidation,
}

struct Modern<'a> {
    guard: Option<crate::DispatchGuard>,
    endpoint: String,
    bearer: &'a str,
    agent: Agent,
    _cancellation: Option<dunlin_http::Cancellation>,
    started: Instant,
    next_id: u64,
    discovery: Value,
    catalog: HashMap<String, CatalogTool>,
    input_support: InputSupport,
    identity: Arc<continuation::Identity>,
    signals: Arc<subscriptions::Signals>,
    catalog_epoch: u64,
    cache: cache::Cache,
    unauthorized: bool,
}
impl Drop for Modern<'_> {
    fn drop(&mut self) {
        self.signals.cancel();
        self.agent.shutdown();
    }
}

impl<'a> Client<'a> {
    pub fn connect(endpoint: &str, bearer: &'a str) -> Result<Self, Error> {
        Self::connect_with_input_support(endpoint, bearer, InputSupport::default())
    }

    /// Enable only input modes actually supported by the integration's user
    /// interaction layer. This never opens URLs or approves input requests.
    pub fn connect_with_input_support(
        endpoint: &str,
        bearer: &'a str,
        input_support: InputSupport,
    ) -> Result<Self, Error> {
        Self::connect_inner(endpoint, bearer, input_support, None)
    }

    pub fn connect_authorized(
        endpoint: &str,
        bearer: &'a str,
        guard: crate::DispatchGuard,
    ) -> Result<Self, Error> {
        Self::connect_authorized_with_input_support(
            endpoint,
            bearer,
            InputSupport::default(),
            guard,
        )
    }

    pub fn connect_authorized_with_input_support(
        endpoint: &str,
        bearer: &'a str,
        input_support: InputSupport,
        guard: crate::DispatchGuard,
    ) -> Result<Self, Error> {
        Self::connect_inner(endpoint, bearer, input_support, Some(guard))
    }

    fn connect_inner(
        endpoint: &str,
        bearer: &'a str,
        input_support: InputSupport,
        guard: Option<crate::DispatchGuard>,
    ) -> Result<Self, Error> {
        crate::validate_endpoint(endpoint)?;
        crate::validate_credential(bearer)?;
        let started = Instant::now();
        let agent = AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .build();
        crate::authorize(guard.as_ref())?;
        let signals = Arc::<subscriptions::Signals>::default();
        let checked_signals = signals.clone();
        let checked_guard = guard.clone();
        let cancellation = Some(
            agent
                .watch(move || {
                    !checked_signals.cancelled() && crate::authorize(checked_guard.as_ref()).is_ok()
                })
                .map_err(|_| Error::Transport)?,
        );
        let mut modern = Modern {
            guard: guard.clone(),
            endpoint: endpoint.into(),
            bearer,
            agent,
            _cancellation: cancellation,
            started,
            next_id: 2,
            discovery: Value::Null,
            catalog: HashMap::new(),
            input_support,
            identity: Arc::default(),
            signals,
            catalog_epoch: 0,
            cache: cache::Cache::default(),
            unauthorized: false,
        };
        // Era detection is read-only and precedes any potentially mutating call.
        let probe = metadata::request_with_capabilities(
            1,
            "server/discover",
            json!({}),
            input_support.metadata(),
        )?;
        let response = modern.post(&probe, None)?;
        let transport = match response.status() {
            200 => {
                let received = Instant::now();
                let discovery = crate::decode_response(response, 1);
                crate::authorize(modern.guard.as_ref())?;
                let discovery = discovery?;
                validate_discovery(&discovery)?;
                if let Some(key) = cache::Key::new("server/discover", &json!({}))? {
                    modern.cache.put(
                        key,
                        &discovery,
                        modern.signals.cache_epoch("server/discover"),
                        received,
                    )?;
                }
                modern.discovery = discovery;
                Transport::Modern(modern)
            }
            400 => {
                let body = read_json(response);
                crate::authorize(modern.guard.as_ref())?;
                let body = body?;
                if let Some(body) = body
                    && matches!(body["error"]["code"].as_i64(), Some(-32022..=-32020))
                {
                    // A recognized modern error is never a downgrade signal.
                    return match crate::rpc_result(body, 1) {
                        Err(error) => Err(error),
                        Ok(_) => Err(Error::Protocol),
                    };
                }
                Transport::Legacy(crate::Client::connect_since(
                    endpoint, bearer, started, guard,
                )?)
            }
            401 | 403 => return Err(Error::Unauthorized),
            status => return Err(http_error(response, 1, status)),
        };
        Ok(Self {
            transport: Some(transport),
        })
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.authorized().ok()?;
        match self.transport.as_ref()? {
            Transport::Modern(_) => Some(metadata::VERSION),
            Transport::Legacy(client) => Some(&client.version),
        }
    }

    /// Self-reported display metadata; must not be used for authorization.
    pub fn discovery(&self) -> Option<&Value> {
        self.authorized().ok()?;
        match self.transport.as_ref()? {
            Transport::Modern(client) => Some(&client.discovery),
            Transport::Legacy(_) => None,
        }
    }

    pub fn tools(&mut self) -> Result<Vec<Tool>, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.tools(),
            Transport::Legacy(client) => client.tools(),
        })
    }

    // Schema compilation/validation may run after the last HTTP check. Fence
    // the fully processed result before exposing it to the integration.
    fn checked<T>(
        &mut self,
        operation: impl FnOnce(&mut Transport<'a>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.authorized()?;
        let result = operation(self.transport.as_mut().ok_or(Error::Protocol)?)?;
        self.authorized()?;
        Ok(result)
    }

    fn authorized(&self) -> Result<(), Error> {
        match self.transport.as_ref().ok_or(Error::Protocol)? {
            Transport::Modern(client) => crate::authorize(client.guard.as_ref()),
            Transport::Legacy(client) => crate::authorize(client.guard.as_ref()),
        }
    }

    /// Force a catalog read instead of using fresh cached pages.
    pub fn refresh_tools(&mut self) -> Result<Vec<Tool>, Error> {
        if let Some(Transport::Modern(client)) = self.transport.as_mut() {
            client.cache.invalidate("tools/list");
        }
        self.tools()
    }

    pub fn discover(&mut self) -> Result<Value, Error> {
        self.checked(|transport| match transport {
            Transport::Legacy(_) => Err(Error::Unsupported),
            Transport::Modern(client) => {
                let result = client
                    .request("server/discover", json!({}), Vec::new())
                    .and_then(|value| {
                        validate_discovery(&value)?;
                        Ok(value)
                    });
                match result {
                    Ok(value) => {
                        client.discovery = value.clone();
                        Ok(value)
                    }
                    Err(error) => {
                        client.discovery = Value::Null;
                        client.catalog.clear();
                        client.cache.clear();
                        Err(error)
                    }
                }
            }
        })
    }

    pub fn call(&mut self, tool: &Tool, arguments: Value) -> Result<Value, Error> {
        match self.begin_call(tool, arguments)? {
            Outcome::Complete(value) => Ok(value),
            Outcome::InputRequired(_) => Err(Error::Unsupported),
        }
    }

    pub fn begin_call(&mut self, tool: &Tool, arguments: Value) -> Result<Outcome, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.begin(continuation::Operation::Tool {
                tool: tool.clone(),
                arguments,
            }),
            Transport::Legacy(client) => {
                client
                    .catalog
                    .get(&tool.name)
                    .ok_or(Error::Unsupported)?
                    .arguments(tool, &arguments)?;
                client.call(tool, arguments).map(Outcome::Complete)
            }
        })
    }

    pub fn get_prompt(&mut self, name: &str, arguments: Value) -> Result<Outcome, Error> {
        if name.is_empty()
            || name.len() > 256
            || !arguments
                .as_object()
                .is_some_and(|args| args.values().all(Value::is_string))
        {
            return Err(Error::Protocol);
        }
        self.begin_other(continuation::Operation::Prompt {
            name: name.into(),
            arguments,
        })
    }

    pub fn read_resource(&mut self, uri: &str) -> Result<Outcome, Error> {
        if uri.len() > 4096 || url::Url::parse(uri).is_err() {
            return Err(Error::Protocol);
        }
        self.begin_other(continuation::Operation::Resource { uri: uri.into() })
    }

    fn begin_other(&mut self, operation: continuation::Operation) -> Result<Outcome, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.begin(operation),
            Transport::Legacy(client) => {
                let (method, params) = operation.request();
                let result = client.request(method, params)?;
                if result["resultType"] == "input_required" {
                    return Err(Error::Unsupported);
                }
                if !result.is_object() {
                    return Err(Error::Protocol);
                }
                Ok(Outcome::Complete(result))
            }
        })
    }

    /// Consume a pending round once, preserving its operation, credentials and
    /// opaque server state. Callers explicitly collect each user's decision.
    pub fn continue_request(
        &mut self,
        pending: Pending,
        responses: std::collections::BTreeMap<String, ElicitationAction>,
    ) -> Result<Outcome, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.continue_request(pending, responses),
            Transport::Legacy(_) => Err(Error::Unsupported),
        })
    }

    pub fn continue_with_responses(
        &mut self,
        pending: Pending,
        responses: std::collections::BTreeMap<String, ElicitationResponse>,
    ) -> Result<Outcome, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.continue_with_responses(pending, responses),
            Transport::Legacy(_) => Err(Error::Unsupported),
        })
    }

    pub fn close(&mut self) -> Result<(), Error> {
        match self.transport.take() {
            Some(Transport::Legacy(mut client)) => client.close(),
            // Modern MCP has no protocol session and sends no DELETE.
            Some(Transport::Modern(client)) => {
                client.signals.cancel();
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Open an independently owned, bounded notification stream. Drive it on a
    /// reader thread if calls must proceed concurrently. No reconnect is added.
    pub fn listen(&mut self, filter: SubscriptionFilter) -> Result<Subscription, Error> {
        self.checked(|transport| match transport {
            Transport::Modern(client) => client.listen(filter),
            Transport::Legacy(_) => Err(Error::Unsupported),
        })
    }
}

impl Modern<'_> {
    fn reject_auth(&mut self) -> Error {
        self.unauthorized = true;
        self.signals.cancel();
        self.cache.clear();
        self.catalog.clear();
        Error::Unauthorized
    }
    fn post(
        &self,
        prepared: &metadata::PreparedRequest,
        deadline: Option<Instant>,
    ) -> Result<Response, Error> {
        crate::authorize(self.guard.as_ref())?;
        if self.unauthorized {
            return Err(Error::Unauthorized);
        }
        if self.signals.cancelled() {
            return Err(Error::Protocol);
        }
        let mut remaining = Duration::from_secs(90).saturating_sub(self.started.elapsed());
        if let Some(deadline) = deadline {
            remaining = remaining.min(deadline.saturating_duration_since(Instant::now()));
        }
        if remaining.is_zero() {
            return Err(Error::Transport);
        }
        let bytes = serde_json::to_vec(&prepared.body).map_err(|_| Error::Protocol)?;
        if bytes.len() > MAX_BYTES
            || prepared
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 16 * 1024
        {
            return Err(Error::Limit);
        }
        let mut request = self
            .agent
            .post(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.bearer))
            .set("Accept", "application/json, text/event-stream")
            .set("Content-Type", "application/json")
            .timeout(remaining.min(Duration::from_secs(20)));
        for (name, value) in &prepared.headers {
            request = request.set(name, value);
        }
        let response = request.send_bytes(&bytes);
        crate::authorize(self.guard.as_ref())?;
        if self.signals.cancelled() {
            return Err(Error::Protocol);
        }
        let response = match response {
            Ok(response) => Ok(response),
            Err(dunlin_http::Error::Status(_, response)) => Ok(*response),
            Err(_) => Err(Error::Transport),
        }?;
        crate::authorize(self.guard.as_ref())?;
        Ok(response)
    }

    fn request(
        &mut self,
        method: &str,
        params: Value,
        headers: Vec<(String, String)>,
    ) -> Result<Value, Error> {
        self.request_until(method, params, headers, None)
    }

    fn request_until(
        &mut self,
        method: &str,
        params: Value,
        headers: Vec<(String, String)>,
        deadline: Option<Instant>,
    ) -> Result<Value, Error> {
        crate::authorize(self.guard.as_ref())?;
        if self.unauthorized {
            return Err(Error::Unauthorized);
        }
        if self.signals.cancelled() {
            return Err(Error::Protocol);
        }
        if self.started.elapsed() >= Duration::from_secs(90)
            || deadline.is_some_and(|at| at <= Instant::now())
        {
            return Err(Error::Transport);
        }
        let key = cache::Key::new(method, &params)?;
        let epoch = self.signals.cache_epoch(method);
        if let Some(key) = &key
            && let Some(value) = self.cache.get(key, epoch)
        {
            // A reader thread may invalidate the entry while its JSON value is
            // being cloned. Fence the completed lookup before exposing it.
            if self.signals.cancelled() {
                return Err(Error::Protocol);
            }
            if self.started.elapsed() >= Duration::from_secs(90)
                || deadline.is_some_and(|at| at <= Instant::now())
            {
                return Err(Error::Transport);
            }
            if self.signals.cache_epoch(method) == epoch {
                crate::authorize(self.guard.as_ref())?;
                return Ok(value);
            }
            self.cache.invalidate(method);
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(Error::Limit)?;
        let mut prepared =
            metadata::request_with_capabilities(id, method, params, self.input_support.metadata())?;
        prepared.headers.extend(headers);
        let response = self.post(&prepared, deadline)?;
        let received = Instant::now();
        let result = match response.status() {
            200 => crate::decode_response(response, id),
            401 | 403 => Err(self.reject_auth()),
            status => Err(http_error(response, id, status)),
        };
        crate::authorize(self.guard.as_ref())?;
        match result {
            Ok(value) => {
                crate::authorize(self.guard.as_ref())?;
                if let Some(key) = key
                    && self.signals.cache_epoch(method) == epoch
                {
                    self.cache.put(key, &value, epoch, received)?;
                }
                Ok(value)
            }
            Err(error) => {
                self.cache.invalidate(method);
                if method == "tools/call" && matches!(error, Error::Rpc(-32601 | -32602 | -32020)) {
                    self.cache.invalidate("tools/list");
                    self.catalog.clear();
                }
                Err(error)
            }
        }
    }

    fn tools(&mut self) -> Result<Vec<Tool>, Error> {
        // A failed refresh must not leave a formerly usable catalog behind.
        self.catalog.clear();
        let result = self.collect_tools();
        if result.is_err() {
            self.cache.invalidate("tools/list");
        }
        result
    }

    fn collect_tools(&mut self) -> Result<Vec<Tool>, Error> {
        let epoch = self.signals.tool_epoch();
        if !self.discovery["capabilities"]["tools"].is_object() {
            return Err(Error::Unsupported);
        }
        let mut catalog = HashMap::new();
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        let mut total = 0usize;
        let mut page_scope = None;
        let mut budget = crate::schema::CatalogBudget::default();
        for _ in 0..16 {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({"cursor":cursor}))
                .unwrap_or(json!({}));
            let result = self.request("tools/list", params, Vec::new())?;
            complete(&result)?;
            if let Some(scope) = cache::scope(&result)? {
                if page_scope.is_some_and(|previous| previous != scope) {
                    return Err(Error::Protocol);
                }
                page_scope = Some(scope);
            }
            let page: Vec<Tool> =
                serde_json::from_value(result["tools"].clone()).map_err(|_| Error::Protocol)?;
            total = total.checked_add(page.len()).ok_or(Error::Limit)?;
            if total > MAX_TOOLS {
                return Err(Error::Limit);
            }
            for tool in page {
                if tool.name.is_empty() || tool.name.len() > 256 || !names.insert(tool.name.clone())
                {
                    return Err(Error::Protocol);
                }
                let headers = match metadata::ToolHeaders::from_schema(&tool.input_schema) {
                    Ok(headers) => headers,
                    Err(Error::Protocol) => continue,
                    Err(error) => return Err(error),
                };
                let validation = match crate::schema::ToolValidation::compile(&tool) {
                    Ok(validation) => validation,
                    Err(Error::Schema) => continue,
                    Err(error) => return Err(error),
                };
                budget.admit(&validation)?;
                catalog.insert(
                    tool.name.clone(),
                    CatalogTool {
                        headers,
                        validation,
                    },
                );
                tools.push(tool);
            }
            match result.get("nextCursor").filter(|v| !v.is_null()) {
                None => {
                    if self.signals.tool_epoch() != epoch {
                        return Err(Error::Protocol);
                    }
                    self.catalog = catalog;
                    self.catalog_epoch = epoch;
                    return Ok(tools);
                }
                Some(value) => {
                    let next = value
                        .as_str()
                        .filter(|v| !v.is_empty() && v.len() <= 4096)
                        .ok_or(Error::Protocol)?;
                    if !cursors.insert(next.to_owned()) {
                        return Err(Error::Protocol);
                    }
                    cursor = Some(next.to_owned());
                }
            }
        }
        Err(Error::Limit)
    }

    fn tool_headers(&self, tool: &Tool, arguments: &Value) -> Result<Vec<(String, String)>, Error> {
        if self.signals.cancelled() || self.catalog_epoch != self.signals.tool_epoch() {
            return Err(Error::Unsupported);
        }
        let advertised = self.catalog.get(&tool.name).ok_or(Error::Unsupported)?;
        advertised.validation.arguments(tool, arguments)?;
        advertised.headers.project(arguments)
    }
}

fn complete(result: &Value) -> Result<(), Error> {
    if result.is_object() && result["resultType"] == "complete" {
        Ok(())
    } else {
        Err(Error::Protocol)
    }
}

fn validate_discovery(result: &Value) -> Result<(), Error> {
    complete(result)?;
    let versions = result["supportedVersions"]
        .as_array()
        .ok_or(Error::Protocol)?;
    if versions.is_empty()
        || versions.len() > 16
        || versions
            .iter()
            .any(|v| v.as_str().is_none_or(|s| s.len() > 32))
        || !result["capabilities"].is_object()
    {
        return Err(Error::Protocol);
    }
    if !versions.iter().any(|v| v == metadata::VERSION) {
        return Err(Error::Unsupported);
    }
    Ok(())
}

fn read_json(response: Response) -> Result<Option<Value>, Error> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Transport)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    Ok(serde_json::from_slice(&bytes).ok())
}

fn http_error(response: Response, id: u64, status: u16) -> Error {
    if matches!(status, 400 | 404 | 409 | 500) {
        match read_json(response) {
            Ok(Some(body)) if body.get("error").is_some() => {
                return crate::rpc_result(body, id).err().unwrap_or(Error::Protocol);
            }
            Err(error) => return error,
            _ => {}
        }
    }
    Error::Http(status)
}

#[cfg(test)]
mod tests;
