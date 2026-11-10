//! Right-hand inspector: changed-file tree above details, patch and operations.
use super::*;
use dunlin_ui::component::tooltip::Tooltip;
use dunlin_workspace::git::history::CommitFile;

mod files;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Details,
    Patch,
    Operations,
}

pub(super) struct State {
    pub(super) tab: Tab,
    files: files::State,
}
impl State {
    pub(super) fn new(cx: &mut Context<DunlinApp>) -> Self {
        Self {
            tab: Tab::Details,
            files: files::State::new(cx),
        }
    }
    pub(super) fn clear(&mut self) {
        self.tab = Tab::Details;
        self.files.clear();
    }
    pub(super) fn install(&mut self, files: &[CommitFile]) {
        self.files.install(files);
    }
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(8).collect()
}

struct FileLabels {
    name: String,
    directory: String,
    full: String,
}
fn file_labels(file: &CommitFile) -> FileLabels {
    let (directory, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
    let full = file
        .old_path
        .as_ref()
        .map(|old| format!("{} → {}", old.escape_debug(), file.path.escape_debug()))
        .unwrap_or_else(|| file.path.escape_debug().to_string());
    FileLabels {
        name: name.escape_debug().to_string(),
        directory: file
            .old_path
            .as_ref()
            .map(|old| format!("from {}", old.escape_debug()))
            .unwrap_or_else(|| directory.escape_debug().to_string()),
        full,
    }
}
fn message_body<'a>(message: &'a str, subject: &str) -> &'a str {
    let (first, rest) = message.split_once('\n').unwrap_or((message, ""));
    if first.trim_end() == subject {
        rest.trim()
    } else {
        message.trim()
    }
}
fn status_color(status: &str, p: &Palette) -> Hsla {
    match status.as_bytes().first() {
        Some(b'A') => p.green,
        Some(b'D') => p.red,
        Some(b'R' | b'C') => p.blue,
        Some(b'T') => p.violet,
        _ => p.amber,
    }
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(view) = &this.git_history.view else {
        return div().into_any_element();
    };
    let Some(commit) = view.selected.and_then(|i| view.rows.get(i)) else {
        return div()
            .p(px(12.))
            .text_size(px(12.))
            .text_color(p.muted_fg)
            .child(if view.loading {
                "Loading…"
            } else {
                "Select a commit to inspect its changes."
            })
            .into_any_element();
    };
    let selected = view.selected.unwrap();
    let date = chrono::DateTime::from_timestamp(commit.timestamp, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d %b %Y · %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let copy_oid = commit.oid.clone();
    let full_subject = commit.subject.clone();
    let mut comparison = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.))
        .text_size(px(10.))
        .text_color(p.muted_fg);
    match commit.parents.as_slice() {
        [] => comparison = comparison.child("Initial commit · compared with empty tree"),
        [parent] => comparison = comparison.child(format!("Compared with {}", short_oid(parent))),
        parents => {
            comparison = comparison.child("Compare parent");
            for (index, parent) in parents.iter().enumerate() {
                comparison = comparison.child(
                    dunlin_ui::button(format!("history-parent-{index}"))
                        .ghost()
                        .small()
                        .label(format!("{} · {}", index + 1, short_oid(parent)))
                        .tooltip(format!("Compare with parent {}: {parent}", index + 1))
                        .when(index == view.parent, |button| {
                            button.bg(p.blue_soft).text_color(p.blue)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_history_commit(selected, index, cx)
                        })),
                );
            }
        }
    }
    let mut header = div()
        .px(px(12.))
        .py(px(8.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(4.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .id("commit-subject")
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(commit.subject.clone())
                .tooltip(move |window, cx| Tooltip::new(full_subject.clone()).build(window, cx)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child(
                    dunlin_ui::button("commit-copy-oid")
                        .ghost()
                        .small()
                        .px(px(2.))
                        .font_family(dunlin_ui::theme::code_font())
                        .label(short_oid(&commit.oid))
                        .tooltip(format!("Copy commit ID: {}", commit.oid))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_oid.clone()))
                        }),
                )
                .child(commit.author.clone())
                .child(date.clone()),
        )
        .child(comparison);
    if let Some(detail) = &view.detail {
        let body = message_body(&detail.message, &commit.subject);
        if !body.is_empty() {
            header = header.child(
                div()
                    .id("commit-message")
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(body.to_string()),
            );
        }
    }
    let count = view.detail.as_ref().map_or(0, |d| d.files.len());
    let patch: AnyElement = if view.detail_loading || view.patch_loading {
        notice("Loading comparison…", p.muted_fg)
    } else if let Some(error) = &view.detail_error {
        notice(error, p.red)
    } else if let Some(note) = &view.patch_note {
        div()
            .id("commit-patch-note")
            .size_full()
            .p(px(12.))
            .overflow_scroll()
            .font_family(dunlin_ui::theme::code_font())
            .text_size(px(11.))
            .child(note.clone())
            .into_any_element()
    } else if count == 0 {
        notice("No file changes against this parent.", p.muted_fg)
    } else {
        Diff::new(&view.diff)
            .line_number(true)
            .soft_wrap(false)
            .rounded_br(px(dunlin_ui::scale::WORKSPACE_RADIUS))
            .pb(px(dunlin_ui::scale::WORKSPACE_RADIUS))
            .into_any_element()
    };
    let metadata = div()
        .id("commit-metadata-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .child(header)
        .child(
            div()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .text_size(px(11.))
                .child(metadata_row("Commit", commit.oid.clone(), p))
                .child(metadata_row("Author", commit.author.clone(), p))
                .child(metadata_row("Committed", date, p))
                .child(metadata_row(
                    "Refs",
                    if commit.refs.is_empty() {
                        "—".into()
                    } else {
                        commit.refs.clone()
                    },
                    p,
                ))
                .child(metadata_row(
                    "Parents",
                    if commit.parents.is_empty() {
                        "Initial commit".into()
                    } else {
                        commit.parents.join("\n")
                    },
                    p,
                )),
        )
        .when(view.detail_loading, |r| {
            r.child(notice("Loading commit details…", p.muted_fg))
        })
        .when_some(view.detail_error.as_ref(), |r, error| {
            r.child(notice(error, p.red))
        })
        .into_any_element();
    let content = match view.inspector.tab {
        Tab::Details => metadata,
        Tab::Patch => patch,
        Tab::Operations => div()
            .id("history-operations-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .child(
                div().p(px(16.)).child(
                    dunlin_ui::settings::button("history-branch-form")
                        .self_start()
                        .label(if view.branch_form_open {
                            "Hide branch form"
                        } else {
                            "Create branch…"
                        })
                        .disabled(this.git_history.running || view.loading)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.branch_form_open = !v.branch_form_open;
                            }
                            cx.notify();
                        })),
                ),
            )
            .when(view.branch_form_open, |r| {
                r.child(
                    div()
                        .p(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .border_b_1()
                        .border_color(p.border)
                        .child("Create branch from selected commit")
                        .child(
                            dunlin_ui::settings::input(&view.name, p)
                                .w_full()
                                .min_w(px(0.))
                                .aria_label("New branch name"),
                        )
                        .child(
                            dunlin_ui::settings::button("history-create")
                                .self_start()
                                .primary()
                                .label("Create branch")
                                .disabled(this.git_history.running || view.loading)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.create_history_branch(cx)),
                                ),
                        )
                        .child(
                            div().text_color(p.muted_fg).child(
                                "Creates a ref only; no checkout or changes to files/staging.",
                            ),
                        ),
                )
            })
            .child(
                div()
                    .px(px(16.))
                    .pb(px(16.))
                    .child(mutations::render(this, cx)),
            )
            .into_any_element(),
    };
    let mut tabs = div()
        .min_h(px(34.))
        .px(px(6.))
        .flex_shrink_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.))
        .border_b_1()
        .border_color(p.border);
    for (tab, label) in [
        (Tab::Details, "Commit details"),
        (Tab::Patch, "Diff"),
        (Tab::Operations, "Operations"),
    ] {
        tabs = tabs.child(
            dunlin_ui::button(format!("history-inspector-{label}"))
                .ghost()
                .small()
                .rounded(px(dunlin_ui::radii::CODE))
                .label(label)
                .when(view.inspector.tab == tab, |b| {
                    b.bg(p.hover).text_color(p.fg)
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if tab == Tab::Operations
                        && let Some(view) = &mut this.git_history.view
                        && let Some(commit) = view.selected.and_then(|i| view.rows.get(i))
                    {
                        // Opening this tab must not invalidate an already
                        // prepared request; its pinned target stays visible.
                        view.mutations.initialize_target(&commit.oid, window, cx);
                    }
                    if tab == Tab::Patch
                        && this.git_history.view.as_ref().is_some_and(|v| {
                            v.selected_file.is_none()
                                && v.detail.as_ref().is_some_and(|d| !d.files.is_empty())
                        })
                    {
                        this.select_history_file(0, cx);
                    }
                    if let Some(v) = &mut this.git_history.view {
                        v.inspector.tab = tab;
                    }
                    cx.notify();
                })),
        );
    }
    let lower = div()
        .size_full()
        .flex()
        .flex_col()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_hidden()
        .child(tabs)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(content),
        );
    div()
        .id("commit-inspector")
        .size_full()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .text_size(px(11.))
        .child(
            dunlin_ui::component::resizable::v_resizable("history-inspector-rows")
                .child(
                    dunlin_ui::component::resizable::resizable_panel()
                        .size(px(380.))
                        .size_range(px(100.)..px(4000.))
                        .child(files::render(this, cx)),
                )
                .child(
                    dunlin_ui::component::resizable::resizable_panel()
                        .size(px(260.))
                        .size_range(px(120.)..px(4000.))
                        .child(lower),
                ),
        )
        .into_any_element()
}

