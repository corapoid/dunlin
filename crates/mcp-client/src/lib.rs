//! Host-owned MCP calls, independent of an LLM, harness, GPUI or credentials
//! store. Streamable HTTP only; no subprocesses, automatic retries or redirects.
use dunlin_http::{Agent, AgentBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read},
    time::{Duration, Instant},
};

mod authorization;
pub mod linear;
pub mod metadata;
pub mod negotiated;
pub mod oauth;
pub mod schema;
pub use authorization::DispatchGuard;
use authorization::authorize;

const VERSION: &str = "2025-06-18";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOOLS: usize = 512;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(
        "Invalid MCP endpoint; use HTTPS or loopback HTTP without credentials, query or fragment"
    )]
    Endpoint,
    #[error("Invalid MCP credential")]
    Credential,
    #[error(
        "MCP connection failed; the requested operation may have completed. It was not retried"
    )]
    Transport,
    #[error("MCP authentication failed; reconnect the integration")]
    Unauthorized,
    #[error("MCP session expired; reconnect before issuing another operation")]
    SessionExpired,
    #[error("MCP HTTP request failed ({0}); it was not retried")]
    Http(u16),
    #[error("Invalid or unsupported MCP response")]
    Protocol,
    #[error("MCP response or catalog exceeds the supported limit")]
    Limit,
    #[error("MCP server rejected the request (code {0})")]
    Rpc(i64),
    #[error("MCP tool reported an error; inspect the integration configuration")]
    ToolFailed,
    #[error("The MCP server does not advertise the required tool or input schema")]
    Unsupported,
    #[error("Invalid or unsupported MCP JSON Schema")]
    Schema,
    #[error("MCP input does not match the requested schema")]
    Arguments,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(
        default,
        rename = "outputSchema",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "schema::optional_output"
    )]
    pub output_schema: Option<Value>,
}

impl Tool {
    /// Pin the complete admitted definition, including both schemas. No server
    /// annotations or metadata can turn this observation into a tool grant.
    pub fn definition_digest(&self) -> Result<String, Error> {
        use sha2::{Digest, Sha256};
        if self.name.is_empty() || self.name.len() > 256 || self.name.chars().any(char::is_control)
        {
            return Err(Error::Protocol);
        }
        schema::ToolValidation::compile(self)?;
        let mut value = serde_json::to_value(self).map_err(|_| Error::Schema)?;
        value.sort_all_objects();
        let encoded = serde_json::to_vec(&value).map_err(|_| Error::Schema)?;
        let mut hash = Sha256::new();
        hash.update(b"dunlin.mcp.tool.v1\0");
        hash.update(encoded);
        Ok(hex::encode(hash.finalize()))
    }
    pub fn supports(&self, fields: &[&str]) -> bool {
        if self
            .input_schema
            .get("type")
            .is_some_and(|kind| kind != "object")
            || self
                .input_schema
                .get("required")
                .is_some_and(|required| !required.is_array())
        {
            return false;
        }
        let Some(properties) = self.input_schema["properties"].as_object() else {
            return false;
        };
        fields.iter().all(|field| properties.contains_key(*field))
            && self.input_schema["required"]
                .as_array()
                .is_none_or(|required| {
                    required
                        .iter()
                        .all(|field| field.as_str().is_some_and(|field| fields.contains(&field)))
                })
    }
}

pub struct Client<'a> {
    catalog: std::collections::HashMap<String, schema::ToolValidation>,
    guard: Option<DispatchGuard>,
    endpoint: String,
    bearer: &'a str,
    agent: Agent,
    _cancellation: Option<dunlin_http::Cancellation>,
    session: Option<String>,
    version: String,
    next_id: u64,
    initialized: bool,
    started: Instant,
}

pub fn validate_endpoint(endpoint: &str) -> Result<(), Error> {
    let url = url::Url::parse(endpoint).map_err(|_| Error::Endpoint)?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Endpoint);
    }
    Ok(())
}

impl<'a> Client<'a> {
    /// Explicit adapter for initialization-based MCP 2025 servers.
    /// New integrations can use `negotiated::Client` to probe the modern era.
    pub fn connect(endpoint: &str, bearer: &'a str) -> Result<Self, Error> {
        Self::connect_since(endpoint, bearer, Instant::now(), None)
    }

