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

pub mod linear;

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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl Tool {
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
    endpoint: String,
    bearer: &'a str,
    agent: Agent,
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
    pub fn connect(endpoint: &str, bearer: &'a str) -> Result<Self, Error> {
        validate_endpoint(endpoint)?;
        if bearer.is_empty()
            || bearer.len() > 8192
            || !bearer.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(Error::Credential);
        }
        let mut client = Self {
            endpoint: endpoint.into(),
            bearer,
            agent: AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0)
                .build(),
            session: None,
            version: VERSION.into(),
            next_id: 1,
            initialized: false,
            started: Instant::now(),
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
        let response = request.send_bytes(&bytes).map_err(|error| match error {
            dunlin_http::Error::Status(401 | 403, _) => Error::Unauthorized,
            dunlin_http::Error::Status(404, _) if self.session.is_some() => Error::SessionExpired,
            dunlin_http::Error::Status(status, _) => Error::Http(status),
            _ => Error::Transport,
        })?;
        if response.status() != 200 && response.status() != 202 {
            return Err(Error::Http(response.status()));
        }
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
        decode_response(response, id)
    }

    pub fn tools(&mut self) -> Result<Vec<Tool>, Error> {
        if !self.initialized {
            return Err(Error::Protocol);
        }
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
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
                tools.push(tool);
                if tools.len() > MAX_TOOLS {
                    return Err(Error::Limit);
                }
            }
            match result.get("nextCursor").filter(|value| !value.is_null()) {
                None => return Ok(tools),
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
        let result = self.request(
            "tools/call",
            json!({"name":tool.name,"arguments":arguments}),
        )?;
        if result.get("isError").is_some_and(|flag| !flag.is_boolean()) {
            return Err(Error::Protocol);
        }
        if result["isError"].as_bool() == Some(true) {
            return Err(Error::ToolFailed);
        }
        if !result.is_object() {
            return Err(Error::Protocol);
        }
        Ok(result)
    }

    pub fn close(&mut self) -> Result<(), Error> {
        self.initialized = false;
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        let response = self
            .agent
            .delete(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.bearer))
            .set("Mcp-Session-Id", &session)
            .set("MCP-Protocol-Version", &self.version)
            .timeout(Duration::from_secs(5))
            .call();
        match response {
            Ok(response) if matches!(response.status(), 200 | 202 | 204) => Ok(()),
            Err(dunlin_http::Error::Status(404 | 405, _)) => Ok(()),
            _ => Err(Error::Transport),
        }
    }
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
