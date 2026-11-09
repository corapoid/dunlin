//! A local instruction library with explicit, ownership-checked global installs.
use crate::app::DunlinApp;
use dunlin_http as ureq;
use dunlin_ui::{
    component::Disableable,
    component::button::{Button, ButtonVariants},
    component::input::{Editor, EditorState, InputState},
    prelude::FluentBuilder as _,
    *,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

pub(crate) const COMMIT_SKILL: &str = "conventional-commits";
pub(crate) const SECURITY_AUDIT_SKILL: &str = "cloudflare-security-audit";
pub(crate) const SPEC_KIT_SKILL: &str = "spec-driven-development";
pub(crate) const PLUGIN_BUILDER_SKILL: &str = "plugin-builder";
pub(crate) const COORDINATION_SKILL: &str = "agent-coordination";
const COORDINATION_INSTRUCTIONS: &str =
    include_str!("../../../assets/skills/agent-coordination.md");
const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_SKILL_FILE_BYTES: usize = 256 * 1024;
const MAX_SKILL_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_SKILL_FILES: usize = 128;
pub(crate) const COMMIT_INSTRUCTIONS: &str =
    include_str!("../../../assets/skills/conventional-commits.md");
const SPEC_KIT_INSTRUCTIONS: &str =
    include_str!("../../../assets/skills/spec-driven-development.md");
const PLUGIN_BUILDER_INSTRUCTIONS: &str = include_str!("../../../assets/skills/plugin-builder.md");

/// Pinned Cloudflare `security-audit-skill` bundle. Every companion file is
/// installed together so relative references in `SKILL.md` keep working.
/// Source: https://github.com/cloudflare/security-audit-skill
/// Commit: c1c8a8c1471069fb0e188eeaff69b8e8db6564a8
/// License: MIT (see the bundled LICENSE file).
const SECURITY_AUDIT_FILES: &[(&str, &str)] = &[
    (
        "AI-AND-LLM.md",
        include_str!("../../../assets/skills/security-audit/AI-AND-LLM.md"),
    ),
    (
        "ATTACK-CLASSES.md",
        include_str!("../../../assets/skills/security-audit/ATTACK-CLASSES.md"),
    ),
    (
        "CLIENT-SIDE.md",
        include_str!("../../../assets/skills/security-audit/CLIENT-SIDE.md"),
    ),
    (
        "CLOUD-AND-DEPLOYMENT.md",
        include_str!("../../../assets/skills/security-audit/CLOUD-AND-DEPLOYMENT.md"),
    ),
    (
        "DATA-ISOLATION-AND-LIFECYCLE.md",
        include_str!("../../../assets/skills/security-audit/DATA-ISOLATION-AND-LIFECYCLE.md"),
    ),
    (
        "DESKTOP-MOBILE-AND-LOCAL-IPC.md",
        include_str!("../../../assets/skills/security-audit/DESKTOP-MOBILE-AND-LOCAL-IPC.md"),
    ),
    (
        "HUNTING.md",
        include_str!("../../../assets/skills/security-audit/HUNTING.md"),
    ),
    (
        "LICENSE",
        include_str!("../../../assets/skills/security-audit/LICENSE"),
    ),
    (
        "MEMORY-SAFETY-AND-BINARY.md",
        include_str!("../../../assets/skills/security-audit/MEMORY-SAFETY-AND-BINARY.md"),
    ),
    (
        "PROTOCOLS-RPC-AND-MESSAGING.md",
        include_str!("../../../assets/skills/security-audit/PROTOCOLS-RPC-AND-MESSAGING.md"),
    ),
    (
        "RECONNAISSANCE.md",
        include_str!("../../../assets/skills/security-audit/RECONNAISSANCE.md"),
    ),
    (
        "RESOURCE-EXHAUSTION-AND-AVAILABILITY.md",
        include_str!(
            "../../../assets/skills/security-audit/RESOURCE-EXHAUSTION-AND-AVAILABILITY.md"
        ),
    ),
    (
        "SKILL.md",
        include_str!("../../../assets/skills/security-audit/SKILL.md"),
    ),
    (
        "SUPPLY-CHAIN-AND-RELEASE.md",
        include_str!("../../../assets/skills/security-audit/SUPPLY-CHAIN-AND-RELEASE.md"),
    ),
    (
        "VALIDATION-AND-REPORTING.md",
        include_str!("../../../assets/skills/security-audit/VALIDATION-AND-REPORTING.md"),
    ),
    (
        "WEB-PROTOCOL-AND-AUTH.md",
        include_str!("../../../assets/skills/security-audit/WEB-PROTOCOL-AND-AUTH.md"),
    ),
    (
        "report-schema.json",
        include_str!("../../../assets/skills/security-audit/report-schema.json"),
    ),
    (
        "validate-coverage-ledger.cjs",
        include_str!("../../../assets/skills/security-audit/validate-coverage-ledger.cjs"),
    ),
    (
        "validate-coverage-ledger.test.cjs",
        include_str!("../../../assets/skills/security-audit/validate-coverage-ledger.test.cjs"),
    ),
    (
        "validate-findings.cjs",
        include_str!("../../../assets/skills/security-audit/validate-findings.cjs"),
    ),
    (
        "validate-findings.test.cjs",
        include_str!("../../../assets/skills/security-audit/validate-findings.test.cjs"),
    ),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Scope {
    #[default]
    Global,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    /// One canonical global bundle, shared by every supported harness.
    Shared,
    OpenCode,
    Claude,
    Codex,
    Other,
}
impl Target {
    pub const ALL: [Self; 3] = [Self::OpenCode, Self::Claude, Self::Codex];
    pub(crate) fn from_harness(name: &str) -> Self {
        match name {
            "OpenCode" => Self::OpenCode,
            "Claude Code" => Self::Claude,
            "Codex" | "OpenAI Codex" => Self::Codex,
            _ => Self::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Shared => "All harnesses",
            Self::OpenCode => "OpenCode",
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Other => "Other",
        }
    }
    pub(crate) fn project_directory(self, root: &Path) -> Result<PathBuf, String> {
        let relative = match self {
            Self::Shared => ".agents/skills",
            Self::OpenCode => ".opencode/skills",
            Self::Claude => ".claude/skills",
            Self::Codex => ".agents/skills",
            Self::Other => return Err("Choose an explicit project skills directory".into()),
        };
        let path = root.join(relative);
        if !path.is_absolute() {
            return Err("Project skills directories must be absolute".into());
        }
        Ok(path)
    }
    fn directory(self) -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("Home directory is unavailable")?;
        let path = match self {
            Self::Shared => home.join(".agents/skills"),
            Self::OpenCode => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode/skills"),
            Self::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"))
                .join("skills"),
            Self::Codex => home.join(".agents/skills"),
            Self::Other => return Err("Choose this harness's global skills directory".into()),
        };
        if !path.is_absolute() {
            return Err("Global skill directories must be absolute".into());
        }
        Ok(path)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Installation {
    pub target: Target,
    /// Path to the installed `SKILL.md`.
    pub path: String,
    /// Installed `SKILL.md` contents. Kept for older records and for a quick
    /// ownership check; companion files are recorded in `files`.
    pub contents: String,
    #[serde(default)]
    pub files: Vec<InstalledFile>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub project_id: String,
    /// Harness discovery link to the canonical bundle, never a second copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_to: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct InstalledFile {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SkillFile {
    /// Relative path below the skill directory, using `/` separators.
    pub path: String,
    pub contents: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub files: Vec<SkillFile>,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub installations: Vec<Installation>,
}
impl Skill {
    pub fn rendered(&self) -> Result<String, String> {
        if self.id.is_empty()
            || self.id.len() > 48
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("Invalid skill ID".into());
        }
        if self.title.trim().is_empty()
            || self.description.trim().is_empty()
            || self.body.trim().is_empty()
        {
            return Err("A skill needs a name, description and instructions".into());
        }
        if self.title.len() > 120 || self.description.len() > 1024 {
            return Err("Use a name up to 120 bytes and a description up to 1,024 bytes".into());
        }
        let contents = format!(
            "---\nname: dunlin-{}\ndescription: {}\n{}---\n\n{}\n",
            self.id,
            serde_json::to_string(&self.description).map_err(|e| e.to_string())?,
            if self.id == COMMIT_SKILL {
                "disable-model-invocation: true\n"
            } else {
                ""
            },
            self.body.trim()
        );
        if contents.len() > MAX_SKILL_BYTES {
            return Err("A skill must fit in 64 KiB".into());
        }
        Ok(contents)
    }

    /// Complete portable bundle: `SKILL.md` first, then supporting files sorted
    /// by their relative path. Every path and size is validated before write.
    pub(crate) fn rendered_files(&self) -> Result<Vec<(String, String)>, String> {
        let skill_md = self.rendered()?;
        let mut files = vec![("SKILL.md".to_string(), skill_md)];
        let mut seen = std::collections::HashSet::new();
        seen.insert("SKILL.md".to_string());
        let mut total = files[0].1.len();
        for file in &self.files {
            if !valid_relative_path(&file.path) {
                return Err(format!("Unsafe supporting file path: {}", file.path));
            }
            if !seen.insert(file.path.clone()) {
                return Err(format!("Duplicate supporting file: {}", file.path));
            }
            if file.contents.len() > MAX_SKILL_FILE_BYTES {
                return Err(format!(
                    "Supporting file {} is larger than 256 KiB",
                    file.path
                ));
            }
            total += file.contents.len();
            files.push((file.path.clone(), file.contents.clone()));
        }
        if files.len() > MAX_SKILL_FILES {
            return Err(format!(
                "A skill bundle is limited to {MAX_SKILL_FILES} files"
            ));
        }
        if total > MAX_SKILL_TOTAL_BYTES {
            return Err("A skill bundle is limited to 2 MiB".into());
        }
        files[1..].sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    pub(crate) fn bundle_size(&self) -> usize {
        self.rendered()
            .map(|contents| {
                contents.len()
                    + self
                        .files
                        .iter()
                        .map(|file| file.contents.len())
                        .sum::<usize>()
            })
            .unwrap_or(0)
    }

    pub(crate) fn content_hash(&self) -> String {
        let Ok(files) = self.rendered_files() else {
            return String::new();
        };
        let mut hasher = Sha256::new();
        for (path, contents) in files {
            hasher.update(path.as_bytes());
            hasher.update([0]);
            hasher.update(contents.as_bytes());
            hasher.update([0]);
        }
        hex::encode(hasher.finalize())
    }
}
fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path.split('/').count() <= 32
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.ends_with([' ', '.'])
                && !segment.contains(':')
        })
}
fn file_hash(contents: &str) -> String {
    hex::encode(Sha256::digest(contents.as_bytes()))
}
fn installation_files(installation: &Installation) -> Vec<InstalledFile> {
    if installation.files.is_empty() {
        vec![InstalledFile {
            path: "SKILL.md".into(),
            sha256: file_hash(&installation.contents),
        }]
    } else {
        installation.files.clone()
    }
}
fn installation_matches(skill: &Skill, installation: &Installation) -> bool {
    let Ok(skill_md) = skill.rendered() else {
        return false;
    };
    let installed = installation_files(installation);
    installed.len() == skill.files.len() + 1
        && installed
            .iter()
            .any(|file| file.path == "SKILL.md" && file.sha256 == file_hash(&skill_md))
        && skill.files.iter().all(|file| {
            installed.iter().any(|installed| {
                installed.path == file.path && installed.sha256 == file_hash(&file.contents)
            })
        })
}

