//! A single header mode toggle and a quiet, grouped formatting toolbar.
use super::{Format, Mode, editable, table::Operation, visual::TableTarget};
use crate::app::DunlinApp;
use dunlin_ui::{
    assets::IconName,
    component::{
        Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        input::{Redo, Undo},
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
    *,
};

const INLINE_TOOLS: [(Format, IconName, &str); 5] = [
    (Format::Bold, IconName::Bold, "Bold"),
    (Format::Italic, IconName::Italic, "Italic"),
    (Format::Strike, IconName::Strikethrough, "Strikethrough"),
    (Format::InlineCode, IconName::Code, "Inline code"),
    (Format::Link, IconName::Link, "Insert link"),
];
const BLOCK_TOOLS: [(Format, IconName, &str); 8] = [
    (Format::Heading, IconName::Heading2, "Heading"),
    (Format::Bullet, IconName::List, "Bullet list"),
    (Format::Numbered, IconName::ListOrdered, "Numbered list"),
    (Format::Task, IconName::ListTodo, "Task list"),
    (Format::Quote, IconName::Quote, "Quote"),
    (Format::CodeBlock, IconName::SquareCode, "Code block"),
    (Format::Table, IconName::Table2, "Table"),
    (Format::Image, IconName::Image, "Image"),
];

pub(super) fn mode_toggle(mode: Mode, cx: &mut Context<DunlinApp>) -> Button {
    let (icon, label, hint) = match mode {
        Mode::Visual => (IconName::CodeXml, "Raw", "Switch to raw Markdown"),
        Mode::Source => (IconName::Eye, "Visual", "Switch to visual editing"),
    };
    Button::new("markdown-mode-toggle")
        .ghost()
        .small()
        .h(px(26.))
        .rounded(px(4.))
        .border_1()
        .border_color(palette(cx).border)
        .icon(Icon::new(icon).size(px(14.)))
        .label(label)
        .text_size(px(11.))
        .tooltip(hint)
        .accessibility_label(hint)
        .on_click(cx.listener(|this, _, window, cx| this.toggle_markdown_mode(window, cx)))
}

fn icon_button(id: impl Into<ElementId>, icon: IconName, hint: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .size(px(28.))
        .flex_shrink_0()
        .rounded(px(4.))
        .icon(Icon::new(icon).size(px(15.)))
        .tooltip(hint)
        .accessibility_label(hint)
}

fn separator(p: &Palette) -> Div {
    div()
        .w(px(1.))
        .h(px(14.))
        .mx(px(7.))
        .bg(p.border)
        .flex_shrink_0()
}

fn block_item(icon: IconName, label: &'static str) -> PopupMenuItem {
    // Inline formatting is non-modal. The effects::menu_item helper registers
    // padded row bounds as a foreground cutout and dims the whole document,
    // leaving a bright rectangle around this dropdown rather than just its menu.
    PopupMenuItem::new(label).icon(icon)
}

fn table_menu(
    target: TableTarget,
    disabled: bool,
    tab_id: String,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    use markdown::mdast::AlignKind;
    let app = cx.entity().downgrade();
    Button::new("markdown-table-tools")
        .ghost()
        .small()
        .h(px(28.))
        .flex_shrink_0()
        .rounded(px(4.))
        .icon(Icon::new(IconName::Table2).size(px(14.)))
        .label("Table")
        .text_size(px(11.))
        .dropdown_caret(true)
        .tooltip("Rows, columns and alignment")
        .accessibility_label("Table operations")
        .disabled(disabled)
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
            menu = menu.label(format!(
                "Row {} · Column {}",
                target.row + 1,
                target.column + 1
            ));
            for (index, (operation, icon, label)) in [
                (Operation::RowBefore, IconName::ArrowUp, "Insert row above"),
                (Operation::RowAfter, IconName::ArrowDown, "Insert row below"),
                (Operation::DeleteRow, IconName::Trash, "Delete row"),
                (
                    Operation::ColumnBefore,
                    IconName::ArrowLeft,
                    "Insert column left",
                ),
                (
                    Operation::ColumnAfter,
                    IconName::ArrowRight,
                    "Insert column right",
                ),
                (Operation::DeleteColumn, IconName::Trash, "Delete column"),
                (
                    Operation::Align(AlignKind::Left),
                    IconName::TextAlignStart,
                    "Align left",
                ),
                (
                    Operation::Align(AlignKind::Center),
                    IconName::TextAlignCenter,
                    "Align center",
                ),
                (
                    Operation::Align(AlignKind::Right),
                    IconName::TextAlignEnd,
                    "Align right",
                ),
                (
                    Operation::Align(AlignKind::None),
                    IconName::TextAlignJustify,
                    "Default alignment",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                if index == 3 || index == 6 {
                    menu = menu.separator();
                }
                let disabled = matches!(operation, Operation::RowBefore | Operation::DeleteRow)
                    && target.row == 0
                    || operation == Operation::DeleteColumn && target.columns <= 1;
                let checked = matches!(operation, Operation::Align(align) if align == target.align);
                let app = app.clone();
                let tab_id = tab_id.clone();
                let target = target.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .icon(icon)
                        .disabled(disabled)
                        .checked(checked)
                        .on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                if this.active_file_tab().is_some_and(|t| t.id == tab_id) {
                                    this.edit_visual_table(target.clone(), operation, window, cx);
                                }
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let tab = this.active_file_tab().unwrap();
    let p = palette(cx);
    let disabled = !editable(this, tab);
    let mut tools = div()
        .id("markdown-format-toolbar")
        .px(px(12.))
        .py(px(5.))
        .min_w(px(0.))
        .overflow_x_scroll()
        .flex_shrink_0()
        .flex()
        .items_center()
        .text_color(p.muted_fg)
        .border_b_1()
        .border_color(p.border);
    for (redo, icon, hint) in [
        (false, IconName::Undo2, "Undo · ⌘Z"),
        (true, IconName::Redo2, "Redo · ⇧⌘Z"),
    ] {
        tools = tools.child(
            icon_button(
                if redo {
                    "markdown-redo"
                } else {
                    "markdown-undo"
                },
                icon,
                hint,
            )
            .disabled(disabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                if this
                    .active_file_tab()
                    .is_some_and(|t| t.markdown.mode == Mode::Visual)
                {
                    this.visual_history(redo, window, cx);
                } else if let Some(tab) = this.active_file_tab().filter(|t| editable(this, t)) {
                    let focus = tab.editor.read(cx).focus_handle(cx);
                    if redo {
                        focus.dispatch_action(&Redo, window, cx);
                    } else {
                        focus.dispatch_action(&Undo, window, cx);
                    }
                    focus.focus(window, cx);
                }
            })),
        );
    }
    tools = tools.child(separator(&p));
    for (index, (format, icon, hint)) in INLINE_TOOLS.into_iter().enumerate() {
        if index == 3 {
            tools = tools.child(separator(&p));
        }
        tools =
            tools.child(
                icon_button(format!("markdown-format-{hint}"), icon, hint)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.format_markdown(format, window, cx)
                    })),
            );
    }
    if this.visual_resource_target(cx).is_some() {
        tools = tools.child(separator(&p)).child(
            Button::new("markdown-resource-properties")
                .ghost()
                .small()
                .h(px(28.))
                .rounded(px(4.))
                .label("Link / image…")
                .text_size(px(11.))
                .on_click(cx.listener(|this, _, window, cx| this.open_visual_resource(window, cx))),
        );
    }
    let app = cx.entity().downgrade();
    let tab_id = tab.id.clone();
    tools
        .child(separator(&p))
        .child(
            Button::new("markdown-insert")
                .ghost()
                .small()
                .h(px(28.))
                .flex_shrink_0()
                .rounded(px(4.))
                .icon(Icon::new(IconName::Plus).size(px(14.)))
                .label("Insert")
                .text_size(px(11.))
                .dropdown_caret(true)
                .tooltip("Headings, lists and blocks")
                .accessibility_label("Insert Markdown block")
                .disabled(disabled)
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
                    for (index, (format, icon, label)) in BLOCK_TOOLS.into_iter().enumerate() {
                        if index == 5 {
                            menu = menu.separator();
                        }
                        let app = app.clone();
                        let tab_id = tab_id.clone();
                        menu = menu.item(block_item(icon, label).on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                if this.active_file_tab().is_some_and(|t| t.id == tab_id) {
                                    this.format_markdown(format, window, cx);
                                }
                            });
                        }));
                    }
                    menu
                }),
        )
        .when_some(this.visual_table_target(), |tools, target| {
            tools
                .child(separator(&p))
                .child(table_menu(target, disabled, tab.id.clone(), cx))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_TOOLS, INLINE_TOOLS, block_item};
    use dunlin_ui::{AssetSource, assets::AllAssets, component::menu::PopupMenuItem};
    use std::{collections::HashSet, mem::discriminant};

    #[test]
    fn compact_toolbar_keeps_all_formats_and_bundles_their_icons() {
        let mut formats = HashSet::new();
        let mut labels = HashSet::new();
        for (format, icon, label) in INLINE_TOOLS.into_iter().chain(BLOCK_TOOLS) {
            assert!(formats.insert(discriminant(&format)));
            assert!(!label.is_empty() && labels.insert(label));
            assert!(AllAssets.load(&icon.path()).unwrap().is_some());
        }
        assert_eq!(formats.len(), 13);
    }

    #[test]
    fn insert_uses_standard_menu_rows_without_modal_backdrop_probes() {
        for (_, icon, expected) in BLOCK_TOOLS {
            let PopupMenuItem::Item {
                label,
                icon,
                disabled,
                ..
            } = block_item(icon, expected)
            else {
                panic!("Insert must use standard rows, not custom modal surface renderers");
            };
            assert_eq!(label.as_ref(), expected);
            assert!(icon.is_some());
            assert!(!disabled);
        }
    }
}
