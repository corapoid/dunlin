//! Compact commit inspector: quiet metadata, flat file rows and a dominant patch.
use super::*;
use dunlin_ui::component::{Icon, button::ButtonCustomVariant, tooltip::Tooltip};
use dunlin_workspace::git::history::CommitFile;

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
                .truncate()
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
                .child(date),
        )
        .child(comparison);
    if let Some(detail) = &view.detail {
        let body = message_body(&detail.message, &commit.subject);
        if !body.is_empty() {
            header = header.child(
                div()
                    .id("commit-message")
                    .max_h(px(52.))
                    .overflow_y_scroll()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(body.to_string()),
            );
        }
    }
    let count = view.detail.as_ref().map_or(0, |d| d.files.len());
    let mut rows = div()
        .id("commit-file-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col();
    if let Some(detail) = &view.detail {
        for (index, file) in detail.files.iter().enumerate() {
            let labels = file_labels(file);
            let active = view.selected_file == Some(index);
            let (icon, icon_color) = crate::file_icons::appearance(&file.path, &p);
            rows = rows.child(
                dunlin_ui::button(format!("commit-file-{index}"))
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(if active { p.blue_soft } else { p.panel })
                            .foreground(p.fg)
                            .hover(if active { p.blue_soft } else { p.hover })
                            .active(p.blue_soft),
                    )
                    .small()
                    .rounded(px(0.))
                    .w_full()
                    .h(px(44.))
                    .px(px(10.))
                    .flex_shrink_0()
                    .border_l_2()
                    .border_color(if active { p.blue } else { p.panel })
                    .accessibility_label(format!(
                        "{} {}{}",
                        file.status,
                        labels.full,
                        if active { ", selected" } else { "" }
                    ))
                    .tooltip(format!("{} · {}", file.status, labels.full))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
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
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .w_full()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::NORMAL)
                                            .text_color(p.fg)
                                            .truncate()
                                            .child(labels.name),
                                    )
                                    .when(!labels.directory.is_empty(), |text| {
                                        text.child(
                                            div()
                                                .w_full()
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::NORMAL)
                                                .text_color(p.muted_fg)
                                                .truncate()
                                                .child(labels.directory),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .w(px(12.))
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(status_color(&file.status, &p))
                                    .child(file.status.chars().take(1).collect::<String>()),
                            ),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_history_file(index, cx)),
                    ),
            );
        }
    }
    let files = div()
        .w(relative(0.28))
        .min_w(px(170.))
        .max_w(px(260.))
        .flex_shrink_0()
        .min_h(px(0.))
        .bg(p.panel)
        .rounded_bl(px(dunlin_ui::scale::WORKSPACE_RADIUS))
        // Overflow masks are rectangular; keep scrolling row fills above the curved bottom.
        .pb(px(dunlin_ui::scale::WORKSPACE_RADIUS))
        .border_r_1()
        .border_color(p.border)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(30.))
                .px(px(12.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child("CHANGED FILES")
                .child(if view.detail_loading {
                    "…".into()
                } else {
                    count.to_string()
                }),
        )
        .child(rows);
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
    div()
        .id("commit-inspector")
        .flex_1()
        .min_h(px(160.))
        .min_w(px(0.))
        .overflow_hidden()
        .border_t_1()
        .border_color(p.border)
        .flex()
        .flex_col()
        .text_size(px(11.))
        .child(header)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .overflow_hidden()
                .child(files)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(patch),
                ),
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
