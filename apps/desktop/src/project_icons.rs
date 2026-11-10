//! One shared project identity for sidebar, switcher and Dashboard. Detection
//! only reads bounded local assets; it never writes repo/settings files or uses AI.
mod discovery;

use crate::app::{DunlinApp, tone_color, tone_soft};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use dunlin_domain::Project;
use dunlin_ui::component::Icon;
use dunlin_ui::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    io::{Cursor, Read},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;
const ICON_SIDE: u32 = 128;
const REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Detected {
    source: PathBuf,
    signature: u64,
    image: Arc<Image>,
}
struct Entry {
    root: String,
    custom: Option<String>,
    checked: Instant,
    icon: Option<Detected>,
}
#[derive(Default)]
pub(crate) struct State {
    color: Option<(String, Entity<ColorPickerState>, Subscription)>,
    entries: HashMap<String, Entry>,
    groups: HashMap<String, (String, Arc<Image>)>,
    busy: bool,
    revision: u64,
}

fn matches_project(project: &Project, id: &str, root: &str) -> bool {
    project.id == id
        && project.path == root
        && (project.settings.custom_icon.is_some() || (project.trusted && !project.is_remote()))
}

pub(crate) fn decode(bytes: &[u8], extension: &str) -> Result<Arc<Image>, String> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err("Project icon exceeds 2 MiB".into());
    }
    let png = if extension.eq_ignore_ascii_case("svg") {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
            return Err("SVG entities are not supported".into());
        }
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
        let tree = resvg::usvg::Tree::from_str(text, &options).map_err(|e| e.to_string())?;
        let size = tree.size();
        let scale = ICON_SIDE as f32 / size.width().max(size.height());
        let width = (size.width() * scale).ceil().clamp(1., ICON_SIDE as f32) as u32;
        let height = (size.height() * scale).ceil().clamp(1., ICON_SIDE as f32) as u32;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).ok_or("Invalid icon dimensions")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        pixmap.encode_png().map_err(|e| e.to_string())?
    } else {
        let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Ico
        ) {
            return Err("Unsupported icon format".into());
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|e| e.to_string())?
            .thumbnail(ICON_SIDE, ICON_SIDE);
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        png.into_inner()
    };
    Ok(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
}

fn custom_image(value: &str) -> Result<Arc<Image>, String> {
    if value.len() > MAX_ICON_BYTES * 2 {
        return Err("Project icon exceeds limit".into());
    }
    let (extension, data) = value.split_once(':').ok_or("Invalid custom icon")?;
    decode(
        &STANDARD.decode(data).map_err(|e| e.to_string())?,
        extension,
    )
}

pub(crate) fn read_icon(path: PathBuf) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "svg" | "ico") {
        return Err("Choose PNG, JPEG, SVG or ICO".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular image file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ICON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let image = decode(&bytes, &extension)?;
    Ok(format!("png:{}", STANDARD.encode(image.bytes())))
}

fn group_image(value: &str) -> Result<Arc<Image>, String> {
    let builtin = match value {
        "aws-lambda" => Some((include_str!("../../../assets/aws-lambda.svg"), "#FF9900")),
        "kubernetes" => Some((include_str!("../../../assets/kubernetes.svg"), "#78A9FF")),
        _ => None,
    };
    if let Some((svg, color)) = builtin {
        return decode(
            svg.replace("<path ", &format!("<path fill=\"{color}\" "))
                .as_bytes(),
            "svg",
        );
    }
    custom_image(value)
}

pub(crate) fn group_choice_icon(
    value: Option<&str>,
    tone: dunlin_domain::ProjectTone,
    side: f32,
    p: &Palette,
) -> AnyElement {
    use std::sync::OnceLock;
    static LAMBDA: OnceLock<Arc<Image>> = OnceLock::new();
    static KUBERNETES: OnceLock<Arc<Image>> = OnceLock::new();
    let image = match value {
        Some("aws-lambda") => Some(
            LAMBDA
                .get_or_init(|| group_image("aws-lambda").unwrap())
                .clone(),
        ),
        Some("kubernetes") => Some(
            KUBERNETES
                .get_or_init(|| group_image("kubernetes").unwrap())
                .clone(),
        ),
        _ => None,
    };
    div()
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .map(|view| match image {
            Some(image) => view.child(img(image).size(px(side)).object_fit(ObjectFit::Contain)),
            None => view.child(dot(tone_color(p, tone), side * 0.65)),
        })
        .into_any_element()
}

pub(crate) fn render_group(
    this: &DunlinApp,
    group: &dunlin_domain::ProjectGroup,
    side: f32,
    p: &Palette,
) -> AnyElement {
    let image = this
        .project_icons
        .groups
        .get(&group.id)
        .filter(|(value, _)| group.settings.icon.as_ref() == Some(value));
    let view = div()
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center();
    if let Some((_, image)) = image {
        #[cfg(debug_assertions)]
        let view = view
            .relative()
            .children(crate::workspace_ui_smoke::probe("group-image"));
        view.child(
            img(image.clone())
                .size(px(side))
                .object_fit(ObjectFit::Contain),
        )
        .into_any_element()
    } else {
        #[cfg(debug_assertions)]
        let view = view
            .relative()
            .children(crate::workspace_ui_smoke::probe("group-dot"));
        view.child(
            div()
                .size(px((side * 0.5).clamp(8., 14.)))
                .rounded_full()
                .bg(tone_color(p, group.tone)),
        )
        .into_any_element()
    }
}

fn detect(root: &str, previous: Option<&Detected>) -> Option<Detected> {
    let root = std::fs::canonicalize(root).ok()?;
    let mut attempts = 0;
    for source in discovery::candidates(&root) {
        if !source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "svg" | "png" | "jpg" | "jpeg" | "ico"
                )
            })
        {
            continue;
        }
        let Some(bytes) = discovery::read_local(&root, &source, MAX_ICON_BYTES) else {
            continue;
        };
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hash);
        let signature = hash.finish();
        if let Some(previous) =
            previous.filter(|old| old.source == source && old.signature == signature)
        {
            return Some(previous.clone());
        }
        attempts += 1;
        if let Ok(image) = decode(
            &bytes,
            source.extension().and_then(|s| s.to_str()).unwrap_or(""),
        ) {
            return Some(Detected {
                source,
                signature,
                image,
            });
        }
        if attempts >= 12 {
            break;
        }
    }
    None
}

