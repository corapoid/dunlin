//! Portable allowlists, never a dump of local app/project state.
use crate::app::StoredSettings;
use dunlin_domain::{PendingClone, Project, ProjectGroup, ProjectSettings, ProjectTone};
use dunlin_workspace::{catalog_backup::MAX_BYTES, git::hosting::RepositoryLink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Read as _,
    path::Path,
};

macro_rules! preferences {
    ($action:ident) => {
        $action!(
            default_harness,
            hidden_harnesses,
            restart_harness,
            restore_session,
            word_wrap,
            editor_minimap,
            inline_blame,
            autosave,
            code_font_size,
            ui_font_family,
            code_font_family,
            reasoning_open,
            final_answer_only,
            stream_responses,
            chat_timestamps,
            chat_duration,
            chat_logo,
            appearance,
            skin,
            notifications_enabled,
            show_shortcut_hints,
            show_file_sizes,
            project_sort,
            sidebar_width,
            show_project_branches,
            project_list,
            account_usage_collapsed,
            dashboard_settings,
            billing_budget_usd,
            billing_alerts,
            density,
            reduce_motion,
            bottom_panel_height,
            bottom_panel_width,
            terminal_dock,
            shell_integration
        );
    };
}
pub(super) use preferences;
macro_rules! keys { ($($field:ident),*) => { const KEYS: &[&str] = &[$(stringify!($field)),*]; }; }
preferences!(keys);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Backup {
    pub format: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<Catalog>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub groups: Vec<Group>,
    pub projects: Vec<Repository>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Group {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub tone: ProjectTone,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Repository {
    pub name: String,
    pub url: String,
    pub group_id: Option<String>,
    pub tone: ProjectTone,
    pub pinned: bool,
}

pub(super) fn portable(settings: &StoredSettings) -> Result<Value, String> {
    let mut value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    value
        .as_object_mut()
        .unwrap()
        .retain(|key, _| KEYS.contains(&key.as_str()));
    value["dashboard_settings"]
        .as_object_mut()
        .unwrap()
        .remove("recent_projects");
    Ok(value)
}
pub(super) fn merge_settings(
    current: &StoredSettings,
    imported: &Value,
) -> Result<StoredSettings, String> {
    let object = imported.as_object().ok_or("Invalid portable settings")?;
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err("Backup contains unsupported or machine-specific settings".into());
    }
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    value.as_object_mut().unwrap().extend(object.clone());
    if let Some(dashboard) = object.get("dashboard_settings") {
        let mut current_dashboard =
            serde_json::to_value(&current.dashboard_settings).map_err(|e| e.to_string())?;
        let entries = dashboard
            .as_object()
            .ok_or("Invalid dashboard preferences")?;
        if entries.keys().any(|key| {
            !matches!(
                key.as_str(),
                "start_at_dashboard" | "show_agents" | "show_activity" | "metric"
            )
        }) {
            return Err("Dashboard history is not portable".into());
        }
        current_dashboard
            .as_object_mut()
            .unwrap()
            .extend(entries.clone());
        value["dashboard_settings"] = current_dashboard;
    }
    let settings: StoredSettings =
        serde_json::from_value(value).map_err(|_| "Invalid portable settings")?;
    for text in [
        &settings.default_harness,
        &settings.ui_font_family,
        &settings.code_font_family,
    ] {
        label(text)?;
    }
    for name in &settings.hidden_harnesses {
        label(name)?;
    }
    for (value, min, max) in [
        (settings.code_font_size, 8., 40.),
        (settings.sidebar_width, 180., 1000.),
        (settings.bottom_panel_height, 100., 2000.),
        (settings.bottom_panel_width, 100., 2000.),
    ] {
        if !value.is_finite() || !(min..=max).contains(&value) {
            return Err("Imported settings contain an invalid size".into());
        }
    }
    if settings
        .billing_budget_usd
        .is_some_and(|n| !n.is_finite() || n < 0.)
    {
        return Err("Invalid budget".into());
    }
    Ok(settings)
}

pub(super) fn repository_url(project: &Project) -> Option<String> {
    if project.is_remote() || project.is_chat_space() {
        return None;
    }
    if let Some(pending) = &project.settings.pending_clone {
        return RepositoryLink::parse(&pending.url).map(|link| link.url);
    }
    dunlin_workspace::git::hosting::detect(&project.path).map(|link| link.url)
}

pub(super) fn export(
    projects: &[Project],
    groups: &[ProjectGroup],
    settings: Option<Value>,
    include_projects: bool,
) -> Result<(Backup, Vec<String>), String> {
    let mut skipped = Vec::new();
    let catalog = if include_projects {
        // Folder-derived IDs can contain local paths; replace every ID, not just project.path.
        let ids: HashMap<_, _> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.id.as_str(), format!("g{i}")))
            .collect();
        let groups = groups
            .iter()
            .map(|g| Group {
                id: ids[g.id.as_str()].clone(),
                name: g.name.clone(),
                tone: g.tone,
                parent_id: g.parent_id.as_deref().and_then(|id| ids.get(id)).cloned(),
            })
            .collect();
        let mut repositories = Vec::new();
        let mut seen = HashSet::new();
        for project in projects.iter().filter(|p| !p.is_chat_space()) {
            if let Some(url) = repository_url(project) {
                if !seen.insert(url.to_ascii_lowercase()) {
                    skipped.push(format!(
                        "{} · duplicate repository URL — excluded",
                        project.name
                    ));
                    continue;
                }
                repositories.push(Repository {
                    name: project.name.clone(),
                    url,
                    group_id: project
                        .group_id
                        .as_deref()
                        .and_then(|id| ids.get(id))
                        .cloned(),
                    tone: project.tone,
                    pinned: project.pinned,
                });
            } else {
                skipped.push(format!(
                    "{} · {}",
                    project.name,
                    if project.is_remote() {
                        "remote host / relay project — excluded"
                    } else {
                        "no supported GitHub/GitLab remote — excluded"
                    }
                ));
            }
        }
        Some(Catalog {
            groups,
            projects: repositories,
        })
    } else {
        None
    };
    let backup = Backup {
        format: "dunlin-backup".into(),
        version: 1,
        settings,
        catalog,
    };
    backup.validate()?;
    Ok((backup, skipped))
}
fn label(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err("Invalid backup label".into());
    }
    Ok(())
}
impl Backup {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Choose a regular JSON backup file".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::parse(&bytes)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Backup exceeds 1 MiB".into());
        }
        let backup: Self =
            serde_json::from_slice(bytes).map_err(|_| "Invalid Dunlin backup JSON")?;
        backup.validate()?;
        Ok(backup)
    }
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Backup exceeds 1 MiB".into());
        }
        Ok(bytes)
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != "dunlin-backup" || self.version != 1 {
            return Err("Unsupported backup format/version".into());
        }
        if self.settings.is_none() && self.catalog.is_none() {
            return Err("Backup contains no settings or projects".into());
        }
        if let Some(settings) = &self.settings {
            merge_settings(&StoredSettings::default(), settings)?;
        }
        if let Some(catalog) = &self.catalog {
            if catalog.groups.len() > 1000 || catalog.projects.len() > 2000 {
                return Err("Backup exceeds 1000 groups / 2000 projects".into());
            }
            let mut ids = HashSet::new();
            let mut siblings = HashSet::new();
            for group in &catalog.groups {
                label(&group.id)?;
                label(&group.name)?;
                if !ids.insert(&group.id) {
                    return Err("Duplicate group ID".into());
                }
                if !siblings.insert((group.parent_id.as_ref(), group.name.to_ascii_lowercase())) {
                    return Err("Duplicate sibling group name".into());
                }
            }
            let groups: Vec<_> = catalog
                .groups
                .iter()
                .map(|g| ProjectGroup {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    parent_id: g.parent_id.clone(),
                    tone: g.tone,
                    settings: Default::default(),
                })
                .collect();
            for group in &groups {
                group.validate_parent(&groups)?;
            }
            let mut urls = HashSet::new();
            for project in &catalog.projects {
                label(&project.name)?;
                let link = RepositoryLink::parse(&project.url)
                    .ok_or("Invalid GitHub/GitLab repository URL")?;
                if link.url != project.url {
                    return Err("Use canonical HTTPS repository URLs without credentials".into());
                }
                if !urls.insert(project.url.to_ascii_lowercase()) {
                    return Err("Duplicate repository URL".into());
                }
                if project
                    .group_id
                    .as_ref()
                    .is_some_and(|id| !ids.contains(id))
                {
                    return Err("Missing project group".into());
                }
            }
        }
        Ok(())
    }
}

