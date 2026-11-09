//! Persistent native editors: focus moves the caret, never swaps a rendered
//! label for a differently sized input. Source remains the history authority.
use super::{
    Format, Mode,
    document::{Block, Document, Kind},
    editable, editing, selection, structure, table,
};
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{
        ActiveTheme, Disableable, Sizable,
        input::{
            Backspace, Delete, Editor, EditorState, Enter, IndentInline, InputEvent, OutdentInline,
            Redo, TextDecoration, TextDecorationCollection, Undo,
        },
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{cell::Cell, ops::Range, rc::Rc, sync::Arc};

#[derive(Default)]
pub(super) struct State {
    pub document: Option<Document>,
    pub active: Option<Active>,
    pub editors: Vec<Active>,
    pub error: Option<String>,
    pub selection: Option<selection::Selection>,
    pub drag_anchor: Option<selection::Point>,
    pub preferred_x: Option<Pixels>,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Palette mode used for the current editor decorations. A theme switch
    /// must rebuild inline-code/link backgrounds or light-mode code spans keep
    /// a light background under dark-mode text.
    pub decorations_dark: Option<bool>,
}
#[derive(Clone)]
pub(super) struct Active {
    pub block: Block,
    pub snapshot: Arc<str>,
    pub editor: Entity<EditorState>,
    decorations: TextDecorationCollection,
    _subscription: Rc<Subscription>,
    _observation: Rc<Subscription>,
    pub(super) selection_style: TextDecorationCollection,
    pub(super) viewport: Rc<Cell<Bounds<Pixels>>>,
}

#[derive(Clone)]
pub(super) struct TableTarget {
    pub table: usize,
    pub row: usize,
    pub column: usize,
    pub columns: usize,
    pub align: markdown::mdast::AlignKind,
    snapshot: Arc<str>,
}
impl State {
    pub fn sync(&mut self, source: &str) {
        if self.document.as_ref().is_some_and(|d| d.source == source) {
            return;
        }
        self.selection = None;
        self.drag_anchor = None;
        match Document::parse(source) {
            Ok(doc) => {
                self.document = Some(doc);
                self.error = None;
            }
            Err(error) => {
                self.document = None;
                self.error = Some(error);
            }
        }
    }
}

fn decorations(block: &Block, p: &Palette) -> Vec<TextDecoration> {
    block
        .leaves
        .iter()
        .map(|leaf| {
            TextDecoration::new(
                leaf.visible.clone(),
                HighlightStyle {
                    font_weight: leaf.marks.bold.then_some(FontWeight::BOLD),
                    font_style: leaf.marks.italic.then_some(FontStyle::Italic),
                    strikethrough: leaf.marks.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(p.muted_fg),
                    }),
                    background_color: leaf.marks.code.then_some(p.muted),
                    color: leaf.marks.link.then_some(p.blue),
                    ..Default::default()
                },
            )
        })
        .collect()
}