impl DunlinApp {
    pub(crate) fn reset_project_color_picker(&mut self) {
        self.project_icons.color = None;
    }

    pub(crate) fn project_color_picker(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = &self.projects[index];
        if self
            .project_icons
            .color
            .as_ref()
            .is_none_or(|(id, _, _)| id != &project.id)
        {
            let id = project.id.clone();
            let color = project
                .settings
                .custom_color
                .map(|value| rgba(value).into())
                .unwrap_or_else(|| tone_color(&palette(cx), project.tone));
            let state = cx.new(|cx| ColorPickerState::new(window, cx).default_value(color));
            let owner = id.clone();
            let subscription =
                cx.subscribe(&state, move |this, _, event: &ColorPickerEvent, cx| {
                    let ColorPickerEvent::Change(color) = event;
                    if let Some(index) = this.projects.iter().position(|p| p.id == owner) {
                        let mut project = this.projects[index].clone();
                        project.settings.custom_color =
                            color.map(|color| u32::from(color.to_rgb()));
                        if let Some(store) = &this.store
                            && let Err(error) = store.upsert_project(&project)
                        {
                            this.file_notice(format!("Could not save project color: {error}"), cx);
                            return;
                        }
                        this.projects[index] = project;
                        cx.notify();
                    }
                });
            self.project_icons.color = Some((id, state, subscription));
        }
        ColorPicker::new(&self.project_icons.color.as_ref().unwrap().1)
            .icon(Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="currentColor"/></svg>"#)
                .size(px(24.)).text_color(self.project_icons.color.as_ref().unwrap().1.read(cx).value()
                    .unwrap_or_else(|| tone_color(&palette(cx), self.projects[index].tone))))
            .label("Custom dot color")
            .into_any_element()
    }

