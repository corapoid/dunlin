//! Source-editor inline blame. Cache is scoped to tab/backend/root/path/HEAD;
//! only exact equal diff runs receive attribution, never edited or inserted lines.
use crate::app::DunlinApp;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
    },
    *,
};
use dunlin_workspace::git::blame::{Blame, Line};
use similar::{DiffTag, TextDiff};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct State {
    busy: bool,
    request: uuid::Uuid,
    last_check: Option<Instant>,
    scope: Option<String>,
    path: Option<String>,
    value: Option<Arc<Blame>>,
    error: Option<String>,
    mapped_text: Option<String>,
    mapping: Vec<Option<usize>>,
    map_request: uuid::Uuid,
    pending_map: Option<String>,
}
impl State {
    pub fn invalidate(&mut self) {
        self.request = uuid::Uuid::now_v7();
        self.busy = false;
        self.last_check = None;
        self.value = None;
        self.path = None;
        self.mapped_text = None;
        self.mapping.clear();
        self.map_request = uuid::Uuid::now_v7();
        self.pending_map = None;
    }
}
fn map_lines(head: &str, current: &str) -> Vec<Option<usize>> {
    let mut mapping = vec![None; current.split('\n').count()];
    for op in TextDiff::configure()
        .timeout(Duration::from_millis(60))
        .diff_lines(head, current)
        .ops()
    {
        if op.tag() == DiffTag::Equal {
            for (old, new) in op.old_range().zip(op.new_range()) {
                if let Some(line) = mapping.get_mut(new) {
                    *line = Some(old);
                }
            }
        }
    }
    mapping
}
fn raw_editor(this: &DunlinApp) -> bool {
    this.workbench.show_file
        && this.active_file_tab().is_some_and(|t| {
            t.kind == dunlin_workspace::preview::PreviewKind::Text
                && (!crate::markdown_editor::supports(&t.inspector.path)
                    || !t.markdown.preview_only())
        })
}
fn scope_current(this: &DunlinApp, tab: &crate::workbench::Tab) -> bool {
    if tab.root != this.project().path
        || tab.project != this.project().id
        || !this.project().trusted
    {
        return false;
    }
    match (&tab.backend, &this.project().remote) {
        (crate::remote::WorkspaceBackend::Local { root }, None) => root == &tab.root,
        (crate::remote::WorkspaceBackend::Remote(r), Some(p)) => {
            r.project.host_id == p.host_id
                && r.project.project_id == p.project_id
                && r.project.path == p.path
                && this.remote_hosts.iter().any(|h| {
                    h.id == r.host.id && h.url == r.host.url && h.device_id == r.host.device_id
                })
        }
        _ => false,
    }
}
impl DunlinApp {
    pub(crate) fn toggle_inline_blame(&mut self, cx: &mut Context<Self>) {
        self.inline_blame = !self.inline_blame;
        for tab in &mut self.workbench.tabs {
            tab.blame.invalidate();
        }
        self.persist_settings(cx);
        cx.notify();
    }
    pub(crate) fn refresh_inline_blame(&mut self, cx: &mut Context<Self>) {
        if !self.inline_blame || !raw_editor(self) {
            return;
        }
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| !t.loading && scope_current(self, t))
        else {
            return;
        };
        if tab.blame.path.as_ref() != Some(&tab.inspector.path) {
            let id = tab.id.clone();
            let path = tab.inspector.path.clone();
            let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
            tab.blame.invalidate();
            tab.blame.path = Some(path);
        }
        let tab = self.active_file_tab().unwrap();
        let text = tab.editor.read(cx).text().to_string();
        if tab.blame.mapped_text.as_ref() != Some(&text)
            && tab.blame.pending_map.as_ref() != Some(&text)
            && let Some(value) = tab.blame.value.clone()
        {
            let id = tab.id.clone();
            let request = uuid::Uuid::now_v7();
            let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
            tab.blame.map_request = request;
            tab.blame.pending_map = Some(text.clone());
            cx.spawn(async move |this, cx| {
                let head = value.head.clone();
                let (text, mapping) = cx
                    .background_executor()
                    .spawn(async move {
                        let mapping = map_lines(&value.contents, &text);
                        (text, mapping)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let valid = this
                        .workbench
                        .tabs
                        .iter()
                        .find(|t| t.id == id)
                        .is_some_and(|t| this.inline_blame && scope_current(this, t));
                    if let Some(tab) = this
                        .workbench
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == id && t.blame.map_request == request)
                    {
                        tab.blame.pending_map = None;
                        if valid
                            && tab.blame.value.as_ref().is_some_and(|v| v.head == head)
                            && *tab.editor.read(cx).text() == text
                        {
                            tab.blame.mapped_text = Some(text);
                            tab.blame.mapping = mapping;
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let tab = self.active_file_tab().unwrap();
        if tab.blame.busy
            || tab
                .blame
                .last_check
                .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        let id = tab.id.clone();
        let path = tab.inspector.path.clone();
        let root = tab.root.clone();
        let project = tab.project.clone();
        let backend = tab.backend.clone();
        let scope = tab.blame.scope.clone();
        let previous = tab.blame.value.clone();
        let request = uuid::Uuid::now_v7();
        let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.blame.request = request;
        tab.blame.busy = true;
        tab.blame.last_check = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let history = backend.history_load("HEAD", 1, scope.as_deref())?;
                    let head = &history.value.first().ok_or("No HEAD commit")?.oid;
                    if let Some(value) = previous.filter(|v| v.head == *head) {
                        return Ok::<_, String>((history.scope, value));
                    }
                    let result = backend.blame(&path, Some(&history.scope), Some(head))?;
                    Ok((result.scope, Arc::new(result.value)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let valid = this
                    .workbench
                    .tabs
                    .iter()
                    .find(|t| t.id == id && t.project == project && t.root == root)
                    .is_some_and(|t| this.inline_blame && scope_current(this, t));
                let Some(tab) = this
                    .workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id && t.blame.request == request)
                else {
                    return;
                };
                tab.blame.busy = false;
                if valid {
                    match result {
                        Ok((scope, value)) => {
                            tab.blame.scope = Some(scope);
                            tab.blame.value = Some(value);
                            tab.blame.error = None;
                            tab.blame.mapped_text = None;
                        }
                        Err(error) => {
                            tab.blame.error = Some(error);
                            tab.blame.value = None;
                            tab.blame.mapping.clear();
                        }
                    }
                }
                cx.notify();
            });
            // Wake a visible editor for a bounded HEAD recheck without relying on
            // typing/file watcher events. No full blame reload if HEAD is unchanged.
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
    pub(crate) fn open_blame_commit(
        &mut self,
        oid: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_git_history(window, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        if let Some(view) = self.git_history.view.as_ref() {
            view.revision
                .update(cx, |s, cx| s.set_value(oid, window, cx));
        }
        self.load_git_history(cx);
    }
}
fn current<'a>(this: &'a DunlinApp, cx: &App) -> Option<(&'a Blame, &'a Line)> {
    if !raw_editor(this) {
        return None;
    }
    let tab = this.active_file_tab()?;
    if !this.inline_blame
        || !scope_current(this, tab)
        || tab.blame.path.as_ref() != Some(&tab.inspector.path)
    {
        return None;
    }
    let value = tab.blame.value.as_ref()?;
    // Never paint cached author mappings during the frame in which text changed.
    if tab
        .blame
        .mapped_text
        .as_deref()
        .is_none_or(|text| *tab.editor.read(cx).text() != text)
    {
        return None;
    }
    let row = tab.editor.read(cx).cursor_position().line as usize;
    let index = tab.blame.mapping.get(row).copied().flatten()?;
    Some((value, value.lines.get(index)?))
}
#[cfg(debug_assertions)]
pub(crate) fn smoke_attribution(this: &DunlinApp, cx: &App) -> Option<String> {
    current(this, cx).map(|(_, line)| line.author.clone())
}
fn date(line: &Line) -> String {
    chrono::DateTime::from_timestamp(line.timestamp, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown date".into())
}
fn label(line: &Line) -> String {
    format!("{} · {}", line.author, date(line))
}
fn info(value: &Blame, line: &Line) -> String {
    format!(
        "{}\n{} · {}\n{}\n{}:{}\nSnapshot HEAD {}\nClick to inspect this commit (read-only)",
        line.oid,
        line.author.escape_debug(),
        date(line),
        line.summary.escape_debug(),
        line.original_path.escape_debug(),
        line.original_line,
        value.head
    )
}
pub(crate) fn toggle(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Button {
    let enabled = this.inline_blame;
    dunlin_ui::button("editor-inline-blame-toggle")
        .ghost()
        .small()
        .rounded(px(4.))
        .icon(assets::IconName::GitCommitHorizontal)
        .accessibility_label(if enabled {
            "Disable inline blame"
        } else {
            "Enable inline blame"
        })
        .when(enabled, |button| button.bg(palette(cx).accent))
        .tooltip("Inline blame for unchanged lines in all source files")
        .on_click(cx.listener(|this, _, _, cx| this.toggle_inline_blame(cx)))
}
pub(crate) fn overlay(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let Some((value, line)) = current(this, cx) else {
        return div().into_any_element();
    };
    let editor = this.editor.read(cx);
    let text = editor.text().to_string();
    let cursor = editor.cursor();
    let end = text[cursor..].find('\n').map_or(text.len(), |n| cursor + n);
    let end = if text.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r') {
        end - 1
    } else {
        end
    };
    let Some(bounds) = editor.range_to_bounds(&(end..end)) else {
        return div().into_any_element();
    };
    let Some(caret) = editor.range_to_bounds(&(cursor..cursor)) else {
        return div().into_any_element();
    };
    let input = editor.input_bounds();
    let available = input.right() - bounds.right() - px(24.);
    if available < px(170.) || caret.top() != bounds.top() {
        return div().into_any_element();
    }
    let tooltip = info(value, line);
    let oid = line.oid.clone();
    let line_height = editor.line_height().unwrap_or(px(20.));
    div()
        .absolute()
        .left(bounds.right() - input.left() + px(24.))
        .top(bounds.top() - input.top())
        .max_w(available)
        .overflow_hidden()
        .child(
            dunlin_ui::button("editor-inline-blame")
                .ghost()
                .small()
                .h(line_height)
                .rounded(px(2.))
                .text_size(px(10.5))
                .text_color(palette(cx).muted_fg.opacity(0.6))
                .label(label(line))
                .tooltip(tooltip)
                .on_click(
                    cx.listener(move |this, _, w, cx| this.open_blame_commit(oid.clone(), w, cx)),
                ),
        )
        .into_any_element()
}
pub(crate) fn status(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Option<AnyElement> {
    let tab = this.active_file_tab()?;
    if !raw_editor(this) || !this.inline_blame {
        return None;
    }
    let p = palette(cx);
    let content = if let Some((value, line)) = current(this, cx) {
        let oid = line.oid.clone();
        dunlin_ui::button("blame-current-line-details")
            .ghost()
            .small()
            .h(px(22.))
            .text_size(px(10.5))
            .text_color(p.muted_fg.opacity(0.75))
            .label(label(line))
            .tooltip(info(value, line))
            .on_click(cx.listener(move |this, _, w, cx| this.open_blame_commit(oid.clone(), w, cx)))
            .into_any_element()
    } else {
        div()
            .text_size(px(11.))
            .child(if tab.blame.busy && tab.blame.value.is_none() {
                "Reading HEAD blame…".to_string()
            } else if let Some(e) = &tab.blame.error {
                format!("Blame unavailable: {e}")
            } else {
                "Uncommitted / changed line — no author attribution".into()
            })
            .into_any_element()
    };
    Some(
        div()
            .h(px(26.))
            .px(px(10.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .text_color(p.muted_fg.opacity(0.75))
            .border_t_1()
            .border_color(p.border)
            .child(content)
            .into_any_element(),
    )
}
#[cfg(test)]
mod tests {
    use super::{label, map_lines};
    use dunlin_workspace::git::blame::Line;

    #[test]
    fn label_shows_author_and_local_modification_date_not_commit() {
        let timestamp = 1_700_000_000;
        let mut line = Line {
            oid: "a".repeat(40),
            author: "Test Author".into(),
            timestamp,
            summary: "Commit subject".into(),
            original_line: 1,
            original_path: "file.rs".into(),
            text: "text".into(),
        };
        let expected_date = chrono::DateTime::from_timestamp(timestamp, 0)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M");
        assert_eq!(label(&line), format!("Test Author · {expected_date}"));
        line.timestamp = i64::MAX;
        assert_eq!(label(&line), "Test Author · Unknown date");
    }

    #[test]
    fn never_attributes_inserted_or_modified_lines_and_recovers_after_undo() {
        let head = "one\ntwo\nthree\n";
        let current = "one\nunsaved\nchanged\nthree\n";
        assert_eq!(
            map_lines(head, current),
            vec![Some(0), None, None, Some(2), None]
        );
        assert_eq!(map_lines(head, head), vec![Some(0), Some(1), Some(2), None]);
    }
    #[test]
    fn unicode_crlf_and_deletions_map_only_exact_equal_runs() {
        assert_eq!(
            map_lines("żółw\r\ntwo\r\nthree", "żółw\r\nthree"),
            vec![Some(0), Some(2)]
        );
        assert_eq!(map_lines("one\n", "changed\n"), vec![None, None]);
    }
}