fn metadata_row(label: &str, value: String, p: Palette) -> AnyElement {
    div()
        .flex()
        .gap(px(8.))
        .min_w(px(0.))
        .child(
            div()
                .w(px(62.))
                .flex_shrink_0()
                .text_color(p.muted_fg)
                .child(label.to_owned()),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .font_family(dunlin_ui::theme::code_font())
                .child(value),
        )
        .into_any_element()
}

fn notice(text: &str, color: Hsla) -> AnyElement {
    div()
        .p(px(16.))
        .text_size(px(12.))
        .text_color(color)
        .child(text.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // A wildcard import also brings GPUI's #[test] macro into this module.
    use super::{CommitFile, file_labels, message_body, short_oid};

    #[test]
    fn file_rows_prioritize_basenames_without_losing_full_paths_or_rename_identity() {
        let mut file = CommitFile {
            status: "M".into(),
            path: "src/features/landing/page.tsx".into(),
            old_path: None,
        };
        let labels = file_labels(&file);
        assert_eq!(labels.name, "page.tsx");
        assert_eq!(labels.directory, "src/features/landing");
        assert_eq!(labels.full, file.path);
        file.old_path = Some("old/page.tsx".into());
        let labels = file_labels(&file);
        assert_eq!(labels.name, "page.tsx");
        assert_eq!(labels.directory, "from old/page.tsx");
        assert_eq!(labels.full, "old/page.tsx → src/features/landing/page.tsx");
    }

    #[test]
    fn literal_names_are_escaped_after_splitting_and_root_files_do_not_repeat_the_name() {
        let file = CommitFile {
            status: "A".into(),
            path: "目录/line\nbreak\t.rs".into(),
            old_path: None,
        };
        let labels = file_labels(&file);
        assert_eq!(labels.name, "line\\nbreak\\t.rs");
        assert_eq!(labels.directory, "目录");
        let labels = file_labels(&CommitFile {
            path: "README.md".into(),
            ..file
        });
        assert_eq!(labels.name, "README.md");
        assert!(labels.directory.is_empty());
    }

    #[test]
    fn commit_metadata_keeps_the_body_without_repeating_the_subject() {
        assert_eq!(
            message_body("Subject\n\nBody\nSecond line\n", "Subject"),
            "Body\nSecond line"
        );
        assert_eq!(message_body("Subject", "Subject"), "");
        assert_eq!(
            message_body("Different\nBody", "Subject"),
            "Different\nBody"
        );
        assert_eq!(short_oid(&"a".repeat(40)), "aaaaaaaa");
    }
}
