//! Validated, literal project runtime defaults. No shell or dotenv evaluation.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{Id, PermissionMode, Project, ProjectGroup, ProjectSettings};

const MAX_ENV_ENTRIES: usize = 128;
const MAX_ENV_BYTES: usize = 64 * 1024;

/// Inheritable overrides only. Project icons, trust and memberships are never
/// inherited, and these preferences do not configure a relay server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSettings {
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub default_models: BTreeMap<String, String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl GroupSettings {
    pub fn default_model(&self, harness: &str) -> Option<&str> {
        self.default_models.get(harness).map(String::as_str)
    }

    pub fn set_default_model(&mut self, harness: &str, model: &str) -> Result<(), String> {
        set_model(&mut self.default_models, harness, model)
    }
}

impl From<&ProjectSettings> for GroupSettings {
    fn from(settings: &ProjectSettings) -> Self {
        Self {
            harness: settings.harness.clone(),
            permission_mode: settings.permission_mode,
            default_models: settings.default_models.clone(),
            environment: settings.environment.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingSource {
    Global,
    Group(Id),
    Project(Id),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingValue<T> {
    pub value: T,
    pub source: SettingSource,
}

#[derive(Clone, Debug)]
pub struct ResolvedSettings {
    pub harness: SettingValue<String>,
    pub permission_mode: SettingValue<PermissionMode>,
    pub default_models: BTreeMap<String, SettingValue<String>>,
    pub environment: BTreeMap<String, SettingValue<String>>,
}

impl ResolvedSettings {
    pub fn default_model(&self, harness: &str) -> Option<&str> {
        self.default_models
            .get(harness)
            .map(|setting| setting.value.as_str())
    }

    pub fn process_environment(&self) -> Result<BTreeMap<String, String>, String> {
        let environment = self
            .environment
            .iter()
            .map(|(name, setting)| (name.clone(), setting.value.clone()))
            .collect();
        validate_project_environment(&environment)?;
        Ok(environment)
    }

    fn apply(&mut self, settings: &GroupSettings, source: SettingSource) {
        if let Some(harness) = &settings.harness {
            self.harness = SettingValue {
                value: harness.clone(),
                source: source.clone(),
            };
        }
        if let Some(mode) = settings.permission_mode {
            self.permission_mode = SettingValue {
                value: mode,
                source: source.clone(),
            };
        }
        for (name, value) in &settings.default_models {
            self.default_models.insert(
                name.clone(),
                SettingValue {
                    value: value.clone(),
                    source: source.clone(),
                },
            );
        }
        for (name, value) in &settings.environment {
            self.environment.insert(
                name.clone(),
                SettingValue {
                    value: value.clone(),
                    source: source.clone(),
                },
            );
        }
    }
}

/// Resolve one complete ancestry, root first. Missing/cyclic links are errors,
/// not partial inheritance that could silently grant broader tool permissions.
/// Iterative and linear in group count, including for imported deep trees.
pub fn resolve_group_settings(
    group_id: Option<&str>,
    groups: &[ProjectGroup],
    default_harness: &str,
    default_permission: PermissionMode,
) -> Result<ResolvedSettings, String> {
    let by_id: HashMap<_, _> = groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect();
    let mut seen = HashSet::new();
    let mut chain = Vec::new();
    let mut next = group_id;
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err("Settings inheritance contains a group cycle; repair group membership before connecting".into());
        }
        let group = by_id.get(id).ok_or("Settings inheritance refers to a missing group; repair group membership before connecting")?;
        chain.push(*group);
        next = group.parent_id.as_deref();
    }
    let mut resolved = ResolvedSettings {
        harness: SettingValue {
            value: default_harness.into(),
            source: SettingSource::Global,
        },
        permission_mode: SettingValue {
            value: default_permission,
            source: SettingSource::Global,
        },
        default_models: BTreeMap::new(),
        environment: BTreeMap::new(),
    };
    for group in chain.into_iter().rev() {
        resolved.apply(&group.settings, SettingSource::Group(group.id.clone()));
    }
    Ok(resolved)
}

pub fn resolve_project_settings(
    project: &Project,
    groups: &[ProjectGroup],
    default_harness: &str,
    default_permission: PermissionMode,
) -> Result<ResolvedSettings, String> {
    let mut resolved = resolve_group_settings(
        project.group_id.as_deref(),
        groups,
        default_harness,
        default_permission,
    )?;
    resolved.apply(
        &GroupSettings::from(&project.settings),
        SettingSource::Project(project.id.clone()),
    );
    Ok(resolved)
}

fn set_model(
    models: &mut BTreeMap<String, String>,
    harness: &str,
    model: &str,
) -> Result<(), String> {
    let model = model.trim();
    if harness.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
        return Err("Use a model ID of at most 512 bytes, without control characters".into());
    }
    if model.is_empty() {
        models.remove(harness);
    } else {
        models.insert(harness.into(), model.into());
    }
    Ok(())
}

impl ProjectSettings {
    pub fn default_model(&self, harness: &str) -> Option<&str> {
        self.default_models.get(harness).map(String::as_str)
    }

