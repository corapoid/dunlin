//! In-memory document tabs. Each editor owns its buffer, undo and scroll state.
use crate::{
    app::{DunlinApp, EditorConflict, Inspector, Toast, ToastAction},
    file_preview::Preview,
    remote::WorkspaceBackend,
};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::component::{
    Icon,
    button::ButtonVariants,
    input::{EditorState, InputEvent},
};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use dunlin_workspace::preview::{self, PreviewKind};
use std::sync::Arc;

mod new_tabs;
mod saving;

const FILE_NOTICE_DURATION: std::time::Duration = std::time::Duration::from_millis(2500);

pub(crate) fn empty_workspace(cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .size_full()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .gap(px(18.))
        .relative()
        .children(tab_probe("empty-workspace"))
        .child(
            Icon::new(IconName::Bird)
                .size(px(132.))
                .text_color(p.muted_fg.opacity(0.12)),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted_fg.opacity(0.55))
                .child("A little space for your next idea"),
        )
        .child(
            dunlin_ui::action_button("reopen-chat-tab")
                .label("Open Chat")
                .on_click(cx.listener(|this, _, window, cx| this.show_chat_tab(window, cx))),
        )
        .into_any_element()
}
fn tab_probe(name: &'static str) -> Option<AnyElement> {
    #[cfg(debug_assertions)]
    {
        crate::workspace_ui_smoke::probe(name)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        None
    }
}

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

fn save_new_buffer(
    root: &str,
    path: &std::path::Path,
    text: &str,
) -> Result<(String, dunlin_workspace::fs::FileStat), String> {
    let display_root = std::path::PathBuf::from(root);
    if text.len() as u64 > dunlin_workspace::fs::MAX_EDITOR_BYTES {
        return Err("This file is too large to save".into());
    }
    let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
    let parent = std::fs::canonicalize(path.parent().ok_or("Choose a file name")?)
        .map_err(|error| error.to_string())?;
    let path = parent.join(path.file_name().ok_or("Choose a file name")?);
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| "Choose a location inside this project")?
        .to_string_lossy();
    let root = root.to_string_lossy();
    let created = dunlin_workspace::fs::create_file(&root, &relative)?;
    // Keep any created file on failure; the unsaved buffer remains available to retry.
    let stat = WorkspaceBackend::Local {
        root: root.into_owned(),
    }
    .write_file(&path.to_string_lossy(), text, Some(created.mtime_ms))?;
    Ok((
        display_root
            .join(relative.as_ref())
            .to_string_lossy()
            .into_owned(),
        stat,
    ))
}

fn closing_tab_ids<'a>(
    project: &str,
    tabs: impl Iterator<Item = (&'a str, &'a str, bool)>,
) -> Vec<String> {
    tabs.filter(|(_, owner, locked)| *owner == project && !locked)
        .map(|(id, _, _)| id.to_string())
        .collect()
}
fn preview_fit() -> ObjectFit {
    ObjectFit::Contain
}

#[derive(Default)]
pub(crate) struct State {
    pub panes: crate::workspace_panes::State,
    pub tabs: Vec<Tab>,
    pub active: Option<String>,
    pub show_file: bool,
    pub terminal: Option<String>,
    pub new_tab_menu: Option<usize>,
    pub new_tab_press: Option<(usize, std::time::Instant)>,
    pub preview_cache: crate::file_preview::Cache,
    pub closed_chats: std::collections::HashSet<String>,
    pub chat_tabs: Vec<String>,
    pub(crate) chat_drafts: std::collections::HashMap<String, String>,
    pub(crate) chat_scrolls: std::collections::HashMap<String, crate::chat_scroll::ChatScroll>,
    pub pending_chat_draft: Option<String>,
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
    pub untitled: bool,
    pub saving: bool,
    pub locked: bool,
    pub minimap: crate::minimap::State,
    pub markdown: crate::markdown_editor::State,
    pub blame: crate::inline_blame::State,
    pub git_changed: bool,
    pub reload_generation: u64,
    _subscription: Subscription,
    _observation: Subscription,
}

