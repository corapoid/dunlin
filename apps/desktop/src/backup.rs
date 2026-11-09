//! Explicit portable backup/import and sequential, on-demand checkouts.
mod data;
use crate::app::{Appearance, DunlinApp};
use data::Backup;
use dunlin_ui::{
    component::{Disableable, button::Button, input::InputState},
    prelude::FluentBuilder as _,
    *,
};
use dunlin_workspace::catalog_backup as hosted;
use std::{collections::HashSet, io::Write as _, path::PathBuf};

struct Pending {
    backup: Backup,
    importing: bool,
    repository: Option<String>,
    skipped: Vec<String>,
}
pub(crate) struct State {
    pub busy: bool,
    pub cloning: bool,
    include_settings: bool,
    include_projects: bool,
    repository: Option<Entity<InputState>>,
    destination: Option<Entity<InputState>>,
    pending: Option<Pending>,
    message: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            busy: false,
            cloning: false,
            include_settings: true,
            include_projects: true,
            repository: None,
            destination: None,
            pending: None,
            message: None,
        }
    }
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
fn destination(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let path = if value == "~" {
        home()
    } else if let Some(rest) = value.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() || value.contains('\0') {
        return Err("Choose an absolute download folder (or ~/…)".into());
    }
    Ok(path)
}
fn button(id: &'static str, label: &str, enabled: bool) -> Button {
    dunlin_ui::button(id)
        .label(label.to_owned())
        .disabled(!enabled)
}
impl DunlinApp {
    fn backup_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.backup.busy = false;
        self.backup.message = Some(message.into());
        cx.notify();
    }
    fn prepare_backup_export(&mut self, repository: bool, cx: &mut Context<Self>) {
        if self.backup.busy || self.factory_resetting {
            return;
        }
        if !self.backup.include_settings && !self.backup.include_projects {
            self.backup_error("Choose settings and/or project structure", cx);
            return;
        }
        let target = if repository {
            let value = self.backup.repository.as_ref().unwrap().read(cx).value();
            match dunlin_workspace::git::hosting::RepositoryLink::parse(value.trim()) {
                Some(link) => Some(link.url),
                None => {
                    self.backup_error(
                        "Enter an explicit GitHub.com or GitLab.com repository URL",
                        cx,
                    );
                    return;
                }
            }
        } else {
            None
        };
        let settings = if self.backup.include_settings {
            match data::portable(&self.stored_settings(cx)) {
                Ok(s) => Some(s),
                Err(e) => {
                    self.backup_error(e, cx);
                    return;
                }
            }
        } else {
            None
        };
        let projects = self.projects.clone();
        let groups = self.project_groups.clone();
        let include_projects = self.backup.include_projects;
        self.backup.busy = true;
        self.backup.pending = None;
        self.backup.message = Some("Reading repository remotes…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { data::export(&projects, &groups, settings, include_projects) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup.busy = false;
                match result {
                    Ok((backup, skipped)) => {
                        this.backup.pending = Some(Pending {
                            backup,
                            skipped,
                            importing: false,
                            repository: target,
                        });
                        this.backup.message = None;
                    }
                    Err(e) => this.backup.message = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn read_backup(&mut self, repository: bool, cx: &mut Context<Self>) {
        if self.backup.busy || self.factory_resetting {
            return;
        }
        let url = if repository {
            let value = self.backup.repository.as_ref().unwrap().read(cx).value();
            match dunlin_workspace::git::hosting::RepositoryLink::parse(value.trim()) {
                Some(link) => Some(link.url),
                None => {
                    self.backup_error(
                        "Enter an explicit GitHub.com or GitLab.com repository URL",
                        cx,
                    );
                    return;
                }
            }
        } else {
            None
        };
        let paths = if !repository {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose Dunlin JSON backup".into()),
            });
            Some(self.overlay_backdrop.track_native(receiver, cx))
        } else {
            None
        };
        self.backup.busy = true;
        self.backup.pending = None;
        self.backup.message = Some("Reading backup…".into());
        cx.spawn(async move |this, cx| {
            let path = if let Some(paths) = paths {
                match paths.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    Ok(Ok(None)) => None,
                    _ => {
                        let _ = this.update(cx, |this, cx| {
                            this.backup_error("Could not select backup file", cx)
                        });
                        return;
                    }
                }
            } else {
                None
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(url) = url {
                        hosted::download(&url)
                            .and_then(|bytes| Backup::parse(&bytes))
                            .map(Some)
                    } else if let Some(path) = path {
                        Backup::read(&path).map(Some)
                    } else {
                        Ok(None)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup.busy = false;
                this.backup.message = None;
                match result {
                    Ok(Some(backup)) => {
                        this.backup.pending = Some(Pending {
                            backup,
                            importing: true,
                            repository: None,
                            skipped: Vec::new(),
                        })
                    }
                    Err(e) => this.backup.message = Some(e),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn write_backup(&mut self, cx: &mut Context<Self>) {
        if self.backup.busy || self.factory_resetting {
            return;
        }
        let Some(pending) = &self.backup.pending else {
            return;
        };
        if pending.importing {
            return;
        }
        let bytes = match pending.backup.bytes() {
            Ok(bytes) => bytes,
            Err(e) => {
                self.backup_error(e, cx);
                return;
            }
        };
        let repository = pending.repository.clone();
        let file = if repository.is_none() {
            let receiver = cx.prompt_for_new_path(&home(), Some(hosted::FILE));
            Some(self.overlay_backdrop.track_native(receiver, cx))
        } else {
            None
        };
        self.backup.busy = true;
        self.backup.message = Some("Saving backup…".into());
        cx.spawn(async move |this, cx| {
            let path = if let Some(file) = file {
                match file.await {
                    Ok(Ok(Some(path))) => Some(path),
                    Ok(Ok(None)) => {
                        let _ =
                            this.update(cx, |this, cx| this.backup_error("Export cancelled", cx));
                        return;
                    }
                    _ => {
                        let _ = this.update(cx, |this, cx| {
                            this.backup_error("Could not select backup destination", cx)
                        });
                        return;
                    }
                }
            } else {
                None
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(url) = repository {
                        hosted::upload(&url, &bytes)
                            .map(|_| format!("Saved {} to {url}", hosted::FILE))
                    } else {
                        let path = path.unwrap();
                        let mut temp = tempfile::NamedTempFile::new_in(
                            path.parent().ok_or("Invalid backup destination")?,
                        )
                        .map_err(|e| e.to_string())?;
                        temp.write_all(&bytes).map_err(|e| e.to_string())?;
                        temp.as_file().sync_all().map_err(|e| e.to_string())?;
                        temp.persist(&path).map_err(|e| e.to_string())?;
                        Ok(format!("Saved backup to {}", path.display()))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup.busy = false;
                match result {
                    Ok(message) => {
                        this.backup.pending = None;
                        this.backup.message = Some(message);
                    }
                    Err(e) => this.backup.message = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn import_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.backup.busy
            || self.backup.cloning
            || self.factory_resetting
            || self.local_import_running
            || self.project_import.busy
        {
            return;
        }
        let Some(pending) = &self.backup.pending else {
            return;
        };
        if !pending.importing {
            return;
        }
        let backup = pending.backup.clone();
        let settings = backup.settings.filter(|_| self.backup.include_settings);
        let catalog = backup.catalog.filter(|_| self.backup.include_projects);
        if settings.is_none() && catalog.is_none() {
            self.backup_error("No selected data is present in this backup", cx);
            return;
        }
        let destination = if catalog.is_some() {
            match destination(&self.backup.destination.as_ref().unwrap().read(cx).value()) {
                Ok(path) => path,
                Err(e) => {
                    self.backup_error(e, cx);
                    return;
                }
            }
        } else {
            home()
        };
        let existing = self.projects.clone();
        let groups = self.project_groups.clone();
        let signature = serde_json::to_value((&existing, &groups)).unwrap();
        self.backup.busy = true;
        self.backup.message = Some("Preparing additive import…".into());
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let urls: HashSet<_> = if catalog.is_some() { existing.iter().filter_map(data::repository_url).map(|url| url.to_ascii_lowercase()).collect() } else { HashSet::new() };
                catalog.as_ref().map(|c| data::prepare(c, &groups, &urls, &destination)).transpose()
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.backup.busy = false;
                let result = (|| -> Result<String, String> {
                    if this.factory_resetting || serde_json::to_value((&this.projects, &this.project_groups)).unwrap() != signature { return Err("Projects changed. Preview and import again.".into()); }
                    let import = result?;
                    let (groups, projects, duplicates) = import.map(|i| (i.groups,i.projects,i.duplicates)).unwrap_or_default();
                    let settings = settings.as_ref().map(|s| data::merge_settings(&this.stored_settings(cx), s)).transpose()?;
                    let imported_settings = settings.is_some();
                    let saved = settings.as_ref().map(serde_json::to_value).transpose().map_err(|e| e.to_string())?;
                    this.store.as_ref().ok_or("Local storage unavailable; nothing imported")?.import_catalog(&groups, &projects, saved.as_ref()).map_err(|e| e.to_string())?;
                    let count = projects.len(); this.project_groups.extend(groups); this.projects.extend(projects);
                    if let Some(settings) = settings {
                        macro_rules! apply_one {
                            (appearance) => { this.set_appearance(settings.appearance.unwrap_or(Appearance::System), window, cx); };
                            ($field:ident) => { this.$field = settings.$field.clone(); };
                        }
                        macro_rules! apply { ($($field:ident),*) => { $(apply_one!($field);)* }; }
                        data::preferences!(apply);
                        dunlin_ui::theme::set_font_families(&this.ui_font_family, &this.code_font_family, cx);
                        dunlin_ui::theme::set_code_font_size(this.code_font_size, cx);
                        this.set_ui_density(this.density, cx);
                    }
                    this.backup.pending = None;
                    this.repo_summaries.invalidate();
                    Ok(format!("Imported {}{count} projects (not downloaded) · {duplicates} already present. Existing projects and histories were kept.", if imported_settings { "settings and " } else { "" }))
                })();
                this.backup.message = Some(result.unwrap_or_else(|e| e));
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(crate) fn download_projects(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if self.backup.cloning
            || self.backup.busy
            || self.factory_resetting
            || self.project_import.busy
            || self.local_import_running
        {
            return;
        }
        let queue: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let p = self
                    .projects
                    .iter()
                    .find(|p| p.id == id && !p.is_remote() && !p.is_chat_space())?;
                Some((id, p.settings.pending_clone.clone()?))
            })
            .collect();
        if queue.is_empty() {
            return;
        }
        self.backup.cloning = true;
        cx.spawn(async move |this, cx| {
            let count = queue.len(); let mut completed = 0; let mut errors = Vec::new();
            for (index, (id, pending)) in queue.into_iter().enumerate() {
                let valid = this.update(cx, |this, cx| {
                    if this.factory_resetting || !this.projects.iter().any(|p| p.id == id && p.settings.pending_clone.as_ref() == Some(&pending)) { return false; }
                    let message = format!("Downloading {}/{} · {}", index + 1, count, pending.url);
                    this.backup.message = Some(message.clone()); this.file_notice(message, cx); true
                }).unwrap_or(false);
                if !valid { continue; }
                let request = pending.clone();
                let result = cx.background_executor().spawn(async move { hosted::clone(&request.url, &PathBuf::from(request.destination)) }).await;
                let result = this.update(cx, |this, cx| -> Result<(), String> {
                    let path = result?;
                    let Some(project) = this.projects.iter_mut().find(|p| p.id == id && p.settings.pending_clone.as_ref() == Some(&pending)) else { return Err("Project changed; downloaded files were kept".into()); };
                    let mut updated = project.clone(); updated.path = path.to_string_lossy().into_owned(); updated.settings.pending_clone = None;
                    // Downloading code never grants trust or starts an agent.
                    updated.trusted = false;
                    this.store.as_ref().ok_or("Local storage unavailable; downloaded files were kept")?.upsert_project(&updated).map_err(|e| format!("{e}; downloaded files were kept"))?;
                    *project = updated; this.repo_summaries.invalidate(); cx.notify(); Ok(())
                });
                match result { Ok(Ok(())) => completed += 1, Ok(Err(e)) => errors.push(e), Err(_) => return }
            }
            let _ = this.update(cx, |this, cx| {
                this.backup.cloning = false;
                let mut message = format!("Downloaded {completed}/{count} repositories. Trust each project before running an agent.");
                if let Some(first) = errors.first() { message.push_str(&format!(" {} failed: {first}. Failed projects keep their download icon.", errors.len())); }
                this.backup.message = Some(message.clone()); this.file_notice(message, cx);
            });
        }).detach();
        cx.notify();
    }
    pub(crate) fn download_group(&mut self, group: &str, cx: &mut Context<Self>) {
        let descendants = crate::project_tree::descendants(&self.project_groups, group);
        let ids = self
            .projects
            .iter()
            .filter(|p| {
                p.is_not_downloaded()
                    && p.group_id
                        .as_ref()
                        .is_some_and(|id| descendants.contains(id))
            })
            .map(|p| p.id.clone())
            .collect();
        self.download_projects(ids, cx);
    }
}

pub(crate) fn settings(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    if this.backup.repository.is_none() {
        this.backup.repository = Some(cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/owner/backup or https://gitlab.com/group/backup")
        }));
        let default = crate::app::data_dir()
            .unwrap_or_else(home)
            .join("repositories");
        this.backup.destination = Some(cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Download folder")
                .default_value(default.to_string_lossy().into_owned())
        }));
    }
    let p = palette(cx);
    let busy = this.backup.busy || this.factory_resetting;
    let mut view = dunlin_ui::settings::card(p).flex().flex_col().gap(px(12.)).p(px(14.))
        .child(dunlin_ui::settings::section("Portable preferences and project structure", p))
        .child(dunlin_ui::settings::description("Export selected preferences and repository/group organization. This is not a full backup of conversations or credentials.", p))
        .child(button("backup-details", if this.settings_ui.backup_details { "Hide export details" } else { "What is included?" }, true)
            .on_click(cx.listener(|this, _, _, cx| { this.settings_ui.backup_details = !this.settings_ui.backup_details; cx.notify(); })))
        .when(this.settings_ui.backup_details, |view| view.child(dunlin_ui::settings::notice("Includes portable preferences and nested groups with GitHub.com / GitLab.com URLs only. Excludes local paths, sessions, trust, permissions, commands, environment variables, credentials, relay configuration, remote projects, unsupported repositories, project/group runtime overrides and custom icons.", p)))
        .child(div().flex().gap(px(8.))
            .child(dunlin_ui::checkbox("backup-settings", "Settings", this.backup.include_settings).label("Settings").disabled(busy).on_click(cx.listener(|this, checked: &bool, _, cx| { this.backup.include_settings = *checked; if this.backup.pending.as_ref().is_some_and(|p| !p.importing) { this.backup.pending = None; } cx.notify(); })))
            .child(dunlin_ui::checkbox("backup-projects", "Project structure", this.backup.include_projects).label("Project structure").disabled(busy).on_click(cx.listener(|this, checked: &bool, _, cx| { this.backup.include_projects = *checked; if this.backup.pending.as_ref().is_some_and(|p| !p.importing) { this.backup.pending = None; } cx.notify(); }))))
        .child(div().flex().gap(px(8.))
            .child(button("backup-export-file", "Export to JSON…", !busy).on_click(cx.listener(|this, _, _, cx| this.prepare_backup_export(false, cx))))
            .child(button("backup-import-file", "Import JSON…", !busy).on_click(cx.listener(|this, _, _, cx| this.read_backup(false, cx)))))
        .child(dunlin_ui::settings::field("Backup repository URL", dunlin_ui::input(this.backup.repository.as_ref().unwrap()).aria_label("Backup repository URL").disabled(busy), p))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Repository export creates/updates dunlin-backup.json with a commit on the default branch using gh/glab. Use an initialized private repository: repository names and preferences may be sensitive. Import reads that file; it does not execute code."))
        .child(div().flex().gap(px(8.))
            .child(button("backup-export-repo", "Export to repository…", !busy).on_click(cx.listener(|this, _, _, cx| this.prepare_backup_export(true, cx))))
            .child(button("backup-import-repo", "Import from repository…", !busy).on_click(cx.listener(|this, _, _, cx| this.read_backup(true, cx)))))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Download folder for imported projects · <host>/<owner or group>/<repo>. Nothing is cloned during import."))
        .child(div().flex().gap(px(8.)).child(div().flex_1().min_w(px(0.)).child(dunlin_ui::input(this.backup.destination.as_ref().unwrap()).aria_label("Download folder for imported projects").disabled(busy)))
            .child(button("backup-folder", "Browse…", !busy).on_click(cx.listener(|this, _, window, cx| {
                let paths = cx.prompt_for_paths(PathPromptOptions { files:false, directories:true, multiple:false, prompt:Some("Choose repository download folder".into()) });
                let paths = this.overlay_backdrop.track_native(paths, cx);
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = paths.await && let Some(path) = paths.first() {
                        let value = path.to_string_lossy().into_owned();
                        let _ = this.update_in(cx, |this, window, cx| { if let Some(input) = &this.backup.destination { input.update(cx, |state,cx| state.set_value(value,window,cx)); } });
                    }
                }).detach();
            }))));
    if let Some(pending) = &this.backup.pending {
        let groups = pending
            .backup
            .catalog
            .as_ref()
            .map_or(0, |c| c.groups.len());
        let projects = pending
            .backup
            .catalog
            .as_ref()
            .map_or(0, |c| c.projects.len());
        view = view.child(div().font_weight(FontWeight::MEDIUM).child(format!(
            "Preview: {}settings · {groups} groups · {projects} repositories",
            if pending.backup.settings.is_some() {
                ""
            } else {
                "no "
            }
        )));
        if let Some(catalog) = &pending.backup.catalog {
            let mut list = div()
                .id("backup-preview")
                .max_h(px(150.))
                .overflow_y_scroll()
                .text_size(px(11.));
            for g in &catalog.groups {
                list = list.child(div().child(format!(
                        "Group: {}{}",
                        g.name,
                        g.parent_id
                            .as_ref()
                            .map(|id| format!(" (parent: {id})"))
                            .unwrap_or_default()
                    )));
            }
            for r in &catalog.projects {
                list = list.child(div().child(format!("{} · {}", r.name, r.url)));
            }
            view = view.child(list);
        }
        if !pending.skipped.is_empty() {
            let mut list = div()
                .id("backup-excluded")
                .max_h(px(120.))
                .overflow_y_scroll()
                .text_size(px(11.))
                .text_color(p.amber)
                .child(format!(
                    "EXCLUDED: {} projects (not part of the export)",
                    pending.skipped.len()
                ));
            for skipped in &pending.skipped {
                list = list.child(div().child(skipped.clone()));
            }
            view = view.child(list);
        }
        if pending.importing {
            view = view.child(div().text_size(px(11.)).text_color(p.muted_fg).child("Import adds missing groups/repositories, skips existing repository URLs and replaces selected portable preferences. Existing projects, local paths and history stay unchanged. New projects require download, then trust."))
                .child(button("backup-confirm-import", "Import selected data", !busy && !this.backup.cloning).on_click(cx.listener(|this, _, window, cx| this.import_backup(window, cx))));
        } else {
            let label = pending
                .repository
                .as_ref()
                .map(|url| format!("Confirm export to {url} (default branch)"))
                .unwrap_or_else(|| "Confirm export to JSON…".into());
            view = view.child(
                button("backup-confirm-export", &label, !busy)
                    .on_click(cx.listener(|this, _, _, cx| this.write_backup(cx))),
            );
        }
        view = view.child(
            button("backup-cancel-preview", "Cancel preview", !busy).on_click(cx.listener(
                |this, _, _, cx| {
                    this.backup.pending = None;
                    cx.notify();
                },
            )),
        );
    }
    if let Some(message) = &this.backup.message {
        view = view.child(
            div()
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child(message.clone()),
        );
    }
    view.into_any_element()
}
