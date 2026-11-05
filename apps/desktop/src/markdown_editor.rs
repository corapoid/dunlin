//! Source-backed visual Markdown, rendered by default, with an explicit Raw mode.
//! Native block projections share the canonical source buffer, history and save.
use crate::{app::DunlinApp, workbench::Tab};
use dunlin_ui::{
    component::{
        Disableable, Sizable,
        button::ButtonVariants,
        input::Editor,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{path::Path, time::Duration};

#[cfg(debug_assertions)]
pub(crate) mod smoke;

mod containers;
mod document;
mod editing;
mod interaction;
mod resources;
mod selection;
mod structure;
mod table;
mod toolbar;
mod visual;
use editing::Format;

const MAX_PREVIEW_BYTES: usize = 128 * 1024;
const MAX_PREVIEW_LINES: usize = 2000;
const MAX_FORMAT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Visual,
    Source,
}

#[derive(Default)]
pub(crate) struct State {
    mode: Mode,
    generation: u64,
    requested: Option<SharedString>,
    rendered: Option<SharedString>,
    pending: bool,
    error: Option<String>,
    scroll: ScrollHandle,
    visual: visual::State,
    resource: Option<resources::Draft>,
}

impl State {
    pub(crate) fn preview_only(&self) -> bool {
        self.mode == Mode::Visual
    }
}

pub(crate) fn supports(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}
fn preview_allowed(text: &str) -> bool {
    text.len() <= MAX_PREVIEW_BYTES
        && text.lines().take(MAX_PREVIEW_LINES + 1).count() <= MAX_PREVIEW_LINES
}
fn editable(app: &DunlinApp, tab: &Tab) -> bool {
    supports(&tab.inspector.path)
        && !tab.loading
        && !tab.readonly
        && !app.editor_readonly
        && !app.explorer.busy
        && !app.ai_commit_locks_project()
        && tab.root == app.project().path
        && tab.project == app.project().id
}

impl DunlinApp {
    fn toggle_markdown_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_file_tab() {
            let mode = match tab.markdown.mode {
                Mode::Visual => Mode::Source,
                Mode::Source => Mode::Visual,
            };
            self.set_markdown_mode(mode, window, cx);
        }
    }

    pub(crate) fn queue_markdown_preview(&mut self, editor: EntityId, cx: &mut Context<Self>) {
        let Some(tab) = self.workbench.tabs.iter_mut().find(|t| {
            t.editor.entity_id() == editor
                && supports(&t.inspector.path)
                && t.markdown.mode != Mode::Source
                && !t.loading
        }) else {
            return;
        };
        if tab.editor.read(cx).text().len() > MAX_PREVIEW_BYTES {
            tab.markdown.generation = tab.markdown.generation.wrapping_add(1);
            tab.markdown.pending = false;
            tab.markdown.requested = None;
            tab.markdown.rendered = None;
            tab.markdown.error = Some(
                "Preview is limited to 128 KiB / 2,000 lines. The source remains available.".into(),
            );
            cx.notify();
            return;
        }
        let text = tab.editor.read(cx).text().to_string();
        if tab
            .markdown
            .requested
            .as_ref()
            .is_some_and(|old| old.as_ref() == text)
        {
            return;
        }
        tab.markdown.generation = tab.markdown.generation.wrapping_add(1);
        let generation = tab.markdown.generation;
        tab.markdown.rendered = None;
        tab.markdown.requested = Some(text.clone().into());
        tab.markdown.error = None;
        if !preview_allowed(&text) {
            tab.markdown.pending = false;
            tab.markdown.error = Some(
                "Preview is limited to 128 KiB / 2,000 lines. The source remains available.".into(),
            );
            cx.notify();
            return;
        }
        tab.markdown.pending = true;
        let (id, project, root) = (tab.id.clone(), tab.project.clone(), tab.root.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this
                    .projects
                    .iter()
                    .any(|p| p.id == project && p.path == root)
                {
                    return;
                }
                let Some(tab) = this.workbench.tabs.iter_mut().find(|t| {
                    t.id == id
                        && t.root == root
                        && t.markdown.mode != Mode::Source
                        && t.markdown.generation == generation
                }) else {
                    return;
                };
                tab.markdown.rendered = tab.markdown.requested.clone();
                tab.markdown.pending = false;
                if tab.markdown.mode == Mode::Visual {
                    tab.markdown
                        .visual
                        .sync(tab.markdown.rendered.as_deref().unwrap_or(""));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn set_markdown_mode(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == Mode::Source
            && let Some(entity) = self.active_file_tab().and_then(|t| {
                t.markdown
                    .visual
                    .active
                    .as_ref()
                    .map(|a| a.editor.entity_id())
            })
        {
            self.apply_visual_change(entity, window, cx);
        }
        let Some(id) = self.active_file_tab().map(|t| t.id.clone()) else {
            return;
        };
        let Some(tab) = self
            .workbench
            .tabs
            .iter_mut()
            .find(|t| t.id == id && supports(&t.inspector.path))
        else {
            return;
        };
        tab.markdown.mode = mode;
        tab.markdown.resource = None;
        tab.markdown.visual.active = None;
        tab.markdown.generation = tab.markdown.generation.wrapping_add(1);
        tab.markdown.pending = false;
        tab.markdown.requested = None;
        let editor = tab.editor.clone();
        if mode == Mode::Visual {
            self.focus_handle.focus(window, cx);
        } else {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        if mode != Mode::Source {
            self.queue_markdown_preview(editor.entity_id(), cx);
        }
        cx.notify();
    }

    fn format_markdown(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .active_file_tab()
            .is_some_and(|t| t.markdown.mode == Mode::Visual)
        {
            self.format_visual_markdown(format, window, cx);
            return;
        }
        let Some(tab) = self.active_file_tab().filter(|t| editable(self, t)) else {
            return;
        };
        let editor = tab.editor.clone();
        if editor.read(cx).text().len() > MAX_FORMAT_BYTES {
            self.file_notice("Markdown formatting is limited to 2 MiB", cx);
            return;
        }
        let text = editor.read(cx).text().to_string();
        let range = editor.read(cx).selected_range();
        let edit = match editing::edit(&text, range, format) {
            Ok(edit) => edit,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        // One atomic edit, using the existing buffer/history. Never set_value.
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
            state.set_selected_range(edit.selection, cx);
            state.focus(window, cx);
        });
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let Some(tab) = this.active_file_tab() else {
        return div().into_any_element();
    };
    let p = palette(cx);
    let mode = tab.markdown.mode;
    let mut body = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .overflow_hidden();
    if mode == Mode::Visual {
        body = body.child(visual::render(this, cx));
    } else {
        body = body.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_hidden()
                .child(
                    Editor::new(&tab.editor)
                        .appearance(false)
                        .bordered(false)
                        .readonly(!editable(this, tab))
                        .aria_label("Markdown source")
                        .h_full(),
                )
                .relative()
                .child(crate::inline_blame::overlay(this, cx)),
        );
        if this.editor_minimap && mode == Mode::Source {
            body = body.child(crate::minimap::render(this, cx));
        }
    }
    div()
        .id("markdown-editor")
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(p.card)
        .text_color(p.fg)
        .child(
            div()
                .px(px(12.))
                .py(px(7.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.))
                        .truncate()
                        .child(format!(
                            "{}{}",
                            tab.inspector.name,
                            if this.editor_dirty { " •" } else { "" }
                        )),
                )
                .child(toolbar::mode_toggle(mode, cx))
                .child(
                    dunlin_ui::button("markdown-save")
                        .ghost()
                        .small()
                        .icon(assets::IconName::Save)
                        .tooltip("Save · ⌘S")
                        .accessibility_label("Save Markdown")
                        .disabled(!this.editor_dirty || !editable(this, tab))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_editor(cx);
                        })),
                )
                .child(
                    dunlin_ui::button("markdown-close")
                        .ghost()
                        .small()
                        .icon(assets::IconName::X)
                        .tooltip("Close")
                        .accessibility_label("Close Markdown")
                        .on_click(cx.listener(|this, _, _, cx| this.request_close_inspector(cx))),
                ),
        )
        .child(toolbar::render(this, cx))
        .when_some(this.retained_buffer_banner(cx), |root, banner| {
            root.child(banner)
        })
        .when_some(resources::render(this, cx), |root, panel| root.child(panel))
        .child(body)
        .when_some(crate::inline_blame::status(this, cx), |root, status| {
            root.child(status)
        })
        .when(mode == Mode::Visual, |root| {
            root.child(
                // Keep the canonical editor's existing document history/action node
                // mounted, outside the visual editor's Undo/Redo capture branch.
                div()
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .size(px(1.))
                    .overflow_hidden()
                    .opacity(0.)
                    .child(
                        Editor::new(&tab.editor)
                            .appearance(false)
                            .bordered(false)
                            .tab_index(-1)
                            .role(dunlin_ui::base::RoleOverride::Presentational)
                            .readonly(!editable(this, tab))
                            .w(px(800.))
                            .h(px(1.)),
                    ),
            )
        })
        .child(
            div()
                .px(px(12.))
                .py(px(4.))
                .flex_shrink_0()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .border_t_1()
                .border_color(p.border)
                .child(if mode == Mode::Visual {
                    "Visual editing · ⌘S to save"
                } else {
                    "Markdown source · ⌘S to save"
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Mode, preview_allowed, supports};

    #[test]
    fn only_markdown_files_get_the_markdown_surface() {
        for path in ["README.md", "dir/notes.MARKDOWN", "a.MD"] {
            assert!(supports(path));
        }
        for path in ["file.rs", "file.md.png", "file.mdx", "README"] {
            assert!(!supports(path));
        }
    }
    #[test]
    fn visual_is_the_default_and_large_sources_remain_bounded() {
        assert_eq!(Mode::default(), Mode::Visual);
        assert!(preview_allowed("# Heading\n\ntext"));
        assert!(!preview_allowed(&"x".repeat(128 * 1024 + 1)));
        assert!(!preview_allowed(&"line\n".repeat(2001)));
    }
}
