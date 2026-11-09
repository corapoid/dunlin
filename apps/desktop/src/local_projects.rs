//! Bulk import keeps existing identities/settings and never activates every
//! discovered repository. Only explicit top-level selections may be symlinks.
use crate::app::DunlinApp;
use dunlin_domain::{Project, ProjectGroup, ProjectTone};
use dunlin_ui::*;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

fn collection_group(root: &Path, groups: &[ProjectGroup]) -> ProjectGroup {
    // Canonical folder identity survives rescans without merging same-named folders.
    let id = format!("group-folder:{}", root.to_string_lossy());
    if let Some(group) = groups.iter().find(|group| group.id == id) {
        return group.clone();
    }
    let base = root
        .file_name()
        .unwrap_or(root.as_os_str())
        .to_string_lossy();
    let mut name = base.to_string();
    let mut suffix = 2;
    while groups
        .iter()
        .any(|group| group.parent_id.is_none() && group.name.eq_ignore_ascii_case(&name))
    {
        name = format!("{base} ({suffix})");
        suffix += 1;
    }
    ProjectGroup {
        id,
        name,
        tone: ProjectTone::Blue,
        parent_id: None,
        settings: Default::default(),
    }
}

pub(crate) fn canonical_local_paths(projects: &[Project]) -> HashSet<PathBuf> {
    projects
        .iter()
        .filter(|p| !p.is_remote() && !p.is_chat_space())
        .filter_map(|p| std::fs::canonicalize(&p.path).ok())
        .collect()
}
fn activate_import(was_empty: bool, single: bool, count: usize, same_navigation: bool) -> bool {
    same_navigation && (was_empty || (single && count == 1))
}
impl DunlinApp {
    pub(crate) fn import_local_folders(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.local_import_running || self.backup.busy || self.backup.cloning {
            self.file_notice("Repository discovery is already running", cx);
            return;
        }
        self.local_import_running = true;
        let navigation = self.navigation_epoch();
        let single = paths.len() == 1;
        self.file_notice("Looking for local repositories…", cx);
        cx.spawn(async move |this, cx| {
            let discovery = cx
                .background_executor()
                .spawn(async move { dunlin_workspace::repositories::discover(&paths) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.local_import_running = false;
                this.finish_project_import(discovery, single, Some(navigation), true, None, cx);
            });
        })
        .detach();
    }

