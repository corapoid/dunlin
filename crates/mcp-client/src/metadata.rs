//! MCP 2026-07-28 request metadata and HTTP header projection.
//!
//! This module does not authorize a tool, resolve schema references or send a
//! request. The caller must separately validate arguments and grant authority.
use crate::{Error, MAX_BYTES};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const VERSION: &str = "2026-07-28";
const SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// One body and its matching transport headers; contains no credentials.
pub struct PreparedRequest {
    pub body: Value,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct Projection {
    path: Vec<String>,
    name: String,
    kind: String,
}

/// Compiled projections for a single tool schema. Invalid annotations exclude
/// the tool from the modern HTTP catalog; they must not silently be ignored.
#[derive(Clone, Debug)]
pub struct ToolHeaders(Vec<Projection>);

impl ToolHeaders {
    pub fn from_schema(schema: &Value) -> Result<Self, Error> {
        if !schema.is_object() {
            return Err(Error::Protocol);
        }
        if serde_json::to_vec(schema)
            .map_err(|_| Error::Protocol)?
            .len()
            > MAX_BYTES
        {
            return Err(Error::Limit);
        }
        let mut result = Self(Vec::new());
        let mut names = HashSet::new();
        let mut budget = 4096;
        result.visit(schema, &mut Vec::new(), false, 0, &mut names, &mut budget)?;
        Ok(result)
    }

    fn visit(
        &mut self,
        schema: &Value,
        path: &mut Vec<String>,
        reachable: bool,
        depth: usize,
        names: &mut HashSet<String>,
        budget: &mut usize,
    ) -> Result<(), Error> {
        if depth > 64 || *budget == 0 {
            return Err(Error::Limit);
        }
        *budget -= 1;
        let Some(object) = schema.as_object() else {
            return if schema.is_boolean() {
                Ok(())
            } else {
                Err(Error::Protocol)
            };
        };
        if let Some(annotation) = object.get("x-mcp-header") {
            let name = annotation.as_str().ok_or(Error::Protocol)?;
            let kind = schema["type"].as_str().ok_or(Error::Protocol)?;
            if !reachable
                || name.is_empty()
                || name.len() > 256
                || !name.bytes().all(token)
                || !names.insert(name.to_ascii_lowercase())
                || !matches!(kind, "string" | "integer" | "boolean")
            {
                return Err(Error::Protocol);
            }
            if self.0.len() == MAX_HEADERS {
                return Err(Error::Limit);
            }
            self.0.push(Projection {
                path: path.clone(),
                name: format!("Mcp-Param-{name}"),
                kind: kind.into(),
            });
        }
        // Only `properties` preserves an exact instance path. Other schema
        // locations are still inspected so hidden invalid annotations fail.
        for key in [
            "properties",
            "patternProperties",
            "$defs",
            "definitions",
            "dependentSchemas",
        ] {
            if let Some(children) = object.get(key) {
                let children = children.as_object().ok_or(Error::Protocol)?;
                for (name, child) in children {
                    let exact = key == "properties" && (depth == 0 || reachable);
                    path.push(name.clone());
                    self.visit(child, path, exact, depth + 1, names, budget)?;
                    path.pop();
                }
            }
        }
        for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
            if let Some(children) = object.get(key) {
                for child in children.as_array().ok_or(Error::Protocol)? {
                    self.visit(child, path, false, depth + 1, names, budget)?;
                }
            }
        }
        for key in [
            "items",
            "contains",
            "additionalProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "propertyNames",
            "not",
            "if",
            "then",
            "else",
            "contentSchema",
        ] {
            if let Some(child) = object.get(key) {
                self.visit(child, path, false, depth + 1, names, budget)?;
            }
        }
        Ok(())
    }

    pub fn project(&self, arguments: &Value) -> Result<Vec<(String, String)>, Error> {
        if !arguments.is_object() {
            return Err(Error::Protocol);
        }
        let mut headers = Vec::new();
        let mut bytes = 0;
        for projection in &self.0 {
            let mut value = arguments;
            for segment in &projection.path {
                value = value.get(segment).unwrap_or(&Value::Null);
            }
            if value.is_null() {
                continue;
            }
            let text = match projection.kind.as_str() {
                "string" => value.as_str().ok_or(Error::Protocol)?.to_owned(),
                "boolean" => value.as_bool().ok_or(Error::Protocol)?.to_string(),
                "integer" => {
                    let number = value.as_f64().ok_or(Error::Protocol)?;
                    if number.fract() != 0.0 || number.abs() > SAFE_INTEGER as f64 {
                        return Err(Error::Protocol);
                    }
                    (number as i64).to_string()
                }
                _ => return Err(Error::Protocol),
            };
            let encoded = header_value(&text)?;
            bytes += projection.name.len() + encoded.len();
            if bytes > MAX_HEADER_BYTES {
                return Err(Error::Limit);
            }
            headers.push((projection.name.clone(), encoded));
        }
        Ok(headers)
    }
}

fn token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Header values preserve UTF-8 and sentinel-shaped literals unambiguously.
pub fn header_value(value: &str) -> Result<String, Error> {
    if value.len() > MAX_HEADER_BYTES {
        return Err(Error::Limit);
    }
    let safe = value
        .bytes()
        .all(|byte| (0x20..=0x7e).contains(&byte) || byte == b'\t')
        && value.trim_matches([' ', '\t']) == value
        && !(value.starts_with("=?base64?") && value.ends_with("?="));
    let result = if safe {
        value.into()
    } else {
        format!("=?base64?{}?=", STANDARD.encode(value.as_bytes()))
    };
    if result.len() > MAX_HEADER_BYTES {
        return Err(Error::Limit);
    }
    Ok(result)
}

/// Insert transport-owned metadata. Client capabilities are deliberately empty
/// until the corresponding input handlers exist; untrusted params cannot
/// advertise sampling, roots or elicitation on the caller's behalf.
pub fn request(id: u64, method: &str, params: Value) -> Result<PreparedRequest, Error> {
    request_with_capabilities(id, method, params, json!({}))
}

pub(crate) fn request_with_capabilities(
    id: u64,
    method: &str,
    mut params: Value,
    capabilities: Value,
) -> Result<PreparedRequest, Error> {
    if !capabilities.is_object() {
        return Err(Error::Protocol);
    }
    if id > SAFE_INTEGER as u64
        || method.is_empty()
        || method.len() > 128
        || !method.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(Error::Protocol);
    }
    let object = params.as_object_mut().ok_or(Error::Protocol)?;
    let meta = object
        .entry("_meta")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(Error::Protocol)?;
    for (key, value) in [
        ("io.modelcontextprotocol/protocolVersion", json!(VERSION)),
        ("io.modelcontextprotocol/clientCapabilities", capabilities),
        (
            "io.modelcontextprotocol/clientInfo",
            json!({"name":"dunlin", "version":env!("CARGO_PKG_VERSION")}),
        ),
    ] {
        if meta.contains_key(key) {
            return Err(Error::Protocol);
        }
        meta.insert(key.into(), value);
    }
    let mut headers = vec![
        ("MCP-Protocol-Version".into(), VERSION.into()),
        ("Mcp-Method".into(), method.into()),
    ];
    let name = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(key) = name {
        headers.push((
            "Mcp-Name".into(),
            header_value(params[key].as_str().ok_or(Error::Protocol)?)?,
        ));
    }
    let body = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
    if serde_json::to_vec(&body)
        .map_err(|_| Error::Protocol)?
        .len()
        > MAX_BYTES
    {
        return Err(Error::Limit);
    }
    Ok(PreparedRequest { body, headers })
}

#[cfg(test)]
mod tests;
