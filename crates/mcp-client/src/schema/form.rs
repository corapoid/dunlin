//! MCP's flat form profile, not an arbitrary JSON Schema form generator.
use super::{Error, Schema, check_size};
use serde_json::{Map, Value};
use std::collections::HashSet;

pub struct FormSchema {
    schema: Value,
    validator: Schema,
}
fn keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), Error> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::Schema);
    }
    Ok(())
}
fn label(object: &Map<String, Value>) -> Result<(), Error> {
    for (key, max) in [("title", 256), ("description", 4096)] {
        if let Some(value) = object.get(key)
            && value.as_str().is_none_or(|text| text.len() > max)
        {
            return Err(Error::Schema);
        }
    }
    Ok(())
}
fn choices(schema: &Map<String, Value>, titled: &str) -> Result<(), Error> {
    let mut seen = HashSet::new();
    if let Some(values) = schema.get("enum") {
        if schema.contains_key(titled) {
            return Err(Error::Schema);
        }
        let values = values.as_array().ok_or(Error::Schema)?;
        if values.is_empty() || values.len() > 64 {
            return Err(Error::Limit);
        }
        for value in values {
            let text = value.as_str().ok_or(Error::Schema)?;
            if text.len() > 2048 || !seen.insert(text) {
                return Err(Error::Schema);
            }
        }
    } else if let Some(values) = schema.get(titled) {
        let values = values.as_array().ok_or(Error::Schema)?;
        if values.is_empty() || values.len() > 64 {
            return Err(Error::Limit);
        }
        for value in values {
            let option = value.as_object().ok_or(Error::Schema)?;
            keys(option, &["const", "title"])?;
            label(option)?;
            let text = option
                .get("const")
                .and_then(Value::as_str)
                .ok_or(Error::Schema)?;
            if text.len() > 2048 || !seen.insert(text) {
                return Err(Error::Schema);
            }
        }
    } else {
        return Err(Error::Schema);
    }
    Ok(())
}
fn field(schema: &Value) -> Result<(), Error> {
    let object = schema.as_object().ok_or(Error::Schema)?;
    label(object)?;
    let mut allowed = vec!["type", "title", "description", "default"];
    match object.get("type").and_then(Value::as_str) {
        Some("string") => {
            allowed.extend(["minLength", "maxLength", "format", "enum", "oneOf"]);
            if let Some(format) = object.get("format")
                && !matches!(
                    format.as_str(),
                    Some("email" | "uri" | "date" | "date-time")
                )
            {
                return Err(Error::Schema);
            }
            if object.contains_key("enum") || object.contains_key("oneOf") {
                choices(object, "oneOf")?;
            }
        }
        Some("number" | "integer") => allowed.extend(["minimum", "maximum"]),
        Some("boolean") => {}
        Some("array") => {
            allowed.extend(["minItems", "maxItems", "items"]);
            let items = object
                .get("items")
                .and_then(Value::as_object)
                .ok_or(Error::Schema)?;
            keys(items, &["type", "enum", "anyOf"])?;
            if items.get("type").is_some_and(|value| value != "string") {
                return Err(Error::Schema);
            }
            choices(items, "anyOf")?;
        }
        _ => return Err(Error::Schema),
    }
    keys(object, &allowed)?;
    if let Some(default) = object.get("default") {
        if object.get("type").and_then(Value::as_str) == Some("integer")
            && default
                .as_f64()
                .is_some_and(|number| number.abs() > 9_007_199_254_740_991.0)
        {
            return Err(Error::Schema);
        }
        Schema::compile_formats(schema, true)?
            .validate(default)
            .map_err(|_| Error::Schema)?;
        value_bounds(default)?;
    }
    Ok(())
}
fn value_bounds(value: &Value) -> Result<(), Error> {
    match value {
        Value::String(text) if text.len() > 8192 => Err(Error::Limit),
        Value::Array(values) => {
            if values.len() > 64 {
                return Err(Error::Limit);
            }
            let mut seen = HashSet::new();
            for value in values {
                let text = value.as_str().ok_or(Error::Arguments)?;
                if text.len() > 2048 || !seen.insert(text) {
                    return Err(Error::Arguments);
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
impl FormSchema {
    pub fn compile(schema: &Value) -> Result<Self, Error> {
        check_size(schema, 1024, 16 * 1024)?;
        let object = schema.as_object().ok_or(Error::Schema)?;
        keys(
            object,
            &[
                "type",
                "properties",
                "required",
                "title",
                "description",
                "$schema",
                "additionalProperties",
            ],
        )?;
        label(object)?;
        if object.get("type").and_then(Value::as_str) != Some("object")
            || object
                .get("additionalProperties")
                .is_some_and(|value| value.as_bool() != Some(false))
        {
            return Err(Error::Schema);
        }
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .ok_or(Error::Schema)?;
        if properties.len() > 32 {
            return Err(Error::Limit);
        }
        for (name, schema) in properties {
            if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
                return Err(Error::Schema);
            }
            field(schema)?;
        }
        if let Some(required) = object.get("required") {
            let mut names = HashSet::new();
            for name in required.as_array().ok_or(Error::Schema)? {
                let name = name.as_str().ok_or(Error::Schema)?;
                if !properties.contains_key(name) || !names.insert(name) {
                    return Err(Error::Schema);
                }
            }
        }
        Ok(Self {
            schema: schema.clone(),
            validator: Schema::compile_formats(schema, true)?,
        })
    }
    pub fn schema(&self) -> &Value {
        &self.schema
    }
    pub fn validate(&self, content: &Value) -> Result<(), Error> {
        check_size(content, 1024, 64 * 1024)?;
        let object = content.as_object().ok_or(Error::Arguments)?;
        let properties = self.schema["properties"].as_object().ok_or(Error::Schema)?;
        for (name, value) in object {
            if !properties.contains_key(name) {
                return Err(Error::Arguments);
            }
            value_bounds(value)?;
            if properties[name]["type"] == "integer"
                && value
                    .as_f64()
                    .is_some_and(|number| number.abs() > 9_007_199_254_740_991.0)
            {
                return Err(Error::Arguments);
            }
        }
        self.validator.validate(content)
    }
}

#[cfg(test)]
mod tests;
