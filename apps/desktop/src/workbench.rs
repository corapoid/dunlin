//! In-memory document tabs. Each editor owns its buffer, undo and scroll state.
use crate::{
    app::{DunlinApp, EditorConflict, Inspector, Toast, ToastAction},
    file_preview::Preview,
    remote::WorkspaceBackend,
};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{
    Icon,
    input::{EditorState, InputEvent},
};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use dunlin_workspace::preview::{self, PreviewKind};
use std::sync::Arc;

const FILE_NOTICE_DURATION: std::time::Duration = std::time::Duration::from_millis(2500);

fn preview_failure(path: &str, detail: Option<&str>) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    match detail {
        Some(detail) => format!("Nie można otworzyć podglądu pliku „{name}”: {detail}"),
        None => format!("Nie można otworzyć podglądu pliku „{name}”."),
    }
}

fn buffer_changed(saved: Option<&str>, current: &str) -> bool {
    saved != Some(current)
}

fn closing_tab_ids<'a>(
    project: &str,
    tabs: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<String> {
    tabs.filter(|(_, owner)| *owner == project)
        .map(|(id, _)| id.to_string())
        .collect()
}
fn preview_fit() -> ObjectFit {
    ObjectFit::Contain
}

#[derive(Default)]
pub(crate) struct State {
    pub tabs: Vec<Tab>,
    pub active: Option<String>,
    pub show_file: bool,
    pub preview_cache: crate::file_preview::Cache,
}
pub(crate) struct Tab {
    pub id: String,
    pub project: String,
    pub root: String,
    pub inspector: Inspector,
    pub editor: Entity<EditorState>,
    pub backend: WorkspaceBackend,
    pub dirty: bool,
    pub saved_text: Option<String>,
    pub readonly: bool,
    pub mtime: Option<u64>,
    pub conflict: Option<EditorConflict>,
    pub kind: PreviewKind,
    pub preview: Option<Preview>,
    pub loading: bool,
    pub minimap: crate::minimap::State,
    _subscription: Subscription,
    _observation: Subscription,
}

