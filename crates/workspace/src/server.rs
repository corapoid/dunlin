//! Wire contract shared by the relay and its management surface. No credentials.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Harness {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub isolated_agents: bool,
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub harnesses: Vec<Harness>,
    pub default_harness: String,
    pub account_scope: String,
    pub sandbox_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub output: String,
    pub finished: bool,
    pub success: bool,
}

pub const HARNESSES: &[(&str, &str, &str)] = &[
    ("opencode", "OpenCode", "opencode acp"),
    ("codex", "Codex", "codex-acp"),
    ("claude", "Claude Code", "claude-agent-acp"),
];

pub fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_cannot_escape_the_server_root() {
        for name in ["", ".", "..", "../repo", "/repo", "a\\b", "a\n", "é"] {
            assert!(!super::component(name), "{name:?}");
        }
        assert!(super::component("studio-mac"));
    }
}
