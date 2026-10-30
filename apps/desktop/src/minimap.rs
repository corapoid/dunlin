//! Bounded, asynchronous file overview. Git marks compare to HEAD; a separate
//! blue rail tracks unsaved edits against the saved buffer, including Undo.
use crate::app::DunlinApp;
use dunlin_ui::*;
use similar::{DiffTag, TextDiff};
use std::{ops::Range, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Added,
    Modified,
    Deleted,
}
#[derive(Clone, Debug)]
struct Mark {
    lines: Range<usize>,
    kind: Kind,
}
#[derive(Clone)]
struct Row {
    line: usize,
    indent: usize,
    width: usize,
    comment: bool,
}
#[derive(Default)]
pub(crate) struct Snapshot {
    rows: Vec<Row>,
    lines: usize,
    changes: Vec<Mark>,
    unsaved: Vec<Mark>,
}
#[derive(Default)]
pub(crate) struct State {
    revision: u64,
    rendered: Option<u64>,
    busy: bool,
    baseline: Option<Option<Arc<String>>>,
    snapshot: Arc<Snapshot>,
}
impl State {
    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn invalidate_baseline(&mut self) {
        self.baseline = None;
        self.invalidate();
    }
}
fn marks(before: &str, after: &str, lines: usize) -> Vec<Mark> {
    if before == after {
        return Vec::new();
    }
    TextDiff::configure()
        .timeout(Duration::from_millis(60))
        .diff_lines(before, after)
        .ops()
        .iter()
        .filter_map(|op| {
            let kind = match op.tag() {
                DiffTag::Equal => return None,
                DiffTag::Insert => Kind::Added,
                DiffTag::Delete => Kind::Deleted,
                DiffTag::Replace => Kind::Modified,
            };
            let range = op.new_range();
            let start = range.start.min(lines.saturating_sub(1));
            Some(Mark {
                lines: start..range.end.max(start + 1).min(lines),
                kind,
            })
        })
        .collect()
}
fn snapshot(current: &str, saved: &str, baseline: Option<&str>) -> Snapshot {
    let lines = current.split('\n').count().max(1);
    let stride = lines.div_ceil(1200).max(1);
    let rows = current
        .split('\n')
        .enumerate()
        .filter(|(line, _)| line % stride == 0)
        .map(|(line, text)| {
            let indent = text
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .take(100)
                .count();
            let width = text.chars().take(160).count().saturating_sub(indent);
            Row {
                line,
                indent,
                width,
                comment: ["//", "#", "/*", "*"]
                    .iter()
                    .any(|prefix| text.trim_start().starts_with(prefix)),
            }
        })
        .collect();
    Snapshot {
        rows,
        lines,
        changes: baseline
            .map(|before| marks(before, current, lines))
            .unwrap_or_default(),
        unsaved: marks(saved, current, lines),
    }
}
fn map_height(lines: usize, height: f32) -> f32 {
    (lines.max(1) as f32 * 2.).min(height).max(1.)
}
fn line_at(y: f32, height: f32, lines: usize) -> usize {
    ((y.max(0.) / map_height(lines, height)) * lines.max(1) as f32)
        .floor()
        .min(lines.saturating_sub(1) as f32) as usize
}

