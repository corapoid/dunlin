//! Cached directory rows preserve the backend's immutable file indices.
use super::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{Icon, button::ButtonCustomVariant};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: String,
    depth: usize,
    file: Option<usize>,
}

pub(super) struct State {
    tree: bool,
    collapsed: BTreeSet<String>,
    tree_rows: Vec<Row>,
    flat_rows: Vec<Row>,
    visible: Vec<Row>,
    scroll: UniformListScrollHandle,
    focused: usize,
    focus: FocusHandle,
}
impl State {
    pub(super) fn new(cx: &mut Context<DunlinApp>) -> Self {
        Self {
            tree: true,
            collapsed: BTreeSet::new(),
            tree_rows: Vec::new(),
            flat_rows: Vec::new(),
            visible: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            focused: 0,
            focus: cx.focus_handle(),
        }
    }
    pub(super) fn clear(&mut self) {
        self.collapsed.clear();
        self.tree_rows.clear();
        self.flat_rows.clear();
        self.visible.clear();
        self.focused = 0;
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    }
    pub(super) fn install(&mut self, files: &[CommitFile]) {
        self.tree_rows = tree_rows(files);
        self.flat_rows = files
            .iter()
            .enumerate()
            .map(|(i, f)| Row {
                path: f.path.clone(),
                depth: 0,
                file: Some(i),
            })
            .collect();
        self.rebuild();
    }
    fn rebuild(&mut self) {
        self.visible = if self.tree {
            visible_rows(&self.tree_rows, &self.collapsed)
        } else {
            self.flat_rows.clone()
        };
        self.focused = self.focused.min(self.visible.len().saturating_sub(1));
    }
    fn toggle(&mut self, path: String) {
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.rebuild();
    }
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<String, Directory>,
    files: Vec<(String, usize)>,
}

fn tree_rows(files: &[CommitFile]) -> Vec<Row> {
    let mut root = Directory::default();
    for (index, file) in files.iter().enumerate() {
        let mut parts = file.path.split('/').peekable();
        let mut directory = &mut root;
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                directory = directory.directories.entry(part.into()).or_default();
            } else {
                directory.files.push((file.path.clone(), index));
            }
        }
    }
    fn visit(dir: Directory, prefix: &str, depth: usize, rows: &mut Vec<Row>) {
        for (name, child) in dir.directories {
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            rows.push(Row {
                path: path.clone(),
                depth,
                file: None,
            });
            visit(child, &path, depth + 1, rows);
        }
        let mut files = dir.files;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        rows.extend(files.into_iter().map(|(path, index)| Row {
            path,
            depth,
            file: Some(index),
        }));
    }
    let mut rows = Vec::new();
    visit(root, "", 0, &mut rows);
    rows
}

fn visible_rows(rows: &[Row], collapsed: &BTreeSet<String>) -> Vec<Row> {
    let mut hidden_depth = None;
    rows.iter()
        .filter(|row| {
            if hidden_depth.is_some_and(|depth| row.depth > depth) {
                return false;
            }
            hidden_depth =
                (row.file.is_none() && collapsed.contains(&row.path)).then_some(row.depth);
            true
        })
        .cloned()
        .collect()
}