impl DunlinApp {
    pub(super) fn indent_visual_container(
        &mut self,
        outdent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)
        else {
            return false;
        };
        let Some(active) = &tab.markdown.visual.active else {
            return false;
        };
        let Some(index) = tab
            .markdown
            .visual
            .editors
            .iter()
            .position(|e| e.editor == active.editor)
        else {
            return false;
        };
        if active.block.containers.is_empty() || active.block.is_literal() {
            return false;
        }
        if !editable(self, tab) {
            return true;
        }
        let Some(doc) = &tab.markdown.visual.document else {
            return true;
        };
        if doc.source != *tab.editor.read(cx).text() {
            return true;
        }
        if tab.markdown.visual.selection.is_some_and(|s| !s.is_empty()) {
            self.file_notice("Indent one list item or quote subtree at a time.", cx);
            return true;
        }
        let caret = active.editor.read(cx).cursor();
        let edit = match super::containers::indent(doc, index, outdent) {
            Ok(Some(edit)) => edit,
            Ok(None) => return false,
            Err(e) => {
                self.file_notice(e, cx);
                return true;
            }
        };
        let editor = tab.editor.clone();
        let id = tab.id.clone();
        self.clear_visual_selection(cx);
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
        });
        self.workbench
            .tabs
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .markdown
            .visual
            .document = Some(edit.document);
        self.begin_visual_block(edit.focus, Some(caret..caret), window, cx);
        true
    }
    fn edit_visual_structure(
        &mut self,
        entity: EntityId,
        operation: structure::Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.replace_visual_selection("", window, cx, Some(operation)) {
            return true;
        }
        self.apply_visual_change(entity, window, cx);
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)
        else {
            return false;
        };
        if !editable(self, tab) {
            return true;
        }
        let Some(index) = tab
            .markdown
            .visual
            .editors
            .iter()
            .position(|e| e.editor.entity_id() == entity)
        else {
            return false;
        };
        let active = &tab.markdown.visual.editors[index];
        let source = tab.editor.read(cx).text().to_string();
        let Some(document) = &tab.markdown.visual.document else {
            return true;
        };
        if source != active.snapshot.as_ref() || document.source != source {
            self.file_notice(
                "The source changed. Select the text again before splitting or merging.",
                cx,
            );
            return true;
        }
        let selection = active.editor.read(cx).selected_range();
        let edit = match structure::edit(document, index, selection, operation) {
            Ok(Some(edit)) => edit,
            Ok(None) => return false,
            Err(error) => {
                self.file_notice(error, cx);
                return true;
            }
        };
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
        });
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            tab.markdown.visual.document = Some(edit.document);
        }
        self.begin_visual_block(edit.focus, Some(edit.caret..edit.caret), window, cx);
        cx.notify();
        true
    }

    pub(super) fn visual_table_target(&self) -> Option<TableTarget> {
        let tab = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)?;
        let active = tab.markdown.visual.active.as_ref()?;
        let Kind::Cell {
            table,
            row,
            column,
            columns,
            align,
        } = active.block.kind
        else {
            return None;
        };
        Some(TableTarget {
            table,
            row,
            column,
            columns,
            align,
            snapshot: active.snapshot.clone(),
        })
    }

    pub(super) fn edit_visual_table(
        &mut self,
        target: TableTarget,
        operation: table::Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(entity) = self.active_file_tab().and_then(|t| {
            t.markdown
                .visual
                .active
                .as_ref()
                .map(|a| a.editor.entity_id())
        }) {
            self.apply_visual_change(entity, window, cx);
        }
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| editable(self, t) && t.markdown.mode == Mode::Visual)
        else {
            return;
        };
        let source = tab.editor.read(cx).text().to_string();
        let current = self.visual_table_target();
        if source != target.snapshot.as_ref()
            || !current.is_some_and(|c| {
                c.table == target.table
                    && c.row == target.row
                    && c.column == target.column
                    && c.columns == target.columns
            })
        {
            self.file_notice(
                "The table or selected cell changed. Reopen the Table menu.",
                cx,
            );
            return;
        }
        let edit = match table::edit(&source, target.table, target.row, target.column, operation) {
            Ok(edit) => edit,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let mut next = source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        if next == source {
            return;
        }
        let document = match Document::parse(&next) {
            Ok(document) => document,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let Some(block) = document.blocks.iter().find(|b| matches!(b.kind, Kind::Cell { table, row, column, .. } if table == target.table && row == edit.row && column == edit.column)).cloned() else {
            self.file_notice("This operation would change the surrounding table structure. The source is unchanged.", cx); return;
        };
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
        });
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            tab.markdown.visual.document = Some(document);
        }
        self.begin_visual_block(block.clone(), Some(0..block.text.len()), window, cx);
        cx.notify();
    }

    fn navigate_visual_table(
        &mut self,
        entity: EntityId,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_visual_change(entity, window, cx);
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| editable(self, t) && t.markdown.mode == Mode::Visual)
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
        let active = &tab.markdown.visual.editors[index];
        let Kind::Cell {
            table,
            row: old_row,
            ..
        } = active.block.kind
        else {
            return false;
        };
        let next = if backwards {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        let block = next
            .and_then(|i| tab.markdown.visual.editors.get(i))
            .filter(|e| matches!(e.block.kind, Kind::Cell { table: t, .. } if t == table))
            .map(|e| e.block.clone());
        if let Some(block) = block {
            self.begin_visual_block(block.clone(), Some(0..block.text.len()), window, cx);
        } else if !backwards {
            if let Some(target) = self.visual_table_target() {
                self.edit_visual_table(target, table::Operation::RowAfter, window, cx);
            }
            // Appending selects the first cell of the new row, not the old column.
            if let Some(target) = self.visual_table_target()
                && target.row > old_row
            {
                let block = self.active_file_tab().and_then(|t| t.markdown.visual.document.as_ref()).and_then(|d| d.blocks.iter().find(|b| matches!(b.kind, Kind::Cell { table: id, row, column: 0, .. } if id == table && row == target.row))).cloned();
                if let Some(block) = block {
                    self.begin_visual_block(block.clone(), Some(0..block.text.len()), window, cx);
                }
            }
        } else {
            // Shift+Tab at the first cell leaves the table without mutating it.
            let previous = index
                .checked_sub(1)
                .and_then(|i| tab.markdown.visual.editors.get(i))
                .map(|e| e.block.clone());
            if let Some(block) = previous {
                self.begin_visual_block(
                    block.clone(),
                    Some(block.text.len()..block.text.len()),
                    window,
                    cx,
                );
            } else {
                self.focus_handle.focus(window, cx);
            }
        }
        true
    }

    /// Run before rendering the workbench, with its native window available.
    pub(crate) fn prepare_markdown_visual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.paint_visual_selection(cx);
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual && !t.loading)
        else {
            return;
        };
        let Some(doc) = &tab.markdown.visual.document else {
            return;
        };
        if *tab.editor.read(cx).text() != doc.source {
            return;
        }
        let dark = cx.theme().is_dark();
        let theme_changed = tab.markdown.visual.decorations_dark != Some(dark);
        if !theme_changed
            && tab.markdown.visual.editors.len() == doc.blocks.len()
            && tab
                .markdown
                .visual
                .editors
                .iter()
                .zip(&doc.blocks)
                .all(|(editor, block)| {
                    editor.block.raw == block.raw
                        && editor.block.kind == block.kind
                        && editor.snapshot.as_ref() == doc.source
                })
        {
            // A background native window can acquire a focus handle without
            // emitting InputEvent::Focus until it becomes active. Track the
            // real focused field as well, so toolbar/undo use the right block.
            let focused = tab
                .markdown
                .visual
                .editors
                .iter()
                .find(|editor| editor.editor.read(cx).focus_handle(cx).is_focused(window))
                .cloned();
            if let Some(focused) = focused {
                let id = tab.id.clone();
                self.workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .markdown
                    .visual
                    .active = Some(focused);
            }
            return;
        }
        let source: Arc<str> = Arc::from(doc.source.as_str());
        let blocks = doc.blocks.clone();
        let id = tab.id.clone();
        let old = std::mem::take(
            &mut self
                .workbench
                .tabs
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .markdown
                .visual
                .editors,
        );
        let mut available = old.into_iter().map(Some).collect::<Vec<_>>();
        let mut editors = Vec::with_capacity(blocks.len());
        for block in blocks {
            let existing = available
                .iter_mut()
                .find(|candidate| {
                    candidate.as_ref().is_some_and(|old| {
                        old.block.raw.start == block.raw.start
                            && *old.editor.read(cx).text() == block.text
                    })
                })
                .and_then(Option::take);
            let mut editor = existing.unwrap_or_else(|| {
                self.new_visual_editor(block.clone(), source.clone(), window, cx)
            });
            editor.block = block;
            editor.snapshot = source.clone();
            editor
                .decorations
                .set(decorations(&editor.block, &palette(cx)), cx);
            editors.push(editor);
        }
        let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.markdown.visual.active = tab
            .markdown
            .visual
            .active
            .as_ref()
            .and_then(|active| editors.iter().find(|e| e.editor == active.editor).cloned());
        tab.markdown.visual.editors = editors;
        tab.markdown.visual.decorations_dark = Some(dark);
    }

    fn new_visual_editor(
        &mut self,
        block: Block,
        source: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Active {
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("plaintext")
                .line_number(false)
                .soft_wrap(true)
                .scroll_beyond_last_line(None);
            // Blocks grow with their content; their own scrollbar and wheel
            // handling only create a second, confusing scroll target. The
            // document owns scrolling.
            state.set_scrollbar_visible(false, cx);
            state.set_scrollable(false, cx);
            state
        });
        let collection = editor.update(cx, |state, cx| {
            state.set_value(block.text.clone(), window, cx);
            state.create_decorations_collection(decorations(&block, &palette(cx)), cx)
        });
        let selection_style = editor.update(cx, |state, cx| {
            state.create_decorations_collection(Vec::new(), cx)
        });
        let subscription = cx.subscribe_in(
            &editor,
            window,
            |this, editor, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.apply_visual_change(editor.entity_id(), window, cx),
                InputEvent::Focus => {
                    if let Some(id) = this.active_file_tab().map(|t| t.id.clone())
                        && let Some(tab) = this.workbench.tabs.iter_mut().find(|t| t.id == id)
                    {
                        tab.markdown.visual.active = tab
                            .markdown
                            .visual
                            .editors
                            .iter()
                            .find(|a| a.editor == *editor)
                            .cloned();
                    }
                }
                _ => {}
            },
        );
        let observation = cx.observe(&editor, |_, _, cx| cx.notify());
        Active {
            block,
            snapshot: source,
            editor,
            decorations: collection,
            _subscription: Rc::new(subscription),
            _observation: Rc::new(observation),
            selection_style,
            viewport: Rc::new(Cell::new(Bounds::default())),
        }
    }

    pub(super) fn begin_visual_block(
        &mut self,
        block: Block,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_visual_selection(cx);
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| !t.loading && t.markdown.mode == Mode::Visual)
        else {
            return;
        };
        let id = tab.id.clone();
        self.prepare_markdown_visual(window, cx);
        let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Some(active) = tab
            .markdown
            .visual
            .editors
            .iter()
            .find(|a| a.block.raw == block.raw && a.block.kind == block.kind)
            .cloned()
        else {
            return;
        };
        active.editor.update(cx, |state, cx| {
            let end = state.text().len();
            state.set_selected_range(selection.unwrap_or(end..end), cx);
            state.focus(window, cx);
        });
        tab.markdown.visual.active = Some(active);
        cx.notify();
    }

    pub(super) fn apply_visual_change(
        &mut self,
        entity: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)
        else {
            return;
        };
        let id = tab.id.clone();
        let source_editor = tab.editor.clone();
        let Some(active) = tab
            .markdown
            .visual
            .editors
            .iter()
            .find(|a| a.editor.entity_id() == entity)
        else {
            return;
        };
        let live = source_editor.read(cx).text().to_string();
        let new = active.editor.read(cx).text().to_string();
        if new == active.block.text {
            return;
        }
        if !editable(self, tab) {
            let original = active.block.text.clone();
            let editor = active.editor.clone();
            editor.update(cx, |state, cx| state.set_value(original, window, cx));
            self.file_notice("Editing is currently locked. The source is unchanged.", cx);
            return;
        }
        if tab
            .markdown
            .visual
            .selection
            .is_some_and(|selection| !selection.is_empty())
        {
            let (_, inserted) = super::document::difference(&active.block.text, &new);
            let inserted = inserted.to_string();
            let editor = active.editor.clone();
            let original = active.block.text.clone();
            editor.update(cx, |state, cx| state.set_value(original, window, cx));
            self.replace_visual_selection(&inserted, window, cx, None);
            return;
        }
        let result = if live == active.snapshot.as_ref() {
            active.block.replace_visible(&live, &new)
        } else {
            Err("The source changed. Reopen the block before editing.")
        };
        let replacement = match result {
            Ok(value) => value,
            Err(error) => {
                let original = active.block.text.clone();
                let editor = active.editor.clone();
                editor.update(cx, |state, cx| state.set_value(original, window, cx));
                self.file_notice(error, cx);
                return;
            }
        };
        let range = active.block.raw.clone();
        let editor = active.editor.clone();
        let mut source = live;
        source.replace_range(range.clone(), &replacement);
        let mut doc = match Document::parse(&source) {
            Ok(doc) => doc,
            Err(error) => {
                let original = active.block.text.clone();
                editor.update(cx, |state, cx| state.set_value(original, window, cx));
                self.file_notice(error, cx);
                return;
            }
        };
        let block = doc
            .blocks
            .iter()
            .find(|b| b.raw.start == range.start && b.editable && b.text == new)
            .cloned()
            .or_else(|| {
                let end = range.start + replacement.len();
                let parts = doc
                    .blocks
                    .iter()
                    .filter(|b| b.raw.start >= range.start && b.raw.end <= end)
                    .cloned()
                    .collect::<Vec<_>>();
                Block::join(&source, range.start..end, active.block.kind, &parts)
                    .filter(|b| b.text == new)
            })
            .or_else(|| new.is_empty().then(|| Block::empty(range.start)));
        let Some(block) = block else {
            let original = active.block.text.clone();
            editor.update(cx, |state, cx| state.set_value(original, window, cx));
            self.file_notice(
                "This edit would change the surrounding block structure. The source is unchanged.",
                cx,
            );
            return;
        };
        if !doc.blocks.iter().any(|b| b.raw == block.raw) {
            doc.blocks
                .retain(|b| b.raw.end <= block.raw.start || b.raw.start >= block.raw.end);
            doc.blocks.push(block.clone());
            doc.blocks.sort_by_key(|b| b.raw.start);
        }
        let styles = decorations(&block, &palette(cx));
        if let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) {
            let delta = replacement.len() as isize - range.len() as isize;
            for editor in &mut tab.markdown.visual.editors {
                if editor.editor.entity_id() != entity && editor.block.raw.start >= range.end {
                    editor.block.raw.start =
                        editor.block.raw.start.checked_add_signed(delta).unwrap();
                    editor.block.raw.end = editor.block.raw.end.checked_add_signed(delta).unwrap();
                }
            }
            let active = tab
                .markdown
                .visual
                .editors
                .iter_mut()
                .find(|a| a.editor.entity_id() == entity)
                .unwrap();
            active.block = block;
            active.snapshot = Arc::from(source.as_str());
            active.decorations.set(styles, cx);
            tab.markdown.visual.active = Some(active.clone());
            tab.markdown.visual.document = Some(doc);
        }
        source_editor.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(replacement, window, cx);
        });
        self.prepare_markdown_visual(window, cx);
    }

    pub(super) fn format_visual_markdown(
        &mut self,
        format: Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_file_tab().filter(|t| editable(self, t)) else {
            return;
        };
        if tab.markdown.visual.selection.is_some_and(|s| !s.is_empty()) {
            self.file_notice("Select text within one block to apply formatting. Cross-block text replacement remains available.",cx);
            return;
        }
        let Some(active) = &tab.markdown.visual.active else {
            self.file_notice("Click a text block and select text to format it.", cx);
            return;
        };
        let source = tab.editor.read(cx).text().to_string();
        if source != active.snapshot.as_ref() {
            self.file_notice("The source changed. Reopen the block.", cx);
            return;
        }
        let Some(range) = active
            .block
            .source_selection(active.editor.read(cx).selected_range())
        else {
            self.file_notice(
                "Formatting applies to prose. Code and syntax fields are edited literally.",
                cx,
            );
            return;
        };
        let anchor = active.block.raw.start;
        let inserted_at = range.start;
        let edit = match editing::edit(&source, range, format) {
            Ok(edit) => edit,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let editor = tab.editor.clone();
        let id = tab.id.clone();
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.replacement, window, cx);
            state.set_selected_range(edit.selection, cx);
        });
        self.refresh_visual_document(&id, anchor, window, cx);
        if matches!(format, Format::Link | Format::Image)
            && let Some(block) = self
                .active_file_tab()
                .and_then(|t| t.markdown.visual.document.as_ref())
                .and_then(|d| {
                    d.blocks
                        .iter()
                        .find(|b| b.raw.start <= inserted_at && b.raw.end > inserted_at)
                })
                .cloned()
        {
            let caret = block
                .leaves
                .iter()
                .find(|l| l.raw.start >= inserted_at)
                .map_or(0, |l| l.visible.start);
            self.begin_visual_block(block, Some(caret..caret), window, cx);
            self.open_visual_resource(window, cx);
        }
    }

    fn refresh_visual_document(
        &mut self,
        id: &str,
        anchor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workbench.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let source = tab.editor.read(cx).text().to_string();
        tab.markdown.visual.document = None;
        tab.markdown.visual.sync(&source);
        let block = tab
            .markdown
            .visual
            .document
            .as_ref()
            .and_then(|d| {
                d.blocks
                    .iter()
                    .find(|b| b.raw.start == anchor && b.editable)
                    .or_else(|| {
                        d.blocks
                            .iter()
                            .rev()
                            .find(|b| b.editable && b.raw.start <= anchor)
                    })
                    .or_else(|| d.blocks.iter().find(|b| b.editable))
            })
            .cloned();
        if let Some(block) = block {
            self.begin_visual_block(block, None, window, cx);
        }
        cx.notify();
    }

    pub(super) fn visual_history(
        &mut self,
        redo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_visual_selection(cx);
        let Some(tab) = self.active_file_tab().filter(|t| editable(self, t)) else {
            return;
        };
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        let anchor = tab
            .markdown
            .visual
            .active
            .as_ref()
            .map_or(0, |a| a.block.raw.start);
        let focus = editor.read(cx).focus_handle(cx);
        if redo {
            focus.dispatch_action(&Redo, window, cx);
        } else {
            focus.dispatch_action(&Undo, window, cx);
        }
        self.refresh_visual_document(&id, anchor, window, cx);
    }

    fn append_visual_paragraph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_file_tab().filter(|t| editable(self, t)) else {
            return;
        };
        let editor = tab.editor.clone();
        let id = tab.id.clone();
        let text = editor.read(cx).text().to_string();
        let suffix = if text.is_empty() || text.ends_with("\n\n") {
            ""
        } else if text.contains("\r\n") {
            "\r\n\r\n"
        } else {
            "\n\n"
        };
        let at = text.len();
        if !suffix.is_empty() {
            editor.update(cx, |state, cx| {
                state.set_selected_range(at..at, cx);
                state.replace(suffix, window, cx);
            });
        }
        self.refresh_visual_document(&id, at + suffix.len(), window, cx);
    }

    fn toggle_visual_task(
        &mut self,
        offset: usize,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_file_tab().filter(|t| editable(self, t)) else {
            return;
        };
        let source = tab.editor.read(cx).text().to_string();
        if !matches!(source.as_bytes().get(offset), Some(b' ' | b'x' | b'X')) {
            return;
        }
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset + 1, cx);
            state.replace(if checked { " " } else { "x" }, window, cx);
        });
        self.refresh_visual_document(&id, offset, window, cx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Typography {
    size: f32,
    bold: bool,
    code: bool,
}

fn typography(block: &Block) -> Typography {
    let size = match block.kind {
        Kind::Heading(1) => 20.,
        Kind::Heading(2) => 17.,
        Kind::Heading(3) => 15.,
        Kind::Heading(4) => 14.,
        Kind::Heading(_) => 13.,
        _ if block.is_literal() => 12.,
        _ => 13.,
    };
    Typography {
        size,
        bold: matches!(block.kind, Kind::Heading(_) | Kind::Cell { row: 0, .. }),
        code: block.is_literal(),
    }
}

fn editor_element(active: &Active, readonly: bool, cx: &mut Context<DunlinApp>) -> AnyElement {
    let style = typography(&active.block);
    let rows = active
        .editor
        .read(cx)
        .display_map()
        .wrap_row_count()
        .max(active.block.text.lines().count())
        .max(1);
    let height = rows as f32 * (style.size * 1.4).ceil();
    let entity = active.editor.clone();
    let entity_id = active.editor.entity_id();
    let viewport = active.viewport.clone();
    let is_cell = matches!(active.block.kind, Kind::Cell { .. });
    let is_prose = !active.block.is_literal() && !is_cell;
    let app = cx.entity().downgrade();
    let align = match active.block.kind {
        Kind::Cell {
            align: markdown::mdast::AlignKind::Center,
            ..
        } => TextAlign::Center,
        Kind::Cell {
            align: markdown::mdast::AlignKind::Right,
            ..
        } => TextAlign::Right,
        _ => TextAlign::Left,
    };
    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        // EditorState has a two-row native minimum. Clip its empty trailing
        // row rather than letting its paint/hitboxes overlap the next block.
        .h(px(height))
        .overflow_hidden()
        .capture_action(cx.listener(move |this, _: &IndentInline, window, cx| {
            if (is_cell && this.navigate_visual_table(entity_id, false, window, cx))
                || (!is_cell && this.indent_visual_container(false, window, cx))
            {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |this, _: &OutdentInline, window, cx| {
            if (is_cell && this.navigate_visual_table(entity_id, true, window, cx))
                || (!is_cell && this.indent_visual_container(true, window, cx))
            {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |this, action: &Enter, window, cx| {
            if is_prose
                && !action.secondary
                && !action.shift
                && this.edit_visual_structure(entity_id, structure::Operation::Split, window, cx)
            {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |this, _: &Backspace, window, cx| {
            if is_prose
                && this.edit_visual_structure(entity_id, structure::Operation::Backward, window, cx)
            {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |this, _: &Delete, window, cx| {
            if is_prose
                && this.edit_visual_structure(entity_id, structure::Operation::Forward, window, cx)
            {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::component::input::MoveLeft, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Left,
                    false,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::component::input::MoveRight, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Right,
                    false,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::component::input::MoveUp, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Up,
                    false,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::component::input::MoveDown, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Down,
                    false,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::base::actions::SelectLeft, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Left,
                    true,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::base::actions::SelectRight, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Right,
                    true,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::base::actions::SelectUp, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Up,
                    true,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .capture_action(cx.listener(
            move |this, _: &dunlin_ui::base::actions::SelectDown, window, cx| {
                if this.move_visual_caret(
                    entity_id,
                    super::interaction::Direction::Down,
                    true,
                    window,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            },
        ))
        .child(
            Editor::new(&active.editor)
                .xsmall()
                .appearance(false)
                .bordered(false)
                .readonly(readonly)
                .font_family(if style.code {
                    dunlin_ui::theme::code_font()
                } else {
                    dunlin_ui::theme::ui_font()
                })
                .text_size(px(style.size))
                .line_height(relative(1.4))
                .font_weight(if style.bold {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_align(align)
                .h(px(height))
                .w_full()
                .context_menu(|menu, _, _| menu)
                .aria_label(match active.block.kind {
                    Kind::Cell { .. } => "Markdown table cell",
                    Kind::Code => "Markdown code",
                    Kind::Html => "HTML content",
                    Kind::Metadata => "Document metadata",
                    _ => "Visual Markdown text",
                }),
        )
        .child(
            canvas(
                move |bounds, _, _| {
                    viewport.set(bounds);
                },
                move |_, _, window, cx| {
                    // Wrapping is known only after native text layout. Grow on text/width
                    // changes, never on focus; the next frame uses this same editor entity.
                    let next = entity.read(cx).display_map().wrap_row_count().max(1);
                    if next != rows {
                        let app = app.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = app.update(cx, |_, cx| cx.notify());
                        });
                        window.request_animation_frame();
                    }
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
        .into_any_element()
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let tab = this.active_file_tab().unwrap();
    let p = palette(cx);
    let state = &tab.markdown.visual;
    let document_bounds = state.bounds.clone();
    let mut root = div()
        .id("markdown-visual-document")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .track_scroll(&tab.markdown.scroll)
        .px(px(20.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .capture_action(cx.listener(|this, _: &Undo, window, cx| {
            this.visual_history(false, window, cx);
            cx.stop_propagation();
        }))
        .capture_action(cx.listener(|this, _: &Redo, window, cx| {
            this.visual_history(true, window, cx);
            cx.stop_propagation();
        }))
        .capture_action(
            cx.listener(|this, _: &dunlin_ui::component::input::Copy, _, cx| {
                if this.copy_visual_selection(false, None, cx) {
                    cx.stop_propagation();
                }
            }),
        )
        .capture_action(
            cx.listener(|this, _: &dunlin_ui::component::input::Cut, window, cx| {
                if this.copy_visual_selection(true, Some(window), cx) {
                    cx.stop_propagation();
                }
            }),
        )
        .capture_action(
            cx.listener(|this, _: &dunlin_ui::component::input::Paste, window, cx| {
                if this.paste_visual_selection(window, cx) {
                    cx.stop_propagation();
                }
            }),
        )
        .capture_action(cx.listener(
            |this, _: &dunlin_ui::component::input::SelectAll, window, cx| {
                this.select_visual_document(window, cx);
                cx.stop_propagation();
            },
        ))
        .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
            if this.visual_pointer_down(event, window, cx) {
                cx.stop_propagation();
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            this.visual_pointer_move(event, window, cx);
        }))
        .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, _, _| {
            if event.button == MouseButton::Left
                && let Some(id) = this.active_file_tab().map(|t| t.id.clone())
                && let Some(tab) = this.workbench.tabs.iter_mut().find(|t| t.id == id)
            {
                tab.markdown.visual.drag_anchor = None;
            }
        }))
        .child(
            canvas(
                move |_, window, _| document_bounds.set(window.content_mask().bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .size_full(),
        );
    if let Some(error) = state.error.as_ref().or(tab.markdown.error.as_ref()) {
        return root
            .child(div().text_color(p.amber).child(error.clone()))
            .into_any_element();
    }
    let Some(doc) = &state.document else {
        return root.child("Rendering document…").into_any_element();
    };
    if *tab.editor.read(cx).text() != doc.source {
        return root.child("Updating document…").into_any_element();
    }
    if state.editors.len() != doc.blocks.len() {
        return root.child("Preparing document…").into_any_element();
    }
    let mut index = 0;
    while index < state.editors.len() {
        let active = &state.editors[index];
        let block = &active.block;
        if let Kind::Cell { table, .. } = block.kind {
            let mut grid = SurfaceGeometry::TABLE
                .paint(p.card, p.border)
                .w_full()
                .min_w(px(0.))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .overflow_hidden();
            while index < state.editors.len()
                && matches!(state.editors[index].block.kind, Kind::Cell { table: t, .. } if t == table)
            {
                let Kind::Cell {
                    row: current_row, ..
                } = state.editors[index].block.kind
                else {
                    unreachable!()
                };
                let mut row = div()
                    .w_full()
                    .min_w(px(0.))
                    .flex()
                    .when(current_row == 0, |row| {
                        SurfaceGeometry::TABLE
                            .round_child(row, SurfaceEdges::Top)
                            .bg(p.muted)
                    })
                    .when(current_row > 0, |row| {
                        row.border_t_1().border_color(p.border)
                    });
                while index < state.editors.len()
                    && matches!(state.editors[index].block.kind, Kind::Cell { table: t, row: r, .. } if t == table && r == current_row)
                {
                    let Kind::Cell {
                        column, columns, ..
                    } = state.editors[index].block.kind
                    else {
                        unreachable!()
                    };
                    row = row.child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .p(px(6.))
                            .when(column + 1 < columns, |cell| {
                                cell.border_r_1().border_color(p.border)
                            })
                            .child(editor_element(
                                &state.editors[index],
                                !editable(this, tab),
                                cx,
                            )),
                    );
                    index += 1;
                }
                grid = grid.child(row);
            }
            root = root.child(grid);
            continue;
        }
        let mut row = div()
            .id(("markdown-visual-block", index))
            .w_full()
            .min_w(px(0.))
            .flex_shrink_0()
            .flex()
            .gap(px(8.))
            .pl(px(block.containers.len().saturating_sub(1) as f32 * 18.))
            .when(
                block.containers.iter().any(|c| !c.list) && !matches!(block.kind, Kind::Quote),
                |row| row.border_l_2().border_color(p.border_strong),
            );
        match block.kind {
            Kind::Task(checked, offset) => {
                row = row.child(
                    dunlin_ui::checkbox(format!("visual-task-{index}"), "Task completion", checked)
                        .disabled(!editable(this, tab))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_visual_task(offset, checked, window, cx)
                        })),
                )
            }
            Kind::Bullet => {
                row = row.child(
                    div()
                        .pt(px(2.))
                        .text_size(px(13.))
                        .text_color(p.muted_fg)
                        .child("•"),
                )
            }
            Kind::Numbered(n) => {
                row = row.child(
                    div()
                        .pt(px(2.))
                        .text_size(px(13.))
                        .text_color(p.muted_fg)
                        .child(format!("{n}.")),
                )
            }
            Kind::Quote => row = row.border_l_2().border_color(p.border_strong).pl(px(12.)),
            _ => {}
        }
        if block.is_literal() {
            let label = match block.kind {
                Kind::Code => "Code",
                Kind::Html => "HTML",
                Kind::Metadata => "Metadata",
                Kind::Definition => "Link definition",
                Kind::Rule => "Separator",
                _ => "Content",
            };
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(8.))
                    .bg(p.muted)
                    .rounded(px(dunlin_ui::radii::CODE))
                    .child(div().text_size(px(10.)).text_color(p.muted_fg).child(label))
                    .child(editor_element(active, !editable(this, tab), cx)),
            );
        } else {
            row = row.child(editor_element(active, !editable(this, tab), cx));
        }
        root = root.child(row);
        index += 1;
    }
    root.child(
        div().flex().gap(px(4.)).child(
            dunlin_ui::action_button("visual-add-paragraph")
                .label("+ Paragraph")
                .disabled(!editable(this, tab))
                .on_click(
                    cx.listener(|this, _, window, cx| this.append_visual_paragraph(window, cx)),
                ),
        ),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::typography;
    use crate::markdown_editor::document::Document;

    #[test]
    fn document_typography_is_compact_with_a_bounded_heading_hierarchy() {
        let doc = Document::parse("# H1\n\n## H2\n\n### H3\n\nBody\n\n```\ncode\n```").unwrap();
        let styles = doc.blocks.iter().map(typography).collect::<Vec<_>>();
        assert_eq!(
            styles.iter().map(|style| style.size).collect::<Vec<_>>(),
            [20., 17., 15., 13., 12.]
        );
        assert!(styles[0].size < styles[3].size * 1.6);
        assert!(styles[..3].iter().all(|s| s.bold));
        assert!(!styles[3].bold && styles[4].code);
    }
}