    pub(crate) fn finish_project_import(
        &mut self,
        discovery: dunlin_workspace::repositories::Discovery,
        single: bool,
        navigation: Option<u64>,
        trusted: bool,
        target_group: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let this = self;
        if target_group
            .as_ref()
            .is_some_and(|id| !this.project_groups.iter().any(|g| &g.id == id))
        {
            this.file_notice(
                "The selected group no longer exists. Reopen import and select a group.",
                cx,
            );
            return;
        }
        let mut existing = canonical_local_paths(&this.projects);
        let was_empty = this.projects.iter().all(|p| p.is_chat_space());
        let mut first = None;
        let mut added = 0;
        let mut duplicate = 0;
        let mut failures = discovery.warnings;
        let mut collections = HashMap::new();
        for root in discovery
            .projects
            .iter()
            .filter_map(|project| project.collection.as_ref())
            .filter(|_| target_group.is_none())
        {
            if collections.contains_key(root) {
                continue;
            }
            let group = collection_group(root, &this.project_groups);
            if !this
                .project_groups
                .iter()
                .any(|existing| existing.id == group.id)
            {
                if let Some(store) = &this.store
                    && let Err(error) = store.upsert_project_group(&group)
                {
                    failures.push(error.to_string());
                    collections.insert(root.clone(), None);
                    continue;
                }
                this.project_groups.push(group.clone());
            }
            collections.insert(root.clone(), Some(group.id));
        }
        for candidate in discovery.projects {
            let group_id = target_group.clone().or_else(|| {
                candidate
                    .collection
                    .as_ref()
                    .and_then(|root| collections.get(root))
                    .cloned()
                    .flatten()
            });
            if candidate.collection.is_some() && group_id.is_none() {
                continue;
            }
            if existing.contains(&candidate.path) {
                duplicate += 1;
                if let Some(index) = this.projects.iter().position(|p| {
                    !p.is_remote()
                        && !p.is_chat_space()
                        && std::fs::canonicalize(&p.path).ok().as_ref() == Some(&candidate.path)
                }) {
                    first.get_or_insert(index);
                    if this.projects[index].group_id.is_none()
                        && group_id.is_some()
                        && !this.assign_project_group(index, group_id, cx)
                    {
                        failures.push(this.notice.clone().unwrap_or_else(|| {
                            "Could not assign the imported project to its group".into()
                        }));
                    }
                }
                continue;
            }
            let path = candidate.path.to_string_lossy().into_owned();
            let name = candidate
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let tones = [
                ProjectTone::Blue,
                ProjectTone::Green,
                ProjectTone::Violet,
                ProjectTone::Amber,
                ProjectTone::Rose,
                ProjectTone::Teal,
            ];
            let project = Project {
                id: format!("project-{}", uuid::Uuid::now_v7()),
                name,
                path,
                branch: candidate.branch,
                tone: tones[this.projects.len() % tones.len()],
                pinned: false,
                trusted,
                running_sessions: 0,
                kind: Default::default(),
                remote: None,
                group_id,
                settings: Default::default(),
            };
            if let Some(store) = &this.store
                && let Err(error) = store.upsert_project(&project)
            {
                failures.push(error.to_string());
                continue;
            }
            existing.insert(candidate.path);
            this.repo_summaries.values.insert(
                project.id.clone(),
                crate::project_groups::RepoSummary {
                    path: project.path.clone(),
                    branch: project.branch.clone(),
                    changes: None,
                    activity: Default::default(),
                    monorepo: candidate.monorepo,
                    languages: None,
                    size_bytes: None,
                },
            );
            this.projects.push(project);
            first.get_or_insert(this.projects.len() - 1);
            added += 1;
        }
        this.repo_summaries.invalidate();
        // Do not interrupt the current project/buffers while adding a
        // collection. The normal single-project selection guard stays.
        if activate_import(
            was_empty,
            single,
            added + duplicate,
            navigation == Some(this.navigation_epoch()),
        ) && let Some(index) = first
        {
            this.select_project(index, cx);
        }
        this.persist_settings(cx);
        let mut message = format!("Added {added} projects · {duplicate} already open");
        if discovery.limited {
            message.push_str(" · discovery limit reached; select smaller folders for the remainder (8 levels / 20,000 entries / 256 projects / 8 seconds)");
        }
        if !failures.is_empty() {
            message.push_str(&format!(
                " · {} folders could not be imported: {}",
                failures.len(),
                failures[0]
            ));
        }
        this.file_notice(message.clone(), cx);
        if discovery.limited || !failures.is_empty() {
            // Partial import reports must remain available until the
            // user dismisses them, not expire like a routine file toast.
            this.toast = Some(crate::app::Toast {
                id: uuid::Uuid::now_v7(),
                message,
                action_label: "OK".into(),
                action: crate::app::ToastAction::Info,
            });
        }
    }
}

pub(crate) fn monorepo_badge(
    this: &DunlinApp,
    project: &Project,
    p: &Palette,
) -> Option<AnyElement> {
    let kind = this
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|s| s.path == project.path)?
        .monorepo
        .clone()?;
    Some(div().flex().flex_shrink_0().child(div().id(format!("monorepo:{}", project.id)).px(px(4.)).py(px(1.)).rounded(px(dunlin_ui::radii::CODE)).border_1().border_color(p.border_strong).bg(p.muted).text_size(px(9.)).text_color(p.muted_fg).child("Monorepo")
        .tooltip(move |window, cx| component::tooltip::Tooltip::new(format!("Detected: {kind}. Packages stay inside this project; no scripts are executed.")).build(window, cx))).into_any_element())
}

