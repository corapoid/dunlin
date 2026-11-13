//! Admission cost for local reference graphs, including repeated DAG branches.
//! Productive recursion descends into the instance; same-instance cycles fail.
use super::{Draft, Error};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Same,
    Descend,
    Definition,
}
fn children(schema: &Value, draft: Draft) -> Result<Vec<(&Value, Edge)>, Error> {
    let mut result = Vec::new();
    let modern = matches!(draft, Draft::Draft201909 | Draft::Draft202012);
    for (key, edge, enabled) in [
        ("properties", Edge::Descend, true),
        ("patternProperties", Edge::Descend, true),
        ("$defs", Edge::Definition, modern),
        (
            "definitions",
            Edge::Definition,
            !matches!(draft, Draft::Draft202012),
        ),
        ("dependentSchemas", Edge::Same, modern),
    ] {
        if enabled && let Some(value) = schema.get(key) {
            for child in value.as_object().ok_or(Error::Schema)?.values() {
                result.push((child, edge));
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if key == "prefixItems" && !matches!(draft, Draft::Draft202012) {
            continue;
        }
        if let Some(value) = schema.get(key) {
            let edge = if key == "prefixItems" {
                Edge::Descend
            } else {
                Edge::Same
            };
            for child in value.as_array().ok_or(Error::Schema)? {
                result.push((child, edge));
            }
        }
    }
    let conditional = matches!(
        draft,
        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
    );
    for (key, edge, enabled) in [
        ("additionalProperties", Edge::Descend, true),
        (
            "additionalItems",
            Edge::Descend,
            draft != Draft::Draft202012,
        ),
        ("unevaluatedProperties", Edge::Descend, modern),
        ("unevaluatedItems", Edge::Descend, modern),
        ("propertyNames", Edge::Descend, draft != Draft::Draft4),
        ("contains", Edge::Descend, draft != Draft::Draft4),
        ("not", Edge::Same, true),
        ("if", Edge::Same, conditional),
        ("then", Edge::Same, conditional),
        ("else", Edge::Same, conditional),
        ("contentSchema", Edge::Descend, modern),
    ] {
        if enabled && let Some(child) = schema.get(key) {
            result.push((child, edge));
        }
    }
    if let Some(items) = schema.get("items") {
        if let Some(tuple) = items.as_array() {
            for child in tuple {
                result.push((child, Edge::Descend));
            }
        } else {
            result.push((items, Edge::Descend));
        }
    }
    if !modern && let Some(value) = schema.get("dependencies") {
        for child in value
            .as_object()
            .ok_or(Error::Schema)?
            .values()
            .filter(|value| !value.is_array())
        {
            result.push((child, Edge::Same));
        }
    }
    Ok(result)
}
fn refs(schema: &Value, draft: Draft) -> Result<Vec<&str>, Error> {
    let mut result = Vec::new();
    let keys: &[&str] = match draft {
        Draft::Draft201909 => &["$ref", "$recursiveRef"],
        Draft::Draft202012 => &["$ref", "$dynamicRef"],
        _ => &["$ref"],
    };
    for key in keys {
        if let Some(value) = schema.get(key) {
            result.push(value.as_str().ok_or(Error::Schema)?);
        }
    }
    Ok(result)
}
fn fragment(reference: &str) -> Result<String, Error> {
    let bytes = reference.strip_prefix('#').ok_or(Error::Schema)?.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let text = std::str::from_utf8(bytes.get(index + 1..index + 3).ok_or(Error::Schema)?)
                .map_err(|_| Error::Schema)?;
            decoded.push(u8::from_str_radix(text, 16).map_err(|_| Error::Schema)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| Error::Schema)
}
struct Graph<'a> {
    root: &'a Value,
    draft: Draft,
    nodes: Vec<&'a Value>,
    seen: HashSet<usize>,
    anchors: HashMap<String, &'a Value>,
}
impl<'a> Graph<'a> {
    fn collect(&mut self, node: &'a Value) -> Result<(), Error> {
        if !node.is_object() && !node.is_boolean() {
            return Err(Error::Schema);
        }
        let id = std::ptr::from_ref(node) as usize;
        if !self.seen.insert(id) {
            return Ok(());
        }
        if self.nodes.len() >= 1024 {
            return Err(Error::Limit);
        }
        let id_key = if self.draft == Draft::Draft4 {
            "id"
        } else {
            "$id"
        };
        if !std::ptr::eq(node, self.root) && node.get(id_key).is_some() {
            return Err(Error::Schema);
        }
        for (key, enabled) in [
            (
                "$anchor",
                matches!(self.draft, Draft::Draft201909 | Draft::Draft202012),
            ),
            ("$dynamicAnchor", self.draft == Draft::Draft202012),
        ] {
            if enabled && let Some(anchor) = node.get(key) {
                let anchor = anchor.as_str().ok_or(Error::Schema)?;
                if self
                    .anchors
                    .insert(anchor.into(), node)
                    .is_some_and(|previous| !std::ptr::eq(previous, node))
                {
                    return Err(Error::Schema);
                }
            }
        }
        self.nodes.push(node);
        for (child, _) in children(node, self.draft)? {
            self.collect(child)?;
        }
        Ok(())
    }
    fn target(&self, reference: &str) -> Result<&'a Value, Error> {
        let fragment = fragment(reference)?;
        if fragment.is_empty() {
            Ok(self.root)
        } else if fragment.starts_with('/') {
            self.root.pointer(&fragment).ok_or(Error::Schema)
        } else {
            self.anchors.get(&fragment).copied().ok_or(Error::Schema)
        }
    }
    fn cost(
        &self,
        node: &Value,
        progress: usize,
        active: &mut Vec<(usize, usize)>,
        budget: &mut usize,
    ) -> Result<usize, Error> {
        if *budget == 0 || active.len() >= 64 {
            return Err(Error::Limit);
        }
        *budget -= 1;
        let id = std::ptr::from_ref(node) as usize;
        if let Some((_, before)) = active.iter().find(|(current, _)| *current == id) {
            return if progress > *before {
                Ok(1)
            } else {
                Err(Error::Schema)
            };
        }
        active.push((id, progress));
        let mut cost = 1usize;
        for reference in refs(node, self.draft)? {
            cost = cost.saturating_add(self.cost(
                self.target(reference)?,
                progress,
                active,
                budget,
            )?);
            if cost > 65536 {
                return Err(Error::Limit);
            }
        }
        for (child, edge) in children(node, self.draft)? {
            if edge == Edge::Definition {
                continue;
            }
            cost = cost.saturating_add(self.cost(
                child,
                progress + usize::from(edge == Edge::Descend),
                active,
                budget,
            )?);
            if cost > 65536 {
                return Err(Error::Limit);
            }
        }
        active.pop();
        Ok(cost)
    }
}
pub(super) fn analyze(root: &Value, draft: Draft) -> Result<(usize, usize), Error> {
    let mut graph = Graph {
        root,
        draft,
        nodes: Vec::new(),
        seen: HashSet::new(),
        anchors: HashMap::new(),
    };
    graph.collect(root)?;
    let mut index = 0;
    while index < graph.nodes.len() {
        let node = graph.nodes[index];
        index += 1;
        for reference in refs(node, draft)? {
            let decoded = fragment(reference)?;
            if decoded.is_empty() || decoded.starts_with('/') {
                graph.collect(graph.target(reference)?)?;
            }
        }
    }
    let mut maximum = 1;
    let mut budget = 1_000_000;
    let mut patterns = HashSet::new();
    for node in &graph.nodes {
        if let Some(pattern) = node.get("pattern") {
            patterns.insert(pattern.as_str().ok_or(Error::Schema)?);
        }
        if let Some(map) = node.get("patternProperties").and_then(Value::as_object) {
            patterns.extend(map.keys().map(String::as_str));
        }
        maximum = maximum.max(graph.cost(node, 0, &mut Vec::new(), &mut budget)?);
    }
    Ok((maximum, patterns.len()))
}