    fn connect_since(
        endpoint: &str,
        bearer: &'a str,
        started: Instant,
        guard: Option<DispatchGuard>,
    ) -> Result<Self, Error> {
        validate_endpoint(endpoint)?;
        validate_credential(bearer)?;
        let agent = AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .build();
        let cancellation = guard
            .as_ref()
            .map(|guard| guard.watch(&agent))
            .transpose()?;
        let mut client = Self {
            catalog: Default::default(),
            guard,
            endpoint: endpoint.into(),
            bearer,
            agent,
            _cancellation: cancellation,
            session: None,
            version: VERSION.into(),
            next_id: 1,
            initialized: false,
            started,
        };
        let result = client.request(
            "initialize",
            json!({
                "protocolVersion": VERSION, "capabilities": {},
                "clientInfo": {"name":"dunlin", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        let version = result["protocolVersion"].as_str().ok_or(Error::Protocol)?;
        if !matches!(version, "2025-03-26" | "2025-06-18" | "2025-11-25") {
            return Err(Error::Unsupported);
        }
        client.version = version.into();
        let response =
            client.post(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
        if response.status() != 202 {
            return Err(Error::Protocol);
        }
        client.initialized = true;
        Ok(client)
    }

    fn post(&self, body: &Value) -> Result<Response, Error> {
        authorize(self.guard.as_ref())?;
        let remaining = Duration::from_secs(90).saturating_sub(self.started.elapsed());
        if remaining.is_zero() {
            return Err(Error::Transport);
        }
        let bytes = serde_json::to_vec(body).map_err(|_| Error::Protocol)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Limit);
        }
        let mut request = self
            .agent
            .post(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.bearer))
            .set("Accept", "application/json, text/event-stream")
            .set("Content-Type", "application/json")
            .set("MCP-Protocol-Version", &self.version)
            .timeout(remaining.min(Duration::from_secs(20)));
        if let Some(session) = &self.session {
            request = request.set("Mcp-Session-Id", session);
        }
        let response = request.send_bytes(&bytes);
        authorize(self.guard.as_ref())?;
        let response = response.map_err(|error| match error {
            dunlin_http::Error::Status(401 | 403, _) => Error::Unauthorized,
            dunlin_http::Error::Status(404, _) if self.session.is_some() => Error::SessionExpired,
            dunlin_http::Error::Status(status, _) => Error::Http(status),
            _ => Error::Transport,
        })?;
        if response.status() != 200 && response.status() != 202 {
            return Err(Error::Http(response.status()));
        }
        authorize(self.guard.as_ref())?;
        Ok(response)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Limit)?;
        let response =
            self.post(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))?;
        if method == "initialize"
            && let Some(session) = response.header("Mcp-Session-Id")
        {
            if session.is_empty()
                || session.len() > 1024
                || !session.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            {
                return Err(Error::Protocol);
            }
            self.session = Some(session.into());
        }
        let result = decode_response(response, id);
        authorize(self.guard.as_ref())?;
        result
    }

    pub fn tools(&mut self) -> Result<Vec<Tool>, Error> {
        self.catalog.clear();
        self.collect_tools()
    }
    fn collect_tools(&mut self) -> Result<Vec<Tool>, Error> {
        if !self.initialized {
            return Err(Error::Protocol);
        }
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        let mut catalog = std::collections::HashMap::new();
        let mut budget = schema::CatalogBudget::default();
        for _ in 0..16 {
            let result = self.request(
                "tools/list",
                cursor
                    .as_ref()
                    .map(|cursor| json!({"cursor":cursor}))
                    .unwrap_or(json!({})),
            )?;
            let page: Vec<Tool> =
                serde_json::from_value(result["tools"].clone()).map_err(|_| Error::Protocol)?;
            for tool in page {
                if tool.name.is_empty() || tool.name.len() > 256 || !names.insert(tool.name.clone())
                {
                    return Err(Error::Protocol);
                }
                if names.len() > MAX_TOOLS {
                    return Err(Error::Limit);
                }
                let compiled = match schema::ToolValidation::compile(&tool) {
                    Ok(compiled) => compiled,
                    Err(Error::Schema) => continue,
                    Err(error) => return Err(error),
                };
                budget.admit(&compiled)?;
                catalog.insert(tool.name.clone(), compiled);
                tools.push(tool);
            }
            match result.get("nextCursor").filter(|value| !value.is_null()) {
                None => {
                    self.catalog = catalog;
                    return Ok(tools);
                }
                Some(value) => {
                    let next = value
                        .as_str()
                        .filter(|value| !value.is_empty() && value.len() <= 4096)
                        .ok_or(Error::Protocol)?;
                    if !cursors.insert(next.to_string()) {
                        return Err(Error::Protocol);
                    }
                    cursor = Some(next.to_string());
                }
            }
        }
        Err(Error::Limit)
    }