impl DunlinApp {
    pub(crate) fn refresh_minimap(&mut self, cx: &mut Context<Self>) {
        if !self.editor_minimap || !self.workbench.show_file {
            return;
        }
        let Some(tab) = self.active_file_tab() else {
            return;
        };
        if tab.kind != dunlin_workspace::preview::PreviewKind::Text
            || tab.loading
            || tab.minimap.busy
            || tab.minimap.rendered == Some(tab.minimap.revision)
        {
            return;
        }
        let id = tab.id.clone();
        let revision = tab.minimap.revision;
        let text = tab.editor.read(cx).text().clone();
        let saved = tab.saved_text.clone().unwrap_or_default();
        let previous = tab.minimap.baseline.clone();
        let backend = tab.backend.clone();
        let path = tab.inspector.path.clone();
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            tab.minimap.busy = true;
        }
        cx.spawn(async move |this, cx| {
            let (view, baseline) = cx
                .background_executor()
                .spawn(async move {
                    let baseline =
                        previous.unwrap_or_else(|| backend.editor_baseline(&path).map(Arc::new));
                    let view = snapshot(
                        &text.to_string(),
                        &saved,
                        baseline.as_deref().map(String::as_str),
                    );
                    (Arc::new(view), baseline)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(tab) = this.workbench.tabs.iter_mut().find(|t| t.id == id) {
                    tab.minimap.busy = false;
                    if tab.minimap.revision == revision {
                        tab.minimap.snapshot = view;
                        tab.minimap.baseline = Some(baseline);
                        tab.minimap.rendered = Some(revision);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn invalidate_minimap_baselines(&mut self) {
        let project = self.project().id.clone();
        for tab in self
            .workbench
            .tabs
            .iter_mut()
            .filter(|t| t.project == project)
        {
            tab.minimap.invalidate_baseline();
        }
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let editor = this.editor.clone();
    let snapshot = this
        .active_file_tab()
        .map(|tab| tab.minimap.snapshot.clone())
        .unwrap_or_default();
    let hit_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let paint_bounds = hit_bounds.clone();
    let click_editor = editor.clone();
    let click_snapshot = snapshot.clone();
    div()
        .id("editor-minimap")
        .aria_label("File map: Git changes and unsaved edits; click to navigate")
        .w(px(100.))
        .h_full()
        .flex_shrink_0()
        .relative()
        .overflow_hidden()
        .border_l_1()
        .border_color(p.border)
        .bg(p.subtle)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let bounds = hit_bounds.get();
            let line = line_at(
                f32::from(event.position.y - bounds.origin.y),
                f32::from(bounds.size.height),
                click_snapshot.lines,
            );
            click_editor.update(cx, |state, cx| {
                let row = state.display_map().buffer_line_to_display_row(line);
                let line_height = state.line_height().unwrap_or(px(20.));
                let mut offset = state.scroll_offset();
                offset.y =
                    -(line_height * row as f32 - state.input_bounds().size.height / 2.).max(px(0.));
                state.set_scroll_offset(offset, cx);
                state.focus(window, cx);
            });
        })
        .child(
            canvas(
                move |bounds, _, _| {
                    paint_bounds.set(bounds);
                    bounds
                },
                move |bounds, _, window, cx| {
                    let height = map_height(snapshot.lines, f32::from(bounds.size.height));
                    let scale = height / snapshot.lines.max(1) as f32;
                    for row in &snapshot.rows {
                        if row.width == 0 {
                            continue;
                        }
                        let x = 9. + row.indent.min(70) as f32 * 0.65;
                        let width = (row.width as f32 * 0.65).min(84. - x).max(1.);
                        let rect = Bounds::new(
                            bounds.origin + point(px(x), px(row.line as f32 * scale)),
                            size(px(width), px(scale.clamp(0.5, 1.4))),
                        );
                        window.paint_quad(fill(
                            rect,
                            if row.comment {
                                p.green.opacity(0.5)
                            } else {
                                p.muted_fg.opacity(0.4)
                            },
                        ));
                    }
                    for (marks, x) in [(&snapshot.changes, 0.), (&snapshot.unsaved, 4.)] {
                        for mark in marks {
                            let color = if x > 0. {
                                p.blue
                            } else {
                                match mark.kind {
                                    Kind::Added => p.green,
                                    Kind::Modified => p.amber,
                                    Kind::Deleted => p.red,
                                }
                            };
                            let top = (mark.lines.start as f32 * scale).min((height - 2.).max(0.));
                            let rect = Bounds::new(
                                bounds.origin + point(px(x), px(top)),
                                size(
                                    px(3.),
                                    px(((mark.lines.end - mark.lines.start) as f32 * scale)
                                        .max(2.)),
                                ),
                            );
                            window.paint_quad(fill(rect, color));
                        }
                    }
                    let state = editor.read(cx);
                    if let Some(visible) = state.visible_row_range() {
                        let start = state
                            .display_map()
                            .display_row_to_buffer_line(visible.start)
                            .min(snapshot.lines.saturating_sub(1));
                        let end = state
                            .display_map()
                            .display_row_to_buffer_line(
                                visible
                                    .end
                                    .saturating_sub(1)
                                    .min(state.display_map().display_row_count().saturating_sub(1)),
                            )
                            .saturating_add(1)
                            .min(snapshot.lines);
                        let top = start as f32 * scale;
                        let viewport = Bounds::new(
                            bounds.origin + point(px(7.), px(top)),
                            size(
                                (bounds.size.width - px(7.)).max(px(0.)),
                                px(((end.saturating_sub(start)) as f32 * scale)
                                    .max(4.)
                                    .min((height - top).max(0.))),
                            ),
                        );
                        window.paint_quad(fill(viewport, p.blue.opacity(0.12)));
                    }
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Kind, line_at, snapshot};
    #[test]
    fn separate_git_and_unsaved_marks_follow_additions_modifications_deletions_and_undo() {
        let view = snapshot("one\nchanged\nnew\n", "one\nchanged\n", Some("one\ntwo\n"));
        assert!(
            view.changes
                .iter()
                .any(|m| m.kind == Kind::Modified && m.lines.contains(&1))
        );
        assert!(
            view.unsaved
                .iter()
                .any(|m| m.kind == Kind::Added && m.lines.contains(&2))
        );
        let view = snapshot("one\n", "one\n", Some("one\ntwo\n"));
        assert!(view.unsaved.is_empty());
        assert!(view.changes.iter().any(|m| m.kind == Kind::Deleted));
        assert!(snapshot("one\n", "one\n", Some("one\n")).changes.is_empty());
    }
    #[test]
    fn overview_and_click_targets_stay_bounded_for_empty_short_and_large_documents() {
        assert_eq!(line_at(-10., 300., 0), 0);
        assert_eq!(line_at(9999., 300., 10), 9);
        assert_eq!(line_at(150., 300., 10000), 5000);
        let text = "  line\n".repeat(10000);
        let view = snapshot(&text, &text, None);
        assert!(view.rows.len() <= 1200);
        assert!(view.changes.is_empty());
    }
    #[test]
    fn minimap_setting_migrates_and_persists_without_resetting_other_editor_preferences() {
        let legacy: crate::app::StoredSettings =
            serde_json::from_str(r#"{"word_wrap":true,"autosave":true}"#).unwrap();
        assert!(legacy.editor_minimap && legacy.word_wrap && legacy.autosave);
        let settings = crate::app::StoredSettings {
            editor_minimap: false,
            ..legacy
        };
        let restored: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!restored.editor_minimap);
        assert!(restored.word_wrap && restored.autosave);
    }
}