impl DunlinApp {
    pub(crate) fn save_untitled_file(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self
            .workbench
            .tabs
            .iter()
            .find(|tab| tab.id == id && tab.untitled && !tab.saving)
        else {
            return;
        };
        if self.project().is_remote() {
            // shortcut: native Save As is local-only; add a relay path prompt when remote scratch saving is needed.
            self.file_notice("Keep or copy this buffer; saving an untitled file currently requires a local project.", cx);
            return;
        }
        let id = tab.id.clone();
        let root = tab.root.clone();
        let project = tab.project.clone();
        if !self.guard_ai_commit_mutation(cx) {
            return;
        }
        let receiver = cx.prompt_for_new_path(std::path::Path::new(&root), Some("Untitled.txt"));
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        self.workbench
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .unwrap()
            .saving = true;
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |this, cx| {
                let Some(index) = this.workbench.tabs.iter().position(|tab| tab.id == id) else {
                    return;
                };
                this.workbench.tabs[index].saving = false;
                let path = match result {
                    Ok(Ok(Some(path))) => path,
                    Ok(Err(error)) => {
                        this.file_notice(error.to_string(), cx);
                        return;
                    }
                    _ => {
                        cx.notify();
                        return;
                    }
                };
                if this.project().id != project
                    || this.project().path != root
                    || !this.guard_ai_commit_mutation(cx)
                {
                    return;
                }
                let text = this.workbench.tabs[index]
                    .editor
                    .read(cx)
                    .value()
                    .to_string();
                match save_new_buffer(&root, &path, &text) {
                    Ok((path, stat)) => {
                        let tab = &mut this.workbench.tabs[index];
                        tab.inspector.path = path.clone();
                        tab.inspector.name = std::path::Path::new(&path)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned();
                        tab.inspector.language = crate::app::editor_language(&path).into();
                        tab.inspector.bytes = stat.bytes as usize;
                        tab.inspector.directory = std::path::Path::new(&path)
                            .parent()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned();
                        tab.untitled = false;
                        tab.dirty = false;
                        tab.saved_text = Some(text);
                        tab.mtime = Some(stat.mtime_ms);
                        tab.minimap.invalidate_baseline();
                        tab.blame = Default::default();
                        tab.git_changed = false;
                        tab.conflict = None;
                        let language = tab.inspector.language.clone();
                        tab.editor
                            .update(cx, |editor, cx| editor.set_highlighter(language, cx));
                        if this.workbench.active.as_deref() == Some(&id) {
                            this.inspector = Some(tab.inspector.clone());
                            this.editor_dirty = false;
                            this.editor_mtime = tab.mtime;
                            this.editor_conflict = None;
                            this.finish_pending_editor(cx);
                        }
                        this.refresh_git_status(cx);
                    }
                    Err(error) => this.file_notice(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn ensure_chat_tab(&mut self) {
        let id = self.session().id.clone();
        self.workbench.closed_chats.remove(&id);
        if !self.workbench.chat_tabs.contains(&id) {
            self.workbench.chat_tabs.push(id);
        }
        self.workbench
            .panes
            .opened(crate::workspace_panes::TabKey::Chat(
                self.session().id.clone(),
            ));
    }

    pub(crate) fn stash_chat_ui(&mut self, cx: &App) {
        let id = self.session().id.clone();
        self.workbench
            .panes
            .composers
            .insert(id.clone(), self.composer.clone());
        self.workbench
            .panes
            .transcripts
            .insert(id.clone(), self.harness.transcript.clone());
        let draft = self
            .workbench
            .pending_chat_draft
            .take()
            .unwrap_or_else(|| self.composer.read(cx).value().to_string());
        self.workbench.chat_drafts.insert(id.clone(), draft);
        self.workbench
            .chat_scrolls
            .insert(id, std::mem::take(&mut self.chat_scroll));
    }

    pub(crate) fn rekey_chat_tab(&mut self, old: &str, new: &str) {
        for id in self.project_session_selections.values_mut() {
            if id == old {
                *id = new.into();
            }
        }
        for id in &mut self.workbench.chat_tabs {
            if id == old {
                *id = new.into();
            }
        }
        if self.workbench.closed_chats.remove(old) {
            self.workbench.closed_chats.insert(new.into());
        }
        if let Some(draft) = self.workbench.chat_drafts.remove(old) {
            self.workbench.chat_drafts.insert(new.into(), draft);
        }
        if let Some(scroll) = self.workbench.chat_scrolls.remove(old) {
            self.workbench.chat_scrolls.insert(new.into(), scroll);
        }
        self.workbench.panes.rekey_chat(old, new);
    }

    pub(crate) fn restore_chat_ui(&mut self) {
        let id = self.session().id.clone();
        if let Some(composer) = self.workbench.panes.composers.get(&id) {
            self.composer = composer.clone();
        }
        self.workbench.pending_chat_draft = Some(
            self.workbench
                .chat_drafts
                .get(&id)
                .cloned()
                .unwrap_or_default(),
        );
        self.chat_scroll = self.workbench.chat_scrolls.remove(&id).unwrap_or_default();
        self.ensure_chat_tab();
    }

    pub(crate) fn activate_chat_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_session_id(id, cx) {
            self.show_chat_tab(window, cx);
        }
    }
    pub(crate) fn retained_buffer_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.active_file_tab().filter(|t| t.git_changed)?;
        let p = palette(cx);
        let is_text = tab.kind == PreviewKind::Text;
        Some(
            div()
                .flex_shrink_0()
                .px(px(10.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(p.amber)
                .child(if is_text {
                    "Review file after Git · open buffer preserved"
                } else {
                    "Review preview after Git · close and reopen it"
                })
                .when(is_text, |banner| {
                    banner.child(
                        dunlin_ui::action_button("review-retained-git-buffer")
                            .label("Review…")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor_conflict =
                                    this.active_file_tab().and_then(|t| t.conflict.clone());
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        )
    }
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
            tab.git_changed = false;
            tab.conflict = None;
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
            if !tab.git_changed || self.editor_conflict.is_some() {
                tab.conflict = self.editor_conflict.clone();
            }
        }
    }
    pub(crate) fn has_dirty_tabs(&self) -> bool {
        self.editor_dirty
            || self
                .workbench
                .tabs
                .iter()
                .any(|t| t.saving || (t.dirty && Some(&t.id) != self.workbench.active.as_ref()))
    }
    pub(crate) fn dirty_tab_paths(&self) -> Vec<String> {
        let mut paths: Vec<_> = self
            .workbench
            .tabs
            .iter()
            .filter(|t| {
                t.project == self.project().id
                    && (t.loading || t.dirty || t.saving)
                    && Some(&t.id) != self.workbench.active.as_ref()
            })
            .map(|t| t.inspector.path.clone())
            .collect();
        if self.editor_dirty
            && let Some(i) = &self.inspector
        {
            paths.push(i.path.clone());
        }
        if let Some(tab) = self.active_file_tab().filter(|t| t.loading || t.saving) {
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
        self.remember_workspace_tab();
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
        let preview_only =
            crate::markdown_editor::supports(&tab.inspector.path) && tab.markdown.preview_only();
        self.workbench.active = Some(id.into());
        self.workbench.show_file = true;
        self.workbench.terminal = None;
        self.git_history.active = false;
        self.spec.active = false;
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let wrap = self.word_wrap;
        self.editor.update(cx, |state, cx| {
            state.set_soft_wrap(wrap, window, cx);
            if !preview_only {
                state.focus(window, cx);
            }
        });
        if preview_only {
            self.focus_handle.focus(window, cx);
        }
        self.check_open_file(cx);
        self.remember_workspace_tab();
        cx.notify();
    }
    pub(crate) fn show_chat_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.spec.active = false;
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
    pub(crate) fn close_chat_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.session().id.clone();
        self.close_session_tab(&id, window, cx);
    }

    pub(crate) fn close_session_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active =
            self.current_workspace_tab() == Some(crate::workspace_panes::TabKey::Chat(id.into()));
        self.workbench.chat_tabs.retain(|tab| tab != id);
        self.workbench
            .panes
            .closed(&crate::workspace_panes::TabKey::Chat(id.into()));
        self.workbench.closed_chats.insert(id.to_string());
        if !active {
            cx.notify();
            return;
        }
        self.sync_workspace_panes(window, cx);
        cx.notify();
    }
    pub(crate) fn open_workbench_file(
        &mut self,
        path: &str,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_buffer(path, false, line, window, cx);
    }

    pub(crate) fn new_untitled_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_buffer(
            &format!("untitled:{}", uuid::Uuid::now_v7()),
            true,
            None,
            window,
            cx,
        );
    }

    fn open_file_buffer(
        &mut self,
        path: &str,
        untitled: bool,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.explorer.busy || self.group_cleanup.busy {
            self.file_notice(
                "Wait for the current file operation before opening a preview",
                cx,
            );
            return;
        }
        let kind = if untitled {
            PreviewKind::Text
        } else {
            preview::kind(path)
        };
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
                if crate::markdown_editor::supports(path) {
                    self.set_markdown_mode(crate::markdown_editor::Mode::Source, window, cx);
                }
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
                    if this.autosave
                        && dirty
                        && !this.active_file_tab().is_some_and(|tab| tab.untitled)
                    {
                        this.schedule_autosave(cx);
                    }
                }
                cx.notify();
            },
        );
        let observation = cx.observe(&editor, |this, entity, cx| {
            this.queue_markdown_preview(entity.entity_id(), cx);
            if this.editor.entity_id() == entity.entity_id()
                && (this.editor_minimap || this.inline_blame)
            {
                cx.notify();
            }
        });
        let p = std::path::Path::new(path);
        let inspector = Inspector {
            path: path.into(),
            name: if untitled {
                "Untitled".into()
            } else {
                p.file_name().unwrap_or_default().to_string_lossy().into()
            },
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
            saved_text: untitled.then(String::new),
            readonly: !untitled,
            mtime: None,
            conflict: None,
            kind,
            preview: None,
            loading: !untitled,
            untitled,
            saving: false,
            locked: false,
            minimap: Default::default(),
            markdown: Default::default(),
            blame: Default::default(),
            git_changed: false,
            reload_generation: 0,
            _subscription: subscription,
            _observation: observation,
        });
        self.workbench
            .panes
            .opened(crate::workspace_panes::TabKey::File(id.clone()));
        // Do not stash the previous editor again after inserting the loading tab.
        self.workbench.active = None;
        self.activate_file_tab(&id, window, cx);
        if untitled {
            return;
        }
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
        let Some(tab) = self
            .workbench
            .tabs
            .iter()
            .find(|tab| tab.id == id && tab.project == self.project().id)
        else {
            return;
        };
        if tab.saving {
            self.file_notice("Wait for the file to finish saving before closing it", cx);
            return;
        }
        if self.current_workspace_tab() != Some(crate::workspace_panes::TabKey::File(id.into()))
            && !tab.dirty
        {
            self.workbench.tabs.retain(|tab| tab.id != id);
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::File(id.into()));
            cx.notify();
            return;
        }
        self.activate_file_tab(id, window, cx);
        self.request_close_inspector(cx);
    }
    pub(crate) fn close_active_tab(&mut self) {
        if let Some(id) = self.workbench.active.take() {
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::File(id.clone()));
            self.workbench.tabs.retain(|t| t.id != id);
        }
        self.workbench.show_file = false;
        self.close_inspector();
    }
    pub(crate) fn request_close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        if self
            .workbench
            .tabs
            .iter()
            .any(|tab| tab.project == self.project().id && tab.saving)
        {
            self.file_notice("Wait for pending saves before closing all tabs", cx);
            return;
        }
        let project = self.project().id.clone();
        let locked: std::collections::HashSet<String> = self
            .workbench
            .tabs
            .iter()
            .filter(|tab| tab.locked)
            .map(|tab| tab.id.clone())
            .collect();
        let keys: Vec<_> = self
            .workspace_tab_keys()
            .into_iter()
            .filter(|key| {
                !matches!(key, crate::workspace_panes::TabKey::File(id) if locked.contains(id))
            })
            .collect();
        let ids = closing_tab_ids(
            &project,
            self.workbench
                .tabs
                .iter()
                .map(|tab| (tab.id.as_str(), tab.project.as_str(), tab.locked)),
        );
        if keys.is_empty() {
            return;
        }
        if !self
            .workbench
            .tabs
            .iter()
            .any(|tab| ids.contains(&tab.id) && tab.dirty)
        {
            self.finish_close_all_tabs(&keys, cx);
            return;
        }
        let root = self.project().path.clone();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Close all tabs?",
            Some("Some files have unsaved changes. Terminal tabs will close; conversations stay in history and agents keep running."),
            &["Save All and Close", "Discard and Close", "Cancel"],
            cx,
        );
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice @ (0 | 1)) = answer.await else {
                return;
            };
            let _ = this.update_in(cx, |this, _, cx| {
                if this.project().id != project || this.project().path != root {
                    return;
                }
                if choice == 0 {
                    let expected_project = project.clone();
                    this.save_tabs_then(Some(&project), cx, move |this, cx| {
                        if this.project().id == expected_project && this.project().path == root {
                            this.finish_close_all_tabs(&keys, cx);
                        }
                    });
                    return;
                }
                this.finish_close_all_tabs(&keys, cx);
            });
        })
        .detach();
    }

    fn finish_close_all_tabs(
        &mut self,
        keys: &[crate::workspace_panes::TabKey],
        cx: &mut Context<Self>,
    ) {
        use crate::workspace_panes::TabKey;
        for key in keys {
            match key {
                TabKey::Chat(id) => {
                    self.workbench.chat_tabs.retain(|tab| tab != id);
                    self.workbench.closed_chats.insert(id.clone());
                }
                TabKey::File(id) => self.workbench.tabs.retain(|tab| &tab.id != id),
                TabKey::Terminal(id) => self.close_terminal_tab(id, cx),
                TabKey::Git(_) => self.close_git_history(cx),
                TabKey::Spec(_) => self.close_spec_workspace(cx),
            }
            self.workbench.panes.closed(key);
        }
        if self
            .workbench
            .active
            .as_ref()
            .is_some_and(|id| keys.contains(&TabKey::File(id.clone())))
        {
            self.workbench.active = None;
            self.close_inspector();
        }
        self.workbench.terminal = None;
        self.workbench.panes.active = [None, None];
        self.workbench.panes.restore = None;
        cx.notify();
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
            .retain(|t| t.untitled || t.kind == preview::kind(&t.inspector.path));
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
    pub(crate) fn render_workbench_tabs(
        &self,
        pane: usize,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        use crate::workspace_panes::TabKey;
        let p = palette(cx);
        let selection = self.workbench.panes.active[pane].as_ref();
        let mut tabs = Vec::new();
        let mut keys = Vec::new();
        for session in self
            .workbench
            .chat_tabs
            .iter()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
        {
            if !self
                .workbench
                .panes
                .belongs(&TabKey::Chat(session.id.clone()), pane)
            {
                continue;
            }
            let current = session.id == self.session().id;
            let id = session.id.clone();
            let key_open = id.clone();
            let close = id.clone();
            keys.push(TabKey::Chat(id.clone()));
            tabs.push(
                document_tab(session.title.clone())
                    .focusable()
                    .tab_stop(true)
                    .aria_label(format!("Open session {}", session.title))
                    .prefix(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .child(Icon::new(IconName::MessageSquare).size(px(14.)))
                            .when(current, |icon| icon.children(tab_probe("chat-tab"))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_chat_tab(&id, window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.activate_chat_tab(&key_open, window, cx);
                        }
                    }))
                    .suffix(
                        icon_button(
                            format!("close-chat-tab:{close}"),
                            IconName::X,
                            format!(
                                "Close session tab {} without stopping its agent",
                                session.title
                            ),
                        )
                        .when(current, |button| {
                            button.tooltip(self.shortcut_label(
                                "Close active session tab without stopping its agent",
                                "⌘W",
                            ))
                        })
                        .relative()
                        .when(current, |button| {
                            button.children(tab_probe("close-chat-tab"))
                        })
                        .size(px(24.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_session_tab(&close, window, cx);
                            },
                        )),
                    ),
            );
        }
        for tab in self.workbench.tabs.iter().filter(|t| {
            t.project == self.project().id
                && self
                    .workbench
                    .panes
                    .belongs(&TabKey::File(t.id.clone()), pane)
        }) {
            let id = tab.id.clone();
            let close = id.clone();
            let lock = id.clone();
            let menu = id.clone();
            let active = selection == Some(&TabKey::File(id.clone()));
            let name = tab.inspector.name.clone();
            let locked = tab.locked;
            let dirty = if self.workbench.active.as_deref() == Some(&id) {
                self.editor_dirty
            } else {
                tab.dirty
            };
            keys.push(TabKey::File(id.clone()));
            tabs.push(
                document_tab(format!("{}{}", name, if dirty { " •" } else { "" }))
                    .prefix(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .child(
                                Icon::new(crate::file_icons::appearance(&name, &p).0).size(px(14.)),
                            )
                            .when(active, |view| view.children(tab_probe("file-tab-drag"))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_file_tab(&id, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.context_menu = Some(crate::app::ContextMenuState {
                                target: crate::app::ContextTarget::Tab(menu.clone()),
                                position: event.position,
                            });
                            cx.notify();
                        }),
                    )
                    .suffix(if locked {
                        icon_button(
                            format!("lock-tab:{lock}"),
                            IconName::Lock,
                            format!("{name} is locked · click to unlock"),
                        )
                        .relative()
                        .when(active, |button| {
                            button.children(tab_probe("close-file-tab"))
                        })
                        .size(px(24.))
                        .text_color(p.amber)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(tab) =
                                this.workbench.tabs.iter_mut().find(|tab| tab.id == lock)
                            {
                                tab.locked = false;
                            }
                            cx.notify();
                        }))
                        .into_any_element()
                    } else {
                        crate::gui_probe::target(
                            "close-file-tab",
                            icon_button(
                                format!("close-tab:{close}"),
                                IconName::X,
                                format!("Close {name}"),
                            )
                            .relative()
                            .when(active, |button| {
                                button.children(tab_probe("close-file-tab"))
                            })
                            .size(px(24.))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.request_close_tab(&close, window, cx);
                                },
                            )),
                        )
                    }),
            );
        }
        for terminal in self
            .terminal
            .tabs
            .iter()
            .filter(|tab| tab.workspace && tab.project == self.project().id)
        {
            let key = TabKey::Terminal(terminal.id.clone());
            if !self.workbench.panes.belongs(&key, pane) {
                continue;
            }
            keys.push(key);
            let id = terminal.id.clone();
            let close = id.clone();
            tabs.push(
                document_tab(terminal.title.clone())
                    .prefix(Icon::new(IconName::Terminal).size(px(14.)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_workspace_terminal(&id, window, cx);
                    }))
                    .suffix(
                        icon_button(
                            format!("close-workspace-terminal:{close}"),
                            IconName::X,
                            "Close terminal",
                        )
                        .size(px(24.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_terminal_tab(&close, cx);
                        })),
                    ),
            );
        }
        if self.git_history.is_open_for(self.project())
            && self
                .workbench
                .panes
                .belongs(&TabKey::Git(self.project().id.clone()), pane)
        {
            keys.push(TabKey::Git(self.project().id.clone()));
            tabs.push(
                document_tab(self.git_history.tab_label())
                    .prefix(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .child(Icon::new(IconName::GitBranch).size(px(14.)))
                            .children(tab_probe("git-history-tab")),
                    )
                    .aria_label("Git history tab")
                    .on_click(cx.listener(|this, _, window, cx| this.open_git_history(window, cx)))
                    .suffix(
                        icon_button("close-git-history-tab", IconName::X, "Close Git history")
                            .when(self.git_history.is_active_for(self.project()), |button| {
                                button.tooltip(self.shortcut_label("Close Git history", "⌘W"))
                            })
                            .relative()
                            .children(tab_probe("close-git-history-tab"))
                            .size(px(24.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                let active = this.git_history.is_active_for(this.project());
                                this.close_git_history(cx);
                                if active {
                                    this.focus_handle.focus(window, cx);
                                }
                            })),
                    ),
            );
        }
        if self.spec_tools_enabled
            && self.spec.is_open_for(self.project())
            && self
                .workbench
                .panes
                .belongs(&TabKey::Spec(self.project().id.clone()), pane)
        {
            keys.push(TabKey::Spec(self.project().id.clone()));
            tabs.push(
                document_tab("Spec")
                    .aria_label("Spec workspace tab")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_spec_workspace(window, cx)),
                    )
                    .suffix(
                        icon_button("close-spec-workspace-tab", IconName::X, "Close Spec")
                            .size(px(24.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                let active = this.spec.is_active_for(this.project());
                                this.close_spec_workspace(cx);
                                if active {
                                    this.focus_handle.focus(window, cx);
                                }
                            })),
                    ),
            );
        }
        let order = self.workspace_tab_keys();
        let mut ordered: Vec<_> = keys.into_iter().zip(tabs).collect();
        ordered.sort_by_key(|(key, _)| {
            order
                .iter()
                .position(|tab| tab == key)
                .unwrap_or(usize::MAX)
        });
        let selected = ordered.iter().position(|(key, _)| Some(key) == selection);
        let project = self.project().id.clone();
        let tabs = ordered
            .into_iter()
            .map(|(key, tab)| {
                let (label, icon) = match &key {
                    TabKey::Chat(id) => (
                        self.sessions
                            .iter()
                            .find(|session| &session.id == id)
                            .map(|session| session.title.clone())
                            .unwrap_or_else(|| "Chat".into()),
                        IconName::MessageSquare,
                    ),
                    TabKey::File(id) => (
                        self.workbench
                            .tabs
                            .iter()
                            .find(|tab| &tab.id == id)
                            .map(|tab| tab.inspector.name.clone())
                            .unwrap_or_else(|| "File".into()),
                        IconName::File,
                    ),
                    TabKey::Git(_) => (self.git_history.tab_label().into(), IconName::GitBranch),
                    TabKey::Spec(_) => ("Spec".into(), IconName::File),
                    TabKey::Terminal(id) => (
                        self.terminal
                            .tabs
                            .iter()
                            .find(|tab| &tab.id == id)
                            .map(|tab| tab.title.clone())
                            .unwrap_or_else(|| "Terminal".into()),
                        IconName::Terminal,
                    ),
                };
                tab.background_motion(!self.reduce_motion)
                    .drag_over::<crate::workspace_panes::TabDrag>(|style, _, _, cx| {
                        style.border_b_2().border_color(palette(cx).blue)
                    })
                    .on_drop(cx.listener({
                        let key = key.clone();
                        move |this, drag: &crate::workspace_panes::TabDrag, window, cx| {
                            cx.stop_propagation();
                            this.drop_workspace_tab_at(drag, pane, Some(&key), window, cx);
                        }
                    }))
                    .on_drag(
                        crate::workspace_panes::TabDrag {
                            key,
                            project: project.clone(),
                        },
                        move |_, _, _, cx| {
                            cx.new(|_| crate::workspace_panes::TabPreview {
                                label: label.clone(),
                                icon,
                            })
                        },
                    )
            })
            .collect::<Vec<_>>();
        let menu = div()
            .flex()
            .flex_shrink_0()
            .child(
                icon_button(
                    format!("split-workspace:{pane}"),
                    IconName::PanelLeft,
                    "Split workspace side by side",
                )
                .on_click(cx.listener(|this, _, window, cx| this.split_workspace(window, cx))),
            )
            .when(self.workbench.panes.enabled, |menu| {
                menu.child(
                    dunlin_ui::button(format!("move-workspace-tab:{pane}"))
                        .label(if pane == 0 { "→" } else { "←" })
                        .accessibility_label("Move selected tab to other pane")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.move_workspace_tab(pane, window, cx)
                        })),
                )
                .child(
                    icon_button(
                        format!("merge-workspace:{pane}"),
                        IconName::X,
                        "Merge workspace panes",
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.merge_workspace(cx))),
                )
            })
            .map(|bar| {
                bar.child(
                    dunlin_ui::button("document-tabs-menu")
                        .ghost()
                        .relative()
                        .when(pane == 0, |button| {
                            button.children(tab_probe("document-tabs-menu"))
                        })
                        .icon(IconName::Ellipsis)
                        .accessibility_label("Document tab menu")
                        .dropdown_menu_with_anchor(Anchor::TopRight, {
                            let app = cx.entity().downgrade();
                            let project = self.project().id.clone();
                            move |menu, _, cx| {
                                let minimap =
                                    app.upgrade().is_some_and(|app| app.read(cx).editor_minimap);
                                let split_app = app.clone();
                                let terminal_app = app.clone();
                                let map_app = app.clone();
                                let app = app.clone();
                                let project = project.clone();
                                menu.item(PopupMenuItem::new("Split view").on_click(
                                    move |_, window, cx| {
                                        let _ = split_app.update(cx, |this, cx| {
                                            this.split_workspace(window, cx)
                                        });
                                    },
                                ))
                                .item(PopupMenuItem::new("Terminal tab").on_click(
                                    move |_, window, cx| {
                                        let _ = terminal_app.update(cx, |this, cx| {
                                            this.open_workspace_terminal(window, cx)
                                        });
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new(if minimap {
                                        "Hide file map"
                                    } else {
                                        "Show file map"
                                    })
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = map_app.update(cx, |this, cx| {
                                                this.editor_minimap = !this.editor_minimap;
                                                this.persist_settings(cx);
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new("Close all tabs").on_click(
                                        move |_, window, cx| {
                                            if let Some(app) = app.upgrade() {
                                                app.update(cx, |this, cx| {
                                                    if this.project().id == project {
                                                        this.request_close_all_tabs(window, cx);
                                                    }
                                                });
                                            }
                                        },
                                    ),
                                )
                            }
                        }),
                )
            });
        // Folder tabs reserve 4 px for their foot; align + with the visible labels.
        let new_tab = div()
            .relative()
            .flex()
            .flex_shrink_0()
            .mt(-px(4.))
            .child(new_tabs::button(self, pane, cx))
            .children(tab_probe("new-chat-tab"));
        let strip = div()
            .id(format!("workspace-tabs-drop:{pane}"))
            .h(px(36.))
            .relative()
            .children(tab_probe("document-tabs-strip"))
            .flex_shrink_0()
            .flex()
            .overflow_hidden()
            .rounded_t(px(dunlin_ui::scale::WORKSPACE_RADIUS))
            .drag_over::<crate::workspace_panes::TabDrag>(|style, _, _, cx| {
                style.border_b_2().border_color(palette(cx).blue)
            })
            .on_drop(cx.listener(
                move |this, drag: &crate::workspace_panes::TabDrag, window, cx| {
                    cx.stop_propagation();
                    this.drop_workspace_tab(drag, pane, window, cx);
                },
            ))
            .child(
                document_tabs(format!("document-tabs:{pane}"))
                    .children(tabs)
                    // The kit reserves one radius before its first tab for the foot.
                    .ml(-px(dunlin_ui::radii::FOLDER_TAB))
                    .when_some(selected, |tabs, selected| tabs.selected_index(selected))
                    .last_empty_space(new_tab)
                    .suffix(menu)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .into_any_element();
        (strip, selected.is_none_or(|index| index > 0))
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
            for (label, icon, delta) in [
                ("Previous page", IconName::ArrowLeft, -1isize),
                ("Next page", IconName::ArrowRight, 1isize),
            ] {
                header = header.child(
                    icon_button(format!("pdf-page:{label}"), icon, label).on_click(
                        cx.listener(move |this, _, _, cx| this.change_pdf_page(delta, cx)),
                    ),
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
            .child(header)
            .when_some(self.retained_buffer_banner(cx), |root, banner| {
                root.child(banner)
            })
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
    #[test]
    fn scratch_save_creates_only_a_new_project_file_and_never_overwrites_or_escapes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("draft.txt");
        assert!(!path.exists());
        super::save_new_buffer(root.path().to_str().unwrap(), &path, "draft").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft");
        assert!(super::save_new_buffer(root.path().to_str().unwrap(), &path, "overwrite").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft");
        let outside = tempfile::tempdir().unwrap();
        assert!(
            super::save_new_buffer(
                root.path().to_str().unwrap(),
                &outside.path().join("escape.txt"),
                "no"
            )
            .is_err()
        );
        assert!(!outside.path().join("escape.txt").exists());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
            assert!(
                super::save_new_buffer(
                    root.path().to_str().unwrap(),
                    &root.path().join("linked/escape.txt"),
                    "no"
                )
                .is_err()
            );
        }
    }
    use super::{FILE_NOTICE_DURATION, buffer_changed, preview_failure};
    use crate::app::{Toast, ToastAction};
    #[test]
    fn close_all_only_targets_the_visible_projects_unlocked_file_tabs() {
        let tabs = [
            ("one", "project", false),
            ("other", "another", false),
            ("two", "project", true),
            ("three", "project", false),
        ];
        assert_eq!(
            super::closing_tab_ids("project", tabs.into_iter()),
            ["one", "three"]
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
                group: Box::new(dunlin_domain::ProjectGroup {
                    id: "group".into(),
                    name: "Group".into(),
                    tone: dunlin_domain::ProjectTone::Blue,
                    parent_id: None,
                    settings: Default::default(),
                }),
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
            group: Box::new(dunlin_domain::ProjectGroup {
                id: "deleted".into(),
                name: "Deleted".into(),
                tone: dunlin_domain::ProjectTone::Blue,
                parent_id: None,
                settings: Default::default(),
            }),
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
