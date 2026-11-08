use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub const API_VERSION: u32 = 1;
pub const MAX_OBJECTS: usize = 10_000;
pub const MAX_EDGES: usize = 50_000;
pub const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Markdown,
    List,
    Enum,
    Bool,
    Number,
}

impl FieldKind {
    pub fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Text | Self::Markdown | Self::Enum => value.is_string(),
            Self::List => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            Self::Bool => value.is_boolean(),
            Self::Number => value.is_number(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
    pub id: String,
    pub label: String,
    #[serde(default = "default_field_kind")]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    /// Allowed values for `FieldKind::Enum`.
    #[serde(default)]
    pub values: Vec<String>,
}

fn default_field_kind() -> FieldKind {
    FieldKind::Text
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationDef {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub from_kinds: Vec<String>,
    #[serde(default)]
    pub to_kinds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeKind {
    pub id: String,
    pub label: String,
    pub directory: String,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub default_status: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    #[serde(default)]
    pub relations: Vec<String>,
}

impl KnowledgeKind {
    pub fn has_status(&self, status: &str) -> bool {
        self.statuses.is_empty() || self.statuses.iter().any(|known| known == status)
    }

    pub fn field(&self, id: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.id == id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSchema {
    #[serde(default = "default_api")]
    pub api: u32,
    #[serde(default)]
    pub kinds: Vec<KnowledgeKind>,
    #[serde(default)]
    pub relations: Vec<RelationDef>,
}

fn default_api() -> u32 {
    API_VERSION
}

impl KnowledgeSchema {
    pub fn kind(&self, id: &str) -> Option<&KnowledgeKind> {
        self.kinds.iter().find(|kind| kind.id == id)
    }

    pub fn relation(&self, id: &str) -> Option<&RelationDef> {
        self.relations.iter().find(|relation| relation.id == id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.api != API_VERSION {
            return Err(format!(
                "Knowledge schema API {} is not supported (expected {API_VERSION})",
                self.api
            ));
        }
        if self.kinds.is_empty() {
            return Err("Knowledge schema has no kinds".into());
        }
        let mut kind_ids = HashSet::new();
        for kind in &self.kinds {
            if !valid_id(&kind.id) {
                return Err(format!("Invalid knowledge kind id: {}", kind.id));
            }
            if !kind_ids.insert(kind.id.as_str()) {
                return Err(format!("Duplicate knowledge kind: {}", kind.id));
            }
            if kind.label.trim().is_empty() || kind.label.len() > 120 {
                return Err(format!("Knowledge kind {} has an invalid label", kind.id));
            }
            if kind.directory.trim().is_empty()
                || kind.directory.starts_with('/')
                || kind.directory.contains("..")
                || kind.directory.contains('\\')
            {
                return Err(format!(
                    "Knowledge kind {} has an invalid directory",
                    kind.id
                ));
            }
            let mut statuses = HashSet::new();
            for status in &kind.statuses {
                if status.trim().is_empty() || !statuses.insert(status.as_str()) {
                    return Err(format!(
                        "Knowledge kind {} has an empty or duplicate status",
                        kind.id
                    ));
                }
            }
            if !kind.default_status.is_empty() && !kind.has_status(&kind.default_status) {
                return Err(format!(
                    "Knowledge kind {} has a default status outside its status list",
                    kind.id
                ));
            }
            let mut fields = HashSet::new();
            for field in &kind.fields {
                if !valid_id(&field.id) || !fields.insert(field.id.as_str()) {
                    return Err(format!(
                        "Knowledge kind {} has an invalid or duplicate field",
                        kind.id
                    ));
                }
                if field.kind == FieldKind::Enum && field.values.is_empty() {
                    return Err(format!(
                        "Knowledge kind {} field {} is an enum without values",
                        kind.id, field.id
                    ));
                }
            }
        }
        let mut relation_ids = HashSet::new();
        for relation in &self.relations {
            if !valid_id(&relation.id) || !relation_ids.insert(relation.id.as_str()) {
                return Err(format!(
                    "Invalid or duplicate knowledge relation: {}",
                    relation.id
                ));
            }
            if relation.label.trim().is_empty() {
                return Err(format!("Knowledge relation {} has no label", relation.id));
            }
            for kind in relation.from_kinds.iter().chain(&relation.to_kinds) {
                if !kind_ids.contains(kind.as_str()) {
                    return Err(format!(
                        "Knowledge relation {} refers to unknown kind {}",
                        relation.id, kind
                    ));
                }
            }
        }
        for kind in &self.kinds {
            for relation in &kind.relations {
                if !relation_ids.contains(relation.as_str()) {
                    return Err(format!(
                        "Knowledge kind {} refers to unknown relation {}",
                        kind.id, relation
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeObject {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub path: Option<String>,
    pub revision: String,
    pub updated_ms: i64,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default = "default_api")]
    pub schema_version: u32,
}

impl KnowledgeObject {
    pub fn validate(&self, schema: &KnowledgeSchema) -> Vec<String> {
        let mut errors = Vec::new();
        if !valid_id(&self.id) {
            errors.push(format!("Invalid object id: {}", self.id));
        }
        if self.project_id.trim().is_empty() {
            errors.push("Object has no project id".into());
        }
        if self.title.trim().is_empty() || self.title.len() > 240 {
            errors.push(format!("Object {} has an invalid title", self.id));
        }
        if self.body.len() > MAX_BODY_BYTES {
            errors.push(format!(
                "Object {} body exceeds {} KiB",
                self.id,
                MAX_BODY_BYTES / 1024
            ));
        }
        let Some(kind) = schema.kind(&self.kind) else {
            errors.push(format!("Object {} has unknown kind {}", self.id, self.kind));
            return errors;
        };
        if !kind.has_status(&self.status) {
            errors.push(format!(
                "Object {} has status {} outside kind {}",
                self.id, self.status, self.kind
            ));
        }
        for field in &kind.fields {
            let value = self.fields.get(&field.id);
            if field.required && value.is_none_or(is_empty_value) {
                errors.push(format!(
                    "Object {} is missing required field {}",
                    self.id, field.id
                ));
                continue;
            }
            if let Some(value) = value
                && !field.kind.accepts(value)
            {
                errors.push(format!(
                    "Object {} field {} has the wrong type",
                    self.id, field.id
                ));
            }
            if field.kind == FieldKind::Enum
                && let Some(value) = value.and_then(Value::as_str)
                && !field.values.iter().any(|allowed| allowed == value)
            {
                errors.push(format!(
                    "Object {} field {} is not one of the allowed values",
                    self.id, field.id
                ));
            }
        }
        errors
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeEdge {
    pub from_id: String,
    pub to_id: String,
    pub relation: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    #[serde(default)]
    pub objects: Vec<KnowledgeObject>,
    #[serde(default)]
    pub edges: Vec<KnowledgeEdge>,
}

impl KnowledgeGraph {
    pub fn get(&self, id: &str) -> Option<&KnowledgeObject> {
        self.objects.iter().find(|object| object.id == id)
    }

    pub fn by_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a KnowledgeObject> {
        self.objects
            .iter()
            .filter(move |object| object.kind == kind)
    }

    pub fn edges_from<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a KnowledgeEdge> {
        self.edges.iter().filter(move |edge| edge.from_id == id)
    }

    pub fn edges_to<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a KnowledgeEdge> {
        self.edges.iter().filter(move |edge| edge.to_id == id)
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
}
