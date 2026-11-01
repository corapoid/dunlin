//! Document-wide selection and navigation over persistent native fields. Hit
//! testing uses the editor's own laid-out caret bounds (not guessed font widths).
use super::{
    Mode, editable,
    selection::{self, Point, Selection},
    structure,
    visual::Active,
};
use crate::app::DunlinApp;
use dunlin_ui::{component::input::TextDecoration, *};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy)]
pub(super) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

fn caret_at(active: &Active, position: dunlin_ui::Point<Pixels>, cx: &App) -> usize {
    let state = active.editor.read(cx);
    let mut offsets = active
        .block
        .text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    offsets.push(active.block.text.len());
    let rect = |index: usize| state.range_to_bounds(&(offsets[index]..offsets[index]));
    let Some(first) = rect(0) else {
        return state.cursor();
    };
    let height = state.line_height().unwrap_or(px(18.));
    let y = first.top() + ((position.y - first.top()) / height).floor().max(0.) * height;
    let start = offsets.partition_point(|offset| {
        state
            .range_to_bounds(&(*offset..*offset))
            .is_some_and(|b| b.top() < y)
    });
    let end = offsets.partition_point(|offset| {
        state
            .range_to_bounds(&(*offset..*offset))
            .is_some_and(|b| b.top() <= y)
    });
    let mut best = offsets[start.min(offsets.len() - 1)];
    let mut distance = f32::MAX;
    for (i, offset) in offsets
        .iter()
        .enumerate()
        .take(end.max(start + 1).min(offsets.len()))
        .skip(start.min(offsets.len() - 1))
    {
        if let Some(bounds) = rect(i) {
            let d = f32::from((position.x - bounds.left()).abs());
            if d < distance {
                distance = d;
                best = *offset;
            }
        }
    }
    best
}

