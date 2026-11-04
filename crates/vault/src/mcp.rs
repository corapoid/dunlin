//! Minimal MCP stdio server for the `Dunlin --mcp-vault` proxy.
//!
//! The proxy has no access to the Keychain. It only forwards tool calls to the
//! running Dunlin process over the local Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::client::VaultClient;
use crate::http::HttpInput;
use crate::recipe::RecipeInput;

const MAX_LINE: usize = 4 * 1024 * 1024;
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

pub fn run_stdio() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("[dunlin-vault] {error}");
            1
        }
    }
}

fn run() -> Result<(), String> {
    let socket = std::env::var_os("DUNLIN_VAULT_SOCKET")
        .map(PathBuf::from)
        .ok_or_else(|| "DUNLIN_VAULT_SOCKET is not set".to_string())?;
    let token = std::env::var("DUNLIN_VAULT_TOKEN")
        .map_err(|_| "DUNLIN_VAULT_TOKEN is not set".to_string())?;
    let mut client = VaultClient::connect(&socket, &token)?;
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("Could not read MCP input: {error}"))?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > MAX_LINE {
            return Err("The MCP request is too large".into());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(_) => {
                write_message(
                    &mut stdout,
                    json!({"jsonrpc":"2.0","id":Value::Null,"error":{"code":-32700,"message":"Parse error"}}),
                )?;
                continue;
            }
        };
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        if id.is_none() {
            // Notifications have no response.
            continue;
        }
        let id = id.unwrap();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => call_tool(&mut client, &params),
            "resources/list" => Ok(json!({"resources":[]})),
            "prompts/list" => Ok(json!({"prompts":[]})),
            _ => Err((-32601, "Method not found".to_string())),
        };
        match result {
            Ok(result) => write_message(
                &mut stdout,
                json!({"jsonrpc":"2.0","id":id,"result":result}),
            )?,
            Err((code, message)) => write_message(
                &mut stdout,
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
            )?,
        }
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let protocol = if SUPPORTED_PROTOCOLS.contains(&requested) {
        requested
    } else {
        "2025-06-18"
    };
    json!({
        "protocolVersion": protocol,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "dunlin-secrets", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Dunlin credentials are use-only. Call secrets_list to discover aliases, then secrets_request for an HTTP request or secrets_run for a fixed CLI recipe. The credential value is injected by Dunlin and is never returned to you. Never ask the user to paste or reveal a credential."
    })
}

fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "secrets_list",
            "description": "List credentials available to this project. Returns public aliases, descriptions, allowed hosts/methods, named HTTP operations and CLI recipes. Never returns secret values.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "secrets_request",
            "description": "Perform an HTTPS request with a named credential injected by Dunlin. Use a named operation from secrets_list or an explicit method+url. The credential value is never returned. Calls are limited to the credential's allowed hosts, path prefix, methods and policy. User approval may be required.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "secret": {"type": "string", "description": "Alias from secrets_list"},
                    "operation": {"type": "string", "description": "Named operation from secrets_list; when set, do not send method, url or body"},
                    "params": {"type": "object", "additionalProperties": {"type":"string"}, "description": "Declared operation path parameters"},
                    "method": {"type": "string", "enum": ["GET","HEAD","POST","PUT","PATCH","DELETE"]},
                    "url": {"type": "string", "description": "Absolute HTTPS URL on an allowed host; omit when using operation"},
                    "headers": {"type": "object", "additionalProperties": {"type":"string"}, "description": "Only headers allowed by the credential; Authorization and Cookie are refused"},
                    "body": {"type": "string", "description": "Optional request body; omit when using operation"}
                },
                "required": ["secret"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": true}
        }),
        json!({
            "name": "secrets_run",
            "description": "Run a fixed CLI recipe registered for a credential. The credential is injected into the child process environment and is never returned. Only declared recipe parameters may be supplied.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "secret": {"type": "string", "description": "Alias from secrets_list"},
                    "recipe": {"type": "string", "description": "Recipe name from secrets_list"},
                    "params": {"type": "object", "additionalProperties": {"type":"string"}, "description": "Declared recipe parameters"}
                },
                "required": ["secret","recipe"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": true}
        }),
    ]
}

fn call_tool(client: &mut VaultClient, params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "Missing tool name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "secrets_list" => match client.list() {
            Ok(secrets) => Ok(tool_text(
                &serde_json::to_string_pretty(&secrets).unwrap_or_default(),
                false,
            )),
            Err(error) => Ok(tool_text(&format!("Dunlin vault: {error}"), true)),
        },
        "secrets_request" => {
            let input: HttpInput = serde_json::from_value(arguments).map_err(|error| {
                (
                    -32602,
                    format!("Invalid secrets_request arguments: {error}"),
                )
            })?;
            match client.http(input) {
                Ok(output) => Ok(tool_text(
                    &serde_json::to_string_pretty(&output).unwrap_or_default(),
                    false,
                )),
                Err(error) => Ok(tool_text(&format!("Dunlin vault: {error}"), true)),
            }
        }
        "secrets_run" => {
            let input: RecipeInput = serde_json::from_value(arguments)
                .map_err(|error| (-32602, format!("Invalid secrets_run arguments: {error}")))?;
            match client.run(input) {
                Ok(output) => Ok(tool_text(
                    &serde_json::to_string_pretty(&output).unwrap_or_default(),
                    false,
                )),
                Err(error) => Ok(tool_text(&format!("Dunlin vault: {error}"), true)),
            }
        }
        _ => Err((-32602, format!("Unknown tool: {name}"))),
    }
}

fn tool_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": is_error
    })
}

fn write_message(stdout: &mut impl Write, value: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdout, &value)
        .map_err(|error| format!("Could not write MCP output: {error}"))?;
    stdout
        .write_all(b"\n")
        .and_then(|_| stdout.flush())
        .map_err(|error| format!("Could not flush MCP output: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_have_no_value_reading_surface() {
        let tools = tools();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["secrets_list", "secrets_request", "secrets_run"]
        );
        let json = serde_json::to_string(&tools).unwrap();
        assert!(!json.contains("reveal"));
        assert!(!json.contains("get_secret"));
    }

    #[test]
    fn initialize_negotiates_supported_protocols() {
        assert_eq!(
            initialize(&json!({"protocolVersion":"2025-06-18"}))["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(
            initialize(&json!({"protocolVersion":"2024-11-05"}))["protocolVersion"],
            "2024-11-05"
        );
        assert_eq!(
            initialize(&json!({"protocolVersion":"unknown"}))["protocolVersion"],
            "2025-06-18"
        );
    }
}
