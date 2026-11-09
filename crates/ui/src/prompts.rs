//! One themed renderer for all GPUI confirmations; answer indices stay untouched.
use crate::{palette, *};
use component::button::ButtonVariants;
use gpui_kit::prelude::FluentBuilder as _;

#[cfg(debug_assertions)]
thread_local! {
    static ANSWER_BOUNDS: std::cell::RefCell<Vec<Option<Bounds<Pixels>>>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[cfg(debug_assertions)]
pub fn debug_confirmation_answer(index: usize) -> Option<Bounds<Pixels>> {
    ANSWER_BOUNDS.with(|bounds| bounds.borrow().get(index).copied().flatten())
}

pub(crate) fn install(cx: &mut App) {
    cx.set_prompt_builder(|level, title, detail, answers, handle, window, cx| {
        #[cfg(debug_assertions)]
        ANSWER_BOUNDS.with(|bounds| *bounds.borrow_mut() = vec![None; answers.len()]);
        let view = cx.new(|cx| Confirmation {
            level,
            title: title.into(),
            detail: detail.map(str::to_owned),
            answers: answers.to_vec(),
            focus: cx.focus_handle(),
        });
        handle.with_view(view, window, cx)
    });
}

struct Confirmation {
    level: PromptLevel,
    title: String,
    detail: Option<String>,
    answers: Vec<PromptButton>,
    focus: FocusHandle,
}
fn cancel_index(answers: &[PromptButton]) -> Option<usize> {
    answers.iter().position(PromptButton::is_cancel)
}
impl EventEmitter<PromptResponse> for Confirmation {}
impl Focusable for Confirmation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Confirmation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mut actions = div()
            .flex_shrink_0()
            .p(px(16.))
            .border_t_1()
            .border_color(p.border)
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(8.));
        for (index, answer) in self.answers.iter().enumerate() {
            let button = crate::button(("confirmation-answer", index))
                .label(answer.label().clone())
                .max_w_full()
                .when(
                    index + 1 == self.answers.len() && !answer.is_cancel(),
                    |b| b.primary(),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(PromptResponse(index));
                    cx.stop_propagation();
                }));
            #[cfg(debug_assertions)]
            let button = button.relative().child(
                canvas(
                    move |bounds, _, _| {
                        ANSWER_BOUNDS.with(|entries| {
                            if let Some(entry) = entries.borrow_mut().get_mut(index) {
                                *entry = Some(bounds);
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
            actions = actions.child(button);
        }
        div()
            .id("app-confirmation")
            .role(Role::Dialog)
            .aria_label(self.title.clone())
            .tab_group()
            .absolute()
            .inset_0()
            .size_full()
            .occlude()
            .bg(Hsla {
                a: 0.12,
                ..p.overlay
            })
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key == "escape" || (key == "enter" && this.focus.is_focused(window)) {
                    // Enter on the dialog itself must never authorize a destructive action.
                    if let Some(index) = cancel_index(&this.answers) {
                        cx.emit(PromptResponse(index));
                    }
                }
                if key != "tab" {
                    cx.stop_propagation();
                }
            }))
            .child(
                crate::SurfaceGeometry::DIALOG.paint(p.card, p.border_strong)
                    .w(px(520.))
                    .max_w(relative(0.92))
                    .max_h(relative(0.85))
                    .flex()
                    .flex_col()
                    .text_color(p.fg)
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .p(px(16.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                component::Icon::new(if self.level == PromptLevel::Info {
                                    assets::IconName::Info
                                } else {
                                    assets::IconName::TriangleAlert
                                })
                                .size(px(17.))
                                .text_color(
                                    if self.level == PromptLevel::Info {
                                        p.muted_fg
                                    } else {
                                        p.amber
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.title.clone()),
                            ),
                    )
                    .when_some(self.detail.clone(), |panel, detail| {
                        panel.child(
                            div()
                                .id("confirmation-detail")
                                .min_h(px(0.))
                                .overflow_y_scroll()
                                .px(px(16.))
                                .pb(px(16.))
                                .text_size(px(12.))
                                .line_height(relative(1.5))
                                .child(detail),
                        )
                    })
                    .child(actions),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_preserves_original_index_and_never_guesses_a_destructive_answer() {
        assert_eq!(cancel_index(&["Cancel".into(), "Delete".into()]), Some(0));
        assert_eq!(
            cancel_index(&["Discard".into(), "Cancel".into(), "Save".into()]),
            Some(1)
        );
        assert_eq!(cancel_index(&["Delete everything".into()]), None);
        assert_eq!(cancel_index(&[PromptButton::cancel("Keep files")]), Some(0));
        assert_eq!(
            cancel_index(&[
                PromptButton::cancel("Deny"),
                PromptButton::new("Allow once")
            ]),
            Some(0)
        );
        assert_eq!(
            cancel_index(&[
                PromptButton::cancel("Keep Running"),
                PromptButton::new("Quit Anyway")
            ]),
            Some(0)
        );
    }
}