    pub fn call(&mut self, tool: &Tool, arguments: Value) -> Result<Value, Error> {
        if !self.initialized || !arguments.is_object() {
            return Err(Error::Protocol);
        }
        let validation = self
            .catalog
            .get(&tool.name)
            .cloned()
            .ok_or(Error::Unsupported)?;
        validation.arguments(tool, &arguments)?;
        let result = self.request(
            "tools/call",
            json!({"name":tool.name,"arguments":arguments}),
        )?;
        if result["resultType"] == "input_required" {
            return Err(Error::Unsupported);
        }
        if result.get("isError").is_some_and(|flag| !flag.is_boolean()) {
            return Err(Error::Protocol);
        }
        if result["isError"].as_bool() == Some(true) {
            return Err(Error::ToolFailed);
        }
        if !result.is_object() {
            return Err(Error::Protocol);
        }
        validation.output(&result)?;
        Ok(result)
    }

    pub fn close(&mut self) -> Result<(), Error> {
        let result = self.close_inner();
        self.agent.shutdown();
        self._cancellation.take();
        result
    }
    fn close_inner(&mut self) -> Result<(), Error> {
        self.initialized = false;
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        authorize(self.guard.as_ref())?;
        let response = self
            .agent
            .delete(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.bearer))
            .set("Mcp-Session-Id", &session)
            .set("MCP-Protocol-Version", &self.version)
            .timeout(Duration::from_secs(5))
            .call();
        authorize(self.guard.as_ref())?;
        match response {
            Ok(response) if matches!(response.status(), 200 | 202 | 204) => Ok(()),
            Err(dunlin_http::Error::Status(404 | 405, _)) => Ok(()),
            _ => Err(Error::Transport),
        }
    }
}

impl Drop for Client<'_> {
    fn drop(&mut self) {
        self.agent.shutdown();
    }
}

fn validate_credential(bearer: &str) -> Result<(), Error> {
    if bearer.is_empty()
        || bearer.len() > 8192
        || !bearer.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(Error::Credential);
    }
    Ok(())
}

fn rpc_result(message: Value, id: u64) -> Result<Value, Error> {
    if message["jsonrpc"] != "2.0"
        || message["id"].as_u64() != Some(id)
        || message.get("result").is_some() == message.get("error").is_some()
    {
        return Err(Error::Protocol);
    }
    if let Some(error) = message.get("error") {
        return Err(Error::Rpc(error["code"].as_i64().ok_or(Error::Protocol)?));
    }
    Ok(message["result"].clone())
}

fn decode_response(response: Response, id: u64) -> Result<Value, Error> {
    let content_type = response
        .header("Content-Type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if response.status() != 200 {
        return Err(Error::Protocol);
    }
    let reader = response.into_reader().take(MAX_BYTES as u64 + 1);
    if content_type == "application/json" {
        let mut bytes = Vec::new();
        let mut reader = reader;
        reader
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Transport)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Limit);
        }
        return rpc_result(
            serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?,
            id,
        );
    }
    if content_type != "text/event-stream" {
        return Err(Error::Protocol);
    }
    let mut reader = BufReader::new(reader);
    let mut data = String::new();
    let mut consumed = 0;
    let mut events = 0;
    loop {
        let mut line = String::new();
        let count = reader.read_line(&mut line).map_err(|_| Error::Transport)?;
        consumed += count;
        if consumed > MAX_BYTES {
            return Err(Error::Limit);
        }
        if count == 0 {
            return Err(Error::Protocol);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() && !data.is_empty() {
            events += 1;
            if events > 128 {
                return Err(Error::Limit);
            }
            let message: Value = serde_json::from_str(&data).map_err(|_| Error::Protocol)?;
            data.clear();
            if message.get("method").is_some() {
                // No sampling, elicitation or other server-initiated authority.
                if message.get("id").is_some() {
                    return Err(Error::Unsupported);
                }
                continue;
            }
            return rpc_result(message, id);
        }
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod cancellation_tests;