pub(crate) fn language_badge(
    this: &DunlinApp,
    project: &Project,
    p: &Palette,
) -> Option<AnyElement> {
    let summary = this
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|row| row.path == project.path)?;
    let languages = summary.languages.as_ref()?;
    let total = languages.iter().map(|stat| stat.bytes).sum::<u64>();
    if total == 0 {
        return None;
    }
    let dominant = languages.first()?;
    let dominant_pct = dominant.bytes as f32 * 100.0 / total as f32;
    let color: Hsla = rgb(dominant.color).into();
    let tooltip_languages = languages.clone();
    let palette = *p;
    Some(
        div()
            .id(format!("project-languages:{}", project.id))
            .flex()
            .items_center()
            .justify_start()
            .gap(px(4.))
            .child(dot(color, 5.0))
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(p.muted_fg)
                    .child(format!("{} {:.0}%", dominant.name, dominant_pct)),
            )
            .tooltip(move |window, cx| {
                let languages = tooltip_languages.clone();
                component::tooltip::Tooltip::element(move |_window, _cx| {
                    let strip = segmented_bar(
                        languages
                            .iter()
                            .map(|stat| (stat.bytes as f32, Hsla::from(rgb(stat.color)))),
                    );
                    let mut rows = div().flex().flex_col().gap(px(4.));
                    for stat in &languages {
                        let fraction = stat.bytes as f32 / total as f32;
                        let stat_color: Hsla = rgb(stat.color).into();
                        rows = rows.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(dot(stat_color, 6.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.0))
                                        .text_color(palette.fg)
                                        .child(stat.name),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_family(dunlin_ui::theme::code_font())
                                        .text_color(palette.muted_fg)
                                        .child(format!("{:.1}%", fraction * 100.0)),
                                ),
                        );
                    }
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .min_w(px(190.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.fg)
                                .child("Languages"),
                        )
                        .child(strip)
                        .child(rows)
                        .into_any_element()
                })
                .build(window, cx)
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn collection_group_identity_is_stable_and_preserves_user_edits() {
        let root = std::path::Path::new("/work/product");
        let mut group = super::collection_group(root, &[]);
        assert_eq!(group.name, "product");
        group.name = "My product".into();
        group.settings.harness = Some("Claude Code".into());
        let repeated = super::collection_group(root, &[group.clone()]);
        assert_eq!(repeated.id, group.id);
        assert_eq!(repeated.name, "My product");
        assert_eq!(repeated.settings.harness.as_deref(), Some("Claude Code"));
        let mut original = group.clone();
        original.name = "product".into();
        let different =
            super::collection_group(std::path::Path::new("/other/product"), &[original]);
        assert_ne!(different.id, group.id);
        assert_eq!(different.name, "product (2)");
    }

    #[test]
    fn collection_imports_do_not_switch_every_project_or_interrupt_later_navigation() {
        assert!(!super::activate_import(false, true, 12, true));
        assert!(!super::activate_import(false, false, 2, true));
        assert!(super::activate_import(true, false, 12, true));
        assert!(super::activate_import(false, true, 1, true));
        assert!(!super::activate_import(true, true, 1, false));
    }
    #[cfg(unix)]
    #[test]
    fn canonical_duplicates_do_not_change_saved_project_settings_or_remote_identities() {
        let root = std::env::temp_dir().join(format!("dunlin-import-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&root).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let mut project = dunlin_domain::Project {
            id: "old".into(),
            name: "Custom name".into(),
            path: root.to_string_lossy().into(),
            branch: Some("main".into()),
            tone: dunlin_domain::ProjectTone::Green,
            pinned: true,
            trusted: false,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: Some("group".into()),
            settings: Default::default(),
        };
        project.settings.harness = Some("Custom harness".into());
        let mut duplicate = project.clone();
        duplicate.path = alias.to_string_lossy().into();
        let paths = super::canonical_local_paths(&[project.clone(), duplicate]);
        assert_eq!(paths.len(), 1);
        assert!(project.pinned);
        assert!(!project.trusted);
        assert_eq!(project.group_id.as_deref(), Some("group"));
        assert_eq!(project.settings.harness.as_deref(), Some("Custom harness"));
        project.remote = Some(dunlin_domain::RemoteProject::default());
        assert!(super::canonical_local_paths(&[project]).is_empty());
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
