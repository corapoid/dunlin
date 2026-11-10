//! Flat sliding selector: equal segments, scrub preview, commit on release.
use crate::app::DunlinApp;
use dunlin_domain::MainView;
use dunlin_ui::*;
use std::time::Duration;

const HEIGHT: f32 = 30.;
const SEGMENT: f32 = 76.;
const INSET: f32 = 3.;

#[derive(Default)]
struct State {
    bounds: Option<Bounds<Pixels>>,
    pointer: Option<f32>,
    focus: Option<FocusHandle>,
}

fn position(x: f32, width: f32) -> f32 {
    ((x - INSET) / ((width - INSET * 2.) / 3.) - 0.5).clamp(0., 2.)
}

pub(super) fn render(
    this: &DunlinApp,
    visible: bool,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let state = window.use_keyed_state("toolbar-view-selector", cx, |_, _| State::default());
    if !visible {
        state.update(cx, |state, _| {
            state.pointer = None;
        });
        return div().into_any_element();
    }
    let p = palette(cx);
    let reduce_motion = this.reduce_motion || cx.reduce_motion();
    let dragging = state.read(cx).pointer.is_some();
    let target = state.read(cx).pointer.unwrap_or(this.view as usize as f32);
    let motion = gpui_kit::base::Spring::new(Duration::from_millis(240))
        .with_travel(!dragging && !reduce_motion);
    let selected = gpui_kit::base::spring("toolbar-view-selection", target, motion, window, cx);
    let focus = state.update(cx, |state, cx| {
        state.focus.get_or_insert_with(|| cx.focus_handle()).clone()
    });
    let measure = state.clone();
    let down = state.clone();
    let movement = state.clone();
    let release = state.clone();
    let cancel = state.clone();
    let width = SEGMENT * 3. + INSET * 2.;
    let mut control = sliding_segments(p, selected, 3, HEIGHT)
        .id("toolbar-view-selector")
        .relative()
        .w(px(width))
        .h(px(HEIGHT))
        .flex_shrink_0()
        .track_focus(&focus)
        .capture_any_mouse_down(cx.listener(move |_, event: &MouseDownEvent, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            cx.stop_propagation();
            down.update(cx, |state, cx| {
                if let Some(bounds) = state.bounds {
                    state.pointer = Some(position(
                        f32::from(event.position.x - bounds.left()),
                        f32::from(bounds.size.width),
                    ));
                    if let Some(focus) = &state.focus {
                        focus.focus(window, cx);
                    }
                    cx.notify();
                }
            });
        }))
        .on_mouse_move(cx.listener(move |_, event: &MouseMoveEvent, _, cx| {
            movement.update(cx, |state, cx| {
                if state.pointer.is_some()
                    && event.pressed_button == Some(MouseButton::Left)
                    && let Some(bounds) = state.bounds
                {
                    state.pointer = Some(position(
                        f32::from(event.position.x - bounds.left()),
                        f32::from(bounds.size.width),
                    ));
                    cx.notify();
                }
            });
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                let selected = release.update(cx, |state, cx| {
                    let value = state.pointer.take();
                    cx.notify();
                    value
                });
                if let Some(selected) = selected {
                    this.set_view(MainView::ALL[selected.round() as usize], cx);
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(move |_, _, _, cx| {
                cancel.update(cx, |state, cx| {
                    state.pointer = None;
                    cx.notify();
                });
            }),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            let index = this.view as usize;
            let next = match event.keystroke.key.as_str() {
                "left" | "up" => Some(index.saturating_sub(1)),
                "right" | "down" => Some((index + 1).min(2)),
                "home" => Some(0),
                "end" => Some(2),
                _ => None,
            };
            if let Some(next) = next {
                cx.stop_propagation();
                this.set_view(MainView::ALL[next], cx);
            }
        }))
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, _, cx| {
                    measure.update(cx, |state, _| {
                        state.bounds = Some(bounds);
                    });
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
        .child(
            div()
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("toolbar-view-lens")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .absolute()
                .left(px(INSET + selected * SEGMENT))
                .top(px(INSET))
                .w(px(SEGMENT))
                .h(px(HEIGHT - INSET * 2.)),
        );
    let mut labels = div().absolute().inset_0().flex().p(px(INSET));
    for (index, view) in MainView::ALL.into_iter().enumerate() {
        labels = labels.child(
            sliding_segment(
                ("toolbar-view", index),
                view.label(),
                this.view == view,
                p,
                cx,
            )
            .relative()
            .children({
                #[cfg(debug_assertions)]
                {
                    crate::workspace_ui_smoke::probe(
                        ["view-label-chat", "view-label-graph", "view-label-timeline"][index],
                    )
                }
                #[cfg(not(debug_assertions))]
                {
                    None::<AnyElement>
                }
            })
            .w(px(SEGMENT))
            .h(px(HEIGHT - INSET * 2.))
            .min_w(px(0.))
            .p(px(0.))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .accessibility_label(format!(
                "Open {} view{}",
                view.label(),
                if this.view == view { ", selected" } else { "" }
            ))
            .tooltip(if this.show_shortcut_hints {
                format!("{} · ⌘{} · drag to switch", view.label(), index + 1)
            } else {
                format!("{} · drag to switch", view.label())
            })
            .text_color(if target.round() as usize == index {
                p.fg
            } else {
                p.muted_fg
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx))),
        );
    }
    control = control.child(labels);
    #[cfg(debug_assertions)]
    let control = control.children(crate::workspace_ui_smoke::probe("toolbar-view-selector"));
    control.into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn scrub_position_is_continuous_and_clamped_to_the_three_segments() {
        let width = super::SEGMENT * 3. + super::INSET * 2.;
        assert_eq!(super::position(-100., width), 0.);
        assert_eq!(super::position(width + 100., width), 2.);
        assert!((super::position(width / 2., width) - 1.).abs() < 0.001);
        assert!((super::position(super::INSET + super::SEGMENT, width) - 0.5).abs() < 0.001);
    }
}
