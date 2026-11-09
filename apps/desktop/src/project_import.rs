//! One import sheet, backed by existing local discovery and gh credentials.
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{
        Disableable, Selectable, Sizable,
        button::{Button, ButtonVariants},
        input::{InputEvent, InputState},
        menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    },
    *,
};
use dunlin_workspace::{
    github::imports as github,
    gitlab,
    repositories::{self, Discovery},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Local,
    GitHub,
    GitLab,
}
impl Mode {
    fn parse(self, value: &str) -> Result<github::Source, String> {
        if self == Self::GitLab {
            gitlab::parse(value)
        } else {
            github::Source::parse(value)
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Local,
    Destination,
}
struct Inputs {
    query: Entity<InputState>,
    destination: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
enum Preview {
    Local {
        root: PathBuf,
        discovery: Discovery,
    },
    Hosted {
        mode: Mode,
        source: github::Source,
        repositories: Vec<String>,
    },
}
#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    pub busy: bool,
    mode: Mode,
    local: Option<PathBuf>,
    search: Option<Arc<github::PreviewRequest>>,
    inputs: Option<Inputs>,
    preview: Option<Preview>,
    watch: bool,
    error: Option<String>,
    progress: Option<String>,
    group: Option<String>,
    request: u64,
    pub cli: Option<github::CliStatus>,
    pub gitlab_cli: Option<github::CliStatus>,
    pub checking_cli: bool,
    last_scan: Option<Instant>,
    pub watch_error: Option<String>,
}
impl State {
    pub fn focus(&self, cx: &App) -> Option<FocusHandle> {
        let inputs = self.inputs.as_ref()?;
        Some(inputs.query.read(cx).focus_handle(cx))
    }
    pub fn dismiss(&mut self) {
        if !self.busy || self.search.is_some() {
            if let Some(search) = self.search.take() {
                search.cancel();
            }
            self.busy = false;
            self.progress = None;
            self.open = false;
            self.request = self.request.wrapping_add(1);
            self.inputs = None;
            self.local = None;
            self.preview = None;
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
fn absolute_path(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let path = if value == "~" {
        home()
    } else if let Some(rest) = value.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("Choose an absolute folder path (or ~/…)".into());
    }
    Ok(path)
}
fn can_watch(root: &Path, discovery: &Discovery) -> bool {
    discovery.projects.len() > 1
        && discovery
            .projects
            .iter()
            .all(|repo| repo.collection.as_deref() == Some(root))
}

fn only_new_repositories(discovery: &mut Discovery, existing: &std::collections::HashSet<PathBuf>) {
    discovery
        .projects
        .retain(|repo| !existing.contains(&repo.path) && repo.path.join(".git").exists());
}

pub(crate) fn source_menu(menu: PopupMenu, app: WeakEntity<DunlinApp>) -> PopupMenu {
    let local = app.clone();
    menu.item(
        PopupMenuItem::new("Add from local drive")
            .icon(dunlin_ui::assets::IconName::FolderOpen)
            .on_click(move |_, _, cx| {
                let _ = local.update(cx, |this, cx| this.open_folder(cx));
            }),
    )
    .item(
        PopupMenuItem::new("Add from remote repo")
            .icon(dunlin_ui::assets::IconName::GitFork)
            .on_click(move |_, _, cx| {
                let _ = app.update(cx, |this, cx| this.open_project_import(cx));
            }),
    )
}

impl DunlinApp {
    pub(crate) fn open_project_import(&mut self, cx: &mut Context<Self>) {
        if self.factory_resetting
            || self.project_import.busy
            || self.local_import_running
            || self.backup.busy
            || self.backup.cloning
        {
            self.file_notice("Wait for the current import to finish", cx);
            return;
        }
        if self.navigation_blocked() && !self.project_import.open {
            return;
        }
        self.settings_open = false;
        self.command_open = false;
        self.context_menu = None;
        self.remote_picker = None;
        self.project_menu_open = false;
        self.chat_menu_open = false;
        self.branch_ui.dismiss();
        self.project_import.dismiss();
        self.project_import.open = true;
        self.project_import.mode = Mode::GitHub;
        self.project_import.error = None;
        self.project_import.watch = false;
        self.project_import.group = None;
        self.refresh_hosting_cli(cx);
        cx.notify();
    }

    pub(crate) fn refresh_hosting_cli(&mut self, cx: &mut Context<Self>) {
        if self.project_import.checking_cli {
            return;
        }
        self.project_import.checking_cli = true;
        cx.spawn(async |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async { (github::status(), gitlab::status()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.project_import.cli = Some(status.0);
                this.project_import.gitlab_cli = Some(status.1);
                this.project_import.checking_cli = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_hosting_accounts(&mut self, cx: &mut Context<Self>) {
        self.refresh_hosting_cli(cx);
        self.user_profile.checked = None;
        self.refresh_user_profile(cx);
    }

    fn choose_import_folder(
        &mut self,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_import.busy {
            return;
        }
        let request = self.project_import.request;
        let mode = self.project_import.mode;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                if purpose == Purpose::Local {
                    "Choose project folder"
                } else {
                    "Choose download folder"
                }
                .into(),
            ),
        });
        let paths = self.overlay_backdrop.track_native(paths, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = paths.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.project_import.open
                    || this.project_import.request != request
                    || this.project_import.mode != mode
                {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.select_import_folder(path, purpose, window, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        this.project_import.error = Some(error.to_string());
                        cx.notify();
                    }
                    _ => {}
                }
            });
        })
        .detach();
    }

    fn select_import_folder(
        &mut self,
        path: PathBuf,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_import.busy {
            return;
        }
        self.project_import.error = None;
        if purpose == Purpose::Local {
            self.project_import.local = Some(path);
            self.project_import.watch = false;
            self.preview_project_import(cx);
        } else if let Some(inputs) = &self.project_import.inputs {
            inputs.destination.update(cx, |state, cx| {
                state.set_value(path.to_string_lossy().into_owned(), window, cx)
            });
        }
        cx.notify();
    }

    fn preview_project_import(&mut self, cx: &mut Context<Self>) {
        if self.project_import.busy || self.local_import_running {
            return;
        }
        let mode = self.project_import.mode;
        let local = self.project_import.local.clone();
        let source = if mode != Mode::Local {
            let Some(inputs) = &self.project_import.inputs else {
                return;
            };
            match mode.parse(&inputs.query.read(cx).value()) {
                Ok(source) => Some(source),
                Err(error) => {
                    self.project_import.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        if mode == Mode::Local && local.is_none() {
            return;
        }
        self.project_import.busy = true;
        self.project_import.error = None;
        self.project_import.preview = None;
        self.project_import.progress = Some("Looking for repositories…".into());
        self.project_import.request = self.project_import.request.wrapping_add(1);
        let request = self.project_import.request;
        let requested_source = source.clone();
        let search = Arc::new(github::PreviewRequest::default());
        self.project_import.search = Some(search.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(source) = source {
                        (if mode == Mode::GitLab { gitlab::preview_cancellable(&source, &search) } else { github::preview_cancellable(&source, &search) }).map(|repositories| Preview::Hosted {
                            mode,
                            source,
                            repositories,
                        })
                    } else {
                        let root = local.unwrap();
                        let discovery = repositories::discover(std::slice::from_ref(&root));
                        Ok(Preview::Local { root, discovery })
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.project_import.open || this.project_import.request != request {
                    return;
                }
                this.project_import.busy = false;
                this.project_import.search = None;
                this.project_import.progress = None;
                if let Some(source) = requested_source.as_ref() {
                    let current = this.project_import.inputs.as_ref().and_then(|inputs| mode.parse(&inputs.query.read(cx).value()).ok());
                    if this.project_import.mode != mode || current.as_ref() != Some(source) {
                        this.project_import.preview = None;
                        cx.notify();
                        return;
                    }
                }
                match result {
                    Ok(preview) => {
                        this.project_import.watch = matches!(&preview, Preview::Local { root, discovery }
                            if can_watch(root, discovery) && this.watched_project_folders.contains(root));
                        this.project_import.preview = Some(preview);
                    }
                    Err(error) => this.project_import.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_project_import(&mut self, cx: &mut Context<Self>) {
        if self.project_import.busy || self.local_import_running {
            return;
        }
        let Some(preview) = self.project_import.preview.take() else {
            return;
        };
        match preview {
            Preview::Local { root, discovery } => {
                if discovery.projects.is_empty() {
                    self.project_import.preview = Some(Preview::Local { root, discovery });
                    return;
                }
                if can_watch(&root, &discovery) {
                    if self.project_import.watch {
                        if !self.watched_project_folders.contains(&root) {
                            self.watched_project_folders.push(root.clone());
                        }
                    } else {
                        self.watched_project_folders.retain(|path| path != &root);
                    }
                    self.persist_settings(cx);
                }
                self.project_import.dismiss();
                // Rescan on confirmation: a preview never grants stale paths access.
                self.import_local_folders(vec![root], cx);
            }
            Preview::Hosted {
                mode,
                source,
                repositories,
            } => {
                let Some(inputs) = &self.project_import.inputs else {
                    return;
                };
                if mode != self.project_import.mode
                    || mode.parse(&inputs.query.read(cx).value()).ok().as_ref() != Some(&source)
                {
                    self.project_import.error = Some("The query changed. Preview again.".into());
                    cx.notify();
                    return;
                }
                let destination = match absolute_path(&inputs.destination.read(cx).value()) {
                    Ok(path) => path,
                    Err(error) => {
                        self.project_import.error = Some(error);
                        self.project_import.preview = Some(Preview::Hosted {
                            mode,
                            source,
                            repositories,
                        });
                        cx.notify();
                        return;
                    }
                };
                if repositories.is_empty() {
                    return;
                }
                if self
                    .project_import
                    .group
                    .as_ref()
                    .is_some_and(|id| !self.project_groups.iter().any(|g| &g.id == id))
                {
                    self.project_import.error =
                        Some("The selected group no longer exists. Select another group.".into());
                    self.project_import.preview = Some(Preview::Hosted {
                        mode,
                        source,
                        repositories,
                    });
                    cx.notify();
                    return;
                }
                self.project_import.busy = true;
                self.project_import.error = None;
                let group = self.project_import.group.clone();
                let count = repositories.len();
                cx.spawn(async move |this, cx| {
                    let mut discovery = Discovery::default();
                    for (index, name) in repositories.into_iter().enumerate() {
                        if this
                            .update(cx, |this, cx| {
                                this.project_import.progress =
                                    Some(format!("Cloning {}/{} · {name}", index + 1, count));
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                        let destination = destination.clone();
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                // Inspect individually: GitHub batches can exceed the local 256-root scan limit.
                                (if mode == Mode::GitLab {
                                    gitlab::clone_repository(&name, &destination)
                                } else {
                                    github::clone_repository(&name, &destination)
                                })
                                .map(|path| repositories::discover(&[path]))
                            })
                            .await;
                        match result {
                            Ok(found) => {
                                discovery.projects.extend(found.projects);
                                discovery.warnings.extend(found.warnings);
                                discovery.limited |= found.limited;
                            }
                            Err(error) => discovery.warnings.push(error),
                        }
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.project_import.busy = false;
                        this.project_import.progress = None;
                        this.project_import.dismiss();
                        this.finish_project_import(discovery, false, None, false, group, cx);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            }
        }
    }

    pub(crate) fn poll_project_folders(&mut self, cx: &mut Context<Self>) {
        if self.watched_project_folders.is_empty()
            || self.backup.busy
            || self.backup.cloning
            || self.local_import_running
            || self.project_import.busy
            || self.has_modal_overlay()
            || self.factory_resetting
            || self
                .project_import
                .last_scan
                .is_some_and(|time| time.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        self.project_import.last_scan = Some(Instant::now());
        self.local_import_running = true;
        let roots = self.watched_project_folders.clone();
        cx.spawn(async move |this, cx| {
            let mut discovery = cx
                .background_executor()
                .spawn(async move { repositories::discover(&roots) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.local_import_running = false;
                let existing = crate::local_projects::canonical_local_paths(&this.projects);
                only_new_repositories(&mut discovery, &existing);
                // A root removed while scanning must not import another project.
                discovery.projects.retain(|repo| {
                    repo.collection
                        .as_ref()
                        .is_some_and(|root| this.watched_project_folders.contains(root))
                });
                this.project_import.watch_error = if discovery.limited {
                    Some("Folder tracking reached discovery limits; select smaller roots.".into())
                } else {
                    discovery.warnings.first().cloned()
                };
                discovery.warnings.clear();
                if !discovery.projects.is_empty() {
                    this.finish_project_import(discovery, false, None, false, None, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn button(id: &'static str, label: impl Into<SharedString>, enabled: bool) -> Button {
    dunlin_ui::button(id).label(label).disabled(!enabled)
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    if this.project_import.inputs.is_none() {
        let query = cx
            .new(|cx| InputState::new(window, cx).placeholder("Repository URL or provider search"));
        let destination = cx.new(|cx| InputState::new(window, cx).placeholder("Clone destination"));
        let default = crate::app::data_dir()
            .map(|p| p.join("repositories"))
            .unwrap_or_default();
        destination.update(cx, |state, cx| {
            state.set_value(default.to_string_lossy().into_owned(), window, cx)
        });
        let changed = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && this.project_import.mode != Mode::Local {
                this.project_import.preview = None;
                this.project_import.error = None;
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.preview_project_import(cx);
            }
        });
        this.project_import.inputs = Some(Inputs {
            query,
            destination,
            _subscriptions: vec![changed],
        });
    }
    let p = palette(cx);
    let busy = this.project_import.busy;
    let mode = this.project_import.mode;
    let inputs = this.project_import.inputs.as_ref().unwrap();
    let query = inputs.query.clone();
    let destination = inputs.destination.clone();
    let mut content = div().flex().flex_col().gap(px(12.));
    let mut tabs = segment_group(p).self_start();
    for (candidate, label) in [
        (Mode::Local, "Local folder"),
        (Mode::GitHub, "GitHub"),
        (Mode::GitLab, "GitLab"),
    ] {
        tabs = tabs.child(
            dunlin_ui::button(format!("import-source:{label}"))
                .label(label)
                .selected(mode == candidate)
                .toggled(mode == candidate)
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.project_import.mode = candidate;
                    this.project_import.preview = None;
                    this.project_import.error = None;
                    this.project_import.watch = false;
                    this.project_import.request = this.project_import.request.wrapping_add(1);
                    if candidate == Mode::Local {
                        this.choose_import_folder(Purpose::Local, window, cx);
                    } else {
                        if let Some(inputs) = &this.project_import.inputs {
                            inputs.query.update(cx, |state, cx| state.focus(window, cx));
                        }
                    }
                    cx.notify();
                })),
        );
    }
    content = content.child(tabs);
    if mode != Mode::Local {
        let status = (if mode == Mode::GitLab {
            &this.project_import.gitlab_cli
        } else {
            &this.project_import.cli
        })
        .as_ref()
        .map(|s| s.message.clone())
        .unwrap_or_else(|| "Checking hosting CLI…".into());
        content = content
            .child(div().flex().items_center().gap(px(8.)).child(div().flex_1().text_size(px(11.)).text_color(p.muted_fg).child(status))
                .child(button("import-refresh-gh", "Check again", !this.project_import.checking_cli).on_click(cx.listener(|this, _, _, cx| this.refresh_hosting_accounts(cx)))))
            .child(div().rounded(px(dunlin_ui::radii::PANEL)).border_1().border_color(p.border).bg(p.card).p(px(12.)).flex().flex_col().gap(px(8.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("1. Find repositories"))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child(if mode == Mode::GitLab { "GitLab repository URL or search text" } else { "GitHub repository URL or native search query" }))
                .child(div().flex().flex_wrap().items_center().gap(px(8.))
                    .child(div().flex_1().min_w(px(180.)).child(search_input(&query, "Find repositories").with_size(dunlin_ui::component::Size::Medium).disabled(busy)))
                    .child(button("import-search-preview", if busy { "Searching…" } else { "Search repositories" }, !busy && !query.read(cx).value().trim().is_empty())
                        .relative().children({
                            #[cfg(debug_assertions)]
                            { crate::workspace_ui_smoke::probe("import-search-action") }
                            #[cfg(not(debug_assertions))]
                            { None::<AnyElement> }
                        })
                        .primary().on_click(cx.listener(|this, _, _, cx| this.preview_project_import(cx)))))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Click Search repositories or press Enter. Nothing is downloaded yet."))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child(if mode == Mode::GitLab {
                    "Search includes projects you are a member of, using your glab account."
                } else { "Example: org:example in:name media archived:false · uses your active gh account." })))
            .child(div().rounded(px(dunlin_ui::radii::PANEL)).border_1().border_color(p.border).bg(p.card).p(px(12.)).flex().flex_col().gap(px(8.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("2. Choose a destination"))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Repositories are downloaded into <owner>/<repo> subfolders. Existing folders are never overwritten."))
                .child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w(px(0.)).child(dunlin_ui::input(&destination).aria_label("Repository download folder").disabled(busy)))
                 .child(button("import-browse-destination", "Browse…", !busy).on_click(cx.listener(|this, _, window, cx| {
                    this.choose_import_folder(Purpose::Destination, window, cx);
                }))))
            .child({
                let app = cx.entity().downgrade();
                dunlin_ui::button("import-target-group").label(format!("Add to group: {}", this.project_import.group.as_deref()
                    .map(|id| crate::project_tree::group_path(&this.project_groups, id)).unwrap_or_else(|| "Top level".into())))
                    .dropdown_caret(true).disabled(busy)
                    .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
                        let Some(entity) = app.upgrade() else { return menu; };
                        let this = entity.read(cx);
                        for id in std::iter::once(None).chain(this.project_groups.iter().map(|g| Some(g.id.clone()))) {
                            let label = id.as_deref().map(|id| crate::project_tree::group_path(&this.project_groups, id)).unwrap_or_else(|| "Top level".into());
                            let app = app.clone();
                            menu = menu.item(PopupMenuItem::new(label).checked(this.project_import.group == id).on_click(move |_, _, cx| {
                                let _ = app.update(cx, |this, cx| { this.project_import.group = id.clone(); cx.notify(); });
                            }));
                        }
                        menu.scrollable(true).max_h(px(280.))
                    })
            }));
    }
    if mode == Mode::Local {
        content = content.child(
            button("import-choose-folder", "Choose folder…", !busy).on_click(cx.listener(
                |this, _, window, cx| this.choose_import_folder(Purpose::Local, window, cx),
            )),
        );
    }
    let mut ready = false;
    if let Some(preview) = &this.project_import.preview {
        let (names, summary, warning) = match preview {
            Preview::Local { root, discovery } => {
                let count = discovery
                    .projects
                    .iter()
                    .filter(|repo| repo.path.join(".git").exists())
                    .count();
                ready = !discovery.projects.is_empty();
                let summary = if count == 0 && ready {
                    "No Git repositories · add as a local project".into()
                } else {
                    format!(
                        "{count} repositories found · {} projects to import",
                        discovery.projects.len()
                    )
                };
                let warning = if discovery.limited {
                    Some("Partial scan: discovery limits reached. Select a smaller folder for a complete count.".into())
                } else {
                    discovery.warnings.first().cloned()
                };
                if can_watch(root, discovery) {
                    let watch = this.project_import.watch;
                    content = content.child(dunlin_ui::button("import-watch-root").label(if watch { "☑ Track this root and automatically import new repositories" } else { "☐ Track this root and automatically import new repositories" }).selected(watch).toggled(watch).disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| { this.project_import.watch = !this.project_import.watch; cx.notify(); })))
                        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Checked every 30 seconds while Dunlin runs. New projects require trust; stop tracking in Settings → Workspace."));
                }
                (
                    discovery
                        .projects
                        .iter()
                        .map(|repo| repo.path.display().to_string())
                        .collect::<Vec<_>>(),
                    summary,
                    warning,
                )
            }
            Preview::Hosted { repositories, .. } => {
                ready = !repositories.is_empty();
                (
                    repositories.clone(),
                    format!(
                        "{} repositories will be downloaded and imported",
                        repositories.len()
                    ),
                    None,
                )
            }
        };
        let mut list = div()
            .id("import-preview-list")
            .max_h(px(110.))
            .overflow_y_scroll()
            .text_size(px(11.))
            .text_color(p.fg);
        for name in names {
            list = list.child(div().child(name));
        }
        content = content.child(
            div()
                .rounded(px(dunlin_ui::radii::PANEL))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if mode == Mode::Local {
                            "Review projects"
                        } else {
                            "3. Review repositories"
                        }),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(summary),
                )
                .child(list),
        );
        if let Some(warning) = warning {
            content = content.child(div().text_color(p.amber).child(warning));
        }
    }
    if let Some(error) = &this.project_import.error {
        content = content.child(
            div()
                .rounded(px(dunlin_ui::radii::PANEL))
                .bg(p.red_soft)
                .border_1()
                .border_color(p.red.opacity(0.3))
                .p(px(10.))
                .text_size(px(11.))
                .text_color(p.red)
                .child(error.clone()),
        );
    }
    if let Some(progress) = &this.project_import.progress {
        content = content.child(
            div()
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child(progress.clone()),
        );
    }
    let width = (f32::from(window.viewport_size().width) - 48.).clamp(320., 760.);
    let height = (f32::from(window.viewport_size().height) - 64.).clamp(300., 700.);
    div()
        .id("project-import-overlay")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(cx.listener(|this, _, _, cx| {
            this.project_import.dismiss();
            cx.notify();
        }))
        .child(
            div()
                .id("project-import-sheet")
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("project-import-sheet")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .w(px(width))
                .h(px(height))
                .rounded(px(SurfaceGeometry::DIALOG.radius))
                .border(px(SurfaceGeometry::DIALOG.border))
                .border_color(p.border_strong)
                .bg(p.panel)
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(crate::effects::panel_probe(&this.overlay_backdrop, SurfaceGeometry::DIALOG, false))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_shrink_0()
                        .px(px(20.))
                        .py(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Add projects"),
                )
                .child(
                    div()
                        .id("project-import-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .px(px(20.))
                        .pb(px(14.))
                        .child(content),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px(px(20.))
                        .py(px(12.))
                        .border_t_1()
                        .border_color(p.border)
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            button(
                                "import-close",
                                "Cancel",
                                !busy || this.project_import.search.is_some(),
                            )
                            .relative()
                            .children({
                                #[cfg(debug_assertions)]
                                {
                                    crate::workspace_ui_smoke::probe("import-cancel")
                                }
                                #[cfg(not(debug_assertions))]
                                {
                                    None::<AnyElement>
                                }
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_import.dismiss();
                                cx.notify();
                            })),
                        )
                        .child(
                            button(
                                "import-confirm",
                                if mode != Mode::Local {
                                    if ready {
                                        "Download and import"
                                    } else {
                                        "Search repositories first"
                                    }
                                } else {
                                    "Import projects"
                                },
                                ready && !busy && !this.local_import_running,
                            )
                            .primary()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.confirm_project_import(cx)),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

pub(crate) fn settings(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    if this.project_import.cli.is_none() && !this.project_import.checking_cli {
        this.refresh_hosting_cli(cx);
    }
    let p = palette(cx);
    let status = this.project_import.cli.as_ref();
    let message = status
        .map(|s| s.message.clone())
        .unwrap_or_else(|| "Checking GitHub CLI…".into());
    let color = if status.is_some_and(|s| s.authenticated) {
        p.green
    } else if status.is_some_and(|s| s.installed) {
        p.amber
    } else {
        p.muted_fg
    };
    let mut card = dunlin_ui::settings::card(p).overflow_hidden()
        .child(div().px(px(14.)).py(px(12.)).flex().items_center().gap(px(10.))
            .child(div().flex_1().flex().flex_col().gap(px(4.))
                .child(div().font_weight(FontWeight::MEDIUM).child("GitHub CLI"))
                .child(div().text_size(px(11.)).text_color(color).child(message))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Authenticate outside Dunlin: gh auth login --hostname github.com. Dunlin never reads or stores your token.")))
            .child(button("settings-refresh-gh", if this.project_import.checking_cli { "Checking…" } else { "Check again" }, !this.project_import.checking_cli)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_hosting_accounts(cx)))));
    let gitlab_status = this.project_import.gitlab_cli.as_ref();
    card = card.child(div().px(px(14.)).py(px(12.)).border_t_1().border_color(p.border)
        .flex().items_center().gap(px(10.))
        .child(div().flex_1().min_w(px(0.)).flex().flex_col().gap(px(4.))
            .child(div().font_weight(FontWeight::MEDIUM).child("GitLab CLI"))
            .child(div().text_size(px(11.)).text_color(if gitlab_status.is_some_and(|s| s.authenticated) { p.green } else { p.muted_fg })
                .child(gitlab_status.map(|s| s.message.clone()).unwrap_or_else(|| "Checking GitLab CLI…".into())))
            .child(div().text_size(px(11.)).text_color(p.muted_fg)
                .child("Authenticate outside Dunlin: glab auth login --hostname gitlab.com. Tokens stay in glab.")))
        .child(button("settings-refresh-glab", "Check again", !this.project_import.checking_cli)
            .on_click(cx.listener(|this, _, _, cx| this.refresh_hosting_accounts(cx)))));
    for (index, root) in this.watched_project_folders.clone().into_iter().enumerate() {
        card = card.child(
            div()
                .px(px(14.))
                .py(px(10.))
                .border_t_1()
                .border_color(p.border)
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.))
                        .child(format!("Tracking · {}", root.display())),
                )
                .child(
                    dunlin_ui::button(format!("stop-tracking-root:{index}"))
                        .small()
                        .label("Stop tracking")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.watched_project_folders.retain(|path| path != &root);
                            this.persist_settings(cx);
                            cx.notify();
                        })),
                ),
        );
    }
    if let Some(error) = &this.project_import.watch_error {
        card = card.child(
            div()
                .p(px(12.))
                .text_size(px(11.))
                .text_color(p.amber)
                .child(error.clone()),
        );
    }
    div().flex().flex_col().gap(px(10.))
        .child(dunlin_ui::settings::section("Project discovery and Git hosting · this Mac", p))
        .child(card)
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Local folder tracking adds new Git repositories every 30 seconds while Dunlin runs. It never deletes projects or updates repository contents."))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dismiss_cancels_search_and_invalidates_results_but_never_interrupts_cloning() {
        let mut state = State {
            open: true,
            busy: true,
            request: 7,
            search: Some(Arc::new(github::PreviewRequest::default())),
            ..Default::default()
        };
        state.dismiss();
        assert!(!state.open && !state.busy && state.search.is_none());
        assert_eq!(state.request, 8);
        let mut clone = State {
            open: true,
            busy: true,
            request: 7,
            ..Default::default()
        };
        clone.dismiss();
        assert!(clone.open && clone.busy);
        assert_eq!(clone.request, 7);
    }
    #[test]
    fn watch_is_only_offered_for_a_collection_and_only_new_git_roots_are_auto_imported() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let one = root.join("one");
        let two = root.join("two");
        for path in [&one, &two] {
            assert!(
                std::process::Command::new("git")
                    .args(["init", "-q", "--template="])
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let single = repositories::discover(std::slice::from_ref(&one));
        assert!(!can_watch(&one, &single));
        let mut collection = repositories::discover(std::slice::from_ref(&root));
        assert!(can_watch(&root, &collection));
        only_new_repositories(
            &mut collection,
            &std::collections::HashSet::from([one.clone()]),
        );
        assert_eq!(collection.projects.len(), 1);
        assert_eq!(collection.projects[0].path, two);
        let ordinary = tempfile::tempdir().unwrap();
        let mut discovery = repositories::discover(&[ordinary.path().into()]);
        only_new_repositories(&mut discovery, &Default::default());
        assert!(discovery.projects.is_empty());
    }
    #[test]
    fn watches_default_to_empty_and_survive_settings_roundtrip() {
        let mut settings: crate::app::StoredSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.watched_project_folders.is_empty());
        settings
            .watched_project_folders
            .push("/work/projects".into());
        let saved: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            saved.watched_project_folders,
            settings.watched_project_folders
        );
        assert!(saved.auto_connect);
    }
    #[test]
    fn destination_requires_an_absolute_path_and_expands_home() {
        assert!(absolute_path("relative").is_err());
        assert_eq!(
            absolute_path("~/projects").unwrap(),
            home().join("projects")
        );
    }
}
