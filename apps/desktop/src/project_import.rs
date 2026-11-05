//! One import sheet, backed by existing local discovery and gh credentials.
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{
        Disableable, Selectable, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
    },
    *,
};
use dunlin_workspace::{
    github::imports as github,
    repositories::{self, Discovery},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Local,
    GitHub,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Local,
    Destination,
}
struct Browser {
    path: PathBuf,
    purpose: Purpose,
    folders: Vec<PathBuf>,
    loading: bool,
}
struct Inputs {
    query: Entity<InputState>,
    path: Entity<InputState>,
    destination: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
enum Preview {
    Local {
        root: PathBuf,
        discovery: Discovery,
    },
    GitHub {
        source: github::Source,
        repositories: Vec<String>,
    },
}
#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    pub busy: bool,
    mode: Mode,
    browser: Option<Browser>,
    inputs: Option<Inputs>,
    preview: Option<Preview>,
    watch: bool,
    error: Option<String>,
    progress: Option<String>,
    request: u64,
    pub cli: Option<github::CliStatus>,
    pub checking_cli: bool,
    last_scan: Option<Instant>,
    pub watch_error: Option<String>,
}
impl State {
    pub fn focus(&self, cx: &App) -> Option<FocusHandle> {
        let inputs = self.inputs.as_ref()?;
        Some(
            if self.browser.is_some() {
                &inputs.path
            } else {
                &inputs.query
            }
            .read(cx)
            .focus_handle(cx),
        )
    }
    pub fn dismiss(&mut self) {
        if !self.busy {
            self.open = false;
            self.request = self.request.wrapping_add(1);
            self.inputs = None;
            self.browser = None;
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
fn folders(path: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("Cannot open folder: {e}"))?;
    if !path.is_dir() {
        return Err("That path is not a folder".into());
    }
    let mut folders = Vec::new();
    for (index, entry) in std::fs::read_dir(&path)
        .map_err(|e| format!("Cannot read folder: {e}"))?
        .enumerate()
    {
        if index >= 20_000 {
            return Err("Too many entries to display. Enter a more specific path.".into());
        }
        let entry = entry.map_err(|e| format!("Cannot read folder entry: {e}"))?;
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            if entry.metadata().is_ok_and(|metadata| metadata.st_flags() & libc::UF_HIDDEN != 0) { continue; }
        }
        if !entry.file_name().to_string_lossy().starts_with('.') && entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir())
        {
            folders.push(entry.path());
        }
    }
    folders.sort_by_cached_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    Ok((path, folders))
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

impl DunlinApp {
    pub(crate) fn open_project_import(&mut self, cx: &mut Context<Self>) {
        if self.factory_resetting || self.project_import.busy || self.local_import_running {
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
        self.project_import.mode = Mode::Local;
        self.project_import.error = None;
        self.project_import.watch = false;
        self.refresh_github_cli(cx);
        cx.notify();
    }

    pub(crate) fn refresh_github_cli(&mut self, cx: &mut Context<Self>) {
        if self.project_import.checking_cli {
            return;
        }
        self.project_import.checking_cli = true;
        cx.spawn(async |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async { github::status() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.project_import.cli = Some(status);
                this.project_import.checking_cli = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn browse_import_folder(
        &mut self,
        path: PathBuf,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_import.busy {
            return;
        }
        self.project_import.request = self.project_import.request.wrapping_add(1);
        let request = self.project_import.request;
        if purpose == Purpose::Local {
            self.project_import.preview = None;
            self.project_import.watch = false;
        }
        self.project_import.error = None;
        self.project_import.browser = Some(Browser {
            path: path.clone(),
            purpose,
            folders: Vec::new(),
            loading: true,
        });
        if let Some(inputs) = &self.project_import.inputs {
            inputs.path.update(cx, |state, cx| {
                state.set_value(path.to_string_lossy().into_owned(), window, cx)
            });
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { folders(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.project_import.open || this.project_import.request != request {
                    return;
                }
                if let Some(browser) = &mut this.project_import.browser {
                    browser.loading = false;
                    match result {
                        Ok((path, folders)) => {
                            browser.path = path;
                            browser.folders = folders;
                        }
                        Err(error) => {
                            this.project_import.error = Some(error);
                            this.project_import.browser = None;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go_import_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = &self.project_import.inputs else {
            return;
        };
        let purpose = self
            .project_import
            .browser
            .as_ref()
            .map(|b| b.purpose)
            .unwrap_or(if self.project_import.mode == Mode::Local {
                Purpose::Local
            } else {
                Purpose::Destination
            });
        match absolute_path(&inputs.path.read(cx).value()) {
            Ok(path) => self.browse_import_folder(path, purpose, window, cx),
            Err(error) => {
                self.project_import.error = Some(error);
                cx.notify();
            }
        }
    }

    fn preview_project_import(&mut self, cx: &mut Context<Self>) {
        if self.project_import.busy || self.local_import_running {
            return;
        }
        let mode = self.project_import.mode;
        let local = self
            .project_import
            .browser
            .as_ref()
            .filter(|b| !b.loading && b.purpose == Purpose::Local)
            .map(|b| b.path.clone());
        let source = if mode == Mode::GitHub {
            let Some(inputs) = &self.project_import.inputs else {
                return;
            };
            match github::Source::parse(&inputs.query.read(cx).value()) {
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
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(source) = source {
                        github::preview(&source).map(|repositories| Preview::GitHub {
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
                if this.project_import.request != request {
                    return;
                }
                this.project_import.busy = false;
                this.project_import.progress = None;
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
            Preview::GitHub {
                source,
                repositories,
            } => {
                let Some(inputs) = &self.project_import.inputs else {
                    return;
                };
                if github::Source::parse(&inputs.query.read(cx).value())
                    .ok()
                    .as_ref()
                    != Some(&source)
                {
                    self.project_import.error = Some("The query changed. Preview again.".into());
                    cx.notify();
                    return;
                }
                let destination = match absolute_path(&inputs.destination.read(cx).value()) {
                    Ok(path) => path,
                    Err(error) => {
                        self.project_import.error = Some(error);
                        self.project_import.preview = Some(Preview::GitHub {
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
                self.project_import.busy = true;
                self.project_import.error = None;
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
                                github::clone_repository(&name, &destination)
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
                        this.finish_project_import(discovery, false, None, false, cx);
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
                    this.finish_project_import(discovery, false, None, false, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn button(id: &'static str, label: impl Into<SharedString>, enabled: bool) -> Button {
    Button::new(id).small().label(label).disabled(!enabled)
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    if this.project_import.inputs.is_none() {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/owner/repo or org:example archived:false")
        });
        let path =
            cx.new(|cx| InputState::new(window, cx).placeholder("Absolute folder path or ~/…"));
        let destination = cx.new(|cx| InputState::new(window, cx).placeholder("Clone destination"));
        let default = crate::app::data_dir()
            .map(|p| p.join("repositories"))
            .unwrap_or_default();
        destination.update(cx, |state, cx| {
            state.set_value(default.to_string_lossy().into_owned(), window, cx)
        });
        let changed = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && this.project_import.mode == Mode::GitHub {
                this.project_import.preview = None;
                this.project_import.error = None;
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.preview_project_import(cx);
            }
        });
        let go = cx.subscribe_in(&path, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.go_import_folder(window, cx);
            }
        });
        this.project_import.inputs = Some(Inputs {
            query,
            path,
            destination,
            _subscriptions: vec![changed, go],
        });
        this.browse_import_folder(home(), Purpose::Local, window, cx);
    }
    let p = palette(cx);
    let busy = this.project_import.busy;
    let mode = this.project_import.mode;
    let inputs = this.project_import.inputs.as_ref().unwrap();
    let query = inputs.query.clone();
    let path_input = inputs.path.clone();
    let destination = inputs.destination.clone();
    let mut content = div().flex().flex_col().gap(px(12.));
    let mut tabs = div().flex().gap(px(8.));
    for (candidate, label) in [
        (Mode::Local, "Local folder"),
        (Mode::GitHub, "GitHub URL or filter"),
    ] {
        tabs = tabs.child(
            Button::new(format!("import-source:{label}"))
                .small()
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
                        this.browse_import_folder(home(), Purpose::Local, window, cx);
                    } else {
                        this.project_import.browser = None;
                        if let Some(inputs) = &this.project_import.inputs {
                            inputs.query.update(cx, |state, cx| state.focus(window, cx));
                        }
                    }
                    cx.notify();
                })),
        );
    }
    content = content.child(tabs);
    if mode == Mode::GitHub {
        let status = this
            .project_import
            .cli
            .as_ref()
            .map(|s| s.message.clone())
            .unwrap_or_else(|| "Checking GitHub CLI…".into());
        content = content
            .child(div().flex().items_center().gap(px(8.)).child(div().flex_1().text_size(px(11.)).text_color(p.muted_fg).child(status))
                .child(button("import-refresh-gh", "Check again", !this.project_import.checking_cli).on_click(cx.listener(|this, _, _, cx| this.refresh_github_cli(cx)))))
            .child(Input::new(&query).disabled(busy))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Native GitHub repository search, e.g. org:example archived:false language:Rust. Only repositories accessible to your gh account are included."))
            .child(button("import-search-preview", "Preview repositories", !busy && this.project_import.browser.is_none()).on_click(cx.listener(|this, _, _, cx| this.preview_project_import(cx))))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Download into · repositories use <owner>/<repo> subfolders"))
            .child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w(px(0.)).child(Input::new(&destination).disabled(busy)))
                .child(button("import-browse-destination", "Browse…", !busy).on_click(cx.listener(|this, _, window, cx| {
                    let value = this.project_import.inputs.as_ref().unwrap().destination.read(cx).value().to_string();
                    let path = absolute_path(&value).ok().filter(|p| p.is_dir()).unwrap_or_else(home);
                    this.browse_import_folder(path, Purpose::Destination, window, cx);
                }))));
    }
    if let Some(browser) = &this.project_import.browser {
        let purpose = browser.purpose;
        let loading = browser.loading;
        let current = browser.path.clone();
        let up = current.parent().filter(|_| current != home()).map(Path::to_path_buf);
        let mut list = div()
            .id("import-folders")
            .h(px(160.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if loading {
            list = list.child("Reading folders…");
        }
        for (index, folder) in browser.folders.iter().enumerate() {
            let folder = folder.clone();
            let label = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            list = list.child(
                Button::new(format!("import-folder:{index}"))
                    .ghost()
                    .small()
                    .flex_shrink_0()
                    .accessibility_label(label.clone())
                    .icon(dunlin_ui::assets::IconName::ChevronRight)
                    .child(div().flex_1().text_left().child(label))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.browse_import_folder(folder.clone(), purpose, window, cx)
                    })),
            );
        }
        if !loading && browser.folders.is_empty() {
            list = list.child(
                div()
                    .text_color(p.muted_fg)
                    .child("No subfolders. You can select this folder."),
            );
        }
        content = content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button("import-parent", "Up", !busy && up.is_some()).on_click(cx.listener(
                            move |this, _, window, cx| {
                                if let Some(path) = &up {
                                    this.browse_import_folder(path.clone(), purpose, window, cx);
                                }
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(Input::new(&path_input).disabled(busy)),
                    )
                    .child(button("import-go", "Go", !busy).on_click(
                        cx.listener(|this, _, window, cx| this.go_import_folder(window, cx)),
                    )),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(format!("Selected folder: {}", current.display())),
            )
            .child(list)
            .child(if purpose == Purpose::Local {
                button(
                    "import-scan-folder",
                    "Select this folder · count repositories",
                    !busy && !loading,
                )
                .on_click(cx.listener(|this, _, _, cx| this.preview_project_import(cx)))
                .into_any_element()
            } else {
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button(
                            "import-use-destination",
                            "Download into this folder",
                            !busy && !loading,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if let Some(inputs) = &this.project_import.inputs {
                                    inputs.destination.update(cx, |state, cx| {
                                        state.set_value(
                                            current.to_string_lossy().into_owned(),
                                            window,
                                            cx,
                                        )
                                    });
                                }
                                this.project_import.browser = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        button(
                            "import-cancel-destination",
                            "Cancel folder selection",
                            !busy,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.project_import.browser = None;
                            cx.notify();
                        })),
                    )
                    .into_any_element()
            });
    } else if mode == Mode::Local {
        content = content.child(
            div()
                .flex()
                .gap(px(6.))
                .child(div().flex_1().child(Input::new(&path_input)))
                .child(
                    button("import-retry-folder", "Open folder", !busy).on_click(
                        cx.listener(|this, _, window, cx| this.go_import_folder(window, cx)),
                    ),
                ),
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
                    content = content.child(Button::new("import-watch-root").small().label(if watch { "☑ Track this root and automatically import new repositories" } else { "☐ Track this root and automatically import new repositories" }).selected(watch).toggled(watch).disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| { this.project_import.watch = !this.project_import.watch; cx.notify(); })))
                        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Checked every 30 seconds while Dunlin runs. New projects require trust; stop tracking in Settings → General."));
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
            Preview::GitHub { repositories, .. } => {
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
        content = content.child(div().font_weight(FontWeight::SEMIBOLD).child(summary));
        let mut list = div()
            .id("import-preview-list")
            .max_h(px(110.))
            .overflow_y_scroll()
            .text_size(px(11.))
            .text_color(p.muted_fg);
        for name in names {
            list = list.child(div().child(name));
        }
        content = content.child(list);
        if let Some(warning) = warning {
            content = content.child(div().text_color(p.amber).child(warning));
        }
    }
    if let Some(error) = &this.project_import.error {
        content = content.child(
            div()
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
                .w(px(width))
                .max_h(px(height))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border_strong)
                .bg(p.panel)
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(crate::effects::rounded_probe(&this.overlay_backdrop, 12.))
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
                            button("import-close", "Cancel", !busy).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.project_import.dismiss();
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            button(
                                "import-confirm",
                                if mode == Mode::GitHub {
                                    "Download and import"
                                } else {
                                    "Import projects"
                                },
                                ready
                                    && !busy
                                    && !this.local_import_running
                                    && this
                                        .project_import
                                        .browser
                                        .as_ref()
                                        .is_none_or(|b| b.purpose == Purpose::Local),
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
        this.refresh_github_cli(cx);
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
    let mut card = div().rounded(px(8.)).border_1().border_color(p.border).overflow_hidden()
        .child(div().px(px(14.)).py(px(12.)).flex().items_center().gap(px(10.))
            .child(div().flex_1().flex().flex_col().gap(px(4.))
                .child(div().font_weight(FontWeight::MEDIUM).child("GitHub CLI"))
                .child(div().text_size(px(11.)).text_color(color).child(message))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Authenticate outside Dunlin: gh auth login --hostname github.com. Dunlin never reads or stores your token.")))
            .child(button("settings-refresh-gh", if this.project_import.checking_cli { "Checking…" } else { "Check again" }, !this.project_import.checking_cli)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_github_cli(cx)))));
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
                    Button::new(format!("stop-tracking-root:{index}"))
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
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted_fg).child("Project imports"))
        .child(card)
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Local folder tracking adds new Git repositories every 30 seconds while Dunlin runs. It never deletes projects or updates repository contents."))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn explorer_validates_paths_and_lists_only_directories() {
        assert!(absolute_path("relative").is_err());
        assert_eq!(
            absolute_path("~/projects").unwrap(),
            home().join("projects")
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        #[cfg(target_os = "macos")]
        {
            std::fs::create_dir(dir.path().join("finder-hidden")).unwrap();
            assert!(std::process::Command::new("/usr/bin/chflags").arg("hidden").arg(dir.path().join("finder-hidden")).status().unwrap().success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join(".hidden"), dir.path().join("linked-folder")).unwrap();
        std::fs::create_dir(dir.path().join("z")).unwrap();
        std::fs::create_dir(dir.path().join("A")).unwrap();
        std::fs::write(dir.path().join("file"), "not a directory").unwrap();
        let (_, entries) = folders(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file_name().unwrap(), "A");
    }
}
