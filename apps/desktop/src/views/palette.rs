//! The ⌘K command palette: a calm, keyboard-first launcher for navigation,
//! project switching and workspace actions.

use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::input::Input;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::DunlinApp;

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let show_shortcuts = this.show_shortcut_hints;
    let items = this.filtered_commands();
    let selected = if items.is_empty() {
        0
    } else {
        this.command_index.min(items.len() - 1)
    };

    let search = div()
        .h(px(46.))
        .px(px(13.))
        .flex()
        .items_center()
        .gap(px(9.))
        .child(
            Icon::new(IconName::Search)
                .size(px(14.))
                .text_color(p.muted_fg),
        )
        .child(
            div().flex_1().min_w(px(0.)).child(
                Input::new(&this.command_query)
                    .appearance(false)
                    .bordered(false)
                    .h(px(40.)),
            ),
        )
        .when(show_shortcuts, |this| this.child(kbd("esc", p)));

    let mut list = div().flex().flex_col().gap(px(1.)).p(px(5.));
    if items.is_empty() {
        list = list.child(
            div()
                .h(px(74.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(p.muted_fg)
                        .child("No matching commands"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.65))
                        .child("Try “chat”, “folder”, or a project name"),
                ),
        );
    } else {
        for (index, item) in items.iter().enumerate() {
            let active = index == selected;
            let command = item.command;
            let label = item.label.clone();
            let hint = if show_shortcuts {
                item.hint.clone()
            } else {
                None
            };

            list = list.child(
                div()
                    .id(format!("command-row:{index}"))
                    .h(px(38.))
                    .px(px(7.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .cursor_pointer()
                    .when(active, |this| this.bg(p.accent))
                    .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_command(command, window, cx);
                    }))
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(6.))
                            .bg(if active { p.card } else { p.muted })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(item.icon).size(px(12.5)).text_color(if active {
                                p.fg
                            } else {
                                p.muted_fg
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(p.fg)
                            .child(label),
                    )
                    .children(hint.map(|hint| kbd(hint, p))),
            );
        }
    }

    let footer = div()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(12.))
        .when(show_shortcuts, |this| {
            this.child(hint_pair("↑↓", "navigate", p))
                .child(hint_pair("↵", "run", p))
                .child(hint_pair("esc", "close", p))
        })
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.0))
                .font_family("Geist Mono")
                .text_color(p.muted_fg.opacity(0.6))
                .child(format!("{} commands", items.len())),
        );

    let panel = div()
        .id("command-palette")
        .w(px(590.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.card)
        .overflow_hidden()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(search)
        .child(divider(p))
        .child(
            div()
                .id("command-list")
                .max_h(px(330.))
                .overflow_y_scroll()
                .child(list),
        )
        .child(divider(p))
        .child(footer);

    div()
        .id("command-overlay")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(p.overlay)
        .flex()
        .justify_center()
        .items_start()
        .pt(px(92.))
        .on_click(cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
        .child(panel)
        .into_any_element()
}

fn hint_pair(keys: &'static str, label: &'static str, p: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(kbd(keys, p))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child(label),
        )
        .into_any_element()
}
