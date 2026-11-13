//! Bounded JSON Schema validation. References never read files or the network.
//! Validation failures deliberately omit schema paths and instance contents.
use crate::{Error, Tool};
use jsonschema::{Draft, PatternOptions, Retrieve, Uri, Validator};
use serde_json::Value;
use std::sync::Arc;
mod form;
mod references;
pub use form::FormSchema;

pub(crate) fn check_size(value: &Value, nodes: usize, bytes: usize) -> Result<(), Error> {
    size(value, nodes, bytes).map(|_| ())
}

pub(crate) fn optional_output<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    use serde::Deserialize;
    let value = Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(serde::de::Error::custom(
            "MCP output schema must be an object",
        ));
    }
    Ok(Some(value))
}

const SCHEMA_BYTES: usize = 64 * 1024;
const SCHEMA_NODES: usize = 1024;
const INSTANCE_BYTES: usize = 256 * 1024;
const INSTANCE_NODES: usize = 8192;
const DEPTH: usize = 64;

struct NoExternal;
impl Retrieve for NoExternal {
    fn retrieve(&self, _: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("External JSON Schema references are disabled".into())
    }
}

#[derive(Clone)]
pub struct Schema {
    validator: Arc<Validator>,
    nodes: usize,
    patterns: usize,
}
#[derive(Default)]
struct Size {
    nodes: usize,
    bytes: usize,
}
fn size(value: &Value, max_nodes: usize, max_bytes: usize) -> Result<Size, Error> {
    fn visit(
        value: &Value,
        depth: usize,
        size: &mut Size,
        nodes: usize,
        bytes: usize,
    ) -> Result<(), Error> {
        size.nodes += 1;
        if depth > DEPTH || size.nodes > nodes {
            return Err(Error::Limit);
        }
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    size.bytes = size.bytes.saturating_add(key.len());
                    if size.bytes > bytes {
                        return Err(Error::Limit);
                    }
                    visit(child, depth + 1, size, nodes, bytes)?;
                }
            }
            Value::Array(array) => {
                for child in array {
                    visit(child, depth + 1, size, nodes, bytes)?;
                }
            }
            Value::String(text) => size.bytes = size.bytes.saturating_add(text.len()),
            _ => size.bytes = size.bytes.saturating_add(8),
        }
        if size.bytes > bytes {
            return Err(Error::Limit);
        }
        Ok(())
    }
    let mut result = Size::default();
    visit(value, 0, &mut result, max_nodes, max_bytes)?;
    if serde_json::to_vec(value)
        .map_err(|_| Error::Protocol)?
        .len()
        > max_bytes
    {
        return Err(Error::Limit);
    }
    Ok(result)
}
impl Schema {
    pub fn compile(schema: &Value) -> Result<Self, Error> {
        Self::compile_formats(schema, true)
    }
    pub(crate) fn compile_formats(schema: &Value, formats: bool) -> Result<Self, Error> {
        let size = size(schema, SCHEMA_NODES, SCHEMA_BYTES)?;
        let declared = match schema.get("$schema") {
            None => None,
            Some(Value::String(value)) => Some(value.strip_suffix('#').unwrap_or(value)),
            _ => return Err(Error::Schema),
        };
        let draft = match declared {
            None => Draft::Draft202012,
            Some("http://json-schema.org/draft-04/schema") => Draft::Draft4,
            Some("http://json-schema.org/draft-06/schema") => Draft::Draft6,
            Some("http://json-schema.org/draft-07/schema") => Draft::Draft7,
            Some("https://json-schema.org/draft/2019-09/schema") => Draft::Draft201909,
            Some("https://json-schema.org/draft/2020-12/schema") => Draft::Draft202012,
            _ => return Err(Error::Schema),
        };
        let (expanded, patterns) = references::analyze(schema, draft)?;
        let weight = expanded.max(size.nodes);
        if patterns > 16 {
            return Err(Error::Limit);
        }
        // A linear engine avoids treating a backtracking-limit error as a
        // false predicate inside `not`/`if`. Unsupported patterns fail closed.
        let validator = jsonschema::options()
            .with_draft(draft)
            .with_retriever(NoExternal)
            .should_validate_formats(formats)
            .with_pattern_options(
                PatternOptions::regex()
                    .size_limit(32 * 1024)
                    .dfa_size_limit(32 * 1024),
            )
            .build(schema)
            .map_err(|_| Error::Schema)?;
        Ok(Self {
            validator: Arc::new(validator),
            nodes: weight,
            patterns,
        })
    }
    pub fn validate(&self, instance: &Value) -> Result<(), Error> {
        let size = size(instance, INSTANCE_NODES, INSTANCE_BYTES)?;
        let instance_work = size.nodes.saturating_add(size.bytes.div_ceil(64));
        if self.nodes.saturating_mul(instance_work) > 1_000_000 {
            return Err(Error::Limit);
        }
        self.validator
            .validate(instance)
            .map_err(|_| Error::Arguments)
    }
}

#[derive(Clone)]
pub(crate) struct ToolValidation {
    pub tool: Tool,
    input: Schema,
    output: Option<Schema>,
}
impl ToolValidation {
    pub fn compile(tool: &Tool) -> Result<Self, Error> {
        if !tool.input_schema.is_object()
            || tool
                .output_schema
                .as_ref()
                .is_some_and(|schema| !schema.is_object())
        {
            return Err(Error::Schema);
        }
        Ok(Self {
            tool: tool.clone(),
            input: Schema::compile(&tool.input_schema)?,
            output: tool
                .output_schema
                .as_ref()
                .map(Schema::compile)
                .transpose()?,
        })
    }
    pub fn arguments(&self, tool: &Tool, arguments: &Value) -> Result<(), Error> {
        if &self.tool != tool {
            return Err(Error::Unsupported);
        }
        if !arguments.is_object() {
            return Err(Error::Arguments);
        }
        self.input.validate(arguments)
    }
    pub fn output(&self, result: &Value) -> Result<(), Error> {
        if let Some(schema) = &self.output {
            let structured = result
                .get("structuredContent")
                .filter(|value| value.is_object())
                .ok_or(Error::Protocol)?;
            schema.validate(structured).map_err(|error| {
                if error == Error::Limit {
                    error
                } else {
                    Error::Protocol
                }
            })?;
        }
        Ok(())
    }
}
#[derive(Default)]
pub(crate) struct CatalogBudget {
    nodes: usize,
    patterns: usize,
}
impl CatalogBudget {
    pub fn admit(&mut self, tool: &ToolValidation) -> Result<(), Error> {
        self.nodes += tool.input.nodes + tool.output.as_ref().map_or(0, |s| s.nodes);
        self.patterns += tool.input.patterns + tool.output.as_ref().map_or(0, |s| s.patterns);
        if self.nodes > 65536 || self.patterns > 128 {
            return Err(Error::Limit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