impl DunlinApp {
    pub(super) fn clear_visual_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_file_tab().map(|t| t.id.clone())
            && let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id)
        {
            tab.markdown.visual.selection = None;
            for editor in &tab.markdown.visual.editors {
                editor.selection_style.clear(cx);
            }
        }
    }

    pub(super) fn paint_visual_selection(&self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_file_tab() else {
            return;
        };
        let p = palette(cx);
        for (i, editor) in tab.markdown.visual.editors.iter().enumerate() {
            let range = tab
                .markdown
                .visual
                .selection
                .and_then(|s| s.range(i, editor.block.text.len()))
                .filter(|r| !r.is_empty());
            editor.selection_style.set(
                range
                    .map(|r| {
                        TextDecoration::new(
                            r,
                            HighlightStyle {
                                background_color: Some(p.blue_soft),
                                ..Default::default()
                            },
                        )
                    })
                    .into_iter()
                    .collect(),
                cx,
            );
        }
    }

    pub(super) fn set_visual_selection(
        &mut self,
        selection: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_file_tab() else {
            return;
        };
        let Some(document) = &tab.markdown.visual.document else {
            return;
        };
        if selection.validate(document).is_err() {
            return;
        }
        let block = document.blocks[selection.head.block].clone();
        if selection.anchor.block == selection.head.block {
            self.begin_visual_block(
                block,
                Some(selection.anchor.offset..selection.head.offset),
                window,
                cx,
            );
            self.reveal_visual_point(selection.head, cx);
            return;
        }
        self.begin_visual_block(
            block,
            Some(selection.head.offset..selection.head.offset),
            window,
            cx,
        );
        let id = self.active_file_tab().unwrap().id.clone();
        let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.markdown.visual.selection = Some(selection);
        for editor in &tab.markdown.visual.editors {
            if !editor.editor.read(cx).selected_range().is_empty() {
                let cursor = editor.editor.read(cx).cursor();
                editor
                    .editor
                    .update(cx, |state, cx| state.set_selected_range(cursor..cursor, cx));
            }
        }
        self.reveal_visual_point(selection.head, cx);
        self.paint_visual_selection(cx);
        cx.notify();
    }

    fn reveal_visual_point(&self, head: Point, cx: &App) {
        let Some(tab) = self.active_file_tab() else {
            return;
        };
        let viewport = tab.markdown.visual.bounds.get();
        if let Some(editor) = tab.markdown.visual.editors.get(head.block) {
            let bounds = editor
                .editor
                .read(cx)
                .range_to_bounds(&(head.offset..head.offset))
                .unwrap_or(editor.viewport.get());
            let offset = tab.markdown.scroll.offset();
            let delta = if bounds.top() < viewport.top() {
                viewport.top() - bounds.top()
            } else if bounds.bottom() > viewport.bottom() {
                viewport.bottom() - bounds.bottom()
            } else {
                px(0.)
            };
            if viewport.size.height > px(0.) && delta != px(0.) {
                tab.markdown
                    .scroll
                    .set_offset(point(offset.x, offset.y + delta));
            }
        }
    }

    pub(super) fn replace_visual_selection(
        &mut self,
        inserted: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: Option<structure::Operation>,
    ) -> bool {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)
        else {
            return false;
        };
        let Some(selection) = tab.markdown.visual.selection.filter(|s| !s.is_empty()) else {
            return false;
        };
        if !editable(self, tab) {
            return true;
        }
        let Some(document) = &tab.markdown.visual.document else {
            return true;
        };
        if *tab.editor.read(cx).text() != document.source {
            self.file_notice("The source changed. Select the text again.", cx);
            return true;
        }
        let value = if matches!(action, Some(structure::Operation::Split)) {
            "\n\n"
        } else {
            inserted
        };
        let edit = match selection::replace(document, selection, value) {
            Ok(edit) => edit,
            Err(error) => {
                self.file_notice(error, cx);
                return true;
            }
        };
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        self.clear_visual_selection(cx);
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
        });
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            tab.markdown.visual.document = Some(edit.document);
            tab.markdown.visual.drag_anchor = None;
        }
        self.begin_visual_block(edit.focus, Some(edit.caret..edit.caret), window, cx);
        cx.notify();
        true
    }

    pub(super) fn copy_visual_selection(
        &mut self,
        cut: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.active_file_tab() else {
            return false;
        };
        let Some(selection) = tab.markdown.visual.selection.filter(|s| !s.is_empty()) else {
            return false;
        };
        let Some(document) = &tab.markdown.visual.document else {
            return true;
        };
        if *tab.editor.read(cx).text() != document.source {
            return true;
        }
        if let Ok(text) = selection.text(document) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            if cut && let Some(window) = window {
                self.replace_visual_selection("", window, cx, None);
            }
        }
        true
    }
    pub(super) fn paste_visual_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .active_file_tab()
            .is_some_and(|t| t.markdown.visual.selection.is_some_and(|s| !s.is_empty()))
        {
            return false;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_visual_selection(&text, window, cx, None);
        }
        true
    }
    pub(super) fn select_visual_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self
            .active_file_tab()
            .and_then(|t| t.markdown.visual.document.as_ref())
        else {
            return;
        };
        if let Some(last) = doc.blocks.last() {
            self.set_visual_selection(
                Selection {
                    anchor: Point {
                        block: 0,
                        offset: 0,
                    },
                    head: Point {
                        block: doc.blocks.len() - 1,
                        offset: last.text.len(),
                    },
                },
                window,
                cx,
            );
        }
    }

    pub(super) fn move_visual_caret(
        &mut self,
        entity: EntityId,
        direction: Direction,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)
        else {
            return false;
        };
        let Some(index) = tab
            .markdown
            .visual
            .editors
            .iter()
            .position(|e| e.editor.entity_id() == entity)
        else {
            return false;
        };
        let editor = &tab.markdown.visual.editors[index];
        let state = editor.editor.read(cx);
        let Some(doc) = &tab.markdown.visual.document else {
            return false;
        };
        let current = Point {
            block: index,
            offset: state.cursor(),
        };
        let global = tab.markdown.visual.selection;
        let native = state.selected_range();
        let anchor = global.map(|s| s.anchor).unwrap_or(Point {
            block: index,
            offset: if state.cursor() == native.end {
                native.start
            } else {
                native.end
            },
        });
        let head = global.map_or(current, |s| s.head);
        let backwards = matches!(direction, Direction::Left | Direction::Up);
        if !extend && let Some(selection) = global.filter(|s| !s.is_empty()) {
            let (a, b) = selection.ordered();
            let point = if backwards { a } else { b };
            self.set_visual_selection(
                Selection {
                    anchor: point,
                    head: point,
                },
                window,
                cx,
            );
            self.clear_visual_selection(cx);
            return true;
        }
        let vertical = matches!(direction, Direction::Up | Direction::Down);
        let (next, preferred) = if !vertical {
            if global.is_none() && !extend && !native.is_empty() {
                let id = tab.id.clone();
                self.workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .markdown
                    .visual
                    .preferred_x = None;
                return false;
            }
            (selection::step(doc, head, backwards), None)
        } else {
            let field = &tab.markdown.visual.editors[head.block];
            let field_state = field.editor.read(cx);
            let Some(caret) = field_state.range_to_bounds(&(head.offset..head.offset)) else {
                return false;
            };
            let x = tab.markdown.visual.preferred_x.unwrap_or(caret.left());
            let edge = if backwards { 0 } else { field.block.text.len() };
            let edge_y = field_state
                .range_to_bounds(&(edge..edge))
                .map(|r| r.top())
                .unwrap_or(caret.top());
            if global.is_none() && caret.top() != edge_y {
                let id = tab.id.clone();
                self.workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .markdown
                    .visual
                    .preferred_x = Some(x);
                return false;
            }
            if caret.top() != edge_y {
                let y = caret.top()
                    + if backwards {
                        -caret.size.height
                    } else {
                        caret.size.height
                    };
                (
                    Point {
                        block: head.block,
                        offset: caret_at(field, point(x, y + px(1.)), cx),
                    },
                    Some(x),
                )
            } else {
                let target = selection::vertical_block(doc, head.block, backwards);
                if let Some(mut target) = target {
                    if !matches!(field.block.kind, super::document::Kind::Cell { .. })
                        && let super::document::Kind::Cell { table, row, .. } =
                            doc.blocks[target].kind
                    {
                        let distance = |active: &Active| {
                            let b = active.viewport.get();
                            if x < b.left() {
                                b.left() - x
                            } else if x > b.right() {
                                x - b.right()
                            } else {
                                px(0.)
                            }
                        };
                        target=tab.markdown.visual.editors.iter().enumerate().filter(|(_,a)|matches!(a.block.kind,super::document::Kind::Cell{table:t,row:r,..} if t==table && r==row)).min_by(|(_,a),(_,b)|distance(a).partial_cmp(&distance(b)).unwrap_or(std::cmp::Ordering::Equal)).map_or(target,|(i,_)|i);
                    }
                    let field = &tab.markdown.visual.editors[target];
                    let pos = if backwards { field.block.text.len() } else { 0 };
                    let y = field
                        .editor
                        .read(cx)
                        .range_to_bounds(&(pos..pos))
                        .map(|r| r.top())
                        .unwrap_or(field.viewport.get().top());
                    (
                        Point {
                            block: target,
                            offset: caret_at(field, point(x, y + px(1.)), cx),
                        },
                        Some(x),
                    )
                } else {
                    (head, Some(x))
                }
            }
        };
        if extend && anchor.block == next.block {
            let block = doc.blocks[next.block].clone();
            let id = tab.id.clone();
            self.begin_visual_block(block, Some(anchor.offset..next.offset), window, cx);
            self.reveal_visual_point(next, cx);
            self.workbench
                .tabs
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .markdown
                .visual
                .preferred_x = preferred;
            return true;
        }
        self.set_visual_selection(
            Selection {
                anchor: if extend { anchor } else { next },
                head: next,
            },
            window,
            cx,
        );
        if !extend {
            self.clear_visual_selection(cx);
        }
        if let Some(id) = self.active_file_tab().map(|t| t.id.clone()) {
            self.workbench
                .tabs
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .markdown
                .visual
                .preferred_x = preferred;
        }
        true
    }

    fn visual_hit(
        &self,
        position: dunlin_ui::Point<Pixels>,
        nearest: bool,
        cx: &App,
    ) -> Option<Point> {
        let tab = self.active_file_tab()?;
        let index = tab
            .markdown
            .visual
            .editors
            .iter()
            .position(|e| e.viewport.get().contains(&position))
            .or_else(|| {
                nearest
                    .then(|| {
                        tab.markdown
                            .visual
                            .editors
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| {
                                let distance = |e: &Active| {
                                    let r = e.viewport.get();
                                    if position.y < r.top() {
                                        f32::from(r.top() - position.y)
                                    } else if position.y > r.bottom() {
                                        f32::from(position.y - r.bottom())
                                    } else {
                                        0.
                                    }
                                };
                                distance(a).total_cmp(&distance(b))
                            })
                            .map(|(i, _)| i)
                    })
                    .flatten()
            })?;
        Some(Point {
            block: index,
            offset: caret_at(&tab.markdown.visual.editors[index], position, cx),
        })
    }
    pub(super) fn visual_pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left {
            return false;
        }
        let Some(head) = self.visual_hit(event.position, false, cx) else {
            return false;
        };
        let Some(tab) = self.active_file_tab() else {
            return false;
        };
        let anchor = if event.modifiers.shift {
            tab.markdown
                .visual
                .selection
                .map(|s| s.anchor)
                .or_else(|| {
                    tab.markdown.visual.active.as_ref().and_then(|a| {
                        tab.markdown
                            .visual
                            .editors
                            .iter()
                            .position(|e| e.editor == a.editor)
                            .map(|block| Point {
                                block,
                                offset: a.editor.read(cx).cursor(),
                            })
                    })
                })
                .unwrap_or(head)
        } else {
            head
        };
        let id = tab.id.clone();
        self.clear_visual_selection(cx);
        let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.markdown.visual.drag_anchor = Some(anchor);
        tab.markdown.visual.preferred_x = None;
        if event.modifiers.shift {
            self.set_visual_selection(Selection { anchor, head }, window, cx);
            true
        } else {
            false
        }
    }
    pub(super) fn visual_pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(anchor) = self
            .active_file_tab()
            .and_then(|t| t.markdown.visual.drag_anchor)
        else {
            return;
        };
        let Some(head) = self.visual_hit(event.position, true, cx) else {
            return;
        };
        if anchor.block != head.block
            || self
                .active_file_tab()
                .is_some_and(|t| t.markdown.visual.selection.is_some())
        {
            self.set_visual_selection(Selection { anchor, head }, window, cx);
        }
    }
}