pub(super) struct Import {
    pub groups: Vec<ProjectGroup>,
    pub projects: Vec<Project>,
    pub duplicates: usize,
}
pub(super) fn prepare(
    catalog: &Catalog,
    existing_groups: &[ProjectGroup],
    existing_urls: &HashSet<String>,
    destination: &Path,
) -> Result<Import, String> {
    if !destination.is_absolute() {
        return Err("Choose an absolute download folder".into());
    }
    let mut all = existing_groups.to_vec();
    let mut map: HashMap<String, String> = HashMap::new();
    let mut added = Vec::new();
    while map.len() < catalog.groups.len() {
        let before = map.len();
        for group in &catalog.groups {
            if map.contains_key(&group.id)
                || group
                    .parent_id
                    .as_ref()
                    .is_some_and(|id| !map.contains_key(id))
            {
                continue;
            }
            let parent = group.parent_id.as_ref().map(|id| map[id].clone());
            let id = if let Some(existing) = all
                .iter()
                .find(|g| g.parent_id == parent && g.name.eq_ignore_ascii_case(&group.name))
            {
                existing.id.clone()
            } else {
                let new = ProjectGroup {
                    id: format!("group-{}", uuid::Uuid::now_v7()),
                    name: group.name.clone(),
                    parent_id: parent,
                    tone: group.tone,
                    settings: Default::default(),
                };
                let id = new.id.clone();
                all.push(new.clone());
                added.push(new);
                id
            };
            map.insert(group.id.clone(), id);
        }
        if before == map.len() {
            return Err("Missing or cyclic group ancestry".into());
        }
    }
    let mut projects = Vec::new();
    let mut duplicates = 0;
    for repo in &catalog.projects {
        if existing_urls.contains(&repo.url.to_ascii_lowercase()) {
            duplicates += 1;
            continue;
        }
        let link = RepositoryLink::parse(&repo.url).ok_or("Invalid repository URL")?;
        let suffix = link.url.strip_prefix("https://").unwrap();
        projects.push(Project {
            id: format!("project-{}", uuid::Uuid::now_v7()),
            name: repo.name.clone(),
            path: destination.join(suffix).to_string_lossy().into_owned(),
            branch: None,
            tone: repo.tone,
            pinned: repo.pinned,
            trusted: false,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: repo.group_id.as_ref().map(|id| map[id].clone()),
            settings: ProjectSettings {
                pending_clone: Some(PendingClone {
                    url: repo.url.clone(),
                    destination: destination.to_string_lossy().into_owned(),
                }),
                ..Default::default()
            },
        });
    }
    Ok(Import {
        groups: added,
        projects,
        duplicates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_endpoint_consent_is_explicit_local_and_not_portable() {
        let legacy: StoredSettings = serde_json::from_str("{}").unwrap();
        assert!(!legacy.opencode_free_consent);
        let accepted = StoredSettings {
            opencode_free_consent: true,
            ..Default::default()
        };
        let stored = serde_json::to_value(&accepted).unwrap();
        assert!(
            serde_json::from_value::<StoredSettings>(stored)
                .unwrap()
                .opencode_free_consent
        );
        let exported = portable(&accepted).unwrap();
        assert!(exported.get("opencode_free_consent").is_none());
        assert!(
            !merge_settings(&legacy, &exported)
                .unwrap()
                .opencode_free_consent
        );
        assert!(
            merge_settings(&accepted, &exported)
                .unwrap()
                .opencode_free_consent
        );
        for value in [true, false] {
            assert!(
                merge_settings(
                    &legacy,
                    &serde_json::json!({"opencode_free_consent": value})
                )
                .is_err()
            );
        }
    }
    fn fixture() -> Backup {
        Backup::parse(br#"{"format":"dunlin-backup","version":1,"catalog":{"groups":[{"id":"child","name":"Child","parent_id":"root","tone":"Blue"},{"id":"root","name":"Root","parent_id":null,"tone":"Teal"}],"projects":[{"name":"Repo","url":"https://gitlab.com/g/sub/r","group_id":"child","tone":"Green","pinned":true}]}}"#).unwrap()
    }
    #[test]
    fn portable_settings_never_transfer_paths_commands_secrets_or_permissions() {
        let settings = StoredSettings {
            custom_harness_command: "/private/secret --token=secret".into(),
            hidden_harnesses: vec!["Claude Code".into()],
            _legacy_terminal_ssh_path: "/private/key".into(),
            watched_project_folders: vec!["/work/local".into()],
            permission_mode: dunlin_domain::PermissionMode::AutoApprove,
            ..Default::default()
        };
        let value = portable(&settings).unwrap();
        assert_eq!(
            value["hidden_harnesses"],
            serde_json::json!(["Claude Code"])
        );
        let restored = merge_settings(&StoredSettings::default(), &value).unwrap();
        assert_eq!(restored.hidden_harnesses, settings.hidden_harnesses);
        let text = value.to_string();
        assert!(
            !text.contains("/private") && !text.contains("/work") && !text.contains("AutoApprove")
        );
        let merged = merge_settings(&settings, &serde_json::json!({"chat_logo":false})).unwrap();
        assert!(!merged.chat_logo);
        assert_eq!(
            merged.custom_harness_command,
            settings.custom_harness_command
        );
        for bad in [
            serde_json::json!({"permission_mode":"AutoApprove"}),
            serde_json::json!({"code_font_size":-1}),
            serde_json::json!({"ui_font_family":"x\ny"}),
            serde_json::json!({"hidden_harnesses":["x\ny"]}),
        ] {
            assert!(merge_settings(&settings, &bad).is_err());
        }
    }
    #[test]
    fn hierarchy_is_parent_first_additive_idempotent_and_deferred() {
        let backup = fixture();
        let catalog = backup.catalog.as_ref().unwrap();
        let import = prepare(catalog, &[], &HashSet::new(), Path::new("/chosen")).unwrap();
        assert_eq!(import.groups[0].name, "Root");
        assert_eq!(
            import.groups[1].parent_id.as_ref(),
            Some(&import.groups[0].id)
        );
        let project = &import.projects[0];
        assert!(project.is_not_downloaded() && !project.trusted);
        assert_eq!(project.path, "/chosen/gitlab.com/g/sub/r");
        let again = prepare(
            catalog,
            &import.groups,
            &HashSet::from(["https://gitlab.com/g/sub/r".into()]),
            Path::new("/other"),
        )
        .unwrap();
        assert!(again.groups.is_empty() && again.projects.is_empty());
        assert_eq!(again.duplicates, 1);
        let (exported, skipped) = export(&import.projects, &import.groups, None, true).unwrap();
        assert!(skipped.is_empty());
        let text = String::from_utf8(exported.bytes().unwrap()).unwrap();
        assert!(
            !text.contains("/chosen")
                && !text.contains("pending_clone")
                && !text.contains(&project.id)
        );
    }
    #[test]
    fn malicious_or_unknown_backups_are_rejected_before_any_write() {
        let base = serde_json::to_value(fixture()).unwrap();
        for (pointer, value) in [
            ("/version", serde_json::json!(2)),
            (
                "/catalog/projects/0/url",
                serde_json::json!("https://token@github.com/o/r"),
            ),
            (
                "/catalog/projects/0/url",
                serde_json::json!("file:///private/repo"),
            ),
            ("/catalog/projects/0/group_id", serde_json::json!("missing")),
            ("/catalog/groups/1/parent_id", serde_json::json!("child")),
        ] {
            let mut v = base.clone();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(Backup::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        assert!(Backup::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
    #[test]
    fn real_git_remotes_are_exported_but_relay_projects_paths_and_runtime_secrets_are_not() {
        let temp = tempfile::tempdir().unwrap();
        let import = prepare(
            fixture().catalog.as_ref().unwrap(),
            &[],
            &HashSet::new(),
            temp.path(),
        )
        .unwrap();
        let mut groups = import.groups;
        groups[0]
            .settings
            .environment
            .insert("TOKEN".into(), "group-secret".into());
        let mut template = import.projects[0].clone();
        template.settings.pending_clone = None;
        template
            .settings
            .environment
            .insert("TOKEN".into(), "project-secret".into());
        let mut projects = Vec::new();
        for (name, remote) in [
            ("github", "git@github.com:o/r.git"),
            ("gitlab", "https://gitlab.com/g/sub/r.git"),
            ("unsupported", "https://other.example/o/r"),
        ] {
            let path = temp.path().join(name);
            let init = std::process::Command::new("git")
                .args(["init", "--template=", "-q"])
                .arg(&path)
                .status()
                .unwrap();
            assert!(init.success());
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(["remote", "add", "origin", remote])
                    .status()
                    .unwrap()
                    .success()
            );
            let mut project = template.clone();
            project.id = name.into();
            project.name = name.into();
            project.path = path.to_string_lossy().into_owned();
            projects.push(project);
        }
        let mut relay = projects[0].clone();
        relay.name = "relay-only".into();
        relay.remote = Some(dunlin_domain::RemoteProject {
            host_id: "secret-host".into(),
            project_id: "relay-project".into(),
            path: "/server/private".into(),
        });
        projects.push(relay);
        let mut duplicate = projects[0].clone();
        duplicate.name = "duplicate".into();
        projects.push(duplicate);
        let (backup, skipped) = export(&projects, &groups, None, true).unwrap();
        let repos = &backup.catalog.as_ref().unwrap().projects;
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].url, "https://github.com/o/r");
        assert_eq!(repos[1].url, "https://gitlab.com/g/sub/r");
        assert_eq!(skipped.len(), 3);
        assert!(
            skipped
                .iter()
                .any(|s| s.contains("relay-only") && s.contains("excluded"))
        );
        let text = String::from_utf8(backup.bytes().unwrap()).unwrap();
        for private in [
            temp.path().to_str().unwrap(),
            "group-secret",
            "project-secret",
            "secret-host",
            "/server/private",
            "relay-only",
            "unsupported",
        ] {
            assert!(!text.contains(private), "leaked {private}");
        }
    }
    #[test]
    fn old_project_settings_remain_downloaded_and_dashboard_history_stays_local() {
        let settings: ProjectSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.pending_clone.is_none());
        let current = StoredSettings {
            dashboard_settings: serde_json::from_value(
                serde_json::json!({"recent_projects":[{"id":"group-folder:/private/work","at_ms":42}]}),
            )
            .unwrap(),
            ..Default::default()
        };
        let exported = portable(&current).unwrap();
        assert!(!exported.to_string().contains("/private"));
        let merged = merge_settings(
            &current,
            &serde_json::json!({"dashboard_settings":{"show_activity":false}}),
        )
        .unwrap();
        assert!(!merged.dashboard_settings.show_activity);
        assert_eq!(
            serde_json::to_value(merged.dashboard_settings).unwrap()["recent_projects"],
            serde_json::to_value(&current.dashboard_settings).unwrap()["recent_projects"]
        );
        assert!(
            merge_settings(
                &current,
                &serde_json::json!({"dashboard_settings":{"recent_projects":[]}})
            )
            .is_err()
        );
    }

    #[test]
    fn portable_skin_roundtrips_and_old_backups_preserve_the_current_skin() {
        use crate::app::Appearance;
        use dunlin_ui::Skin;
        for skin in Skin::ALL {
            let current = StoredSettings {
                skin,
                appearance: Some(Appearance::Dark),
                ..Default::default()
            };
            let exported = portable(&current).unwrap();
            let restored = merge_settings(&StoredSettings::default(), &exported).unwrap();
            assert_eq!(restored.skin, skin);
            assert_eq!(restored.appearance, Some(Appearance::Dark));
            let merged =
                merge_settings(&current, &serde_json::json!({"appearance":"Light"})).unwrap();
            assert_eq!(merged.skin, skin);
            assert_eq!(merged.appearance, Some(Appearance::Light));
        }
    }
}
