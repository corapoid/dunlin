//! Bounded agent profile data. Skills contain literal, explicitly versioned
//! instructions; this module reads no files, fetches no URLs and runs no code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const AGENT_SCHEMA_VERSION: u16 = 1;
pub const AGENT_MCP_SCHEMA_VERSION: u16 = 2;
pub const AGENT_HARNESS_IDS: &[&str] = &["opencode", "codex", "claude"];
/// Transport/storage must enforce this encoded JSON limit before parsing.
pub const MAX_AGENT_DEFINITION_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_INSTRUCTIONS_BYTES: usize = 16 * 1024;
pub const MAX_AGENT_SKILLS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileRef {
    pub agent_id: String,
    pub version: u64,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSkill {
    pub id: String,
    /// Explicit immutable version label, for example `1.2.0` or a content digest.
    pub version: String,
    pub instructions: String,
}

/// Team-owned policy only. Personal connection IDs, URLs and credentials do not
/// belong to a published agent. Grants are exact, including the tool definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMcpTool {
    pub provider_id: String,
    pub name: String,
    pub definition_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub schema_version: u16,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A fixed server harness ID, never a command or host executable path.
    pub harness: String,
    pub model: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<AgentMcpTool>,
    pub max_seconds: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentValidationError {
    pub path: String,
    pub message: String,
}
fn invalid(path: impl Into<String>, message: &str) -> AgentValidationError {
    AgentValidationError {
        path: path.into(),
        message: message.into(),
    }
}
fn text(value: &str, limit: usize, multiline: bool) -> bool {
    value.len() <= limit
        && !value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
}
fn token(value: &str, limit: usize, dotted: bool) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_') || dotted && c == b'.')
}
impl AgentDefinition {
    pub fn validate(&self) -> Result<(), AgentValidationError> {
        if !matches!(
            self.schema_version,
            AGENT_SCHEMA_VERSION | AGENT_MCP_SCHEMA_VERSION
        ) {
            return Err(invalid(
                "schema_version",
                "Unsupported agent schema version",
            ));
        }
        if self.mcp.len() > 32
            || !self.mcp.is_empty() && self.schema_version != AGENT_MCP_SCHEMA_VERSION
        {
            return Err(invalid(
                "mcp",
                "MCP requires agent schema 2 and at most 32 exact tool grants",
            ));
        }
        let mut mcp_names = BTreeSet::new();
        let mut providers = BTreeSet::new();
        for (index, tool) in self.mcp.iter().enumerate() {
            providers.insert(&tool.provider_id);
            if !token(&tool.provider_id, 64, false)
                || !text(&tool.name, 256, false)
                || tool.name.trim().is_empty()
                || tool.name.contains(['*', '?'])
                || !mcp_names.insert((&tool.provider_id, &tool.name))
                || tool.definition_digest.len() != 64
                || !tool
                    .definition_digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid(
                    format!("mcp[{index}]"),
                    "Unique provider/tool and a lowercase SHA-256 definition digest are required; wildcards are unsupported",
                ));
            }
        }
        if providers.len() > 8 {
            return Err(invalid("mcp", "At most eight MCP providers are supported"));
        }
        if self.name.trim().is_empty() || !text(&self.name, 128, false) {
            return Err(invalid(
                "name",
                "A nonempty agent name of at most 128 UTF-8 bytes is required",
            ));
        }
        if !text(&self.description, 4096, true) {
            return Err(invalid(
                "description",
                "Description exceeds 4096 UTF-8 bytes or contains control characters",
            ));
        }
        if !AGENT_HARNESS_IDS.contains(&self.harness.as_str()) {
            return Err(invalid("harness", "Unsupported harness ID"));
        }
        for (path, value, limit) in [
            ("model", self.model.as_str(), 256),
            ("mode", self.mode.as_str(), 128),
        ] {
            if value.trim().is_empty() || !text(value, limit, false) {
                return Err(invalid(
                    path,
                    "A bounded, nonempty value without control characters is required",
                ));
            }
        }
        if !(1..=1800).contains(&self.max_seconds) {
            return Err(invalid(
                "max_seconds",
                "Duration must be between 1 and 1800 seconds",
            ));
        }
        if !(1024..=4 * 1024 * 1024).contains(&self.max_output_bytes) {
            return Err(invalid(
                "max_output_bytes",
                "Output budget must be between 1024 bytes and 4 MiB",
            ));
        }
        let mut total = 0;
        if let Some(instructions) = &self.instructions {
            if !text(instructions, MAX_AGENT_INSTRUCTIONS_BYTES, true) {
                return Err(invalid(
                    "instructions",
                    "Instructions exceed 16 KiB or contain control characters",
                ));
            }
            total = instructions.len();
        }
        if self.skills.len() > MAX_AGENT_SKILLS {
            return Err(invalid(
                "skills",
                "At most eight embedded skills are supported",
            ));
        }
        let mut ids = BTreeSet::new();
        for (index, skill) in self.skills.iter().enumerate() {
            if !token(&skill.id, 128, false) || !ids.insert(skill.id.as_str()) {
                return Err(invalid(
                    format!("skills[{index}].id"),
                    "Skill IDs must be unique bounded ASCII identifiers",
                ));
            }
            if !token(&skill.version, 64, true) {
                return Err(invalid(
                    format!("skills[{index}].version"),
                    "An explicit bounded ASCII version label is required",
                ));
            }
            if skill.instructions.trim().is_empty() || !text(&skill.instructions, 8192, true) {
                return Err(invalid(
                    format!("skills[{index}].instructions"),
                    "Nonempty skill instructions of at most 8 KiB without control characters are required",
                ));
            }
            total += skill.instructions.len();
        }
        if total > MAX_AGENT_INSTRUCTIONS_BYTES {
            return Err(invalid(
                "instructions",
                "Combined agent and skill instructions exceed 16 KiB",
            ));
        }
        Ok(())
    }

    /// Exact policy match, not a credential, runtime admission or human approval.
    pub fn allows_mcp(&self, provider: &str, name: &str, definition_digest: &str) -> bool {
        self.validate().is_ok()
            && self.mcp.iter().any(|tool| {
                tool.provider_id == provider
                    && tool.name == name
                    && tool.definition_digest == definition_digest
            })
    }

    /// Call `validate` first. Renders ordered literal text for runtime context;
    /// it makes no system-role, tool permission or external skill-loading claim.
    pub fn render_instructions(&self) -> String {
        let mut rendered = self.instructions.clone().unwrap_or_default();
        for skill in &self.skills {
            if !rendered.is_empty() {
                rendered.push_str("\n\n");
            }
            rendered.push_str("[Skill ");
            rendered.push_str(&skill.id);
            rendered.push('@');
            rendered.push_str(&skill.version);
            rendered.push_str("]\n");
            rendered.push_str(&skill.instructions);
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> AgentDefinition {
        AgentDefinition {
            schema_version: AGENT_SCHEMA_VERSION,
            name: "Reviewer".into(),
            description: String::new(),
            harness: "codex".into(),
            model: "provider/model-id".into(),
            mode: "read-only".into(),
            instructions: Some("Check the patch.".into()),
            skills: vec![AgentSkill {
                id: "review".into(),
                version: "1.2.0".into(),
                instructions: "Report concrete failures.".into(),
            }],
            max_seconds: 1800,
            mcp: vec![],
            max_output_bytes: 4 * 1024 * 1024,
        }
    }
    #[test]
    fn bounded_definition_roundtrips_and_renders_exact_versioned_literal_context() {
        let d = definition();
        d.validate().unwrap();
        assert_eq!(
            d.render_instructions(),
            "Check the patch.\n\n[Skill review@1.2.0]\nReport concrete failures."
        );
        let encoded = serde_json::to_vec(&d).unwrap();
        assert!(encoded.len() < MAX_AGENT_DEFINITION_BYTES);
        assert_eq!(
            serde_json::from_slice::<AgentDefinition>(&encoded).unwrap(),
            d
        );
        let mut empty = d.clone();
        empty.instructions = None;
        empty.skills.clear();
        empty.validate().unwrap();
        assert!(empty.render_instructions().is_empty());
        // Paths/URLs inside prose are literal text, never external references.
        empty.skills.push(AgentSkill {
            id: "literal".into(),
            version: "v1".into(),
            instructions: "Discuss src/lib.rs and https://example.invalid without fetching them."
                .into(),
        });
        empty.validate().unwrap();
        assert!(
            empty
                .render_instructions()
                .contains("https://example.invalid")
        );
    }
    #[test]
    fn mcp_policy_requires_schema_two_exact_unique_pins_and_keeps_legacy_bytes() {
        let legacy = definition();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(
            !serde_json::to_value(&legacy)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("mcp")
        );
        let mut raw = serde_json::to_value(&legacy).unwrap();
        raw["mcp"] = serde_json::json!([]);
        assert_eq!(
            serde_json::to_vec(&serde_json::from_value::<AgentDefinition>(raw).unwrap()).unwrap(),
            bytes
        );
        let pin = AgentMcpTool {
            provider_id: "work".into(),
            name: "lookup".into(),
            definition_digest: "a".repeat(64),
        };
        let mut d = legacy.clone();
        d.mcp.push(pin.clone());
        assert!(d.validate().is_err());
        d.schema_version = AGENT_MCP_SCHEMA_VERSION;
        d.validate().unwrap();
        assert!(d.allows_mcp("work", "lookup", &"a".repeat(64)));
        for (provider, name, digest) in [
            ("other", "lookup", "a".repeat(64)),
            ("work", "LOOKUP", "a".repeat(64)),
            ("work", "lookup", "b".repeat(64)),
        ] {
            assert!(!d.allows_mcp(provider, name, &digest));
        }
        assert!(!legacy.allows_mcp("work", "lookup", &"a".repeat(64)));
        d.mcp.push(pin);
        assert!(d.validate().is_err());
        for fault in ["provider", "name", "digest", "secret", "count", "providers"] {
            let mut d = d.clone();
            d.mcp.truncate(1);
            match fault {
                "provider" => d.mcp[0].provider_id = "https://endpoint.example".into(),
                "name" => d.mcp[0].name = "*".into(),
                "digest" => d.mcp[0].definition_digest = "A".repeat(64),
                "secret" => {
                    let mut raw = serde_json::to_value(&d).unwrap();
                    raw["mcp"][0]["connection_id"] = serde_json::json!("private");
                    assert!(serde_json::from_value::<AgentDefinition>(raw).is_err());
                    continue;
                }
                "count" => {
                    d.mcp = (0..33)
                        .map(|i| AgentMcpTool {
                            name: format!("tool-{i}"),
                            ..d.mcp[0].clone()
                        })
                        .collect()
                }
                _ => {
                    d.mcp = (0..9)
                        .map(|i| AgentMcpTool {
                            provider_id: format!("provider-{i}"),
                            ..d.mcp[0].clone()
                        })
                        .collect()
                }
            }
            assert!(d.validate().is_err(), "{fault}");
        }
    }
    #[test]
    fn unknown_execution_fields_and_unversioned_or_external_skills_are_rejected() {
        for field in ["command", "mcp", "a2a", "tools", "url", "path"] {
            let mut value = serde_json::to_value(definition()).unwrap();
            value[field] = serde_json::json!("unsupported");
            assert!(serde_json::from_value::<AgentDefinition>(value).is_err());
        }
        for field in ["url", "path", "command"] {
            let mut value = serde_json::to_value(definition()).unwrap();
            value["skills"][0][field] = serde_json::json!("unsupported");
            assert!(serde_json::from_value::<AgentDefinition>(value).is_err());
        }
        for version in ["", "../skill", "https://example.invalid/v1", "v1\nnext"] {
            let mut d = definition();
            d.skills[0].version = version.into();
            assert!(d.validate().is_err());
        }
        for harness in ["/bin/sh", "codex --unsafe", "unknown", "Codex"] {
            let mut d = definition();
            d.harness = harness.into();
            assert_eq!(d.validate().unwrap_err().path, "harness");
        }
    }
    #[test]
    fn utf8_byte_limits_controls_duplicates_and_combined_instruction_budget_are_enforced() {
        let mut d = definition();
        d.name = "界".repeat(43);
        assert_eq!(d.validate().unwrap_err().path, "name");
        let mut d = definition();
        d.skills[0].instructions.push('\u{1b}');
        assert!(d.validate().is_err());
        let mut d = definition();
        d.skills.push(d.skills[0].clone());
        assert_eq!(d.validate().unwrap_err().path, "skills[1].id");
        let mut d = definition();
        d.instructions = Some("x".repeat(MAX_AGENT_INSTRUCTIONS_BYTES));
        assert_eq!(d.validate().unwrap_err().path, "instructions");
        let mut d = definition();
        d.instructions = None;
        d.skills[0].instructions = "x".repeat(8192);
        d.skills.push(AgentSkill {
            id: "two".into(),
            version: "1".into(),
            instructions: "y".repeat(8192),
        });
        d.validate().unwrap();
        assert!(d.render_instructions().len() < 20 * 1024);
        d.skills[1].instructions.push('z');
        assert!(d.validate().is_err());
        let mut d = definition();
        d.skills = (0..9)
            .map(|index| AgentSkill {
                id: format!("skill-{index}"),
                version: "1".into(),
                instructions: "text".into(),
            })
            .collect();
        assert_eq!(d.validate().unwrap_err().path, "skills");
    }
    #[test]
    fn schema_and_runtime_limits_are_explicit_and_encoded_worst_case_fits_transport() {
        let mut d = definition();
        d.schema_version = 3;
        assert_eq!(d.validate().unwrap_err().path, "schema_version");
        for seconds in [0, 1801, u64::MAX] {
            let mut d = definition();
            d.max_seconds = seconds;
            assert_eq!(d.validate().unwrap_err().path, "max_seconds");
        }
        for bytes in [0, 1023, 4 * 1024 * 1024 + 1, usize::MAX] {
            let mut d = definition();
            d.max_output_bytes = bytes;
            assert_eq!(d.validate().unwrap_err().path, "max_output_bytes");
        }
        let mut d = definition();
        d.name = "\\".repeat(128);
        d.description = "\\".repeat(4096);
        d.model = "\\".repeat(256);
        d.mode = "\\".repeat(128);
        d.instructions = Some("\\".repeat(MAX_AGENT_INSTRUCTIONS_BYTES));
        d.skills.clear();
        d.validate().unwrap();
        assert!(serde_json::to_vec(&d).unwrap().len() <= MAX_AGENT_DEFINITION_BYTES);
        for harness in AGENT_HARNESS_IDS {
            let mut d = definition();
            d.harness = (*harness).into();
            d.validate().unwrap();
        }
    }
}
