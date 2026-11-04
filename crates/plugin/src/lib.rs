//! Declarative plugin packs. A pack contributes methodology metadata,
//! templates, commands and skill names; it never links GPUI and cannot execute
//! code. MCP declarations are data only until a connector host is added.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const API_VERSION: u32 = 1;
pub const MANIFEST: &str = "plugin.json";
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

pub const BUNDLED: &[(&str, &str)] = &[
    (
        "spec-kit",
        include_str!("../../../assets/plugins/spec-kit/plugin.json"),
    ),
    (
        "agent-contract",
        include_str!("../../../assets/plugins/agent-contract/plugin.json"),
    ),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "default_api")]
    pub api: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub contributes: Contributions,
    #[serde(default)]
    pub permissions: Permissions,
}

fn default_api() -> u32 {
    API_VERSION
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contributions {
    #[serde(default)]
    pub workflows: Vec<Workflow>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub views: Vec<String>,
    #[serde(default)]
    pub artifact_kinds: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub phases: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Permissions {
    #[serde(default)]
    pub project_read: Vec<String>,
    #[serde(default)]
    pub project_write: Vec<String>,
    #[serde(default)]
    pub agent_prompt: bool,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub process: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Bundled,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadedPack {
    pub manifest: Manifest,
    pub source: Source,
    /// Local directory for external packs; `None` for bundled content.
    pub directory: Option<String>,
    /// SHA-256 of the exact manifest bytes. Pinning is local evidence; this is
    /// not a publisher signature or a remote catalog trust decision.
    pub manifest_hash: String,
}

impl LoadedPack {
    pub fn summary(&self) -> String {
        let c = &self.manifest.contributes;
        let mut parts = Vec::new();
        if !c.workflows.is_empty() {
            parts.push(format!("{} workflow(s)", c.workflows.len()));
        }
        if !c.commands.is_empty() {
            parts.push(format!("{} command(s)", c.commands.len()));
        }
        if !c.skills.is_empty() {
            parts.push(format!("{} skill(s)", c.skills.len()));
        }
        if !c.mcp_servers.is_empty() {
            parts.push(format!("{} MCP server(s)", c.mcp_servers.len()));
        }
        if !c.views.is_empty() {
            parts.push(format!("{} view(s)", c.views.len()));
        }
        if parts.is_empty() {
            "metadata only".into()
        } else {
            parts.join(" · ")
        }
    }
    pub fn command_prompt(&self, command_id: &str) -> Option<&str> {
        self.manifest
            .contributes
            .commands
            .iter()
            .find(|command| command.id == command_id)
            .map(|command| command.prompt.as_str())
            .filter(|prompt| !prompt.trim().is_empty())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn validate(manifest: &Manifest) -> Result<(), String> {
    if !valid_id(&manifest.id) {
        return Err("Plugin id must be lowercase letters, digits and hyphens".into());
    }
    if manifest.name.trim().is_empty() || manifest.name.len() > 160 {
        return Err("Plugin name is required (up to 160 bytes)".into());
    }
    if manifest.version.trim().is_empty() || manifest.version.len() > 64 {
        return Err("Plugin version is required".into());
    }
    if manifest.api != API_VERSION {
        return Err(format!(
            "Plugin {} targets API {}, but this build supports {}",
            manifest.id, manifest.api, API_VERSION
        ));
    }
    let mut commands = HashSet::new();
    for command in &manifest.contributes.commands {
        if !valid_id(&command.id) {
            return Err(format!("Plugin {} has an invalid command id", manifest.id));
        }
        if !commands.insert(&command.id) {
            return Err(format!(
                "Plugin {} declares command {} twice",
                manifest.id, command.id
            ));
        }
        if command.label.trim().is_empty() {
            return Err(format!(
                "Plugin {} command {} has no label",
                manifest.id, command.id
            ));
        }
    }
    let mut workflows = HashSet::new();
    for workflow in &manifest.contributes.workflows {
        if !valid_id(&workflow.id) {
            return Err(format!("Plugin {} has an invalid workflow id", manifest.id));
        }
        if !workflows.insert(&workflow.id) {
            return Err(format!(
                "Plugin {} declares workflow {} twice",
                manifest.id, workflow.id
            ));
        }
    }
    let mut servers = HashSet::new();
    for server in &manifest.contributes.mcp_servers {
        if server.name.trim().is_empty() || !servers.insert(&server.name) {
            return Err(format!(
                "Plugin {} has an empty or duplicate MCP server name",
                manifest.id
            ));
        }
    }
    if manifest.permissions.process && manifest.permissions.project_write.iter().any(|p| p == "**")
    {
        return Err(format!(
            "Plugin {} requests process execution and full project writes; v1 packs cannot receive both",
            manifest.id
        ));
    }
    Ok(())
}

pub fn parse(contents: &str) -> Result<Manifest, String> {
    let manifest: Manifest = serde_json::from_str(contents)
        .map_err(|error| format!("Invalid plugin manifest: {error}"))?;
    validate(&manifest)?;
    Ok(manifest)
}

pub fn load_dir(directory: &Path) -> Result<Manifest, String> {
    load_dir_with_hash(directory).map(|(manifest, _)| manifest)
}

pub fn load_dir_with_hash(directory: &Path) -> Result<(Manifest, String), String> {
    let path = directory.join(MANIFEST);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("plugin.json must be a regular file".into());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("plugin.json is larger than 256 KiB".into());
    }
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let hash = format!("{:x}", Sha256::digest(contents.as_bytes()));
    Ok((parse(&contents)?, hash))
}

/// Discover bundled packs and optional external packs from one directory.
/// Bundled ids win over an external pack with the same id, so an installed
/// pack cannot silently impersonate a reviewed built-in pack.
pub fn discover(external: Option<&Path>) -> (Vec<LoadedPack>, Vec<String>) {
    let mut packs = Vec::new();
    let mut warnings = Vec::new();
    let mut ids = HashSet::new();
    for (expected, contents) in BUNDLED {
        match parse(contents) {
            Ok(manifest) if manifest.id == *expected => {
                ids.insert(manifest.id.clone());
                packs.push(LoadedPack {
                    manifest,
                    source: Source::Bundled,
                    directory: None,
                    manifest_hash: format!("{:x}", Sha256::digest(contents.as_bytes())),
                });
            }
            Ok(_) => warnings.push(format!("Bundled plugin {expected} has a mismatched id")),
            Err(error) => warnings.push(format!("Bundled plugin {expected}: {error}")),
        }
    }
    let Some(directory) = external else {
        return (packs, warnings);
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            if directory.exists() {
                warnings.push(format!(
                    "Cannot read plugin directory {}: {error}",
                    directory.display()
                ));
            }
            return (packs, warnings);
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        match load_dir_with_hash(&path) {
            Ok((manifest, manifest_hash)) => {
                if ids.contains(&manifest.id) {
                    warnings.push(format!(
                        "External plugin {} was ignored because a bundled pack owns that id",
                        manifest.id
                    ));
                    continue;
                }
                ids.insert(manifest.id.clone());
                packs.push(LoadedPack {
                    manifest,
                    source: Source::External,
                    directory: Some(path.to_string_lossy().to_string()),
                    manifest_hash,
                });
            }
            Err(error) => warnings.push(format!("{}: {error}", path.display())),
        }
    }
    (packs, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn external(root: &Path, id: &str, body: &str) {
        let directory = root.join(id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(MANIFEST), body).unwrap();
    }

    #[test]
    fn bundled_packs_parse_and_describe_contributions() {
        let (packs, warnings) = discover(None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(packs.len(), 2);
        let spec_kit = packs
            .iter()
            .find(|pack| pack.manifest.id == "spec-kit")
            .unwrap();
        assert_eq!(spec_kit.source, Source::Bundled);
        assert!(spec_kit.summary().contains("command(s)"));
        assert!(spec_kit.command_prompt("speckit-plan").is_some());
        assert_eq!(spec_kit.manifest.api, 1);
    }

    #[test]
    fn invalid_or_duplicate_bundled_ids_are_rejected() {
        let manifest = parse(r#"{"id":"Bad_Id","name":"Bad","version":"1.0.0","api":1}"#);
        assert!(manifest.is_err());
        let manifest = parse(r#"{"id":"ok","name":"OK","version":"1.0.0","api":2}"#);
        assert!(manifest.is_err());
        let manifest = parse(
            r#"{"id":"ok","name":"OK","version":"1.0.0","api":1,"contributes":{"commands":[{"id":"run","label":"Run"},{"id":"run","label":"Again"}]}}"#,
        );
        assert!(manifest.is_err());
    }

    #[test]
    fn external_packs_are_discovered_but_cannot_shadow_bundled_ids() {
        let root = tempdir().unwrap();
        external(
            root.path(),
            "local-pack",
            r#"{"id":"local-pack","name":"Local","version":"1.0.0","api":1,"contributes":{"commands":[{"id":"local-run","label":"Local run","prompt":"/local"}]}}"#,
        );
        external(
            root.path(),
            "spec-kit",
            r#"{"id":"spec-kit","name":"Shadow","version":"9.0.0","api":1}"#,
        );
        let (packs, warnings) = discover(Some(root.path()));
        assert_eq!(packs.len(), 3);
        let local = packs
            .iter()
            .find(|pack| pack.manifest.id == "local-pack")
            .unwrap();
        assert_eq!(local.source, Source::External);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("bundled pack owns"))
        );
        assert_eq!(
            packs
                .iter()
                .find(|pack| pack.manifest.id == "spec-kit")
                .unwrap()
                .manifest
                .name,
            "Spec-Driven Development (Spec Kit)"
        );
    }

    #[test]
    fn process_plus_full_write_is_rejected() {
        let manifest = parse(
            r#"{"id":"danger","name":"Danger","version":"1.0.0","api":1,"permissions":{"process":true,"project_write":["**"]}}"#,
        );
        assert!(manifest.is_err());
    }
}
