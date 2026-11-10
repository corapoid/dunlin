//! Literal, selectable chat text and copy-on-release scoped to conversation
//! selections. Composer/editor selections never reach these handlers.
use dunlin_ui::{component::text::TextView, *};
use gpui_kit::base::TextSelection;
use std::{cell::Cell, rc::Rc};

pub(super) fn plain(id: String, text: &str) -> TextView {
    TextView::plain(id, text.to_string())
        .min_w(px(0.))
        .max_w_full()
        .whitespace_normal()
        .scrollable(false)
        .selectable(true)
}

fn copy_on_release(armed: &Cell<bool>, window: &mut Window, cx: &mut App) {
    if !armed.replace(false) {
        return;
    }
    // Let the selection layer finish the gesture first. Empty clicks never
    // replace the clipboard, and dragging outside the text still completes.
    window.defer(cx, |window, cx| {
        let text = TextSelection::selected_text(window, cx);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    });
}

pub(super) fn auto_copy(
    view: Stateful<Div>,
    session: &str,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let armed = window
        .use_keyed_state(format!("chat-copy-selection:{session}"), cx, |_, _| {
            Rc::new(Cell::new(false))
        })
        .read(cx)
        .clone();
    let on_up = armed.clone();
    let on_out = armed.clone();
    view.on_mouse_down(MouseButton::Left, move |event, window, cx| {
        armed.set(TextSelection::is_selectable_at(event.position, window, cx));
    })
    .on_mouse_up(MouseButton::Left, move |_, window, cx| {
        copy_on_release(&on_up, window, cx)
    })
    .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
        copy_on_release(&on_out, window, cx)
    })
}
