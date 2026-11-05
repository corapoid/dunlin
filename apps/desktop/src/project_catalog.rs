//! Cached, read-only repository metadata and project-list filters.
use crate::{app::DunlinApp, remote::WorkspaceBackend};
use dunlin_domain::Project;
use dunlin_ui::{component::tooltip::Tooltip, prelude::FluentBuilder as _, *};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub show_language: bool,
    pub show_topics: bool,
    pub wrap_names: bool,
    pub show_preview: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_language: true,
            show_topics: true,
            wrap_names: false,
            show_preview: true,
        }
    }
}
#[derive(Default)]
struct Details {
    github: Option<dunlin_workspace::github::Metadata>,
    readme: Option<String>,
    github_error: Option<String>,
    size: Option<u64>,
}
struct Entry {
    key: String,
    checked: Instant,
    topics: bool,
    preview: bool,
    details: Details,
}
#[derive(Default)]
pub(crate) struct State {
    entries: HashMap<String, Entry>,
    busy: bool,
    pub language: Option<String>,
    pub topic: Option<String>,
}

fn key(app: &DunlinApp, project: &Project) -> String {
    format!(
        "{}|{}|{}|{}",
        project.path,
        project.trusted,
        serde_json::to_string(&project.remote).unwrap_or_default(),
        project
            .remote
            .as_ref()
            .and_then(|p| app.remote_host(&p.host_id))
            .map(|h| format!("{}|{}", h.url, h.device_id))
            .unwrap_or_default()
    )
}
fn details<'a>(app: &'a DunlinApp, project: &Project) -> Option<&'a Details> {
    app.project_catalog
        .entries
        .get(&project.id)
        .filter(|e| e.key == key(app, project))
        .map(|e| &e.details)
}
pub(crate) fn primary_language(app: &DunlinApp, project: &Project) -> Option<String> {
    if let Some(languages) = app
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|r| r.path == project.path)
        .and_then(|r| r.languages.as_ref())
    {
        return languages.first().map(|l| l.name.to_string());
    }
    details(app, project)
        .and_then(|d| d.github.as_ref())
        .and_then(|g| g.language.clone())
}
pub(crate) fn topics(app: &DunlinApp, project: &Project) -> Vec<String> {
    details(app, project)
        .and_then(|d| d.github.as_ref())
        .map(|g| g.topics.clone())
        .unwrap_or_default()
}
pub(crate) fn matches(app: &DunlinApp, project: &Project) -> bool {
    (!app.project_list.show_language
        || app
            .project_catalog
            .language
            .as_ref()
            .is_none_or(|s| primary_language(app, project).as_ref() == Some(s)))
        && (!app.project_list.show_topics
            || app
                .project_catalog
                .topic
                .as_ref()
                .is_none_or(|s| topics(app, project).contains(s)))
}
pub(crate) fn languages(app: &DunlinApp) -> Vec<String> {
    app.projects
        .iter()
        .filter(|p| !p.is_chat_space())
        .filter_map(|p| primary_language(app, p))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn readme(backend: &WorkspaceBackend) -> Option<String> {
    for name in [
        "README.md",
        "README",
        "readme.md",
        "Readme.md",
        "README.rst",
        "README.txt",
        ".github/README.md",
        "docs/README.md",
    ] {
        if let Ok(file) = backend.read_file(name) {
            // Plain text only: repo markdown must not load remote images or HTML.
            return Some(file.contents.chars().take(32_768).collect());
        }
    }
    None
}
fn load(backend: WorkspaceBackend, topics: bool, preview: bool) -> Details {
    let mut details = Details::default();
    if preview {
        details.readme = readme(&backend);
        details.size = match &backend {
            WorkspaceBackend::Local { root } => dunlin_workspace::tree::storage_size(std::path::Path::new(root)),
            WorkspaceBackend::Remote(remote) => remote.storage_size().ok().flatten(),
        };
    }
    if topics {
        let result = match &backend {
            WorkspaceBackend::Local { root } => dunlin_workspace::github::metadata(root),
            WorkspaceBackend::Remote(remote) => remote.github_metadata(),
        };
        match result {
            Ok(metadata) => details.github = Some(metadata),
            Err(e) => details.github_error = Some(e),
        }
    }
    details
}

impl DunlinApp {
    pub(crate) fn refresh_project_catalog(&mut self, cx: &mut Context<Self>) {
        if self.project_catalog.busy
            || (!self.project_list.show_language
                && !self.project_list.show_topics
                && !self.project_list.show_preview)
        {
            return;
        }
        let topics = self.project_list.show_topics;
        let preview = self.project_list.show_preview;
        let Some(project) = self
            .projects
            .iter()
            .find(|p| {
                !p.is_chat_space()
                    && p.trusted
                    && (topics || preview || (p.is_remote() && self.project_list.show_language))
                    && self.project_catalog.entries.get(&p.id).is_none_or(|e| {
                        e.key != key(self, p)
                            || e.checked.elapsed() >= Duration::from_secs(600)
                            || ((topics || (p.is_remote() && self.project_list.show_language))
                                && !e.topics)
                            || (preview && !e.preview)
                    })
            })
            .cloned()
        else {
            return;
        };
        let topics = topics || (project.is_remote() && self.project_list.show_language);
        let expected = key(self, &project);
        let backend = if let Some(remote) = &project.remote {
            let Some(host) = self.remote_host(&remote.host_id) else {
                self.project_catalog.entries.insert(
                    project.id,
                    Entry {
                        key: expected,
                        checked: Instant::now(),
                        topics,
                        preview,
                        details: Details {
                            github_error: Some("Remote host unavailable".into()),
                            ..Default::default()
                        },
                    },
                );
                cx.notify();
                return;
            };
            match self.remote_token(&host.id) {
                Ok(token) => WorkspaceBackend::Remote(Box::new(crate::remote::RemoteWorkspace {
                    host,
                    project: remote.clone(),
                    token,
                })),
                Err(_) => {
                    self.project_catalog.entries.insert(
                        project.id,
                        Entry {
                            key: expected,
                            checked: Instant::now(),
                            topics,
                            preview,
                            details: Details {
                                github_error: Some("Remote credentials unavailable".into()),
                                ..Default::default()
                            },
                        },
                    );
                    cx.notify();
                    return;
                }
            }
        } else {
            WorkspaceBackend::Local {
                root: project.path.clone(),
            }
        };
        self.project_catalog.busy = true;
        cx.spawn(async move |this, cx| {
            let details = cx
                .background_executor()
                .spawn(async move { load(backend, topics, preview) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.project_catalog.busy = false;
                if this
                    .projects
                    .iter()
                    .any(|p| p.id == project.id && key(this, p) == expected)
                {
                    this.project_catalog.entries.insert(
                        project.id,
                        Entry {
                            key: expected,
                            checked: Instant::now(),
                            topics,
                            preview,
                            details,
                        },
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }
}

pub(crate) fn preview(app: &DunlinApp, project: &Project, cx: &App) -> AnyElement {
    let p = palette(cx);
    let info = details(app, project);
    let size = info.and_then(|d| d.size);
    let name = info
        .and_then(|d| d.github.as_ref())
        .map(|g| g.full_name.as_str())
        .unwrap_or(&project.name);
    let panel = div()
        .w(px(480.))
        .max_w(px(560.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(12.))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(name.to_string()),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child("Cached repository preview · refreshed up to every 10 minutes"),
        )
        .child(format!(
            "Language: {} · Storage: {}",
            primary_language(app, project).unwrap_or_else(|| "Unknown".into()),
            size.map(|bytes| crate::app::format_bytes(bytes as usize))
                .unwrap_or_else(|| "Unknown".into())
        ))
        .when(app.project_list.show_topics, |view| {
            view.child(format!("Topics: {}", topics(app, project).join(", ")))
        })
        .when_some(info.and_then(|d| d.github_error.as_ref()), |view, error| {
            view.child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(error.clone()),
            )
        })
        .child(
            div()
                .id(format!("project-readme-preview:{}", project.id))
                .max_h(px(320.))
                .overflow_y_scroll()
                .overflow_x_hidden()
                .child(info.and_then(|d| d.readme.clone()).unwrap_or_else(|| {
                    if project.trusted {
                        "README unavailable or metadata still loading.".into()
                    } else {
                        "Trust this project to load its README and metadata.".into()
                    }
                })),
        );
    #[cfg(debug_assertions)]
    let panel = panel.relative().children(crate::workspace_ui_smoke::probe("project-preview"));
    panel.into_any_element()
}

pub(crate) fn attach_preview(
    row: Stateful<Div>,
    app: &DunlinApp,
    project: &Project,
    cx: &Context<DunlinApp>,
) -> AnyElement {
    if !app.project_list.show_preview {
        return row.into_any_element();
    }
    let owner = cx.entity().downgrade();
    let project = project.id.clone();
    row.hoverable_tooltip(move |window, cx| {
        let owner = owner.clone();
        let id = project.clone();
        Tooltip::element(move |_, cx| {
            let Some(owner) = owner.upgrade() else {
                return div().into_any_element();
            };
            let app = owner.read(cx);
            app.projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| preview(app, p, cx))
                .unwrap_or_else(|| div().into_any_element())
        })
        .build(window, cx)
    })
    .into_any_element()
}

#[cfg(debug_assertions)]
pub(crate) fn smoke_filters(app: &mut DunlinApp) {
    let preferences = app.project_list.clone();
    let state = std::mem::take(&mut app.project_catalog);
    let first = app.project().clone();
    let mut second = first.clone();
    second.id = "catalog-filter-fixture".into();
    second.path.push_str("/catalog-filter-fixture");
    app.projects.push(second.clone());
    for (project, language, topic) in [
        (&first, "Rust", "aws-lambda"),
        (&second, "TypeScript", "kubernetes"),
    ] {
        app.project_catalog.entries.insert(
            project.id.clone(),
            Entry {
                key: key(app, project),
                checked: Instant::now(),
                topics: true,
                preview: true,
                details: Details {
                    github: Some(dunlin_workspace::github::Metadata {
                        full_name: format!("fixture/{}", project.id),
                        language: Some(language.into()),
                        topics: vec![topic.into()],
                    }),
                    ..Default::default()
                },
            },
        );
    }
    app.project_list.show_language = true;
    app.project_list.show_topics = true;
    assert!(languages(app).contains(&"TypeScript".into()));
    app.project_catalog.language = Some("TypeScript".into());
    assert!(matches(app, &second));
    assert!(!matches(app, &first));
    app.project_catalog.topic = Some("aws-lambda".into());
    assert!(!matches(app, &second));
    app.project_catalog.language = None;
    assert!(matches(app, &first));
    assert!(!matches(app, &second));
    app.project_list.show_topics = false;
    assert!(matches(app, &first) && matches(app, &second));
    app.projects.pop();
    app.project_list = preferences;
    app.project_catalog = state;
}

#[cfg(test)]
mod tests {
    #[test]
    fn old_preferences_keep_existing_defaults_and_roundtrip_new_choices() {
        let p: super::Preferences = serde_json::from_str("{}").unwrap();
        assert!(p.show_language && p.show_topics && p.show_preview && !p.wrap_names);
        let p = super::Preferences {
            show_language: false,
            show_topics: false,
            show_preview: false,
            wrap_names: true,
        };
        let saved: super::Preferences =
            serde_json::from_value(serde_json::to_value(p).unwrap()).unwrap();
        assert!(
            !saved.show_language && !saved.show_topics && !saved.show_preview && saved.wrap_names
        );
    }
    #[test]
    fn readme_is_bounded_inert_and_cannot_escape_a_local_project() {
        let root = tempfile::tempdir().unwrap();
        let backend = crate::remote::WorkspaceBackend::Local {
            root: root.path().to_string_lossy().into_owned(),
        };
        std::fs::write(root.path().join("README.md"), "é".repeat(40_000)).unwrap();
        assert_eq!(super::readme(&backend).unwrap().chars().count(), 32_768);
        #[cfg(unix)]
        {
            std::fs::remove_file(root.path().join("README.md")).unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("private"), "must not read").unwrap();
            std::os::unix::fs::symlink(
                outside.path().join("private"),
                root.path().join("README.md"),
            )
            .unwrap();
            assert!(super::readme(&backend).is_none());
        }
    }
}