    pub fn set_default_model(&mut self, harness: &str, model: &str) -> Result<(), String> {
        set_model(&mut self.default_models, harness, model)
    }
}

pub fn validate_project_environment(environment: &BTreeMap<String, String>) -> Result<(), String> {
    if environment.len() > MAX_ENV_ENTRIES
        || environment
            .iter()
            .map(|(k, v)| k.len() + v.len() + 2)
            .sum::<usize>()
            > MAX_ENV_BYTES
    {
        return Err("Project environment is limited to 128 entries and 64 KiB".into());
    }
    for (key, value) in environment {
        let mut bytes = key.bytes();
        if !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("Environment names must match [A-Za-z_][A-Za-z0-9_]*".into());
        }
        if value.contains(['\0', '\r', '\n']) {
            return Err("Environment values must be single-line and contain no NUL bytes".into());
        }
    }
    Ok(())
}

/// One NAME=value per line. Values are literal, including spaces, #, quotes,
/// equals and $. Duplicate names are errors, never silently last-wins.
/// Errors deliberately omit input names/values which could contain secrets.
pub fn parse_project_environment(text: &str) -> Result<BTreeMap<String, String>, String> {
    if text.len() > MAX_ENV_BYTES {
        return Err("Project environment is limited to 64 KiB".into());
    }
    let mut environment = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("Line {}: expected NAME=value (no export prefix)", index + 1))?;
        if environment.insert(name.into(), value.into()).is_some() {
            return Err(format!("Line {}: duplicate environment name", index + 1));
        }
    }
    validate_project_environment(&environment)?;
    Ok(environment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: &str, parent: Option<&str>) -> ProjectGroup {
        ProjectGroup {
            id: id.into(),
            name: id.into(),
            tone: crate::ProjectTone::Blue,
            parent_id: parent.map(str::to_owned),
            settings: GroupSettings::default(),
        }
    }

    fn project(group: Option<&str>) -> Project {
        Project {
            id: "project".into(),
            name: "Project".into(),
            path: "/unused".into(),
            branch: None,
            tone: crate::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: group.map(str::to_owned),
            settings: ProjectSettings::default(),
        }
    }

    #[test]
    fn hierarchy_merges_per_key_and_reports_each_values_origin() {
        let mut root = group("root", None);
        root.settings.harness = Some("Claude Code".into());
        root.settings.permission_mode = Some(PermissionMode::ReadOnly);
        root.settings.environment = parse_project_environment("ROOT=one\nSHARED=root").unwrap();
        root.settings
            .set_default_model("Claude Code", "root-model")
            .unwrap();
        let mut child = group("child", Some("root"));
        child.settings.environment = parse_project_environment("CHILD=two\nSHARED=child").unwrap();
        child
            .settings
            .set_default_model("OpenCode", "other-harness")
            .unwrap();
        let mut project = project(Some("child"));
        project.settings.environment = parse_project_environment("SHARED=project\nEMPTY=").unwrap();
        project.settings.permission_mode = Some(PermissionMode::Ask);
        project.settings.custom_icon = Some("not inherited".into());
        let resolved = resolve_project_settings(
            &project,
            &[child, root],
            "OpenCode",
            PermissionMode::AutoApprove,
        )
        .unwrap();
        assert_eq!(
            resolved.harness,
            SettingValue {
                value: "Claude Code".into(),
                source: SettingSource::Group("root".into())
            }
        );
        assert_eq!(
            resolved.permission_mode,
            SettingValue {
                value: PermissionMode::Ask,
                source: SettingSource::Project("project".into())
            }
        );
        assert_eq!(resolved.default_model("Claude Code"), Some("root-model"));
        assert_eq!(resolved.default_model("OpenCode"), Some("other-harness"));
        assert_eq!(
            resolved.environment["ROOT"].source,
            SettingSource::Group("root".into())
        );
        assert_eq!(
            resolved.environment["CHILD"].source,
            SettingSource::Group("child".into())
        );
        assert_eq!(
            resolved.environment["SHARED"].source,
            SettingSource::Project("project".into())
        );
        assert_eq!(resolved.process_environment().unwrap()["SHARED"], "project");
        assert_eq!(resolved.process_environment().unwrap()["EMPTY"], "");
    }

    #[test]
    fn clearing_overrides_and_reparenting_recompute_without_copying_inherited_values() {
        let mut a = group("a", None);
        a.settings.harness = Some("Claude Code".into());
        a.settings
            .set_default_model("Claude Code", "a-model")
            .unwrap();
        let mut b = group("b", None);
        b.settings.permission_mode = Some(PermissionMode::ReadOnly);
        let mut child = group("child", Some("a"));
        let mut project = project(Some("child"));
        project.settings.harness = Some("Custom ACP".into());
        assert_eq!(
            resolve_project_settings(
                &project,
                &[a.clone(), b.clone(), child.clone()],
                "OpenCode",
                PermissionMode::Ask
            )
            .unwrap()
            .harness
            .value,
            "Custom ACP"
        );
        project.settings.harness = None;
        assert_eq!(
            resolve_project_settings(
                &project,
                &[a.clone(), b.clone(), child.clone()],
                "OpenCode",
                PermissionMode::Ask
            )
            .unwrap()
            .harness
            .value,
            "Claude Code"
        );
        child.parent_id = Some("b".into());
        let resolved =
            resolve_project_settings(&project, &[a, b, child], "OpenCode", PermissionMode::Ask)
                .unwrap();
        assert_eq!(resolved.harness.source, SettingSource::Global);
        assert_eq!(resolved.permission_mode.value, PermissionMode::ReadOnly);
        assert!(resolved.default_models.is_empty());
        assert!(project.settings.default_models.is_empty());
    }

    #[test]
    fn corrupt_ancestry_is_never_partially_applied() {
        let a = group("a", Some("b"));
        let b = group("b", Some("a"));
        assert!(
            resolve_project_settings(
                &project(Some("a")),
                &[a.clone(), b],
                "OpenCode",
                PermissionMode::AutoApprove
            )
            .is_err()
        );
        assert!(
            resolve_group_settings(Some("a"), &[a], "OpenCode", PermissionMode::AutoApprove)
                .is_err()
        );
        assert!(
            resolve_group_settings(
                Some("missing"),
                &[],
                "OpenCode",
                PermissionMode::AutoApprove
            )
            .is_err()
        );
    }

    #[test]
    fn merged_environment_is_bounded_even_if_each_layer_is_valid() {
        let mut root = group("root", None);
        root.settings.environment =
            parse_project_environment(&(0..128).map(|i| format!("K{i}=v\n")).collect::<String>())
                .unwrap();
        let mut project = project(Some("root"));
        project.settings.environment = parse_project_environment("EXTRA=v").unwrap();
        let resolved =
            resolve_project_settings(&project, &[root], "OpenCode", PermissionMode::Ask).unwrap();
        assert!(resolved.process_environment().is_err());
    }

    #[test]
    fn deep_hierarchy_is_resolved_iteratively() {
        let mut groups: Vec<_> = (0..5000)
            .map(|i| {
                group(
                    &i.to_string(),
                    (i > 0).then(|| (i - 1).to_string()).as_deref(),
                )
            })
            .collect();
        groups[0].settings.permission_mode = Some(PermissionMode::ReadOnly);
        assert_eq!(
            resolve_project_settings(
                &project(Some("4999")),
                &groups,
                "OpenCode",
                PermissionMode::AutoApprove
            )
            .unwrap()
            .permission_mode
            .value,
            PermissionMode::ReadOnly
        );
    }

    #[test]
    fn models_are_scoped_to_harness_and_blank_restores_default() {
        let mut settings = ProjectSettings::default();
        settings
            .set_default_model("OpenCode", " provider/model ")
            .unwrap();
        settings.set_default_model("Claude Code", "other").unwrap();
        assert_eq!(settings.default_model("OpenCode"), Some("provider/model"));
        assert_eq!(settings.default_model("Custom ACP"), None);
        settings.set_default_model("OpenCode", " ").unwrap();
        assert_eq!(settings.default_model("OpenCode"), None);
        assert_eq!(settings.default_model("Claude Code"), Some("other"));
        assert!(
            settings
                .set_default_model("Claude Code", "bad\0id")
                .is_err()
        );
        assert_eq!(settings.default_model("Claude Code"), Some("other"));
    }

    #[test]
    fn environment_is_literal_and_preserves_values() {
        let env =
            parse_project_environment("\nEMPTY=\r\n_PATH=/bin\r\nVALUE= $HOME='#a=b' \n").unwrap();
        assert_eq!(env["EMPTY"], "");
        assert_eq!(env["_PATH"], "/bin");
        assert_eq!(env["VALUE"], " $HOME='#a=b' ");
        assert!(parse_project_environment("").unwrap().is_empty());
    }

    #[test]
    fn invalid_environment_never_echoes_input() {
        for text in [
            "secret",
            "export KEY=secret",
            "KEY=1\nKEY=secret",
            "9KEY=secret",
            "KEY=secret\0",
            "KEY=secret\rvalue",
            " KEY=secret",
        ] {
            let error = parse_project_environment(text).unwrap_err();
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn environment_has_entry_and_byte_limits() {
        let text = (0..129).map(|i| format!("K{i}=v\n")).collect::<String>();
        assert!(parse_project_environment(&text).is_err());
        assert!(parse_project_environment(&format!("KEY={}", "x".repeat(MAX_ENV_BYTES))).is_err());
        let map = BTreeMap::from([("KEY".into(), "a\nb".into())]);
        assert!(validate_project_environment(&map).is_err());
    }
}