fn shared_installation_current(skill: &Skill) -> bool {
    [Target::Shared, Target::OpenCode, Target::Claude]
        .into_iter()
        .all(|target| {
            skill.installations.iter().any(|i| {
                i.scope == Scope::Global
                    && i.target == target
                    && (target == Target::Shared || i.linked_to.is_some())
                    && installation_matches(skill, i)
            })
        })
}
pub(crate) fn defaults() -> Vec<Skill> {
    vec![Skill { id: COMMIT_SKILL.into(), title: "Conventional commits".into(), description: "Analyze repository changes and create focused Conventional Commits only when the user explicitly requests commits.".into(), body: COMMIT_INSTRUCTIONS.into(), files: Vec::new(), origin: "Built-in".into(), source_hash: String::new(), installations: Vec::new() }]
}
pub(crate) fn catalog_metadata() -> Vec<&'static Skill> {
    vec![
        security_audit_cached(),
        spec_kit_cached(),
        plugin_builder_cached(),
        coordination_cached(),
    ]
}
pub(crate) fn catalog_skill(id: &str) -> Option<Skill> {
    match id {
        COMMIT_SKILL => defaults().into_iter().next(),
        SECURITY_AUDIT_SKILL => Some(security_audit_cached().clone()),
        SPEC_KIT_SKILL => Some(spec_kit_cached().clone()),
        PLUGIN_BUILDER_SKILL => Some(plugin_builder_cached().clone()),
        COORDINATION_SKILL => Some(coordination_cached().clone()),
        _ => None,
    }
}
fn spec_kit_cached() -> &'static Skill {
    static CACHE: std::sync::OnceLock<Skill> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let mut skill = parse_skill_contents(SPEC_KIT_INSTRUCTIONS, SPEC_KIT_SKILL)
            .expect("bundled Spec Kit skill is valid");
        skill.title = "Spec-driven development".into();
        skill.origin = "Dunlin · bundled instruction · MIT".into();
        skill.source_hash = skill.content_hash();
        skill
    })
}
fn coordination_cached() -> &'static Skill {
    static CACHE: std::sync::OnceLock<Skill> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let mut skill = parse_skill_contents(COORDINATION_INSTRUCTIONS, COORDINATION_SKILL)
            .expect("bundled coordination skill is valid");
        skill.title = "Project agent coordination".into();
        skill.origin = "Dunlin · bundled instruction · MIT".into();
        skill.source_hash = skill.content_hash();
        skill
    })
}
fn plugin_builder_cached() -> &'static Skill {
    static CACHE: std::sync::OnceLock<Skill> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let mut skill = parse_skill_contents(PLUGIN_BUILDER_INSTRUCTIONS, PLUGIN_BUILDER_SKILL)
            .expect("bundled plugin-builder skill is valid");
        skill.title = "Build Dunlin plugins".into();
        skill.origin = "Dunlin · bundled instruction · MIT".into();
        skill.source_hash = skill.content_hash();
        skill
    })
}

fn security_audit_cached() -> &'static Skill {
    static CACHE: std::sync::OnceLock<Skill> = std::sync::OnceLock::new();
    CACHE.get_or_init(security_audit_skill)
}
fn security_audit_skill() -> Skill {
    let contents = SECURITY_AUDIT_FILES
        .iter()
        .find(|(path, _)| *path == "SKILL.md")
        .map(|(_, contents)| *contents)
        .unwrap_or_default();
    let mut skill = parse_skill_contents(contents, SECURITY_AUDIT_SKILL)
        .expect("bundled security-audit SKILL.md is valid");
    skill.title = "Security audit".into();
    skill.origin = "Cloudflare security-audit-skill · c1c8a8c · MIT".into();
    skill.files = SECURITY_AUDIT_FILES
        .iter()
        .filter(|(path, _)| *path != "SKILL.md")
        .map(|(path, contents)| SkillFile {
            path: (*path).to_string(),
            contents: (*contents).to_string(),
        })
        .collect();
    skill.source_hash = skill.content_hash();
    skill
}

fn frontmatter_value(front: &str, key: &str) -> Option<String> {
    let line = front.lines().find(|line| {
        line.strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(':'))
    })?;
    let value = line.split_once(':')?.1.trim();
    Some(
        serde_json::from_str::<String>(value)
            .unwrap_or_else(|_| value.trim_matches('"').to_string()),
    )
}

fn parse_skill_contents(contents: &str, fallback_id: &str) -> Result<Skill, String> {
    let normalized = contents.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or("SKILL.md needs YAML frontmatter")?;
    let (front, body) = rest
        .split_once("\n---\n")
        .ok_or("SKILL.md frontmatter is not closed")?;
    let name = frontmatter_value(front, "name").ok_or("SKILL.md needs a name")?;
    let description =
        frontmatter_value(front, "description").ok_or("SKILL.md needs a description")?;
    let raw_id = name.strip_prefix("dunlin-").unwrap_or(fallback_id);
    let mut id = raw_id
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    while id.starts_with('-') || id.ends_with('-') {
        id = id.trim_matches('-').to_string();
    }
    if id.is_empty() {
        id = "imported-skill".into();
    }
    id.truncate(48);
    let title = name
        .strip_prefix("dunlin-")
        .unwrap_or(&name)
        .replace('-', " ");
    Ok(Skill {
        id,
        title: if title.trim().is_empty() {
            fallback_id.replace('-', " ")
        } else {
            title
        },
        description,
        body: body.trim().to_string(),
        files: Vec::new(),
        origin: String::new(),
        source_hash: hex::encode(Sha256::digest(contents.as_bytes())),
        installations: Vec::new(),
    })
}

fn skill_from_directory(directory: &Path) -> Result<Skill, String> {
    let path = directory.join("SKILL.md");
    regular_file(&path)?;
    let contents = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let fallback = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("imported-skill");
    let fallback = fallback.strip_prefix("dunlin-").unwrap_or(fallback);
    let mut skill = parse_skill_contents(&contents, fallback)?;
    skill.files = read_supporting_files(directory)?;
    skill.origin = directory.to_string_lossy().to_string();
    skill.source_hash = skill.content_hash();
    Ok(skill)
}