    pub(crate) fn choose_project_icon(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let id = project.id.clone();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose project icon (PNG, JPEG, SVG or ICO; up to 2 MiB)".into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { read_icon(path) })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(icon) => {
                    if let Some(index) = this.projects.iter().position(|p| p.id == id) {
                        this.projects[index].settings.custom_icon = Some(icon);
                        this.projects[index].settings.custom_color = None;
                        this.project_icons.color = None;
                        this.store_project_settings(index);
                        this.project_icons.entries.remove(&id);
                        this.refresh_project_icons(cx);
                        cx.notify();
                    }
                }
                Err(error) => this.file_notice(error, cx),
            });
        })
        .detach();
    }
    pub(crate) fn reset_project_icon(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(project) = self.projects.get_mut(index) {
            project.settings.custom_icon = None;
            project.settings.custom_color = None;
            self.project_icons.color = None;
            self.project_icons.entries.remove(&project.id);
            self.store_project_settings(index);
            self.refresh_project_icons(cx);
            cx.notify();
        }
    }
    pub(crate) fn refresh_project_icons(&mut self, cx: &mut Context<Self>) {
        self.project_icons.groups.retain(|id, (value, _)| {
            self.project_groups
                .iter()
                .any(|g| &g.id == id && g.settings.icon.as_ref() == Some(value))
        });
        for group in &self.project_groups {
            if !self.project_icons.groups.contains_key(&group.id)
                && let Some(value) = &group.settings.icon
                && let Ok(image) = group_image(value)
            {
                self.project_icons
                    .groups
                    .insert(group.id.clone(), (value.clone(), image));
            }
        }
        self.project_icons.entries.retain(|id, entry| {
            self.projects.iter().any(|p| {
                matches_project(p, id, &entry.root) && p.settings.custom_icon == entry.custom
            })
        });
        if self.project_icons.busy {
            return;
        }
        // A single worker, at most four projects per batch. Slow/missing mounts
        // never cause filesystem work in a render or a burst of parallel decoders.
        let mut projects: Vec<_> = self
            .projects
            .iter()
            .filter(|p| p.settings.custom_icon.is_some() || (p.trusted && !p.is_remote()))
            .filter(|p| {
                self.project_icons
                    .entries
                    .get(&p.id)
                    .is_none_or(|entry| entry.checked.elapsed() >= REFRESH)
            })
            .collect();
        projects.sort_by_key(|p| p.id != self.project().id);
        let projects: Vec<_> = projects
            .into_iter()
            .take(4)
            .map(|p| {
                (
                    p.id.clone(),
                    p.path.clone(),
                    p.settings.custom_icon.clone(),
                    self.project_icons
                        .entries
                        .get(&p.id)
                        .and_then(|entry| entry.icon.clone()),
                )
            })
            .collect();
        if projects.is_empty() {
            return;
        }
        self.project_icons.busy = true;
        let revision = self.project_icons.revision;
        cx.spawn(async move |this, cx| {
            let rows = cx
                .background_executor()
                .spawn(async move {
                    projects
                        .into_iter()
                        .map(|(id, root, custom, previous)| {
                            let icon = if let Some(value) = &custom {
                                custom_image(value).ok().map(|image| Detected {
                                    source: PathBuf::new(),
                                    signature: 0,
                                    image,
                                })
                            } else {
                                detect(&root, previous.as_ref())
                            };
                            (id, root, custom, icon)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ =
                this.update(cx, |this, cx| {
                    this.project_icons.busy = false;
                    for (id, root, custom, icon) in rows {
                        if this.projects.iter().any(|p| {
                            matches_project(p, &id, &root) && p.settings.custom_icon == custom
                        }) {
                            this.project_icons.entries.insert(
                                id,
                                Entry {
                                    root,
                                    custom,
                                    checked: if this.project_icons.revision == revision {
                                        Instant::now()
                                    } else {
                                        Instant::now() - REFRESH
                                    },
                                    icon,
                                },
                            );
                        }
                    }
                    cx.notify();
                });
        })
        .detach();
    }
    pub(crate) fn invalidate_project_icon(&mut self) {
        // A watcher change during an in-flight read must trigger another scan,
        // not mark the old snapshot as fresh for an entire refresh interval.
        self.project_icons.revision = self.project_icons.revision.wrapping_add(1);
        let id = self.project().id.clone();
        if let Some(entry) = self.project_icons.entries.get_mut(&id) {
            entry.checked = Instant::now() - REFRESH;
        }
    }
}

fn monogram(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect();
    if letters.is_empty() {
        name.chars().take(2).collect::<String>().to_uppercase()
    } else {
        letters.to_uppercase()
    }
}

pub(crate) fn render(this: &DunlinApp, project: &Project, side: f32, p: &Palette) -> AnyElement {
    if let Some(color) = project.settings.custom_color {
        return div()
            .size(px(side))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(dot(rgba(color).into(), (side * 0.55).clamp(10., 18.)))
            .into_any_element();
    }
    let icon = this
        .project_icons
        .entries
        .get(&project.id)
        .filter(|entry| {
            matches_project(project, &project.id, &entry.root)
                && project.settings.custom_icon == entry.custom
        })
        .and_then(|entry| entry.icon.as_ref());
    let base = div()
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center();
    match icon {
        Some(icon) => base
            .child(
                img(icon.image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
        None => base
            .rounded(px(side * 0.27))
            .bg(tone_soft(p, project.tone))
            .child(
                div()
                    .text_size(px((side * 0.43).max(10.)))
                    .font_family(dunlin_ui::theme::code_font())
                    .font_weight(FontWeight::BOLD)
                    .text_color(tone_color(p, project.tone))
                    .child(monogram(&project.name)),
            )
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests;