impl DunlinApp {
    pub(crate) fn file_notice(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        let id = uuid::Uuid::now_v7();
        self.toast = Some(Toast {
            id,
            message: message.clone(),
            action_label: "OK".into(),
            action: ToastAction::Info,
        });
        self.set_notice(message, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILE_NOTICE_DURATION).await;
            let _ = this.update(cx, |this, cx| {
                // A prior notification must never dismiss a newer one (or Undo).
                if this
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.is_expiration_target(id))
                {
                    if this.notice.as_ref() == this.toast.as_ref().map(|toast| &toast.message) {
                        this.notice = None;
                    }
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn record_saved_editor_text(&mut self, text: String) {
        if let Some(tab) = self
            .workbench
            .tabs
            .iter_mut()
            .find(|tab| Some(&tab.id) == self.workbench.active.as_ref())
        {
            tab.saved_text = Some(text);
            tab.dirty = false;
            tab.minimap.invalidate();
        }
        self.editor_dirty = false;
    }
    pub(crate) fn stash_editor_tab(&mut self) {
        if let Some(tab) = self
            .workbench
            .tabs
            .iter_mut()
            .find(|t| Some(&t.id) == self.workbench.active.as_ref())
            && let Some(inspector) = self
                .inspector
                .as_ref()
                .filter(|i| i.path == tab.inspector.path)
        {
            tab.inspector = inspector.clone();
            tab.editor = self.editor.clone();
            tab.dirty = self.editor_dirty;
            tab.readonly = self.editor_readonly;
            tab.mtime = self.editor_mtime;
            tab.conflict = self.editor_conflict.clone();
        }
    }
    pub(crate) fn has_dirty_tabs(&self) -> bool {
        self.editor_dirty
            || self
                .workbench
                .tabs
                .iter()
                .any(|t| t.dirty && Some(&t.id) != self.workbench.active.as_ref())
    }
    pub(crate) fn dirty_tab_paths(&self) -> Vec<String> {
        let mut paths: Vec<_> = self
            .workbench
            .tabs
            .iter()
            .filter(|t| {
                t.project == self.project().id
                    && (t.loading || t.dirty)
                    && Some(&t.id) != self.workbench.active.as_ref()
            })
            .map(|t| t.inspector.path.clone())
            .collect();
        if self.editor_dirty
            && let Some(i) = &self.inspector
        {
            paths.push(i.path.clone());
        }
        if let Some(tab) = self.active_file_tab().filter(|t| t.loading) {
            paths.push(tab.inspector.path.clone());
        }
        paths
    }
    pub(crate) fn active_file_tab(&self) -> Option<&Tab> {
        self.workbench.tabs.iter().find(|t| {
            Some(&t.id) == self.workbench.active.as_ref() && t.project == self.project().id
        })
    }
    pub(crate) fn activate_file_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_editor_tab();
        let Some(tab) = self
            .workbench
            .tabs
            .iter()
            .find(|t| t.id == id && t.project == self.project().id)
        else {
            return;
        };
        self.inspector = Some(tab.inspector.clone());
        self.editor = tab.editor.clone();
        self.editor_dirty = tab.dirty;
        self.editor_readonly = tab.readonly;
        self.editor_mtime = tab.mtime;
        self.editor_conflict = tab.conflict.clone();
        self.editor_stale = false;
        self.workbench.active = Some(id.into());
        self.workbench.show_file = true;
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let wrap = self.word_wrap;
        self.editor.update(cx, |state, cx| {
            state.set_soft_wrap(wrap, window, cx);
            state.focus(window, cx);
        });
        self.check_open_file(cx);
        cx.notify();
    }
    pub(crate) fn show_chat_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
    pub(crate) fn open_workbench_file(
        &mut self,
        path: &str,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.explorer.busy {
            self.file_notice(
                "Wait for the current file operation before opening a preview",
                cx,
            );
            return;
        }
        let kind = preview::kind(path);
        if kind == PreviewKind::Unsupported {
            self.file_notice(preview_failure(path, None), cx);
            return;
        }
        self.remember_dashboard_project(cx);
        if let Some(id) = self
            .workbench
            .tabs
            .iter()
            .find(|t| t.project == self.project().id && t.inspector.path == path)
            .map(|t| t.id.clone())
        {
            self.activate_file_tab(&id, window, cx);
            if kind == PreviewKind::Text
                && let Some(line) = line
            {
                self.jump_to_line(line, window, cx);
            }
            return;
        }
        if self.workbench.tabs.len() >= 48 {
            self.file_notice("Close a file tab before opening another (limit: 48)", cx);
            return;
        }
        let backend = match self.backend() {
            Ok(b) => b,
            Err(e) => {
                self.file_notice(e, cx);
                return;
            }
        };
        self.stash_editor_tab();
        let id = uuid::Uuid::now_v7().to_string();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(crate::app::editor_language(path))
                .soft_wrap(self.word_wrap)
        });
        let subscription = cx.subscribe_in(
            &editor,
            window,
            |this, entity, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let active = this.editor.entity_id() == entity.entity_id();
                let mut dirty = true;
                if let Some(tab) = this
                    .workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.editor.entity_id() == entity.entity_id())
                {
                    if tab.loading || tab.readonly {
                        return;
                    }
                    let text = entity.read(cx).text();
                    dirty = match &tab.saved_text {
                        Some(saved) if saved.len() != text.len() => true,
                        saved => buffer_changed(saved.as_deref(), &text.to_string()),
                    };
                    tab.dirty = dirty;
                    tab.minimap.invalidate();
                }
                if active {
                    this.editor_dirty = dirty;
                    if this.autosave && dirty {
                        this.schedule_autosave(cx);
                    }
                }
                cx.notify();
            },
        );
        let observation = cx.observe(&editor, |this, entity, cx| {
            if this.editor_minimap && this.editor.entity_id() == entity.entity_id() {
                cx.notify();
            }
        });
        let p = std::path::Path::new(path);
        let inspector = Inspector {
            path: path.into(),
            name: p.file_name().unwrap_or_default().to_string_lossy().into(),
            directory: p
                .parent()
                .map(|p| dunlin_workspace::fs::relative_to(&self.project().path, p))
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| ".".into()),
            language: crate::app::editor_language(path).into(),
            bytes: 0,
            snippet: None,
        };
        self.workbench.tabs.push(Tab {
            id: id.clone(),
            project: self.project().id.clone(),
            root: self.project().path.clone(),
            inspector,
            editor: editor.clone(),
            backend: backend.clone(),
            dirty: false,
            saved_text: None,
            readonly: true,
            mtime: None,
            conflict: None,
            kind,
            preview: None,
            loading: true,
            minimap: Default::default(),
            _subscription: subscription,
            _observation: observation,
        });
        // Do not stash the previous editor again after inserting the loading tab.
        self.workbench.active = None;
        self.activate_file_tab(&id, window, cx);
        let path = path.to_string();
        let requested_path = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if kind == PreviewKind::Text {
                        let file = backend.read_file(&path)?;
                        if file.binary {
                            return Err("Plik binarny.".to_string());
                        }
                        Ok((Some(file), None))
                    } else {
                        let bytes = Arc::new(backend.preview_bytes(&path)?);
                        Ok((None, Some(crate::file_preview::decode(bytes, kind, 1)?)))
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(index) = this.workbench.tabs.iter().position(|t| t.id == id) else {
                    return;
                };
                let active = this.workbench.active.as_deref() == Some(&id)
                    && this.project().id == this.workbench.tabs[index].project;
                match result {
                    Ok((file, preview)) => {
                        if kind == PreviewKind::Pdf
                            && let Some(preview) = preview.clone()
                        {
                            this.workbench.preview_cache.insert(&id, preview);
                        }
                        let tab = &mut this.workbench.tabs[index];
                        tab.preview = preview;
                        if let Some(file) = file {
                            tab.inspector.bytes = file.bytes as usize;
                            tab.mtime = Some(file.mtime_ms);
                            tab.readonly = file.readonly;
                            let language = tab.inspector.language.clone();
                            tab.saved_text = Some(file.contents.clone());
                            tab.minimap.invalidate();
                            tab.editor.update(cx, |state, cx| {
                                state.set_value(file.contents, window, cx);
                                state.set_highlighter(language, cx);
                            });
                        }
                        tab.loading = false;
                        tab.dirty = false;
                        if active {
                            this.inspector = Some(tab.inspector.clone());
                            this.editor_mtime = tab.mtime;
                            this.editor_readonly = tab.readonly;
                            this.editor_dirty = false;
                            if kind == PreviewKind::Text
                                && let Some(line) = line
                            {
                                this.jump_to_line(line, window, cx);
                            }
                        }
                    }
                    Err(e) => {
                        this.workbench.tabs.remove(index);
                        if active {
                            this.workbench.active = None;
                            this.workbench.show_file = false;
                            this.close_inspector();
                        }
                        this.file_notice(preview_failure(&requested_path, Some(&e)), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn request_close_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_file_tab(id, window, cx);
        self.request_close_inspector(cx);
    }
    pub(crate) fn close_active_tab(&mut self) {
        if let Some(id) = self.workbench.active.take() {
            self.workbench.tabs.retain(|t| t.id != id);
        }
        self.workbench.show_file = false;
        self.close_inspector();
    }
    pub(crate) fn save_all_tabs(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_tabs_for_project(None, cx)
    }

    pub(crate) fn save_tabs_for_project(
        &mut self,
        project: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.stash_editor_tab();
        for index in 0..self.workbench.tabs.len() {
            let tab = &self.workbench.tabs[index];
            if !tab.dirty || project.is_some_and(|project| tab.project != project) {
                continue;
            }
            if self.ai_commit_locks_project_id(&tab.project) {
                self.file_notice(
                    "Cancel or finish the AI commit before saving files in its repository",
                    cx,
                );
                return false;
            }
            if !self
                .projects
                .iter()
                .any(|p| p.id == tab.project && p.path == tab.root)
            {
                self.file_notice(
                    "A dirty tab's project was relocated. Restore the project path before saving.",
                    cx,
                );
                return false;
            }
            let text = tab.editor.read(cx).value().to_string();
            match tab
                .backend
                .write_file(&tab.inspector.path, &text, tab.mtime)
            {
                Ok(stat) => {
                    let tab = &mut self.workbench.tabs[index];
                    tab.dirty = false;
                    tab.saved_text = Some(text);
                    tab.minimap.invalidate();
                    tab.mtime = Some(stat.mtime_ms);
                    tab.conflict = None;
                    if Some(&tab.id) == self.workbench.active.as_ref() {
                        self.editor_dirty = false;
                        self.editor_mtime = tab.mtime;
                        self.editor_conflict = None;
                    }
                }
                Err(e) => {
                    self.file_notice(format!("Could not save {}: {e}", tab.inspector.path), cx);
                    return false;
                }
            }
        }
        true
    }

    pub(crate) fn request_close_all_file_tabs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_editor_tab();
        let project = self.project().id.clone();
        let ids = closing_tab_ids(
            &project,
            self.workbench
                .tabs
                .iter()
                .map(|tab| (tab.id.as_str(), tab.project.as_str())),
        );
        if ids.is_empty() {
            return;
        }
        if !self
            .workbench
            .tabs
            .iter()
            .any(|tab| ids.contains(&tab.id) && tab.dirty)
        {
            self.finish_close_all_file_tabs(&ids, window, cx);
            return;
        }
        let root = self.project().path.clone();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Close all file tabs?",
            Some("Some files have unsaved changes. Chat and its agent will remain open."),
            &["Save All and Close", "Discard and Close", "Cancel"],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice @ (0 | 1)) = answer.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.project().id != project || this.project().path != root {
                    return;
                }
                if choice == 0 && !this.save_tabs_for_project(Some(&project), cx) {
                    return;
                }
                this.finish_close_all_file_tabs(&ids, window, cx);
            });
        })
        .detach();
    }

    fn finish_close_all_file_tabs(
        &mut self,
        ids: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_closed = self
            .workbench
            .active
            .as_ref()
            .is_some_and(|id| ids.contains(id));
        self.workbench.tabs.retain(|tab| !ids.contains(&tab.id));
        if active_closed {
            self.workbench.active = None;
            self.close_inspector();
        }
        self.show_chat_tab(window, cx);
    }
    pub(crate) fn retarget_file_tabs(
        &mut self,
        report: &dunlin_workspace::transfer::TransferReport,
        cx: &mut Context<Self>,
    ) {
        self.stash_editor_tab();
        let project = self.project().id.clone();
        for tab in self
            .workbench
            .tabs
            .iter_mut()
            .filter(|t| t.project == project)
        {
            if let Some(path) = report
                .completed
                .iter()
                .filter(|entry| entry.moved)
                .find_map(|entry| {
                    std::path::Path::new(&tab.inspector.path)
                        .strip_prefix(&entry.source)
                        .ok()
                        .map(|tail| std::path::Path::new(&entry.destination).join(tail))
                })
            {
                tab.inspector.path = path.to_string_lossy().into();
                tab.inspector.name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
                tab.inspector.directory = path
                    .parent()
                    .map(|p| dunlin_workspace::fs::relative_to(&tab.root, p))
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| ".".into());
                tab.inspector.language = crate::app::editor_language(&tab.inspector.path).into();
                let language = tab.inspector.language.clone();
                tab.editor
                    .update(cx, |state, cx| state.set_highlighter(language, cx));
            }
        }
        // A changed extension must go through the allowlist/decoder again.
        self.workbench
            .tabs
            .retain(|t| t.kind == preview::kind(&t.inspector.path));
        if let Some(tab) = self
            .workbench
            .tabs
            .iter()
            .find(|t| Some(&t.id) == self.workbench.active.as_ref())
        {
            self.inspector = Some(tab.inspector.clone());
        } else {
            self.workbench.active = None;
            self.workbench.show_file = false;
            self.close_inspector();
        }
    }
    pub(crate) fn render_workbench_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut row = div()
            .id("document-tabs")
            .h(px(35.))
            .flex_shrink_0()
            .overflow_x_scroll()
            .flex()
            .items_center()
            .gap(px(2.))
            .bg(p.panel)
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .id("chat-tab")
                    .h_full()
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(!self.workbench.show_file, |v| {
                        v.bg(p.card).border_b_2().border_color(p.blue)
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.show_chat_tab(window, cx)))
                    .child("Chat"),
            );
        for tab in self
            .workbench
            .tabs
            .iter()
            .filter(|t| t.project == self.project().id)
        {
            let id = tab.id.clone();
            let close = id.clone();
            let active = self.workbench.show_file && self.workbench.active.as_deref() == Some(&id);
            let dirty = if self.workbench.active.as_deref() == Some(&id) {
                self.editor_dirty
            } else {
                tab.dirty
            };
            row = row.child(
                div()
                    .id(format!("document-tab:{id}"))
                    .h_full()
                    .px(px(10.))
                    .max_w(px(240.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .cursor_pointer()
                    .when(active, |v| v.bg(p.card).border_b_2().border_color(p.blue))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_file_tab(&id, window, cx)
                    }))
                    .child(div().text_size(px(11.5)).truncate().child(format!(
                        "{}{}",
                        tab.inspector.name,
                        if dirty { " •" } else { "" }
                    )))
                    .child(
                        div()
                            .id(format!("close-tab:{close}"))
                            .size(px(17.))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close_tab(&close, window, cx);
                            }))
                            .child(Icon::new(IconName::X).size(px(11.))),
                    ),
            );
        }
        div()
            .h(px(35.))
            .flex_shrink_0()
            .flex()
            .bg(p.panel)
            .border_b_1()
            .border_color(p.border)
            .child(div().flex_1().min_w(px(0.)).overflow_hidden().child(row))
            .when(
                self.workbench
                    .tabs
                    .iter()
                    .any(|tab| tab.project == self.project().id),
                |bar| {
                    bar.child(
                        div()
                            .id("close-all-file-tabs")
                            .aria_label("Close all file tabs, keep Chat")
                            .px(px(10.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover).text_color(p.fg))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.request_close_all_file_tabs(window, cx)
                            }))
                            .child(Icon::new(IconName::X).size(px(12.)))
                            .child("Close all"),
                    )
                },
            )
            .into_any_element()
    }
    pub(crate) fn preview_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(tab) = self.active_file_tab() else {
            return div().into_any_element();
        };
        let mut header = div()
            .h(px(42.))
            .flex_shrink_0()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(p.border)
            .child(tab.inspector.name.clone());
        if let Some(preview) = &tab.preview
            && preview.pages > 1
        {
            for (label, delta) in [("←", -1isize), ("→", 1isize)] {
                header = header.child(
                    div()
                        .id(format!("pdf-page:{label}"))
                        .px(px(8.))
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.change_pdf_page(delta, cx)),
                        )
                        .child(label),
                );
            }
            header = header.child(format!("{} / {}", preview.page, preview.pages));
        }
        if tab.loading && tab.preview.is_some() {
            header = header.child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child("Rendering page…"),
            );
        }
        let mut body = div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden();
        if let Some(preview) = &tab.preview {
            let image = preview.image.clone();
            body = body.child(
                // Paint against the actual allocated viewport; image intrinsic
                // dimensions do not participate in flex or percentage sizing.
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        let viewport = Bounds::new(
                            bounds.origin + point(px(16.), px(16.)),
                            size(
                                (bounds.size.width - px(32.)).max(px(0.)),
                                (bounds.size.height - px(32.)).max(px(0.)),
                            ),
                        );
                        if viewport.size.width > px(0.) && viewport.size.height > px(0.) {
                            let fitted = preview_fit().get_bounds(viewport, image.size(0));
                            let _ = window.paint_image(
                                fitted,
                                fitted,
                                Corners::default(),
                                image.clone(),
                                0,
                                false,
                            );
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        } else {
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(p.muted_fg)
                    .child("Loading preview…"),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .bg(p.card)
            .child(header)
            .child(body)
            .into_any_element()
    }
    fn change_pdf_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(tab) = self.active_file_tab() else {
            return;
        };
        if tab.loading {
            return;
        }
        let Some(preview) = &tab.preview else {
            return;
        };
        let page = (preview.page as isize + delta).clamp(1, preview.pages as isize) as usize;
        if page == preview.page {
            return;
        }
        let bytes = preview.bytes.clone();
        let path = tab.inspector.path.clone();
        let id = tab.id.clone();
        if let Some(preview) = self.workbench.preview_cache.get(&id, page) {
            if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
                tab.preview = Some(preview);
            }
            cx.notify();
            return;
        }
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            tab.loading = true;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::file_preview::decode(bytes, PreviewKind::Pdf, page) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workbench.tabs.iter().any(|tab| tab.id == id)
                    && let Ok(preview) = &result
                {
                    this.workbench.preview_cache.insert(&id, preview.clone());
                }
                if let Some(tab) = this.workbench.tabs.iter_mut().find(|t| t.id == id) {
                    tab.loading = false;
                    match result {
                        Ok(preview) => tab.preview = Some(preview),
                        Err(e) => this.file_notice(preview_failure(&path, Some(&e)), cx),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{FILE_NOTICE_DURATION, buffer_changed, preview_failure};
    use crate::app::{Toast, ToastAction};
    #[test]
    fn close_all_only_targets_the_visible_projects_file_tabs() {
        let tabs = [("one", "project"), ("other", "another"), ("two", "project")];
        assert_eq!(
            super::closing_tab_ids("project", tabs.into_iter()),
            ["one", "two"]
        );
        assert!(super::closing_tab_ids("missing", tabs.into_iter()).is_empty());
    }
    #[test]
    fn preview_containment_keeps_portrait_and_landscape_images_inside_resized_viewports() {
        use dunlin_ui::{Bounds, DevicePixels, point, px, size};
        for (width, height) in [(120., 80.), (800., 600.), (320., 900.)] {
            let viewport = Bounds::new(point(px(0.), px(0.)), size(px(width), px(height)));
            for (iw, ih) in [(1600u32, 400u32), (400, 1600), (1600, 1600)] {
                let image = super::preview_fit().get_bounds(
                    viewport,
                    size(DevicePixels::from(iw), DevicePixels::from(ih)),
                );
                assert!(
                    image.size.width <= viewport.size.width
                        && image.size.height <= viewport.size.height
                );
                assert!(image.origin.x >= px(0.) && image.origin.y >= px(0.));
                assert!(
                    (image.size.width / image.size.height - iw as f32 / ih as f32).abs() < 0.001
                );
            }
        }
    }
    #[test]
    fn preview_failures_name_the_requested_file_without_its_parent_path() {
        for path in ["/project/assets/movie.mp4", "assets/movie.mp4", "movie.mp4"] {
            let message = preview_failure(path, None);
            assert!(message.contains("„movie.mp4”"));
            assert!(!message.contains("assets/"));
        }
        let message = preview_failure("/project/zażółć.png", Some("Invalid image"));
        assert!(message.contains("„zażółć.png”"));
        assert!(message.contains("Invalid image"));
    }

    #[test]
    fn returning_to_the_saved_text_is_clean_even_after_same_length_changes() {
        assert!(!buffer_changed(Some("original"), "original"));
        assert!(buffer_changed(Some("original"), "modified"));
        assert!(!buffer_changed(Some("original"), "original"));
        assert!(!buffer_changed(Some(""), ""));
        assert!(buffer_changed(Some("żółć"), "zolc"));
        assert!(buffer_changed(None, ""));
    }

    #[test]
    fn an_old_toast_timer_cannot_remove_a_new_identical_notification_or_undo() {
        let old = uuid::Uuid::now_v7();
        let toast = Toast {
            id: uuid::Uuid::now_v7(),
            message: "same message".into(),
            action_label: "OK".into(),
            action: ToastAction::Info,
        };
        assert!(!toast.is_expiration_target(old));
        assert!(toast.is_expiration_target(toast.id));
        let undo = Toast {
            id: old,
            message: "Undo group deletion".into(),
            action_label: "Undo".into(),
            action: ToastAction::RestoreGroup {
                group: dunlin_domain::ProjectGroup {
                    id: "group".into(),
                    name: "Group".into(),
                    tone: dunlin_domain::ProjectTone::Blue,
                    parent_id: None,
                },
                project_ids: Vec::new(),
                child_ids: Vec::new(),
            },
        };
        assert!(!undo.is_expiration_target(old));
        assert!((2000..=3000).contains(&FILE_NOTICE_DURATION.as_millis()));
    }

    #[test]
    fn undo_does_not_reclaim_members_moved_away_and_back_after_deletion() {
        let mut action = ToastAction::RestoreGroup {
            group: dunlin_domain::ProjectGroup {
                id: "deleted".into(),
                name: "Deleted".into(),
                tone: dunlin_domain::ProjectTone::Blue,
                parent_id: None,
            },
            project_ids: vec!["moved-project".into(), "untouched-project".into()],
            child_ids: vec!["moved-child".into(), "untouched-child".into()],
        };
        action.member_moved("moved-project", false);
        action.member_moved("moved-child", true);
        action.member_moved("moved-project", false);
        action.member_moved("moved-child", true);
        let ToastAction::RestoreGroup {
            project_ids,
            child_ids,
            ..
        } = action
        else {
            panic!("Undo lost");
        };
        assert_eq!(project_ids, ["untouched-project"]);
        assert_eq!(child_ids, ["untouched-child"]);
    }
}