/// Read a bounded, symlink-free tree of UTF-8 supporting files. The skill
/// directory itself is canonicalized by [`discover`] before this runs.
fn read_supporting_files(directory: &Path) -> Result<Vec<SkillFile>, String> {
    let mut files = Vec::new();
    let mut total = 0usize;
    let mut entries_seen = 0usize;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|e| e.to_string())? {
            entries_seen += 1;
            if entries_seen > 4096 {
                return Err("Skill folder contains too many entries".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.file_name().is_some_and(|name| {
                matches!(
                    name.to_str(),
                    Some(".git")
                        | Some(".svn")
                        | Some(".hg")
                        | Some("node_modules")
                        | Some(".DS_Store")
                )
            }) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if pending.len() + files.len() < MAX_SKILL_FILES {
                    pending.push(path);
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == "SKILL.md") {
                continue;
            }
            if metadata.len() > MAX_SKILL_FILE_BYTES as u64 {
                return Err(format!(
                    "Supporting file {} is larger than 256 KiB",
                    path.display()
                ));
            }
            let relative = path
                .strip_prefix(directory)
                .map_err(|_| "Supporting file escaped the skill directory")?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !valid_relative_path(&relative) {
                return Err(format!("Unsafe supporting file path: {relative}"));
            }
            let contents = fs::read_to_string(&path)
                .map_err(|_| format!("Supporting file {} is not UTF-8 text", relative))?;
            total += contents.len();
            if total > MAX_SKILL_TOTAL_BYTES {
                return Err("A skill bundle is limited to 2 MiB".into());
            }
            if files.len() + 1 > MAX_SKILL_FILES {
                return Err(format!(
                    "A skill bundle is limited to {MAX_SKILL_FILES} files"
                ));
            }
            files.push(SkillFile {
                path: relative,
                contents,
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Read bounded SKILL.md files from one skill directory or a directory of
/// skill directories. Never follows symlinks, executes scripts or reads
/// supporting files.
pub(crate) fn discover(path: &Path) -> Result<Vec<Skill>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Choose a regular skill directory".into());
    }
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let mut directories = Vec::new();
    if path.join("SKILL.md").is_file() {
        directories.push(path);
    } else {
        for entry in fs::read_dir(&path).map_err(|e| e.to_string())?.take(128) {
            let entry = entry.map_err(|e| e.to_string())?;
            let child = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&child) else {
                continue;
            };
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && child.join("SKILL.md").is_file()
            {
                directories.push(child);
            }
        }
    }
    let mut skills = Vec::new();
    for directory in directories {
        if let Ok(skill) = skill_from_directory(&directory)
            && !skills
                .iter()
                .any(|existing: &Skill| existing.id == skill.id)
        {
            skills.push(skill);
        }
    }
    if skills.is_empty() {
        return Err("No readable SKILL.md files were found".into());
    }
    Ok(skills)
}

/// Discover skills already installed in the three default global harness
/// directories. Results are library imports, not ownership claims.
pub(crate) fn discover_global() -> Vec<Skill> {
    let mut skills = Vec::new();
    for target in Target::ALL {
        let Ok(directory) = target.directory() else {
            continue;
        };
        if !directory.is_dir() {
            continue;
        }
        let Ok(found) = discover(&directory) else {
            continue;
        };
        for mut skill in found {
            skill.origin = format!("{} · {}", target.label(), skill.origin);
            if !skills
                .iter()
                .any(|existing: &Skill| existing.id == skill.id)
            {
                skills.push(skill);
            }
        }
    }
    skills
}

fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Refusing a symlink or non-regular skill file".into());
    }
    if metadata.len() > MAX_SKILL_BYTES as u64 {
        return Err("Installed skill is unexpectedly large".into());
    }
    Ok(())
}
/// Create a directory one component at a time, refusing symlinks and files.
fn create_directory_chain(root: &Path, relative: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Unsafe supporting file path".into());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                let resolved = fs::canonicalize(&current).map_err(|e| e.to_string())?;
                if resolved != current {
                    return Err("Skill directory moved through a symlink".into());
                }
            }
            Ok(_) => return Err("Refusing a symlink or non-directory in a skill path".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn existing_parents_are_directories(root: &Path, relative: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Unsafe supporting file path".into());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                let resolved = fs::canonicalize(&current).map_err(|e| e.to_string())?;
                if resolved != current {
                    return Err("Skill directory moved through a symlink".into());
                }
            }
            Ok(_) => return Err("Refusing a symlink or non-directory in a skill path".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn preflight_file(
    path: &Path,
    relative: &str,
    recorded: &[InstalledFile],
    owned: bool,
    new_contents: &str,
) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            if !owned {
                return Err("A skill already exists here and is not owned by Dunlin".into());
            }
            let existing = fs::read_to_string(path).map_err(|e| e.to_string())?;
            let hash = file_hash(&existing);
            if hash == file_hash(new_contents)
                || recorded
                    .iter()
                    .any(|file| file.path == relative && file.sha256 == hash)
            {
                return Ok(());
            }
            Err("Installed skill file was changed outside Dunlin; preserve or move it before updating"
                .into())
        }
        Ok(_) => Err("Refusing a symlink or non-regular skill file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn write_bundle_file(root: &Path, relative: &str, contents: &str) -> Result<(), String> {
    let relative = Path::new(relative);
    let parent = relative.parent().unwrap_or(Path::new(""));
    create_directory_chain(root, parent)?;
    let path = root.join(relative);
    let directory = path.parent().ok_or("Missing skill file directory")?;
    let temporary = directory.join(format!(".dunlin-{}.tmp", uuid::Uuid::now_v7()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(contents.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, &path).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn cleanup_empty_directories(root: &Path, mut directory: &Path) {
    while let Ok(relative) = directory.strip_prefix(root) {
        if relative.as_os_str().is_empty() || fs::remove_dir(directory).is_err() {
            break;
        }
        let Some(parent) = directory.parent() else {
            break;
        };
        directory = parent;
    }
}

pub(crate) fn install_at(
    base: &Path,
    skill: &Skill,
    target: Target,
) -> Result<Installation, String> {
    let files = skill.rendered_files()?;
    let main_contents = files
        .iter()
        .find(|(path, _)| path == "SKILL.md")
        .map(|(_, contents)| contents.clone())
        .ok_or("A skill bundle needs SKILL.md")?;
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
    let directory = base.join(format!("dunlin-{}", skill.id));
    let path = directory.join("SKILL.md");
    let recorded = skill
        .installations
        .iter()
        .find(|i| Path::new(&i.path) == path);
    let recorded_files = recorded.map(installation_files).unwrap_or_default();
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("Refusing an existing skill-directory symlink or file".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }

    // Verify every existing managed file before changing anything. Files that
    // already match the new bundle are tolerated so an interrupted update can
    // be completed safely.
    for (relative, contents) in &files {
        if directory.exists() {
            existing_parents_are_directories(&directory, Path::new(relative))?;
            preflight_file(
                &directory.join(relative),
                relative,
                &recorded_files,
                recorded.is_some(),
                contents,
            )?;
        }
    }
    let expected = files
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut obsolete = Vec::new();
    for file in &recorded_files {
        if !valid_relative_path(&file.path) {
            return Err("Invalid managed skill path".into());
        }
        if expected.contains(file.path.as_str()) {
            continue;
        }
        let obsolete_path = directory.join(&file.path);
        match fs::symlink_metadata(&obsolete_path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let contents = fs::read_to_string(&obsolete_path).map_err(|e| e.to_string())?;
                if file_hash(&contents) != file.sha256 {
                    return Err(
                        "Installed skill file was changed outside Dunlin; preserve or move it before updating"
                            .into(),
                    );
                }
                obsolete.push(obsolete_path);
            }
            Ok(_) => return Err("Refusing a symlink or non-regular skill file".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    if !directory.exists() {
        let temporary = base.join(format!(".dunlin-{}.tmp-{}", skill.id, uuid::Uuid::now_v7()));
        fs::create_dir(&temporary).map_err(|e| e.to_string())?;
        let result = (|| {
            for (relative, contents) in &files {
                write_bundle_file(&temporary, relative, contents)?;
            }
            fs::rename(&temporary, &directory).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&temporary);
        }
        result?;
    } else {
        // Update companions first and SKILL.md last. A crash cannot make the
        // skill's entry point claim a version whose companions are not present.
        let mut ordered = files.clone();
        ordered.sort_by_key(|(path, _)| usize::from(path == "SKILL.md"));
        for (relative, contents) in &ordered {
            write_bundle_file(&directory, relative, contents)?;
        }
        for obsolete_path in obsolete {
            fs::remove_file(&obsolete_path).map_err(|e| e.to_string())?;
            if let Some(parent) = obsolete_path.parent()
                && parent != directory
            {
                cleanup_empty_directories(&directory, parent);
            }
        }
    }

    let installed_files = files
        .iter()
        .map(|(path, contents)| InstalledFile {
            path: path.clone(),
            sha256: file_hash(contents),
        })
        .collect::<Vec<_>>();
    for file in &installed_files {
        let installed =
            fs::read_to_string(directory.join(&file.path)).map_err(|e| e.to_string())?;
        if file_hash(&installed) != file.sha256 {
            return Err("Installed skill verification failed".into());
        }
    }
    Ok(Installation {
        target,
        path: path.to_string_lossy().into(),
        contents: main_contents,
        files: installed_files,
        scope: Scope::Global,
        project_id: String::new(),
        linked_to: None,
    })
}

/// Keep one bundle on disk. Harness-specific discovery directories only contain
/// links. Legacy managed copies are replaced only after their ownership checks.
fn install_shared_at(
    base: &Path,
    aliases: &[(Target, PathBuf)],
    skill: &Skill,
) -> (Vec<Installation>, Vec<String>) {
    let canonical = match install_at(base, skill, Target::Shared) {
        Ok(installation) => installation,
        Err(error) => return (Vec::new(), vec![error]),
    };
    let directory = Path::new(&canonical.path).parent().unwrap();
    let mut installed = vec![canonical.clone()];
    let mut errors = Vec::new();
    for (target, base) in aliases {
        let result = (|| {
            fs::create_dir_all(base).map_err(|e| e.to_string())?;
            let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
            let alias = base.join(format!("dunlin-{}", skill.id));
            if alias == directory {
                return Ok(None);
            }
            match fs::symlink_metadata(&alias) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if fs::read_link(&alias).map_err(|e| e.to_string())? != directory {
                        return Err(
                            "An unrelated skill link already exists; it was preserved".into()
                        );
                    }
                    // Do not claim a pre-existing link unless previously recorded.
                    if !skill.installations.iter().any(|i| {
                        i.path == alias.join("SKILL.md").to_string_lossy()
                            && i.linked_to.as_deref() == Some(canonical.path.as_str())
                    }) {
                        return Err(
                            "An unmanaged skill link already exists; it was preserved".into()
                        );
                    }
                }
                Ok(_) => {
                    let record = skill
                        .installations
                        .iter()
                        .find(|i| {
                            i.scope == Scope::Global && Path::new(&i.path) == alias.join("SKILL.md")
                        })
                        .ok_or("An unmanaged skill already exists; it was preserved")?;
                    uninstall(record, &skill.id)?;
                    if alias.exists() {
                        return Err(
                            "The old skill folder contains user files; it was preserved".into()
                        );
                    }
                    std::os::unix::fs::symlink(directory, &alias).map_err(|e| e.to_string())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    std::os::unix::fs::symlink(directory, &alias).map_err(|e| e.to_string())?;
                }
                Err(e) => return Err(e.to_string()),
            }
            let mut link = canonical.clone();
            link.target = *target;
            link.path = alias.join("SKILL.md").to_string_lossy().into_owned();
            link.linked_to = Some(canonical.path.clone());
            Ok(Some(link))
        })();
        match result {
            Ok(Some(link)) => installed.push(link),
            Ok(None) => {}
            Err(error) => errors.push(format!("{}: {error}", target.label())),
        }
    }
    (installed, errors)
}

pub(crate) fn install_at_scoped(
    base: &Path,
    skill: &Skill,
    target: Target,
    scope: Scope,
    project_id: &str,
) -> Result<Installation, String> {
    let mut installation = install_at(base, skill, target)?;
    installation.scope = scope;
    installation.project_id = project_id.to_string();
    Ok(installation)
}

pub(crate) fn uninstall(installation: &Installation, id: &str) -> Result<(), String> {
    let path = Path::new(&installation.path);
    if !path.is_absolute()
        || path.file_name().is_none_or(|n| n != "SKILL.md")
        || path
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|n| n != format!("dunlin-{id}").as_str())
    {
        return Err("Invalid managed skill path".into());
    }
    let parent = path.parent().ok_or("Missing skill directory")?;
    if let Some(canonical) = &installation.linked_to {
        let destination = Path::new(canonical)
            .parent()
            .ok_or("Invalid shared skill path")?;
        match fs::symlink_metadata(parent) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && fs::read_link(parent).is_ok_and(|target| target == destination) =>
            {
                return fs::remove_file(parent).map_err(|e| e.to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            _ => return Err("The managed skill link was replaced; it was preserved".into()),
        }
    }
    if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing a symlinked skill directory".into());
    }
    match fs::canonicalize(parent) {
        Ok(resolved) if resolved != parent => {
            return Err(
                "Managed skill directory moved through a symlink; preserve it instead of deleting"
                    .into(),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let files = installation_files(installation);
    // Preflight the whole bundle before deleting anything.
    let mut existing = Vec::new();
    for file in &files {
        if !valid_relative_path(&file.path) {
            return Err("Invalid managed skill path".into());
        }
        let target = parent.join(&file.path);
        existing_parents_are_directories(parent, Path::new(&file.path))?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let contents = fs::read_to_string(&target).map_err(|e| e.to_string())?;
                if file_hash(&contents) != file.sha256 {
                    return Err(
                        "Installed skill was modified outside Dunlin and will not be deleted"
                            .into(),
                    );
                }
                existing.push(target);
            }
            Ok(_) => {
                return Err("Refusing a symlink or non-regular managed skill file".into());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    for target in existing {
        fs::remove_file(&target).map_err(|e| e.to_string())?;
        if let Some(directory) = target.parent()
            && directory != parent
        {
            cleanup_empty_directories(parent, directory);
        }
    }
    let _ = fs::remove_dir(parent); // Non-recursive: keep any supporting/user files.
    Ok(())
}

#[derive(Default)]
pub(crate) struct Ui {
    draft: Option<Draft>,
    pub(crate) busy: bool,
    installing: Option<String>,
    pub(crate) message: Option<String>,
}
impl Ui {
    pub(crate) fn has_draft(&self) -> bool {
        self.draft.is_some()
    }
    pub(crate) fn discard_draft(&mut self) {
        self.draft = None;
    }
    pub(crate) fn draft_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.draft
            .as_ref()
            .map(|draft| draft.title.read(cx).focus_handle(cx))
    }
}
struct Draft {
    id: String,
    title: Entity<InputState>,
    description: Entity<InputState>,
    body: Entity<EditorState>,
}
const MAX_GITHUB_SKILL_BYTES: usize = MAX_SKILL_BYTES;

fn github_skill_urls(value: &str) -> Result<Vec<Url>, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 {
        return Err("Paste a GitHub link to SKILL.md or a skill folder".into());
    }
    let url = Url::parse(value).map_err(|_| "Invalid GitHub URL".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use a plain HTTPS github.com or raw.githubusercontent.com link".into());
    }
    let host = url.host_str().unwrap_or_default();
    let segments = url
        .path_segments()
        .map(|segments| segments.filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    if host == "raw.githubusercontent.com" {
        let mut raw = url.clone();
        if segments.last() != Some(&"SKILL.md") {
            let path = format!("{}/SKILL.md", url.path().trim_end_matches('/'));
            raw.set_path(&path);
        }
        return Ok(vec![raw]);
    }
    if host != "github.com" {
        return Err("Only github.com or raw.githubusercontent.com links are supported".into());
    }
    if segments.len() < 2 {
        return Err("The GitHub link must include an owner and repository".into());
    }
    let (owner, repo) = (segments[0], segments[1]);
    let mut urls = Vec::new();
    match segments.get(2).copied() {
        Some("blob") if segments.len() >= 5 => {
            if segments.last() != Some(&"SKILL.md") {
                return Err("Link directly to SKILL.md or to the skill folder".into());
            }
            urls.push(raw_url(owner, repo, segments[3], &segments[4..].join("/"))?);
        }
        Some("tree") if segments.len() >= 4 => {
            let path = if segments.len() == 4 {
                "SKILL.md".to_string()
            } else {
                format!("{}/SKILL.md", segments[4..].join("/"))
            };
            urls.push(raw_url(owner, repo, segments[3], &path)?);
        }
        Some(_) | None => {
            let path = if segments.len() == 2 {
                "SKILL.md".to_string()
            } else {
                format!("{}/SKILL.md", segments[2..].join("/"))
            };
            for branch in ["main", "master"] {
                urls.push(raw_url(owner, repo, branch, &path)?);
            }
        }
    }
    Ok(urls)
}

fn raw_url(owner: &str, repo: &str, reference: &str, path: &str) -> Result<Url, String> {
    Url::parse(&format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{path}"
    ))
    .map_err(|_| "Could not build the GitHub raw URL".to_string())
}

fn fetch_github_skill(urls: &[Url]) -> Result<Skill, String> {
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(Duration::from_secs(20))
        .build();
    let mut last_error = None;
    for url in urls {
        let response = agent
            .get(url.as_str())
            .set("User-Agent", "Dunlin")
            .set("Accept", "text/plain")
            .call();
        match response {
            Ok(response) => {
                let mut text = String::new();
                response
                    .into_reader()
                    .take(MAX_GITHUB_SKILL_BYTES as u64 + 1)
                    .read_to_string(&mut text)
                    .map_err(|error| format!("Could not read SKILL.md: {error}"))?;
                if text.len() > MAX_GITHUB_SKILL_BYTES {
                    return Err("SKILL.md must fit in 64 KiB".into());
                }
                let mut skill = parse_skill_contents(&text, "github-skill")?;
                skill.rendered()?;
                skill.origin = url.to_string();
                return Ok(skill);
            }
            Err(ureq::Error::Status(404, _)) => {
                last_error = Some(format!("No SKILL.md at {url}"));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(format!("GitHub returned HTTP {status}"));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(format!("Could not reach GitHub: {error}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "No SKILL.md found at the linked GitHub location".into()))
}

impl DunlinApp {
    fn edit_skill(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let skill = id
            .and_then(|id| self.skills.iter().find(|s| s.id == id))
            .cloned();
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Skill name"));
        let description =
            cx.new(|cx| InputState::new(window, cx).placeholder("When to use this skill"));
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .soft_wrap(true)
        });
        if let Some(skill) = &skill {
            title.update(cx, |s, cx| s.set_value(skill.title.clone(), window, cx));
            description.update(cx, |s, cx| {
                s.set_value(skill.description.clone(), window, cx)
            });
            body.update(cx, |s, cx| s.set_value(skill.body.clone(), window, cx));
        }
        self.skills_ui.draft = Some(Draft {
            id: skill
                .map(|s| s.id)
                .unwrap_or_else(|| format!("skill-{}", uuid::Uuid::now_v7().simple())),
            title,
            description,
            body,
        });
        self.skills_ui.message = None;
        cx.notify();
    }
    fn import_skills(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Reading skill folders…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut found = Vec::new();
                    let mut errors = Vec::new();
                    for path in paths {
                        match discover(&path) {
                            Ok(skills) => found.extend(skills),
                            Err(error) => errors.push(format!("{}: {error}", path.display())),
                        }
                    }
                    (found, errors)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let (found, mut errors) = result;
                let mut added = 0;
                let mut skipped = 0;
                for skill in found {
                    if this.skills.iter().any(|existing| existing.id == skill.id) {
                        skipped += 1;
                    } else if this.skills.len() < 128 {
                        this.skills.push(skill);
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                }
                if added > 0 {
                    this.persist_settings(cx);
                }
                if !errors.is_empty() {
                    errors.truncate(3);
                    this.skills_ui.message = Some(format!(
                        "Imported {added}, skipped {skipped}. {}",
                        errors.join(" · ")
                    ));
                } else {
                    this.skills_ui.message = Some(format!(
                        "Imported {added} skill(s), skipped {skipped} existing or over-limit entries. Imported files are not claimed as Dunlin-managed until installed."
                    ));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_skill_from_github(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let value = self.skills_import_url.read(cx).value().to_string();
        let urls = match github_skill_urls(&value) {
            Ok(urls) => urls,
            Err(error) => {
                self.skills_ui.message = Some(error);
                cx.notify();
                return;
            }
        };
        self.skills_import_url
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Fetching skill from GitHub…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { fetch_github_skill(&urls) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                match result {
                    Ok(skill) => {
                        if this.skills.iter().any(|existing| existing.id == skill.id) {
                            this.skills_ui.message = Some(format!(
                                "Skill “{}” already exists in the library",
                                skill.title
                            ));
                        } else if this.skills.len() >= 128 {
                            this.skills_ui.message =
                                Some("The skill library is limited to 128 entries".into());
                        } else {
                            let title = skill.title.clone();
                            this.skills.push(skill);
                            this.persist_settings(cx);
                            this.skills_ui.message =
                                Some(format!("Imported “{title}” from GitHub"));
                        }
                    }
                    Err(error) => this.skills_ui.message = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scan_global_skills(&mut self, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Scanning global harness skill folders…".into());
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { discover_global() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let mut added = 0;
                let mut skipped = 0;
                for skill in found {
                    if this.skills.iter().any(|existing| existing.id == skill.id) {
                        skipped += 1;
                    } else if this.skills.len() < 128 {
                        this.skills.push(skill);
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                }
                if added > 0 {
                    this.persist_settings(cx);
                }
                this.skills_ui.message = Some(format!(
                    "Scanned global folders: imported {added}, skipped {skipped}. Discovery is read-only and does not claim ownership of existing files."
                ));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_skill(&mut self, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let Some(draft) = &self.skills_ui.draft else {
            return;
        };
        let mut skill = Skill {
            id: draft.id.clone(),
            title: draft.title.read(cx).value().trim().to_string(),
            description: draft.description.read(cx).value().trim().to_string(),
            body: draft.body.read(cx).value().to_string(),
            files: Vec::new(),
            origin: String::new(),
            source_hash: String::new(),
            installations: Vec::new(),
        };
        if let Err(e) = skill.rendered() {
            self.skills_ui.message = Some(e);
            cx.notify();
            return;
        }
        if let Some(old) = self.skills.iter_mut().find(|s| s.id == skill.id) {
            skill.installations = old.installations.clone();
            // The inline editor only edits SKILL.md. Supporting files stay in
            // the library and in the installed bundle until explicitly removed.
            skill.files = old.files.clone();
            skill.origin = if old.origin.is_empty() {
                "Manual".into()
            } else {
                old.origin.clone()
            };
            skill.source_hash = skill.content_hash();
            *old = skill;
        } else if self.skills.len() < 128 {
            skill.origin = "Manual".into();
            skill.source_hash = skill.content_hash();
            self.skills.push(skill);
        } else {
            self.skills_ui.message = Some("The library limit is 128 skills".into());
            cx.notify();
            return;
        }
        self.skills_ui.draft = None;
        self.skills_ui.message = Some(
            "Saved in the global library. Use Install globally to install or update the shared bundle."
                .into(),
        );
        self.persist_settings(cx);
        cx.notify();
    }
    fn install_skill(
        &mut self,
        id: &str,
        target: Target,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.install_skill_scoped(id, target, directory, Scope::Global, "", cx);
    }

    fn install_shared_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let Some(skill) = self.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        self.skills_ui.busy = true;
        self.skills_ui.installing = Some(id.to_string());
        self.skills_ui.message = Some("Installing shared global skill…".into());
        cx.spawn(async move |this, cx| {
            let id = skill.id.clone();
            let (installed, errors) = cx.background_executor().spawn(async move {
                let paths = (|| {
                    Ok::<_, String>((Target::Shared.directory()?, vec![
                        (Target::OpenCode, Target::OpenCode.directory()?),
                        (Target::Claude, Target::Claude.directory()?),
                    ]))
                })();
                match paths {
                    Ok((base, aliases)) => install_shared_at(&base, &aliases, &skill),
                    Err(error) => (Vec::new(), vec![error]),
                }
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                this.skills_ui.installing = None;
                if let Some(skill) = this.skills.iter_mut().find(|s| s.id == id) {
                    for installation in installed {
                        skill.installations.retain(|old| old.path != installation.path);
                        skill.installations.push(installation);
                    }
                    this.persist_settings(cx);
                }
                this.skills_ui.message = Some(if errors.is_empty() {
                    "Installed once globally for OpenCode, Claude Code and Codex. Restart existing harness sessions to discover it.".into()
                } else {
                    format!("Global installation needs attention: {}", errors.join(" · "))
                });
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(crate) fn install_skill_scoped(
        &mut self,
        id: &str,
        target: Target,
        directory: Option<PathBuf>,
        scope: Scope,
        project_id: &str,
        cx: &mut Context<Self>,
    ) {
        if scope == Scope::Global && directory.is_none() {
            self.install_shared_skill(id, cx);
            return;
        }
        if self.skills_ui.busy {
            return;
        }
        let Some(skill) = self.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let base = match directory.map(Ok).unwrap_or_else(|| {
            if scope == Scope::Project {
                let project = self.project();
                if project.is_remote() {
                    return Err("Project skill installation requires a local project".into());
                }
                target.project_directory(Path::new(&project.path))
            } else {
                target.directory()
            }
        }) {
            Ok(p) => p,
            Err(e) => {
                self.skills_ui.message = Some(e);
                cx.notify();
                return;
            }
        };
        let project_id = project_id.to_string();
        self.skills_ui.busy = true;
        self.skills_ui.message = Some(if scope == Scope::Project {
            "Installing project skill…".into()
        } else {
            "Installing skill…".into()
        });
        cx.spawn(async move |this, cx| {
            let id = skill.id.clone();
            let scope_for_result = scope;
            let result = cx.background_executor().spawn(async move {
                install_at_scoped(&base, &skill, target, scope, &project_id)
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                match result {
                    Ok(installation) => {
                        if let Some(skill) = this.skills.iter_mut().find(|s| s.id == id) {
                            skill.installations.retain(|i| i.path != installation.path);
                            skill.installations.push(installation);
                            this.persist_settings(cx);
                        }
                        this.skills_ui.message = Some(if scope_for_result == Scope::Project {
                            "Installed in this project. The harness may prefer it over the global copy; no harness program was installed or executed.".into()
                        } else {
                            "Installed globally. Project-level skills may override global definitions; no harness program was installed or executed.".into()
                        });
                    }
                    Err(e) => this.skills_ui.message = Some(e),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn remove_skill_installations(
        &mut self,
        id: &str,
        target: Option<Target>,
        delete: bool,
        cx: &mut Context<Self>,
    ) {
        self.remove_skill_installations_scoped(id, target, None, None, delete, cx);
    }

    fn remove_skill_installations_scoped(
        &mut self,
        id: &str,
        target: Option<Target>,
        scope: Option<Scope>,
        project_id: Option<&str>,
        delete: bool,
        cx: &mut Context<Self>,
    ) {
        if self.skills_ui.busy {
            return;
        }
        let Some(skill) = self.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let project_id = project_id.map(str::to_string);
        self.skills_ui.busy = true;
        cx.spawn(async move |this, cx| {
            let id = skill.id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut installations = skill
                        .installations
                        .iter()
                        .filter(|i| target.is_none_or(|target| i.target == target))
                        .filter(|i| scope.is_none_or(|scope| i.scope == scope))
                        .filter(|i| {
                            project_id
                                .as_deref()
                                .is_none_or(|project| i.project_id == project)
                        })
                        .collect::<Vec<_>>();
                    // Discovery links go first; never leave dangling links after
                    // removing their canonical bundle.
                    installations.sort_by_key(|i| i.target == Target::Shared);
                    let mut results = Vec::new();
                    for installation in installations {
                        if installation.target == Target::Shared
                            && results
                                .iter()
                                .any(|(_, result): &(String, Result<(), String>)| result.is_err())
                        {
                            continue;
                        }
                        results.push((
                            installation.path.clone(),
                            uninstall(installation, &skill.id),
                        ));
                    }
                    results
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let mut errors = Vec::new();
                if let Some(skill) = this.skills.iter_mut().find(|s| s.id == id) {
                    for (path, result) in result {
                        match result {
                            Ok(()) => skill.installations.retain(|i| i.path != path),
                            Err(e) => errors.push(e),
                        }
                    }
                }
                if delete && errors.is_empty() {
                    this.skills.retain(|s| s.id != id);
                }
                this.skills_ui.message = Some(if errors.is_empty() {
                    "Removed managed skill copies".into()
                } else {
                    errors.join("\n")
                });
                this.persist_settings(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn skill_in_library(&self, id: &str) -> bool {
        self.skills.iter().any(|skill| skill.id == id)
    }

    pub(crate) fn skill_installed_for(&self, id: &str, target: Target, project_id: &str) -> bool {
        self.skills.iter().any(|skill| {
            skill.id == id
                && skill.installations.iter().any(|installation| {
                    (installation.target == target
                        || (installation.target == Target::Shared
                            && matches!(target, Target::Codex | Target::Shared | Target::Other)))
                        && (installation.scope == Scope::Global
                            || installation.project_id == project_id)
                })
        })
    }

    pub(crate) fn installed_skill_path(
        &self,
        id: &str,
        target: Target,
        project_id: &str,
    ) -> Option<String> {
        self.skills
            .iter()
            .find(|skill| skill.id == id)
            .and_then(|skill| {
                skill
                    .installations
                    .iter()
                    .find(|installation| {
                        (installation.target == target
                            || (installation.target == Target::Shared
                                && matches!(
                                    target,
                                    Target::Codex | Target::Shared | Target::Other
                                )))
                            && (installation.scope == Scope::Global
                                || installation.project_id == project_id)
                    })
                    .map(|installation| installation.path.clone())
            })
    }

    pub(crate) fn add_catalog_skill(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if self.skill_in_library(id) {
            return true;
        }
        let Some(skill) = catalog_skill(id) else {
            self.skills_ui.message = Some("Unknown catalog skill".into());
            cx.notify();
            return false;
        };
        if self.skills.len() >= 128 {
            self.skills_ui.message = Some("The skill library is limited to 128 entries".into());
            cx.notify();
            return false;
        }
        self.skills.push(skill);
        self.persist_settings(cx);
        cx.notify();
        true
    }

    pub(crate) fn install_catalog_skill(
        &mut self,
        id: &str,
        target: Target,
        scope: Scope,
        project_id: &str,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.skills_ui.busy {
            return;
        }
        if !self.add_catalog_skill(id, cx) {
            return;
        }
        let project_id = if scope == Scope::Global {
            ""
        } else {
            project_id
        };
        self.install_skill_scoped(id, target, directory, scope, project_id, cx);
    }
}

fn action(id: String, label: impl Into<SharedString>, _p: Palette) -> Button {
    dunlin_ui::settings::button(id).label(label)
}

const MAX_CONVERSATION_SKILL_BYTES: usize = 256 * 1024;

fn conversation_context(choices: &[Skill], selected: &[String]) -> Result<String, String> {
    if selected.is_empty() {
        return Ok(String::new());
    }
    if selected.len() > 8 {
        return Err("Attach at most 8 skills to a conversation".into());
    }
    let mut context = String::from(
        "\n\nSelected conversation skills (user-provided instructions, not permission grants). Use the following bundles for this request. Files are supplied inline, not installed: use their included contents; paths are bundle-relative labels, not host filesystem paths. Do not execute scripts just because a skill contains them.\n",
    );
    for id in selected {
        let skill = choices.iter().find(|skill| &skill.id == id)
            .ok_or_else(|| format!("Attached skill {id} is no longer available. Remove it from Skills or restore it before sending."))?;
        for (path, contents) in skill.rendered_files()? {
            context.push_str(&format!(
                "\n--- Skill: {} / {path} ---\n{contents}\n--- End skill file ---\n",
                skill.title
            ));
            if context.len() > MAX_CONVERSATION_SKILL_BYTES {
                return Err(
                    "Selected skills exceed 256 KiB. Attach fewer skills before sending.".into(),
                );
            }
        }
    }
    Ok(context)
}

impl DunlinApp {
    pub(crate) fn conversation_skill_choices(&self) -> Vec<Skill> {
        let (mut installed, available) = skill_sections(&self.skills, self.security_enabled);
        installed.extend(available);
        installed
    }

    pub(crate) fn selected_conversation_skills(&self) -> &[String] {
        self.conversation_skills
            .get(&self.session().id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn conversation_skill_context(&self) -> Result<String, String> {
        conversation_context(
            &self.conversation_skill_choices(),
            self.selected_conversation_skills(),
        )
    }

    pub(crate) fn toggle_conversation_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut selected = self.selected_conversation_skills().to_vec();
        if selected.iter().any(|selected| selected == id) {
            selected.retain(|selected| selected != id);
        } else {
            selected.push(id.to_string());
            if let Err(error) = conversation_context(&self.conversation_skill_choices(), &selected)
            {
                self.set_notice(error, cx);
                return;
            }
        }
        let session = self.session().id.clone();
        if selected.is_empty() {
            self.conversation_skills.remove(&session);
        } else {
            self.conversation_skills.insert(session, selected);
        }
        self.persist_settings(cx);
        cx.notify();
    }
}

fn visible_catalog(security_enabled: bool) -> Vec<&'static Skill> {
    catalog_metadata()
        .into_iter()
        .filter(|skill| security_enabled || skill.id != SECURITY_AUDIT_SKILL)
        .collect()
}

/// Saved but uninstalled skills belong beside the catalog, not installed skills.
/// Prefer saved instructions over a bundled version with the same ID.
fn skill_sections(saved: &[Skill], security_enabled: bool) -> (Vec<Skill>, Vec<Skill>) {
    let mut skills = saved.to_vec();
    for entry in visible_catalog(security_enabled) {
        if !skills.iter().any(|skill| skill.id == entry.id) {
            skills.push(entry.clone());
        }
    }
    if !skills.iter().any(|skill| skill.id == COMMIT_SKILL) {
        skills.extend(defaults());
    }
    let (mut installed, mut available): (Vec<_>, Vec<_>) = skills
        .into_iter()
        .partition(|skill| !skill.installations.is_empty());
    installed.sort_by_key(|skill| skill.title.to_lowercase());
    available.sort_by_key(|skill| skill.title.to_lowercase());
    (installed, available)
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = dunlin_ui::settings::page()
        .child(dunlin_ui::settings::notice("Install once on this Mac and share skills across OpenCode, Claude Code and Codex. One global bundle; harness folders contain discovery links, not copies. Review instructions before installing. Only unchanged Dunlin-managed files are updated or removed.", p));
    if let Some(message) = &this.skills_ui.message {
        body = body.child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child(message.clone()),
        );
    }
    if let Some(draft) = &this.skills_ui.draft {
        return body
            .child(dunlin_ui::settings::section(
                "Edit skill · save to update the library",
                p,
            ))
            .child(dunlin_ui::settings::field(
                "Title",
                dunlin_ui::settings::input(&draft.title, p).aria_label("Skill title"),
                p,
            ))
            .child(dunlin_ui::settings::field(
                "Description",
                dunlin_ui::settings::input(&draft.description, p).aria_label("Skill description"),
                p,
            ))
            .child(
                div().h(px(240.)).overflow_hidden().child(
                    Editor::new(&draft.body)
                        .aria_label("Skill instructions")
                        .h_full(),
                ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(
                        action("save-skill".into(), "Save skill", p)
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.save_skill(cx))),
                    )
                    .child(
                        action("cancel-skill".into(), "Cancel", p).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.skills_ui.draft = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .into_any_element();
    }
    let mut imports = div().flex().flex_col().gap(px(12.)).child(
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(6.))
            .child(
                action("new-skill".into(), "Add skill", p)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_skill(None, window, cx))),
            )
            .child(
                action("scan-skills".into(), "Scan harness folders", p)
                    .on_click(cx.listener(|this, _, _, cx| this.scan_global_skills(cx))),
            )
            .child(
                action("import-skills".into(), "Import folder…", p).on_click(cx.listener(
                    |this, _, _, cx| {
                        let selected = cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: Some(
                                "Choose a folder containing SKILL.md or skill folders".into(),
                            ),
                        });
                        let selected = this.overlay_backdrop.track_native(selected, cx);
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(Some(paths))) = selected.await
                                && let Some(path) = paths.into_iter().next()
                            {
                                let _ =
                                    this.update(cx, |this, cx| this.import_skills(vec![path], cx));
                            }
                        })
                        .detach();
                    },
                )),
            ),
    );
    imports = imports.child(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_fg)
                    .child("GitHub skill (SKILL.md only)"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div().flex_1().min_w(px(0.)).child(
                            dunlin_ui::settings::input(&this.skills_import_url, p)
                                .aria_label("GitHub skill URL")
                                .disabled(this.skills_ui.busy),
                        ),
                    )
                    .child(
                        action("import-github-skill".into(), "Import from GitHub", p).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.import_skill_from_github(window, cx)
                            }),
                        ),
                    ),
            ),
    );
    imports = imports.child(
        div()
            .text_size(px(10.5))
            .text_color(p.muted_fg)
            .child("GitHub links fetch only SKILL.md. To import a multi-file bundle, download the folder and use Import folder…; the catalog bundles are embedded and install all companion files."),
    );
    let (installed, available) = skill_sections(&this.skills, this.security_enabled);
    for (title, description, skills) in [
        (
            "Installed skills",
            "Skills installed globally or in a project. Manage updates and uninstall them here.",
            installed,
        ),
        (
            "Available skills",
            "Browse bundled and imported instructions. Install a skill to make it available to your harnesses.",
            available,
        ),
    ] {
        body = body
            .child(dunlin_ui::settings::section(
                format!("{title} · {}", skills.len()),
                p,
            ))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted_fg)
                    .child(description),
            );
        if skills.is_empty() {
            body = body.child(dunlin_ui::settings::notice(
                if title == "Installed skills" {
                    "No skills installed yet. Choose Install globally from Available skills below."
                } else {
                    "All available skills are installed. Add or import more instructions below."
                },
                p,
            ));
        }
        for skill in &skills {
            let saved = this.skill_in_library(&skill.id);
            let id = skill.id.clone();
            let remove = id.clone();
            let bundle_size = format!("{:.1} KiB", skill.bundle_size() as f32 / 1024.0);
            let mut card = dunlin_ui::settings::card(p).p(px(14.)).flex().flex_col().gap(px(10.))
            .child(div().flex().items_center().gap(px(6.)).child(div().flex_1().font_weight(FontWeight::MEDIUM).child(skill.title.clone()))
                .when(saved, |header| header
                .child(action(format!("edit-skill:{id}"), "Edit", p).disabled(this.skills_ui.busy).on_click(cx.listener(move |this, _, window, cx| this.edit_skill(Some(&id), window, cx))))
                .child(action(format!("delete-skill:{remove}"), "Delete", p).danger().outline().disabled(this.skills_ui.busy).on_click(cx.listener(move |this, _, window, cx| {
                    let id = remove.clone();
                    let answer = window.prompt(PromptLevel::Warning, "Delete skill?", Some("Delete the library entry and its unchanged Dunlin-managed installations? Externally modified files are preserved."), &["Cancel", "Delete"], cx);
                    let answer = this.overlay_backdrop.track_native(answer, cx);
                    cx.spawn(async move |this, cx| { if answer.await == Ok(1) { let _ = this.update(cx, |this, cx| this.remove_skill_installations(&id, None, true, cx)); } }).detach();
                })))))
            .child(div().text_size(px(11.5)).text_color(p.muted_fg).child(skill.description.clone()))
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "{} file(s) · {} · supporting files are installed together and are not editable here",
                        skill.files.len() + 1,
                        bundle_size
                    )),
            );
            let id = skill.id.clone();
            let ready = shared_installation_current(skill);
            let installing = this.skills_ui.installing.as_deref() == Some(&skill.id);
            let has_global = skill.installations.iter().any(|i| i.scope == Scope::Global);
            let mut targets = div().flex().flex_wrap().justify_end().gap(px(8.)).child(
                action(
                    format!("install-skill:{id}"),
                    if installing {
                        "Installing…"
                    } else if ready {
                        "Installed globally"
                    } else if has_global {
                        "Update global installation"
                    } else {
                        "Install globally"
                    },
                    p,
                )
                .loading(installing)
                .disabled((this.skills_ui.busy && !installing) || ready)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if saved {
                        this.install_skill(&id, Target::Shared, None, cx);
                    } else {
                        this.install_catalog_skill(
                            &id,
                            Target::Shared,
                            Scope::Global,
                            "",
                            None,
                            cx,
                        );
                    }
                })),
            );
            if has_global {
                let id = skill.id.clone();
                targets = targets.child(
                    action(format!("uninstall-skill:{id}"), "Uninstall globally", p)
                        .disabled(this.skills_ui.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_skill_installations_scoped(
                                &id,
                                None,
                                Some(Scope::Global),
                                None,
                                false,
                                cx,
                            )
                        })),
                );
            }
            card = card.child(targets);
            let origin = if skill.origin.is_empty() {
                "Manual".to_string()
            } else {
                skill.origin.clone()
            };
            let hash = if skill.source_hash.is_empty() {
                skill.content_hash()
            } else {
                skill.source_hash.clone()
            };
            let hash = &hash[..hash.len().min(12)];
            card = card.child(
                div()
                    .text_size(px(10.))
                    .text_color(p.muted_fg)
                    .child(format!("Origin: {origin} · SHA-256 {hash}")),
            );
            if skill
                .installations
                .iter()
                .any(|i| i.scope == Scope::Project)
            {
                let id = skill.id.clone();
                card = card.child(div().flex().items_center().gap(px(8.))
                .child(div().flex_1().text_size(px(10.5)).text_color(p.muted_fg).child("Legacy project copies can override the global skill. Remove them to use the shared installation."))
                .child(action(format!("remove-project-skills:{id}"), "Remove project copies", p)
                    .disabled(this.skills_ui.busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_skill_installations_scoped(&id, None, Some(Scope::Project), None, false, cx)))));
            }
            body = body.child(card);
        }
    }
    body.child(dunlin_ui::settings::section("Add or import skills", p))
        .child(imports)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        InstalledFile, PLUGIN_BUILDER_SKILL, SECURITY_AUDIT_SKILL, SPEC_KIT_SKILL, Scope,
        SkillFile, Target, catalog_metadata, catalog_skill, defaults, discover, install_at,
        install_at_scoped, uninstall,
    };
    use std::{fs, path::PathBuf};

    #[test]
    fn conversation_skills_include_companions_without_installing_any_files() {
        let mut skill = catalog_skill(PLUGIN_BUILDER_SKILL).unwrap();
        skill.files.push(super::SkillFile {
            path: "references/example.md".into(),
            contents: "Companion instructions".into(),
        });
        let context = super::conversation_context(&[skill.clone()], &[skill.id.clone()]).unwrap();
        assert!(context.contains("not permission grants"));
        assert!(context.contains("SKILL.md"));
        assert!(context.contains(skill.body.trim()));
        assert!(context.contains("references/example.md"));
        assert!(context.contains("Companion instructions"));
        assert!(skill.installations.is_empty());
        assert!(super::conversation_context(&[], &["missing".into()]).is_err());
        assert!(super::conversation_context(&[], &[]).unwrap().is_empty());
        let mut large = skill.clone();
        large.files = (0..5)
            .map(|n| super::SkillFile {
                path: format!("reference-{n}.txt"),
                contents: "x".repeat(60_000),
            })
            .collect();
        assert!(
            super::conversation_context(&[large], &[skill.id])
                .unwrap_err()
                .contains("256 KiB")
        );
    }

    #[test]
    fn conversation_skill_settings_migrate_without_leaking_between_sessions() {
        let mut settings: crate::app::StoredSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.conversation_skills.is_empty());
        settings
            .conversation_skills
            .insert("session-a".into(), vec![PLUGIN_BUILDER_SKILL.into()]);
        let restored: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            restored.conversation_skills["session-a"],
            vec![PLUGIN_BUILDER_SKILL]
        );
        assert!(!restored.conversation_skills.contains_key("session-b"));
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("dunlin-skills-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn disabling_audits_does_not_hide_unrelated_catalog_skills() {
        let visible = super::visible_catalog(false);
        assert!(!visible.iter().any(|skill| skill.id == SECURITY_AUDIT_SKILL));
        assert!(visible.iter().any(|skill| skill.id == SPEC_KIT_SKILL));
        assert!(visible.iter().any(|skill| skill.id == PLUGIN_BUILDER_SKILL));
        assert_eq!(super::visible_catalog(true).len(), catalog_metadata().len());
    }

    #[test]
    fn sections_follow_installations_not_library_membership() {
        let temp = Temp::new();
        let mut saved = defaults();
        let mut edited = catalog_skill(SPEC_KIT_SKILL).unwrap();
        edited.body.push_str("\nMy reviewed instructions.\n");
        saved.push(edited);
        let (installed, available) = super::skill_sections(&saved, false);
        assert!(installed.is_empty());
        assert_eq!(
            available.iter().filter(|s| s.id == SPEC_KIT_SKILL).count(),
            1
        );
        assert!(
            available
                .iter()
                .find(|s| s.id == SPEC_KIT_SKILL)
                .unwrap()
                .body
                .contains("My reviewed instructions.")
        );
        assert!(!available.iter().any(|s| s.id == SECURITY_AUDIT_SKILL));

        let installation = install_at_scoped(
            &temp.0,
            &saved[1],
            Target::Claude,
            Scope::Project,
            "project",
        )
        .unwrap();
        saved[1].installations.push(installation);
        let (installed, available) = super::skill_sections(&saved, false);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id, SPEC_KIT_SKILL);
        assert!(!available.iter().any(|s| s.id == SPEC_KIT_SKILL));

        uninstall(&saved[1].installations[0], &saved[1].id).unwrap();
        saved[1].installations.clear();
        let (installed, available) = super::skill_sections(&saved, false);
        assert!(installed.is_empty());
        assert!(available.iter().any(|s| s.id == SPEC_KIT_SKILL));
    }

    #[test]
    fn removed_builtin_remains_available_for_one_step_installation() {
        let (installed, available) = super::skill_sections(&[], true);
        assert!(installed.is_empty());
        let builtin = available
            .iter()
            .find(|s| s.id == super::COMMIT_SKILL)
            .unwrap();
        assert_eq!(catalog_skill(&builtin.id).unwrap().body, builtin.body);
    }

    #[test]
    fn harness_names_map_to_the_correct_skill_target() {
        assert_eq!(Target::from_harness("OpenCode"), Target::OpenCode);
        assert_eq!(Target::from_harness("Claude Code"), Target::Claude);
        assert_eq!(Target::from_harness("Codex"), Target::Codex);
        assert_eq!(Target::from_harness("OpenAI Codex"), Target::Codex);
        assert_eq!(Target::from_harness("Cursor"), Target::Other);
    }

    #[test]
    fn shared_bundle_is_installed_once_and_updates_through_every_discovery_link() {
        use std::os::unix::fs::MetadataExt as _;
        let temp = Temp::new();
        let base = temp.0.join("shared");
        let aliases = vec![
            (Target::OpenCode, temp.0.join("opencode")),
            (Target::Claude, temp.0.join("claude")),
        ];
        let mut skill = catalog_skill(SECURITY_AUDIT_SKILL).unwrap();
        let (installed, errors) = super::install_shared_at(&base, &aliases, &skill);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(installed.len(), 3);
        let canonical = PathBuf::from(&installed[0].path);
        for link in &installed[1..] {
            assert!(
                fs::symlink_metadata(PathBuf::from(&link.path).parent().unwrap())
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                fs::metadata(&link.path).unwrap().ino(),
                fs::metadata(&canonical).unwrap().ino()
            );
            assert!(
                PathBuf::from(&link.path)
                    .parent()
                    .unwrap()
                    .join("report-schema.json")
                    .is_file()
            );
        }
        skill.installations = installed;
        assert!(super::shared_installation_current(&skill));
        skill.body.push_str("\nReviewed update.\n");
        assert!(!super::shared_installation_current(&skill));
        let (installed, errors) = super::install_shared_at(&base, &aliases, &skill);
        assert!(errors.is_empty(), "{errors:?}");
        skill.installations = installed;
        assert!(super::shared_installation_current(&skill));
        for link in &skill.installations[1..] {
            assert!(
                fs::read_to_string(&link.path)
                    .unwrap()
                    .contains("Reviewed update.")
            );
            uninstall(link, &skill.id).unwrap();
        }
        uninstall(&skill.installations[0], &skill.id).unwrap();
        assert!(!canonical.exists());
    }

    #[test]
    fn shared_install_migrates_only_unchanged_owned_legacy_copies() {
        let temp = Temp::new();
        let alias = temp.0.join("claude");
        let mut skill = defaults().remove(0);
        skill
            .installations
            .push(install_at(&alias, &skill, Target::Claude).unwrap());
        let (installed, errors) =
            super::install_shared_at(&temp.0.join("shared"), &[(Target::Claude, alias)], &skill);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(installed.len(), 2);
        assert!(installed[1].linked_to.is_some());
    }

    #[test]
    fn shared_install_preserves_unmanaged_and_externally_edited_legacy_skills() {
        for managed in [false, true] {
            let temp = Temp::new();
            let alias = temp.0.join("claude");
            let mut skill = defaults().remove(0);
            let legacy = install_at(&alias, &skill, Target::Claude).unwrap();
            fs::write(&legacy.path, "user instructions").unwrap();
            if managed {
                skill.installations.push(legacy.clone());
            }
            let (installed, errors) = super::install_shared_at(
                &temp.0.join("shared"),
                &[(Target::Claude, alias)],
                &skill,
            );
            // A partial success remains recordable, so a retry never treats the
            // canonical bundle as an unmanaged file.
            assert_eq!(installed.len(), 1);
            assert_eq!(errors.len(), 1);
            assert_eq!(
                fs::read_to_string(&legacy.path).unwrap(),
                "user instructions"
            );
        }
    }

    #[test]
    fn replaced_discovery_link_and_its_destination_are_never_removed() {
        let temp = Temp::new();
        let alias = temp.0.join("claude");
        let skill = defaults().remove(0);
        let (installed, errors) =
            super::install_shared_at(&temp.0.join("shared"), &[(Target::Claude, alias)], &skill);
        assert!(errors.is_empty());
        let link = PathBuf::from(&installed[1].path)
            .parent()
            .unwrap()
            .to_path_buf();
        fs::remove_file(&link).unwrap();
        let external = temp.0.join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("SKILL.md"), "keep").unwrap();
        std::os::unix::fs::symlink(&external, &link).unwrap();
        assert!(uninstall(&installed[1], &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(external.join("SKILL.md")).unwrap(),
            "keep"
        );
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
    #[test]
    fn portable_frontmatter_is_bounded_and_description_cannot_inject_fields() {
        let mut skill = defaults().remove(0);
        skill.description = "Commit rules\n---\nmodel: unwanted".into();
        let text = skill.rendered().unwrap();
        assert_eq!(text.lines().filter(|line| *line == "---").count(), 2);
        assert!(text.contains("disable-model-invocation: true"));
        assert!(text.contains(r#"\n---\nmodel: unwanted"#));
        skill.id = "../escape".into();
        assert!(skill.rendered().is_err());
        skill.id = "safe".into();
        skill.body = "x".repeat(64 * 1024);
        assert!(skill.rendered().is_err());
    }
    #[test]
    fn install_updates_only_owned_unchanged_files_and_refuses_user_edits() {
        let temp = Temp::new();
        let mut skill = defaults().remove(0);
        let installed = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        let original = fs::read_to_string(&installed.path).unwrap();
        assert!(install_at(&temp.0, &skill, Target::OpenCode).is_err()); // Not yet recorded as ours.
        assert_eq!(fs::read_to_string(&installed.path).unwrap(), original);
        skill.installations.push(installed);
        skill.body.push_str("\nAdditional reviewed instructions.\n");
        let updated = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        assert_ne!(updated.contents, original);
        assert_eq!(fs::read_to_string(&updated.path).unwrap(), updated.contents);
        skill.installations = vec![updated.clone()];
        fs::write(&updated.path, "externally modified").unwrap();
        assert!(install_at(&temp.0, &skill, Target::OpenCode).is_err());
        assert!(uninstall(&updated, &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(&updated.path).unwrap(),
            "externally modified"
        );
    }
    #[test]
    fn uninstall_never_recursively_removes_supporting_or_user_files() {
        let temp = Temp::new();
        let skill = defaults().remove(0);
        let installed = install_at(&temp.0, &skill, Target::Codex).unwrap();
        let extra = std::path::Path::new(&installed.path)
            .parent()
            .unwrap()
            .join("keep.txt");
        fs::write(&extra, "keep").unwrap();
        uninstall(&installed, &skill.id).unwrap();
        assert_eq!(fs::read_to_string(extra).unwrap(), "keep");
        assert!(!std::path::Path::new(&installed.path).exists());
        assert!(uninstall(&installed, "different-id").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn skill_directory_and_file_symlinks_cannot_overwrite_external_content() {
        use std::os::unix::fs::symlink;
        let temp = Temp::new();
        let outside = Temp::new();
        let skill = defaults().remove(0);
        fs::write(outside.0.join("SKILL.md"), "external").unwrap();
        let directory = temp.0.join(format!("dunlin-{}", skill.id));
        symlink(&outside.0, &directory).unwrap();
        assert!(install_at(&temp.0, &skill, Target::Claude).is_err());
        fs::remove_file(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        symlink(outside.0.join("SKILL.md"), directory.join("SKILL.md")).unwrap();
        assert!(install_at(&temp.0, &skill, Target::Claude).is_err());
        assert_eq!(
            fs::read_to_string(outside.0.join("SKILL.md")).unwrap(),
            "external"
        );
    }
    #[cfg(unix)]
    #[test]
    fn uninstall_refuses_an_ancestor_replaced_with_a_symlink() {
        let temp = Temp::new();
        let outside = Temp::new();
        let base = temp.0.join("skills");
        let skill = defaults().remove(0);
        let installed = install_at(&base, &skill, Target::OpenCode).unwrap();
        fs::rename(&base, temp.0.join("moved-skills")).unwrap();
        let external = outside.0.join(format!("dunlin-{}", skill.id));
        fs::create_dir(&external).unwrap();
        fs::write(external.join("SKILL.md"), &installed.contents).unwrap();
        std::os::unix::fs::symlink(&outside.0, &base).unwrap();
        assert!(uninstall(&installed, &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(external.join("SKILL.md")).unwrap(),
            installed.contents
        );
    }
    #[test]
    fn ordinary_skills_remain_discoverable_without_commit_only_invocation_restriction() {
        let mut skill = defaults().remove(0);
        skill.id = "review".into();
        assert!(
            !skill
                .rendered()
                .unwrap()
                .contains("disable-model-invocation")
        );
    }
    #[test]
    fn commit_skill_preserves_index_scope_and_repository_policy_without_bypasses() {
        let skill = defaults().remove(0);
        for rule in [
            "Conventional Commits",
            "AGENTS.md",
            "recent commit",
            "partial staging",
            "nonempty",
            "--no-verify",
            "never use blind",
            "Never infer authorization",
            "Repository policy wins",
        ] {
            assert!(skill.body.contains(rule), "missing {rule}");
        }
        let serialized = serde_json::to_string(&vec![skill]).unwrap();
        let restored: Vec<super::Skill> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored[0].id, super::COMMIT_SKILL);
    }
    #[test]
    fn discovery_reads_frontmatter_and_rejects_unbounded_or_symlinked_skills() {
        let temp = Temp::new();
        let directory = temp.0.join("dunlin-docs");
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: dunlin-docs\ndescription: \"Use project docs\"\n---\n\nRead the docs.\n",
        )
        .unwrap();
        let found = discover(&temp.0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "docs");
        assert_eq!(found[0].description, "Use project docs");
        assert_eq!(found[0].body, "Read the docs.");
        assert_eq!(found[0].source_hash.len(), 64);
        #[cfg(unix)]
        {
            let link = temp.0.join("linked");
            std::os::unix::fs::symlink(&directory, &link).unwrap();
            assert!(discover(&link).is_err());
        }
        fs::write(directory.join("SKILL.md"), "x".repeat(64 * 1024 + 1)).unwrap();
        assert!(discover(&temp.0).is_err());
    }
    #[test]
    fn project_installation_records_scope_and_can_be_removed_without_global_scope() {
        let temp = Temp::new();
        let skill = defaults().remove(0);
        let base = Target::Claude.project_directory(&temp.0).unwrap();
        let installation =
            install_at_scoped(&base, &skill, Target::Claude, Scope::Project, "project-1").unwrap();
        assert_eq!(installation.scope, Scope::Project);
        assert_eq!(installation.project_id, "project-1");
        assert!(installation.path.contains(".claude"));
        uninstall(&installation, &skill.id).unwrap();
        assert!(!PathBuf::from(&installation.path).exists());
    }
    #[test]
    fn github_skill_links_map_to_raw_skill_md() {
        let repo = super::github_skill_urls("https://github.com/owner/repo").unwrap();
        assert_eq!(
            repo[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/SKILL.md"
        );
        assert_eq!(
            repo[1].as_str(),
            "https://raw.githubusercontent.com/owner/repo/master/SKILL.md"
        );
        let tree = super::github_skill_urls("https://github.com/owner/repo/tree/main/skills/demo")
            .unwrap();
        assert_eq!(
            tree[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/skills/demo/SKILL.md"
        );
        let blob = super::github_skill_urls(
            "https://github.com/owner/repo/blob/main/skills/demo/SKILL.md",
        )
        .unwrap();
        assert_eq!(
            blob[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/skills/demo/SKILL.md"
        );
        assert!(super::github_skill_urls("http://github.com/owner/repo").is_err());
        assert!(super::github_skill_urls("https://example.com/owner/repo").is_err());
        assert!(
            super::github_skill_urls("https://github.com/owner/repo/blob/main/README.md").is_err()
        );
    }
    #[test]
    fn multi_file_bundles_install_update_and_preserve_ownership_checks() {
        let temp = Temp::new();
        let mut skill = defaults().remove(0);
        skill.files = vec![
            SkillFile {
                path: "references/guide.md".into(),
                contents: "original guide".into(),
            },
            SkillFile {
                path: "scripts/check.sh".into(),
                contents: "#!/bin/sh\necho ok\n".into(),
            },
        ];
        let installed = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        assert_eq!(installed.files.len(), 3);
        let root = PathBuf::from(&installed.path)
            .parent()
            .unwrap()
            .to_path_buf();
        assert_eq!(
            fs::read_to_string(root.join("references/guide.md")).unwrap(),
            "original guide"
        );
        assert_eq!(
            fs::read_to_string(root.join("scripts/check.sh")).unwrap(),
            "#!/bin/sh\necho ok\n"
        );
        skill.installations.push(installed.clone());
        skill.files[0].contents = "updated guide".into();
        let updated = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("references/guide.md")).unwrap(),
            "updated guide"
        );
        // An interrupted update that already wrote the new bytes is tolerated.
        install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        skill.installations = vec![updated];
        fs::write(root.join("scripts/check.sh"), "externally changed").unwrap();
        assert!(install_at(&temp.0, &skill, Target::OpenCode).is_err());
        assert!(uninstall(&skill.installations[0], &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(root.join("scripts/check.sh")).unwrap(),
            "externally changed"
        );
    }
    #[test]
    fn recorded_supporting_paths_cannot_escape_the_skill_directory() {
        let temp = Temp::new();
        let skill = defaults().remove(0);
        let mut installation = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        installation.files.push(InstalledFile {
            path: "../escape".into(),
            sha256: "0".repeat(64),
        });
        let mut tampered = skill.clone();
        tampered.installations.push(installation.clone());
        assert!(install_at(&temp.0, &tampered, Target::OpenCode).is_err());
        assert!(uninstall(&installation, &skill.id).is_err());
    }
    #[test]
    fn bundle_file_paths_are_bounded_and_cannot_escape_the_skill_directory() {
        let mut skill = defaults().remove(0);
        for path in ["../escape.md", "/absolute.md", "a/../../b.md", "a\\b.md"] {
            skill.files = vec![SkillFile {
                path: path.into(),
                contents: "x".into(),
            }];
            assert!(skill.rendered_files().is_err(), "{path} must be rejected");
        }
        skill.files = vec![SkillFile {
            path: "a.md".into(),
            contents: "x".repeat(256 * 1024 + 1),
        }];
        assert!(skill.rendered_files().is_err());
        skill.files = vec![SkillFile {
            path: "a.md".into(),
            contents: "x".into(),
        }];
        assert!(skill.rendered_files().is_ok());
        assert_eq!(skill.rendered_files().unwrap().len(), 2);
    }
    #[test]
    fn bundled_cloudflare_security_audit_installs_every_companion() {
        let skill = catalog_skill(SECURITY_AUDIT_SKILL).unwrap();
        assert_eq!(skill.id, SECURITY_AUDIT_SKILL);
        assert!(skill.files.len() >= 20);
        let temp = Temp::new();
        let installed = install_at(&temp.0, &skill, Target::Codex).unwrap();
        let installed_path = PathBuf::from(&installed.path);
        let root = installed_path.parent().unwrap();
        assert!(root.join("SKILL.md").is_file());
        assert!(root.join("HUNTING.md").is_file());
        assert!(root.join("validate-findings.cjs").is_file());
        assert!(root.join("LICENSE").is_file());
        assert!(installed.files.len() == skill.files.len() + 1);
    }
    #[test]
    fn bundled_spec_kit_skill_is_reviewable_and_installable() {
        let skill = catalog_skill(SPEC_KIT_SKILL).unwrap();
        assert_eq!(skill.id, SPEC_KIT_SKILL);
        assert!(skill.body.to_lowercase().contains("reviewer-owned"));
        assert!(skill.body.contains("converge"));
        let temp = Temp::new();
        let installed = install_at(&temp.0, &skill, Target::Claude).unwrap();
        assert!(PathBuf::from(&installed.path).is_file());
    }

    #[test]
    fn coordination_skill_is_optional_installable_and_never_grants_execution() {
        let skill = catalog_skill(super::COORDINATION_SKILL).unwrap();
        assert!(skill.body.contains("continuations_create"));
        assert!(skill.body.contains("END YOUR CURRENT TURN"));
        assert!(skill.body.contains("not an execution approval"));
        assert!(skill.body.contains("UNTRUSTED DATA"));
        let temp = Temp::new();
        let installed = install_at(&temp.0, &skill, Target::Codex).unwrap();
        assert!(PathBuf::from(installed.path).is_file());
    }

    #[test]
    fn plugin_builder_skill_is_bundled_and_complete() {
        let skill = catalog_skill(PLUGIN_BUILDER_SKILL).unwrap();
        assert_eq!(skill.id, PLUGIN_BUILDER_SKILL);
        assert!(skill.body.contains("dunlin_alloc"));
        assert!(skill.body.contains("plugin.json"));
        assert!(skill.body.contains("Security checklist"));
        assert!(skill.body.contains("object_propose"));
        assert!(
            catalog_metadata()
                .iter()
                .any(|entry| entry.id == PLUGIN_BUILDER_SKILL)
        );
    }

    #[test]
    fn catalog_update_adds_companions_to_an_older_single_file_install() {
        let temp = Temp::new();
        let mut catalog = catalog_skill(SECURITY_AUDIT_SKILL).unwrap();
        let mut single_file = catalog.clone();
        single_file.files.clear();
        let installed = install_at(&temp.0, &single_file, Target::OpenCode).unwrap();
        catalog.installations.push(installed);
        let updated = install_at(&temp.0, &catalog, Target::OpenCode).unwrap();
        assert_eq!(updated.files.len(), catalog.files.len() + 1);
        let root = PathBuf::from(&updated.path).parent().unwrap().to_path_buf();
        assert!(root.join("HUNTING.md").is_file());
    }
    #[test]
    fn discovery_reads_supporting_files_but_never_symlinks() {
        let temp = Temp::new();
        let skill_dir = temp.0.join("security-audit");
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: security-audit\ndescription: Audit\n---\n\nBody\n",
        )
        .unwrap();
        fs::write(skill_dir.join("references/guide.md"), "guide").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            skill_dir.join("references/guide.md"),
            skill_dir.join("references/link.md"),
        )
        .unwrap();
        let found = discover(&skill_dir).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].files.len(), 1);
        assert_eq!(found[0].files[0].path, "references/guide.md");
    }
    #[test]
    fn removing_a_supporting_file_updates_the_owned_bundle_without_touching_user_files() {
        let temp = Temp::new();
        let mut skill = defaults().remove(0);
        skill.files = vec![SkillFile {
            path: "references/old.md".into(),
            contents: "old".into(),
        }];
        let installed = install_at(&temp.0, &skill, Target::Claude).unwrap();
        let root = PathBuf::from(&installed.path)
            .parent()
            .unwrap()
            .to_path_buf();
        fs::write(root.join("keep.txt"), "user").unwrap();
        skill.installations.push(installed);
        skill.files.clear();
        install_at(&temp.0, &skill, Target::Claude).unwrap();
        assert!(!root.join("references/old.md").exists());
        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "user");
        uninstall(&skill.installations[0], &skill.id).unwrap();
        assert!(!root.join("SKILL.md").exists());
        assert!(root.join("keep.txt").exists());
    }
}
