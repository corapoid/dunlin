//! Short click creates chat; holding opens the kit's keyboard-accessible menu.
use super::*;
use dunlin_ui::component::{menu::PopupMenu, popover::Popover};

pub(super) fn button(this: &DunlinApp, pane: usize, cx: &mut Context<DunlinApp>) -> AnyElement {
    let owner = cx.entity().downgrade();
    let changed = owner.clone();
    let control = Popover::new(format!("new-tab-choices:{pane}"))
        .appearance(false)
        .anchor(Anchor::TopLeft)
        .mouse_button(MouseButton::Right)
        .open(this.workbench.new_tab_menu == Some(pane))
        .on_open_change(move |open, _, cx| {
            let _ = changed.update(cx, |this, cx| {
                this.workbench.new_tab_menu = open.then_some(pane);
                cx.notify();
            });
        })
        .trigger(
            icon_button(
                format!("new-chat-tab:{pane}"),
                IconName::Plus,
                this.shortcut_label("New tab · click for Chat, hold for choices", "⌘N"),
            )
            .size(px(28.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    let pressed = std::time::Instant::now();
                    this.workbench.new_tab_press = Some((pane, pressed));
                    let project = this.project().id.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(350))
                            .await;
                        let _ = this.update_in(cx, |this, _, cx| {
                            if this.workbench.new_tab_press == Some((pane, pressed))
                                && this.project().id == project
                                && !this.has_modal_overlay()
                            {
                                this.workbench.new_tab_menu = Some(pane);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.workbench.new_tab_press = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.workbench.new_tab_press = None;
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "down" {
                    this.workbench.new_tab_menu = Some(pane);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                if this.workbench.new_tab_menu.is_none() {
                    this.new_session(cx);
                }
            })),
        )
        .content(move |_, window, cx| {
            let state =
                window.use_keyed_state(format!("new-tab-menu-content:{pane}"), cx, |_, _| {
                    None::<Entity<PopupMenu>>
                });
            if let Some(menu) = state.read(cx).as_ref() {
                return surface(menu.clone());
            }
            let chat = owner.clone();
            let file = owner.clone();
            let terminal = owner.clone();
            let menu = PopupMenu::build(window, cx, move |menu, _, _| {
                menu.item(
                    PopupMenuItem::new("Chat")
                        .icon(IconName::MessageSquare)
                        .on_click(move |_, window, cx| {
                            let _ = chat.update(cx, |this, cx| {
                                this.workbench.new_tab_menu = None;
                                if this.focus_workspace_pane(pane, window, cx) {
                                    this.new_session(cx);
                                }
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("Empty file")
                        .icon(IconName::File)
                        .on_click(move |_, window, cx| {
                            let _ = file.update(cx, |this, cx| {
                                this.workbench.new_tab_menu = None;
                                if this.focus_workspace_pane(pane, window, cx) {
                                    this.new_untitled_file(window, cx);
                                }
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("Terminal")
                        .icon(IconName::Terminal)
                        .on_click(move |_, window, cx| {
                            let _ = terminal.update(cx, |this, cx| {
                                this.workbench.new_tab_menu = None;
                                if this.focus_workspace_pane(pane, window, cx) {
                                    this.open_workspace_terminal(window, cx);
                                }
                            });
                        }),
                )
            });
            let app = owner.clone();
            window
                .subscribe(&menu, cx, move |_, _: &DismissEvent, _, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.workbench.new_tab_menu = None;
                        this.workbench.new_tab_press = None;
                        cx.notify();
                    });
                })
                .detach();
            state.update(cx, |state, _| *state = Some(menu.clone()));
            use dunlin_ui::Focusable;
            menu.focus_handle(cx).focus(window, cx);
            surface(menu)
        });
    div()
        .id(format!("new-tab-hover:{pane}"))
        .on_hover(cx.listener(|this, hovered, _, _| {
            if !hovered {
                this.workbench.new_tab_press = None;
            }
        }))
        .child(control)
        .into_any_element()
}

fn surface(menu: Entity<PopupMenu>) -> AnyElement {
    div()
        .relative()
        .children(super::tab_probe("new-tab-menu"))
        .child(menu)
        .into_any_element()
}