fn status_label(status: &str) -> &'static str {
    match status.as_bytes().first() {
        Some(b'A') => "Added",
        Some(b'D') => "Deleted",
        Some(b'R') => "Renamed",
        Some(b'C') => "Copied",
        Some(b'T') => "Type changed",
        _ => "Modified",
    }
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let p = palette(cx);
    let count = view.detail.as_ref().map_or(0, |d| d.files.len());
    let mut heading = div()
        .h(px(34.))
        .px(px(12.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(10.))
        .text_color(p.muted_fg)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("CHANGED FILES"),
        )
        .child(if view.detail_loading {
            "…".into()
        } else {
            count.to_string()
        })
        .child(div().flex_1());
    for (tree, label) in [(false, "Flat"), (true, "Tree")] {
        heading = heading.child(
            dunlin_ui::button(format!("history-files-{label}"))
                .ghost()
                .small()
                .rounded(px(dunlin_ui::radii::CODE))
                .label(label)
                .when(tree == view.inspector.files.tree, |b| {
                    b.bg(p.hover).text_color(p.fg)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = &mut this.git_history.view {
                        v.inspector.files.tree = tree;
                        v.inspector.files.rebuild();
                        v.inspector
                            .files
                            .scroll
                            .scroll_to_item_strict(0, ScrollStrategy::Top);
                    }
                    cx.notify();
                })),
        );
    }
    let list = uniform_list(
        "commit-file-list",
        view.inspector.files.visible.len(),
        cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
            let Some(view) = &this.git_history.view else {
                return Vec::new();
            };
            let Some(detail) = &view.detail else {
                return Vec::new();
            };
            range
                .filter_map(|i| view.inspector.files.visible.get(i).map(|row| (i, row)))
                .map(|(i, row)| {
                    let path = row.path.clone();
                    let file = row.file.and_then(|index| detail.files.get(index));
                    let selected = row.file.is_some() && row.file == view.selected_file;
                    let focused = i == view.inspector.files.focused
                        && view.inspector.files.focus.is_focused(window);
                    let collapsed = view.inspector.files.collapsed.contains(&row.path);
                    let (icon, icon_color) = file
                        .map(|f| crate::file_icons::appearance(&f.path, &p))
                        .unwrap_or((IconName::Folder, p.muted_fg));
                    let labels = file.map(file_labels);
                    let label = if view.inspector.files.tree {
                        row.path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&row.path)
                            .escape_debug()
                            .to_string()
                    } else {
                        labels.as_ref().map(|l| l.name.clone()).unwrap_or_default()
                    };
                    let tooltip = labels
                        .as_ref()
                        .map(|l| l.full.clone())
                        .unwrap_or_else(|| row.path.escape_debug().to_string());
                    let index = row.file;
                    dunlin_ui::action_row(
                        ("commit-file", i),
                        format!("{}{}", tooltip, if collapsed { ", collapsed" } else { "" }),
                    )
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(if selected {
                                p.selection
                            } else {
                                transparent_black()
                            })
                            .foreground(p.fg)
                            .hover(if selected { p.selection } else { p.hover })
                            .active(p.selection),
                    )
                    .tab_index(-1)
                    .rounded(px(dunlin_ui::radii::SQUARE))
                    .py(px(0.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pr(px(12.))
                    .pl(px(10. + row.depth as f32 * 14.))
                    .min_w(px(0.))
                    .border_l_2()
                    .border_color(if selected || focused {
                        p.blue
                    } else {
                        transparent_black()
                    })
                    .when(focused, |r| r.bg(p.hover))
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(v) = &this.git_history.view {
                            v.inspector.files.focus.focus(window, cx);
                        }
                        if let Some(index) = index {
                            this.select_history_file(index, cx);
                            if let Some(v) = &mut this.git_history.view {
                                v.inspector.tab = Tab::Patch;
                                v.inspector.files.focused = i;
                            }
                        } else if let Some(v) = &mut this.git_history.view {
                            v.inspector.files.focused = i;
                            v.inspector.files.toggle(path.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(10.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .child(if file.is_some() {
                                ""
                            } else if collapsed {
                                "›"
                            } else {
                                "⌄"
                            }),
                    )
                    .child(
                        Icon::new(icon)
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(icon_color),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(12.))
                            .when_some(file, |r, f| r.text_color(status_color(&f.status, &p)))
                            .child(label),
                    )
                    .when(!view.inspector.files.tree, |r| {
                        r.child(
                            div()
                                .min_w(px(0.))
                                .max_w(relative(0.4))
                                .truncate()
                                .text_size(px(10.))
                                .text_color(p.muted_fg)
                                .child(labels.map(|l| l.directory).unwrap_or_default()),
                        )
                    })
                    .when_some(file, |r, f| {
                        r.child(
                            div()
                                .w(px(76.))
                                .flex_shrink_0()
                                .text_right()
                                .text_size(px(10.))
                                .text_color(status_color(&f.status, &p))
                                .child(status_label(&f.status)),
                        )
                    })
                    .into_any_element()
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&view.inspector.files.scroll)
    .flex_1()
    .min_h(px(0.))
    .min_w(px(0.));
    div()
        .id("history-file-tree")
        .track_focus(&view.inspector.files.focus)
        .tab_index(0)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            let Some(view) = &mut this.git_history.view else {
                return;
            };
            let files = &mut view.inspector.files;
            if !files.focus.is_focused(window)
                || event.keystroke.modifiers.platform
                || files.visible.is_empty()
            {
                return;
            }
            let Some(row) = files.visible.get(files.focused).cloned() else {
                return;
            };
            let mut open = None;
            match event.keystroke.key.as_str() {
                "up" => files.focused = files.focused.saturating_sub(1),
                "down" => files.focused = (files.focused + 1).min(files.visible.len() - 1),
                "home" => files.focused = 0,
                "end" => files.focused = files.visible.len() - 1,
                "right" if files.tree => {
                    if row.file.is_none() && files.collapsed.remove(&row.path) {
                        files.rebuild();
                    } else {
                        files.focused = (files.focused + 1).min(files.visible.len() - 1);
                    }
                }
                "left" if files.tree => {
                    if row.file.is_none() && !files.collapsed.contains(&row.path) {
                        files.toggle(row.path);
                    } else if let Some((parent, _)) = row.path.rsplit_once('/')
                        && let Some(i) = files
                            .visible
                            .iter()
                            .position(|r| r.file.is_none() && r.path == parent)
                    {
                        files.focused = i;
                    }
                }
                "enter" | "space" => {
                    if let Some(index) = row.file {
                        open = Some(index);
                    } else {
                        files.toggle(row.path);
                    }
                }
                _ => return,
            }
            files
                .scroll
                .scroll_to_item(files.focused, ScrollStrategy::Nearest);
            if let Some(index) = open {
                this.select_history_file(index, cx);
                if let Some(v) = &mut this.git_history.view {
                    v.inspector.tab = Tab::Patch;
                }
            }
            cx.stop_propagation();
            cx.notify();
        }))
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(heading)
        .child(
            div()
                .h(px(24.))
                .px(px(12.))
                .flex_shrink_0()
                .flex()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .border_b_1()
                .border_color(p.border)
                .child(div().flex_1().child("Path"))
                .child("Status"),
        )
        .when(view.detail_loading, |r| {
            r.child(notice("Loading changed files…", p.muted_fg))
        })
        .when_some(view.detail_error.as_ref(), |r, error| {
            r.child(notice(error, p.red))
        })
        .when(
            !view.detail_loading && view.detail_error.is_none() && count == 0,
            |r| r.child(notice("No changed files against this parent.", p.muted_fg)),
        )
        .child(list)
        .child(
            div()
                .h(px(26.))
                .px(px(12.))
                .flex_shrink_0()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child("Select a file to preview its immutable commit diff."),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{BTreeSet, CommitFile, tree_rows, visible_rows};
    fn file(path: &str) -> CommitFile {
        CommitFile {
            status: "M".into(),
            path: path.into(),
            old_path: None,
        }
    }
    #[test]
    fn sorted_tree_preserves_original_patch_indices_and_literal_paths() {
        let rows = tree_rows(&[
            file("src/z.rs"),
            file("README.md"),
            file("src/a\nb.rs"),
            file("assets/a.svg"),
        ]);
        let files: Vec<_> = rows
            .iter()
            .filter_map(|r| r.file.map(|i| (r.path.as_str(), i)))
            .collect();
        assert_eq!(
            files,
            [
                ("assets/a.svg", 3),
                ("src/a\nb.rs", 2),
                ("src/z.rs", 0),
                ("README.md", 1)
            ]
        );
    }
    #[test]
    fn collapse_hides_descendants_not_similarly_named_siblings() {
        let rows = tree_rows(&[file("src/a/b.rs"), file("src2/c.rs"), file("root.rs")]);
        let visible = visible_rows(&rows, &BTreeSet::from(["src".into()]));
        assert_eq!(
            visible.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["src", "src2", "src2/c.rs", "root.rs"]
        );
    }
}
